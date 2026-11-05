//! Local llama.cpp speech engine (engine "qwen", v0.12): `llama-server` runs
//! a GGUF speech LLM and takes the recording as an `input_audio` part on
//! `/v1/chat/completions`. Models: Qwen3-ASR 0.6B / 1.7B and, since v0.16,
//! NVIDIA Nemotron 3.5 Transcribe 30B-A4B (IQ4_NL; 40 languages, MoE).
//!
//! Kept apart from the whisper engine on purpose: its own install folder
//! (`{app_data}/llama/` — `setup::find_server_exe` scans `bin/` for any
//! `*server*.exe`, and the llama.cpp archive also ships `ggml-rpc-server.exe`),
//! its own process slot, port and settings (`stt.qwen`).

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Family {
    /// Qwen3-ASR: `language X<asr_text>` output, language forced by an
    /// assistant prefill, vocabulary hint as the system message.
    Qwen,
    /// Nemotron 3.5 Transcribe (v0.16): plain-text instruction next to the
    /// audio, thinking off, MoE experts that may live in system RAM.
    Nemotron,
}

pub struct QwenModel {
    pub id: &'static str,
    pub family: Family,
    repo: &'static str,
    /// Hugging Face commit the files are pinned to.
    rev: &'static str,
    model_file: &'static str,
    model_sha256: &'static str,
    mmproj_file: &'static str,
    mmproj_sha256: &'static str,
    /// Model + mmproj, for the settings screen.
    pub size_mb: u64,
}

/// Q8_0 conversions by ggml-org (sizes and SHA-256 from the Hugging Face LFS
/// metadata; 0.6B checked against a local download).
pub const MODELS: &[QwenModel] = &[
    QwenModel {
        id: "0.6b",
        family: Family::Qwen,
        repo: "ggml-org/Qwen3-ASR-0.6B-GGUF",
        rev: "928ab958557df9aa2ef1c93e0e83c7ad0933fae2",
        model_file: "Qwen3-ASR-0.6B-Q8_0.gguf",
        model_sha256: "bca259818b50ca7c4c05e9bdb35a5dc04fa039653a6d6f3f0f331f96f6aa1971",
        mmproj_file: "mmproj-Qwen3-ASR-0.6B-Q8_0.gguf",
        mmproj_sha256: "41a342b5e4c514e968cb756de6cd1b7be39eff43c44c57a2ef5fc6522e36603d",
        size_mb: 972,
    },
    QwenModel {
        id: "1.7b",
        family: Family::Qwen,
        repo: "ggml-org/Qwen3-ASR-1.7B-GGUF",
        rev: "36a678687ba7d07a74ca70ccb0e36902e005fb80",
        model_file: "Qwen3-ASR-1.7B-Q8_0.gguf",
        model_sha256: "58e22d0532d4eacaf034cfac17a6fed159f37c41390c710186783be439d1fc57",
        mmproj_file: "mmproj-Qwen3-ASR-1.7B-Q8_0.gguf",
        mmproj_sha256: "46c1d533af3f354ceb37ce855dbceff7da7fa7cf1e6a523df3b13440bd164c0d",
        size_mb: 2404,
    },
    // IQ4_NL is within 0.2 points of Q8_0 on every set (sakasegawa's card;
    // llama.cpp #31471: LibriSpeech 1.02 vs 1.03, AMI cpWER 10.38 vs 9.91)
    // at about half the size. Lower quants (IQ3_NL / IQ2_NL) need the
    // unmerged llama.cpp #27322. The mmproj holds the audio + speaker
    // encoders; F16 is what #31471 validated with.
    QwenModel {
        id: "nemotron",
        family: Family::Nemotron,
        repo: NEMOTRON_REPO,
        rev: NEMOTRON_REV,
        model_file: NEMOTRON_MODEL_FILE,
        model_sha256: NEMOTRON_MODEL_SHA256,
        mmproj_file: NEMOTRON_MMPROJ_FILE,
        mmproj_sha256: NEMOTRON_MMPROJ_SHA256,
        size_mb: NEMOTRON_SIZE_MB,
    },
];

// PIN: fill from the Hugging Face LFS metadata (see `pins_are_filled`).
const NEMOTRON_REPO: &str = "sakasegawa/NVIDIA-Nemotron-3.5-Transcribe-30B-A4B-GGUF";
const NEMOTRON_REV: &str = "PIN_ME";
const NEMOTRON_MODEL_FILE: &str = "PIN_ME";
const NEMOTRON_MODEL_SHA256: &str = "PIN_ME";
const NEMOTRON_MMPROJ_FILE: &str = "PIN_ME";
const NEMOTRON_MMPROJ_SHA256: &str = "PIN_ME";
/// Model + mmproj (IQ4_NL ~19.0 GB + F16 mmproj ~1.47 GB, unconfirmed).
const NEMOTRON_SIZE_MB: u64 = 19_500;

/// llama.cpp build pinned like the Lemonade whisper builds: the Vulkan
/// archive also carries every CPU backend (ggml loads backends at runtime),
/// so one download serves GPU (`auto`/`gpu`) and CPU (`-dev none`).
///
/// Vulkan only, on every platform (v0.12.2, measured on an RTX 4070 Ti SUPER
/// with 15 s of Japanese): CUDA 12.4 ran no faster (0.6B 0.17 s vs 0.15 s,
/// 1.7B 0.29 s vs 0.31 s) for a 630 MB download, and the OpenVINO build
/// (Intel GPU/NPU) fails on the audio encoder ("failed to process mtmd
/// chunk"). llama.cpp has no AMD NPU backend.
///
/// v0.16: bumped past ggml-org/llama.cpp#31471 (merged 2026-11-03, commit
/// 28b812d) for Nemotron 3.5 Transcribe. Installs older than this tag are
/// replaced in the background (`server_outdated`).
// PIN: the first release containing 28b812d, and its four asset hashes.
const LLAMA_TAG: &str = "PIN_ME";

fn llama_asset() -> Option<(&'static str, &'static str)> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        Some((
            "llama-PIN_ME-bin-win-vulkan-x64.zip",
            "PIN_ME",
        ))
    }
    #[cfg(all(windows, target_arch = "aarch64"))]
    {
        Some((
            "llama-PIN_ME-bin-win-vulkan-arm64.zip",
            "PIN_ME",
        ))
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        Some((
            "llama-PIN_ME-bin-ubuntu-vulkan-x64.tar.gz",
            "PIN_ME",
        ))
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        Some((
            "llama-PIN_ME-bin-ubuntu-vulkan-arm64.tar.gz",
            "PIN_ME",
        ))
    }
    #[cfg(not(any(
        all(windows, any(target_arch = "x86_64", target_arch = "aarch64")),
        all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))
    )))]
    {
        None
    }
}

/// Total context, split evenly across the slots (8,192 each). Qwen3-ASR
/// spends about 13 tokens per second of audio, so the 5-minute recording cap
/// is ~3,900 prompt tokens plus the transcript.
const CTX_SIZE: &str = "16384";
/// Parallel slots: the live caption request and the final transcription.
const SLOTS: &str = "2";
const MAX_TOKENS: u32 = 4096;
const HEALTH_TIMEOUT_SECS: u64 = 120;
/// Nemotron reads ~20 GB at start; a SATA SSD needs a minute or more.
const HEALTH_TIMEOUT_BIG_SECS: u64 = 600;
/// Marker written next to the installed build: the `LLAMA_TAG` it came from.
const TAG_FILE: &str = ".fire-speak-llama-tag";
const STT_TIMEOUT_SECS: u64 = 120;

const SETUP_REQUIRED_MSG: &str = "ERR_SETUP_REQUIRED";

static SERVER: Mutex<Option<ManagedServer>> = Mutex::new(None);
/// Serializes server starts and the install swap (a start must not spawn the
/// exe that a download is about to replace).
static START_LOCK: Mutex<()> = Mutex::new(());
static SERVER_DOWNLOADING: AtomicBool = AtomicBool::new(false);
static MODEL_DOWNLOADS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static MOE_PLACEMENT: Mutex<String> = Mutex::new(String::new());

#[derive(Debug, Clone, Serialize)]
pub struct QwenModelInfo {
    pub id: String,
    pub installed: bool,
    pub size_mb: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct QwenStatus {
    pub server_installed: bool,
    pub server_path: String,
    /// The model chosen in `stt.qwen.model` is on disk.
    pub model_installed: bool,
    pub models: Vec<QwenModelInfo>,
    pub server_running: bool,
    /// What `stt.qwen.accel` resolves to here: "gpu" | "cpu".
    pub effective_accel: String,
    /// The installed llama.cpp predates `LLAMA_TAG` (v0.16): it cannot run
    /// Nemotron and is replaced in the background.
    pub server_outdated: bool,
    /// How the MoE experts of the running Nemotron server are placed:
    /// "fit" (llama.cpp sized them to the free VRAM), "cpu" (all experts in
    /// RAM, the fallback), a layer count (manual), or "" (not running).
    pub moe_placement: String,
}

fn llama_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(setup::app_data_dir(app)?.join("llama"))
}

fn server_exe_name() -> &'static str {
    if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
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
    find_exe(&llama_dir(app).ok()?)
}

/// The installed build predates `LLAMA_TAG` (no marker = installed by
/// v0.15 or older).
pub fn server_outdated(app: &AppHandle) -> bool {
    let Ok(dir) = llama_dir(app) else { return false };
    if find_exe(&dir).is_none() {
        return false;
    }
    std::fs::read_to_string(dir.join(TAG_FILE)).map(|t| t.trim() != LLAMA_TAG).unwrap_or(true)
}

pub fn model_def(id: &str) -> Option<&'static QwenModel> {
    MODELS.iter().find(|m| m.id == id)
}

/// (model, mmproj) paths of `id` when both are on disk.
fn model_paths(app: &AppHandle, id: &str) -> Option<(PathBuf, PathBuf)> {
    let m = model_def(id)?;
    let dir = setup::models_dir(app).ok()?;
    let (model, mmproj) = (dir.join(m.model_file), dir.join(m.mmproj_file));
    (model.exists() && mmproj.exists()).then_some((model, mmproj))
}

pub fn effective_accel(settings: &Settings) -> &'static str {
    match settings.stt.qwen.accel.as_str() {
        "gpu" => "gpu",
        "cpu" => "cpu",
        _ => {
            // An iGPU (Intel, the 2-CU Radeon of desktop Ryzen) can be slower
            // than the CPU; only offload to a real GPU.
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

pub fn status(app: &AppHandle, settings: &Settings) -> QwenStatus {
    let server = server_path(app);
    QwenStatus {
        server_installed: server.is_some(),
        server_path: server
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        model_installed: model_paths(app, &settings.stt.qwen.model).is_some(),
        models: MODELS
            .iter()
            .map(|m| QwenModelInfo {
                id: m.id.to_string(),
                installed: model_paths(app, m.id).is_some(),
                size_mb: m.size_mb,
            })
            .collect(),
        server_running: running(),
        effective_accel: effective_accel(settings).to_string(),
        server_outdated: server_outdated(app),
        moe_placement: if running() {
            MOE_PLACEMENT.lock().map(|p| p.clone()).unwrap_or_default()
        } else {
            String::new()
        },
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
    let q = &settings.stt.qwen;
    let exe = server_path(app).ok_or_else(|| SETUP_REQUIRED_MSG.to_string())?;
    let def = model_def(&q.model).ok_or_else(|| SETUP_REQUIRED_MSG.to_string())?;
    if def.family == Family::Nemotron && server_outdated(app) {
        // too old to know the architecture; sync_server_build replaces it
        return Err(SETUP_REQUIRED_MSG.to_string());
    }
    let (model, mmproj) =
        model_paths(app, &q.model).ok_or_else(|| SETUP_REQUIRED_MSG.to_string())?;
    let accel = effective_accel(settings);
    let signature = format!(
        "{}|{}|{}|{}|{}|{}",
        exe.display(),
        model.display(),
        q.server_port,
        q.threads,
        accel,
        q.n_cpu_moe
    );

    let _start = START_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let stale = {
        let mut guard = SERVER.lock().unwrap_or_else(|e| e.into_inner());
        let reusable = guard.as_mut().is_some_and(|m| {
            m.signature == signature
                && matches!(m.child.try_wait(), Ok(None))
                && health(q.server_port) == Some(200)
        });
        if reusable {
            return Ok(q.server_port);
        }
        guard.take()
    };
    if let Some(mut m) = stale {
        setup::terminate(&mut m.child);
    }

    if setup::tcp_ok(q.server_port, Duration::from_millis(300)) {
        std::thread::sleep(Duration::from_millis(400));
        if setup::tcp_ok(q.server_port, Duration::from_millis(300)) {
            return Err(format!("ERR_PORT_IN_USE|{}", q.server_port));
        }
    }

    let plans = moe_plans(def.family, accel, q.n_cpu_moe);
    let mut last_err = String::new();
    for (i, (placement, extra)) in plans.iter().enumerate() {
        let mut cmd = Command::new(&exe);
        cmd.arg("-m")
            .arg(&model)
            .arg("--mmproj")
            .arg(&mmproj)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(q.server_port.to_string())
            .arg("-t")
            .arg(q.threads.to_string())
            .arg("-c")
            .arg(CTX_SIZE)
            .arg("-np")
            .arg(SLOTS)
            .arg("--no-webui");
        if accel == "cpu" {
            cmd.arg("-dev").arg("none");
        }
        if def.family == Family::Nemotron {
            // the chat template thinks by default; without this the server
            // returns only reasoning and no transcript (llama.cpp #31471)
            cmd.arg("--reasoning").arg("off");
        }
        cmd.args(extra);
        let timeout = if def.family == Family::Nemotron {
            HEALTH_TIMEOUT_BIG_SECS
        } else {
            HEALTH_TIMEOUT_SECS
        };
        match spawn_and_wait(cmd, &exe, signature.clone(), q.server_port, timeout) {
            Ok(managed) => {
                *MOE_PLACEMENT.lock().unwrap_or_else(|e| e.into_inner()) = placement.clone();
                *SERVER.lock().unwrap_or_else(|e| e.into_inner()) = Some(managed);
                let _ = app.emit("server-state", true);
                return Ok(q.server_port);
            }
            Err(e) => {
                if i + 1 < plans.len() {
                    eprintln!("llama-server start ({placement}) failed: {e}; retrying with experts in RAM");
                }
                last_err = e;
            }
        }
    }
    Err(last_err)
}

/// Launch plans for the MoE expert weights, tried in order: (placement
/// label for the UI, extra args).
///
/// Nemotron 3.5 Transcribe is 30B with ~4B active, so the ~18 GB of
/// experts can sit in system RAM while attention, Mamba and the shared
/// weights stay on the GPU (`--n-cpu-moe`). `n_cpu_moe` < 0 is automatic:
/// llama.cpp's `--fit on` sizes the offload to the free VRAM; when that
/// start fails (out of VRAM, or a build without `--fit`), every expert
/// layer goes to RAM, which needs only a few GB of VRAM. On the CPU, and
/// for the dense Qwen3-ASR models, there is nothing to place.
pub fn moe_plans(family: Family, accel: &str, n_cpu_moe: i32) -> Vec<(String, Vec<String>)> {
    if family != Family::Nemotron || accel == "cpu" {
        return vec![(String::new(), Vec::new())];
    }
    let all_cpu = ("cpu".to_string(), vec!["--n-cpu-moe".to_string(), "999".to_string()]);
    if n_cpu_moe >= 0 {
        return vec![(
            n_cpu_moe.to_string(),
            vec!["--n-cpu-moe".to_string(), n_cpu_moe.to_string()],
        )];
    }
    vec![("fit".to_string(), vec!["--fit".to_string(), "on".to_string()]), all_cpu]
}

/// Spawn and wait for `/health` 200 — the port opens before the model is
/// loaded (503 "Loading model" until then).
fn spawn_and_wait(
    cmd: Command,
    exe: &Path,
    signature: String,
    port: u16,
    timeout_secs: u64,
) -> Result<ManagedServer, String> {
    let mut managed = setup::spawn_bound(cmd, exe, signature)?;
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        if let Ok(Some(status)) = managed.child.try_wait() {
            eprintln!("llama-server exited immediately (exit: {status})");
            return Err("ERR_SERVER_DIED".to_string());
        }
        if health(port) == Some(200) {
            return Ok(managed);
        }
        if Instant::now() >= deadline {
            setup::terminate(&mut managed.child);
            return Err(format!("ERR_SERVER_START|health check timeout ({timeout_secs}s)"));
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

/// HTTP status of `GET /health`, or None when nothing answers. Plain
/// HTTP/1.0 over a socket: this runs on a blocking thread.
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
    // "HTTP/1.1 200 OK"
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
/// it, so the first dictation does not wait for the model load or the GPU
/// shader compile (~7 s with Vulkan). No-op unless engine = qwen and the
/// server and model are installed.
pub fn prewarm(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let settings = app.state::<crate::AppState>().settings.lock().unwrap().clone();
        if settings.stt.engine != "qwen"
            || server_path(&app).is_none()
            || model_paths(&app, &settings.stt.qwen.model).is_none()
        {
            return;
        }
        if let Ok(wav) = crate::audio::wav_bytes(&[0i16; 16000]) {
            if let Err(e) = transcribe(&app, &settings, wav, None).await {
                eprintln!("qwen prewarm failed: {e}");
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Transcription
// ---------------------------------------------------------------------------

/// Qwen3-ASR language names for the whisper codes in `Settings::language`.
fn language_name(code: &str) -> Option<&'static str> {
    Some(match code {
        "ja" => "Japanese",
        "en" => "English",
        "zh" => "Chinese",
        "ko" => "Korean",
        "es" => "Spanish",
        "fr" => "French",
        "de" => "German",
        "pt" => "Portuguese",
        "ru" => "Russian",
        "vi" => "Vietnamese",
        "id" => "Indonesian",
        _ => return None,
    })
}

/// Split Qwen3-ASR's raw output `language Japanese<asr_text>…` into the
/// detected language and the text. Output without the tag (an endpoint that
/// already strips it) is all text.
pub fn parse_asr_output(raw: &str) -> (Option<String>, String) {
    match raw.split_once("<asr_text>") {
        Some((head, text)) => {
            let lang = head
                .trim()
                .strip_prefix("language")
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty());
            (lang, text.trim().to_string())
        }
        None => (None, raw.trim().to_string()),
    }
}

/// `prompt` (the clipboard vocabulary hint) goes in as the system message —
/// Qwen3-ASR's context-biasing input. A fixed recognition language is forced
/// by prefilling the assistant turn with its language tag.
pub async fn transcribe(
    app: &AppHandle,
    settings: &Settings,
    wav: Vec<u8>,
    prompt: Option<&str>,
) -> Result<crate::stt::Transcript, String> {
    let port = ensure_server(app.clone()).await?;
    match model_def(&settings.stt.qwen.model).map(|m| m.family) {
        Some(Family::Nemotron) => transcribe_nemotron_at(port, &settings.language, wav, prompt).await,
        _ => transcribe_at(port, &settings.language, wav, prompt).await,
    }
}

/// The instruction for Nemotron 3.5 Transcribe: the model card's plain
/// transcription prompt, the recognition language when fixed, and the
/// vocabulary hint as prompt context (the model takes free-text context).
pub fn nemotron_instruction(language: &str, prompt: Option<&str>) -> String {
    let mut s = String::from("Transcribe the following audio with punctuation.");
    if let Some(name) = language_name(language) {
        s.push_str(&format!(" The speech is in {name}."));
    }
    s.push_str(" Output only the transcript.");
    if let Some(p) = prompt.map(str::trim).filter(|p| !p.is_empty()) {
        s.push_str(&format!(" Words that may appear: {p}"));
    }
    s
}

/// Drop a reasoning block and speaker tags (`<spk:0>`) if the model emits
/// them anyway.
pub fn clean_nemotron_output(raw: &str) -> String {
    let mut s = raw.to_string();
    if let Some(end) = s.find("</think>") {
        s = s[end + "</think>".len()..].to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(i) = rest.find("<spk:") {
        out.push_str(&rest[..i]);
        match rest[i..].find('>') {
            Some(j) => rest = &rest[i + j + 1..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ").trim().to_string()
}

async fn transcribe_nemotron_at(
    port: u16,
    language: &str,
    wav: Vec<u8>,
    prompt: Option<&str>,
) -> Result<crate::stt::Transcript, String> {
    use base64::Engine;
    let body = serde_json::json!({
        "messages": [{
            "role": "user",
            "content": [
                { "type": "text", "text": nemotron_instruction(language, prompt) },
                {
                    "type": "input_audio",
                    "input_audio": {
                        "data": base64::engine::general_purpose::STANDARD.encode(&wav),
                        "format": "wav",
                    },
                },
            ],
        }],
        "temperature": 0,
        "max_tokens": MAX_TOKENS,
        "chat_template_kwargs": { "enable_thinking": false },
    });
    let v = post_chat(port, &body).await?;
    let raw = v["choices"][0]["message"]["content"].as_str().unwrap_or("");
    let text = clean_nemotron_output(raw);
    // no "no speech" tag; the speech gate before STT handles silence
    Ok(crate::stt::Transcript { text, no_speech: false })
}

async fn post_chat(port: u16, body: &serde_json::Value) -> Result<serde_json::Value, String> {
    let resp = reqwest::Client::builder()
        .timeout(Duration::from_secs(STT_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("ERR_INTERNAL|http client: {e}"))?
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .json(body)
        .send()
        .await
        .map_err(|e| format!("ERR_STT_NETWORK|{e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        eprintln!(
            "llama STT failed (HTTP {}): {}",
            status.as_u16(),
            text.chars().take(200).collect::<String>()
        );
        return Err(format!("ERR_STT_HTTP|{}", status.as_u16()));
    }
    serde_json::from_str(&text).map_err(|e| format!("ERR_STT_NETWORK|invalid response: {e}"))
}

async fn transcribe_at(
    port: u16,
    language: &str,
    wav: Vec<u8>,
    prompt: Option<&str>,
) -> Result<crate::stt::Transcript, String> {
    use base64::Engine;
    let mut messages = Vec::new();
    if let Some(p) = prompt.filter(|p| !p.trim().is_empty()) {
        messages.push(serde_json::json!({ "role": "system", "content": p }));
    }
    messages.push(serde_json::json!({
        "role": "user",
        "content": [{
            "type": "input_audio",
            "input_audio": {
                "data": base64::engine::general_purpose::STANDARD.encode(&wav),
                "format": "wav",
            },
        }],
    }));
    if let Some(name) = language_name(language) {
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": format!("language {name}<asr_text>"),
        }));
    }
    let body = serde_json::json!({
        "messages": messages,
        "temperature": 0,
        "max_tokens": MAX_TOKENS,
    });
    let v = post_chat(port, &body).await?;
    let raw = v["choices"][0]["message"]["content"].as_str().unwrap_or("");
    let (lang, text) = parse_asr_output(raw);
    // Qwen3-ASR tags audio it hears no speech in as "language None".
    let no_speech = lang.as_deref() == Some("None");
    Ok(crate::stt::Transcript { text, no_speech })
}

// ---------------------------------------------------------------------------
// Downloads
// ---------------------------------------------------------------------------

/// Install the pinned llama.cpp build into `{app_data}/llama/` (staged in
/// `llama.new/`, verified, then swapped in). A second call while one runs
/// joins it (returns Ok; the running one's events report the outcome).
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
        Err(e) => setup::emit_progress(&app, "qwen-server", "llama-server", 0, 0, true, Some(e)),
    }
    result
}

async fn download_server_inner(app: &AppHandle) -> Result<(), String> {
    let (asset, sha256) = llama_asset().ok_or_else(|| "ERR_NO_SERVER_ASSET".to_string())?;
    let url = format!("https://github.com/ggml-org/llama.cpp/releases/download/{LLAMA_TAG}/{asset}");
    let dir = llama_dir(app)?;
    let staging = dir.with_file_name("llama.new");
    if staging.exists() {
        let _ = tokio::fs::remove_dir_all(&staging).await;
    }
    tokio::fs::create_dir_all(&staging)
        .await
        .map_err(|e| format!("ERR_FILE_IO|create dir: {e}"))?;
    let archive = staging.join(if cfg!(windows) { "_llama.zip" } else { "_llama.tar.gz" });

    let staged = async {
        let client = setup::download_client()?;
        let (downloaded, total) =
            setup::stream_download(app, &client, &url, &archive, "qwen-server", asset).await?;
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

    // Swap with no server start in between; the running server holds the
    // old exe open (Windows), so stop it first.
    let app2 = app.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let _start = START_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        kill(&app2);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)
                .map_err(|e| format!("ERR_FILE_IO|remove old server: {e}"))?;
        }
        std::fs::rename(&staging, &dir).map_err(|e| format!("ERR_FILE_IO|install server: {e}"))?;
        std::fs::write(dir.join(TAG_FILE), LLAMA_TAG)
            .map_err(|e| format!("ERR_FILE_IO|write tag: {e}"))
    })
    .await
    .map_err(|e| format!("ERR_INTERNAL|install: {e}"))??;

    setup::emit_progress(app, "qwen-server", asset, downloaded, total.max(downloaded), true, None);
    Ok(())
}

/// Download model + mmproj of `id` (each verified by SHA-256). Progress
/// events carry kind "qwen-model" and name `id`; the mmproj comes first, so
/// the bar runs twice. Joins a running download of the same model.
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
        Err(e) => setup::emit_progress(&app, "qwen-model", &id, 0, 0, true, Some(e)),
    }
    result
}

async fn download_model_inner(app: &AppHandle, m: &QwenModel) -> Result<(), String> {
    let client = setup::download_client()?;
    let dir = setup::models_dir(app)?;
    let mut done = (0u64, 0u64);
    for (file, sha256) in [(m.mmproj_file, m.mmproj_sha256), (m.model_file, m.model_sha256)] {
        let dest = dir.join(file);
        if dest.exists() {
            continue;
        }
        let part = dir.join(format!("{file}.part"));
        let url = format!("https://huggingface.co/{}/resolve/{}/{file}", m.repo, m.rev);
        let (downloaded, total) =
            setup::stream_download(app, &client, &url, &part, "qwen-model", m.id).await?;
        let p = part.clone();
        let actual = tokio::task::spawn_blocking(move || setup::sha256_file(&p))
            .await
            .map_err(|e| format!("ERR_INTERNAL|hash: {e}"))??;
        if actual != sha256 {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(format!("ERR_DOWNLOAD_FAILED|checksum mismatch for {file}"));
        }
        tokio::fs::rename(&part, &dest)
            .await
            .map_err(|e| format!("ERR_FILE_IO|rename model: {e}"))?;
        done = (done.0 + downloaded, done.1 + total.max(downloaded));
    }
    setup::emit_progress(app, "qwen-model", m.id, done.0, done.1, true, None);
    Ok(())
}

fn downloading_model(id: &str) -> bool {
    MODEL_DOWNLOADS.lock().unwrap().iter().any(|m| m == id)
}

/// One-click setup for engine = qwen: server, then the selected model, then
/// prewarm. Steps already done are skipped.
pub async fn quick_setup(app: AppHandle) -> Result<(), String> {
    if server_path(&app).is_none() || server_outdated(&app) {
        download_server(app.clone()).await?;
        while SERVER_DOWNLOADING.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        if server_path(&app).is_none() || server_outdated(&app) {
            return Err(SETUP_REQUIRED_MSG.to_string());
        }
    }
    let id = app.state::<crate::AppState>().settings.lock().unwrap().stt.qwen.model.clone();
    if model_paths(&app, &id).is_none() {
        download_model(app.clone(), id.clone()).await?;
        while downloading_model(&id) {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        if model_paths(&app, &id).is_none() {
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
    fn parses_language_tag() {
        let (lang, text) = parse_asr_output("language Japanese<asr_text>明日の会議は午後三時からです。");
        assert_eq!(lang.as_deref(), Some("Japanese"));
        assert_eq!(text, "明日の会議は午後三時からです。");
        let (lang, text) = parse_asr_output("language None<asr_text>");
        assert_eq!(lang.as_deref(), Some("None"));
        assert_eq!(text, "");
        let (lang, text) = parse_asr_output(" hello ");
        assert_eq!(lang, None);
        assert_eq!(text, "hello");
    }

    /// Against a llama-server with Qwen3-ASR already listening on 8179:
    /// `FIRE_SPEAK_QWEN_WAV=<16 kHz wav> cargo test --lib live_server -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_server() {
        let wav = std::fs::read(std::env::var("FIRE_SPEAK_QWEN_WAV").unwrap()).unwrap();
        let t = transcribe_at(8179, "ja", wav.clone(), Some("fire-speak")).await.unwrap();
        println!("ja+prompt: {:?}", t);
        assert!(!t.text.is_empty() && !t.text.contains("<asr_text>"));
        let t = transcribe_at(8179, "auto", wav, None).await.unwrap();
        println!("auto: {:?}", t);
        let silence = crate::audio::wav_bytes(&[0i16; 16000]).unwrap();
        println!("silence: {:?}", transcribe_at(8179, "ja", silence, None).await.unwrap());
    }

    #[test]
    fn nemotron_prompt_and_cleanup() {
        let p = nemotron_instruction("ja", Some("fire-speak, Qwen3-ASR"));
        assert!(p.contains("Japanese") && p.contains("fire-speak, Qwen3-ASR"));
        assert!(!nemotron_instruction("auto", None).contains("speech is in"));
        assert_eq!(clean_nemotron_output("<think>x</think> 明日 です。"), "明日 です。");
        assert_eq!(clean_nemotron_output("<spk:0> hello <spk:1> world"), "hello world");
        assert_eq!(clean_nemotron_output("  plain  "), "plain");
    }

    #[test]
    fn moe_plans_by_mode() {
        let none = moe_plans(Family::Qwen, "gpu", -1);
        assert_eq!(none.len(), 1);
        assert!(none[0].1.is_empty());
        assert!(moe_plans(Family::Nemotron, "cpu", -1)[0].1.is_empty());
        let auto = moe_plans(Family::Nemotron, "gpu", -1);
        assert_eq!(auto[0].1, ["--fit", "on"]);
        assert_eq!(auto[1].1, ["--n-cpu-moe", "999"]);
        assert_eq!(moe_plans(Family::Nemotron, "gpu", 20)[0].1, ["--n-cpu-moe", "20"]);
    }

    /// Fails until the Nemotron files and the llama.cpp build are pinned, so
    /// a release cannot ship placeholders.
    #[test]
    fn pins_are_filled() {
        let mut all: Vec<&str> = vec![LLAMA_TAG];
        if let Some((a, h)) = llama_asset() {
            all.push(a);
            all.push(h);
        }
        for m in MODELS {
            all.extend([m.rev, m.model_file, m.model_sha256, m.mmproj_file, m.mmproj_sha256]);
            assert_eq!(m.model_sha256.len(), 64, "{}", m.id);
            assert_eq!(m.mmproj_sha256.len(), 64, "{}", m.id);
        }
        assert!(all.iter().all(|s| !s.contains("PIN_ME")), "unpinned: {all:?}");
    }

    #[test]
    fn models_are_known() {
        assert!(model_def("nemotron").is_some_and(|m| m.family == Family::Nemotron));
        assert!(model_def("0.6b").is_some());
        assert!(model_def("1.7b").is_some());
        assert!(model_def("small").is_none());
        assert_eq!(language_name("ja"), Some("Japanese"));
        assert_eq!(language_name("auto"), None);
    }
}
