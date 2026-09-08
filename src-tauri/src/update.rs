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
    let asset_url = INSTALLER_SUFFIXES
        .iter()
        .find_map(|suffix| {
            assets.iter().find(|a| {
                a["name"]
                    .as_str()
                    .map(|n| n.to_lowercase().ends_with(suffix))
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
