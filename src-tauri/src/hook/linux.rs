//! X11/XInput2 listener for single-modifier hotkeys
//! (RAlt / LAlt / RCtrl / LCtrl / RShift / LShift).
//!
//! A dedicated thread opens its own X11 connection and selects
//! `XI_RawKeyPress | XI_RawKeyRelease` on the root window for
//! `XIAllMasterDevices`. Raw events are delivered regardless of focus and
//! WITHOUT grabbing the key, which is a deliberate trade-off:
//!
//! **The hotkey also reaches the focused application on Linux.** XInput2 raw
//! events cannot suppress input (only `XGrabKey`/`XIGrabDevice` can, and those
//! break AltGr, modifier chording and other clients), so the Windows "swallow"
//! behaviour has no Linux equivalent. Everything that exists on Windows purely
//! to keep swallowing consistent (`PASSED_MASK`, the AltGr fake-LCtrl
//! passthrough) is therefore unnecessary here.
//!
//! What IS mirrored exactly from the Windows implementation:
//! - `LATCHED` is the sole press latch (the analogue of `SWALLOWED_VK`): a
//!   press is dispatched only on the 0 -> keycode transition, so OS auto-repeat
//!   never re-fires `hotkey_pressed`.
//! - a press is ignored while `pipeline::hotkey_suspended()`, but the matching
//!   release is dispatched UNCONDITIONALLY so `HOTKEY_HELD` can never go stale.
//! - events feed the SAME single ordered dispatcher thread as the Windows hook
//!   and the global-shortcut plugin; the event loop never spawns a thread.
//!
//! Keycodes are resolved from keysyms in the *current* keyboard mapping (never
//! hardcoded) and re-resolved when the server broadcasts `MappingNotify`.
//! If `DISPLAY` is unavailable (a pure Wayland session), `install()` returns
//! false and the app degrades to combo-only hotkeys.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc;
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::xinput;
use x11rb::protocol::xproto;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

use crate::x11util;

/// Whether the XInput2 listener thread is up and selecting raw key events.
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// Bitset of watched X11 keycodes (bit `k` = keycode `k`, 0..=255).
/// Written only from `set_watched_token` / `MappingNotify` handling, read from
/// the event loop. The four words are not written atomically as a group; a torn
/// read can at worst mis-classify a single keystroke during a hotkey change.
static WATCHED_MASK: [AtomicU64; 4] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
/// The ONLY press latch: keycode whose press we dispatched. 0 = none
/// (0 is never a valid X11 keycode, so it is a safe sentinel).
static LATCHED: AtomicU32 = AtomicU32::new(0);
/// 1-based index into `super::SPECIAL_TOKENS` of the watched token; 0 = none.
/// Kept so the mask can be re-resolved after a keyboard-mapping change.
static WATCHED_TOKEN: AtomicU8 = AtomicU8::new(0);

pub(super) fn is_installed() -> bool {
    INSTALLED.load(Ordering::SeqCst)
}

// ---------------------------------------------------------------------------
// Watched-keycode mask
// ---------------------------------------------------------------------------

fn mask_contains(keycode: u32) -> bool {
    if keycode > 255 {
        return false;
    }
    WATCHED_MASK[(keycode >> 6) as usize].load(Ordering::SeqCst) & (1u64 << (keycode & 63)) != 0
}

fn store_mask(bits: [u64; 4]) {
    for (slot, value) in WATCHED_MASK.iter().zip(bits.iter()) {
        slot.store(*value, Ordering::SeqCst);
    }
}

/// Keysyms that identify a token's physical key in the current mapping.
/// `ISO_Level3_Shift` counts as RAlt: on AltGr layouts the right Alt key
/// carries that keysym instead of `Alt_R` (such layouts are refused earlier by
/// `locale::layout_uses_altgr`, this is belt and braces for a mid-session
/// layout switch).
fn keysyms_for(token: &str) -> Option<&'static [u32]> {
    Some(match token {
        "RAlt" => &[x11util::KEYSYM_ALT_R, x11util::KEYSYM_ISO_LEVEL3_SHIFT],
        "LAlt" => &[x11util::KEYSYM_ALT_L],
        "RCtrl" => &[x11util::KEYSYM_CONTROL_R],
        "LCtrl" => &[x11util::KEYSYM_CONTROL_L],
        "RShift" => &[x11util::KEYSYM_SHIFT_R],
        "LShift" => &[x11util::KEYSYM_SHIFT_L],
        _ => return None,
    })
}

fn token_index(token: &str) -> Option<u8> {
    super::SPECIAL_TOKENS
        .iter()
        .position(|t| *t == token)
        .map(|i| (i + 1) as u8)
}

/// Build the keycode bitset for `syms` from the server's current mapping.
fn resolve_mask<C: Connection + ?Sized>(conn: &C, syms: &[u32]) -> [u64; 4] {
    let mut bits = [0u64; 4];
    let min = conn.setup().min_keycode;
    let max = conn.setup().max_keycode;
    if max < min {
        return bits;
    }
    let count = max - min + 1;
    let reply = match xproto::get_keyboard_mapping(conn, min, count).map(|c| c.reply()) {
        Ok(Ok(reply)) => reply,
        Ok(Err(e)) => {
            eprintln!("x11 GetKeyboardMapping failed: {e}");
            return bits;
        }
        Err(e) => {
            eprintln!("x11 GetKeyboardMapping failed: {e}");
            return bits;
        }
    };
    let per = reply.keysyms_per_keycode as usize;
    if per == 0 {
        return bits;
    }
    for (i, group) in reply.keysyms.chunks(per).enumerate() {
        let keycode = min as usize + i;
        if keycode > 255 {
            break;
        }
        if group.iter().any(|s| syms.contains(s)) {
            bits[keycode >> 6] |= 1u64 << (keycode & 63);
        }
    }
    bits
}

/// Re-resolve the mask for the currently watched token (after MappingNotify).
fn refresh_mask<C: Connection + ?Sized>(conn: &C) {
    let idx = WATCHED_TOKEN.load(Ordering::SeqCst);
    if idx == 0 {
        store_mask([0; 4]);
        return;
    }
    let Some(token) = super::SPECIAL_TOKENS.get((idx - 1) as usize) else {
        return;
    };
    let Some(syms) = keysyms_for(token) else {
        return;
    };
    let bits = resolve_mask(conn, syms);
    // Keep the old mask if the new layout binds the key nowhere, so a
    // transient mapping glitch cannot silently disable the hotkey.
    if bits != [0u64; 4] {
        store_mask(bits);
    }
}

/// Point the listener at `token` (or disable it with `None`).
/// Resolution happens BEFORE any state change, so a failure leaves the
/// previously watched key working.
pub(super) fn set_watched_token(token: Option<&str>) -> bool {
    let Some(token) = token else {
        WATCHED_TOKEN.store(0, Ordering::SeqCst);
        store_mask([0; 4]);
        return true;
    };
    let (Some(idx), Some(syms)) = (token_index(token), keysyms_for(token)) else {
        return false;
    };
    // A short-lived side connection: the event loop owns its own and must not
    // be interrupted. This runs only on hotkey (re)registration.
    let conn = match x11rb::connect(None) {
        Ok((conn, _)) => conn,
        Err(e) => {
            eprintln!("x11 connect for hotkey mapping failed: {e}");
            return false;
        }
    };
    let bits = resolve_mask(&conn, syms);
    if bits == [0u64; 4] {
        eprintln!("hotkey token {token} is not bound in the current keyboard mapping");
        return false;
    }
    store_mask(bits);
    WATCHED_TOKEN.store(idx, Ordering::SeqCst);
    true
}

// ---------------------------------------------------------------------------
// Listener
// ---------------------------------------------------------------------------

/// Connect and select raw key events on the root window for all master
/// devices. Returns the connection the event loop then owns.
fn setup_connection() -> Result<RustConnection, String> {
    let (conn, screen_num) = x11rb::connect(None).map_err(|e| e.to_string())?;
    // Negotiating XI2 also caches the extension's major opcode, which the
    // connection needs to parse the incoming generic (XGE) raw key events.
    xinput::xi_query_version(&conn, 2, 0)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| format!("XInput2 unavailable: {e}"))?;
    let root = conn
        .setup()
        .roots
        .get(screen_num)
        .ok_or_else(|| "no such screen".to_string())?
        .root;
    let masks = [xinput::EventMask {
        deviceid: u16::from(xinput::Device::ALL_MASTER),
        mask: vec![xinput::XIEventMask::RAW_KEY_PRESS | xinput::XIEventMask::RAW_KEY_RELEASE],
    }];
    xinput::xi_select_events(&conn, root, &masks)
        .map_err(|e| e.to_string())?
        .check()
        .map_err(|e| format!("XISelectEvents failed: {e}"))?;
    Ok(conn)
}

pub(super) fn install() -> bool {
    let (tx, rx) = mpsc::channel::<bool>();
    std::thread::spawn(move || {
        let conn = match setup_connection() {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!("x11 hotkey listener unavailable: {e}");
                let _ = tx.send(false);
                return;
            }
        };
        INSTALLED.store(true, Ordering::SeqCst);
        let _ = tx.send(true);
        event_loop(&conn);
        // Connection lost (X server gone / session ended): stop claiming the
        // listener works, and never leave a hold-mode recording running.
        INSTALLED.store(false, Ordering::SeqCst);
        if LATCHED.swap(0, Ordering::SeqCst) != 0 {
            super::dispatch(false);
        }
    });
    rx.recv_timeout(Duration::from_secs(3)).unwrap_or(false)
}

/// Cheap, allocation-light loop: every branch is a few atomic ops plus, at
/// most, a non-blocking channel send. No per-event thread is spawned.
fn event_loop(conn: &RustConnection) {
    loop {
        match conn.wait_for_event() {
            Ok(Event::XinputRawKeyPress(event)) => on_press(event.detail, event.flags),
            Ok(Event::XinputRawKeyRelease(event)) => on_release(event.detail),
            Ok(Event::MappingNotify(event)) => {
                // Broadcast to every client on a layout change; pointer
                // remappings are irrelevant here.
                if u8::from(event.request) != u8::from(xproto::Mapping::POINTER) {
                    refresh_mask(conn);
                }
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("x11 hotkey listener stopped: {e}");
                return;
            }
        }
    }
}

fn on_press(keycode: u32, flags: xinput::KeyEventFlags) {
    // Explicit auto-repeat marker, when the server sets it on raw events.
    if u32::from(flags) & u32::from(xinput::KeyEventFlags::KEY_REPEAT) != 0 {
        return;
    }
    // Sole latch: a non-zero latch means either a repeat of the held hotkey or
    // a second key while one is held. Both are ignored (mirrors the Windows
    // `swallowed == 0` requirement).
    if LATCHED.load(Ordering::SeqCst) != 0 {
        return;
    }
    if !mask_contains(keycode) || crate::pipeline::hotkey_suspended() {
        return;
    }
    LATCHED.store(keycode, Ordering::SeqCst);
    super::dispatch(true);
}

fn on_release(keycode: u32) {
    if LATCHED.load(Ordering::SeqCst) == keycode {
        LATCHED.store(0, Ordering::SeqCst);
        // Dispatched even while suspended — pipeline::hotkey_released handles
        // the suspended case safely and clears HOTKEY_HELD.
        super::dispatch(false);
    }
}
