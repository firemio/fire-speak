use crate::settings::Mode;
use crate::AppState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

// ---------------------------------------------------------------------------
// Hotkey dispatch state (shared by the global-shortcut plugin handler and the
// low-level keyboard hook — both funnel into hotkey_pressed/hotkey_released)
// ---------------------------------------------------------------------------

/// While true the hotkey paths (plugin handler AND keyboard hook) are inert so
/// the settings capture field can observe raw key events. The UI record button
/// (`toggle_recording` -> `toggle`) is unaffected.
static HOTKEY_SUSPENDED: AtomicBool = AtomicBool::new(false);
/// OS auto-repeat guard: a press is handled only on the false->true
/// transition; the matching release resets it. Lives in this shared dispatch
/// layer so both the plugin path and the hook path are covered.
static HOTKEY_HELD: AtomicBool = AtomicBool::new(false);
/// Instant of the hold-mode press that started the current recording
/// (tap-lock timing reference).
static PRESS_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// Hold mode: a release earlier than this after the starting press keeps the
/// recording running (tap-lock) instead of confirming it.
const TAP_LOCK_MS: u64 = 400;

// Live captions (SPEC v0.5): while recording, the audio captured so far is
// periodically sent to the configured STT engine and the partial text is
// emitted as `caption` for the HUD. The final result still comes from the
// full recording in run_pipeline.
/// Pause between the end of one caption request and the next snapshot.
const CAPTION_PERIOD_MS: u64 = 700;
/// Skip a tick unless at least this much new audio arrived since the last one.
const CAPTION_MIN_NEW_SECS: f32 = 0.8;
/// Once the open window is this long its text is committed and a new window
/// starts, so request size (and latency) stays bounded on long dictations.
const CAPTION_WINDOW_SECS: f32 = 20.0;
/// Windows whose peak amplitude stays below this are treated as silence and
/// not sent (whisper hallucinates on silence).
const CAPTION_SILENCE_PEAK: f32 = 0.01;
/// Give up on live captions for this recording after this many consecutive
/// STT failures (e.g. server not installed) instead of retrying forever.
const CAPTION_MAX_FAILURES: u32 = 2;

// Speech gate (v0.10.1): whisper hallucinates on silence and clicks
// ("ご視聴ありがとうございました", "Thanks for watching"…). A recording is
// only sent to STT when it holds some audio that could be speech.
/// 16 kHz samples per gate frame (20 ms).
const GATE_FRAME: usize = 320;
/// A frame counts as active when its peak exceeds this fraction of full scale.
const GATE_FRAME_PEAK: f32 = 0.015;
/// Minimum active audio (seconds) for a recording to be transcribed.
const MIN_SPEECH_SECS: f32 = 0.25;
/// Recordings shorter than this with no speech end silently (the hotkey was
/// used as a modifier); longer silent ones show ERR_NO_SPEECH.
const SILENT_SKIP_SECS: f32 = 1.5;
/// Internal result of the gate for short silent recordings; never emitted.
const SILENT_SKIP: &str = "SILENT_SKIP";

/// Seconds of 20 ms frames whose peak is above `GATE_FRAME_PEAK`.
fn speech_seconds(samples: &[i16]) -> f32 {
    let threshold = (GATE_FRAME_PEAK * i16::MAX as f32) as i32;
    let active = samples
        .chunks(GATE_FRAME)
        .filter(|frame| frame.iter().any(|s| (*s as i32).abs() > threshold))
        .count();
    active as f32 * GATE_FRAME as f32 / 16_000.0
}

/// Text whisper produces for silence or background noise rather than for
/// speech: video sign-offs, subtitle credits, bracketed sound tags. Compared
/// after lowercasing and stripping whitespace and punctuation. Only phrases
/// that practically never occur in dictation are matched (a plain
/// "ありがとうございました" is legitimate input; the speech gate handles the
/// silent case that produces it).
fn is_hallucination(text: &str) -> bool {
    let norm: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && !is_strip_punct(*c))
        .flat_map(|c| c.to_lowercase())
        .collect();
    if norm.is_empty() {
        return true;
    }
    let t = text.trim();
    let bracketed = [('[', ']'), ('(', ')'), ('（', '）'), ('【', '】'), ('「', '」')]
        .iter()
        .any(|(o, c)| t.starts_with(*o) && t.ends_with(*c) && t.matches(*o).count() == 1);
    if bracketed {
        return true;
    }
    // Sign-offs are short; a real sentence that happens to mention subtitles
    // or a channel must not be dropped.
    if norm.chars().count() > 40 {
        return false;
    }
    const MARKERS: &[&str] = &[
        // ja
        "ご視聴", "視聴ありがとう", "チャンネル登録", "字幕by", "字幕提供", "字幕作成", "字幕制作",
        "次回の動画", "最後までご覧", "ご覧いただきありがとう", "ご覧くださりありがとう", "動画をご覧",
        // en
        "thanksforwatching", "thankyouforwatching", "subtitlesby", "subtitledby", "captionsby",
        "transcribedby", "andsubscribe", "pleasesubscribe", "subscribetomychannel",
        "subscribetothechannel", "seeyouinthenext",
        "amara.org", "likeandshare",
        // zh
        "谢谢观看", "謝謝觀看", "感谢观看", "感謝觀看", "字幕由", "明镜与点点", "点赞订阅", "點贊訂閱",
        "订阅转发", "訂閱轉發",
        // ko
        "시청해주셔서", "구독과좋아요", "좋아요와구독", "구독버튼",
        // de / fr / es / pt / ru / vi / id
        "untertitelvon", "untertiteldurch", "untertitelimauftrag", "dankefürszuschauen",
        "vielendankfürszuschauen",
        "soustitrespar", "soustitresréalisés", "soustitrage", "mercidavoirregardé", "abonnezvous",
        "subtítulospor", "subtituladopor", "subtítulosrealizados", "graciasporver", "suscríbete",
        "suscribete",
        "legendaspela", "legendaspor", "legendadopor", "obrigadoporassistir", "obrigadaporassistir",
        "inscrevase",
        "субтитрысделал", "субтитрыот", "редакторсубтитров", "спасибозапросмотр", "подписывайтесь",
        "cảmơncácbạnđãtheodõi", "cảmơnđãtheodõi", "đăngkýkênh", "hẹngặplại",
        "terimakasihtelahmenonton", "terimakasihsudahmenonton",
        // whisper special tokens (bracketed tags are caught above)
        "blank_audio", "blankaudio",
    ];
    MARKERS.iter().any(|m| norm.contains(m))
}

fn is_strip_punct(c: char) -> bool {
    c.is_ascii_punctuation()
        || matches!(
            c,
            '。' | '、' | '！' | '？' | '…' | '「' | '」' | '『' | '』' | '【' | '】' | '（' | '）'
                | '・' | '〜' | '～' | '，' | '．' | '：' | '；' | '\u{201C}' | '\u{201D}' | '\u{2018}' | '\u{2019}'
        )
}

pub fn set_hotkey_suspended(suspended: bool) {
    HOTKEY_SUSPENDED.store(suspended, Ordering::SeqCst);
}

pub fn hotkey_suspended() -> bool {
    HOTKEY_SUSPENDED.load(Ordering::SeqCst)
}

/// Hotkey down. Shared entry for the global-shortcut plugin
/// (ShortcutState::Pressed) and the low-level hook (keydown).
pub fn hotkey_pressed(app: &AppHandle) {
    if hotkey_suspended() {
        return;
    }
    if HOTKEY_HELD.swap(true, Ordering::SeqCst) {
        return; // OS auto-repeat while held
    }
    let hold = app
        .state::<AppState>()
        .settings
        .lock()
        .unwrap()
        .hotkey_mode
        != "toggle";
    if !hold {
        // toggle mode: press = classic toggle, release ignored
        toggle(app);
        return;
    }
    // hold mode
    let current = *app.state::<AppState>().status.lock().unwrap();
    match current {
        Status::Idle => {
            *PRESS_AT.lock().unwrap() = Some(Instant::now());
            #[cfg(windows)]
            spawn_modifier_use_watch(app.clone());
            start_recording(app);
        }
        // recording (tap-locked, or started elsewhere): press confirms
        Status::Recording => stop_and_process(app),
        _ => {} // ignore while transcribing/polishing
    }
}

/// Hotkey up. Shared entry for the global-shortcut plugin
/// (ShortcutState::Released) and the low-level hook (keyup).
pub fn hotkey_released(app: &AppHandle) {
    if hotkey_suspended() {
        // Ignore, but clear the held flag in case suspension started
        // mid-hold — otherwise the next press would be swallowed.
        HOTKEY_HELD.store(false, Ordering::SeqCst);
        return;
    }
    if !HOTKEY_HELD.swap(false, Ordering::SeqCst) {
        return; // release without a press we handled
    }
    let hold = app
        .state::<AppState>()
        .settings
        .lock()
        .unwrap()
        .hotkey_mode
        != "toggle";
    if !hold {
        return; // toggle mode: release ignored
    }
    let is_recording =
        *app.state::<AppState>().status.lock().unwrap() == Status::Recording;
    if !is_recording {
        return;
    }
    let long_enough = {
        let at = *PRESS_AT.lock().unwrap();
        at.map(|t| t.elapsed() >= Duration::from_millis(TAP_LOCK_MS))
            .unwrap_or(false)
    };
    if long_enough {
        stop_and_process(app);
    }
    // else: tap-lock — keep recording; the next press confirms
}

/// Hold mode with a bare modifier hotkey (RAlt…): while the key is held, a
/// mouse click or any other key means the user is using it as a modifier
/// (Alt+click, Alt+Tab…), not talking — cancel the recording silently
/// (v0.10.1). Polls GetAsyncKeyState every 20 ms until the key is released,
/// the recording ends, or it is canceled. Keys already down at the press do
/// not count.
#[cfg(windows)]
fn spawn_modifier_use_watch(app: AppHandle) {
    let special = {
        let state = app.state::<AppState>();
        let settings = state.settings.lock().unwrap();
        crate::hook::is_special_token(&settings.hotkey)
    };
    if !special {
        return;
    }
    // start_recording (called right after) bumps the generation once
    let expected_gen = app.state::<AppState>().generation.load(Ordering::SeqCst) + 1;
    std::thread::spawn(move || {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        let is_down = |vk: i32| (unsafe { GetAsyncKeyState(vk) } as u16) & 0x8000 != 0;
        // modifiers (generic and L/R) never count; neither does the hotkey itself
        let ignored = |vk: i32| matches!(vk, 0x10..=0x12 | 0xA0..=0xA5 | 0xE7 | 0xFF);
        let mut baseline = [false; 256];
        for (vk, slot) in baseline.iter_mut().enumerate().skip(1) {
            *slot = is_down(vk as i32);
        }
        let started = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(20));
            if !HOTKEY_HELD.load(Ordering::SeqCst) {
                return; // released: tap-lock or confirm, no longer a modifier
            }
            let state = app.state::<AppState>();
            let gen = state.generation.load(Ordering::SeqCst);
            if gen > expected_gen {
                return; // canceled or superseded
            }
            if gen == expected_gen && *state.status.lock().unwrap() != Status::Recording {
                return;
            }
            if gen < expected_gen && started.elapsed() > Duration::from_secs(10) {
                return; // the recording never started
            }
            for vk in 1..256usize {
                if ignored(vk as i32) || baseline[vk] {
                    continue;
                }
                if is_down(vk as i32) {
                    cancel(&app);
                    return;
                }
            }
        }
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Idle,
    Recording,
    Transcribing,
    Polishing,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Idle => "idle",
            Status::Recording => "recording",
            Status::Transcribing => "transcribing",
            Status::Polishing => "polishing",
        }
    }
}

pub fn emit_status(app: &AppHandle, status: &str, message: Option<&str>) {
    let payload = match message {
        Some(m) => serde_json::json!({ "status": status, "message": m }),
        None => serde_json::json!({ "status": status }),
    };
    let _ = app.emit("status-changed", payload);
}

fn is_canceled(app: &AppHandle, gen: u64) -> bool {
    app.state::<AppState>().generation.load(Ordering::SeqCst) != gen
}

// ---------------------------------------------------------------------------
// Overlay helpers
// ---------------------------------------------------------------------------

/// Position the overlay at the bottom-center of the primary monitor's work
/// area and show it without stealing focus (the window is focus:false).
pub fn show_overlay(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("overlay") {
        if let Ok(Some(monitor)) = window.primary_monitor() {
            let scale = monitor.scale_factor();
            let (w, h) = match window.outer_size() {
                Ok(s) => (s.width as i32, s.height as i32),
                Err(_) => ((380.0 * scale) as i32, (190.0 * scale) as i32),
            };
            let area = monitor.work_area();
            let margin = (24.0 * scale) as i32;
            let x = area.position.x + (area.size.width as i32 - w) / 2;
            let y = area.position.y + area.size.height as i32 - h - margin;
            let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
        }
        let _ = window.show();
        // never call set_focus on the overlay
    }
}

pub fn hide_overlay(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("overlay") {
        let _ = window.hide();
    }
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

/// Hotkey / toggle_recording entry point.
/// idle -> start recording; recording -> stop & process; otherwise ignored.
pub fn toggle(app: &AppHandle) {
    let current = *app.state::<AppState>().status.lock().unwrap();
    match current {
        Status::Idle => start_recording(app),
        Status::Recording => stop_and_process(app),
        _ => {} // ignore while processing
    }
}

pub fn start_recording(app: &AppHandle) {
    let state = app.state::<AppState>();
    {
        let mut status = state.status.lock().unwrap();
        if *status != Status::Idle {
            return;
        }
        *status = Status::Recording;
    }
    let gen = state.generation.fetch_add(1, Ordering::SeqCst) + 1;

    show_overlay(app);
    emit_status(app, "recording", None);

    match crate::audio::start(app.clone()) {
        Ok(handle) => {
            // audio::start blocks (up to 8s); a cancel/other transition may
            // have happened meanwhile. Only store the handle if this start is
            // still the current one — otherwise drop it (Drop stops the
            // capture thread) so no orphaned recorder keeps running.
            {
                let mut recorder = state.recorder.lock().unwrap();
                let status_ok = *state.status.lock().unwrap() == Status::Recording;
                let gen_ok = state.generation.load(Ordering::SeqCst) == gen;
                if !(status_ok && gen_ok) {
                    drop(recorder);
                    drop(handle); // RecorderHandle::drop signals the thread to stop
                    return;
                }
                *recorder = Some(handle);
            }
            if state.settings.lock().unwrap().live_caption {
                spawn_live_captions(app.clone(), gen);
            }
            // safety watchdog: auto stop+process after 5 minutes
            let app2 = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(301)).await;
                let state = app2.state::<AppState>();
                let still_recording =
                    *state.status.lock().unwrap() == Status::Recording;
                if still_recording && state.generation.load(Ordering::SeqCst) == gen {
                    stop_and_process(&app2);
                }
            });
        }
        Err(e) => {
            {
                let mut status = state.status.lock().unwrap();
                if state.generation.load(Ordering::SeqCst) != gen {
                    // canceled while starting: cancel() owns the Idle write
                    return;
                }
                *status = Status::Idle;
            }
            error_flow(app, e);
        }
    }
}

fn is_recording(app: &AppHandle, gen: u64) -> bool {
    !is_canceled(app, gen) && *app.state::<AppState>().status.lock().unwrap() == Status::Recording
}

/// Scripts written without inter-word spaces (kana, kanji/hanzi, hangul,
/// fullwidth punctuation): a caption window boundary next to one of these
/// characters is joined directly, any other script gets a separating space.
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3000..=0x303F   // CJK symbols & punctuation
        | 0x3040..=0x30FF // hiragana, katakana
        | 0x3400..=0x4DBF // CJK ext A
        | 0x4E00..=0x9FFF // CJK unified
        | 0xAC00..=0xD7AF // hangul syllables
        | 0xF900..=0xFAFF // CJK compatibility
        | 0xFF00..=0xFFEF // fullwidth forms
        | 0x20000..=0x2FFFF // CJK ext B+
    )
}

/// Append a freshly transcribed window to the committed caption text.
fn join_caption(committed: &str, tail: &str) -> String {
    if committed.is_empty() {
        return tail.to_string();
    }
    if tail.is_empty() {
        return committed.to_string();
    }
    let last = committed.chars().last().unwrap_or(' ');
    let first = tail.chars().next().unwrap_or(' ');
    let joined_directly =
        last.is_whitespace() || first.is_whitespace() || is_cjk(last) || is_cjk(first);
    if joined_directly {
        format!("{committed}{tail}")
    } else {
        format!("{committed} {tail}")
    }
}

/// Live-caption loop for the recording identified by `gen`. Exits as soon as
/// the recording stops or is canceled; never touches status.
fn spawn_live_captions(app: AppHandle, gen: u64) {
    tauri::async_runtime::spawn(async move {
        let mut committed = String::new();
        let mut window_start: usize = 0; // device-rate index where the open window begins
        let mut last_end: usize = 0; // device-rate length at the previous request
        let mut failures: u32 = 0;
        loop {
            tokio::time::sleep(Duration::from_millis(CAPTION_PERIOD_MS)).await;
            if !is_recording(&app, gen) {
                return;
            }
            let settings = app.state::<AppState>().settings.lock().unwrap().clone();
            if !settings.live_caption {
                return;
            }
            let (snap, min_new, window_limit) = {
                let state = app.state::<AppState>();
                let recorder = state.recorder.lock().unwrap();
                let Some(handle) = recorder.as_ref() else {
                    return; // recorder taken: stop_and_process is running
                };
                (
                    handle.snapshot(window_start),
                    handle.samples_for_secs(CAPTION_MIN_NEW_SECS),
                    handle.samples_for_secs(CAPTION_WINDOW_SECS),
                )
            };
            if snap.end < last_end + min_new {
                continue; // not enough new audio yet
            }
            last_end = snap.end;
            if snap.peak < CAPTION_SILENCE_PEAK {
                continue; // silence: nothing to transcribe
            }
            let wav = match crate::audio::wav_bytes(&snap.samples) {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("live caption wav failed: {e}");
                    continue;
                }
            };
            let text = match crate::stt::transcribe(&app, &settings, wav).await {
                Ok(t) => {
                    failures = 0;
                    t.trim().to_string()
                }
                Err(e) => {
                    failures += 1;
                    eprintln!("live caption STT failed ({failures}/{CAPTION_MAX_FAILURES}): {e}");
                    if failures >= CAPTION_MAX_FAILURES {
                        return;
                    }
                    continue;
                }
            };
            if !is_recording(&app, gen) {
                return; // recording ended while the request was in flight
            }
            if is_hallucination(&text) {
                continue; // noise window: keep the caption as it was
            }
            let combined = join_caption(&committed, &text);
            let _ = app.emit("caption", serde_json::json!({ "text": combined }));
            if snap.end - window_start >= window_limit {
                committed = combined;
                window_start = snap.end;
            }
        }
    });
}

pub fn stop_and_process(app: &AppHandle) {
    let state = app.state::<AppState>();
    let gen = {
        let mut status = state.status.lock().unwrap();
        if *status != Status::Recording {
            return;
        }
        *status = Status::Transcribing;
        state.generation.load(Ordering::SeqCst)
    };
    let handle = state.recorder.lock().unwrap().take();
    let Some(handle) = handle else {
        // No recorder (e.g. confirmed while the mic was still initializing):
        // surface it as ERR_NO_SPEECH instead of silently going idle —
        // status-changed{error}, 2.5s overlay display, then idle.
        {
            let mut status = state.status.lock().unwrap();
            if state.generation.load(Ordering::SeqCst) != gen {
                return; // canceled: cancel() owns the Idle write
            }
            *status = Status::Idle;
        }
        error_flow(app, "ERR_NO_SPEECH".to_string());
        return;
    };
    emit_status(app, "transcribing", None);

    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = run_pipeline(app2.clone(), handle, gen).await;
        if let Err(e) = result {
            let state = app2.state::<AppState>();
            let notify = {
                let mut status = state.status.lock().unwrap();
                if is_canceled(&app2, gen) {
                    false // canceled: never write status
                } else {
                    *status = Status::Idle;
                    true
                }
            };
            if notify {
                if e == SILENT_SKIP {
                    // modifier tap / click only: no message, just go idle
                    hide_overlay(&app2);
                    emit_status(&app2, "idle", None);
                } else {
                    error_flow(&app2, e);
                }
            }
        }
    });
}

async fn run_pipeline(
    app: AppHandle,
    handle: crate::audio::RecorderHandle,
    gen: u64,
) -> Result<(), String> {
    let settings = app
        .state::<AppState>()
        .settings
        .lock()
        .unwrap()
        .clone();

    // 1. stop recording, collect samples
    let samples = tokio::task::spawn_blocking(move || handle.stop_and_take())
        .await
        .map_err(|e| format!("ERR_INTERNAL|record stop: {e}"))??;
    if is_canceled(&app, gen) {
        return Ok(());
    }
    if samples.is_empty() {
        return Err("ERR_NO_SPEECH".to_string());
    }
    // Speech gate: a click or a modifier tap holds no speech; sending it
    // would only get a hallucinated sign-off back.
    if speech_seconds(&samples) < MIN_SPEECH_SECS {
        let total_secs = samples.len() as f32 / 16_000.0;
        return Err(if total_secs < SILENT_SKIP_SECS {
            SILENT_SKIP.to_string()
        } else {
            "ERR_NO_SPEECH".to_string()
        });
    }
    let wav = crate::audio::wav_bytes(&samples)?;

    // 2. STT
    let raw_text = crate::stt::transcribe(&app, &settings, wav).await?;
    if is_canceled(&app, gen) {
        return Ok(());
    }
    let raw_text = raw_text.trim().to_string();
    if raw_text.is_empty() || is_hallucination(&raw_text) {
        return Err("ERR_NO_SPEECH".to_string());
    }

    // 3. LLM polish (optional)
    let mode = settings
        .modes
        .iter()
        .find(|m| m.id == settings.active_mode_id && m.enabled)
        .cloned()
        // "none", a deleted mode, or a disabled one: paste the transcript as-is.
        .unwrap_or(Mode {
            id: crate::settings::NONE_MODE_ID.to_string(),
            name: String::new(),
            instruction: String::new(),
            use_llm: false,
            enabled: true,
        });
    let provider = settings
        .llm
        .providers
        .iter()
        .find(|p| p.id == settings.llm.active_provider_id)
        .cloned();

    let mut warning: Option<String> = None;
    let mut final_text = raw_text.clone();
    if mode.use_llm {
        match provider.filter(crate::llm::is_configured) {
            // The mode asks for polishing but nothing can run it: paste the
            // transcript and SAY SO in the HUD, instead of silently pretending
            // the polish happened (v0.7 — this hid a missing API key for weeks).
            None => warning = Some("WARN_LLM_NO_KEY".to_string()),
            Some(p) => {
                {
                    // Only advance to Polishing if this pipeline is still current.
                    let state = app.state::<AppState>();
                    let mut status = state.status.lock().unwrap();
                    if is_canceled(&app, gen) || *status != Status::Transcribing {
                        return Ok(());
                    }
                    *status = Status::Polishing;
                }
                emit_status(&app, "polishing", None);
                match crate::llm::polish(&p, &mode.instruction, &raw_text).await {
                    Ok(t) if !t.trim().is_empty() => final_text = t.trim().to_string(),
                    Ok(_) => {
                        warning = Some("WARN_LLM_FALLBACK".to_string());
                    }
                    Err(e) => {
                        eprintln!("llm polish failed, falling back to raw text: {e}");
                        warning = Some("WARN_LLM_FALLBACK".to_string());
                    }
                }
            }
        }
    }
    // A canceled pipeline must not paste.
    if is_canceled(&app, gen) {
        return Ok(());
    }

    // 4. paste / copy
    let paste_mode = settings.paste_mode.clone();
    let restore = settings.restore_clipboard;
    let text_for_paste = final_text.clone();
    let paste_result =
        tokio::task::spawn_blocking(move || {
            crate::paste::paste_text(&text_for_paste, &paste_mode, restore)
        })
        .await
        .map_err(|e| format!("ERR_INTERNAL|paste: {e}"))?;

    // 5. result + history (saved even if paste failed)
    let _ = app.emit(
        "result",
        serde_json::json!({ "raw_text": raw_text, "final_text": final_text }),
    );
    if let Err(e) = crate::history::add(
        &app,
        mode.id.clone(),
        raw_text.clone(),
        final_text.clone(),
        settings.history_limit,
    ) {
        eprintln!("history save failed: {e}");
    }

    paste_result?; // paste failure -> error flow

    // 6. done -> idle (skip entirely if canceled meanwhile; cancel() owns
    // the Idle write on cancellation)
    {
        let state = app.state::<AppState>();
        let mut status = state.status.lock().unwrap();
        if is_canceled(&app, gen) {
            return Ok(());
        }
        *status = Status::Idle;
    }
    emit_status(&app, "done", warning.as_deref());
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1200)).await;
        if !is_canceled(&app2, gen) {
            hide_overlay(&app2);
            emit_status(&app2, "idle", None);
        }
    });
    Ok(())
}

/// Cancel any in-flight recording/processing and hide the overlay.
pub fn cancel(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.generation.fetch_add(1, Ordering::SeqCst);
    if let Some(handle) = state.recorder.lock().unwrap().take() {
        handle.cancel();
    }
    *state.status.lock().unwrap() = Status::Idle;
    hide_overlay(app);
    emit_status(app, "idle", None);
}

/// Emit an error status, then hide the overlay after 2.5s and return to idle.
/// Callers must have already set the status to Idle under their own
/// generation-guarded critical section (this function never writes status).
pub fn error_flow(app: &AppHandle, message: String) {
    let state = app.state::<AppState>();
    emit_status(app, "error", Some(&message));
    let gen = state.generation.load(Ordering::SeqCst);
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(2500)).await;
        if !is_canceled(&app2, gen) {
            let still_idle =
                *app2.state::<AppState>().status.lock().unwrap() == Status::Idle;
            if still_idle {
                hide_overlay(&app2);
                emit_status(&app2, "idle", None);
            }
        }
    });
}

#[cfg(test)]
mod gate_tests {
    use super::*;

    #[test]
    fn hallucinations() {
        for t in [
            "ご視聴ありがとうございました",
            "ご視聴ありがとうございました。",
            " チャンネル登録お願いします ",
            "Thanks for watching!",
            "Thank you for watching.",
            "Subtitles by the Amara.org community",
            "[BLANK_AUDIO]",
            "(拍手)",
            "[Music]",
            "字幕由Amara.org社区提供",
            "시청해주셔서 감사합니다",
            "",
            "。",
        ] {
            assert!(is_hallucination(t), "{t:?} should be dropped");
        }
    }

    #[test]
    fn real_speech_passes() {
        for t in [
            "ありがとうございました",
            "はい",
            "明日の会議は10時からです",
            "The quarterly numbers look good, thank you.",
            "この動画の字幕を日本語に翻訳して、タイトルは変えずに保存してください。全部で三つのファイルがあります。",
            "(仮)のタイトルで保存",
            "字幕を付けて",
            "play some music",
            "구독 취소해 줘",
            "subscribe to the newsletter",
        ] {
            assert!(!is_hallucination(t), "{t:?} should pass");
        }
    }

    #[test]
    fn speech_gate() {
        // 2 s of silence with a 30 ms click: no speech
        let mut click = vec![0i16; 32_000];
        for s in click.iter_mut().skip(8_000).take(480) {
            *s = 12_000;
        }
        assert!(speech_seconds(&click) < MIN_SPEECH_SECS);
        // 0.5 s of quiet tone (peak ~ 3% of full scale) counts as speech
        let tone: Vec<i16> = (0..8_000)
            .map(|i| (1000.0 * (i as f32 * 0.3).sin()) as i16)
            .collect();
        assert!(speech_seconds(&tone) >= MIN_SPEECH_SECS);
        assert_eq!(speech_seconds(&[0i16; 16_000]), 0.0);
    }
}
