//! Small shared X11 helpers (Linux only).
//!
//! Used by `hook::linux` (keycode lookup for the bare-modifier hotkey grab),
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
/// The AltGr (third-level chooser) keysym. When the *right Alt key itself*
/// carries it, that key is needed to type characters and cannot be a bare
/// hotkey. It may equally well sit on some other physical key, which says
/// nothing about right Alt — see `right_alt_uses_level3_shift`.
pub const KEYSYM_ISO_LEVEL3_SHIFT: u32 = 0xfe03;

/// Every keycode in the server's current keyboard mapping that carries at
/// least one of `syms`, ascending.
///
/// `None` = the mapping could not be read; `Some(empty)` = none of `syms` is
/// bound. Callers need that distinction: a failed request must never be
/// mistaken for "the key does not exist".
pub fn keycodes_for<C: Connection + ?Sized>(conn: &C, syms: &[u32]) -> Option<Vec<u8>> {
    let min = conn.setup().min_keycode;
    let max = conn.setup().max_keycode;
    if max < min {
        return None;
    }
    let count = (max - min).saturating_add(1);
    let reply = match xproto::get_keyboard_mapping(conn, min, count).map(|c| c.reply()) {
        Ok(Ok(reply)) => reply,
        Ok(Err(e)) => {
            eprintln!("x11 GetKeyboardMapping failed: {e}");
            return None;
        }
        Err(e) => {
            eprintln!("x11 GetKeyboardMapping failed: {e}");
            return None;
        }
    };
    let per = reply.keysyms_per_keycode as usize;
    if per == 0 {
        return None;
    }
    let mut out = Vec::new();
    for (i, group) in reply.keysyms.chunks(per).enumerate() {
        let Ok(keycode) = u8::try_from(usize::from(min) + i) else {
            break;
        };
        if group.iter().any(|s| syms.contains(s)) {
            out.push(keycode);
        }
    }
    Some(out)
}

/// Whether the **right Alt physical key itself** carries `ISO_Level3_Shift` in
/// the server's current keyboard mapping — i.e. whether right Alt is the AltGr
/// key on this layout and is therefore needed for typing.
///
/// Deliberately NOT "is `ISO_Level3_Shift` bound anywhere": plenty of layouts
/// put a third-level chooser on another physical key (`lv3:switch`,
/// `lv3:menu_switch`, `lv3:lwin_switch`, ...) while right Alt stays a plain
/// `Alt_R`, and those users can use a bare right Alt hotkey perfectly well.
///
/// The right Alt key is the keycode bound to `Alt_R`. When the layout binds
/// `Alt_R` nowhere at all, right Alt is exactly the key that carries
/// `ISO_Level3_Shift` instead (that is what `lv3:ralt_switch` and the AltGr
/// layouts do), so a bound level-3 shift then means "yes".
///
/// `None` = X11 is unreachable or the mapping could not be read.
pub fn right_alt_uses_level3_shift() -> Option<bool> {
    let (conn, _) = x11rb::connect(None).ok()?;
    let alt_r = keycodes_for(&conn, &[KEYSYM_ALT_R])?;
    let level3 = keycodes_for(&conn, &[KEYSYM_ISO_LEVEL3_SHIFT])?;
    if alt_r.is_empty() {
        return Some(!level3.is_empty());
    }
    Some(alt_r.iter().any(|k| level3.contains(k)))
}
