use crate::settings::Settings;
use std::time::Duration;
use tauri::AppHandle;

const STT_TIMEOUT_SECS: u64 = 120;

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(STT_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("ERR_INTERNAL|http client: {e}"))
}

/// Transcribe a 16kHz mono WAV according to the configured engine.
pub async fn transcribe(
    app: &AppHandle,
    settings: &Settings,
    wav: Vec<u8>,
) -> Result<String, String> {
    match settings.stt.engine.as_str() {
        "cloud" => transcribe_cloud(settings, wav).await,
        _ => transcribe_local(app, settings, wav).await,
    }
}

pub async fn transcribe_local(
    app: &AppHandle,
    settings: &Settings,
    wav: Vec<u8>,
) -> Result<String, String> {
    let port = crate::setup::ensure_server(app.clone()).await?;

    let file_part = reqwest::multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| format!("ERR_INTERNAL|multipart: {e}"))?;
    let mut form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("response_format", "json")
        .text("temperature", "0.0");
    if settings.language != "auto" && !settings.language.trim().is_empty() {
        form = form.text("language", settings.language.clone());
    }

    let resp = client()?
        .post(format!("http://127.0.0.1:{port}/inference"))
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("ERR_STT_NETWORK|{e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        eprintln!(
            "local STT failed (HTTP {}): {}",
            status.as_u16(),
            truncate(&text, 200)
        );
        return Err(format!("ERR_STT_HTTP|{}", status.as_u16()));
    }
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("ERR_STT_NETWORK|invalid response: {e}"))?;
    Ok(v["text"].as_str().unwrap_or("").to_string())
}

pub async fn transcribe_cloud(settings: &Settings, wav: Vec<u8>) -> Result<String, String> {
    let cloud = &settings.stt.cloud;
    if cloud.api_key.trim().is_empty() {
        return Err("ERR_INTERNAL|cloud STT API key not set".to_string());
    }
    let url = format!(
        "{}/audio/transcriptions",
        cloud.base_url.trim_end_matches('/')
    );

    let file_part = reqwest::multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| format!("ERR_INTERNAL|multipart: {e}"))?;
    let mut form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("model", cloud.model.clone());
    if settings.language != "auto" && !settings.language.trim().is_empty() {
        form = form.text("language", settings.language.clone());
    }

    let resp = client()?
        .post(&url)
        .bearer_auth(cloud.api_key.trim())
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("ERR_STT_NETWORK|{e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        eprintln!(
            "cloud STT failed (HTTP {}): {}",
            status.as_u16(),
            truncate(&text, 200)
        );
        return Err(format!("ERR_STT_HTTP|{}", status.as_u16()));
    }
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("ERR_STT_NETWORK|invalid response: {e}"))?;
    Ok(v["text"].as_str().unwrap_or("").to_string())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect::<String>() + "…"
    }
}
