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

const SETUP_REQUIRED_MSG: &str =
    "セットアップ画面からWhisperサーバとモデルをインストールしてください";

pub struct ManagedServer {
    pub child: Child,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetupStatus {
    pub server_installed: bool,
    pub server_path: String,
    pub model_installed: bool,
    pub model_path: String,
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
        .map_err(|e| format!("アプリデータフォルダを取得できませんでした: {e}"))
}

pub fn bin_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join("bin"))
}

pub fn models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join("models"))
}

/// Recursively search `dir` for an .exe whose file name contains "server".
pub fn find_server_exe(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut dirs: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            let lower = name.to_lowercase();
            if lower.contains("server") && lower.ends_with(".exe") {
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
        .map_err(|e| format!("内部エラー(サーバ起動): {e}"))?
}

fn ensure_server_blocking(app: &AppHandle, settings: &Settings) -> Result<u16, String> {
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
    let mut guard = state.server.lock().unwrap();

    if let Some(managed) = guard.as_mut() {
        let alive = matches!(managed.child.try_wait(), Ok(None));
        if alive && managed.signature == signature && tcp_ok(port, Duration::from_millis(600)) {
            return Ok(port);
        }
        let _ = managed.child.kill();
        let _ = managed.child.wait();
        *guard = None;
    }

    let mut cmd = Command::new(&server_path);
    cmd.arg("-m")
        .arg(&model_path)
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
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Whisperサーバの起動に失敗しました: {e}"))?;

    // health check: retry TCP connect for up to 20 seconds
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "Whisperサーバが起動直後に終了しました (exit: {status})。モデルファイルやポート設定を確認してください"
            ));
        }
        if tcp_ok(port, Duration::from_millis(400)) {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Whisperサーバの起動がタイムアウトしました(20秒)".to_string());
        }
        std::thread::sleep(Duration::from_millis(500));
    }

    *guard = Some(ManagedServer { child, signature });
    Ok(port)
}

fn tcp_ok(port: u16, timeout: Duration) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    std::net::TcpStream::connect_timeout(&addr, timeout).is_ok()
}

/// Kill the managed whisper-server, if any.
pub fn kill_server(app: &AppHandle) {
    if let Some(state) = app.try_state::<crate::AppState>() {
        if let Ok(mut guard) = state.server.lock() {
            if let Some(mut managed) = guard.take() {
                let _ = managed.child.kill();
                let _ = managed.child.wait();
            }
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
        .map_err(|e| format!("HTTPクライアントの初期化に失敗しました: {e}"))
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
        .map_err(|e| format!("ダウンロードに失敗しました: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "ダウンロードに失敗しました (HTTP {})",
            resp.status().as_u16()
        ));
    }
    let total = resp.content_length().unwrap_or(0);
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("フォルダを作成できませんでした: {e}"))?;
    }
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("ファイルを作成できませんでした: {e}"))?;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_emit = Instant::now();
    emit_progress(app, kind, name, 0, total, false, None);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("ダウンロード中にエラーが発生しました: {e}"))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("ファイルの書き込みに失敗しました: {e}"))?;
        downloaded += chunk.len() as u64;
        if last_emit.elapsed() >= Duration::from_millis(200) {
            last_emit = Instant::now();
            emit_progress(app, kind, name, downloaded, total, false, None);
        }
    }
    file.flush()
        .await
        .map_err(|e| format!("ファイルの書き込みに失敗しました: {e}"))?;
    Ok((downloaded, total))
}

/// Download the latest whisper.cpp Windows x64 server zip and extract it.
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
        .map_err(|e| format!("GitHubへの接続に失敗しました: {e}"))?
        .error_for_status()
        .map_err(|e| format!("GitHub APIエラー: {e}"))?
        .json()
        .await
        .map_err(|e| format!("GitHubリリース情報の解析に失敗しました: {e}"))?;

    let empty = Vec::new();
    let assets = release["assets"].as_array().unwrap_or(&empty);
    let asset_name = |a: &serde_json::Value| a["name"].as_str().unwrap_or("").to_string();
    let picked = assets
        .iter()
        .find(|a| {
            let n = asset_name(a).to_lowercase();
            n.contains("bin-x64") && n.ends_with(".zip")
        })
        .or_else(|| {
            assets.iter().find(|a| {
                let n = asset_name(a).to_lowercase();
                n.contains("win") && n.contains("x64") && n.ends_with(".zip")
            })
        })
        .ok_or_else(|| "Windows(x64)用のwhisper-serverアセットが見つかりませんでした".to_string())?;

    let name = asset_name(picked);
    let url = picked["browser_download_url"]
        .as_str()
        .ok_or_else(|| "ダウンロードURLを取得できませんでした".to_string())?
        .to_string();

    let bin = bin_dir(app)?;
    tokio::fs::create_dir_all(&bin)
        .await
        .map_err(|e| format!("フォルダを作成できませんでした: {e}"))?;
    let zip_path = bin.join("_whisper-server-download.zip");

    let (downloaded, total) =
        stream_download(app, &client, &url, &zip_path, "server", &name).await?;

    // extract in a blocking thread
    let bin2 = bin.clone();
    let zip2 = zip_path.clone();
    tokio::task::spawn_blocking(move || extract_zip(&zip2, &bin2))
        .await
        .map_err(|e| format!("内部エラー(展開): {e}"))??;
    let _ = tokio::fs::remove_file(&zip_path).await;

    if find_server_exe(&bin).is_none() {
        return Err("展開後にwhisper-server(.exe)が見つかりませんでした".to_string());
    }

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

fn extract_zip(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let file =
        std::fs::File::open(zip_path).map_err(|e| format!("ZIPを開けませんでした: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("ZIPの読み込みに失敗しました: {e}"))?;
    archive
        .extract(dest)
        .map_err(|e| format!("ZIPの展開に失敗しました: {e}"))
}

/// Download a whisper.cpp GGML model from Hugging Face.
pub async fn download_model(app: AppHandle, model: String) -> Result<(), String> {
    if !MODELS.iter().any(|(n, _)| *n == model) {
        return Err(format!("不明なモデル名です: {model}"));
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

    tokio::fs::rename(&part, &dest)
        .await
        .map_err(|e| format!("モデルファイルの保存に失敗しました: {e}"))?;

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
