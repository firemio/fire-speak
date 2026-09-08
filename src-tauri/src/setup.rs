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
    Ok(SetupStatus {
        server_installed: server.is_some(),
        server_path: server
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
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
    let server_path =
        resolve_server_path(app, settings).ok_or_else(|| SETUP_REQUIRED_MSG.to_string())?;
    let model_path =
        resolve_model_path(app, settings).ok_or_else(|| SETUP_REQUIRED_MSG.to_string())?;
    let signature = format!(
        "{}|{}|{}|{}",
        server_path.display(),
        model_path.display(),
        port,
        threads
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
    let result = spawn_and_health_check(&server_path, &model_path, port, threads, signature);
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
        .arg(threads.to_string())
        .stdin(Stdio::null())
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

    // health check: retry TCP connect for up to 20 seconds
    let deadline = Instant::now() + Duration::from_secs(20);
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
            return Err("ERR_SERVER_START|health check timeout (20s)".to_string());
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
/// Windows x64: `whisper-bin-x64.zip` (fallback `(?i)win.*x64.*\.zip`).
/// Linux: `whisper-bin-ubuntu-x64.tar.gz` / `whisper-bin-ubuntu-arm64.tar.gz`
/// (asset names verified against the ggml-org/whisper.cpp releases API).
fn pick_server_asset(assets: &[serde_json::Value]) -> Option<&serde_json::Value> {
    let name = |a: &serde_json::Value| a["name"].as_str().unwrap_or("").to_lowercase();

    #[cfg(windows)]
    {
        assets
            .iter()
            .find(|a| {
                let n = name(a);
                n.contains("bin-x64") && n.ends_with(".zip")
            })
            .or_else(|| {
                assets.iter().find(|a| {
                    let n = name(a);
                    n.contains("win") && n.contains("x64") && n.ends_with(".zip")
                })
            })
    }
    #[cfg(target_os = "linux")]
    {
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
        let _ = name;
        let _ = assets;
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
    let result = download_whisper_server_inner(&app).await;
    if let Err(e) = &result {
        emit_progress(&app, "server", "whisper-server", 0, 0, true, Some(e));
    }
    result
}

async fn download_whisper_server_inner(app: &AppHandle) -> Result<(), String> {
    let client = download_client()?;
    let release: serde_json::Value = client
        .get("https://api.github.com/repos/ggml-org/whisper.cpp/releases/latest")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("ERR_DOWNLOAD_FAILED|{e}"))?
        .error_for_status()
        .map_err(|e| format!("ERR_DOWNLOAD_FAILED|GitHub API: {e}"))?
        .json()
        .await
        .map_err(|e| format!("ERR_DOWNLOAD_FAILED|parse release: {e}"))?;

    let empty = Vec::new();
    let assets = release["assets"].as_array().unwrap_or(&empty);
    let picked = pick_server_asset(assets).ok_or_else(|| "ERR_NO_SERVER_ASSET".to_string())?;

    let name = picked["name"].as_str().unwrap_or("").to_string();
    let url = picked["browser_download_url"]
        .as_str()
        .ok_or_else(|| "ERR_NO_SERVER_ASSET".to_string())?
        .to_string();

    let bin = bin_dir(app)?;
    tokio::fs::create_dir_all(&bin)
        .await
        .map_err(|e| format!("ERR_FILE_IO|create dir: {e}"))?;
    let archive_path = bin.join(SERVER_ARCHIVE_TMP);

    let (downloaded, total) =
        stream_download(app, &client, &url, &archive_path, "server", &name).await?;

    // extract in a blocking thread; the temp archive is removed on every outcome
    let bin2 = bin.clone();
    let archive2 = archive_path.clone();
    let extract_result =
        tokio::task::spawn_blocking(move || extract_server_archive(&archive2, &bin2))
            .await
            .map_err(|e| format!("ERR_INTERNAL|extract: {e}"))
            .and_then(|r| r);
    let _ = tokio::fs::remove_file(&archive_path).await;
    extract_result?;

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
        &name,
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
