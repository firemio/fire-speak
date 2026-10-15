//! Context hints for LLM polishing (v0.11): the user's clipboard history and
//! the last few dictations. The model uses them only to choose between
//! candidate words — homophones (例: 機会/機械, 公演/講演), proper nouns,
//! spelling conventions — never as content to output.
//!
//! Clipboard history (v0.11.1): a watcher thread records each new clipboard
//! text while the app runs. It lives in memory only (never written to disk)
//! and is cleared when `llm_context` is turned off. Our own paste / restore
//! is not recorded (`mark_own_write`).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::Manager;

/// Recent dictations older than this are about something else.
const RECENT_WINDOW: Duration = Duration::from_secs(30 * 60);
const RECENT_MAX: usize = 3;
const RECENT_MAX_CHARS: usize = 600;

const CLIP_WINDOW: Duration = Duration::from_secs(60 * 60);
const CLIP_MAX: usize = 5;
const CLIP_ITEM_MAX_CHARS: usize = 800;
/// Total budget across clipboard items (newest kept first).
const CLIP_TOTAL_CHARS: usize = 3000;
/// Entries kept in memory (more than CLIP_MAX so filtering still leaves enough).
const CLIP_KEEP: usize = 10;
const POLL: Duration = Duration::from_millis(1000);

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Context {
    /// Oldest first.
    pub clipboard: Vec<String>,
    /// Oldest first.
    pub recent: Vec<String>,
}

impl Context {
    pub fn is_empty(&self) -> bool {
        self.clipboard.is_empty() && self.recent.is_empty()
    }

    /// System-prompt section, or "" when there is nothing to add.
    pub fn prompt_section(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        let mut s = String::from(
            "\n\n# 参考文脈\n以下はユーザーが直前に扱っていたテキストです(古い順)。口述テキストの語の選択(同音異義語・固有名詞・専門用語・表記ゆれ)を判断する手がかりにだけ使ってください。\n- <clipboard_history> はユーザーが実際にコピーした正確なテキストです。口述テキスト中の語と読み・音が同じか近い語がここにあれば、その表記を最優先で採用してください。\n- <recent_dictations> は過去の音声入力の結果で誤認識を含み得るため、話題の把握の補助としてだけ使ってください。\n関係が無ければ無視してください。参考文脈の内容を出力に含めたり、口述内容を補ったりしないでください。",
        );
        if !self.clipboard.is_empty() {
            s.push_str("\n\n<clipboard_history>");
            for c in &self.clipboard {
                s.push_str("\n<item>");
                s.push_str(c);
                s.push_str("</item>");
            }
            s.push_str("\n</clipboard_history>");
        }
        if !self.recent.is_empty() {
            s.push_str("\n\n<recent_dictations>");
            for r in &self.recent {
                s.push_str("\n<item>");
                s.push_str(r);
                s.push_str("</item>");
            }
            s.push_str("\n</recent_dictations>");
        }
        s
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClipEntry {
    /// unix ms of when the text was first seen
    pub timestamp: u64,
    pub text: String,
}

/// Newest first.
static CLIP_HISTORY: Mutex<VecDeque<ClipEntry>> = Mutex::new(VecDeque::new());
static OWN_WRITE: AtomicBool = AtomicBool::new(false);

/// Called after we set / restore the clipboard ourselves: the watcher adopts
/// the current content as "seen" without recording it.
pub fn mark_own_write() {
    OWN_WRITE.store(true, Ordering::SeqCst);
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Insert `text` as the newest entry (an existing identical entry moves up).
fn record(list: &mut VecDeque<ClipEntry>, text: &str, now: u64) {
    let text = text.trim();
    if text.is_empty() || looks_secret(text) {
        return;
    }
    list.retain(|e| e.text != text);
    list.push_front(ClipEntry { timestamp: now, text: text.to_string() });
    list.truncate(CLIP_KEEP);
}

/// Start the clipboard watcher (once, at startup).
pub fn start_watcher(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let mut last: Option<String> = None;
        #[cfg(windows)]
        let mut last_seq: u32 = 0;
        loop {
            std::thread::sleep(POLL);
            let enabled = app
                .state::<crate::AppState>()
                .settings
                .lock()
                .map(|s| s.llm_context)
                .unwrap_or(false);
            if !enabled {
                if let Ok(mut h) = CLIP_HISTORY.lock() {
                    h.clear();
                }
                last = None;
                continue;
            }
            // Cheap change check: skip opening the clipboard when nothing changed.
            #[cfg(windows)]
            {
                let seq = unsafe {
                    windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber()
                };
                if seq == last_seq && !OWN_WRITE.load(Ordering::SeqCst) {
                    continue;
                }
                last_seq = seq;
            }
            let current = crate::paste::read_text();
            if OWN_WRITE.swap(false, Ordering::SeqCst) {
                last = current;
                continue;
            }
            if current.is_some() && current != last {
                if let (Some(t), Ok(mut h)) = (current.as_deref(), CLIP_HISTORY.lock()) {
                    record(&mut h, t, now_ms());
                }
            }
            last = current;
        }
    });
}

/// Snapshot of the clipboard history, newest first, with `current` (read just
/// now, before our paste) merged in so a copy made <1 s ago is not missed.
pub fn clipboard_snapshot(current: Option<String>) -> Vec<ClipEntry> {
    let mut list = CLIP_HISTORY.lock().map(|h| h.clone()).unwrap_or_default();
    if let Some(c) = current {
        if !list.iter().any(|e| e.text == c.trim()) {
            record(&mut list, &c, now_ms());
        }
    }
    list.into()
}

/// Build the context. `clipboard` is newest first (see `clipboard_snapshot`).
pub fn build(
    history: &[crate::history::HistoryEntry],
    now_ms: u64,
    clipboard: &[ClipEntry],
) -> Context {
    let window_ms = RECENT_WINDOW.as_millis() as u64;
    let mut recent: Vec<String> = history
        .iter() // newest first
        .take_while(|h| now_ms.saturating_sub(h.timestamp) <= window_ms)
        .map(|h| h.final_text.trim())
        .filter(|t| !t.is_empty())
        .take(RECENT_MAX)
        .map(|t| tail(t, RECENT_MAX_CHARS))
        .collect();
    recent.reverse();

    // Our own outputs can still land in the clipboard (copy-only mode,
    // restore off); they are already in `recent` or stale.
    let own: Vec<&str> = history.iter().take(20).map(|h| h.final_text.trim()).collect();
    let clip_window_ms = CLIP_WINDOW.as_millis() as u64;
    let mut budget = CLIP_TOTAL_CHARS;
    let mut clip: Vec<String> = Vec::new();
    for e in clipboard
        .iter()
        .filter(|e| now_ms.saturating_sub(e.timestamp) <= clip_window_ms)
        .filter(|e| !e.text.trim().is_empty() && !looks_secret(e.text.trim()))
        .filter(|e| !own.contains(&e.text.trim()))
    {
        if clip.len() >= CLIP_MAX || budget == 0 {
            break;
        }
        let t = tail(e.text.trim(), CLIP_ITEM_MAX_CHARS.min(budget));
        budget -= t.chars().count();
        clip.push(t);
    }
    clip.reverse();

    Context { clipboard: clip, recent }
}

/// A single whitespace-free ASCII run mixing letters and digits/symbols is
/// most likely a password, API key or token: never forward it to an LLM.
fn looks_secret(s: &str) -> bool {
    s.len() >= 12
        && s.is_ascii()
        && !s.chars().any(char::is_whitespace)
        && s.chars().any(|c| c.is_ascii_alphabetic())
        && s.chars().any(|c| !c.is_ascii_alphabetic())
        && !s.contains("://")
}

/// Last `max` chars (the end of a long text is closest to what is being written).
fn tail(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        s.to_string()
    } else {
        s.chars().skip(n - max).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::HistoryEntry;

    fn entry(ts: u64, text: &str) -> HistoryEntry {
        HistoryEntry {
            id: String::new(),
            timestamp: ts,
            mode_id: "polish".into(),
            raw_text: text.into(),
            final_text: text.into(),
        }
    }

    fn clip(ts: u64, text: &str) -> ClipEntry {
        ClipEntry { timestamp: ts, text: text.into() }
    }

    #[test]
    fn recent_window_and_order() {
        let now = 100 * 60 * 1000;
        let h = vec![
            entry(now - 1_000, "c"),
            entry(now - 2_000, "b"),
            entry(now - 3_000, "a"),
            entry(now - 4_000, "too many"),
        ];
        assert_eq!(build(&h, now, &[]).recent, vec!["a", "b", "c"]);
        let old = vec![entry(now - 31 * 60 * 1000, "stale")];
        assert!(build(&old, now, &[]).recent.is_empty());
    }

    #[test]
    fn clipboard_history_filters_and_order() {
        let now = 200 * 60 * 1000;
        let h = vec![entry(now, "前回の出力")];
        let c = vec![
            clip(now - 1_000, "前回の出力"),
            clip(now - 2_000, "講演会の資料"),
            clip(now - 3_000, "sk-ant-api03-AbCd1234xyz"),
            clip(now - 4_000, "https://example.com/path"),
            clip(now - 61 * 60 * 1000, "古すぎる"),
        ];
        assert_eq!(
            build(&h, now, &c).clipboard,
            vec!["https://example.com/path", "講演会の資料"]
        );
    }

    #[test]
    fn clipboard_limits() {
        let now = 200 * 60 * 1000;
        let c: Vec<ClipEntry> = (0..8).map(|i| clip(now - i, &format!("item{i} あ"))).collect();
        let got = build(&[], now, &c).clipboard;
        assert_eq!(got.len(), CLIP_MAX);
        assert_eq!(got.last().map(String::as_str), Some("item0 あ"));
        let big: Vec<ClipEntry> = (0..5).map(|i| clip(now - i, &"字".repeat(1000))).collect();
        let total: usize = build(&[], now, &big).clipboard.iter().map(|s| s.chars().count()).sum();
        assert_eq!(total, CLIP_TOTAL_CHARS);
    }

    #[test]
    fn record_dedupes_and_skips_secrets() {
        let mut l = VecDeque::new();
        record(&mut l, "a b", 1);
        record(&mut l, "c d", 2);
        record(&mut l, "a b", 3);
        record(&mut l, "hunter2hunter2", 4);
        record(&mut l, "  ", 5);
        let texts: Vec<&str> = l.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, vec!["a b", "c d"]);
    }

    #[test]
    fn prompt_section() {
        assert_eq!(Context::default().prompt_section(), "");
        let c = Context { clipboard: vec!["X".into(), "Z".into()], recent: vec!["Y".into()] };
        let s = c.prompt_section();
        assert!(s.contains("<clipboard_history>\n<item>X</item>\n<item>Z</item>\n</clipboard_history>"));
        assert!(s.contains("<item>Y</item>"));
    }
}
