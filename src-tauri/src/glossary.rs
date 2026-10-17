//! Built-in glossary (v0.15.2): names that speech recognition spells by
//! sound — LLM and AI services, developer tools, everyday services and
//! acronyms. Ships with the app (not stored in `dictionary.json`), so it
//! works from the first dictation and improves with each release.
//!
//! Each entry has:
//! - `aliases`: misrecognitions that can only mean the term ("チャットGPT",
//!   "エーアイ", "Git Hub"). Replaced in code, with or without an LLM.
//! - `readings`: how STT may write the term by sound ("クロード", "ジェミニ").
//!   A reading can also be an ordinary word, so it is never replaced in code:
//!   when one appears, the term goes to the polish prompt's term list, and
//!   for `strong` entries (proper nouns whose official spelling is the
//!   expected one: AI, dev) also to the term-fix pass, where the LLM decides
//!   whether the reading meant the term.
//!
//! Matching is word-bounded: a katakana reading must not touch other
//! katakana ("ライン" does not match inside "オンライン"), an ASCII alias
//! must not touch other ASCII letters/digits, and ASCII compares
//! case-insensitively.

pub struct Entry {
    pub term: &'static str,
    pub aliases: &'static [&'static str],
    pub readings: &'static [&'static str],
    pub strong: bool,
    pub category: &'static str,
}

const fn s(term: &'static str, aliases: &'static [&'static str], readings: &'static [&'static str], category: &'static str) -> Entry {
    Entry { term, aliases, readings, strong: true, category }
}

/// Weak: the reading is a normal way to write it in Japanese (or an ordinary
/// word), so only the polish prompt gets the hint.
const fn w(term: &'static str, aliases: &'static [&'static str], readings: &'static [&'static str], category: &'static str) -> Entry {
    Entry { term, aliases, readings, strong: false, category }
}

const AI: &str = "ai";
const DEV: &str = "dev";
const LIFE: &str = "life";

pub static ENTRIES: &[Entry] = &[
    // ---- LLM / AI ----
    s("ChatGPT", &["チャットGPT", "チャットジーピーティー", "チャットジーピーティ", "Chat GPT", "チャットGTP"], &[], AI),
    s("GPT", &["ジーピーティー", "ジーピーティ"], &[], AI),
    s("OpenAI", &["オープンAI", "オープンエーアイ", "Open AI"], &["オープンエイアイ"], AI),
    s("o3", &[], &["オースリー"], AI),
    s("Codex", &[], &["コーデックス", "コデックス"], AI),
    w("DALL-E", &["DALL E", "Dall-E"], &["ダリ", "ダリー", "ダーリー"], AI),
    w("Whisper", &[], &["ウィスパー"], AI),
    s("Claude", &[], &["クロード"], AI),
    s("Claude Code", &["クロードコード", "クロード・コード", "Claude コード"], &[], AI),
    s("Anthropic", &[], &["アンソロピック", "アンスロピック", "アンソロピー", "アントロピック"], AI),
    w("Opus", &[], &["オーパス"], AI),
    w("Sonnet", &[], &["ソネット"], AI),
    w("Haiku", &[], &["ハイク"], AI),
    s("Gemini", &[], &["ジェミニ", "ジェミナイ"], AI),
    s("Gemma", &[], &["ジェマ", "ジェンマ"], AI),
    s("Google DeepMind", &["グーグルディープマインド"], &[], AI),
    s("DeepMind", &["ディープマインド"], &[], AI),
    s("NotebookLM", &["ノートブックLM", "ノートブックエルエム", "Notebook LM"], &[], AI),
    s("Grok", &[], &["グロック", "グロク"], AI),
    s("xAI", &["エックスAI", "エックスエーアイ"], &[], AI),
    w("Llama", &[], &["ラマ"], AI),
    w("Meta", &[], &["メタ"], AI),
    s("Mistral", &[], &["ミストラル"], AI),
    s("DeepSeek", &["ディープシーク", "Deep Seek"], &[], AI),
    s("Qwen", &["クウェン"], &["クエン", "キューウェン"], AI),
    s("Moonshot AI", &["ムーンショットAI"], &[], AI),
    s("MiniMax", &["ミニマックス"], &[], AI),
    s("GLM", &["ジーエルエム"], &[], AI),
    s("Perplexity", &["パープレキシティ", "パープレキシティー", "パープレ"], &[], AI),
    s("Copilot", &["コパイロット"], &[], AI),
    s("GitHub Copilot", &["ギットハブコパイロット", "GitHubコパイロット"], &[], AI),
    s("Microsoft Copilot", &["マイクロソフトコパイロット"], &[], AI),
    w("Cursor", &[], &["カーソル"], AI),
    s("Windsurf", &["ウィンドサーフ"], &[], AI),
    w("Cline", &[], &["クライン"], AI),
    s("Devin", &[], &["デビン", "デヴィン"], AI),
    s("Manus", &[], &["マヌス", "マナス"], AI),
    s("Genspark", &["ジェンスパーク", "ゲンスパーク"], &[], AI),
    s("Speakly", &["スピークリー"], &[], AI),
    w("Poolside", &[], &["プールサイド"], AI),
    w("Laguna", &[], &["ラグーナ"], AI),
    s("Midjourney", &["ミッドジャーニー"], &[], AI),
    s("Stable Diffusion", &["ステーブルディフュージョン", "ステイブルディフュージョン"], &[], AI),
    w("Runway", &[], &["ランウェイ"], AI),
    s("Hugging Face", &["ハギングフェイス", "ハギングフェース", "HuggingFace"], &[], AI),
    s("Ollama", &["オラマ", "オーラマ"], &[], AI),
    s("LM Studio", &["LMスタジオ", "エルエムスタジオ"], &[], AI),
    s("llama.cpp", &["ラマシーピーピー", "ラマ.cpp"], &[], AI),
    s("vLLM", &["ブイLLM", "ブイエルエルエム"], &[], AI),
    s("LLM", &["エルエルエム"], &[], AI),
    w("RAG", &[], &["ラグ"], AI),
    s("MCP", &["エムシーピー"], &[], AI),
    s("AGI", &["エージーアイ"], &[], AI),
    s("LangChain", &["ラングチェーン", "ラングチェイン"], &[], AI),
    s("LlamaIndex", &["ラマインデックス"], &[], AI),
    s("Dify", &["ディファイ"], &[], AI),
    s("n8n", &["エヌエイトエヌ", "エヌハチエヌ"], &[], AI),
    s("Zapier", &["ザピアー", "ザピア"], &[], AI),
    s("OpenRouter", &["オープンルーター"], &[], AI),
    s("Cohere", &[], &["コヒア", "コーヒア"], AI),
    w("Parakeet", &[], &["パラキート"], AI),
    s("Qwen3-ASR", &["クエン3ASR", "クウェン3ASR", "クエン三ASR"], &[], AI),
    s("DeepL", &["ディープL", "ディープエル"], &[], AI),
    s("ElevenLabs", &["イレブンラボ", "イレブンラボズ", "イレブンラブス"], &[], AI),
    s("NVIDIA", &["エヌビディア", "エヌヴィディア"], &[], AI),
    s("CUDA", &[], &["クーダ"], AI),
    w("Transformer", &[], &["トランスフォーマー"], AI),
    s("Fable", &[], &["フェイブル"], AI),
    // ---- development ----
    s("GitHub", &["ギットハブ", "Git Hub"], &[], DEV),
    s("GitLab", &["ギットラボ", "Git Lab"], &[], DEV),
    s("Git", &[], &["ギット"], DEV),
    s("Python", &[], &["パイソン"], DEV),
    s("JavaScript", &["ジャバスクリプト", "Java Script"], &[], DEV),
    s("TypeScript", &["タイプスクリプト", "Type Script"], &[], DEV),
    w("Rust", &[], &["ラスト"], DEV),
    s("Kotlin", &[], &["コトリン"], DEV),
    w("Swift", &[], &["スウィフト"], DEV),
    s("React", &[], &["リアクト"], DEV),
    s("Next.js", &["ネクストJS", "ネクストジェーエス", "ネクスト.js", "Next JS", "NextJS"], &[], DEV),
    s("Node.js", &["ノードJS", "ノードジェーエス", "Node JS", "NodeJS"], &[], DEV),
    s("Vue.js", &["ビューJS", "ビュージェーエス", "Vue JS"], &[], DEV),
    s("Tailwind CSS", &["テイルウィンド", "テールウィンド"], &[], DEV),
    w("Vite", &[], &["ヴィート", "ビート"], DEV),
    s("Tauri", &[], &["タウリ", "トーリ"], DEV),
    s("Electron", &[], &["エレクトロン"], DEV),
    s("Docker", &[], &["ドッカー"], DEV),
    s("Kubernetes", &["クバネティス", "クーバネティス", "クバネテス", "クーベネティス"], &[], DEV),
    s("AWS", &["エーダブリューエス", "エーダブルユーエス"], &[], DEV),
    s("Azure", &[], &["アジュール", "アズール"], DEV),
    s("Google Cloud", &["グーグルクラウド"], &[], DEV),
    s("Cloudflare", &["クラウドフレア"], &[], DEV),
    s("Vercel", &["バーセル", "ヴァーセル"], &[], DEV),
    s("Supabase", &["スーパベース", "スパベース"], &[], DEV),
    s("Firebase", &["ファイアベース", "ファイヤーベース"], &[], DEV),
    s("PostgreSQL", &["ポストグレスキューエル", "ポスグレ"], &["ポストグレス"], DEV),
    s("SQLite", &["エスキューライト"], &[], DEV),
    s("SQL", &["エスキューエル"], &[], DEV),
    w("JSON", &[], &["ジェイソン"], DEV),
    s("YAML", &["ヤムル"], &[], DEV),
    w("Markdown", &[], &["マークダウン"], DEV),
    s("VS Code", &["ブイエスコード", "VSコード", "VSCode"], &[], DEV),
    s("Visual Studio", &[], &["ビジュアルスタジオ"], DEV),
    s("Neovim", &["ネオビム", "ネオヴィム"], &[], DEV),
    s("Linux", &[], &["リナックス"], DEV),
    s("Ubuntu", &["ウブントゥ", "ウブンツ"], &[], DEV),
    s("macOS", &["マックOS", "マックオーエス", "Mac OS"], &[], DEV),
    s("WSL", &["ダブリューエスエル"], &[], DEV),
    s("PowerShell", &[], &["パワーシェル"], DEV),
    s("API", &["エーピーアイ"], &[], DEV),
    s("SDK", &["エスディーケー"], &[], DEV),
    s("CLI", &["シーエルアイ"], &[], DEV),
    s("GUI", &["ジーユーアイ"], &[], DEV),
    s("UI", &["ユーアイ"], &[], DEV),
    s("UX", &["ユーエックス"], &[], DEV),
    s("CI/CD", &["シーアイシーディー"], &[], DEV),
    s("npm", &["エヌピーエム"], &[], DEV),
    s("pnpm", &["ピーエヌピーエム"], &[], DEV),
    s("GPU", &["ジーピーユー"], &[], DEV),
    s("CPU", &["シーピーユー"], &[], DEV),
    s("NPU", &["エヌピーユー"], &[], DEV),
    w("Vulkan", &[], &["バルカン", "ヴァルカン"], DEV),
    s("Ryzen", &[], &["ライゼン"], DEV),
    w("Slack", &[], &["スラック"], DEV),
    s("Discord", &[], &["ディスコード"], DEV),
    s("Notion", &[], &["ノーション"], DEV),
    s("Figma", &[], &["フィグマ"], DEV),
    w("Jira", &[], &["ジラ"], DEV),
    w("Obsidian", &[], &["オブシディアン"], DEV),
    s("Stack Overflow", &["スタックオーバーフロー"], &[], DEV),
    s("Raspberry Pi", &["ラズベリーパイ"], &[], DEV),
    s("WebGPU", &["ウェブGPU", "ウェブジーピーユー"], &[], DEV),
    s("OAuth", &["オーオース"], &[], DEV),
    w("SaaS", &[], &["サース"], DEV),
    // ---- everyday ----
    w("iPhone", &["i Phone", "i phone"], &["アイフォン", "アイフォーン"], LIFE),
    w("iPad", &["i Pad"], &["アイパッド"], LIFE),
    w("Android", &[], &["アンドロイド"], LIFE),
    w("Apple", &[], &["アップル"], LIFE),
    w("Google", &[], &["グーグル"], LIFE),
    w("Gmail", &["ジーメール"], &[], LIFE),
    w("YouTube", &["You Tube"], &["ユーチューブ"], LIFE),
    w("Amazon", &[], &["アマゾン"], LIFE),
    w("Microsoft", &[], &["マイクロソフト"], LIFE),
    w("Windows", &[], &["ウィンドウズ"], LIFE),
    w("Excel", &[], &["エクセル"], LIFE),
    w("PowerPoint", &[], &["パワーポイント", "パワポ"], LIFE),
    w("Outlook", &[], &["アウトルック"], LIFE),
    w("Teams", &[], &["チームズ"], LIFE),
    w("Zoom", &[], &["ズーム"], LIFE),
    w("Instagram", &[], &["インスタグラム"], LIFE),
    w("TikTok", &["Tik Tok"], &["ティックトック"], LIFE),
    w("Facebook", &[], &["フェイスブック"], LIFE),
    w("Netflix", &[], &["ネットフリックス", "ネトフリ"], LIFE),
    w("Spotify", &[], &["スポティファイ"], LIFE),
    w("PayPay", &["Pay Pay"], &["ペイペイ"], LIFE),
    w("Uber Eats", &[], &["ウーバーイーツ"], LIFE),
    w("Wi-Fi", &["WiFi", "Wi Fi"], &["ワイファイ"], LIFE),
    w("Bluetooth", &[], &["ブルートゥース", "ブルートゥス"], LIFE),
    w("Dropbox", &[], &["ドロップボックス"], LIFE),
    w("ChatGPT Plus", &["チャットGPTプラス"], &[], LIFE),
    s("QRコード", &["キューアールコード"], &[], LIFE),
    s("USB", &["ユーエスビー"], &[], LIFE),
    s("USB-C", &["USBC", "ユーエスビーシー"], &[], LIFE),
    s("PDF", &["ピーディーエフ"], &[], LIFE),
    s("URL", &["ユーアールエル"], &[], LIFE),
    s("AI", &["エーアイ"], &[], LIFE),
    s("IT", &["アイティー"], &[], LIFE),
    s("DX", &["ディーエックス"], &[], LIFE),
    s("SNS", &["エスエヌエス"], &[], LIFE),
    s("PC", &["ピーシー"], &[], LIFE),
    s("SMS", &["エスエムエス"], &[], LIFE),
    s("ID", &["アイディー"], &[], LIFE),
    s("OS", &["オーエス"], &[], LIFE),
    w("SIM", &[], &["シム"], LIFE),
    s("eSIM", &["イーシム"], &[], LIFE),
    s("5G", &["ファイブジー"], &[], LIFE),
    s("4K", &["フォーケー", "ヨンケー"], &[], LIFE),
];

fn is_katakana(c: char) -> bool {
    matches!(c as u32, 0x30A1..=0x30FA | 0x30FC)
}

fn is_word_ascii(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

/// Byte ranges where `needle` occurs in `hay` as a whole word: ASCII is
/// compared case-insensitively; a match may not continue a katakana run
/// (needle starting/ending in katakana) or an ASCII word (needle
/// starting/ending in an ASCII letter or digit).
fn find_words(hay: &str, needle: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let lower_hay = hay.to_ascii_lowercase();
    let lower_needle = needle.to_ascii_lowercase();
    let first = needle.chars().next().unwrap();
    let last = needle.chars().last().unwrap();
    let mut from = 0;
    while let Some(i) = lower_hay[from..].find(&lower_needle) {
        let start = from + i;
        let end = start + lower_needle.len();
        let before = hay[..start].chars().last();
        let after = hay[end..].chars().next();
        let clash = |edge: char, other: Option<char>| match other {
            Some(o) => (is_katakana(edge) && is_katakana(o)) || (is_word_ascii(edge) && is_word_ascii(o)),
            None => false,
        };
        if !clash(first, before) && !clash(last, after) {
            out.push((start, end));
        }
        from = start + lower_needle.len().max(1);
    }
    out
}

/// Replace every built-in alias with its term (longest alias first).
pub fn apply_aliases(text: &str) -> String {
    let mut pairs: Vec<(&str, &str)> = ENTRIES
        .iter()
        .flat_map(|e| e.aliases.iter().map(move |a| (*a, e.term)))
        .collect();
    pairs.sort_by_key(|(a, _)| std::cmp::Reverse(a.len()));
    let mut out = text.to_string();
    for (alias, term) in pairs {
        let hits = find_words(&out, alias);
        if hits.is_empty() {
            continue;
        }
        let mut s = String::with_capacity(out.len());
        let mut last = 0;
        for (a, b) in hits {
            s.push_str(&out[last..a]);
            s.push_str(term);
            last = b;
        }
        s.push_str(&out[last..]);
        out = s;
    }
    out
}

/// Terms whose reading occurs in `text` (and that are not in it yet):
/// (every such term → polish prompt, strong ones → term-fix pass).
pub fn hinted_terms(text: &str) -> (Vec<String>, Vec<String>) {
    let mut hint = Vec::new();
    let mut fix = Vec::new();
    for e in ENTRIES {
        if e.readings.is_empty() || !find_words(text, e.term).is_empty() {
            continue;
        }
        if e.readings.iter().any(|r| !find_words(text, r).is_empty()) {
            hint.push(e.term.to_string());
            if e.strong {
                fix.push(e.term.to_string());
            }
        }
    }
    (hint, fix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_replace_whole_words() {
        assert_eq!(apply_aliases("チャットGPTに聞いた"), "ChatGPTに聞いた");
        assert_eq!(apply_aliases("エーアイで要約する"), "AIで要約する");
        assert_eq!(apply_aliases("git hubに上げた"), "GitHubに上げた");
        assert_eq!(apply_aliases("クロードコードで直す"), "Claude Codeで直す");
        assert_eq!(apply_aliases("ネクストJSとノードJS"), "Next.jsとNode.js");
        // the longer alias wins over "ジーピーティー" inside it
        assert_eq!(apply_aliases("チャットジーピーティーとジーピーティー"), "ChatGPTとGPT");
        // no match inside a longer katakana word / ASCII word
        assert_eq!(apply_aliases("ガイドラインとオンライン"), "ガイドラインとオンライン");
        assert_eq!(apply_aliases("OSAKA"), "OSAKA");
        assert_eq!(apply_aliases("明日は雨です"), "明日は雨です");
    }

    #[test]
    fn readings_hint_terms() {
        let (hint, fix) = hinted_terms("クロードとジェミニを比べた");
        assert!(hint.contains(&"Claude".to_string()) && hint.contains(&"Gemini".to_string()));
        assert!(fix.contains(&"Claude".to_string()));
        // weak: hint only
        let (hint, fix) = hinted_terms("ユーチューブで見た");
        assert_eq!(hint, vec!["YouTube".to_string()]);
        assert!(fix.is_empty());
        // inside a longer katakana word: no match
        let (hint, _) = hinted_terms("オーラストの話");
        assert!(hint.is_empty());
        // already spelled correctly: nothing to hint
        let (hint, _) = hinted_terms("Claudeとクロード");
        assert!(!hint.contains(&"Claude".to_string()));
    }

    /// Glossary -> term-fix pass -> accept, against a local Ollama model:
    /// `FIRE_SPEAK_OLLAMA_MODEL=gemma4:12b-it-qat cargo test --lib glossary_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn glossary_live() {
        let mut p = crate::settings::ollama_provider();
        p.model = std::env::var("FIRE_SPEAK_OLLAMA_MODEL").unwrap();
        for text in [
            "クロードとジェミニを比べてみた。",
            "エーアイの話をチャットジーピーティーとした。",
            "ディープシークとクウェンはオラマで動く。",
            "明日はラストの打ち合わせです。",
            "パイソンで書いたスクリプトをギットハブに上げた。",
        ] {
            let fixed_aliases = apply_aliases(text);
            let (_, fix) = hinted_terms(&fixed_aliases);
            let terms = crate::context::term_fix_candidates(&fix, &fixed_aliases);
            let out = if terms.is_empty() {
                fixed_aliases.clone()
            } else {
                let r = crate::llm::fix_terms(&p, &fixed_aliases, &terms).await.unwrap();
                if crate::context::accept_term_fix(&fixed_aliases, &r, &terms) { r } else { format!("{fixed_aliases}  (rejected: {r})") }
            };
            println!("{text}
  -> {out}   terms={terms:?}");
        }
    }

    #[test]
    fn entries_are_sane() {
        let mut seen = std::collections::HashSet::new();
        for e in ENTRIES {
            assert!(seen.insert(e.term), "duplicate term {}", e.term);
            for a in e.aliases {
                assert!(a.chars().count() >= 2 && *a != e.term, "bad alias {a}");
            }
        }
        assert!(ENTRIES.len() >= 150);
        for e in ENTRIES {
            assert!(!e.aliases.is_empty() || !e.readings.is_empty(), "{} does nothing", e.term);
        }
    }
}
