//! Update check against GitHub Releases (SPEC v0.2).

use serde::Serialize;
use std::time::Duration;

const NOTES_MAX_CHARS: usize = 4000;

/// Release-asset name suffixes that hold this platform's installer, in
/// preference order. The release page (`html_url`) is the fallback when none
/// of them is present.
#[cfg(windows)]
const INSTALLER_SUFFIXES: &[&str] = &["-setup.exe"];
#[cfg(target_os = "linux")]
const INSTALLER_SUFFIXES: &[&str] = &[".appimage", ".deb", ".rpm"];
#[cfg(not(any(windows, target_os = "linux")))]
const INSTALLER_SUFFIXES: &[&str] = &[];

/// Substrings that identify an asset built for the running architecture.
/// A release carries installers for several architectures, so matching only on
/// the file extension would hand an arm64 machine the x86_64 build.
#[cfg(target_arch = "x86_64")]
const ARCH_TOKENS: &[&str] = &["x86_64", "amd64", "x64"];
#[cfg(target_arch = "aarch64")]
const ARCH_TOKENS: &[&str] = &["aarch64", "arm64"];
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
const ARCH_TOKENS: &[&str] = &[];

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    /// e.g. "0.2.0" (the running app's version)
    pub current: String,
    /// e.g. "0.3.1" (release tag with a leading "v" stripped)
    pub latest: String,
    /// numeric semver comparison: latest > current
    pub update_available: bool,
    /// browser_download_url of this platform's installer asset, else the
    /// release html_url
    pub url: String,
    /// release notes body, truncated to 4000 chars
    pub notes: String,
}

/// Query `https://api.github.com/repos/{owner}/{repo}/releases/latest` and
/// compare against `current`. All failures map to `ERR_UPDATE_CHECK|{detail}`.
pub async fn check(owner: &str, repo: &str, current: &str) -> Result<UpdateInfo, String> {
    let err = |detail: String| format!("ERR_UPDATE_CHECK|{detail}");

    let owner = owner.trim();
    let repo = repo.trim();
    if owner.is_empty() || repo.is_empty() {
        return Err(err("owner/repo not configured".to_string()));
    }

    let client = reqwest::Client::builder()
        .user_agent("fire-speak")
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| err(format!("http client: {e}")))?;

    let api_url = format!("https://api.github.com/repos/{owner}/{repo}/releases/latest");
    let resp = client
        .get(&api_url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| err(e.to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(err(format!("HTTP {}", status.as_u16())));
    }
    let release: serde_json::Value = resp.json().await.map_err(|e| err(format!("parse: {e}")))?;

    let tag = release["tag_name"].as_str().unwrap_or("").trim();
    let latest = tag.trim_start_matches(['v', 'V']).to_string();
    if latest.is_empty() {
        return Err(err("release has no tag_name".to_string()));
    }
    let update_available = ver_gt(&parse_ver(&latest), &parse_ver(current));

    let empty = Vec::new();
    let assets = release["assets"].as_array().unwrap_or(&empty);
    // Only ever offer an installer built for this architecture. When the
    // release has none (or the names carry no architecture), fall back to the
    // release page below rather than handing over the wrong binary.
    let asset_url = INSTALLER_SUFFIXES
        .iter()
        .find_map(|suffix| {
            assets.iter().find(|a| {
                a["name"]
                    .as_str()
                    .map(|n| {
                        let n = n.to_lowercase();
                        n.ends_with(suffix) && ARCH_TOKENS.iter().any(|t| n.contains(t))
                    })
                    .unwrap_or(false)
            })
        })
        .and_then(|a| a["browser_download_url"].as_str())
        .map(|s| s.to_string());
    let url = asset_url
        .or_else(|| release["html_url"].as_str().map(|s| s.to_string()))
        .unwrap_or_default();

    let body = release["body"].as_str().unwrap_or("");
    let notes: String = if body.chars().count() > NOTES_MAX_CHARS {
        body.chars().take(NOTES_MAX_CHARS).collect()
    } else {
        body.to_string()
    };

    Ok(UpdateInfo {
        current: current.to_string(),
        latest,
        update_available,
        url,
        notes,
    })
}

/// How often a running app re-checks (v0.12.1). fire-speak starts with
/// Windows and stays in the tray for days, so a check at startup alone missed
/// every release published while it ran.
const RECHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// Let startup (hotkey, tray, local engine) settle before the first check.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(20);

/// Newest release found by any check (background or the settings button),
/// when it is newer than the running app.
static AVAILABLE: std::sync::Mutex<Option<UpdateInfo>> = std::sync::Mutex::new(None);
/// Version the desktop notification was last shown for (once per version).
static NOTIFIED: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

pub fn available() -> Option<UpdateInfo> {
    AVAILABLE.lock().ok().and_then(|g| g.clone())
}

/// Check now with the saved owner/repo and tell every surface about a newer
/// version (see `record`).
pub async fn check_now(app: &tauri::AppHandle) -> Result<UpdateInfo, String> {
    use tauri::Manager;
    let (owner, repo) = {
        let state = app.state::<crate::AppState>();
        let guard = state.settings.lock().unwrap();
        (guard.update.owner.clone(), guard.update.repo.clone())
    };
    let current = app.package_info().version.to_string();
    let info = check(&owner, &repo, &current).await?;
    record(app, &info);
    Ok(info)
}

/// Remember the result; on a newer version: the `update-available` event
/// (settings window banner/dialog/nav badge, HUD), the tray (menu item +
/// tooltip) and, once per version, a desktop notification.
fn record(app: &tauri::AppHandle, info: &UpdateInfo) {
    use tauri::Emitter;
    if !info.update_available {
        *AVAILABLE.lock().unwrap() = None;
        return;
    }
    let changed = {
        let mut guard = AVAILABLE.lock().unwrap();
        let changed = guard.as_ref().map(|i| i.latest.as_str()) != Some(info.latest.as_str());
        *guard = Some(info.clone());
        changed
    };
    let _ = app.emit("update-available", info);
    if changed {
        crate::refresh_tray(app);
    }
    let first_notice = {
        let mut notified = NOTIFIED.lock().unwrap();
        let first = *notified != info.latest;
        *notified = info.latest.clone();
        first
    };
    if first_notice {
        notify(app, &info.latest);
    }
}

fn notify(app: &tauri::AppHandle, latest: &str) {
    use tauri::Manager;
    use tauri_plugin_notification::NotificationExt;
    let lang = {
        let state = app.state::<crate::AppState>();
        let ui_lang = state.settings.lock().unwrap().ui_lang.clone();
        crate::locale::resolve_ui_lang_setting(&ui_lang)
    };
    let (_, title, body) = crate::locale::update_labels(&lang);
    if let Err(e) = app
        .notification()
        .builder()
        .title(title.replace("{0}", latest))
        .body(body)
        .show()
    {
        eprintln!("update notification failed: {e}");
    }
}

/// Background checks while `update.auto_check` is on: shortly after startup,
/// then every 6 hours. Failures are silent (the next round retries).
pub fn start_background_checks(app: tauri::AppHandle) {
    use tauri::Manager;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_DELAY).await;
        loop {
            let enabled = app.state::<crate::AppState>().settings.lock().unwrap().update.auto_check;
            if enabled {
                if let Err(e) = check_now(&app).await {
                    eprintln!("background update check failed: {e}");
                }
            }
            tokio::time::sleep(RECHECK_INTERVAL).await;
        }
    });
}

/// Parse "X.Y.Z" into numeric components; non-digit suffixes in a component
/// are ignored ("1.2.3-beta" -> [1, 2, 3]). Unparsable components become 0.
fn parse_ver(s: &str) -> Vec<u64> {
    // Compare only the numeric base: a prerelease/build suffix ("-beta.1",
    // "+build") must not contribute extra components that outrank the release.
    let s = s.trim();
    let base = s.split(['-', '+']).next().unwrap_or(s);
    base.split('.')
        .map(|part| {
            part.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<u64>()
                .unwrap_or(0)
        })
        .collect()
}

/// Numeric semver-style comparison with zero padding: a > b.
fn ver_gt(a: &[u64], b: &[u64]) -> bool {
    let len = a.len().max(b.len());
    for i in 0..len {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}
