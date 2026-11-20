//! Local speech.cpp engines (v0.18): `speech serve` (nyosegawa/speech.cpp, a
//! ggml runtime for speech models, one executable without Python or CUDA)
//! runs one Japanese recognizer and answers the OpenAI-style
//! `/v1/audio/transcriptions`:
//!
//! - engine "haas": Haas-ja v1.0 (Third Intelligence, FastConformer 616M).
//!   The most accurate of the small Japanese models upstream (FLEURS ja CER
//!   4.4% vs. parakeet-ja 5.0%), with punctuation, and the only local engine
//!   that takes a custom vocabulary (`keywords[]`), which the term dictionary
//!   feeds.
//! - engine "reazon": ReazonSpeech v2 (reazon-research, NeMo RNN-T 619M),
//!   trained on Japanese TV, with punctuation.
//!
//! Built like the CrispASR engines (`crisp.rs`): its own install folder
//! (`{app_data}/speechcpp/`), process slot, port and settings (`stt.speech`,
//! shared by both engines — one server runs at a time, the engine picks the
//! model).

use crate::settings::Settings;
use crate::setup::{self, FlagReset, ManagedServer};
use serde::Serialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

pub struct SpeechModel {
    /// Same as the `stt.engine` value that uses it.
    pub id: &'static str,
    repo: &'static str,
    /// Hugging Face commit the file is pinned to (speech.cpp 0.11.0's catalog).
    rev: &'static str,
    file: &'static str,
    sha256: &'static str,
    pub size_mb: u64,
}

/// Q8_0 conversions by the speech.cpp author; repository, commit and SHA-256
/// from `tools/catalog/catalog.json` of speech.cpp v0.11.0 (its default
/// type for both, checked against the official NeMo text upstream). The Haas
/// file must be from 0.9.4 or later to hold the custom vocabulary.
pub const MODELS: &[SpeechModel] = &[
    SpeechModel {
        id: "haas",
        repo: "sakasegawa/Haas-ja-v1.0-GGUF",
        rev: "6b965d5007ba780585de51549d57a8c1c0d67c7e",
        file: "Haas-616M-ja-v1.0-Q8_0.gguf",
        sha256: "cd5f60b2f1c411404b83fca4a65e3ce30a26d9e951b6a346529e65f22758afeb",
        size_mb: 626,
    },
    SpeechModel {
        id: "reazon",
        repo: "sakasegawa/reazonspeech-nemo-v2-GGUF",
        rev: "ed04d9ee2cd156414982a7c0f6123c6fd1093904",
        file: "reazonspeech-nemo-619M-v2-Q8_0.gguf",
        sha256: "944411cb740938f858c939aa1d42f7eb4bb884fc350d9daae8b7e724853e54a3",
        size_mb: 631,
    },
];

/// The engines served by this module.
pub fn is_speech_engine(engine: &str) -> bool {
    MODELS.iter().any(|m| m.id == engine)
}

/// Pinned speech.cpp release. Windows x64 has only the Vulkan build (it needs
/// the GPU driver's `vulkan-1.dll`; `--device cpu` still runs on the CPU).
/// Linux x64 has a Vulkan build, which does not start without
/// `libvulkan.so.1`, and a CPU build; the installer picks by the loader, as
/// upstream's `install.sh` does. No arm64 builds (Windows or Linux).
const SPEECH_TAG: &str = "v0.11.0";
const SPEECH_VERSION: &str = "0.11.0";

/// (platform, SHA-256 from the release's SHA256SUMS)
fn speech_asset() -> Option<(&'static str, &'static str)> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        Some((
            "windows-x64-vulkan",
            "b64f498179fc46d08149317eaea9fe6f24e6fa0cd194557b374c97e76ccc9c67",
        ))
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        Some(if crate::hw::get().vulkan {
            (
                "linux-x64-vulkan",
                "9c01a35a0c79d36ca29abe379029c71b0cfb950cbc9da835863f5f7d28c81ae8",
            )
        } else {
            (
                "linux-x64-cpu",
                "9acb8becee605c7bb190d7f56d4d9212232ec311df63735b1938cc86bc4257a1",
            )
        })
    }
    #[cfg(not(any(all(windows, target_arch = "x86_64"), all(target_os = "linux", target_arch = "x86_64"))))]
    {
        None
    }
}

/// Written next to the exe at install: the platform of the build installed.
const BUILD_MARKER: &str = "fire-speak-build.txt";

const HEALTH_TIMEOUT_SECS: u64 = 120;
const STT_TIMEOUT_SECS: u64 = 120;

const SETUP_REQUIRED_MSG: &str = "ERR_SETUP_REQUIRED";

static SERVER: Mutex<Option<ManagedServer>> = Mutex::new(None);
/// Serializes server starts and the install swap.
static START_LOCK: Mutex<()> = Mutex::new(());
static SERVER_DOWNLOADING: AtomicBool = AtomicBool::new(false);
static MODEL_DOWNLOADS: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[derive(Debug, Clone, Serialize)]
pub struct SpeechModelInfo {
    pub id: String,
    pub installed: bool,
    pub size_mb: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpeechStatus {
    /// A speech.cpp build exists for this platform (Windows x64 / Linux x64).
    pub available: bool,
    pub server_installed: bool,
    pub server_path: String,
    pub models: Vec<SpeechModelInfo>,
    pub server_running: bool,
    /// What `stt.speech.accel` resolves to here: "gpu" | "cpu".
    pub effective_accel: String,
}

fn speech_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(setup::app_data_dir(app)?.join("speechcpp"))
}

fn server_exe_name() -> &'static str {
    if cfg!(windows) {
        "speech.exe"
    } else {
        "speech"
    }
}

fn find_exe(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        } else if path.file_name().and_then(|n| n.to_str()) == Some(server_exe_name()) {
            return Some(path);
        }
    }
    dirs.iter().find_map(|d| find_exe(d))
}

fn server_path(app: &AppHandle) -> Option<PathBuf> {
    find_exe(&speech_dir(app).ok()?)
}

/// The installed build has a GPU backend (every build but Linux's CPU one).
fn installed_has_gpu(app: &AppHandle) -> bool {
    let Some(exe) = server_path(app) else {
        return speech_asset().is_some_and(|(p, _)| p.ends_with("vulkan"));
    };
    let marker = exe.with_file_name(BUILD_MARKER);
    std::fs::read_to_string(marker)
        .map(|s| s.trim().ends_with("vulkan"))
        .unwrap_or(cfg!(windows))
}

fn model_def(id: &str) -> Option<&'static SpeechModel> {
    MODELS.iter().find(|m| m.id == id)
}

fn model_path(app: &AppHandle, id: &str) -> Option<PathBuf> {
    let path = setup::models_dir(app).ok()?.join(model_def(id)?.file);
    path.exists().then_some(path)
}

pub fn effective_accel(app: &AppHandle, settings: &Settings) -> &'static str {
    if !installed_has_gpu(app) {
        return "cpu";
    }
    match settings.stt.speech.accel.as_str() {
        "gpu" => "gpu",
        "cpu" => "cpu",
        // same rule as Qwen / CrispASR: an iGPU can be slower than the CPU
        _ => {
            let hw = crate::hw::get();
            if hw.nvidia || hw.amd_gpu {
                "gpu"
            } else {
                "cpu"
            }
        }
    }
}

fn running() -> bool {
    SERVER
        .lock()
        .ok()
        .and_then(|mut g| g.as_mut().map(|m| matches!(m.child.try_wait(), Ok(None))))
        .unwrap_or(false)
}

pub fn status(app: &AppHandle, settings: &Settings) -> SpeechStatus {
    let server = server_path(app);
    SpeechStatus {
        available: speech_asset().is_some(),
        server_installed: server.is_some(),
        server_path: server
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        models: MODELS
            .iter()
            .map(|m| SpeechModelInfo {
                id: m.id.to_string(),
                installed: model_path(app, m.id).is_some(),
                size_mb: m.size_mb,
            })
            .collect(),
        server_running: running(),
        effective_accel: effective_accel(app, settings).to_string(),
    }
}

// ---------------------------------------------------------------------------
// Process management
// ---------------------------------------------------------------------------

pub async fn ensure_server(app: AppHandle) -> Result<u16, String> {
    let settings = app
        .state::<crate::AppState>()
        .settings
        .lock()
        .unwrap()
        .clone();
    tokio::task::spawn_blocking(move || ensure_server_blocking(&app, &settings))
        .await
        .map_err(|e| format!("ERR_INTERNAL|server start: {e}"))?
}

fn ensure_server_blocking(app: &AppHandle, settings: &Settings) -> Result<u16, String> {
    let c = &settings.stt.speech;
    let exe = server_path(app).ok_or_else(|| SETUP_REQUIRED_MSG.to_string())?;
    let model =
        model_path(app, &settings.stt.engine).ok_or_else(|| SETUP_REQUIRED_MSG.to_string())?;
    let accel = effective_accel(app, settings);
    let signature = format!(
        "{}|{}|{}|{}|{}",
        exe.display(),
        model.display(),
        c.server_port,
        c.threads,
        accel
    );

    let _start = START_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let stale = {
        let mut guard = SERVER.lock().unwrap_or_else(|e| e.into_inner());
        let reusable = guard.as_mut().is_some_and(|m| {
            m.signature == signature
                && matches!(m.child.try_wait(), Ok(None))
                && health(c.server_port) == Some(200)
        });
        if reusable {
            return Ok(c.server_port);
        }
        guard.take()
    };
    if let Some(mut m) = stale {
        setup::terminate(&mut m.child);
    }

    if setup::tcp_ok(c.server_port, Duration::from_millis(300)) {
        std::thread::sleep(Duration::from_millis(400));
        if setup::tcp_ok(c.server_port, Duration::from_millis(300)) {
            return Err(format!("ERR_PORT_IN_USE|{}", c.server_port));
        }
    }

    // `speech serve FILE.gguf` loads (and warms up) the model before it
    // listens, so /health answering 200 means ready.
    let mut cmd = Command::new(&exe);
    cmd.arg("serve")
        .arg(&model)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(c.server_port.to_string())
        .arg("--threads")
        .arg(c.threads.to_string())
        .arg("--device")
        .arg(accel)
        // the app updates the pinned build itself
        .env("SPEECH_NO_UPDATE_CHECK", "1");
    let mut managed = setup::spawn_bound(cmd, &exe, signature)?;

    let deadline = Instant::now() + Duration::from_secs(HEALTH_TIMEOUT_SECS);
    loop {
        if let Ok(Some(status)) = managed.child.try_wait() {
            eprintln!("speech serve exited immediately (exit: {status})");
            return Err("ERR_SERVER_DIED".to_string());
        }
        if health(c.server_port) == Some(200) {
            break;
        }
        if Instant::now() >= deadline {
            setup::terminate(&mut managed.child);
            return Err(format!(
                "ERR_SERVER_START|health check timeout ({HEALTH_TIMEOUT_SECS}s)"
            ));
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    *SERVER.lock().unwrap_or_else(|e| e.into_inner()) = Some(managed);
    let _ = app.emit("server-state", true);
    Ok(c.server_port)
}

/// HTTP status of `GET /health`, or None when nothing answers.
fn health(port: u16) -> Option<u16> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream =
        std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(400)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    stream
        .write_all(b"GET /health HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
        .ok()?;
    let mut head = [0u8; 16];
    let n = stream.read(&mut head).ok()?;
    std::str::from_utf8(&head[..n])
        .ok()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

pub fn kill(app: &AppHandle) {
    let taken = SERVER.lock().ok().and_then(|mut g| g.take());
    if let Some(mut m) = taken {
        setup::terminate(&mut m.child);
        let _ = app.emit("server-state", false);
    }
}

/// Start the server in the background and run one second of silence through
/// it. No-op unless a speech.cpp engine is selected and its server and model
/// are installed.
pub fn prewarm(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let settings = app.state::<crate::AppState>().settings.lock().unwrap().clone();
        if !is_speech_engine(&settings.stt.engine)
            || server_path(&app).is_none()
            || model_path(&app, &settings.stt.engine).is_none()
        {
            return;
        }
        if let Ok(wav) = crate::audio::wav_bytes(&[0i16; 16000]) {
            if let Err(e) = transcribe(&app, &settings, wav, None).await {
                eprintln!("speech.cpp prewarm failed: {e}");
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Transcription
// ---------------------------------------------------------------------------

/// `prompt` is the STT vocabulary hint (`context::stt_hint`): terms joined by
/// "、" / ", ". Haas takes them as its custom vocabulary (`keywords[]`), with
/// the kana readings the glossary and the term dictionary know; ReazonSpeech
/// takes no vocabulary, so the dictionary aliases and the LLM term-fix pass
/// do that work afterwards.
///
/// `language` is never sent: both models are Japanese only, and speech.cpp
/// answers a language the model lacks with a 400. Nor is `temperature`,
/// which speech.cpp refuses rather than ignores.
pub async fn transcribe(
    app: &AppHandle,
    settings: &Settings,
    wav: Vec<u8>,
    prompt: Option<&str>,
) -> Result<crate::stt::Transcript, String> {
    let port = ensure_server(app.clone()).await?;
    let keywords = if settings.stt.engine == "haas" {
        prompt
            .map(|p| {
                let dict = crate::dictionary::load(app);
                haas_keywords(&prompt_terms(p), &dict)
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    match transcribe_at(port, wav.clone(), &keywords).await {
        // a keyword the model cannot take (its vocabulary refuses it) must not
        // cost the dictation: once more without any
        Err(e) if !keywords.is_empty() && e == "ERR_STT_HTTP|400" => {
            eprintln!("speech.cpp: keywords refused; retrying without them");
            transcribe_at(port, wav, &[]).await
        }
        r => r,
    }
}

async fn transcribe_at(
    port: u16,
    wav: Vec<u8>,
    keywords: &[String],
) -> Result<crate::stt::Transcript, String> {
    let file_part = reqwest::multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| format!("ERR_INTERNAL|multipart: {e}"))?;
    let mut form = reqwest::multipart::Form::new().text("response_format", "json");
    for k in keywords {
        form = form.text("keywords[]", k.clone());
    }
    form = form.part("file", file_part);

    let resp = reqwest::Client::builder()
        .timeout(Duration::from_secs(STT_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("ERR_INTERNAL|http client: {e}"))?
        .post(format!("http://127.0.0.1:{port}/v1/audio/transcriptions"))
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("ERR_STT_NETWORK|{e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        eprintln!(
            "speech.cpp STT failed (HTTP {}): {}",
            status.as_u16(),
            text.chars().take(200).collect::<String>()
        );
        return Err(format!("ERR_STT_HTTP|{}", status.as_u16()));
    }
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("ERR_STT_NETWORK|invalid response: {e}"))?;
    Ok(crate::stt::Transcript {
        text: v["text"].as_str().unwrap_or("").trim().to_string(),
        no_speech: false,
    })
}

/// The terms of an STT prompt (`context::stt_hint` joins them with "、" and
/// ends with "。", or ", " and ".").
fn prompt_terms(prompt: &str) -> Vec<String> {
    let body = prompt.trim();
    let body = body.strip_suffix('。').or_else(|| body.strip_suffix('.')).unwrap_or(body);
    let sep = if body.contains('、') { "、" } else { ", " };
    body.split(sep)
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Kana only (hiragana, katakana, ー): what Haas writes for a word by sound.
fn is_kana(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| matches!(c as u32, 0x3041..=0x3096 | 0x30A1..=0x30FA | 0x30FC))
}

/// One `keywords[]` value per term, `term=reading,reading`, the readings
/// being the kana spellings the built-in glossary (readings and aliases) and
/// the term dictionary (learned misrecognitions) hold for it. A term without
/// one is sent alone: Haas boosts its written form. A term holding the
/// syntax's own characters is left out.
fn haas_keywords(terms: &[String], dict: &[crate::dictionary::DictEntry]) -> Vec<String> {
    let mut out = Vec::new();
    for term in terms {
        if term.contains(['=', ',', '\n', '\r']) {
            continue;
        }
        let mut readings: Vec<&str> = Vec::new();
        for e in crate::glossary::ENTRIES.iter().filter(|e| e.term == term) {
            readings.extend(e.readings.iter().chain(e.aliases.iter()).copied());
        }
        for e in dict.iter().filter(|e| &e.term == term) {
            readings.extend(e.aliases.iter().map(String::as_str));
        }
        let mut kana: Vec<&str> = Vec::new();
        for r in readings {
            if is_kana(r) && r != term && !kana.contains(&r) {
                kana.push(r);
            }
        }
        out.push(if kana.is_empty() {
            term.clone()
        } else {
            format!("{term}={}", kana.join(","))
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Downloads
// ---------------------------------------------------------------------------

/// Install the pinned speech.cpp build into `{app_data}/speechcpp/` (staged
/// in `speechcpp.new/`, verified, then swapped in). A second call while one
/// runs joins it.
pub async fn download_server(app: AppHandle) -> Result<(), String> {
    if SERVER_DOWNLOADING.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let result = {
        let _reset = FlagReset(&SERVER_DOWNLOADING);
        download_server_inner(&app).await
    };
    match &result {
        Ok(()) => prewarm(&app),
        Err(e) => setup::emit_progress(&app, "speech-server", "speechcpp", 0, 0, true, Some(e)),
    }
    result
}

async fn download_server_inner(app: &AppHandle) -> Result<(), String> {
    let _ = tokio::task::spawn_blocking(crate::hw::get).await;
    let (platform, sha256) = speech_asset().ok_or_else(|| "ERR_NO_SERVER_ASSET".to_string())?;
    let asset = format!("speech-{SPEECH_VERSION}-{platform}.zip");
    let url =
        format!("https://github.com/nyosegawa/speech.cpp/releases/download/{SPEECH_TAG}/{asset}");
    let dir = speech_dir(app)?;
    let staging = dir.with_file_name("speechcpp.new");
    if staging.exists() {
        let _ = tokio::fs::remove_dir_all(&staging).await;
    }
    tokio::fs::create_dir_all(&staging)
        .await
        .map_err(|e| format!("ERR_FILE_IO|create dir: {e}"))?;
    let archive = staging.join("_speech.zip");

    let staged = async {
        let client = setup::download_client()?;
        let (downloaded, total) =
            setup::stream_download(app, &client, &url, &archive, "speech-server", "speechcpp")
                .await?;
        let a = archive.clone();
        let actual = tokio::task::spawn_blocking(move || setup::sha256_file(&a))
            .await
            .map_err(|e| format!("ERR_INTERNAL|hash: {e}"))??;
        if actual != sha256 {
            return Err(format!("ERR_DOWNLOAD_FAILED|checksum mismatch for {asset}"));
        }
        let (a, s) = (archive.clone(), staging.clone());
        tokio::task::spawn_blocking(move || setup::extract_zip(&a, &s))
            .await
            .map_err(|e| format!("ERR_INTERNAL|extract: {e}"))??;
        let _ = tokio::fs::remove_file(&archive).await;
        let exe = find_exe(&staging).ok_or_else(|| "ERR_NO_SERVER_ASSET".to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| format!("ERR_FILE_IO|chmod server: {e}"))?;
        }
        std::fs::write(exe.with_file_name(BUILD_MARKER), platform)
            .map_err(|e| format!("ERR_FILE_IO|write marker: {e}"))?;
        Ok::<(u64, u64), String>((downloaded, total))
    }
    .await;
    let (downloaded, total) = match staged {
        Ok(v) => v,
        Err(e) => {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            return Err(e);
        }
    };

    let app2 = app.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let _start = START_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        kill(&app2);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)
                .map_err(|e| format!("ERR_FILE_IO|remove old server: {e}"))?;
        }
        std::fs::rename(&staging, &dir).map_err(|e| format!("ERR_FILE_IO|install server: {e}"))
    })
    .await
    .map_err(|e| format!("ERR_INTERNAL|install: {e}"))??;

    setup::emit_progress(app, "speech-server", "speechcpp", downloaded, total.max(downloaded), true, None);
    Ok(())
}

/// Download the model of engine `id` (verified by SHA-256). Progress events
/// carry kind "speech-model" and name `id`. Joins a running download.
pub async fn download_model(app: AppHandle, id: String) -> Result<(), String> {
    let model = model_def(&id).ok_or_else(|| format!("ERR_INTERNAL|unknown model: {id}"))?;
    {
        let mut list = MODEL_DOWNLOADS.lock().unwrap();
        if list.contains(&id) {
            return Ok(());
        }
        list.push(id.clone());
    }
    let result = download_model_inner(&app, model).await;
    MODEL_DOWNLOADS.lock().unwrap().retain(|m| m != &id);
    match &result {
        Ok(()) => prewarm(&app),
        Err(e) => setup::emit_progress(&app, "speech-model", &id, 0, 0, true, Some(e)),
    }
    result
}

async fn download_model_inner(app: &AppHandle, m: &SpeechModel) -> Result<(), String> {
    let dir = setup::models_dir(app)?;
    let dest = dir.join(m.file);
    if dest.exists() {
        setup::emit_progress(app, "speech-model", m.id, 0, 0, true, None);
        return Ok(());
    }
    let client = setup::download_client()?;
    let part = dir.join(format!("{}.part", m.file));
    let url = format!("https://huggingface.co/{}/resolve/{}/{}", m.repo, m.rev, m.file);
    let (downloaded, total) =
        setup::stream_download(app, &client, &url, &part, "speech-model", m.id).await?;
    let p = part.clone();
    let actual = tokio::task::spawn_blocking(move || setup::sha256_file(&p))
        .await
        .map_err(|e| format!("ERR_INTERNAL|hash: {e}"))??;
    if actual != m.sha256 {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(format!("ERR_DOWNLOAD_FAILED|checksum mismatch for {}", m.file));
    }
    tokio::fs::rename(&part, &dest)
        .await
        .map_err(|e| format!("ERR_FILE_IO|rename model: {e}"))?;
    setup::emit_progress(app, "speech-model", m.id, downloaded, total.max(downloaded), true, None);
    Ok(())
}

fn downloading_model(id: &str) -> bool {
    MODEL_DOWNLOADS.lock().unwrap().iter().any(|m| m == id)
}

/// One-click setup for engine = haas / reazon: server, then the engine's
/// model, then prewarm. Steps already done are skipped.
pub async fn quick_setup(app: AppHandle) -> Result<(), String> {
    if server_path(&app).is_none() {
        download_server(app.clone()).await?;
        while SERVER_DOWNLOADING.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        if server_path(&app).is_none() {
            return Err(SETUP_REQUIRED_MSG.to_string());
        }
    }
    let id = app.state::<crate::AppState>().settings.lock().unwrap().stt.engine.clone();
    if model_path(&app, &id).is_none() {
        download_model(app.clone(), id.clone()).await?;
        while downloading_model(&id) {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        if model_path(&app, &id).is_none() {
            return Err(SETUP_REQUIRED_MSG.to_string());
        }
    }
    prewarm(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engines_are_known() {
        assert!(is_speech_engine("haas"));
        assert!(is_speech_engine("reazon"));
        assert!(!is_speech_engine("parakeet"));
        assert!(!is_speech_engine("qwen"));
        assert!(!is_speech_engine("local"));
        // no engine id is served by two modules
        for m in MODELS {
            assert!(!crate::crisp::is_crisp_engine(m.id));
        }
    }

    #[test]
    fn prompt_terms_split_both_styles() {
        assert_eq!(prompt_terms("Claude Code、渋谷、fire-speak。"), ["Claude Code", "渋谷", "fire-speak"]);
        assert_eq!(prompt_terms("Next.js, Tauri."), ["Next.js", "Tauri"]);
        assert_eq!(prompt_terms("Tauri"), ["Tauri"]);
        assert!(prompt_terms("。").is_empty());
    }

    #[test]
    fn keywords_carry_kana_readings() {
        let dict = vec![crate::dictionary::DictEntry {
            term: "Qwen3-ASR".to_string(),
            aliases: vec!["クエン三ASR".to_string(), "クウェンスリーエーエスアール".to_string()],
            source: "auto".to_string(),
            hits: 0,
            added: 0,
            last_used: 0,
        }];
        let terms: Vec<String> =
            ["Claude", "Qwen3-ASR", "渋谷", "a=b"].iter().map(|s| s.to_string()).collect();
        let k = haas_keywords(&terms, &dict);
        // glossary readings; the mixed-script alias is not a reading
        assert_eq!(k[0], "Claude=クロード");
        assert_eq!(k[1], "Qwen3-ASR=クウェンスリーエーエスアール");
        assert_eq!(k[2], "渋谷");
        assert_eq!(k.len(), 3);
        assert!(k.iter().all(|s| !s.contains('\n')));
    }

    #[test]
    fn kana_check() {
        assert!(is_kana("クロード"));
        assert!(is_kana("はーす"));
        assert!(!is_kana("クエン三ASR"));
        assert!(!is_kana(""));
    }

    /// Against a `speech serve` already listening on 8181:
    /// `FIRE_SPEAK_SPEECH_WAV=<wav> cargo test --lib speechcpp::tests::live_server -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_server() {
        let wav = std::fs::read(std::env::var("FIRE_SPEAK_SPEECH_WAV").unwrap()).unwrap();
        let t = transcribe_at(8181, wav.clone(), &[]).await.unwrap();
        println!("plain: {:?}", t);
        assert!(!t.text.is_empty());
        let k = vec!["Claude=クロード".to_string()];
        println!("keywords: {:?}", transcribe_at(8181, wav, &k).await.unwrap());
    }
}
