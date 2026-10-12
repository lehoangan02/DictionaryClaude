//! The OALD recordings → `oald-audio.pack`.
//!
//! OALData/tools/build_oald_audio.py writes one MP3 per recording (`audio/uk/<n>.mp3`, `audio/us/<n>.mp3`) and
//! `json/oald_audio.json` naming them. This packs them into a SQLite file the app reads one recording at a time:
//! `audio(name, mp3)`, where `name` is what `oald.pack` refers to as `oald:<name>`. MP3 is already compressed, so
//! the blobs are stored as they are.

use std::path::Path;

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection};

use crate::oald::read_audio_index;

pub fn build(dir: &Path, out: &Path) -> Result<()> {
    let idx = read_audio_index(dir)?.context("json/oald_audio.json is missing (run OALData/tools/build_oald_audio.py first)")?;
    if out.exists() {
        std::fs::remove_file(out)?;
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(out)?;
    conn.pragma_update(None, "journal_mode", "OFF")?;
    conn.pragma_update(None, "synchronous", "OFF")?;
    conn.execute_batch("CREATE TABLE audio(name TEXT PRIMARY KEY, mp3 BLOB NOT NULL); CREATE TABLE meta(k TEXT PRIMARY KEY, v TEXT NOT NULL); BEGIN")?;
    let mut count = 0usize;
    for (accent, names) in [("uk", &idx.files.uk), ("us", &idx.files.us)] {
        for (n, name) in names.iter().enumerate() {
            let path = dir.join("audio").join(accent).join(format!("{n}.mp3"));
            let bytes = std::fs::read(&path).with_context(|| path.display().to_string())?;
            if bytes.len() < 200 {
                bail!("{} looks empty", path.display());
            }
            conn.execute("INSERT INTO audio(name, mp3) VALUES (?, ?)", params![name, bytes]).with_context(|| format!("recording {name} twice?"))?;
            count += 1;
        }
    }
    conn.execute("INSERT INTO meta VALUES('source', 'OALD recordings (personal extraction)')", [])?;
    conn.execute("INSERT INTO meta VALUES('recordings', ?)", [count.to_string()])?;
    conn.execute_batch("COMMIT; VACUUM;")?;
    eprintln!("{count} recordings written to {} ({} MB)", out.display(), std::fs::metadata(out)?.len() / 1_000_000);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_every_listed_recording_by_name() {
        let dir = std::env::temp_dir().join(format!("oald-audio-test-{}", std::process::id()));
        for acc in ["uk", "us"] {
            std::fs::create_dir_all(dir.join("audio").join(acc)).unwrap();
        }
        std::fs::create_dir_all(dir.join("json")).unwrap();
        std::fs::write(dir.join("audio/uk/0.mp3"), vec![7u8; 300]).unwrap();
        std::fs::write(dir.join("audio/us/0.mp3"), vec![9u8; 400]).unwrap();
        std::fs::write(
            dir.join("json/oald_audio.json"),
            r##"{"entries":[{"h":"laconic","uk":"laconic#_gb_2","us":"laconic#_us_1"}],"files":{"uk":["laconic#_gb_2"],"us":["laconic#_us_1"]}}"##,
        )
        .unwrap();
        let out = dir.join("oald-audio.pack");
        build(&dir, &out).unwrap();
        let c = Connection::open(&out).unwrap();
        let n: i64 = c.query_row("SELECT COUNT(*) FROM audio", [], |r| r.get(0)).unwrap();
        let uk: Vec<u8> = c.query_row("SELECT mp3 FROM audio WHERE name='laconic#_gb_2'", [], |r| r.get(0)).unwrap();
        assert_eq!((n, uk.len(), uk[0]), (2, 300, 7));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
