//! Local CrispASR engines (v0.14): `crispasr --server` (CrispStrobe/CrispASR,
//! a ggml runtime for many ASR model families) runs NVIDIA Parakeet TDT-CTC
//! 0.6B Japanese (engine "parakeet") or Cohere Transcribe 03-2026 (engine
//! "cohere") and answers the OpenAI-style `/v1/audio/transcriptions`.
//!
//! Built like the Qwen3-ASR engine (`qwen.rs`): its own install folder
//! (`{app_data}/crispasr/`), process slot, port and settings (`stt.crisp`,
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

pub struct CrispModel {
    /// Same as the `stt.engine` value that uses it.
    pub id: &'static str,
    repo: &'static str,
    /// Hugging Face commit the file is pinned to.
    rev: &'static str,
    file: &'static str,
    sha256: &'static str,
    pub size_mb: u64,
}

/// Q8_0 conversions by the CrispASR author (SHA-256 from the Hugging Face LFS
/// metadata, checked against local downloads). Q8_0 decodes like F16 on
/// Parakeet; its Q4_K loops in TDT decoding.
pub const MODELS: &[CrispModel] = &[
    CrispModel {
        id: "parakeet",
        repo: "cstr/parakeet-tdt-0.6b-ja-GGUF",
        rev: "d9e3ba65a6579796389ea89e5939509ed257f972",
        file: "parakeet-tdt-0.6b-ja-q8_0.gguf",
        sha256: "5a61e6c7d956c3c72a76fafcd798cac0c9ea66d0e29b3910cd04865a1e42cc17",
        size_mb: 643,
    },
    CrispModel {
        id: "cohere",
        repo: "cstr/cohere-transcribe-03-2026-GGUF",
        rev: "4e98a8c31c95a92d651e60e3bfae4fe29fc6e026",
        file: "cohere-transcribe-q8_0.gguf",
        sha256: "e725a5ec9b5b9c0b7aba07b071753cae9b39ec3fbaf9f6a622c6810d5a11e3f4",
        size_mb: 2312,
    },
];

/// The engines served by this module.
pub fn is_crisp_engine(engine: &str) -> bool {
    MODELS.iter().any(|m| m.id == engine)
}

/// Pinned CrispASR release. Windows x64 / Linux x64 use the Vulkan build
/// (it also carries the CPU backend, `-ng` forces it); Linux arm64 has only
/// a CPU build. There is no Windows arm64 build.
const CRISP_TAG: &str = "v0.8.43";

fn crisp_asset() -> Option<(&'static str, &'static str)> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        Some((
            "crispasr-windows-x86_64-vulkan.zip",
            "9294c7c6cca319714da2ae913a8839d81d53e2e9583020162ff9573c47654fc8",
        ))
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        Some((
            "crispasr-linux-x86_64-vulkan.tar.gz",
            "d5c676e023df6fac61971dcef06fb7cce22ff3de95367f8e01efb8743c11ca46",
        ))
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        Some((
            "crispasr-linux-arm64.tar.gz",
            "ae6e3f13e795fa6dec55d20c8bf1c96fca0bd581eab1495f344cb9f9195ec2d2",
        ))
    }
    #[cfg(not(any(
        all(windows, target_arch = "x86_64"),
        all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))
    )))]
    {
        None
    }
}

/// Whether this platform's build has a GPU backend at all.
fn asset_has_gpu() -> bool {
    !cfg!(all(target_os = "linux", target_arch = "aarch64"))
}

const HEALTH_TIMEOUT_SECS: u64 = 120;
const STT_TIMEOUT_SECS: u64 = 120;

const SETUP_REQUIRED_MSG: &str = "ERR_SETUP_REQUIRED";

static SERVER: Mutex<Option<ManagedServer>> = Mutex::new(None);
/// Serializes server starts and the install swap.
static START_LOCK: Mutex<()> = Mutex::new(());
static SERVER_DOWNLOADING: AtomicBool = AtomicBool::new(false);
static MODEL_DOWNLOADS: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[derive(Debug, Clone, Serialize)]
pub struct CrispModelInfo {
    pub id: String,
    pub installed: bool,
    pub size_mb: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CrispStatus {
    pub server_installed: bool,
    pub server_path: String,
    pub models: Vec<CrispModelInfo>,
    pub server_running: bool,
    /// What `stt.crisp.accel` resolves to here: "gpu" | "cpu".
    pub effective_accel: String,
}

fn crisp_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(setup::app_data_dir(app)?.join("crispasr"))
}

fn server_exe_name() -> &'static str {
    if cfg!(windows) {
        "crispasr.exe"
    } else {
        "crispasr"
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
    find_exe(&crisp_dir(app).ok()?)
}

fn model_def(id: &str) -> Option<&'static CrispModel> {
    MODELS.iter().find(|m| m.id == id)
}

fn model_path(app: &AppHandle, id: &str) -> Option<PathBuf> {
    let path = setup::models_dir(app).ok()?.join(model_def(id)?.file);
    path.exists().then_some(path)
}

pub fn effective_accel(settings: &Settings) -> &'static str {
    if !asset_has_gpu() {
        return "cpu";
    }
    match settings.stt.crisp.accel.as_str() {
        "gpu" => "gpu",
        "cpu" => "cpu",
        // same rule as Qwen: an iGPU can be slower than the CPU
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

pub fn status(app: &AppHandle, settings: &Settings) -> CrispStatus {
    let server = server_path(app);
    CrispStatus {
        server_installed: server.is_some(),
        server_path: server
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        models: MODELS
            .iter()
            .map(|m| CrispModelInfo {
                id: m.id.to_string(),
                installed: model_path(app, m.id).is_some(),
                size_mb: m.size_mb,
            })
            .collect(),
        server_running: running(),
        effective_accel: effective_accel(settings).to_string(),
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
    let c = &settings.stt.crisp;
    let exe = server_path(app).ok_or_else(|| SETUP_REQUIRED_MSG.to_string())?;
    let model =
        model_path(app, &settings.stt.engine).ok_or_else(|| SETUP_REQUIRED_MSG.to_string())?;
    let accel = effective_accel(settings);
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

    let mut cmd = Command::new(&exe);
    cmd.arg("--server")
        .arg("-m")
        .arg(&model)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(c.server_port.to_string())
        .arg("-t")
        .arg(c.threads.to_string());
    if accel == "cpu" {
        cmd.arg("-ng");
    }
    // the Linux tarballs carry their own libopenblas/libgomp next to the exe
    #[cfg(unix)]
    if let Some(dir) = exe.parent() {
        cmd.env("LD_LIBRARY_PATH", dir);
    }
    let mut managed = setup::spawn_bound(cmd, &exe, signature)?;

    let deadline = Instant::now() + Duration::from_secs(HEALTH_TIMEOUT_SECS);
    loop {
        if let Ok(Some(status)) = managed.child.try_wait() {
            eprintln!("crispasr exited immediately (exit: {status})");
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
/// it (model load, Vulkan shader compile). No-op unless a CrispASR engine is
/// selected and its server and model are installed.
pub fn prewarm(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let settings = app.state::<crate::AppState>().settings.lock().unwrap().clone();
        if !is_crisp_engine(&settings.stt.engine)
            || server_path(&app).is_none()
            || model_path(&app, &settings.stt.engine).is_none()
        {
            return;
        }
        if let Ok(wav) = crate::audio::wav_bytes(&[0i16; 16000]) {
            if let Err(e) = transcribe(&app, &settings, wav).await {
                eprintln!("crispasr prewarm failed: {e}");
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Transcription
// ---------------------------------------------------------------------------

/// Neither model takes a vocabulary prompt (CrispASR ignores `prompt` for
/// them), so the clipboard hint is not sent; the dictionary aliases and the
/// LLM term-fix pass still apply afterwards. Cohere has no language detection:
/// with language "auto" CrispASR falls back to its own default.
pub async fn transcribe(
    app: &AppHandle,
    settings: &Settings,
    wav: Vec<u8>,
) -> Result<crate::stt::Transcript, String> {
    let port = ensure_server(app.clone()).await?;
    transcribe_at(port, &settings.language, wav).await
}

async fn transcribe_at(
    port: u16,
    language: &str,
    wav: Vec<u8>,
) -> Result<crate::stt::Transcript, String> {
    let file_part = reqwest::multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| format!("ERR_INTERNAL|multipart: {e}"))?;
    let mut form = reqwest::multipart::Form::new()
        .text("response_format", "json")
        .text("temperature", "0");
    if language != "auto" && !language.trim().is_empty() {
        form = form.text("language", language.to_string());
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
            "crispasr STT failed (HTTP {}): {}",
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

// ---------------------------------------------------------------------------
// Downloads
// ---------------------------------------------------------------------------

/// Install the pinned CrispASR build into `{app_data}/crispasr/` (staged in
/// `crispasr.new/`, verified, then swapped in). A second call while one runs
/// joins it.
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
        Err(e) => setup::emit_progress(&app, "crisp-server", "crispasr", 0, 0, true, Some(e)),
    }
    result
}

async fn download_server_inner(app: &AppHandle) -> Result<(), String> {
    let (asset, sha256) = crisp_asset().ok_or_else(|| "ERR_NO_SERVER_ASSET".to_string())?;
    let url =
        format!("https://github.com/CrispStrobe/CrispASR/releases/download/{CRISP_TAG}/{asset}");
    let dir = crisp_dir(app)?;
    let staging = dir.with_file_name("crispasr.new");
    if staging.exists() {
        let _ = tokio::fs::remove_dir_all(&staging).await;
    }
    tokio::fs::create_dir_all(&staging)
        .await
        .map_err(|e| format!("ERR_FILE_IO|create dir: {e}"))?;
    let archive = staging.join(if cfg!(windows) { "_crispasr.zip" } else { "_crispasr.tar.gz" });

    let staged = async {
        let client = setup::download_client()?;
        let (downloaded, total) =
            setup::stream_download(app, &client, &url, &archive, "crisp-server", asset).await?;
        let a = archive.clone();
        let actual = tokio::task::spawn_blocking(move || setup::sha256_file(&a))
            .await
            .map_err(|e| format!("ERR_INTERNAL|hash: {e}"))??;
        if actual != sha256 {
            return Err(format!("ERR_DOWNLOAD_FAILED|checksum mismatch for {asset}"));
        }
        let (a, s) = (archive.clone(), staging.clone());
        tokio::task::spawn_blocking(move || setup::extract_server_archive(&a, &s))
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
        #[cfg(not(unix))]
        let _ = exe;
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

    setup::emit_progress(app, "crisp-server", asset, downloaded, total.max(downloaded), true, None);
    Ok(())
}

/// Download the model of engine `id` (verified by SHA-256). Progress events
/// carry kind "crisp-model" and name `id`. Joins a running download.
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
        Err(e) => setup::emit_progress(&app, "crisp-model", &id, 0, 0, true, Some(e)),
    }
    result
}

async fn download_model_inner(app: &AppHandle, m: &CrispModel) -> Result<(), String> {
    let dir = setup::models_dir(app)?;
    let dest = dir.join(m.file);
    if dest.exists() {
        setup::emit_progress(app, "crisp-model", m.id, 0, 0, true, None);
        return Ok(());
    }
    let client = setup::download_client()?;
    let part = dir.join(format!("{}.part", m.file));
    let url = format!("https://huggingface.co/{}/resolve/{}/{}", m.repo, m.rev, m.file);
    let (downloaded, total) =
        setup::stream_download(app, &client, &url, &part, "crisp-model", m.id).await?;
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
    setup::emit_progress(app, "crisp-model", m.id, downloaded, total.max(downloaded), true, None);
    Ok(())
}

fn downloading_model(id: &str) -> bool {
    MODEL_DOWNLOADS.lock().unwrap().iter().any(|m| m == id)
}

/// One-click setup for engine = parakeet / cohere: server, then the engine's
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
        assert!(is_crisp_engine("parakeet"));
        assert!(is_crisp_engine("cohere"));
        assert!(!is_crisp_engine("qwen"));
        assert!(!is_crisp_engine("local"));
    }

    /// Against a crispasr server already listening on 8180:
    /// `FIRE_SPEAK_CRISP_WAV=<16 kHz wav> cargo test --lib crisp::tests::live_server -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_server() {
        let wav = std::fs::read(std::env::var("FIRE_SPEAK_CRISP_WAV").unwrap()).unwrap();
        let t = transcribe_at(8180, "ja", wav.clone()).await.unwrap();
        println!("ja: {:?}", t);
        assert!(!t.text.is_empty());
        println!("auto: {:?}", transcribe_at(8180, "auto", wav).await.unwrap());
    }
}
