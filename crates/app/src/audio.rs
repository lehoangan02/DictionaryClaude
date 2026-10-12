//! Recorded pronunciations: played from a local cache when present, otherwise
//! fetched once (online) and kept for offline use. `oald:<name>` recordings (the
//! Oxford voices) come from the downloadable `oald-audio.pack` instead and never
//! touch the network.

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

/// What `oald.pack` entries point their sounds at: `oald:laconic#_gb_2`.
const OALD_SCHEME: &str = "oald:";
const OALD_PACK: &str = "oald-audio.pack";

impl Api {
    /// One Oxford recording (MP3) from the recordings pack. Opened for each play: a click is rare, opening is cheap,
    /// and a pack that was just downloaded or replaced is found without restarting anything.
    fn oald_audio(&self, name: &str) -> Result<Vec<u8>> {
        let path = crate::packs::locate(&self.data_dir, &self.packs_dir, OALD_PACK);
        if !path.exists() {
            bail!("the OALD recordings are not downloaded (Settings → Downloads)");
        }
        let conn = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.query_row("SELECT mp3 FROM audio WHERE name = ?", [name], |r| r.get(0))
            .map_err(|_| anyhow!("OALD has no recording called {name}"))
    }

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
            .user_agent("LadyBird/0.1 (offline GRE dictionary; personal study app)")
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

    /// The recording's bytes (cached on first use), for players that cannot use a data URL.
    pub fn audio_bytes(&self, url: &str) -> Result<Vec<u8>> {
        if let Some(name) = url.strip_prefix(OALD_SCHEME) {
            return self.oald_audio(name);
        }
        Ok(std::fs::read(self.audio_fetch(url)?)?)
    }

    /// A playable data URL for a recording, from the cache or the network.
    pub(crate) fn audio_get(&self, url: &str) -> Result<Value> {
        if let Some(name) = url.strip_prefix(OALD_SCHEME) {
            let bytes = self.oald_audio(name)?;
            return Ok(json!({"data_url": format!("data:audio/mpeg;base64,{}", STANDARD.encode(bytes)), "cached": true}));
        }
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

    #[test]
    fn oald_recordings_come_from_the_pack_and_never_the_network() {
        let dir = std::env::temp_dir().join(format!("oald-play-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let api = Api::open_headless(&dir).unwrap();
        let err = api.audio_bytes("oald:laconic#_gb_2").unwrap_err().to_string();
        assert!(err.contains("not downloaded"), "without the pack the page falls back to the system voice: {err}");

        std::fs::create_dir_all(dir.join("packs")).unwrap();
        let c = rusqlite::Connection::open(dir.join("packs/oald-audio.pack")).unwrap();
        c.execute_batch("CREATE TABLE audio(name TEXT PRIMARY KEY, mp3 BLOB NOT NULL)").unwrap();
        c.execute("INSERT INTO audio VALUES('laconic#_gb_2', ?)", [vec![1u8, 2, 3]]).unwrap();
        drop(c);
        assert_eq!(api.audio_bytes("oald:laconic#_gb_2").unwrap(), vec![1, 2, 3]);
        let v = api.audio_get("oald:laconic#_gb_2").unwrap();
        assert_eq!(v["data_url"], "data:audio/mpeg;base64,AQID");
        assert!(api.audio_bytes("oald:nothing#_gb_1").unwrap_err().to_string().contains("no recording"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
