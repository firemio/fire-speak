use crate::settings::Settings;
use std::time::Duration;
use tauri::AppHandle;

const STT_TIMEOUT_SECS: u64 = 120;

/// Segments whose `no_speech_prob` is above this count as "no speech"; a
/// result where every segment is above it is a hallucination (v0.10.2, the
/// same rule as OpenAI's own decoder fallback).
const NO_SPEECH_PROB: f64 = 0.5;

/// A transcription plus whisper's own opinion on whether it heard speech.
#[derive(Debug, Clone, Default)]
pub struct Transcript {
    pub text: String,
    /// True when the response carried segments and every one of them had
    /// `no_speech_prob > 0.5`. False when the endpoint sends no segments.
    pub no_speech: bool,
}

/// Parse a `json` or `verbose_json` transcription response.
pub fn parse_transcript(v: &serde_json::Value) -> Transcript {
    let text = v["text"].as_str().unwrap_or("").to_string();
    let segments = v["segments"].as_array();
    let no_speech = segments.is_some_and(|segs| {
        !segs.is_empty()
            && segs
                .iter()
                .all(|seg| seg["no_speech_prob"].as_f64().unwrap_or(0.0) > NO_SPEECH_PROB)
    });
    Transcript { text, no_speech }
}

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
) -> Result<Transcript, String> {
    match settings.stt.engine.as_str() {
        "cloud" => transcribe_cloud(settings, wav).await,
        _ => transcribe_local(app, settings, wav).await,
    }
}

pub async fn transcribe_local(
    app: &AppHandle,
    settings: &Settings,
    wav: Vec<u8>,
) -> Result<Transcript, String> {
    let port = crate::setup::ensure_server(app.clone()).await?;

    let file_part = reqwest::multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| format!("ERR_INTERNAL|multipart: {e}"))?;
    // verbose_json adds per-segment no_speech_prob (hallucination check), but
    // a VAD-enabled whisper-server crashes producing it when VAD found no
    // speech — with VAD the server already drops non-speech, so plain json.
    let format = if crate::setup::server_vad_active() { "json" } else { "verbose_json" };
    let mut form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("response_format", format)
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
    Ok(parse_transcript(&v))
}

pub async fn transcribe_cloud(settings: &Settings, wav: Vec<u8>) -> Result<Transcript, String> {
    let cloud = &settings.stt.cloud;
    if cloud.api_key.trim().is_empty() {
        return Err("ERR_STT_NO_KEY".to_string());
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
        // OpenAI and Groq return segments with no_speech_prob for this
        // format; an endpoint that sends only `text` still parses fine.
        .text("response_format", "verbose_json")
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
    Ok(parse_transcript(&v))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect::<String>() + "…"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_speech_needs_every_segment_above_threshold() {
        let v: serde_json::Value = serde_json::json!({
            "text": " ご視聴ありがとうございました",
            "segments": [{"text": " ご視聴ありがとうございました", "no_speech_prob": 0.93}]
        });
        assert!(parse_transcript(&v).no_speech);
        let v: serde_json::Value = serde_json::json!({
            "text": "a b",
            "segments": [{"no_speech_prob": 0.9}, {"no_speech_prob": 0.1}]
        });
        assert!(!parse_transcript(&v).no_speech);
        // plain json (no segments) never claims silence
        let v: serde_json::Value = serde_json::json!({"text": "hello"});
        let t = parse_transcript(&v);
        assert!(!t.no_speech);
        assert_eq!(t.text, "hello");
        let v: serde_json::Value = serde_json::json!({"text": "", "segments": []});
        assert!(!parse_transcript(&v).no_speech);
    }
}
