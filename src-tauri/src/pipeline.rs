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
const CAPTION_PERIOD_MS: u64 = 1200;
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
                error_flow(&app2, e);
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
    let wav = crate::audio::wav_bytes(&samples)?;

    // 2. STT
    let raw_text = crate::stt::transcribe(&app, &settings, wav).await?;
    if is_canceled(&app, gen) {
        return Ok(());
    }
    let raw_text = raw_text.trim().to_string();
    if raw_text.is_empty() {
        return Err("ERR_NO_SPEECH".to_string());
    }

    // 3. LLM polish (optional)
    let mode = settings
        .modes
        .iter()
        .find(|m| m.id == settings.active_mode_id)
        .cloned()
        .unwrap_or(Mode {
            id: "raw".to_string(),
            name: "そのまま".to_string(),
            instruction: String::new(),
            use_llm: false,
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
        if let Some(p) = provider.filter(|p| !p.api_key.trim().is_empty()) {
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
