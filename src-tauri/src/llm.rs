use crate::settings::LlmProvider;
use std::time::Duration;

const LLM_TIMEOUT_SECS: u64 = 60;
const SYSTEM_PREFIX: &str = "あなたは音声入力の後処理エンジンです。ユーザーが口述したテキストが与えられます。以下の指示に従って処理し、処理後のテキストだけを出力してください。前置き・引用符・説明・コードフェンスは一切付けないでください。\n\n指示: ";

/// Polish `raw` text with the given provider according to the mode instruction.
pub async fn polish(provider: &LlmProvider, instruction: &str, raw: &str) -> Result<String, String> {
    let system = format!("{SYSTEM_PREFIX}{instruction}");
    chat(provider, Some(&system), raw).await
}

/// Connectivity test; returns the model's response text.
pub async fn test(provider: &LlmProvider) -> Result<String, String> {
    chat(
        provider,
        None,
        "こんにちは。接続テストです。ひとことで返答してください。",
    )
    .await
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(LLM_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("HTTPクライアントの初期化に失敗しました: {e}"))
}

async fn chat(
    provider: &LlmProvider,
    system: Option<&str>,
    user: &str,
) -> Result<String, String> {
    if provider.api_key.trim().is_empty() {
        return Err("LLMプロバイダのAPIキーが設定されていません".to_string());
    }
    match provider.kind.as_str() {
        "anthropic" => chat_anthropic(provider, system, user).await,
        _ => chat_openai(provider, system, user).await,
    }
}

async fn chat_anthropic(
    provider: &LlmProvider,
    system: Option<&str>,
    user: &str,
) -> Result<String, String> {
    let url = format!(
        "{}/v1/messages",
        provider.base_url.trim_end_matches('/')
    );
    let mut body = serde_json::json!({
        "model": provider.model,
        "max_tokens": 4096,
        "messages": [{ "role": "user", "content": user }],
    });
    if let Some(sys) = system {
        body["system"] = serde_json::json!(sys);
    }
    let resp = client()?
        .post(&url)
        .header("x-api-key", provider.api_key.trim())
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("LLM APIへの接続に失敗しました: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "LLM APIエラー (HTTP {}): {}",
            status.as_u16(),
            truncate(&text, 300)
        ));
    }
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("LLM APIの応答を解析できませんでした: {e}"))?;
    v["content"][0]["text"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "LLM APIの応答にテキストが含まれていません".to_string())
}

async fn chat_openai(
    provider: &LlmProvider,
    system: Option<&str>,
    user: &str,
) -> Result<String, String> {
    let url = format!(
        "{}/chat/completions",
        provider.base_url.trim_end_matches('/')
    );
    let mut messages: Vec<serde_json::Value> = Vec::new();
    if let Some(sys) = system {
        messages.push(serde_json::json!({ "role": "system", "content": sys }));
    }
    messages.push(serde_json::json!({ "role": "user", "content": user }));
    let body = serde_json::json!({
        "model": provider.model,
        "messages": messages,
    });
    let resp = client()?
        .post(&url)
        .bearer_auth(provider.api_key.trim())
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("LLM APIへの接続に失敗しました: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "LLM APIエラー (HTTP {}): {}",
            status.as_u16(),
            truncate(&text, 300)
        ));
    }
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("LLM APIの応答を解析できませんでした: {e}"))?;
    v["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "LLM APIの応答にテキストが含まれていません".to_string())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect::<String>() + "…"
    }
}
