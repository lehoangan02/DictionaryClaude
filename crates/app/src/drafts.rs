//! Photographed notes: stored as files, tracked as `drafts` rows (synced), read
//! by the server's job runner, reviewed and accepted by the person.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use rusqlite::params;
use serde_json::{json, Value};

use crate::api::Api;
use crate::userdb::{new_id, now_ms};

const MAX_PHOTO: usize = 10 << 20;
const MAX_PHOTOS: usize = 12;

/// "<32 hex>.jpg|png": the only file names ever read or written, so a crafted
/// name cannot point outside the photos folder.
pub fn valid_photo_name(n: &str) -> bool {
    let Some((stem, ext)) = n.split_once('.') else { return false };
    stem.len() == 32 && stem.bytes().all(|b| b.is_ascii_hexdigit()) && matches!(ext, "jpg" | "png")
}

fn sniff(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0xFF, 0xD8, 0xFF, ..] => Some("jpg"),
        [0x89, b'P', b'N', b'G', ..] => Some("png"),
        _ => None,
    }
}

impl Api {
    pub fn photos_dir(&self) -> Result<PathBuf> {
        let d = self.data_dir.join("photos");
        std::fs::create_dir_all(&d)?;
        Ok(d)
    }

    /// Stores an uploaded photo under its own name (server side).
    pub fn photo_store(&self, name: &str, bytes: &[u8]) -> Result<()> {
        if !valid_photo_name(name) {
            bail!("bad photo name");
        }
        if bytes.len() > MAX_PHOTO || sniff(bytes) != name.rsplit('.').next().map(|e| if e == "jpg" { "jpg" } else { "png" }) {
            bail!("not an acceptable image");
        }
        let p = self.photos_dir()?.join(name);
        if !p.exists() {
            std::fs::write(&p, bytes)?;
        }
        Ok(())
    }

    pub fn photo_read(&self, name: &str) -> Option<Vec<u8>> {
        valid_photo_name(name).then(|| std::fs::read(self.photos_dir().ok()?.join(name)).ok()).flatten()
    }

    /// Names of photos on this device that the server has not received yet.
    pub(crate) fn photos_to_upload(&self) -> Vec<String> {
        let Ok(dir) = self.photos_dir() else { return vec![] };
        let db = self.db.lock().unwrap();
        let Ok(mut st) = db.prepare("SELECT photos FROM drafts WHERE deleted=0 AND status NOT IN ('accepted','rejected')") else { return vec![] };
        let lists: Vec<String> = st.query_map([], |r| r.get(0)).map(|r| r.flatten().collect()).unwrap_or_default();
        lists
            .iter()
            .flat_map(|j| serde_json::from_str::<Vec<String>>(j).unwrap_or_default())
            .filter(|n| valid_photo_name(n) && dir.join(n).exists() && !dir.join(format!("{n}.sent")).exists())
            .collect()
    }

    pub(crate) fn photo_mark_sent(&self, name: &str) {
        if let Ok(d) = self.photos_dir() {
            let _ = std::fs::write(d.join(format!("{name}.sent")), b"");
        }
    }

    /// `photos` are base64 images (the UI shrinks them first). One draft per batch.
    pub(crate) fn draft_create(&self, photos: &[String], session_date: &str, tags: &str) -> Result<Value> {
        if photos.is_empty() || photos.len() > MAX_PHOTOS {
            bail!("add between 1 and {MAX_PHOTOS} photos");
        }
        let dir = self.photos_dir()?;
        let mut names = Vec::new();
        for p in photos {
            let b64 = p.split_once(',').map_or(p.as_str(), |(_, rest)| rest); // accept data URLs
            let bytes = STANDARD.decode(b64.trim()).map_err(|_| anyhow!("a photo could not be read"))?;
            if bytes.len() > MAX_PHOTO {
                bail!("a photo is larger than 10 MB");
            }
            let ext = sniff(&bytes).ok_or_else(|| anyhow!("only JPEG and PNG photos are supported"))?;
            let name = format!("{}.{ext}", new_id());
            std::fs::write(dir.join(&name), &bytes)?;
            names.push(name);
        }
        let (id, now) = (new_id(), now_ms());
        self.db.lock().unwrap().execute(
            "INSERT INTO drafts(id,created,updated,status,photos,session_date,tags) VALUES (?,?,?, 'queued', ?,?,?)",
            params![id, now, now, json!(names).to_string(), session_date, tags],
        )?;
        Ok(json!({"id": id, "photos": names}))
    }

    fn draft_row(r: &rusqlite::Row) -> rusqlite::Result<Value> {
        let j = |i: usize| -> Value { serde_json::from_str(&r.get::<_, String>(i).unwrap_or_default()).unwrap_or(Value::Null) };
        Ok(json!({
            "id": r.get::<_, String>(0)?, "created": r.get::<_, i64>(1)?, "updated": r.get::<_, i64>(2)?,
            "status": r.get::<_, String>(3)?, "photos": j(4), "session_date": r.get::<_, String>(5)?, "tags": r.get::<_, String>(6)?,
            "ocr_text": r.get::<_, String>(7)?, "ocr_engine": r.get::<_, String>(8)?, "agent_status": r.get::<_, String>(9)?,
            "agents": j(10), "text": r.get::<_, String>(11)?, "note_id": r.get::<_, Option<String>>(12)?, "error": r.get::<_, String>(13)?,
        }))
    }

    const DRAFT_COLS: &'static str = "id,created,updated,status,photos,session_date,tags,ocr_text,ocr_engine,agent_status,agents,text,note_id,error";

    pub(crate) fn drafts_list(&self, include_done: bool) -> Result<Value> {
        let db = self.db.lock().unwrap();
        let mut st = db.prepare(&format!(
            "SELECT {} FROM drafts WHERE deleted=0 AND (?1 OR status NOT IN ('accepted','rejected')) ORDER BY created DESC LIMIT 100",
            Self::DRAFT_COLS
        ))?;
        let rows = st.query_map([include_done], Self::draft_row)?.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(json!(rows))
    }

    pub(crate) fn draft_get(&self, id: &str) -> Result<Value> {
        let db = self.db.lock().unwrap();
        db.query_row(&format!("SELECT {} FROM drafts WHERE id=?1 AND deleted=0", Self::DRAFT_COLS), [id], Self::draft_row).map_err(|_| anyhow!("no such draft"))
    }

    pub(crate) fn draft_set(&self, id: &str, sql_set: &str, p: impl rusqlite::Params) -> Result<()> {
        let n = self.db.lock().unwrap().execute(&format!("UPDATE drafts SET {sql_set}, updated=?{} WHERE id=?{} AND deleted=0", 1, 2), p)?;
        if n == 0 {
            bail!("no such draft: {id}");
        }
        Ok(())
    }

    /// The person's edits to the text; also what an accepted draft becomes.
    pub(crate) fn draft_update(&self, id: &str, text: &str, session_date: Option<&str>, tags: Option<&str>) -> Result<Value> {
        let n = self.db.lock().unwrap().execute(
            "UPDATE drafts SET text=?2, session_date=coalesce(?3, session_date), tags=coalesce(?4, tags), updated=?5 WHERE id=?1 AND deleted=0 AND status NOT IN ('accepted','rejected')",
            params![id, text, session_date, tags, now_ms()],
        )?;
        if n == 0 {
            bail!("that draft can no longer be edited");
        }
        self.draft_get(id)
    }

    /// Turns the (reviewed) text into a note. Nothing becomes a note without this.
    pub(crate) fn draft_accept(&self, id: &str, text: Option<&str>) -> Result<Value> {
        let d = self.draft_get(id)?;
        if matches!(d["status"].as_str(), Some("accepted" | "rejected")) {
            bail!("that draft was already closed");
        }
        let body = text.map(str::to_string).unwrap_or_else(|| d["text"].as_str().unwrap_or("").to_string());
        if body.trim().is_empty() {
            bail!("there is no text to save");
        }
        let saved = self.call("note_save", json!({"body": body, "session_date": d["session_date"], "tags": d["tags"]}))?;
        self.db.lock().unwrap().execute(
            "UPDATE drafts SET status='accepted', text=?2, note_id=?3, updated=?4 WHERE id=?1",
            params![id, body, saved["id"].as_str(), now_ms()],
        )?;
        Ok(json!({"note": saved, "draft": self.draft_get(id)?}))
    }

    pub(crate) fn draft_reject(&self, id: &str) -> Result<Value> {
        self.draft_set(id, "status='rejected'", params![now_ms(), id])?;
        Ok(json!({"ok": true}))
    }

    /// A photo's bytes as a data URL, from this device or (when paired) the server.
    pub(crate) fn photo_data(&self, name: &str) -> Result<Value> {
        let bytes = self.photo_read(name).or_else(|| self.photo_fetch_from_server(name).ok().flatten()).ok_or_else(|| anyhow!("that photo is not on this device yet; sync to receive it"))?;
        let mime = if name.ends_with(".png") { "image/png" } else { "image/jpeg" };
        Ok(json!({"data_url": format!("data:{mime};base64,{}", STANDARD.encode(bytes))}))
    }
}

#[allow(dead_code)]
fn _path(_: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photo_names_cannot_escape_the_folder() {
        assert!(valid_photo_name("0123456789abcdef0123456789abcdef.jpg"));
        assert!(valid_photo_name("0123456789abcdef0123456789abcdef.png"));
        for bad in ["../0123456789abcdef0123456789abcdef.jpg", "x.jpg", "0123456789abcdef0123456789abcdef.exe", "0123456789abcdef0123456789abcdef.jpg/../a", "0123456789abcdef0123456789abcdeg.jpg", ""] {
            assert!(!valid_photo_name(bad), "{bad}");
        }
    }

    #[test]
    fn sniffs_real_image_types_only() {
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0, 0]), Some("jpg"));
        assert_eq!(sniff(&[0x89, b'P', b'N', b'G', 0]), Some("png"));
        assert_eq!(sniff(b"GIF89a"), None);
        assert_eq!(sniff(b"<svg onload=alert(1)>"), None);
    }
}
