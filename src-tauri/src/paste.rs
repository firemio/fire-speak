use std::time::Duration;

/// Copy `text` to the clipboard.
pub fn copy_only(text: &str) -> Result<(), String> {
    let mut cb = arboard::Clipboard::new()
        .map_err(|e| format!("クリップボードを開けませんでした: {e}"))?;
    cb.set_text(text.to_string())
        .map_err(|e| format!("クリップボードへのコピーに失敗しました: {e}"))
}

/// Paste `text` into the active application.
/// paste_mode == "paste": save old clipboard text -> set new -> Ctrl+V -> optionally restore.
/// paste_mode == "clipboard": copy only.
pub fn paste_text(text: &str, paste_mode: &str, restore_clipboard: bool) -> Result<(), String> {
    let mut cb = arboard::Clipboard::new()
        .map_err(|e| format!("クリップボードを開けませんでした: {e}"))?;
    let old_text = cb.get_text().ok();
    cb.set_text(text.to_string())
        .map_err(|e| format!("クリップボードへのコピーに失敗しました: {e}"))?;

    if paste_mode == "paste" {
        std::thread::sleep(Duration::from_millis(60));
        send_ctrl_v()?;
        std::thread::sleep(Duration::from_millis(400));
        if restore_clipboard {
            if let Some(old) = old_text {
                let _ = cb.set_text(old);
            }
        }
    }
    Ok(())
}

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
