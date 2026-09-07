//! WH_KEYBOARD_LL low-level keyboard hook for single-modifier hotkeys
//! (RAlt / LAlt / RCtrl / LCtrl / RShift / LShift).
//!
//! The hook is installed once at app setup on a dedicated thread that runs a
//! GetMessageW pump. Which virtual key is watched lives in an AtomicU32
//! (0 = disabled), so switching between a special token and a plugin combo is
//! a single atomic store.
//!
//! The hook callback runs on the pump thread and delays EVERY keystroke in
//! the system while it executes, so it does only atomic loads/stores plus the
//! dispatch-thread spawn — no locks, no I/O.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::OnceLock;
use tauri::AppHandle;
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, SetWindowsHookExW, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG,
    WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

/// Virtual key currently watched by the hook; 0 = hook path disabled
/// (a plugin combo is active instead).
static WATCHED_VK: AtomicU32 = AtomicU32::new(0);
/// Whether SetWindowsHookExW succeeded (set once by the pump thread).
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// Physical down-state of the watched key (auto-repeat suppression: only the
/// up->down transition dispatches).
static KEY_DOWN: AtomicBool = AtomicBool::new(false);
/// VK whose keydown we swallowed; its matching keyup must be swallowed too,
/// even if the watched key or the suspend flag changed in between. 0 = none.
static SWALLOWED_VK: AtomicU32 = AtomicU32::new(0);
static APP: OnceLock<AppHandle> = OnceLock::new();

/// Map a special hotkey token to its virtual-key code.
pub fn token_to_vk(token: &str) -> Option<u32> {
    match token {
        "RAlt" => Some(0xA5),
        "LAlt" => Some(0xA4),
        "RCtrl" => Some(0xA3),
        "LCtrl" => Some(0xA2),
        "RShift" => Some(0xA1),
        "LShift" => Some(0xA0),
        _ => None,
    }
}

pub fn is_installed() -> bool {
    INSTALLED.load(Ordering::SeqCst)
}

/// Enable the hook path for `vk`, or disable it with 0.
pub fn set_watched_vk(vk: u32) {
    WATCHED_VK.store(vk, Ordering::SeqCst);
}

/// Install the hook once on a dedicated message-pump thread. Returns whether
/// the hook is installed (idempotent; safe to call again).
pub fn install(app: AppHandle) -> bool {
    if APP.set(app).is_err() {
        return is_installed(); // already installed (or install failed) earlier
    }
    let (tx, rx) = std::sync::mpsc::channel::<bool>();
    std::thread::spawn(move || unsafe {
        let hook =
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), std::ptr::null_mut(), 0);
        let ok = !hook.is_null();
        INSTALLED.store(ok, Ordering::SeqCst);
        let _ = tx.send(ok);
        if !ok {
            return;
        }
        // Message pump: required for the LL hook to receive events.
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
    });
    rx.recv_timeout(std::time::Duration::from_secs(3))
        .unwrap_or(false)
}

/// Run pipeline::hotkey_pressed/hotkey_released off the hook thread.
fn dispatch(pressed: bool) {
    if let Some(app) = APP.get() {
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

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let kb = &*(lparam as *const KBDLLHOOKSTRUCT);
        // Ignore synthetic input (enigo's Ctrl+V etc.).
        if kb.flags & LLKHF_INJECTED == 0 {
            let vk = kb.vkCode;
            let watched = WATCHED_VK.load(Ordering::SeqCst);
            match wparam as u32 {
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    if watched != 0
                        && vk == watched
                        && !crate::pipeline::hotkey_suspended()
                    {
                        if !KEY_DOWN.swap(true, Ordering::SeqCst) {
                            dispatch(true); // up->down transition only
                        }
                        SWALLOWED_VK.store(vk, Ordering::SeqCst);
                        return 1; // swallow (also swallows auto-repeats)
                    }
                    // suspended or not watched: pass through, no dispatch
                }
                WM_KEYUP | WM_SYSKEYUP => {
                    // A keyup matching a swallowed keydown is swallowed even
                    // if the watch/suspend state changed since the down.
                    let swallow = SWALLOWED_VK.load(Ordering::SeqCst) == vk;
                    if swallow {
                        SWALLOWED_VK.store(0, Ordering::SeqCst);
                    }
                    if watched != 0 && vk == watched {
                        KEY_DOWN.store(false, Ordering::SeqCst);
                        if !crate::pipeline::hotkey_suspended() {
                            dispatch(false);
                        }
                    }
                    if swallow {
                        return 1;
                    }
                }
                _ => {}
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}
