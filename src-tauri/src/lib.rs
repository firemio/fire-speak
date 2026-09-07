mod audio;
mod history;
mod llm;
mod locale;
mod paste;
mod pipeline;
mod settings;
mod setup;
mod stt;
mod update;

use settings::Settings;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Mutex;
use tauri::menu::{CheckMenuItem, Menu, MenuBuilder, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, Wry};
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

const TRAY_ID: &str = "fire-speak-tray";

pub struct AppState {
    pub settings: Mutex<Settings>,
    pub status: Mutex<pipeline::Status>,
    pub recorder: Mutex<Option<audio::RecorderHandle>>,
    pub generation: AtomicU64,
    pub server: Mutex<Option<setup::ManagedServer>>,
    pub server_starting: AtomicBool,
    /// Error that occurred before the webview could listen (e.g. startup
    /// hotkey registration failure); drained once by `get_startup_error`.
    pub startup_error: Mutex<Option<String>>,
}

impl AppState {
    fn new(settings: Settings) -> Self {
        Self {
            settings: Mutex::new(settings),
            status: Mutex::new(pipeline::Status::Idle),
            recorder: Mutex::new(None),
            generation: AtomicU64::new(0),
            server: Mutex::new(None),
            server_starting: AtomicBool::new(false),
            startup_error: Mutex::new(None),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Register `hotkey` as the global shortcut.
///
/// The new hotkey string is parsed BEFORE touching the currently registered
/// shortcut, so an invalid string leaves the existing hotkey working. If
/// registration of the new (valid) hotkey fails (e.g. the combo is taken by
/// another app), we attempt to re-register `previous` before returning Err.
fn register_hotkey(app: &AppHandle, hotkey: &str, previous: Option<&str>) -> Result<(), String> {
    let shortcut: Shortcut = hotkey
        .parse()
        .map_err(|_| format!("ERR_HOTKEY_PARSE|{hotkey}"))?;
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    if let Err(e) = gs.register(shortcut) {
        eprintln!("hotkey register failed for {hotkey}: {e}");
        // best effort: restore the previous hotkey so the user keeps a working one
        if let Some(prev) = previous {
            if let Ok(prev_shortcut) = prev.parse::<Shortcut>() {
                let _ = gs.register(prev_shortcut);
            }
        }
        return Err(format!("ERR_HOTKEY_REGISTER|{hotkey}"));
    }
    Ok(())
}

fn apply_autostart(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let autolaunch = app.autolaunch();
    let current = autolaunch.is_enabled().unwrap_or(false);
    if current == enabled {
        return Ok(());
    }
    let result = if enabled {
        autolaunch.enable()
    } else {
        autolaunch.disable()
    };
    result.map_err(|e| format!("ERR_INTERNAL|autostart: {e}"))
}

fn build_tray_menu(app: &AppHandle, settings: &Settings) -> tauri::Result<Menu<Wry>> {
    let lang = if settings.ui_lang.trim().is_empty() {
        locale::resolve_ui_lang()
    } else {
        settings.ui_lang.clone()
    };
    let (open_settings_label, quit_label) = locale::tray_labels(&lang);
    let mut builder = MenuBuilder::new(app);
    for mode in &settings.modes {
        let item = CheckMenuItem::with_id(
            app,
            format!("mode::{}", mode.id),
            &mode.name,
            true,
            mode.id == settings.active_mode_id,
            None::<&str>,
        )?;
        builder = builder.item(&item);
    }
    builder = builder.separator();
    builder = builder.item(&MenuItem::with_id(
        app,
        "open-settings",
        open_settings_label,
        true,
        None::<&str>,
    )?);
    builder = builder.item(&MenuItem::with_id(
        app,
        "quit",
        quit_label,
        true,
        None::<&str>,
    )?);
    builder.build()
}

fn rebuild_tray_menu(app: &AppHandle, settings: &Settings) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Ok(menu) = build_tray_menu(app, settings) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

fn handle_menu_event(app: &AppHandle, id: &str) {
    match id {
        "open-settings" => show_main_window(app),
        "quit" => {
            setup::kill_server(app);
            app.exit(0);
        }
        other => {
            if let Some(mode_id) = other.strip_prefix("mode::") {
                if let Err(e) = do_set_active_mode(app, mode_id.to_string()) {
                    eprintln!("set_active_mode failed: {e}");
                }
            }
        }
    }
}

fn build_tray(app: &AppHandle, settings: &Settings) -> tauri::Result<()> {
    let menu = build_tray_menu(app, settings)?;
    let icon = app
        .default_window_icon()
        .cloned()
        .expect("default window icon missing");
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip("fire-speak")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| handle_menu_event(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

fn do_set_active_mode(app: &AppHandle, mode_id: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let new_settings = {
        let mut guard = state.settings.lock().unwrap();
        if !guard.modes.iter().any(|m| m.id == mode_id) {
            return Err(format!("ERR_INTERNAL|mode not found: {mode_id}"));
        }
        guard.active_mode_id = mode_id;
        guard.clone()
    };
    settings::save(app, &new_settings)?;
    rebuild_tray_menu(app, &new_settings);
    let _ = app.emit("settings-changed", &new_settings);
    Ok(())
}

fn apply_settings(app: &AppHandle, new_settings: Settings) -> Result<(), String> {
    let state = app.state::<AppState>();
    let old_settings = state.settings.lock().unwrap().clone();

    // Hotkey first: if the new hotkey cannot be registered, fail WITHOUT
    // persisting or replacing the in-memory settings (the old hotkey stays
    // registered, best effort).
    if old_settings.hotkey != new_settings.hotkey {
        register_hotkey(app, &new_settings.hotkey, Some(&old_settings.hotkey))?;
    }

    {
        let mut guard = state.settings.lock().unwrap();
        *guard = new_settings.clone();
    }
    settings::save(app, &new_settings)?;

    if old_settings.autostart != new_settings.autostart {
        apply_autostart(app, new_settings.autostart)?;
    }
    if old_settings.stt != new_settings.stt {
        // restart lazily on the next transcription
        setup::kill_server(app);
    }
    rebuild_tray_menu(app, &new_settings);
    let _ = app.emit("settings-changed", &new_settings);
    Ok(())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    Ok(state.settings.lock().unwrap().clone())
}

#[tauri::command]
async fn save_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    apply_settings(&app, settings)
}

#[tauri::command]
async fn toggle_recording(app: AppHandle) -> Result<(), String> {
    pipeline::toggle(&app);
    Ok(())
}

#[tauri::command]
async fn cancel_recording(app: AppHandle) -> Result<(), String> {
    pipeline::cancel(&app);
    Ok(())
}

#[tauri::command]
fn get_status(state: State<'_, AppState>) -> Result<String, String> {
    Ok(state.status.lock().unwrap().as_str().to_string())
}

#[tauri::command]
fn get_startup_error(state: State<'_, AppState>) -> Result<Option<String>, String> {
    Ok(state.startup_error.lock().unwrap().take())
}

#[tauri::command]
async fn set_active_mode(app: AppHandle, mode_id: String) -> Result<(), String> {
    do_set_active_mode(&app, mode_id)
}

#[tauri::command]
fn get_history(app: AppHandle) -> Result<Vec<history::HistoryEntry>, String> {
    Ok(history::load(&app))
}

#[tauri::command]
async fn clear_history(app: AppHandle) -> Result<(), String> {
    history::clear(&app)
}

#[tauri::command]
async fn copy_text(text: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || paste::copy_only(&text))
        .await
        .map_err(|e| format!("ERR_INTERNAL|{e}"))?
}

#[tauri::command]
async fn test_stt(app: AppHandle) -> Result<String, String> {
    let settings = app.state::<AppState>().settings.lock().unwrap().clone();
    if settings.stt.engine == "cloud" {
        // half a second of silence to verify credentials & endpoint
        let wav = audio::wav_bytes(&vec![0i16; 8000])?;
        stt::transcribe_cloud(&settings, wav).await?;
        Ok("OK_STT".to_string())
    } else {
        setup::ensure_server(app.clone()).await?;
        Ok("OK_STT".to_string())
    }
}

#[tauri::command]
async fn test_llm(app: AppHandle, provider_id: String) -> Result<String, String> {
    let provider = {
        let state = app.state::<AppState>();
        let guard = state.settings.lock().unwrap();
        guard
            .llm
            .providers
            .iter()
            .find(|p| p.id == provider_id)
            .cloned()
    };
    let provider =
        provider.ok_or_else(|| format!("ERR_INTERNAL|provider not found: {provider_id}"))?;
    llm::test(&provider).await
}

#[tauri::command]
fn setup_status(app: AppHandle) -> Result<setup::SetupStatus, String> {
    let settings = app.state::<AppState>().settings.lock().unwrap().clone();
    setup::get_setup_status(&app, &settings)
}

#[tauri::command]
async fn download_whisper_server(app: AppHandle) -> Result<(), String> {
    setup::download_whisper_server(app).await
}

#[tauri::command]
async fn download_model(app: AppHandle, model: String) -> Result<(), String> {
    setup::download_model(app, model).await
}

#[tauri::command]
fn open_config_dir(app: AppHandle) -> Result<(), String> {
    let dir = settings::config_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("ERR_FILE_IO|create config dir: {e}"))?;
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_path(dir.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| format!("ERR_OPEN_FOLDER|{e}"))
}

#[tauri::command]
async fn check_update(app: AppHandle) -> Result<update::UpdateInfo, String> {
    let (owner, repo) = {
        let state = app.state::<AppState>();
        let guard = state.settings.lock().unwrap();
        (guard.update.owner.clone(), guard.update.repo.clone())
    };
    let current = app.package_info().version.to_string();
    update::check(&owner, &repo, &current).await
}

#[tauri::command]
async fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err(format!("ERR_INTERNAL|only https:// URLs can be opened: {url}"));
    }
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_url(url, None::<&str>)
        .map_err(|e| format!("ERR_INTERNAL|open url: {e}"))
}

#[tauri::command]
fn quit_app(app: AppHandle) -> Result<(), String> {
    setup::kill_server(&app);
    app.exit(0);
    Ok(())
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        // Never run the pipeline inline: audio::start can block
                        // for up to 8s and would freeze the main thread.
                        let app = app.clone();
                        std::thread::spawn(move || pipeline::toggle(&app));
                    }
                })
                .build(),
        )
        .setup(|app| {
            let handle = app.handle().clone();
            let loaded = settings::load(&handle);
            app.manage(AppState::new(loaded.clone()));

            if let Err(e) = register_hotkey(&handle, &loaded.hotkey, None) {
                // non-fatal; the webview has no listener yet, so stash the
                // error for the frontend to drain via get_startup_error
                eprintln!("hotkey registration failed: {e}");
                let msg = format!("ERR_STARTUP_HOTKEY|{e}");
                *handle.state::<AppState>().startup_error.lock().unwrap() = Some(msg);
            }
            if let Err(e) = apply_autostart(&handle, loaded.autostart) {
                eprintln!("autostart sync failed: {e}");
            }
            build_tray(&handle, &loaded)?;

            // Launched by autostart with --hidden: start minimized to tray.
            if std::env::args().any(|a| a == "--hidden") {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            toggle_recording,
            cancel_recording,
            get_status,
            get_startup_error,
            set_active_mode,
            get_history,
            clear_history,
            copy_text,
            test_stt,
            test_llm,
            setup_status,
            download_whisper_server,
            download_model,
            open_config_dir,
            quit_app,
            check_update,
            open_url
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                setup::kill_server(app);
            }
        });
}
