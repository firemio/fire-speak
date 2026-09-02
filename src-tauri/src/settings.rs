use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::Manager;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_active_mode_id")]
    pub active_mode_id: String,
    #[serde(default = "default_paste_mode")]
    pub paste_mode: String,
    #[serde(default = "default_true")]
    pub restore_clipboard: bool,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default = "default_history_limit")]
    pub history_limit: usize,
    #[serde(default)]
    pub stt: SttSettings,
    #[serde(default)]
    pub llm: LlmSettings,
    #[serde(default = "default_modes")]
    pub modes: Vec<Mode>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey(),
            language: default_language(),
            active_mode_id: default_active_mode_id(),
            paste_mode: default_paste_mode(),
            restore_clipboard: true,
            autostart: false,
            history_limit: default_history_limit(),
            stt: SttSettings::default(),
            llm: LlmSettings::default(),
            modes: default_modes(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SttSettings {
    #[serde(default = "default_stt_engine")]
    pub engine: String,
    #[serde(default)]
    pub local: LocalSttSettings,
    #[serde(default)]
    pub cloud: CloudSttSettings,
}

impl Default for SttSettings {
    fn default() -> Self {
        Self {
            engine: default_stt_engine(),
            local: LocalSttSettings::default(),
            cloud: CloudSttSettings::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalSttSettings {
    #[serde(default = "default_server_port")]
    pub server_port: u16,
    #[serde(default)]
    pub model_path: String,
    #[serde(default)]
    pub server_path: String,
    #[serde(default = "default_threads")]
    pub threads: u32,
}

impl Default for LocalSttSettings {
    fn default() -> Self {
        Self {
            server_port: default_server_port(),
            model_path: String::new(),
            server_path: String::new(),
            threads: default_threads(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloudSttSettings {
    #[serde(default = "default_cloud_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_cloud_model")]
    pub model: String,
}

impl Default for CloudSttSettings {
    fn default() -> Self {
        Self {
            base_url: default_cloud_base_url(),
            api_key: String::new(),
            model: default_cloud_model(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmSettings {
    #[serde(default = "default_active_provider_id")]
    pub active_provider_id: String,
    #[serde(default = "default_providers")]
    pub providers: Vec<LlmProvider>,
}

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            active_provider_id: default_active_provider_id(),
            providers: default_providers(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmProvider {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_provider_kind")]
    pub kind: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mode {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub instruction: String,
    #[serde(default)]
    pub use_llm: bool,
}

fn default_hotkey() -> String {
    "Ctrl+Alt+Space".to_string()
}
fn default_language() -> String {
    "auto".to_string()
}
fn default_active_mode_id() -> String {
    "polish".to_string()
}
fn default_paste_mode() -> String {
    "paste".to_string()
}
fn default_true() -> bool {
    true
}
fn default_history_limit() -> usize {
    50
}
fn default_stt_engine() -> String {
    "local".to_string()
}
fn default_server_port() -> u16 {
    8178
}
fn default_threads() -> u32 {
    4
}
fn default_cloud_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}
fn default_cloud_model() -> String {
    "whisper-1".to_string()
}
fn default_active_provider_id() -> String {
    "anthropic".to_string()
}
fn default_provider_kind() -> String {
    "openai".to_string()
}

fn default_providers() -> Vec<LlmProvider> {
    vec![
        LlmProvider {
            id: "anthropic".to_string(),
            name: "Claude (Anthropic)".to_string(),
            kind: "anthropic".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            api_key: String::new(),
            model: "claude-haiku-4-5".to_string(),
        },
        LlmProvider {
            id: "laguna".to_string(),
            name: "Laguna S 2.1 (OpenRouter free)".to_string(),
            kind: "openai".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            api_key: String::new(),
            model: "poolside/laguna-s-2.1:free".to_string(),
        },
    ]
}

fn default_modes() -> Vec<Mode> {
    vec![
        Mode {
            id: "polish".to_string(),
            name: "整形".to_string(),
            instruction: "フィラー(「えー」「あの」「um」等)と言い直しを除去し、句読点・改行を整えて自然な文章にしてください。内容・意味は変えないでください。話者が使った言語のまま出力してください。".to_string(),
            use_llm: true,
        },
        Mode {
            id: "raw".to_string(),
            name: "そのまま".to_string(),
            instruction: String::new(),
            use_llm: false,
        },
        Mode {
            id: "to_en".to_string(),
            name: "英語に翻訳".to_string(),
            instruction: "内容を自然で流暢な英語に翻訳してください。フィラーは除去してください。".to_string(),
            use_llm: true,
        },
        Mode {
            id: "to_ja".to_string(),
            name: "日本語に翻訳".to_string(),
            instruction: "内容を自然な日本語に翻訳してください。フィラーは除去してください。".to_string(),
            use_llm: true,
        },
        Mode {
            id: "terminal".to_string(),
            name: "ターミナルコマンド".to_string(),
            instruction: "発話内容を Windows PowerShell で実行可能なコマンドに変換してください。コマンドのみを出力し、説明やコードフェンスは付けないでください。".to_string(),
            use_llm: true,
        },
        Mode {
            id: "business".to_string(),
            name: "ビジネス文体".to_string(),
            instruction: "内容を丁寧なビジネス日本語(です・ます調)に書き直してください。フィラーは除去し、簡潔で礼儀正しい文章にしてください。".to_string(),
            use_llm: true,
        },
    ]
}

pub fn config_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map_err(|e| format!("設定フォルダを取得できませんでした: {e}"))
}

fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(config_dir(app)?.join("settings.json"))
}

/// Load settings from disk; creates the file with defaults on first run.
pub fn load(app: &tauri::AppHandle) -> Settings {
    let path = match settings_path(app) {
        Ok(p) => p,
        Err(_) => return Settings::default(),
    };
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(s) = serde_json::from_str::<Settings>(&text) {
            return s;
        }
    }
    let s = Settings::default();
    let _ = save(app, &s);
    s
}

pub fn save(app: &tauri::AppHandle, settings: &Settings) -> Result<(), String> {
    let path = settings_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("設定フォルダを作成できませんでした: {e}"))?;
    }
    let text = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("設定のシリアライズに失敗しました: {e}"))?;
    std::fs::write(&path, text).map_err(|e| format!("設定の保存に失敗しました: {e}"))
}
