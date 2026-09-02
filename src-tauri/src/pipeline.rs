use crate::settings::Mode;
use crate::AppState;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

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
                Err(_) => ((380.0 * scale) as i32, (110.0 * scale) as i32),
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
            *state.recorder.lock().unwrap() = Some(handle);
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
            *state.status.lock().unwrap() = Status::Idle;
            error_flow(app, e);
        }
    }
}

pub fn stop_and_process(app: &AppHandle) {
    let state = app.state::<AppState>();
    {
        let mut status = state.status.lock().unwrap();
        if *status != Status::Recording {
            return;
        }
        *status = Status::Transcribing;
    }
    let handle = state.recorder.lock().unwrap().take();
    let Some(handle) = handle else {
        *state.status.lock().unwrap() = Status::Idle;
        hide_overlay(app);
        return;
    };
    let gen = state.generation.load(Ordering::SeqCst);
    emit_status(app, "transcribing", None);

    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = run_pipeline(app2.clone(), handle, gen).await;
        if let Err(e) = result {
            if !is_canceled(&app2, gen) {
                let state = app2.state::<AppState>();
                *state.status.lock().unwrap() = Status::Idle;
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
        .map_err(|e| format!("内部エラー(録音停止): {e}"))??;
    if is_canceled(&app, gen) {
        return Ok(());
    }
    if samples.is_empty() {
        return Err("音声を認識できませんでした".to_string());
    }
    let wav = crate::audio::wav_bytes(&samples)?;

    // 2. STT
    let raw_text = crate::stt::transcribe(&app, &settings, wav).await?;
    if is_canceled(&app, gen) {
        return Ok(());
    }
    let raw_text = raw_text.trim().to_string();
    if raw_text.is_empty() {
        return Err("音声を認識できませんでした".to_string());
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
                let state = app.state::<AppState>();
                *state.status.lock().unwrap() = Status::Polishing;
            }
            emit_status(&app, "polishing", None);
            match crate::llm::polish(&p, &mode.instruction, &raw_text).await {
                Ok(t) if !t.trim().is_empty() => final_text = t.trim().to_string(),
                Ok(_) => {
                    warning =
                        Some("AI整形の結果が空だったため原文を使用しました".to_string());
                }
                Err(e) => {
                    warning = Some(format!("AI整形に失敗したため原文を使用しました ({e})"));
                }
            }
        }
    }
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
        .map_err(|e| format!("内部エラー(貼り付け): {e}"))?;

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

    // 6. done -> idle
    {
        let state = app.state::<AppState>();
        *state.status.lock().unwrap() = Status::Idle;
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
pub fn error_flow(app: &AppHandle, message: String) {
    let state = app.state::<AppState>();
    *state.status.lock().unwrap() = Status::Idle;
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
