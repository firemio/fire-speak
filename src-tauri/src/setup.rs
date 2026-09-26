use crate::settings::Settings;
use futures_util::StreamExt;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;

pub const MODELS: &[(&str, u64)] = &[
    ("tiny", 78),
    ("base", 148),
    ("small", 488),
    ("medium", 1533),
    ("large-v3-turbo", 1624),
];

/// Preference order when the user did not pin a model path.
const MODEL_PREFERENCE: &[&str] = &["large-v3-turbo", "medium", "small", "base", "tiny"];

const SETUP_REQUIRED_MSG: &str = "ERR_SETUP_REQUIRED";

pub struct ManagedServer {
    pub child: Child,
    pub signature: String,
    /// Job object with kill-on-close: keeps the whisper-server bound to this
    /// process's lifetime even on crash / Task Manager kill. Must stay alive
    /// as long as the child runs.
    ///
    /// The Linux equivalent needs no handle: `PR_SET_PDEATHSIG` is set in the
    /// child itself (see `spawn_and_health_check`).
    #[cfg(windows)]
    #[allow(dead_code)]
    pub job: win32job::Job,
}

/// Fork the whisper-server from a dedicated, never-exiting thread (Linux).
///
/// `PR_SET_PDEATHSIG` is delivered when the *thread* that forked the child
/// exits — not when the process does (see prctl(2)). `ensure_server` runs on a
/// tokio `spawn_blocking` thread, which the runtime retires after a few
/// seconds of idleness, so forking there would have the server SIGKILL itself
/// shortly after every successful start. A single long-lived spawner thread,
/// created on first use and never joined, removes that hazard.
#[cfg(target_os = "linux")]
fn spawn_child(cmd: Command) -> std::io::Result<Child> {
    use std::sync::mpsc::{channel, Sender};
    use std::sync::OnceLock;

    type Request = (Command, Sender<std::io::Result<Child>>);
    static SPAWNER: OnceLock<Sender<Request>> = OnceLock::new();
    let gone = || std::io::Error::other("server spawner thread unavailable");

    let spawner = SPAWNER.get_or_init(|| {
        let (tx, rx) = channel::<Request>();
        std::thread::spawn(move || {
            while let Ok((mut cmd, reply)) = rx.recv() {
                let _ = reply.send(cmd.spawn());
            }
        });
        tx
    });
    let (reply_tx, reply_rx) = channel();
    spawner.send((cmd, reply_tx)).map_err(|_| gone())?;
    reply_rx.recv().map_err(|_| gone())?
}

#[cfg(not(target_os = "linux"))]
fn spawn_child(mut cmd: Command) -> std::io::Result<Child> {
    cmd.spawn()
}

/// Kill a managed child and reap it.
///
/// On Linux the child is its own process group leader (see the `pre_exec`
/// below), so the whole group is signalled first — whisper-server's worker
/// processes, if it ever spawns any, die with it. On Windows this is exactly
/// the previous `kill()` + `wait()` pair; the Job Object handles descendants.
fn terminate(child: &mut Child) {
    #[cfg(target_os = "linux")]
    {
        let pid = child.id() as i32;
        if pid > 0 {
            // Negative pid = "every process in group |pid|". The group exists
            // only because the child called setpgid(0, 0); if that failed the
            // group does not exist and kill() just returns ESRCH.
            unsafe { libc::kill(-pid, libc::SIGKILL) };
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[derive(Debug, Clone, Serialize)]
pub struct SetupStatus {
    pub server_installed: bool,
    pub server_path: String,
    pub model_installed: bool,
    pub model_path: String,
    /// Absolute path of the managed models directory ({app_data}/models).
    pub models_dir: String,
    pub models: Vec<ModelInfo>,
    /// An NVIDIA driver is installed (Windows: nvcuda.dll present), so the
    /// CUDA build of whisper-server can run here (v0.8).
    pub gpu_available: bool,
    /// Build of the installed server: "cuda" | "vulkan" | "npu" | "cpu" | ""
    /// (not installed).
    pub server_backend: String,
    /// Detected accelerators (v0.9).
    pub hw: crate::hw::Hardware,
    /// Values of `stt.local.accel` selectable on this platform.
    pub accel_options: Vec<String>,
    /// What the current settings resolve to: "cuda" | "vulkan" | "npu" | "cpu".
    pub effective_accel: String,
    /// NPU mode only: the compiled encoder (.rai) for the active model is on
    /// disk. Always true outside NPU mode.
    pub npu_cache_ready: bool,
}

/// Whether the NVIDIA driver (and therefore CUDA) is present on this machine.
pub fn gpu_available() -> bool {
    crate::hw::get().nvidia
}

/// The concrete build the current settings ask for (see `hw::resolve_accel`).
pub fn effective_accel(settings: &Settings) -> &'static str {
    crate::hw::resolve_accel(
        &settings.stt.local.accel,
        settings.stt.local.gpu,
        crate::hw::get(),
    )
}

/// Which build sits next to the server executable, from its backend
/// libraries: the Lemonade NPU build ships the VitisAI runtime (flexmlrt),
/// CUDA builds ggml-cuda, Vulkan builds ggml-vulkan; otherwise "cpu".
/// A heuristic for the app-managed install; a custom `server_path` packaged
/// differently may be reported as "cpu".
pub fn server_backend(server_exe: &Path) -> &'static str {
    let dir = server_exe.parent().unwrap_or(server_exe);
    let names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default();
    let has = |stem: &str| {
        names
            .iter()
            .any(|n| n.contains(stem) && (n.ends_with(".dll") || n.contains(".so")))
    };
    if has("flexmlrt") {
        "npu"
    } else if has("ggml-cuda") {
        "cuda"
    } else if has("ggml-vulkan") {
        "vulkan"
    } else {
        "cpu"
    }
}

// ---------------------------------------------------------------------------
// NPU compiled encoder cache (v0.9)
// ---------------------------------------------------------------------------

/// AMD publishes a compiled VitisAI encoder per whisper model on Hugging Face
/// (the files Lemonade uses). The NPU build of whisper-server looks for it
/// next to the model as `ggml-{name}-encoder-vitisai.rai`.
fn npu_cache_source(model: &str) -> Option<(&'static str, String)> {
    let repo = match model {
        "tiny" => "amd/whisper-tiny-onnx-npu",
        "base" => "amd/whisper-base-onnx-npu",
        "small" => "amd/whisper-small-onnx-npu",
        "medium" => "amd/whisper-medium-onnx-npu",
        "large-v3-turbo" => "amd/whisper-large-turbo-onnx-npu",
        _ => return None,
    };
    Some((repo, format!("ggml-{model}-encoder-vitisai.rai")))
}

/// `…/ggml-large-v3-turbo.bin` -> `…/ggml-large-v3-turbo-encoder-vitisai.rai`.
fn npu_cache_path(model_path: &Path) -> Option<PathBuf> {
    let stem = model_path.file_stem()?.to_str()?;
    Some(model_path.with_file_name(format!("{stem}-encoder-vitisai.rai")))
}

/// Managed model name (`large-v3-turbo`) of a resolved model path.
fn managed_model_name(model_path: &Path) -> Option<String> {
    let stem = model_path.file_stem()?.to_str()?;
    let name = stem.strip_prefix("ggml-")?;
    MODELS
        .iter()
        .any(|(n, _)| *n == name)
        .then(|| name.to_string())
}

/// NPU mode: whether the compiled encoder for the active model is on disk.
fn npu_cache_ready(app: &AppHandle, settings: &Settings) -> bool {
    if effective_accel(settings) != "npu" {
        return true;
    }
    resolve_model_path(app, settings)
        .and_then(|m| npu_cache_path(&m))
        .is_some_and(|p| p.exists())
}

static NPU_CACHE_DOWNLOADING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Download the compiled NPU encoder for the active managed model (about
/// 700 MB for large-v3-turbo). Progress is reported as kind `npu`. A call
/// while another is running joins it (returns Ok at once).
pub async fn download_npu_cache(app: AppHandle) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    if NPU_CACHE_DOWNLOADING.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let result = {
        let _reset = FlagReset(&NPU_CACHE_DOWNLOADING);
        download_npu_cache_inner(&app).await
    };
    if let Err(e) = &result {
        emit_progress(&app, "npu", "npu-encoder", 0, 0, true, Some(e));
    }
    result
}

async fn download_npu_cache_inner(app: &AppHandle) -> Result<(), String> {
    let settings = app.state::<crate::AppState>().settings.lock().unwrap().clone();
    let model_path =
        resolve_model_path(app, &settings).ok_or_else(|| SETUP_REQUIRED_MSG.to_string())?;
    let dest = npu_cache_path(&model_path).ok_or_else(|| "ERR_NPU_NO_CACHE".to_string())?;
    if dest.exists() {
        return Ok(());
    }
    let (repo, file_name) = managed_model_name(&model_path)
        .and_then(|m| npu_cache_source(&m))
        .ok_or_else(|| "ERR_NPU_NO_CACHE".to_string())?;
    let url = format!("https://huggingface.co/{repo}/resolve/main/{file_name}");
    let part = dest.with_extension("rai.part");
    let client = download_client()?;
    let (downloaded, total) =
        stream_download(app, &client, &url, &part, "npu", "npu-encoder").await?;
    tokio::fs::rename(&part, &dest)
        .await
        .map_err(|e| format!("ERR_FILE_IO|rename npu cache: {e}"))?;
    emit_progress(app, "npu", "npu-encoder", downloaded, total.max(downloaded), true, None);
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub name: String,
    pub installed: bool,
    pub size_mb: u64,
}

fn app_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("ERR_FILE_IO|app data dir: {e}"))
}

pub fn bin_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join("bin"))
}

pub fn models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join("models"))
}

/// Whether `file_name` is the whisper server executable for this platform:
/// Windows wants `*server*.exe`, Linux the extension-less `whisper-server`
/// (which rules out the `lib*.so*` shipped in the same tarball).
fn is_server_exe_name(file_name: &str) -> bool {
    let lower = file_name.to_lowercase();
    #[cfg(windows)]
    {
        lower.contains("server") && lower.ends_with(".exe")
    }
    #[cfg(not(windows))]
    {
        lower.contains("server") && !lower.contains('.')
    }
}

/// Recursively search `dir` for the whisper server executable.
pub fn find_server_exe(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut dirs: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if is_server_exe_name(name) {
                return Some(path);
            }
        }
    }
    for d in dirs {
        if let Some(found) = find_server_exe(&d) {
            return Some(found);
        }
    }
    None
}

pub fn resolve_server_path(app: &AppHandle, settings: &Settings) -> Option<PathBuf> {
    let overridden = settings.stt.local.server_path.trim();
    if !overridden.is_empty() {
        let p = PathBuf::from(overridden);
        return if p.exists() { Some(p) } else { None };
    }
    let dir = bin_dir(app).ok()?;
    find_server_exe(&dir)
}

pub fn resolve_model_path(app: &AppHandle, settings: &Settings) -> Option<PathBuf> {
    let overridden = settings.stt.local.model_path.trim();
    if !overridden.is_empty() {
        let p = PathBuf::from(overridden);
        return if p.exists() { Some(p) } else { None };
    }
    let dir = models_dir(app).ok()?;
    for name in MODEL_PREFERENCE {
        let p = dir.join(format!("ggml-{name}.bin"));
        if p.exists() {
            return Some(p);
        }
    }
    // any other ggml-*.bin
    let entries = std::fs::read_dir(&dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with("ggml-") && name.ends_with(".bin") {
                return Some(path);
            }
        }
    }
    None
}

pub fn get_setup_status(app: &AppHandle, settings: &Settings) -> Result<SetupStatus, String> {
    let server = resolve_server_path(app, settings);
    let model = resolve_model_path(app, settings);
    let mdir = models_dir(app)?;
    let models = MODELS
        .iter()
        .map(|(name, size_mb)| ModelInfo {
            name: name.to_string(),
            installed: mdir.join(format!("ggml-{name}.bin")).exists(),
            size_mb: *size_mb,
        })
        .collect();
    let server_backend = server
        .as_deref()
        .map(|p| server_backend(p).to_string())
        .unwrap_or_default();
    Ok(SetupStatus {
        server_installed: server.is_some(),
        server_path: server
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        gpu_available: gpu_available(),
        server_backend,
        hw: crate::hw::get().clone(),
        accel_options: crate::hw::supported_accels()
            .iter()
            .map(|s| s.to_string())
            .collect(),
        effective_accel: effective_accel(settings).to_string(),
        npu_cache_ready: npu_cache_ready(app, settings),
        model_installed: model.is_some(),
        model_path: model
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        models_dir: mdir.to_string_lossy().to_string(),
        models,
    })
}

// ---------------------------------------------------------------------------
// whisper-server process management
// ---------------------------------------------------------------------------

/// Make sure the local whisper-server is running with the current settings.
/// Returns the port to talk to.
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
    use std::sync::atomic::Ordering;

    let port = settings.stt.local.server_port;
    let threads = settings.stt.local.threads;
    let accel = effective_accel(settings);
    let server_path =
        resolve_server_path(app, settings).ok_or_else(|| SETUP_REQUIRED_MSG.to_string())?;
    let model_path =
        resolve_model_path(app, settings).ok_or_else(|| SETUP_REQUIRED_MSG.to_string())?;
    let backend = server_backend(&server_path);
    if backend == "npu" && !npu_cache_path(&model_path).is_some_and(|p| p.exists()) {
        // The NPU build cannot run the encoder without AMD's compiled cache.
        return Err("ERR_NPU_CACHE_MISSING".to_string());
    }
    // CPU wanted but a GPU build installed: run it with --no-gpu. Only those
    // builds get the flag, so a user-supplied older binary keeps working.
    let no_gpu = accel == "cpu" && matches!(backend, "cuda" | "vulkan");
    // The build is part of the signature so a server started from the old
    // build right before a download swap is restarted on the next call.
    let signature = format!(
        "{}|{}|{}|{}|backend={}|no_gpu={}",
        server_path.display(),
        model_path.display(),
        port,
        threads,
        backend,
        no_gpu
    );

    let state = app.state::<crate::AppState>();

    // The slow spawn + health-check must never run while holding the mutex.
    // Concurrent callers coordinate through the `server_starting` flag.
    loop {
        let stale = {
            let mut guard = state.server.lock().unwrap();
            match guard.as_mut() {
                Some(managed) => {
                    let alive = matches!(managed.child.try_wait(), Ok(None));
                    if alive
                        && managed.signature == signature
                        && tcp_ok(port, Duration::from_millis(600))
                    {
                        return Ok(port);
                    }
                    // stale/dead/mismatched server: take it out, kill outside
                    guard.take()
                }
                None => {
                    if state.server_starting.swap(true, Ordering::SeqCst) {
                        // another caller is starting; wait and re-check
                        None
                    } else if SERVER_SWAPPING.load(Ordering::SeqCst) {
                        // a server download is replacing bin/: spawning the
                        // old exe now would lock it (Windows) and break the
                        // swap. Back off until the new build is in place.
                        state.server_starting.store(false, Ordering::SeqCst);
                        None
                    } else {
                        // we own the start
                        break;
                    }
                }
            }
        };
        if let Some(mut managed) = stale {
            terminate(&mut managed.child);
            continue;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    // We own `server_starting`: spawn + health-check with no lock held.
    let result = spawn_and_health_check(&server_path, &model_path, port, threads, no_gpu, signature);
    let mut guard = state.server.lock().unwrap();
    state.server_starting.store(false, Ordering::SeqCst);
    match result {
        Ok(managed) => {
            *guard = Some(managed);
            Ok(port)
        }
        Err(e) => Err(e),
    }
}

fn spawn_and_health_check(
    server_path: &Path,
    model_path: &Path,
    port: u16,
    threads: u32,
    no_gpu: bool,
    signature: String,
) -> Result<ManagedServer, String> {
    // Refuse to spawn onto a port something already listens on (e.g. an
    // orphaned whisper-server from a previous crash). A stale managed server
    // may still be mid-kill on another thread (it is killed after the lock is
    // released), so on a hit wait briefly and re-probe before erroring.
    if tcp_ok(port, Duration::from_millis(300)) {
        std::thread::sleep(Duration::from_millis(400));
        if tcp_ok(port, Duration::from_millis(300)) {
            return Err(format!("ERR_PORT_IN_USE|{port}"));
        }
    }

    let mut cmd = Command::new(server_path);
    cmd.arg("-m")
        .arg(model_path)
        .arg("--port")
        .arg(port.to_string())
        .arg("--host")
        .arg("127.0.0.1")
        .arg("-t")
        .arg(threads.to_string());
    if no_gpu {
        cmd.arg("--no-gpu");
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(parent) = server_path.parent() {
        cmd.current_dir(parent);
        // The whisper.cpp Linux release ships libggml/libwhisper next to the
        // binary; make sure they are found even without an RPATH.
        #[cfg(target_os = "linux")]
        {
            let mut lib_path = parent.as_os_str().to_os_string();
            if let Some(existing) = std::env::var_os("LD_LIBRARY_PATH") {
                if !existing.is_empty() {
                    lib_path.push(":");
                    lib_path.push(existing);
                }
            }
            cmd.env("LD_LIBRARY_PATH", lib_path);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        let parent_pid = unsafe { libc::getpid() };
        // SAFETY: `pre_exec` runs in the forked child between fork() and
        // exec(), where only async-signal-safe operations are allowed. The
        // closure calls nothing but raw syscalls (setpgid, prctl, getppid,
        // raise) — no allocation, no locks, no Rust runtime re-entry — so it
        // cannot deadlock on a lock held by another thread at fork time.
        unsafe {
            cmd.pre_exec(move || {
                // Own process group, so `terminate` can signal the whole tree.
                // Failure is non-fatal: the direct child is still killable.
                libc::setpgid(0, 0);
                // Die with this app even on crash / SIGKILL of the parent.
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                // Close the race where the parent died between fork() and the
                // prctl above (the death signal would then never arrive).
                if libc::getppid() != parent_pid {
                    libc::raise(libc::SIGKILL);
                }
                Ok(())
            });
        }
    }
    let mut child = spawn_child(cmd).map_err(|e| format!("ERR_SERVER_START|{e}"))?;

    // Bind the child to a kill-on-close Job Object so it dies with this
    // process even on crash / Task Manager kill. (Linux uses PR_SET_PDEATHSIG,
    // set above in the child itself, so there is no handle to keep.)
    #[cfg(windows)]
    let job = match assign_kill_on_close_job(&child) {
        Ok(job) => job,
        Err(e) => {
            terminate(&mut child);
            return Err(e);
        }
    };

    // health check: retry TCP connect for up to 60 seconds (v0.9: the NPU build
    // loads a ~700 MB compiled encoder first; a dead child still fails fast)
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            eprintln!("whisper-server exited immediately (exit: {status})");
            return Err("ERR_SERVER_DIED".to_string());
        }
        if tcp_ok(port, Duration::from_millis(400)) {
            break;
        }
        if Instant::now() >= deadline {
            terminate(&mut child);
            return Err("ERR_SERVER_START|health check timeout (60s)".to_string());
        }
        std::thread::sleep(Duration::from_millis(500));
    }

    Ok(ManagedServer {
        child,
        signature,
        #[cfg(windows)]
        job,
    })
}

#[cfg(windows)]
fn assign_kill_on_close_job(child: &Child) -> Result<win32job::Job, String> {
    use std::os::windows::io::AsRawHandle;
    let mut info = win32job::ExtendedLimitInfo::new();
    info.limit_kill_on_job_close();
    let job = win32job::Job::create_with_limit_info(&info)
        .map_err(|e| format!("ERR_SERVER_START|job object: {e}"))?;
    job.assign_process(child.as_raw_handle() as isize)
        .map_err(|e| format!("ERR_SERVER_START|job assign: {e}"))?;
    Ok(job)
}

fn tcp_ok(port: u16, timeout: Duration) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    std::net::TcpStream::connect_timeout(&addr, timeout).is_ok()
}

/// Kill the managed whisper-server, if any.
/// The server is taken out under the lock but killed after releasing it.
pub fn kill_server(app: &AppHandle) {
    if let Some(state) = app.try_state::<crate::AppState>() {
        let managed = match state.server.lock() {
            Ok(mut guard) => guard.take(),
            Err(_) => None,
        };
        if let Some(mut managed) = managed {
            terminate(&mut managed.child);
            // on Windows, dropping `managed.job` closes the job handle, which
            // also kills the process tree thanks to kill-on-close
        }
    }
}

// ---------------------------------------------------------------------------
// Downloads
// ---------------------------------------------------------------------------

fn emit_progress(
    app: &AppHandle,
    kind: &str,
    name: &str,
    downloaded: u64,
    total: u64,
    done: bool,
    error: Option<&str>,
) {
    let mut payload = serde_json::json!({
        "kind": kind,
        "name": name,
        "downloaded": downloaded,
        "total": total,
        "done": done,
    });
    if let Some(e) = error {
        payload["error"] = serde_json::json!(e);
    }
    let _ = app.emit("download-progress", payload);
}

fn download_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("fire-speak")
        .build()
        .map_err(|e| format!("ERR_INTERNAL|http client: {e}"))
}

async fn stream_download(
    app: &AppHandle,
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    kind: &str,
    name: &str,
) -> Result<(u64, u64), String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("ERR_DOWNLOAD_FAILED|{e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "ERR_DOWNLOAD_FAILED|HTTP {}",
            resp.status().as_u16()
        ));
    }
    let total = resp.content_length().unwrap_or(0);
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("ERR_FILE_IO|create dir: {e}"))?;
    }
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("ERR_FILE_IO|create file: {e}"))?;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_emit = Instant::now();
    emit_progress(app, kind, name, 0, total, false, None);
    // Any mid-stream failure must not leave a partial file on disk.
    let write_result: Result<(), String> = async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("ERR_DOWNLOAD_FAILED|{e}"))?;
            file.write_all(&chunk)
                .await
                .map_err(|e| format!("ERR_FILE_IO|write: {e}"))?;
            downloaded += chunk.len() as u64;
            if last_emit.elapsed() >= Duration::from_millis(200) {
                last_emit = Instant::now();
                emit_progress(app, kind, name, downloaded, total, false, None);
            }
        }
        file.flush()
            .await
            .map_err(|e| format!("ERR_FILE_IO|flush: {e}"))
    }
    .await;
    drop(file);
    if let Err(e) = write_result {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(e);
    }

    // If the server told us the size, a short read means a truncated download.
    if total > 0 && downloaded != total {
        let _ = tokio::fs::remove_file(dest).await;
        return Err("ERR_DOWNLOAD_INCOMPLETE".to_string());
    }
    Ok((downloaded, total))
}

/// File name of the temporary archive downloaded into `{app_data}/bin`.
#[cfg(windows)]
const SERVER_ARCHIVE_TMP: &str = "_whisper-server-download.zip";
#[cfg(not(windows))]
const SERVER_ARCHIVE_TMP: &str = "_whisper-server-download.tar.gz";

/// Pick the whisper.cpp release asset for this platform + architecture.
///
/// Windows x64, `want_cuda`: the CUDA build (`whisper-cublas-12.x-bin-x64.zip`
/// or the newer `whisper-bin-win-cuda-12.x-x64.zip`; 12.x preferred over
/// 11.x). It bundles the CUDA runtime DLLs (release.yml "Copy CUDA DLLs"), so
/// no toolkit install is needed — only the NVIDIA driver.
/// Windows x64 otherwise: `whisper-bin-x64.zip` (fallback `win…x64….zip`).
/// Linux: `whisper-bin-ubuntu-x64.tar.gz` / `whisper-bin-ubuntu-arm64.tar.gz`.
/// Returns None when this release carries no suitable asset (the caller then
/// tries the next release: `releases/latest` is sometimes tagged before its
/// binaries are uploaded).
fn pick_server_asset<'a>(
    assets: &'a [serde_json::Value],
    want_cuda: bool,
) -> Option<&'a serde_json::Value> {
    let name = |a: &serde_json::Value| a["name"].as_str().unwrap_or("").to_lowercase();

    #[cfg(windows)]
    {
        if want_cuda {
            let is_cuda_x64 = |n: &str| {
                (n.contains("cublas") || n.contains("cuda"))
                    && n.contains("x64")
                    && !n.contains("arm64")
                    && n.ends_with(".zip")
            };
            return assets
                .iter()
                .find(|a| {
                    let n = name(a);
                    is_cuda_x64(&n) && n.contains("12.")
                })
                .or_else(|| assets.iter().find(|a| is_cuda_x64(&name(a))));
        }
        assets
            .iter()
            .find(|a| {
                let n = name(a);
                n.contains("bin-x64") && n.ends_with(".zip")
            })
            .or_else(|| {
                assets.iter().find(|a| {
                    let n = name(a);
                    n.contains("win")
                        && n.contains("x64")
                        && !n.contains("arm64")
                        && !n.contains("cuda")
                        && !n.contains("cublas")
                        && n.ends_with(".zip")
                })
            })
    }
    #[cfg(target_os = "linux")]
    {
        let _ = want_cuda;
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x64"
        };
        let exact = format!("whisper-bin-ubuntu-{arch}.tar.gz");
        assets.iter().find(|a| name(a) == exact).or_else(|| {
            // tolerate a renamed/retagged asset as long as it is clearly the
            // Linux build for this architecture
            assets.iter().find(|a| {
                let n = name(a);
                n.contains("ubuntu") && n.contains(arch) && n.ends_with(".tar.gz")
            })
        })
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (name, assets, want_cuda);
        None
    }
}

/// Extract the downloaded server archive into `dest`
/// (.zip on Windows, .tar.gz on Linux).
fn extract_server_archive(archive: &Path, dest: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        extract_zip(archive, dest)
    }
    #[cfg(target_os = "linux")]
    {
        extract_tar_gz(archive, dest)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (archive, dest);
        Err("ERR_NO_SERVER_ASSET".to_string())
    }
}

/// Download the latest whisper.cpp server build for this platform and extract it.
pub async fn download_whisper_server(app: AppHandle) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    // One download at a time (v0.8.1): the startup GPU upgrade may already be
    // running when the user presses the install button. The second caller
    // joins it — progress and the final done/error event come from the
    // running download, which the frontend listens to either way.
    if SERVER_DOWNLOADING.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let result = {
        let _reset = FlagReset(&SERVER_DOWNLOADING);
        download_whisper_server_inner(&app).await
    };
    match &result {
        Ok(()) => prewarm(&app),
        Err(e) => emit_progress(&app, "server", "whisper-server", 0, 0, true, Some(e)),
    }
    result
}

static SERVER_DOWNLOADING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Set while `download_whisper_server_inner` kills the running server and
/// replaces `bin/`. `ensure_server_blocking` does not start a server while it
/// is set, and the swap waits for an in-flight start to finish first
/// (both sides store their own flag, then read the other's — SeqCst).
static SERVER_SWAPPING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Clears a flag on drop, so an early return or panic cannot leave it set.
struct FlagReset(&'static std::sync::atomic::AtomicBool);

impl Drop for FlagReset {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Start the local whisper-server in the background so the first dictation
/// does not wait for the model to load (v0.8.1). No-op for the cloud engine
/// or when the server/model is not installed. Errors are only logged: the
/// next transcription retries through `ensure_server` and reports them.
pub fn prewarm(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let (local, installed) = {
            let state = app.state::<crate::AppState>();
            let settings = state.settings.lock().unwrap().clone();
            (
                settings.stt.engine == "local",
                resolve_server_path(&app, &settings).is_some()
                    && resolve_model_path(&app, &settings).is_some(),
            )
        };
        if !local || !installed {
            return;
        }
        if let Err(e) = ensure_server(app.clone()).await {
            eprintln!("whisper-server prewarm failed: {e}");
            return;
        }
        // GPU/NPU builds compile their kernels on the first inference (about
        // 3.5 s with Vulkan); pay that now with a second of silence instead
        // of on the user's first dictation (v0.9).
        let settings = app.state::<crate::AppState>().settings.lock().unwrap().clone();
        if effective_accel(&settings) != "cpu" {
            if let Ok(wav) = crate::audio::wav_bytes(&[0i16; 16000]) {
                let _ = crate::stt::transcribe_local(&app, &settings, wav).await;
            }
        }
    });
}

/// Bring the local engine in line with the settings (v0.9; v0.8.1 did only
/// the CPU -> CUDA case): when the app-managed server build cannot serve the
/// wanted accelerator (e.g. CPU build on an AMD Radeon, or NPU selected),
/// download the right one in the background — staged, so the old server
/// keeps serving until the swap. In NPU mode also fetch the compiled encoder
/// for the active model. Then prewarm. Nothing is downloaded when the server
/// or model was never installed (first setup stays an explicit user action)
/// or a custom `server_path` is set.
pub fn sync_server_build(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // The first hardware probe takes about a second (WMI).
        let _ = tokio::task::spawn_blocking(crate::hw::get).await;
        let settings = app.state::<crate::AppState>().settings.lock().unwrap().clone();
        if settings.stt.engine != "local" {
            return;
        }
        let wanted = effective_accel(&settings);
        let managed = settings.stt.local.server_path.trim().is_empty();
        let installed = resolve_server_path(&app, &settings).map(|exe| server_backend(&exe));
        if let (true, Some(installed)) = (managed, installed) {
            if !crate::hw::build_satisfies(installed, wanted) {
                // prewarms on success; with NPU the encoder is still missing
                // and ensure_server says so — fetched right below
                if let Err(e) = download_whisper_server(app.clone()).await {
                    eprintln!("automatic {wanted} server install failed: {e}");
                    return;
                }
                // Ok also means "joined a download already running" (maybe
                // for an older choice): wait for it, then re-check.
                while SERVER_DOWNLOADING.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                let now = app.state::<crate::AppState>().settings.lock().unwrap().clone();
                let ok = resolve_server_path(&app, &now)
                    .is_some_and(|exe| crate::hw::build_satisfies(server_backend(&exe), effective_accel(&now)));
                if !ok {
                    // superseded or failed; a later sync (settings change,
                    // next launch) retries
                    return;
                }
            }
        }
        if wanted == "npu" && !npu_cache_ready(&app, &settings) {
            if let Err(e) = download_npu_cache(app.clone()).await {
                eprintln!("NPU encoder download failed: {e}");
                return;
            }
        }
        prewarm(&app);
    });
}

/// Startup work for the local engine: match the server build to the
/// hardware and prewarm it.
pub fn on_startup(app: &AppHandle) {
    sync_server_build(app);
}

/// Vulkan and NPU builds come from Lemonade's whisper.cpp fork (AMD's local
/// AI server), pinned to one release and verified against the SHA-256 digests
/// GitHub publishes for these assets (v0.9).
const LEMONADE_WHISPER_REPO: &str = "lemonade-sdk/whisper.cpp-rocm";
const LEMONADE_WHISPER_TAG: &str = "v1.8.4";

fn lemonade_asset(accel: &str) -> Option<(&'static str, &'static str)> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        match accel {
            "vulkan" => Some((
                "whisper-v1.8.4-windows-vulkan-x64.zip",
                "e0d20a0f92e31b98adc0faf71172efc810b701e6391a9d858ca045bff26f77cd",
            )),
            "npu" => Some((
                "whisper-v1.8.4-windows-npu-x64.zip",
                "7e191e6dc9a2407b4cb55df49f2582afba51d417b3409edf22552047d4f9b177",
            )),
            _ => None,
        }
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        match accel {
            "vulkan" => Some((
                "whisper-v1.8.4-linux-vulkan-x86_64.tar.gz",
                "55d0bdf9be7092ed8148b27d26353f5755446381eb9e1e5b6b00d5c761d813b1",
            )),
            _ => None,
        }
    }
    #[cfg(not(any(
        all(windows, target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64")
    )))]
    {
        let _ = accel;
        None
    }
}

fn sha256_file(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path).map_err(|e| format!("ERR_FILE_IO|open: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = std::io::Read::read(&mut file, &mut buf)
            .map_err(|e| format!("ERR_FILE_IO|read: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

async fn download_whisper_server_inner(app: &AppHandle) -> Result<(), String> {
    let client = download_client()?;
    let accel = {
        let settings = app.state::<crate::AppState>().settings.lock().unwrap().clone();
        effective_accel(&settings)
    };
    let (name, url, sha256) = match lemonade_asset(accel) {
        Some((asset, digest)) => (
            asset.to_string(),
            format!(
                "https://github.com/{LEMONADE_WHISPER_REPO}/releases/download/{LEMONADE_WHISPER_TAG}/{asset}"
            ),
            Some(digest),
        ),
        None => {
            let (name, url) = pick_ggml_release(&client, accel == "cuda").await?;
            (name, url, None)
        }
    };
    install_server_archive(app, &client, &name, &url, sha256).await
}

/// Latest official whisper.cpp build (CPU, or CUDA when `want_cuda`).
async fn pick_ggml_release(
    client: &reqwest::Client,
    want_cuda: bool,
) -> Result<(String, String), String> {
    // Newest releases first. `releases/latest` alone is not enough: whisper.cpp
    // sometimes tags a release before CI has uploaded its binaries (v1.9.4 had
    // no assets at all), and not every release carries the CUDA build.
    let releases: Vec<serde_json::Value> = client
        .get("https://api.github.com/repos/ggml-org/whisper.cpp/releases?per_page=10")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("ERR_DOWNLOAD_FAILED|{e}"))?
        .error_for_status()
        .map_err(|e| format!("ERR_DOWNLOAD_FAILED|GitHub API: {e}"))?
        .json()
        .await
        .map_err(|e| format!("ERR_DOWNLOAD_FAILED|parse releases: {e}"))?;

    let empty = Vec::new();
    let picked = releases
        .iter()
        .filter(|r| !r["draft"].as_bool().unwrap_or(false))
        .find_map(|r| pick_server_asset(r["assets"].as_array().unwrap_or(&empty), want_cuda))
        .or_else(|| {
            // no CUDA asset anywhere: fall back to the CPU build rather than fail
            if want_cuda {
                releases.iter().find_map(|r| {
                    pick_server_asset(r["assets"].as_array().unwrap_or(&empty), false)
                })
            } else {
                None
            }
        })
        .ok_or_else(|| "ERR_NO_SERVER_ASSET".to_string())?;

    let name = picked["name"].as_str().unwrap_or("").to_string();
    let url = picked["browser_download_url"]
        .as_str()
        .ok_or_else(|| "ERR_NO_SERVER_ASSET".to_string())?
        .to_string();
    Ok((name, url))
}

/// Download `url` into a staging directory, verify (`sha256`) and extract
/// it, then swap it in for the installed server.
async fn install_server_archive(
    app: &AppHandle,
    client: &reqwest::Client,
    name: &str,
    url: &str,
    sha256: Option<&str>,
) -> Result<(), String> {
    let bin = bin_dir(app)?;
    // Stage the new build in a sibling directory and only replace the
    // existing install once the download extracted and contains a server
    // executable. A failed download therefore leaves the old server intact,
    // and the swap guarantees CPU and CUDA files never mix (both archives
    // extract to `Release/`, so stale ggml-*.dll files could otherwise shadow
    // the new backend).
    let staging = bin.with_file_name("bin.new");
    if staging.exists() {
        let _ = tokio::fs::remove_dir_all(&staging).await;
    }
    tokio::fs::create_dir_all(&staging)
        .await
        .map_err(|e| format!("ERR_FILE_IO|create dir: {e}"))?;
    let archive_path = staging.join(SERVER_ARCHIVE_TMP);

    let staged = async {
        let (downloaded, total) =
            stream_download(app, client, url, &archive_path, "server", name).await?;
        if let Some(expected) = sha256 {
            let archive = archive_path.clone();
            let actual = tokio::task::spawn_blocking(move || sha256_file(&archive))
                .await
                .map_err(|e| format!("ERR_INTERNAL|hash: {e}"))??;
            if actual != expected {
                let _ = tokio::fs::remove_file(&archive_path).await;
                return Err(format!("ERR_DOWNLOAD_FAILED|checksum mismatch for {name}"));
            }
        }

        // extract in a blocking thread; the temp archive is removed on every outcome
        let staging2 = staging.clone();
        let archive2 = archive_path.clone();
        let extract_result =
            tokio::task::spawn_blocking(move || extract_server_archive(&archive2, &staging2))
                .await
                .map_err(|e| format!("ERR_INTERNAL|extract: {e}"))
                .and_then(|r| r);
        let _ = tokio::fs::remove_file(&archive_path).await;
        extract_result?;

        if find_server_exe(&staging).is_none() {
            return Err("ERR_NO_SERVER_ASSET".to_string());
        }
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

    // Swap: stop the running server (blocking kill/wait, off the async
    // worker), drop the old install, move the verified one into place.
    // Block new server starts for the whole swap, and wait out one that is
    // already spawning (it would otherwise hold the old exe open).
    SERVER_SWAPPING.store(true, std::sync::atomic::Ordering::SeqCst);
    let _swapping = FlagReset(&SERVER_SWAPPING);
    let app_kill = app.clone();
    tokio::task::spawn_blocking(move || {
        let state = app_kill.state::<crate::AppState>();
        while state.server_starting.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(100));
        }
        crate::setup::kill_server(&app_kill)
    })
    .await
    .map_err(|e| format!("ERR_INTERNAL|kill server: {e}"))?;
    if bin.exists() {
        tokio::fs::remove_dir_all(&bin)
            .await
            .map_err(|e| format!("ERR_FILE_IO|remove old server: {e}"))?;
    }
    tokio::fs::rename(&staging, &bin)
        .await
        .map_err(|e| format!("ERR_FILE_IO|install server: {e}"))?;
    drop(_swapping);

    let Some(server_exe) = find_server_exe(&bin) else {
        return Err("ERR_NO_SERVER_ASSET".to_string());
    };
    // Tar preserves the mode bits, but a release built with a restrictive
    // umask (or a future switch to an archive format that drops them) would
    // leave the server unexecutable. Force it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&server_exe, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("ERR_FILE_IO|chmod server: {e}"))?;
    }
    #[cfg(not(unix))]
    let _ = server_exe;

    emit_progress(
        app,
        "server",
        name,
        downloaded,
        total.max(downloaded),
        true,
        None,
    );
    Ok(())
}

#[cfg(windows)]
fn extract_zip(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let file =
        std::fs::File::open(zip_path).map_err(|e| format!("ERR_ZIP|open: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("ERR_ZIP|read: {e}"))?;
    archive
        .extract(dest)
        .map_err(|e| format!("ERR_ZIP|extract: {e}"))
}

/// Normalize a tar entry path to a relative path guaranteed to stay inside the
/// destination directory: absolute paths, `..` and Windows drive prefixes are
/// rejected outright rather than silently stripped.
#[cfg(target_os = "linux")]
fn safe_relative_path(path: &Path) -> Result<PathBuf, String> {
    use std::path::Component;
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir => {}
            _ => return Err(format!("ERR_ZIP|unsafe entry path: {}", path.display())),
        }
    }
    Ok(safe)
}

/// Extract a gzip-compressed tar into `dest`.
///
/// Every entry path is validated against directory traversal (including via a
/// symlink target) before anything is written. Regular files, directories and
/// relative symlinks are extracted; anything else (device nodes, hard links,
/// FIFOs) is skipped.
#[cfg(target_os = "linux")]
fn extract_tar_gz(archive_path: &Path, dest: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let file =
        std::fs::File::open(archive_path).map_err(|e| format!("ERR_ZIP|open: {e}"))?;
    let decoder = flate2::read::GzDecoder::new(std::io::BufReader::new(file));
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|e| format!("ERR_ZIP|read: {e}"))?;

    for entry in entries {
        let mut entry = entry.map_err(|e| format!("ERR_ZIP|entry: {e}"))?;
        let raw_path = entry
            .path()
            .map_err(|e| format!("ERR_ZIP|path: {e}"))?
            .into_owned();
        let relative = safe_relative_path(&raw_path)?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let out = dest.join(&relative);
        let entry_type = entry.header().entry_type();

        if entry_type.is_dir() {
            std::fs::create_dir_all(&out)
                .map_err(|e| format!("ERR_ZIP|create dir: {e}"))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("ERR_ZIP|create dir: {e}"))?;
        }

        if entry_type.is_symlink() {
            // The whisper.cpp tarball ships versioned shared objects behind
            // symlinks (libwhisper.so -> libwhisper.so.1); keep them, but only
            // when the target cannot escape `dest`.
            let Some(target) = entry
                .link_name()
                .map_err(|e| format!("ERR_ZIP|link: {e}"))?
                .map(|t| t.into_owned())
            else {
                continue;
            };
            safe_relative_path(&target)?;
            let _ = std::fs::remove_file(&out);
            std::os::unix::fs::symlink(&target, &out)
                .map_err(|e| format!("ERR_ZIP|symlink: {e}"))?;
            continue;
        }
        if !entry_type.is_file() {
            continue; // device nodes, hard links, FIFOs: not shipped, not wanted
        }

        let mode = entry.header().mode().unwrap_or(0o644) & 0o777;
        let mut out_file =
            std::fs::File::create(&out).map_err(|e| format!("ERR_ZIP|create: {e}"))?;
        std::io::copy(&mut entry, &mut out_file)
            .map_err(|e| format!("ERR_ZIP|extract: {e}"))?;
        drop(out_file);
        std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode))
            .map_err(|e| format!("ERR_ZIP|chmod: {e}"))?;
    }
    Ok(())
}

/// Download a whisper.cpp GGML model from Hugging Face.
pub async fn download_model(app: AppHandle, model: String) -> Result<(), String> {
    if !MODELS.iter().any(|(n, _)| *n == model) {
        return Err(format!("ERR_INTERNAL|unknown model: {model}"));
    }
    let result = download_model_inner(&app, &model).await;
    if let Err(e) = &result {
        emit_progress(&app, "model", &model, 0, 0, true, Some(e));
    }
    result
}

async fn download_model_inner(app: &AppHandle, model: &str) -> Result<(), String> {
    let client = download_client()?;
    let file_name = format!("ggml-{model}.bin");
    let url =
        format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{file_name}");
    let dir = models_dir(app)?;
    let dest = dir.join(&file_name);
    let part = dir.join(format!("{file_name}.part"));

    let (downloaded, total) =
        stream_download(app, &client, &url, &part, "model", model).await?;

    // Sanity check: final size must be within ±20% of the expected size.
    let expected_bytes = MODELS
        .iter()
        .find(|(n, _)| *n == model)
        .map(|(_, size_mb)| size_mb * 1024 * 1024)
        .unwrap_or(0);
    if expected_bytes > 0 {
        let lo = expected_bytes * 8 / 10;
        let hi = expected_bytes * 12 / 10;
        if downloaded < lo || downloaded > hi {
            let _ = tokio::fs::remove_file(&part).await;
            return Err("ERR_DOWNLOAD_INCOMPLETE".to_string());
        }
    }

    tokio::fs::rename(&part, &dest)
        .await
        .map_err(|e| format!("ERR_FILE_IO|rename model: {e}"))?;

    emit_progress(
        app,
        "model",
        model,
        downloaded,
        total.max(downloaded),
        true,
        None,
    );
    Ok(())
}

#[cfg(all(test, windows))]
mod asset_tests {
    use super::pick_server_asset;

    /// Asset names of whisper.cpp release b5130 (verified via the GitHub API).
    fn b5130() -> Vec<serde_json::Value> {
        [
            "whisper-b5130-xcframework.zip",
            "whisper-bin-ubuntu-arm64.tar.gz",
            "whisper-bin-ubuntu-x64.tar.gz",
            "whisper-bin-win-cpu-arm64.zip",
            "whisper-bin-win-cuda-13.4-arm64.zip",
            "whisper-bin-win-opencl-adreno-arm64.zip",
            "whisper-bin-Win32.zip",
            "whisper-bin-x64.zip",
            "whisper-blas-bin-Win32.zip",
            "whisper-blas-bin-x64.zip",
            "whisper-cublas-11.8.0-bin-x64.zip",
            "whisper-cublas-12.4.0-bin-x64.zip",
        ]
        .iter()
        .map(|n| serde_json::json!({ "name": n }))
        .collect()
    }

    fn name(a: Option<&serde_json::Value>) -> &str {
        a.and_then(|a| a["name"].as_str()).unwrap_or("")
    }

    #[test]
    fn cuda_prefers_the_12x_x64_build() {
        assert_eq!(name(pick_server_asset(&b5130(), true)), "whisper-cublas-12.4.0-bin-x64.zip");
    }

    #[test]
    fn cpu_picks_the_plain_x64_build() {
        assert_eq!(name(pick_server_asset(&b5130(), false)), "whisper-bin-x64.zip");
    }

    #[test]
    fn cuda_accepts_the_newer_naming_and_skips_arm64() {
        let assets: Vec<serde_json::Value> = ["whisper-bin-win-cuda-13.4-arm64.zip", "whisper-bin-win-cuda-12.4-x64.zip"]
            .iter()
            .map(|n| serde_json::json!({ "name": n }))
            .collect();
        assert_eq!(name(pick_server_asset(&assets, true)), "whisper-bin-win-cuda-12.4-x64.zip");
    }

    #[test]
    fn release_without_assets_yields_none() {
        assert!(pick_server_asset(&[], true).is_none());
        assert!(pick_server_asset(&[], false).is_none());
    }
}

