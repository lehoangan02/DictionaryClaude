//! Recorded pronunciations: played from a local cache when present, otherwise
//! fetched once (online) and kept for offline use.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::api::Api;

/// Marker for "slow down": callers stop and try again later.
pub(crate) const RATE_LIMITED: &str = "the server asked us to slow down (rate limited)";

/// Only these hosts are fetched, so a crafted entry cannot make the app
/// request arbitrary addresses.
const HOSTS: &[&str] = &["upload.wikimedia.org", "media.merriam-webster.com"];

fn host_allowed(url: &str) -> bool {
    url.strip_prefix("https://").and_then(|r| r.split('/').next()).is_some_and(|h| HOSTS.contains(&h))
}

fn cache_name(url: &str) -> String {
    let h: String = Sha256::digest(url.as_bytes()).iter().take(12).map(|b| format!("{b:02x}")).collect();
    let ext = if url.ends_with(".ogg") { "ogg" } else { "mp3" };
    format!("{h}.{ext}")
}

fn mime(path: &std::path::Path) -> &'static str {
    if path.extension().is_some_and(|e| e == "ogg") { "audio/ogg" } else { "audio/mpeg" }
}

impl Api {
    fn audio_dir(&self) -> Result<PathBuf> {
        let d = self.data_dir.join("audio");
        std::fs::create_dir_all(&d)?;
        Ok(d)
    }

    /// Fetches `url` into the cache unless it is already there.
    fn audio_fetch(&self, url: &str) -> Result<PathBuf> {
        if !host_allowed(url) {
            bail!("audio from that address is not allowed");
        }
        let path = self.audio_dir()?.join(cache_name(url));
        if path.exists() {
            return Ok(path);
        }
        // Wikimedia asks clients to identify themselves.
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .user_agent("Lexis/0.1 (offline GRE dictionary; personal study app)")
            .timeout_global(Some(Duration::from_secs(15)))
            .http_status_as_error(false)
            .build()
            .into();
        let mut resp = agent.get(url).call().map_err(|e| anyhow!("could not download the recording ({e})"))?;
        match resp.status().as_u16() {
            200 => {}
            429 => {
                let wait = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok()).unwrap_or(30);
                bail!("{RATE_LIMITED}; retry after {wait}s");
            }
            s => bail!("could not download the recording (status {s})"),
        }
        let bytes = resp.body_mut().with_config().limit(4 << 20).read_to_vec()?;
        if bytes.len() < 200 {
            bail!("the recording looks empty");
        }
        let tmp = path.with_extension("part");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &path)?;
        Ok(path)
    }

    /// A playable data URL for a recording, from the cache or the network.
    pub(crate) fn audio_get(&self, url: &str) -> Result<Value> {
        let path = self.audio_fetch(url)?;
        let bytes = std::fs::read(&path)?;
        Ok(json!({"data_url": format!("data:{};base64,{}", mime(&path), STANDARD.encode(bytes)), "cached": true}))
    }

    /// Caches recordings for the Barron's words, `limit` at a time, so the UI
    /// can show progress and stop whenever it likes.
    pub(crate) fn audio_prefetch(&self, limit: usize) -> Result<Value> {
        let barrons = self.barrons().ok_or_else(|| anyhow!("Barron's is not on this device yet"))?;
        let wikt = self.sources.iter().find(|s| s.id == "wiktionary").ok_or_else(|| anyhow!("no Wiktionary pack"))?;
        let dir = self.audio_dir()?;
        let mut keys: Vec<&String> = barrons.entries.keys().collect();
        keys.sort();
        let (mut have, mut pending, mut none) = (0usize, Vec::new(), 0usize);
        for k in keys {
            let url = wikt.pack.lock().unwrap().get(k)?.and_then(|e| e.blocks.iter().flat_map(|b| b.sounds.iter()).find_map(|s| s.audio.clone()));
            match url {
                None => none += 1,
                Some(u) if dir.join(cache_name(&u)).exists() => {
                    let _ = u;
                    have += 1;
                }
                Some(u) => pending.push(u),
            }
        }
        let total = have + pending.len();
        let (mut fetched, mut failed) = (0usize, 0usize);
        let mut last_error = None;
        let mut rate_limited = false;
        let mut retry_after = 0u64;
        for (n, u) in pending.iter().take(limit).enumerate() {
            if n > 0 {
                // Be gentle: one recording at a time, not a burst.
                std::thread::sleep(Duration::from_millis(2000));
            }
            match self.audio_fetch(u) {
                Ok(_) => fetched += 1,
                Err(e) if format!("{e:#}").contains(RATE_LIMITED) => {
                    rate_limited = true;
                    let msg = format!("{e:#}");
                    retry_after = msg.rsplit("retry after ").next().and_then(|t| t.trim_end_matches('s').parse().ok()).unwrap_or(30);
                    last_error = Some(msg);
                    break;
                }
                Err(e) => {
                    failed += 1;
                    last_error = Some(format!("{e:#}"));
                    // Offline or blocked: stop instead of failing every word in turn.
                    if failed >= 3 && fetched == 0 {
                        break;
                    }
                }
            }
        }
        Ok(json!({"total": total, "done": have + fetched, "no_audio": none, "fetched": fetched, "failed": failed, "rate_limited": rate_limited, "retry_after": retry_after, "error": last_error}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_hosts_and_stable_cache_names() {
        assert!(host_allowed("https://upload.wikimedia.org/wikipedia/commons/x.ogg"));
        assert!(!host_allowed("https://evil.example/x.ogg"));
        assert!(!host_allowed("http://upload.wikimedia.org/x.ogg"), "plain http is refused");
        assert!(!host_allowed("https://upload.wikimedia.org.evil.example/x.ogg"));
        let n = cache_name("https://upload.wikimedia.org/a.mp3");
        assert_eq!(n, cache_name("https://upload.wikimedia.org/a.mp3"));
        assert!(n.ends_with(".mp3") && cache_name("https://upload.wikimedia.org/a.ogg").ends_with(".ogg"));
    }
}
