use std::time::Duration;

/// Copy `text` to the clipboard.
pub fn copy_only(text: &str) -> Result<(), String> {
    let mut cb = arboard::Clipboard::new()
        .map_err(|e| format!("クリップボードを開けませんでした: {e}"))?;
    cb.set_text(text.to_string())
        .map_err(|e| format!("クリップボードへのコピーに失敗しました: {e}"))
}

/// Paste `text` into the active application.
/// paste_mode == "paste": save old clipboard (text or image) -> set new ->
/// wait for physical modifiers to be released -> Ctrl+V -> optionally restore.
/// paste_mode == "clipboard": copy only.
pub fn paste_text(text: &str, paste_mode: &str, restore_clipboard: bool) -> Result<(), String> {
    let mut cb = arboard::Clipboard::new()
        .map_err(|e| format!("クリップボードを開けませんでした: {e}"))?;
    let old_text = cb.get_text().ok();
    let old_image = if old_text.is_none() {
        cb.get_image().ok()
    } else {
        None
    };
    cb.set_text(text.to_string())
        .map_err(|e| format!("クリップボードへのコピーに失敗しました: {e}"))?;

    if paste_mode == "paste" {
        std::thread::sleep(Duration::from_millis(60));
        // If the user is still physically holding modifiers (e.g. the hotkey
        // keys), Ctrl+V would turn into Ctrl+Alt+V etc. Wait for release.
        wait_for_modifier_release();
        send_ctrl_v()?;
        std::thread::sleep(Duration::from_millis(900));
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

#[cfg(not(windows))]
fn wait_for_modifier_release() {}

fn send_ctrl_v() -> Result<(), String> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings as EnigoSettings};
    let err = |e: enigo::InputError| format!("キー送信に失敗しました: {e}");
    let mut enigo = Enigo::new(&EnigoSettings::default())
        .map_err(|e| format!("キー送信の初期化に失敗しました: {e}"))?;
    enigo.key(Key::Control, Direction::Press).map_err(err)?;
    let r = enigo.key(Key::Unicode('v'), Direction::Click).map_err(err);
    // always release Ctrl even if 'v' failed
    let _ = enigo.key(Key::Control, Direction::Release);
    r
}
