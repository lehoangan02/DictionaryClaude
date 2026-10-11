//! Downloadable dictionary packs: what exists, what is installed, and
//! installing or removing one. Packs come from the paired server (over the same
//! pinned-TLS route as sync), are checksummed, and live in `<data>/packs`.
//! Packs bundled inside the app stay read-only and cannot be removed.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

use crate::api::Api;

/// id, title, what it is, licence, files.
const KNOWN: &[(&str, &str, &str, &str, &[&str])] = &[
    ("wiktionary", "Wiktionary", "The main dictionary: definitions, examples, IPA, etymology, synonyms, word forms and typo-tolerant search.", "CC BY-SA 4.0", &["wiktionary.pack", "wiktionary.fst"]),
    ("wordnet", "WordNet", "Princeton-style word senses with synonym groups. Open English WordNet 2024.", "CC BY 4.0", &["wordnet.pack"]),
    ("gcide", "Webster 1913", "The classic unabridged Webster, rich in literary words (GCIDE).", "GPL / public domain", &["gcide.pack"]),
    ("moby", "Moby Thesaurus", "Broad synonym lists that feed the GRE thesaurus.", "Public domain", &["moby.pack"]),
    ("ai", "Meaning search (AI model)", "A small language model (bge-small) that lets search find quiz sessions by meaning. It runs on this device, on the Neural Engine or phone NPU when there is one.", "MIT", &["bge-small-en-v1.5-q.onnx", "bge-small-en-v1.5-tokenizer.json"]),
    ("cmudict", "Pronunciations (CMU)", "IPA for words the dictionaries give none for.", "BSD", &["cmudict.pack"]),
];

const SOURCES_FILES: &[(&str, &str)] = &[("wiktionary", "wiktionary.pack"), ("wordnet", "wordnet.pack"), ("gcide", "gcide.pack")];

/// Where a pack file is read from: a downloaded copy wins over a bundled one.
pub(crate) fn locate(data_dir: &Path, bundled: &Path, file: &str) -> PathBuf {
    let own = data_dir.join("packs").join(file);
    if own.exists() { own } else { bundled.join(file) }
}

fn safe_name(f: &str) -> bool {
    !f.is_empty() && f.len() < 64 && f.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) && !f.starts_with('.') && (f.ends_with(".pack") || f.ends_with(".fst") || f.ends_with(".onnx") || f.ends_with(".json"))
}

static HASHES: Mutex<Option<HashMap<(String, u64, u64), String>>> = Mutex::new(None);

fn file_sha(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let meta = std::fs::metadata(path)?;
    let mtime = meta.modified()?.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let key = (path.to_string_lossy().to_string(), meta.len(), mtime);
    if let Some(h) = HASHES.lock().unwrap().as_ref().and_then(|m| m.get(&key)) {
        return Ok(h.clone());
    }
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    let sum: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    HASHES.lock().unwrap().get_or_insert_with(Default::default).insert(key, sum.clone());
    Ok(sum)
}

/// A pack's entry count remembered from the last time it was counted
/// (keyed by file size, so a replaced pack is counted again). 0 if unknown.
pub(crate) fn cached_count(db: &rusqlite::Connection, path: &Path) -> u64 {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let key = format!("packcount.{}.{size}", path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default());
    db.query_row("SELECT v FROM settings WHERE k=?", [key], |r| r.get::<_, String>(0)).ok().and_then(|v| v.parse().ok()).unwrap_or(0)
}

impl Api {
    /// Counts the entries of any pack not yet counted. Scans the whole file, so
    /// run it on a background thread after the app is up.
    pub fn warm_counts(&self) {
        for s in &self.sources {
            if s.entries.load(std::sync::atomic::Ordering::Relaxed) > 0 {
                continue;
            }
            let Ok(n) = s.pack.lock().unwrap().len() else { continue };
            s.entries.store(n, std::sync::atomic::Ordering::Relaxed);
            let file = SOURCES_FILES.iter().find(|(id, _)| *id == s.id).map(|x| x.1).unwrap_or("");
            let path = locate(&self.data_dir, &self.packs_dir, file);
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let _ = self.set_setting(&format!("packcount.{file}.{size}"), &n.to_string());
        }
    }

    fn own_packs(&self) -> PathBuf {
        self.data_dir.join("packs")
    }

    /// Server side: the pack files this machine offers.
    pub fn packs_served(&self) -> Result<Value> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(&self.packs_dir)?.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if safe_name(&name) {
                out.push(json!({"file": name, "size": e.metadata()?.len(), "sha256": file_sha(&e.path())?}));
            }
        }
        out.sort_by_key(|v| v["file"].as_str().unwrap_or("").to_string());
        Ok(json!({"files": out}))
    }

    /// Server side: a pack file opened at `offset`, with its length and checksum.
    pub fn pack_open_for_serving(&self, name: &str, offset: u64) -> Result<(std::fs::File, u64, String)> {
        if !safe_name(name) {
            bail!("no such pack");
        }
        let path = self.packs_dir.join(name);
        let mut f = std::fs::File::open(&path).map_err(|_| anyhow!("no such pack"))?;
        let len = f.metadata()?.len();
        f.seek(SeekFrom::Start(offset.min(len)))?;
        Ok((f, len, file_sha(&path)?))
    }

    /// What this device knows about each pack: installed (downloaded or built
    /// in) and, from the last look at the server, its size and whether a newer
    /// copy exists.
    pub(crate) fn packs_list(&self) -> Result<Value> {
        let remote: Value = self.setting("packs.remote").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
        let find = |f: &str| remote["files"].as_array().and_then(|a| a.iter().find(|x| x["file"] == f)).cloned();
        let mut out = Vec::new();
        for (id, title, about, licence, files) in KNOWN {
            // The .fst is an optional extra: only the main file decides.
            let main = files[0];
            let downloaded = self.own_packs().join(main).exists();
            let built_in = !downloaded && self.packs_dir.join(main).exists();
            let size_local: u64 = files.iter().filter_map(|f| std::fs::metadata(locate(&self.data_dir, &self.packs_dir, f)).ok()).map(|m| m.len()).sum();
            let size_remote: u64 = files.iter().filter_map(|f| find(f)).filter_map(|x| x["size"].as_u64()).sum();
            let on_server = files.iter().all(|f| find(f).is_some()) || (find(main).is_some());
            let update = downloaded
                && files.iter().any(|f| {
                    find(f).is_some_and(|x| self.setting(&format!("packs.sha.{f}")).is_some_and(|have| x["sha256"].as_str() != Some(have.as_str())))
                });
            out.push(json!({"id": id, "title": title, "about": about, "licence": licence, "downloaded": downloaded, "built_in": built_in,
                            "size": if size_local > 0 { size_local } else { size_remote }, "remote_size": size_remote, "on_server": on_server, "update": update}));
        }
        Ok(json!({"packs": out, "checked": remote["at"], "paired": self.pairing().is_some()}))
    }

    /// Asks the server what it offers (and remembers it).
    pub(crate) fn packs_catalog(&self) -> Result<Value> {
        let (host, p, a) = self.server_conn(Duration::from_secs(20))?;
        let mut v = crate::net::post(&a, &host, &p.token, "/v1/packs", &json!({}))?;
        v["at"] = json!(crate::userdb::now_ms());
        self.set_setting("packs.remote", &v.to_string())?;
        self.packs_list()
    }

    pub(crate) fn pack_progress_now(&self) -> Value {
        let m = self.pack_progress.lock().unwrap();
        json!(m.iter().map(|(k, (done, total))| (k.clone(), json!({"done": done, "total": total}))).collect::<serde_json::Map<_, _>>())
    }

    pub(crate) fn pack_install(&self, id: &str) -> Result<Value> {
        let (_, _, _, _, files) = KNOWN.iter().find(|k| k.0 == id).ok_or_else(|| anyhow!("unknown dictionary"))?;
        if self.pack_progress.lock().unwrap().contains_key(id) {
            bail!("already downloading");
        }
        self.packs_catalog()?;
        let remote: Value = self.setting("packs.remote").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
        let total: u64 = files.iter().filter_map(|f| remote["files"].as_array()?.iter().find(|x| x["file"] == *f)?["size"].as_u64()).sum();
        self.pack_progress.lock().unwrap().insert(id.to_string(), (0, total));
        let result = (|| -> Result<()> {
            let mut done_before = 0u64;
            for f in files.iter() {
                let Some(info) = remote["files"].as_array().and_then(|a| a.iter().find(|x| x["file"] == *f)).cloned() else {
                    if *f == files[0] { bail!("the server does not have this download") }
                    continue; // an optional extra (the typo index) the server lacks
                };
                self.download_file(id, f, &info, done_before)?;
                done_before += info["size"].as_u64().unwrap_or(0);
            }
            Ok(())
        })();
        self.pack_progress.lock().unwrap().remove(id);
        result?;
        Ok(json!({"installed": id, "reload": true}))
    }

    fn download_file(&self, id: &str, file: &str, info: &Value, done_before: u64) -> Result<()> {
        let size = info["size"].as_u64().unwrap_or(0);
        let want = info["sha256"].as_str().unwrap_or("").to_string();
        let dir = self.own_packs();
        std::fs::create_dir_all(&dir)?;
        let part = dir.join(format!("{file}.part"));
        for attempt in 0..4 {
            let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0).min(size);
            let step = (|| -> Result<()> {
                if have < size {
                    let (host, p, a) = self.server_conn(Duration::from_secs(3600))?;
                    let mut resp = a
                        .post(&format!("https://{host}/v1/pack/get"))
                        .header("Authorization", &format!("Bearer {}", p.token))
                        .header("Content-Type", "application/json")
                        .send(serde_json::to_vec(&json!({"file": file, "offset": have}))?)?;
                    if resp.status().as_u16() != 200 {
                        bail!("server answered {}", resp.status());
                    }
                    let mut out = std::fs::OpenOptions::new().create(true).append(true).open(&part)?;
                    let mut r = resp.body_mut().with_config().limit(4 << 30).reader();
                    let mut buf = vec![0u8; 256 << 10];
                    let mut got = have;
                    loop {
                        let n = r.read(&mut buf)?;
                        if n == 0 {
                            break;
                        }
                        out.write_all(&buf[..n])?;
                        got += n as u64;
                        if let Some(e) = self.pack_progress.lock().unwrap().get_mut(id) {
                            e.0 = done_before + got;
                        }
                    }
                    out.flush()?;
                }
                Ok(())
            })();
            let len = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
            if len == size {
                break;
            }
            if let Err(e) = step {
                if attempt == 3 {
                    return Err(e).context("the download was interrupted (it will resume next time)");
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        if std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0) != size {
            bail!("the download is incomplete (it will resume next time)");
        }
        if !want.is_empty() && file_sha(&part)? != want {
            let _ = std::fs::remove_file(&part);
            bail!("the downloaded file is damaged (checksum mismatch); try again");
        }
        if file.ends_with(".pack") {
            // Every pack is a SQLite database; refuse anything else before it replaces a working copy.
            let mut head = [0u8; 16];
            std::fs::File::open(&part)?.read_exact(&mut head)?;
            if &head != b"SQLite format 3\0" {
                let _ = std::fs::remove_file(&part);
                bail!("the downloaded file is not a dictionary");
            }
        }
        std::fs::rename(&part, dir.join(file))?;
        self.set_setting(&format!("packs.sha.{file}"), &want)?;
        Ok(())
    }

    pub(crate) fn pack_remove(&self, id: &str) -> Result<Value> {
        let (_, _, _, _, files) = KNOWN.iter().find(|k| k.0 == id).ok_or_else(|| anyhow!("unknown dictionary"))?;
        for f in files.iter() {
            let _ = std::fs::remove_file(self.own_packs().join(f));
            let _ = std::fs::remove_file(self.own_packs().join(format!("{f}.part")));
            let _ = self.db.lock().unwrap().execute("DELETE FROM settings WHERE k=?", [format!("packs.sha.{f}")]);
        }
        Ok(json!({"removed": id, "reload": true}))
    }
}
