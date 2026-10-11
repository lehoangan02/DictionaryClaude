//! Self-update: asks GitHub for the newest release, downloads the file that fits this
//! platform, checks it against the release's SHA-256 and leaves it ready for the shell
//! to install (the shell restarts the app, which this library cannot do by itself).

use std::io::{Read, Write};
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::api::Api;

const LATEST: &str = "https://api.github.com/repos/lehoangan02/DictionaryClaude/releases/latest";

/// The file name suffix of this platform's download in a release.
fn suffix() -> Option<&'static str> {
    if cfg!(target_os = "android") {
        Some("-android-arm64.apk")
    } else if cfg!(target_os = "macos") {
        Some("-macos-arm64.dmg")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("-x86_64.AppImage")
    } else {
        None
    }
}

fn parse(v: &str) -> Vec<u64> {
    v.trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// Whether `latest` is a higher version than `current` (both `x.y.z`).
pub fn is_newer(latest: &str, current: &str) -> bool {
    parse(latest) > parse(current)
}

fn agent(secs: u64) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(secs))).http_status_as_error(false).build().into()
}

impl Api {
    /// What the newest release offers this device, and whether it is newer than `current`.
    pub(crate) fn update_check(&self, current: &str) -> Result<Value> {
        let base = std::env::var("LEXIS_UPDATE_URL").unwrap_or_else(|_| LATEST.to_string());
        let mut resp = agent(20).get(&base).header("Accept", "application/vnd.github+json").header("User-Agent", "LadyBird").call().map_err(|e| anyhow!("could not reach GitHub ({e})"))?;
        if resp.status().as_u16() != 200 {
            bail!("GitHub answered {}", resp.status().as_u16());
        }
        let rel: Value = serde_json::from_str(&resp.body_mut().read_to_string()?)?;
        let tag = rel["tag_name"].as_str().unwrap_or("").to_string();
        let latest = tag.trim_start_matches('v').to_string();
        let assets = rel["assets"].as_array().cloned().unwrap_or_default();
        let find = |pred: &dyn Fn(&str) -> bool| assets.iter().find(|a| a["name"].as_str().is_some_and(pred)).cloned();
        let file = suffix().and_then(|s| find(&|n| n.ends_with(s)));
        let sha = file.as_ref().and_then(|f| {
            let want = format!("{}.sha256", f["name"].as_str().unwrap_or(""));
            find(&|n| n == want)
        });
        Ok(json!({
            "current": current, "latest": latest, "newer": is_newer(&latest, current),
            "notes": rel["body"], "page": rel["html_url"],
            "asset": file.map(|f| json!({"name": f["name"], "url": f["browser_download_url"], "size": f["size"], "sha256_url": sha.map(|s| s["browser_download_url"].clone())})),
        }))
    }

    /// Downloads the asset into the app's data folder and verifies it; returns where it is.
    pub(crate) fn update_download(&self, asset: &Value) -> Result<Value> {
        let name = asset["name"].as_str().ok_or_else(|| anyhow!("missing the file name"))?;
        let url = asset["url"].as_str().ok_or_else(|| anyhow!("missing the download address"))?;
        // Only a plain release file name, never a path.
        if name.contains('/') || name.contains("..") || !name.starts_with("LadyBird-") {
            bail!("that is not a LadyBird release file");
        }
        if !url.starts_with("https://github.com/lehoangan02/DictionaryClaude/releases/download/") && std::env::var("LEXIS_UPDATE_URL").is_err() {
            bail!("downloads only come from this project's releases");
        }
        let dir = self.data_dir.join("updates");
        std::fs::create_dir_all(&dir)?;
        let part = dir.join(format!("{name}.part"));
        let dest = dir.join(name);
        let mut resp = agent(900).get(url).header("User-Agent", "LadyBird").call().map_err(|e| anyhow!("download failed ({e})"))?;
        if resp.status().as_u16() != 200 {
            bail!("the download answered {}", resp.status().as_u16());
        }
        let mut hasher = Sha256::new();
        {
            let mut out = std::fs::File::create(&part)?;
            let mut body = resp.body_mut().as_reader();
            let mut buf = vec![0u8; 1 << 16];
            loop {
                let n = body.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                out.write_all(&buf[..n])?;
            }
        }
        let got: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        let verified = match asset["sha256_url"].as_str() {
            Some(u) => {
                let mut r = agent(30).get(u).header("User-Agent", "LadyBird").call().map_err(|e| anyhow!("could not fetch the checksum ({e})"))?;
                let text = r.body_mut().read_to_string()?;
                let want = text.split_whitespace().next().unwrap_or("").to_lowercase();
                if want != got {
                    let _ = std::fs::remove_file(&part);
                    bail!("the download does not match its checksum; it was discarded");
                }
                true
            }
            // Android APKs carry no checksum file; the system checks their signature when installing.
            None => false,
        };
        std::fs::rename(&part, &dest)?;
        Ok(json!({"path": dest, "sha256": got, "verified": verified}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("0.6.1", "0.6.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("v1.0.0", "0.99.0"));
        assert!(!is_newer("0.6.0", "0.6.0"));
        assert!(!is_newer("0.5.9", "0.6.0"));
    }
}
