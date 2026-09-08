use std::time::Duration;

/// Copy `text` to the clipboard.
pub fn copy_only(text: &str) -> Result<(), String> {
    pin_clipboard_owner();
    let mut cb =
        arboard::Clipboard::new().map_err(|e| format!("ERR_CLIPBOARD|open: {e}"))?;
    cb.set_text(text.to_string())
        .map_err(|e| format!("ERR_CLIPBOARD|{e}"))
}

/// Paste `text` into the active application.
/// paste_mode == "paste": save old clipboard (text or image) -> set new ->
/// wait for physical modifiers to be released -> Ctrl+V -> optionally restore.
/// paste_mode == "clipboard": copy only.
pub fn paste_text(text: &str, paste_mode: &str, restore_clipboard: bool) -> Result<(), String> {
    pin_clipboard_owner();
    // `cb` is deliberately held for the whole function — through the paste AND
    // the restore. On X11 the clipboard has no server-side storage: its
    // contents are served on demand by the owning process, and dropping the
    // last arboard `Clipboard` tears the owner window down (handing off to a
    // clipboard manager, if one is even running). Creating it per step would
    // race that teardown against the Ctrl+V we are about to send.
    let mut cb =
        arboard::Clipboard::new().map_err(|e| format!("ERR_CLIPBOARD|open: {e}"))?;
    let old_text = cb.get_text().ok();
    let old_image = if old_text.is_none() {
        cb.get_image().ok()
    } else {
        None
    };
    cb.set_text(text.to_string())
        .map_err(|e| format!("ERR_CLIPBOARD|{e}"))?;

    if paste_mode == "paste" {
        std::thread::sleep(Duration::from_millis(60));
        // If the user is still physically holding modifiers (e.g. the hotkey
        // keys), Ctrl+V would turn into Ctrl+Alt+V etc. Wait for release.
        wait_for_modifier_release();
        // Our own Ctrl+V is synthetic but the X server delivers it like real
        // input, so a bare-Ctrl hotkey's exclusive grab would swallow it. Drop
        // the grab across the whole synthetic-input window and take it back
        // only after the events have settled. The flag is cleared before `?`
        // so a failed paste cannot leave the hotkey ungrabbed.
        crate::hook::set_synthetic_input(true);
        let sent = send_ctrl_v();
        std::thread::sleep(Duration::from_millis(900));
        crate::hook::set_synthetic_input(false);
        sent?;
        if restore_clipboard {
            // Only restore if the clipboard still contains exactly the text
            // we set; if the user or another app changed it, keep theirs.
            let still_ours = cb.get_text().map(|t| t == text).unwrap_or(false);
            if still_ours {
                if let Some(old) = old_text {
                    let _ = cb.set_text(old);
                } else if let Some(img) = old_image {
                    let _ = cb.set_image(img);
                }
                // clipboard held neither text nor image (e.g. files):
                // skip restore silently
            }
        }
    }
    Ok(())
}

/// Keep one arboard `Clipboard` alive for the whole process (Linux only).
///
/// arboard's X11 backend keeps a process-global connection plus a worker
/// thread that answers selection requests, and tears both down — destroying
/// the selection-owner window — when the LAST `Clipboard` handle is dropped.
/// Without a clipboard manager running, that silently empties the clipboard as
/// soon as `copy_only` / `paste_text` returns. Leaking exactly one handle pins
/// the owner for the app's lifetime, which is precisely the ownership model
/// X11 expects from a resident app; it is a bounded, one-time leak.
#[cfg(target_os = "linux")]
fn pin_clipboard_owner() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| match arboard::Clipboard::new() {
        Ok(cb) => std::mem::forget(cb),
        Err(e) => eprintln!("clipboard owner pin failed: {e}"),
    });
}

#[cfg(not(target_os = "linux"))]
fn pin_clipboard_owner() {}

/// Poll until Alt/Ctrl/Shift/Win are all physically released (30ms interval,
/// 2s cap). Proceeds after release, or unconditionally at the cap.
#[cfg(windows)]
fn wait_for_modifier_release() {
    use std::time::Instant;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let any_down = [VK_MENU, VK_CONTROL, VK_SHIFT, VK_LWIN, VK_RWIN]
            .iter()
            .any(|vk| (unsafe { GetAsyncKeyState(*vk as i32) } as u16 & 0x8000) != 0);
        if !any_down || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

/// X11 equivalent: poll `QueryPointer`'s modifier mask (which reports the
/// logical modifier state the server would apply to a synthetic key event)
/// until Shift/Ctrl/Alt/Super/AltGr are all up. Same 30ms interval and 2s cap
/// as the Windows path. Proceeds immediately when X11 is unreachable.
#[cfg(target_os = "linux")]
fn wait_for_modifier_release() {
    use std::time::Instant;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt, KeyButMask};

    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return;
    };
    let Some(root) = conn.setup().roots.get(screen_num).map(|s| s.root) else {
        return;
    };
    // Mod1 = Alt, Mod4 = Super/Win, Mod5 = AltGr (ISO_Level3_Shift) on the
    // conventional XKB modifier map. Lock (CapsLock) and Mod2 (NumLock) are
    // deliberately excluded: they latch and would never clear.
    let watched = u16::from(
        KeyButMask::SHIFT
            | KeyButMask::CONTROL
            | KeyButMask::MOD1
            | KeyButMask::MOD4
            | KeyButMask::MOD5,
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let any_down = match conn.query_pointer(root).map(|c| c.reply()) {
            Ok(Ok(reply)) => u16::from(reply.mask) & watched != 0,
            _ => return, // connection trouble: do not stall the paste
        };
        if !any_down || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn wait_for_modifier_release() {}

fn send_ctrl_v() -> Result<(), String> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings as EnigoSettings};
    let err = |e: enigo::InputError| format!("ERR_PASTE|{e}");
    let mut enigo = Enigo::new(&EnigoSettings::default())
        .map_err(|e| format!("ERR_PASTE|init: {e}"))?;
    enigo.key(Key::Control, Direction::Press).map_err(err)?;
    let r = enigo.key(Key::Unicode('v'), Direction::Click).map_err(err);
    // always release Ctrl even if 'v' failed
    let _ = enigo.key(Key::Control, Direction::Release);
    r
}
