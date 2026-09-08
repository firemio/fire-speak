//! Small shared X11 helpers (Linux only).
//!
//! Used by `hook::linux` (keycode lookup for bare-modifier hotkeys),
//! `locale::layout_uses_altgr` (AltGr detection) and `paste`
//! (physical-modifier release polling).

use x11rb::connection::Connection;
use x11rb::protocol::xproto;

// X11 keysyms (see <X11/keysymdef.h>). Never hardcode keycodes — they differ
// per layout; these keysyms are looked up in the server's current mapping.
pub const KEYSYM_SHIFT_L: u32 = 0xffe1;
pub const KEYSYM_SHIFT_R: u32 = 0xffe2;
pub const KEYSYM_CONTROL_L: u32 = 0xffe3;
pub const KEYSYM_CONTROL_R: u32 = 0xffe4;
pub const KEYSYM_ALT_L: u32 = 0xffe9;
pub const KEYSYM_ALT_R: u32 = 0xffea;
/// The AltGr keysym. A layout that binds it uses the right Alt key to type
/// characters, so RAlt cannot be a bare hotkey there.
pub const KEYSYM_ISO_LEVEL3_SHIFT: u32 = 0xfe03;

/// Whether any keycode in the server's current keyboard mapping is bound to
/// `keysym`. Returns false when X11 is unreachable (e.g. a pure Wayland
/// session) or the request fails.
pub fn keysym_is_bound(keysym: u32) -> Option<bool> {
    let (conn, _) = x11rb::connect(None).ok()?;
    let min = conn.setup().min_keycode;
    let max = conn.setup().max_keycode;
    if max < min {
        return None;
    }
    let reply = xproto::get_keyboard_mapping(&conn, min, max - min + 1)
        .ok()?
        .reply()
        .ok()?;
    Some(reply.keysyms.contains(&keysym))
}
