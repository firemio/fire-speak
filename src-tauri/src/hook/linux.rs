//! X11 exclusive passive-key-grab listener for single-modifier hotkeys
//! (RAlt / LAlt / RCtrl / LCtrl / RShift / LShift).
//!
//! A dedicated thread opens its own X11 connection and installs a core-X11
//! passive grab (`GrabKey`) on the root window for every keycode the hotkey
//! token resolves to. A passive grab makes the X server deliver the key to
//! THIS client only, so — exactly like the Windows `WH_KEYBOARD_LL` hook —
//! the hotkey never reaches the focused application. That is what makes a bare
//! Right Alt usable: without the grab a bare Alt press/release opens the menu
//! bar in GTK apps and Firefox.
//!
//! Grab parameters (all verified against the X11 protocol semantics):
//! - `owner_events = false`: events go to us, never to the focused window.
//! - `grab_window = root`: the grab is global, whatever has focus.
//! - `pointer_mode = ASYNC`, `keyboard_mode = ASYNC`. **Async keyboard mode is
//!   mandatory.** A `Sync` grab freezes event processing for the whole server
//!   until the grabbing client calls `AllowEvents`, so a slow or wedged
//!   fire-speak would freeze the user's keyboard system-wide. This project
//!   must never risk that.
//! - `modifiers = ModMask::ANY` first. The server treats `AnyModifier`
//!   atomically (BadAccess and no grab at all if ANY combination conflicts
//!   with another client), so on BadAccess we fall back to grabbing the
//!   modifier combinations that actually matter for a bare-modifier hotkey:
//!   none, Lock (CapsLock), Mod2 (NumLock) and Lock|Mod2 — a bare modifier is
//!   by definition pressed with no other modifier held, and grabbing
//!   Shift/Control/Mod1/Mod4 variants on top would additionally steal common
//!   window-manager and layout-switch chords (Alt+Shift, Super+Alt, ...) from
//!   other clients for no real benefit. Per-combination BadAccess is
//!   tolerated; if none of them can be grabbed, registration fails.
//!
//! ## The one behavioural difference from Windows
//!
//! **While the hotkey is physically held, other keys pressed during that hold
//! are consumed by this client instead of reaching the focused application.**
//! A passive grab that triggers becomes an *active* keyboard grab for the
//! duration of the hold, and the X server then routes ALL keyboard events to
//! the grabbing client. The Windows hook swallows only the hotkey itself.
//! This is accepted: the hotkey is a push-to-talk key, the user is speaking
//! rather than typing while it is down, and the alternative (no grab) leaks
//! a bare Alt into every GTK/Firefox menu bar. Keys pressed during a hold are
//! dropped here — see `on_press`, which ignores every keycode outside the
//! watched mask.
//!
//! ## Detectable auto-repeat
//!
//! With plain core-X11 auto-repeat a held key emits repeated KeyPress *and*
//! KeyRelease pairs, which would make hold-to-talk look like a rapid series of
//! taps and break the 400 ms tap-lock. The listener therefore enables
//! per-client **detectable auto-repeat** (XKB `PerClientFlags` with
//! `DETECTABLE_AUTO_REPEAT`) on its own connection before grabbing: repeats
//! then arrive as KeyPress only, with a single KeyRelease at the real release.
//! If XKB is unavailable the `LATCHED` press latch still stops a repeat from
//! re-triggering `hotkey_pressed`; note that all six supported tokens are
//! modifier keys, which X does not auto-repeat by default anyway.
//!
//! ## Suspension
//!
//! While the settings window's hotkey-capture field is focused
//! (`pipeline::hotkey_suspended()`), the grab is RELEASED so the key reaches
//! the webview normally and the user can re-assign it; it is re-established on
//! resume. `super::set_suspended` drives this.
//!
//! What is mirrored exactly from the Windows implementation:
//! - `LATCHED` is the sole press latch (the analogue of `SWALLOWED_VK`): a
//!   press is dispatched only on the not-held -> held transition.
//! - a press is ignored while suspended, but a release is dispatched
//!   UNCONDITIONALLY so `HOTKEY_HELD` can never go stale. Dropping the grab
//!   mid-hold also dispatches the release, in case the server ends the active
//!   grab along with the passive one.
//! - events feed the SAME single ordered dispatcher thread as the Windows hook
//!   and the global-shortcut plugin; the event loop never allocates per event
//!   and never spawns a thread.
//!
//! Keycodes are resolved from keysyms in the *current* keyboard mapping (never
//! hardcoded) and re-resolved (and re-grabbed) on `MappingNotify`.
//! If `DISPLAY` is unavailable (a pure Wayland session), `install()` returns
//! false and the app degrades to combo-only hotkeys.
//!
//! `RustConnection` is explicitly safe to use from several threads at once
//! (one blocked in `wait_for_event`, others sending requests and awaiting
//! replies), so grab/ungrab runs on the caller's thread against the listener's
//! connection; `GRAB` serialises those mutations.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{mpsc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::errors::ReplyError;
use x11rb::protocol::xkb;
use x11rb::protocol::xproto::{self, GrabMode, ModMask, Window};
use x11rb::protocol::{ErrorKind, Event};
use x11rb::rust_connection::RustConnection;

use crate::x11util;

/// Whether the listener thread is up and owns a usable X11 connection.
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// The listener's connection. Set once, before `install()` reports success.
static CONN: OnceLock<RustConnection> = OnceLock::new();
/// Root window the grabs are installed on.
static ROOT: AtomicU32 = AtomicU32::new(0);
/// Bitset of watched X11 keycodes (bit `k` = keycode `k`, 0..=255). Read from
/// the event loop to tell the hotkey apart from keys that arrive only because
/// an active grab is in progress. The four words are not written atomically as
/// a group; a torn read can at worst mis-classify a single keystroke during a
/// hotkey change.
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
/// Kept so the keycodes can be re-resolved after a keyboard-mapping change.
static WATCHED_TOKEN: AtomicU8 = AtomicU8::new(0);
/// Mirrors `pipeline::hotkey_suspended()` for the grab state machine.
static SUSPENDED: AtomicBool = AtomicBool::new(false);
/// Set while we are synthesising input ourselves (the Ctrl+V of a paste).
/// XTest input is delivered like real input and therefore triggers passive
/// grabs, so a Ctrl hotkey would otherwise swallow its own paste keystroke and
/// latch a phantom recording. Independent of `SUSPENDED`: both drop the grab.
static SYNTHETIC: AtomicBool = AtomicBool::new(false);

/// Everything the grab state machine owns. The single mutex serialises
/// `GrabKey`/`UngrabKey` against each other, whichever thread asks.
struct Grab {
    /// Keycodes the current hotkey resolves to; empty = no bare-modifier
    /// hotkey. Independent of whether a grab is established right now.
    desired: Vec<u8>,
    /// Keycodes actually grabbed and the modifier combinations they are
    /// grabbed with (a full cross product). Both empty = nothing grabbed.
    held_keys: Vec<u8>,
    held_mods: Vec<u16>,
}

impl Grab {
    const fn new() -> Self {
        Self {
            desired: Vec::new(),
            held_keys: Vec::new(),
            held_mods: Vec::new(),
        }
    }
}

static GRAB: Mutex<Grab> = Mutex::new(Grab::new());

/// Poisoning carries no risk here (the guarded data is plain vectors that are
/// always left consistent), so recover instead of propagating a panic.
fn lock_grab() -> MutexGuard<'static, Grab> {
    GRAB.lock().unwrap_or_else(|e| e.into_inner())
}

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

fn mask_of(keycodes: &[u8]) -> [u64; 4] {
    let mut bits = [0u64; 4];
    for &k in keycodes {
        bits[(k >> 6) as usize] |= 1u64 << (k & 63);
    }
    bits
}

fn store_mask(bits: [u64; 4]) {
    for (slot, value) in WATCHED_MASK.iter().zip(bits.iter()) {
        slot.store(*value, Ordering::SeqCst);
    }
}

fn token_index(token: &str) -> Option<u8> {
    super::SPECIAL_TOKENS
        .iter()
        .position(|t| *t == token)
        .map(|i| (i + 1) as u8)
}

/// Resolve `token` to the keycodes of its physical key in the server's current
/// mapping. `None` = the mapping could not be read; `Some(empty)` = the key is
/// not bound at all.
fn resolve_keycodes<C: Connection + ?Sized>(conn: &C, token: &str) -> Option<Vec<u8>> {
    let primary: &[u32] = match token {
        "RAlt" => &[x11util::KEYSYM_ALT_R],
        "LAlt" => &[x11util::KEYSYM_ALT_L],
        "RCtrl" => &[x11util::KEYSYM_CONTROL_R],
        "LCtrl" => &[x11util::KEYSYM_CONTROL_L],
        "RShift" => &[x11util::KEYSYM_SHIFT_R],
        "LShift" => &[x11util::KEYSYM_SHIFT_L],
        _ => return None,
    };
    let found = x11util::keycodes_for(conn, primary)?;
    if !found.is_empty() || token != "RAlt" {
        return Some(found);
    }
    // Only right Alt has a fallback: a layout that turns it into the
    // third-level chooser binds `ISO_Level3_Shift` there INSTEAD of `Alt_R`.
    // The fallback is deliberately last-resort — on a layout that keeps a
    // plain `Alt_R` and puts the level-3 shift on some other physical key,
    // grabbing that other key would steal AltGr typing.
    // (`locale::layout_uses_altgr` refuses RAlt on AltGr layouts anyway; this
    // only covers a mid-session layout switch.)
    x11util::keycodes_for(conn, &[x11util::KEYSYM_ISO_LEVEL3_SHIFT])
}

// ---------------------------------------------------------------------------
// Grab state machine
// ---------------------------------------------------------------------------

/// Modifier combinations used when `ModMask::ANY` is refused: the bare key and
/// the same key with the two lock modifiers the user cannot avoid holding
/// (CapsLock = Lock, NumLock = Mod2 on every standard layout).
fn fallback_mods() -> [ModMask; 4] {
    [
        ModMask::default(),
        ModMask::LOCK,
        ModMask::M2,
        ModMask::LOCK | ModMask::M2,
    ]
}

fn is_access_error(e: &ReplyError) -> bool {
    matches!(e, ReplyError::X11Error(err) if matches!(err.error_kind, ErrorKind::Access))
}

/// One `GrabKey`, checked synchronously so a BadAccess surfaces here instead
/// of turning up later as a stray error event in the listener loop.
fn grab_one(conn: &RustConnection, root: Window, key: u8, mods: ModMask) -> Result<(), ReplyError> {
    xproto::grab_key(
        conn,
        false, // owner_events: deliver to us only, never to the focused window
        root,
        mods,
        key,
        GrabMode::ASYNC,
        GrabMode::ASYNC, // never Sync: a Sync grab could freeze the keyboard
    )
    .map_err(ReplyError::ConnectionError)?
    .check()
}

fn ungrab_one(conn: &RustConnection, root: Window, key: u8, mods: ModMask) {
    // UngrabKey on a combination that is not grabbed is a no-op, so the
    // cross-product form below is safe; errors are irrelevant either way.
    if let Ok(cookie) = xproto::ungrab_key(conn, key, root, mods) {
        cookie.ignore_error();
    }
}

/// Grab `keycodes` for every combination in `mods`.
///
/// `all_or_nothing` mirrors how the server treats `AnyModifier`: one failure
/// rolls everything back and returns empty. Otherwise a failing combination is
/// rolled back on its own and the rest are kept.
///
/// Returns the combinations actually held (empty = nothing grabbed AND nothing
/// left behind).
fn grab_combos(
    conn: &RustConnection,
    root: Window,
    keycodes: &[u8],
    mods: &[ModMask],
    all_or_nothing: bool,
) -> Vec<u16> {
    let mut held: Vec<u16> = Vec::new();
    for &m in mods {
        let mut done: Vec<u8> = Vec::new();
        let mut ok = true;
        for &key in keycodes {
            match grab_one(conn, root, key, m) {
                Ok(()) => done.push(key),
                Err(e) => {
                    if is_access_error(&e) {
                        eprintln!(
                            "x11 GrabKey refused (already grabbed by another client): keycode {key} mods {:#x}",
                            u16::from(m)
                        );
                    } else {
                        eprintln!("x11 GrabKey failed for keycode {key}: {e}");
                    }
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            held.push(u16::from(m));
            continue;
        }
        // Roll this combination back so no partial grab survives.
        for &key in &done {
            ungrab_one(conn, root, key, m);
        }
        if all_or_nothing {
            for &hm in &held {
                for &key in keycodes {
                    ungrab_one(conn, root, key, ModMask::from(hm));
                }
            }
            let _ = conn.flush();
            return Vec::new();
        }
    }
    let _ = conn.flush();
    held
}

/// Establish the exclusive grab. `None` = nothing could be grabbed and nothing
/// was left behind.
fn establish(conn: &RustConnection, root: Window, keycodes: &[u8]) -> Option<Vec<u16>> {
    let any = grab_combos(conn, root, keycodes, &[ModMask::ANY], true);
    if !any.is_empty() {
        return Some(any);
    }
    eprintln!("x11: GrabKey with AnyModifier refused, falling back to explicit lock combinations");
    let held = grab_combos(conn, root, keycodes, &fallback_mods(), false);
    (!held.is_empty()).then_some(held)
}

/// Drop whatever is currently grabbed. If the hotkey was held at that moment,
/// end the hold: `UngrabKey` is not specified to cancel the active grab it
/// spawned, but we must not depend on that — a missing release would leave
/// `HOTKEY_HELD` stale and a hold-mode recording running forever.
fn release(conn: &RustConnection, root: Window, grab: &mut Grab) {
    if grab.held_keys.is_empty() {
        return;
    }
    for &key in &grab.held_keys {
        for &m in &grab.held_mods {
            ungrab_one(conn, root, key, ModMask::from(m));
        }
    }
    grab.held_keys.clear();
    grab.held_mods.clear();
    let _ = conn.flush();
    if LATCHED.swap(0, Ordering::SeqCst) != 0 {
        super::dispatch(false);
    }
}

/// Bring the held grab in line with `grab.desired` and the suspension flag.
/// The ONLY place that calls `GrabKey`/`UngrabKey`. Returns whether the
/// intended state was reached.
fn reconcile(grab: &mut Grab) -> bool {
    let Some(conn) = CONN.get() else {
        // No connection: the only reachable state is "nothing grabbed".
        return grab.desired.is_empty();
    };
    let root = ROOT.load(Ordering::SeqCst);
    let want: Vec<u8> = if SUSPENDED.load(Ordering::SeqCst) || SYNTHETIC.load(Ordering::SeqCst) {
        Vec::new()
    } else {
        grab.desired.clone()
    };
    if grab.held_keys == want {
        return true;
    }
    release(conn, root, grab);
    if want.is_empty() {
        return true;
    }
    match establish(conn, root, &want) {
        Some(mods) => {
            grab.held_keys = want;
            grab.held_mods = mods;
            true
        }
        None => false,
    }
}

/// Point the listener at `token` (or disable it with `None`).
/// On failure nothing is changed and no partial grab is left behind, so the
/// previously watched key keeps working.
pub(super) fn set_watched_token(token: Option<&str>) -> bool {
    let Some(token) = token else {
        WATCHED_TOKEN.store(0, Ordering::SeqCst);
        store_mask([0; 4]);
        let mut grab = lock_grab();
        grab.desired.clear();
        reconcile(&mut grab);
        return true;
    };
    let Some(idx) = token_index(token) else {
        return false;
    };
    let Some(conn) = CONN.get() else {
        eprintln!("x11 hotkey listener is not running");
        return false;
    };
    let keycodes = match resolve_keycodes(conn, token) {
        Some(k) if !k.is_empty() => k,
        _ => {
            eprintln!("hotkey token {token} is not bound in the current keyboard mapping");
            return false;
        }
    };

    let mut grab = lock_grab();
    let previous_desired = std::mem::replace(&mut grab.desired, keycodes.clone());
    let previous_token = WATCHED_TOKEN.swap(idx, Ordering::SeqCst);
    let previous_mask = mask_of(&previous_desired);
    store_mask(mask_of(&keycodes));
    if reconcile(&mut grab) {
        return true;
    }
    // The server refused the grab: restore the previous watch completely.
    eprintln!("x11: could not grab {token} exclusively");
    grab.desired = previous_desired;
    WATCHED_TOKEN.store(previous_token, Ordering::SeqCst);
    store_mask(previous_mask);
    reconcile(&mut grab);
    false
}

/// Release the grab across our own synthetic Ctrl+V and take it back once the
/// events have settled. Called from `super::set_synthetic_input`.
pub(super) fn set_synthetic_input(synthetic: bool) {
    if SYNTHETIC.swap(synthetic, Ordering::SeqCst) == synthetic {
        return;
    }
    let mut grab = lock_grab();
    if !reconcile(&mut grab) {
        eprintln!("x11: hotkey grab could not be re-established after synthetic input");
    }
}

/// Release the grab while the settings window captures a new hotkey, and take
/// it back afterwards. Called from `super::set_suspended`.
pub(super) fn set_suspended(suspended: bool) {
    if SUSPENDED.swap(suspended, Ordering::SeqCst) == suspended {
        return;
    }
    let mut grab = lock_grab();
    if !reconcile(&mut grab) {
        eprintln!("x11: hotkey grab could not be re-established after unsuspend");
    }
}

// ---------------------------------------------------------------------------
// Listener
// ---------------------------------------------------------------------------

/// Ask the server for per-client detectable auto-repeat on this connection.
/// Returns whether it is actually in effect.
fn enable_detectable_autorepeat(conn: &RustConnection) -> bool {
    // XkbUseExtension must precede any other XKB request.
    match xkb::use_extension(conn, 1, 0).map(|c| c.reply()) {
        Ok(Ok(reply)) if reply.supported => {}
        _ => return false,
    }
    let flag = xkb::PerClientFlag::DETECTABLE_AUTO_REPEAT;
    let none = xkb::BoolCtrl::default();
    let cookie = match xkb::per_client_flags(
        conn,
        u16::from(xkb::ID::USE_CORE_KBD), // DeviceSpec is a plain u16
        flag,                             // change: which flags to touch
        flag,                             // value: set them
        none,                             // ctrls_to_change
        none,                             // auto_ctrls
        none,                             // auto_ctrls_values
    ) {
        Ok(cookie) => cookie,
        Err(_) => return false,
    };
    match cookie.reply() {
        Ok(reply) => u32::from(reply.value) & u32::from(flag) != 0,
        Err(_) => false,
    }
}

/// Connect, find the root window and turn on detectable auto-repeat.
fn setup_connection() -> Result<(RustConnection, Window), String> {
    let (conn, screen_num) = x11rb::connect(None).map_err(|e| e.to_string())?;
    let root = conn
        .setup()
        .roots
        .get(screen_num)
        .ok_or_else(|| "no such screen".to_string())?
        .root;
    if !enable_detectable_autorepeat(&conn) {
        eprintln!(
            "x11: detectable auto-repeat unavailable; relying on the press latch \
             (the six supported tokens are modifier keys, which X does not auto-repeat)"
        );
    }
    Ok((conn, root))
}

pub(super) fn install() -> bool {
    let (tx, rx) = mpsc::channel::<bool>();
    std::thread::spawn(move || {
        let (conn, root) = match setup_connection() {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!("x11 hotkey listener unavailable: {e}");
                let _ = tx.send(false);
                return;
            }
        };
        ROOT.store(root, Ordering::SeqCst);
        if CONN.set(conn).is_err() {
            // install() already ran; keep the first connection.
            let _ = tx.send(INSTALLED.load(Ordering::SeqCst));
            return;
        }
        let Some(conn) = CONN.get() else {
            let _ = tx.send(false);
            return;
        };
        INSTALLED.store(true, Ordering::SeqCst);
        let _ = tx.send(true);

        event_loop(conn);

        // Connection lost (X server gone / session ended): stop claiming the
        // listener works. Every grab died with the connection, so just forget
        // them, and never leave a hold-mode recording running.
        INSTALLED.store(false, Ordering::SeqCst);
        {
            let mut grab = lock_grab();
            grab.held_keys.clear();
            grab.held_mods.clear();
        }
        if LATCHED.swap(0, Ordering::SeqCst) != 0 {
            super::dispatch(false);
        }
    });
    rx.recv_timeout(Duration::from_secs(3)).unwrap_or(false)
}

/// Cheap, allocation-light loop: every branch is a few atomic ops plus, at
/// most, a non-blocking channel send. No per-event thread is spawned and no
/// per-event allocation happens (`MappingNotify` is not a per-keystroke event).
fn event_loop(conn: &RustConnection) {
    loop {
        match conn.wait_for_event() {
            Ok(Event::KeyPress(event)) => on_press(event.detail),
            Ok(Event::KeyRelease(event)) => on_release(event.detail),
            Ok(Event::MappingNotify(event)) => {
                // Broadcast to every client on a layout change; pointer
                // remappings are irrelevant here.
                if u8::from(event.request) != u8::from(xproto::Mapping::POINTER) {
                    refresh_after_mapping(conn);
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

/// Re-resolve the watched token against the new mapping and re-grab if the
/// physical key moved.
fn refresh_after_mapping(conn: &RustConnection) {
    let idx = WATCHED_TOKEN.load(Ordering::SeqCst);
    if idx == 0 {
        return;
    }
    let Some(token) = super::SPECIAL_TOKENS.get((idx - 1) as usize) else {
        return;
    };
    // Keep the previous resolution when the new mapping reports the key
    // nowhere, so a transient mapping glitch cannot silently disable the
    // hotkey.
    let Some(keycodes) = resolve_keycodes(conn, token).filter(|k| !k.is_empty()) else {
        return;
    };
    let mut grab = lock_grab();
    if grab.desired == keycodes {
        return; // the physical key did not move; keep the grab as it is
    }
    store_mask(mask_of(&keycodes));
    grab.desired = keycodes;
    if !reconcile(&mut grab) {
        eprintln!("x11: hotkey grab lost after a keyboard-mapping change");
    }
}

fn on_press(keycode: u8) {
    let keycode = u32::from(keycode);
    // While an active grab is in progress the server routes EVERY key to this
    // client; anything that is not the hotkey is dropped here.
    if !mask_contains(keycode) {
        return;
    }
    // Sole press latch: a non-zero latch means a repeat of the held hotkey (if
    // detectable auto-repeat could not be enabled) or a second watched keycode
    // while one is held. Both are ignored, mirroring the Windows
    // `swallowed == 0` requirement.
    if LATCHED.load(Ordering::SeqCst) != 0 {
        return;
    }
    if crate::pipeline::hotkey_suspended() {
        return;
    }
    LATCHED.store(keycode, Ordering::SeqCst);
    super::dispatch(true);
}

fn on_release(keycode: u8) {
    let keycode = u32::from(keycode);
    if LATCHED.load(Ordering::SeqCst) == keycode {
        LATCHED.store(0, Ordering::SeqCst);
        // Dispatched even while suspended — pipeline::hotkey_released handles
        // the suspended case safely and clears HOTKEY_HELD.
        super::dispatch(false);
    }
}
