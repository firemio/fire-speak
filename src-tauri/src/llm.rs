use crate::settings::LlmProvider;
use std::time::Duration;

const LLM_TIMEOUT_SECS: u64 = 60;
const SYSTEM_PREFIX: &str = "あなたは音声入力の後処理エンジンです。ユーザーが口述したテキストが与えられます。以下の指示に従って処理し、処理後のテキストだけを出力してください。前置き・引用符・説明・コードフェンスは一切付けないでください。\n\n指示: ";

/// Whether `base_url` points at this machine (Ollama, LM Studio, llama.cpp
/// server...). Local OpenAI-compatible servers need no API key.
pub fn is_local_url(base_url: &str) -> bool {
    let rest = base_url
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    // Bracketed IPv6 literal (`[::1]:11434/v1`) must be taken up to the
    // closing bracket; splitting on ':' first would return "[".
    let host = if let Some(inner) = rest.strip_prefix('[') {
        inner.split(']').next().unwrap_or("")
    } else {
        rest.split(['/', ':', '?', '#']).next().unwrap_or("")
    }
    .to_ascii_lowercase();
    matches!(host.as_str(), "localhost" | "127.0.0.1" | "0.0.0.0" | "::1")
}

/// A provider can be called when it has an API key, or when it is an
/// OpenAI-compatible server on localhost (no key needed).
pub fn is_configured(provider: &LlmProvider) -> bool {
    !provider.api_key.trim().is_empty()
        || (provider.kind != "anthropic" && is_local_url(&provider.base_url))
}

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
        .map_err(|e| format!("ERR_INTERNAL|http client: {e}"))
}

async fn chat(
    provider: &LlmProvider,
    system: Option<&str>,
    user: &str,
) -> Result<String, String> {
    if !is_configured(provider) {
        return Err(format!("ERR_LLM_NO_KEY|{}", provider.name.trim()));
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
        .map_err(|e| format!("ERR_LLM_NETWORK|{e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        eprintln!(
            "anthropic LLM failed (HTTP {}): {}",
            status.as_u16(),
            truncate(&text, 300)
        );
        return Err(format!("ERR_LLM_HTTP|{}", status.as_u16()));
    }
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("ERR_LLM_NETWORK|invalid response: {e}"))?;
    v["content"][0]["text"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "ERR_LLM_NETWORK|no text in response".to_string())
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
    let mut req = client()?.post(&url).header("content-type", "application/json");
    if !provider.api_key.trim().is_empty() {
        req = req.bearer_auth(provider.api_key.trim());
    }
    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("ERR_LLM_NETWORK|{e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        eprintln!(
            "openai LLM failed (HTTP {}): {}",
            status.as_u16(),
            truncate(&text, 300)
        );
        return Err(format!("ERR_LLM_HTTP|{}", status.as_u16()));
    }
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("ERR_LLM_NETWORK|invalid response: {e}"))?;
    v["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "ERR_LLM_NETWORK|no text in response".to_string())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect::<String>() + "…"
    }
}
