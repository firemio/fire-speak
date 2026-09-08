//! WH_KEYBOARD_LL low-level keyboard hook for single-modifier hotkeys
//! (RAlt / LAlt / RCtrl / LCtrl / RShift / LShift).
//!
//! The hook is installed once at app setup on a dedicated thread that runs a
//! GetMessageW pump. Which virtual key is watched lives in an AtomicU32
//! (0 = disabled), so switching between a special token and a plugin combo is
//! a single atomic store.
//!
//! The hook callback runs on the pump thread and delays EVERY keystroke in
//! the system while it executes, so it does only atomic loads/stores plus a
//! non-blocking channel send to the single long-lived dispatcher thread —
//! no locks, no I/O, no per-event thread spawns (ordering is guaranteed by
//! the channel).
//!
//! State machine (SPEC v0.3.1): `SWALLOWED_VK` is the ONLY press latch.
//! - keydown of the latched VK: swallow unconditionally (typematic repeat,
//!   regardless of watched/suspend state).
//! - keydown of the watched VK while not suspended, not injected, and not an
//!   AltGr-generated sequence: latch, dispatch pressed, swallow.
//! - keyup of the latched VK: clear the latch, dispatch released
//!   UNCONDITIONALLY (pipeline::hotkey_released handles suspension safely),
//!   swallow. Every other event passes through — a passed-through down gets a
//!   passed-through up (symmetry).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, SetWindowsHookExW, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG,
    WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

const VK_LCONTROL: u32 = 0xA2;
const VK_RMENU: u32 = 0xA5;
/// Scan code of the fake LCtrl the keyboard driver synthesizes as part of an
/// AltGr press/release sequence.
const SC_ALTGR_FAKE_CTRL: u32 = 0x21D;

/// Virtual key currently watched by the hook; 0 = hook path disabled
/// (a plugin combo is active instead).
static WATCHED_VK: AtomicU32 = AtomicU32::new(0);
/// Whether SetWindowsHookExW succeeded (set once by the pump thread).
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// The ONLY press latch: VK whose keydown we swallowed; its typematic repeats
/// and its matching keyup are swallowed too, even if the watched key or the
/// suspend flag changed in between. 0 = none.
static SWALLOWED_VK: AtomicU32 = AtomicU32::new(0);
/// KBDLLHOOKSTRUCT.time of the most recent NON-injected AltGr fake-LCtrl
/// event (scanCode 0x21D). An RAlt event carrying the same timestamp is part
/// of an AltGr sequence and must pass through untouched.
static ALTGR_TIME: AtomicU32 = AtomicU32::new(0);
/// Bitmask (bit = vk - 0xA0) of modifier VKs (0xA0..=0xA5) whose non-injected
/// keydown we PASSED THROUGH (suspended, pre-watch, or otherwise unlatched).
/// A down that leaked to the OS must never have a later typematic repeat
/// latched (that would swallow the eventual keyup of a delivered down and
/// leave the modifier stuck system-wide); the bit is cleared on that key's up.
static PASSED_MASK: AtomicU32 = AtomicU32::new(0);

/// Bit for a hook-eligible modifier VK, if it is one.
fn modifier_bit(vk: u32) -> Option<u32> {
    (0xA0..=0xA5).contains(&vk).then(|| 1 << (vk - 0xA0))
}

/// Map a special hotkey token to its virtual-key code.
fn token_to_vk(token: &str) -> Option<u32> {
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

pub(super) fn is_installed() -> bool {
    INSTALLED.load(Ordering::SeqCst)
}

/// Enable the hook path for `token`, or disable it with `None`.
pub(super) fn set_watched_token(token: Option<&str>) -> bool {
    match token {
        None => {
            WATCHED_VK.store(0, Ordering::SeqCst);
            true
        }
        Some(t) => match token_to_vk(t) {
            Some(vk) => {
                WATCHED_VK.store(vk, Ordering::SeqCst);
                true
            }
            None => false,
        },
    }
}

/// Install the hook once on a dedicated message-pump thread.
pub(super) fn install() -> bool {
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

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let kb = &*(lparam as *const KBDLLHOOKSTRUCT);
        // Ignore synthetic input (enigo's Ctrl+V etc.).
        if kb.flags & LLKHF_INJECTED == 0 {
            let vk = kb.vkCode;
            // AltGr detection: the driver emits a fake LCtrl (scanCode 0x21D)
            // with the SAME timestamp as the RAlt event of the sequence. The
            // fake Ctrl itself must stay COMPLETELY outside the latch/repeat
            // logic: with hotkey LCtrl it would otherwise be latched (breaking
            // AltGr typing) or clear a genuine LCtrl latch on its keyup.
            if vk == VK_LCONTROL && kb.scanCode == SC_ALTGR_FAKE_CTRL {
                ALTGR_TIME.store(kb.time, Ordering::SeqCst);
                return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
            }
            let altgr_ralt =
                vk == VK_RMENU && kb.time == ALTGR_TIME.load(Ordering::SeqCst);
            match wparam as u32 {
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    let swallowed = SWALLOWED_VK.load(Ordering::SeqCst);
                    if swallowed == vk {
                        // Typematic repeat of the latched key: swallow
                        // unconditionally (regardless of watched/suspend).
                        return 1;
                    }
                    let bit = modifier_bit(vk);
                    let leaked = bit
                        .map(|b| PASSED_MASK.load(Ordering::SeqCst) & b != 0)
                        .unwrap_or(false);
                    let watched = WATCHED_VK.load(Ordering::SeqCst);
                    if watched != 0
                        && vk == watched
                        && swallowed == 0
                        && !leaked
                        && !crate::pipeline::hotkey_suspended()
                        && !altgr_ralt
                    {
                        SWALLOWED_VK.store(vk, Ordering::SeqCst);
                        super::dispatch(true);
                        return 1; // swallow
                    }
                    // Passed through: remember it so no later repeat of this
                    // press can be latched (suspend lifted mid-hold, or the
                    // watch switched onto an already-held key).
                    if let Some(b) = bit {
                        PASSED_MASK.fetch_or(b, Ordering::SeqCst);
                    }
                }
                WM_KEYUP | WM_SYSKEYUP => {
                    if let Some(b) = modifier_bit(vk) {
                        PASSED_MASK.fetch_and(!b, Ordering::SeqCst);
                    }
                    if SWALLOWED_VK.load(Ordering::SeqCst) == vk {
                        SWALLOWED_VK.store(0, Ordering::SeqCst);
                        // Released is dispatched UNCONDITIONALLY — even while
                        // suspended (pipeline::hotkey_released handles the
                        // suspended case safely) — so HOTKEY_HELD never goes
                        // stale and a hold-mode recording always ends.
                        super::dispatch(false);
                        return 1; // swallow (matches the swallowed down)
                    }
                    // pass through (its down was passed through too)
                }
                _ => {}
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}
