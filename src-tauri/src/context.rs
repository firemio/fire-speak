//! Context hints for LLM polishing (v0.11): the user's clipboard and the
//! last few dictations. The model uses them only to choose between
//! candidate words — homophones (例: 機会/機械, 公演/講演), proper nouns,
//! spelling conventions — never as content to output.

use std::time::Duration;

/// Recent dictations older than this are about something else.
const RECENT_WINDOW: Duration = Duration::from_secs(30 * 60);
const RECENT_MAX: usize = 3;
const RECENT_MAX_CHARS: usize = 600;
const CLIPBOARD_MAX_CHARS: usize = 1500;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Context {
    pub clipboard: Option<String>,
    /// Oldest first.
    pub recent: Vec<String>,
}

impl Context {
    pub fn is_empty(&self) -> bool {
        self.clipboard.is_none() && self.recent.is_empty()
    }

    /// System-prompt section, or "" when there is nothing to add.
    pub fn prompt_section(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        let mut s = String::from(
            "\n\n# 参考文脈\n以下はユーザーが直前に扱っていたテキストです。口述テキストの語の選択(同音異義語・固有名詞・専門用語・表記ゆれ)を判断する手がかりにだけ使ってください。文脈と話題が合う語を優先し、関係が無ければ無視してください。参考文脈の内容を出力に含めたり、口述内容を補ったりしないでください。",
        );
        if !self.recent.is_empty() {
            s.push_str("\n\n<recent_dictations>");
            for r in &self.recent {
                s.push_str("\n- ");
                s.push_str(r);
            }
            s.push_str("\n</recent_dictations>");
        }
        if let Some(c) = &self.clipboard {
            s.push_str("\n\n<clipboard>\n");
            s.push_str(c);
            s.push_str("\n</clipboard>");
        }
        s
    }
}

/// Gather context. `clipboard` is the raw clipboard text (read by the caller
/// before our own paste overwrites it).
pub fn build(
    history: &[crate::history::HistoryEntry],
    now_ms: u64,
    clipboard: Option<String>,
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

    let clipboard = clipboard
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty() && !looks_secret(c))
        // With restore off, the clipboard still holds our last output.
        .filter(|c| !history.first().is_some_and(|h| h.final_text.trim() == c))
        .map(|c| tail(&c, CLIPBOARD_MAX_CHARS));

    Context { clipboard, recent }
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

    #[test]
    fn recent_window_and_order() {
        let now = 100 * 60 * 1000;
        let h = vec![
            entry(now - 1_000, "c"),
            entry(now - 2_000, "b"),
            entry(now - 3_000, "a"),
            entry(now - 4_000, "too many"),
        ];
        assert_eq!(build(&h, now, None).recent, vec!["a", "b", "c"]);
        let old = vec![entry(now - 31 * 60 * 1000, "stale")];
        assert!(build(&old, now, None).recent.is_empty());
    }

    #[test]
    fn clipboard_filters() {
        let now = 1_000_000;
        let h = vec![entry(now, "前回の出力")];
        assert_eq!(build(&h, now, Some("前回の出力".into())).clipboard, None);
        assert_eq!(build(&[], now, Some("sk-ant-api03-AbCd1234xyz".into())).clipboard, None);
        assert_eq!(build(&[], now, Some("   ".into())).clipboard, None);
        assert_eq!(
            build(&[], now, Some("https://example.com/path".into())).clipboard.as_deref(),
            Some("https://example.com/path")
        );
        assert_eq!(
            build(&[], now, Some("講演会の資料".into())).clipboard.as_deref(),
            Some("講演会の資料")
        );
    }

    #[test]
    fn prompt_section() {
        assert_eq!(Context::default().prompt_section(), "");
        let c = Context { clipboard: Some("X".into()), recent: vec!["Y".into()] };
        let s = c.prompt_section();
        assert!(s.contains("<clipboard>\nX\n</clipboard>"));
        assert!(s.contains("- Y"));
    }
}
