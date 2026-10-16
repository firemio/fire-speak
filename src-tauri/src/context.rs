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
            "\n\n# 参考文脈\n以下はユーザーが直前に扱っていたテキストです(古い順)。口述テキストの語の選択(同音異義語・固有名詞・専門用語・表記ゆれ)を判断する手がかりにだけ使ってください。処理の対象ではありません。口述が「これ」「それ」などで参考文脈を指しているように見えても、要約・翻訳・引用などはしないでください。\n- <clipboard_history> はユーザーが実際にコピーした正確なテキストです。口述テキスト中の語と読み・音が同じか近い語がここにあれば、その表記を最優先で採用してください。\n- <recent_dictations> は過去の音声入力の結果で誤認識を含み得るため、話題の把握の補助としてだけ使ってください。\n関係が無ければ無視してください。参考文脈の内容を出力に含めたり、口述内容を補ったりしないでください。",
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

/// Polishing with context produced far more text than was dictated — the
/// model most likely summarized or quoted the context (a dictated "これ要約して"
/// came out as a summary of the recent dictations). The pipeline then polishes
/// again without context (v0.12.3).
pub fn looks_like_context_leak(raw: &str, polished: &str) -> bool {
    let raw_len = raw.trim().chars().count();
    polished.trim().chars().count() > raw_len * 2 + 20
}

/// Whisper keeps only the last ~224 tokens of a prompt, and a kanji can cost
/// 2-3 tokens; stay well inside that.
const STT_PROMPT_MAX_CHARS: usize = 100;
const STT_PROMPT_MAX_TERMS: usize = 30;
/// A transcript made of at least this many prompt terms and nothing else is
/// whisper reciting its prompt, not the user.
const ECHO_MIN_TERMS: usize = 3;

/// Vocabulary hint for the STT `prompt` (v0.11): terms from the clipboard.
#[derive(Debug, Clone, PartialEq)]
pub struct SttHint {
    pub prompt: String,
    pub terms: Vec<String>,
}

const ASCII_STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "this", "that", "from", "are", "was", "you", "your", "not",
    "but", "have", "has", "will", "can", "all", "its", "our", "out", "into", "then", "than",
];

fn is_term_cjk(c: char) -> bool {
    matches!(c as u32,
        0x30A1..=0x30FA | 0x30FC   // katakana + ー (・ U+30FB separates)
        | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
        | 0x3005)                  // 々
}

fn is_term_ascii(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+' | '#')
}

/// Proper-noun-ish terms of one text, in order: kanji/katakana runs of 2+
/// chars and ASCII words of 2+ chars with a letter (`Next.js`, `fire-speak`).
/// Hiragana is particles/okurigana, so it only separates. URLs, mail
/// addresses and secret-shaped tokens are dropped (Japanese text has no
/// spaces, so this is done per ASCII run, not per whitespace piece).
fn extract_terms(text: &str) -> Vec<String> {
    fn flush_cjk(cur: &mut String, out: &mut Vec<String>) {
        if cur.chars().count() >= 2 {
            out.push(cur.clone());
        }
        cur.clear();
    }
    fn flush_ascii(cur: &mut String, out: &mut Vec<String>) {
        if !(cur.contains("://") || cur.contains('@') || looks_secret(cur)) {
            for w in cur.split(|c: char| !is_term_ascii(c)) {
                let w = w.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '#' && c != '+');
                if w.len() >= 2
                    && w.chars().any(|c| c.is_ascii_alphabetic())
                    && !ASCII_STOPWORDS.contains(&w.to_ascii_lowercase().as_str())
                    && !looks_secret(w)
                {
                    out.push(w.to_string());
                }
            }
        }
        cur.clear();
    }
    let mut out = Vec::new();
    let (mut cjk, mut ascii) = (String::new(), String::new());
    for c in text.chars() {
        if is_term_cjk(c) {
            flush_ascii(&mut ascii, &mut out);
            cjk.push(c);
        } else if c.is_ascii_graphic() {
            flush_cjk(&mut cjk, &mut out);
            ascii.push(c);
        } else {
            flush_cjk(&mut cjk, &mut out);
            flush_ascii(&mut ascii, &mut out);
        }
    }
    flush_cjk(&mut cjk, &mut out);
    flush_ascii(&mut ascii, &mut out);
    out
}

/// Build the STT prompt from the clipboard history (newest first). Newest
/// terms go LAST: whisper truncates a long prompt from the front.
pub fn stt_hint(clipboard: &[ClipEntry], now_ms: u64, language: &str) -> Option<SttHint> {
    let window_ms = CLIP_WINDOW.as_millis() as u64;
    // One block per clipboard entry, newest entry first.
    let mut blocks: Vec<Vec<String>> = Vec::new();
    let (mut count, mut chars) = (0, 0);
    'outer: for e in clipboard
        .iter()
        .filter(|e| now_ms.saturating_sub(e.timestamp) <= window_ms)
    {
        let mut block = Vec::new();
        for t in extract_terms(&e.text) {
            if block.contains(&t) || blocks.iter().flatten().any(|x| x == &t) {
                continue;
            }
            let n = t.chars().count() + 1;
            if chars + n > STT_PROMPT_MAX_CHARS || count >= STT_PROMPT_MAX_TERMS {
                blocks.push(block);
                break 'outer;
            }
            chars += n;
            count += 1;
            block.push(t);
        }
        blocks.push(block);
    }
    let terms: Vec<String> = blocks.into_iter().rev().flatten().collect();
    if terms.is_empty() {
        return None;
    }
    // The prompt's punctuation style leaks into the transcript.
    let cjk = matches!(language, "ja" | "zh");
    let prompt = if cjk { format!("{}。", terms.join("、")) } else { format!("{}.", terms.join(", ")) };
    Some(SttHint { prompt, terms })
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Whisper reciting the prompt instead of transcribing: the transcript is the
/// whole prompt, or nothing but `ECHO_MIN_TERMS`+ prompt terms. A dictation of
/// one or two of the terms ("講演会") stays valid.
pub fn is_prompt_echo(text: &str, hint: &SttHint) -> bool {
    let mut rest = norm(text);
    if rest.is_empty() {
        return false;
    }
    if rest == norm(&hint.prompt) {
        return true;
    }
    let mut terms: Vec<String> = hint.terms.iter().map(|t| norm(t)).filter(|t| !t.is_empty()).collect();
    // Longest first so "東京タワー" is removed before "東京".
    terms.sort_by_key(|t| std::cmp::Reverse(t.chars().count()));
    let mut hits = 0;
    for t in &terms {
        let n = rest.matches(t.as_str()).count();
        if n > 0 {
            hits += n;
            rest = rest.replace(t.as_str(), "");
        }
    }
    rest.is_empty() && hits >= ECHO_MIN_TERMS
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
    fn terms_extraction() {
        assert_eq!(
            extract_terms("講演会の資料をfire-speakとNext.jsで作る。https://x.com/a me@x.jp"),
            vec!["講演会", "資料", "fire-speak", "Next.js"]
        );
        assert_eq!(extract_terms("東京タワー・スカイツリー"), vec!["東京タワー", "スカイツリー"]);
        assert_eq!(extract_terms("the API and C# 2024 sk-ant-api03-AbCd1234xyz"), vec!["API", "C#"]);
    }

    #[test]
    fn stt_hint_order_and_limits() {
        let now = 200 * 60 * 1000;
        let c = vec![clip(now - 1_000, "新しい用語 Tauri"), clip(now - 2_000, "古い単語 Whisper Tauri")];
        let h = stt_hint(&c, now, "ja").unwrap();
        // older entry first, newest entry last (whisper keeps the tail), deduped
        assert_eq!(h.terms, vec!["単語", "Whisper", "用語", "Tauri"]);
        assert_eq!(h.prompt, "単語、Whisper、用語、Tauri。");
        assert!(stt_hint(&[clip(now - 61 * 60 * 1000, "古すぎる用語")], now, "ja").is_none());
        let many: Vec<ClipEntry> = (0..50).map(|i| clip(now - i, &format!("用語{i}番目 word{i}"))).collect();
        let h = stt_hint(&many, now, "en").unwrap();
        assert!(h.terms.iter().map(|t| t.chars().count() + 1).sum::<usize>() <= STT_PROMPT_MAX_CHARS);
        assert!(h.prompt.contains(", "));
        // the newest entry's terms survive the cap
        assert!(h.prompt.ends_with("word0."));
    }

    #[test]
    fn prompt_echo() {
        let hint = SttHint {
            prompt: "講演会、Tauri、Whisper、東京タワー。".into(),
            terms: vec!["講演会".into(), "Tauri".into(), "Whisper".into(), "東京タワー".into()],
        };
        assert!(is_prompt_echo("講演会、Tauri、Whisper、東京タワー。", &hint));
        assert!(is_prompt_echo("Tauri Whisper 講演会", &hint));
        assert!(!is_prompt_echo("講演会", &hint));
        assert!(!is_prompt_echo("Tauri と Whisper", &hint));
        assert!(!is_prompt_echo("明日の講演会でTauriとWhisperを紹介します", &hint));
    }

    #[test]
    fn context_leak() {
        // 2026-10-17 on Poolside: "これ要約して。" -> a summary of recent dictations
        assert!(looks_like_context_leak(
            "これ要約して。",
            "ゴミ出しのルールは各自治体がバラバラで、まとまりがない。LMUが確かな情報源として使えるようにすることが目的である。"
        ));
        assert!(!looks_like_context_leak("これを要約してください", "これを要約してください。"));
        assert!(!looks_like_context_leak(
            "えーと明日はこうえんかいであたらしいきかいのどうにゅうについて話します",
            "明日は講演会で新しい機械の導入について話します。"
        ));
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
