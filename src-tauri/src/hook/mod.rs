//! Global listening for single-modifier hotkeys
//! (RAlt / LAlt / RCtrl / LCtrl / RShift / LShift).
//!
//! The `tauri-plugin-global-shortcut` plugin cannot bind a bare modifier, so
//! those six tokens are detected by a platform-specific listener instead:
//!
//! - Windows: a `WH_KEYBOARD_LL` low-level keyboard hook (see `windows.rs`),
//!   which can also *swallow* the key so other apps never see it.
//! - Linux/X11: XInput2 raw key events on the root window (see `linux.rs`).
//!   Raw events cannot suppress the key, so on Linux the hotkey ALSO reaches
//!   the focused application. That is accepted (no `XGrabKey`).
//!
//! This module owns everything that is platform independent: the token set and
//! the single long-lived dispatcher thread that both platform listeners AND
//! the global-shortcut plugin handler feed, so `pressed`/`released` ordering
//! is globally preserved and no per-event thread is ever spawned.

use std::sync::mpsc::Sender;
use std::sync::OnceLock;
use tauri::AppHandle;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as imp;

#[cfg(not(any(windows, target_os = "linux")))]
mod unsupported;
#[cfg(not(any(windows, target_os = "linux")))]
use unsupported as imp;

/// The closed set of bare-modifier hotkey tokens (SPEC v0.3).
pub const SPECIAL_TOKENS: [&str; 6] = ["RAlt", "LAlt", "RCtrl", "LCtrl", "RShift", "LShift"];

static APP: OnceLock<AppHandle> = OnceLock::new();
/// Sender into the single long-lived dispatcher thread (true = pressed).
static SENDER: OnceLock<Sender<bool>> = OnceLock::new();

/// Whether `token` is one of the six bare-modifier tokens (as opposed to a
/// plugin combo like `"Ctrl+Alt+Space"`). Platform independent.
pub fn is_special_token(token: &str) -> bool {
    SPECIAL_TOKENS.contains(&token)
}

/// Whether the platform listener is up. When false the app degrades to
/// combo-only hotkeys and `register_hotkey` rejects the special tokens with
/// `ERR_HOTKEY_REGISTER|{token}`.
pub fn is_installed() -> bool {
    imp::is_installed()
}

/// Point the listener at `token`, or disable the listener path with `None`
/// (a plugin combo is active instead).
///
/// Returns false when the token cannot be watched on this platform/session
/// (e.g. the keysym is not bound in the current X11 keyboard mapping); the
/// caller turns that into `ERR_HOTKEY_REGISTER|{token}`.
pub fn set_watched_token(token: Option<&str>) -> bool {
    match token {
        Some(t) if !is_special_token(t) => false,
        other => imp::set_watched_token(other),
    }
}

/// Install the platform listener once and start the single dispatcher thread.
/// Returns whether the listener is installed (idempotent; safe to call again).
pub fn install(app: AppHandle) -> bool {
    if APP.set(app).is_err() {
        return is_installed(); // already installed (or install failed) earlier
    }

    // Single long-lived dispatcher: receives press/release events in order
    // and runs the pipeline entry points off the listener thread.
    let (dtx, drx) = std::sync::mpsc::channel::<bool>();
    let _ = SENDER.set(dtx);
    std::thread::spawn(move || {
        while let Ok(pressed) = drx.recv() {
            if let Some(app) = APP.get() {
                if pressed {
                    crate::pipeline::hotkey_pressed(app);
                } else {
                    crate::pipeline::hotkey_released(app);
                }
            }
        }
    });

    imp::install()
}

/// Queue a press/release for the dispatcher thread (O(1), order-preserving).
/// Called from the platform listener's hot path, so it must never allocate,
/// lock or block.
pub(crate) fn dispatch(pressed: bool) {
    if let Some(tx) = SENDER.get() {
        let _ = tx.send(pressed);
    }
}

/// Queue a press/release from any source. The plugin (combo) path shares the
/// same dispatcher thread so pressed/released ordering is global; if the
/// dispatcher never started (install failed very early), fall back to a
/// one-off thread.
pub fn dispatch_event(app: &AppHandle, pressed: bool) {
    if let Some(tx) = SENDER.get() {
        let _ = tx.send(pressed);
    } else {
        let app = app.clone();
        std::thread::spawn(move || {
            if pressed {
                crate::pipeline::hotkey_pressed(&app);
            } else {
                crate::pipeline::hotkey_released(&app);
            }
        });
    }
}
