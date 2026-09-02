use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::Emitter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    /// unix time in milliseconds
    pub timestamp: u64,
    pub mode_id: String,
    pub raw_text: String,
    pub final_text: String,
}

fn history_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(crate::settings::config_dir(app)?.join("history.json"))
}

pub fn load(app: &tauri::AppHandle) -> Vec<HistoryEntry> {
    let path = match history_path(app) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(list) = serde_json::from_str::<Vec<HistoryEntry>>(&text) {
            return list;
        }
    }
    Vec::new()
}

fn save(app: &tauri::AppHandle, list: &[HistoryEntry]) -> Result<(), String> {
    let path = history_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("履歴フォルダを作成できませんでした: {e}"))?;
    }
    let text = serde_json::to_string_pretty(list)
        .map_err(|e| format!("履歴のシリアライズに失敗しました: {e}"))?;
    std::fs::write(&path, text).map_err(|e| format!("履歴の保存に失敗しました: {e}"))
}

/// Prepend an entry (newest first), cap to `limit`, persist, and notify the UI.
pub fn add(
    app: &tauri::AppHandle,
    mode_id: String,
    raw_text: String,
    final_text: String,
    limit: usize,
) -> Result<(), String> {
    let entry = HistoryEntry {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        mode_id,
        raw_text,
        final_text,
    };
    let mut list = load(app);
    list.insert(0, entry);
    if limit > 0 && list.len() > limit {
        list.truncate(limit);
    }
    save(app, &list)?;
    let _ = app.emit("history-updated", ());
    Ok(())
}

pub fn clear(app: &tauri::AppHandle) -> Result<(), String> {
    save(app, &[])?;
    let _ = app.emit("history-updated", ());
    Ok(())
}
