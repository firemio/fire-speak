//! Term dictionary (v0.12.5): names and terms that should come out spelled
//! a fixed way (`Qwen3-ASR`, `Next.js`, 人名). It grows by itself while the app
//! is used, so a term copied once keeps working after it has left the
//! clipboard window:
//!
//! - a clipboard term (ASCII / katakana) that ends up in a dictation's final
//!   text is added (`source: "auto"`);
//! - when the term-fix pass replaces a misheard span ("クエン三ASR" →
//!   "Qwen3-ASR"), that span is stored as an alias and later replaced in code,
//!   with no LLM call.
//!
//! The user can add entries (with optional aliases) and delete any entry in
//! the settings screen. Stored in `{app_config_dir}/dictionary.json`.
//!
//! Dictionary terms go to the STT prompt after the clipboard terms' share and
//! to the polish prompt as a term list.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Emitter;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DictEntry {
    pub term: String,
    /// Misrecognized spellings replaced by `term` before polishing.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// "user" (added in settings) | "auto" (learned)
    pub source: String,
    #[serde(default)]
    pub hits: u32,
    /// unix ms
    pub added: u64,
    pub last_used: u64,
}

/// Learned entries kept at most; the least recently used go first.
const AUTO_MAX: usize = 300;
/// Learned entries not used for this long are dropped.
const AUTO_TTL_MS: u64 = 180 * 24 * 60 * 60 * 1000;
const ALIAS_MAX: usize = 8;
const TERM_MAX_CHARS: usize = 40;
/// Terms handed to the polish prompt.
pub const PROMPT_TERMS_MAX: usize = 40;

static LOCK: Mutex<()> = Mutex::new(());

fn path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(crate::settings::config_dir(app)?.join("dictionary.json"))
}

pub fn load(app: &tauri::AppHandle) -> Vec<DictEntry> {
    path(app)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(app: &tauri::AppHandle, list: &[DictEntry]) -> Result<(), String> {
    let p = path(app)?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("ERR_FILE_IO|create dir: {e}"))?;
    }
    let text = serde_json::to_string_pretty(list)
        .map_err(|e| format!("ERR_INTERNAL|serialize dictionary: {e}"))?;
    std::fs::write(&p, text).map_err(|e| format!("ERR_FILE_IO|write dictionary: {e}"))
}

fn update(app: &tauri::AppHandle, f: impl FnOnce(&mut Vec<DictEntry>) -> bool) -> Result<(), String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = load(app);
    if f(&mut list) {
        save(app, &list)?;
        let _ = app.emit("dictionary-updated", ());
    }
    Ok(())
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// User entries first, then learned ones by recency.
pub fn ranked(list: &[DictEntry]) -> Vec<&DictEntry> {
    let mut v: Vec<&DictEntry> = list.iter().collect();
    v.sort_by(|a, b| {
        (b.source == "user")
            .cmp(&(a.source == "user"))
            .then(b.last_used.cmp(&a.last_used))
    });
    v
}

pub fn ranked_terms(list: &[DictEntry]) -> Vec<String> {
    ranked(list).into_iter().map(|e| e.term.clone()).collect()
}

/// Replace every known alias with its term (longest alias first, so a
/// longer misspelling wins over a shorter one inside it).
pub fn apply_aliases(list: &[DictEntry], text: &str) -> String {
    let mut pairs: Vec<(&str, &str)> = list
        .iter()
        .flat_map(|e| e.aliases.iter().map(move |a| (a.as_str(), e.term.as_str())))
        .filter(|(a, t)| a.chars().count() >= 2 && a != t)
        .collect();
    pairs.sort_by_key(|(a, _)| std::cmp::Reverse(a.chars().count()));
    let mut out = text.to_string();
    for (a, t) in pairs {
        if out.contains(a) {
            out = out.replace(a, t);
        }
    }
    out
}

/// The span of `before` that became `term` in `after`. The text around the
/// term in `after` must be unchanged in `before`; the common prefix/suffix
/// is capped at the term's position, so a suffix shared with the term
/// ("ASR" in "クエン三ASR" → "Qwen3-ASR") stays part of the alias.
pub fn extract_alias(before: &str, after: &str, term: &str) -> Option<String> {
    let b: Vec<char> = before.trim().chars().collect();
    let a: Vec<char> = after.trim().chars().collect();
    let t: Vec<char> = term.chars().collect();
    let start = a.windows(t.len()).position(|w| w == t.as_slice())?;
    let end = start + t.len();
    let mut p = 0;
    while p < start && p < b.len() && b[p] == a[p] {
        p += 1;
    }
    let mut s = 0;
    while s < a.len() - end && s < b.len() - p && b[b.len() - 1 - s] == a[a.len() - 1 - s] {
        s += 1;
    }
    // everything outside the term must be accounted for by prefix + suffix
    if p != start || s != a.len() - end {
        return None;
    }
    let mid: String = b[p..b.len() - s].iter().collect();
    let n = mid.chars().count();
    ((2..=TERM_MAX_CHARS).contains(&n) && mid != term).then_some(mid)
}

fn is_learnable(term: &str) -> bool {
    let n = term.chars().count();
    (3..=TERM_MAX_CHARS).contains(&n)
        && term.chars().any(|c| c.is_ascii_alphabetic() || matches!(c as u32, 0x30A1..=0x30FA | 0x30FC))
}

/// What one dictation teaches the dictionary.
pub struct Lesson<'a> {
    pub final_text: &'a str,
    /// Clipboard terms of this dictation (`SttHint::terms`).
    pub clipboard_terms: &'a [String],
    /// Accepted term fix: (text before, text after, terms added).
    pub fix: Option<(&'a str, &'a str, &'a [String])>,
}

/// Pure part of `learn`; returns whether anything changed.
pub fn learn_into(list: &mut Vec<DictEntry>, lesson: &Lesson, now: u64) -> bool {
    let mut changed = false;
    let touch = |list: &mut Vec<DictEntry>, term: &str, alias: Option<String>| {
        let i = match list.iter().position(|e| e.term == term) {
            Some(i) => i,
            None => {
                list.push(DictEntry {
                    term: term.to_string(),
                    aliases: Vec::new(),
                    source: "auto".into(),
                    hits: 0,
                    added: now,
                    last_used: now,
                });
                list.len() - 1
            }
        };
        let e = &mut list[i];
        e.hits = e.hits.saturating_add(1);
        e.last_used = now;
        if let Some(a) = alias {
            if !e.aliases.contains(&a) {
                e.aliases.push(a);
                if e.aliases.len() > ALIAS_MAX {
                    e.aliases.remove(0);
                }
            }
        }
    };

    if let Some((before, after, added)) = lesson.fix {
        for t in added {
            let alias = if added.len() == 1 { extract_alias(before, after, t) } else { None };
            touch(list, t, alias);
            changed = true;
        }
    }
    for t in lesson.clipboard_terms.iter().filter(|t| is_learnable(t)) {
        if lesson.final_text.contains(t.as_str()) && !lesson.fix.is_some_and(|f| f.2.contains(t)) {
            touch(list, t, None);
            changed = true;
        }
    }
    let used: Vec<String> = list
        .iter()
        .filter(|e| lesson.final_text.contains(e.term.as_str()) && e.last_used != now)
        .map(|e| e.term.clone())
        .collect();
    for t in used {
        touch(list, &t, None);
        changed = true;
    }

    let before = list.len();
    list.retain(|e| e.source == "user" || now.saturating_sub(e.last_used) <= AUTO_TTL_MS);
    let mut autos: Vec<(u64, String)> =
        list.iter().filter(|e| e.source != "user").map(|e| (e.last_used, e.term.clone())).collect();
    if autos.len() > AUTO_MAX {
        autos.sort();
        let drop: Vec<String> = autos[..autos.len() - AUTO_MAX].iter().map(|x| x.1.clone()).collect();
        list.retain(|e| !drop.contains(&e.term));
    }
    changed || list.len() != before
}

pub fn learn(app: &tauri::AppHandle, lesson: &Lesson) {
    if let Err(e) = update(app, |list| learn_into(list, lesson, now_ms())) {
        eprintln!("dictionary learn failed: {e}");
    }
}

/// Add or replace a user entry. `aliases` are extra spellings to replace.
pub fn add_user(app: &tauri::AppHandle, term: &str, aliases: &[String]) -> Result<(), String> {
    let term = term.trim();
    if term.is_empty() || term.chars().count() > TERM_MAX_CHARS {
        return Err("ERR_DICT_TERM".to_string());
    }
    let aliases: Vec<String> = aliases
        .iter()
        .map(|a| a.trim().to_string())
        .filter(|a| a.chars().count() >= 2 && a != term)
        .collect();
    let now = now_ms();
    update(app, |list| {
        match list.iter_mut().find(|e| e.term == term) {
            Some(e) => {
                e.source = "user".into();
                e.aliases = aliases.clone();
                e.last_used = now;
            }
            None => list.push(DictEntry {
                term: term.to_string(),
                aliases: aliases.clone(),
                source: "user".into(),
                hits: 0,
                added: now,
                last_used: now,
            }),
        }
        true
    })
}

pub fn remove(app: &tauri::AppHandle, term: &str) -> Result<(), String> {
    update(app, |list| {
        let n = list.len();
        list.retain(|e| e.term != term);
        list.len() != n
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(term: &str, aliases: &[&str], source: &str, last: u64) -> DictEntry {
        DictEntry {
            term: term.into(),
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
            source: source.into(),
            hits: 1,
            added: 0,
            last_used: last,
        }
    }

    #[test]
    fn aliases_replace_longest_first() {
        let list = vec![entry("Qwen3-ASR", &["クエン三ASR", "クエン"], "auto", 1)];
        assert_eq!(apply_aliases(&list, "クエン三ASRを推奨にしたい。"), "Qwen3-ASRを推奨にしたい。");
        assert_eq!(apply_aliases(&list, "明日は雨"), "明日は雨");
    }

    #[test]
    fn alias_extraction() {
        assert_eq!(
            extract_alias("クエン三ASRを推奨にしたい。", "Qwen3-ASRを推奨にしたい。", "Qwen3-ASR").as_deref(),
            Some("クエン三ASR")
        );
        assert_eq!(
            extract_alias("ネクストジェーエスで作る", "Next.jsで作る", "Next.js").as_deref(),
            Some("ネクストジェーエス")
        );
        // the fix changed more than the term
        assert_eq!(extract_alias("クエンを使う", "Qwen3-ASRを使います", "Qwen3-ASR"), None);
    }

    #[test]
    fn learning() {
        let mut list = vec![entry("講演会", &[], "user", 5)];
        let terms: Vec<String> = vec!["Qwen3-ASR".into(), "Tauri".into(), "会議".into()];
        let added: Vec<String> = vec!["Qwen3-ASR".into()];
        let lesson = Lesson {
            final_text: "Qwen3-ASRとTauriを講演会で紹介",
            clipboard_terms: &terms,
            fix: Some(("クエン三ASRとTauriを講演会で紹介", "Qwen3-ASRとTauriを講演会で紹介", &added)),
        };
        assert!(learn_into(&mut list, &lesson, 100));
        let q = list.iter().find(|e| e.term == "Qwen3-ASR").unwrap();
        assert_eq!(q.aliases, vec!["クエン三ASR"]);
        assert!(list.iter().any(|e| e.term == "Tauri" && e.source == "auto"));
        assert!(!list.iter().any(|e| e.term == "会議")); // kanji: not learned
        assert_eq!(list.iter().find(|e| e.term == "講演会").unwrap().last_used, 100);
        assert_eq!(ranked_terms(&list)[0], "講演会"); // user first
    }

    #[test]
    fn eviction() {
        let day = 24 * 60 * 60 * 1000;
        let mut list = vec![entry("Old", &[], "auto", 0), entry("Mine", &[], "user", 0)];
        let lesson = Lesson { final_text: "x", clipboard_terms: &[], fix: None };
        assert!(learn_into(&mut list, &lesson, 200 * day));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].term, "Mine");
    }
}
