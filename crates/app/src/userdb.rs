//! The user's own data: history, notes, quiz sessions, settings.
//!
//! Schema changes are *forward-only numbered migrations*. Never edit a shipped
//! entry of [`MIGRATIONS`]; add a new one. Before migrating an existing
//! database, a copy is kept next to it as `<file>.bak-v<old>` so an update can
//! always be rolled back by hand. Rows use text ids (uuid v4) and millisecond
//! timestamps so they merge cleanly across devices later.

use std::path::Path;

use anyhow::{bail, Context, Result};
use rusqlite::Connection;

pub const MIGRATIONS: &[&str] = &[
    // 1: initial schema
    "
    CREATE TABLE settings(k TEXT PRIMARY KEY, v TEXT NOT NULL) WITHOUT ROWID;

    CREATE TABLE lookups(
      id TEXT PRIMARY KEY, ts INTEGER NOT NULL, key TEXT NOT NULL, word TEXT NOT NULL,
      source TEXT NOT NULL, device TEXT NOT NULL, starred INTEGER NOT NULL DEFAULT 0);
    CREATE INDEX lookups_ts ON lookups(ts DESC);
    CREATE INDEX lookups_key ON lookups(key);

    CREATE TABLE notes(
      id TEXT PRIMARY KEY, created INTEGER NOT NULL, updated INTEGER NOT NULL,
      session_date TEXT NOT NULL DEFAULT '', tags TEXT NOT NULL DEFAULT '',
      body TEXT NOT NULL, deleted INTEGER NOT NULL DEFAULT 0);

    CREATE TABLE note_edges(
      note_id TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
      a TEXT NOT NULL, b TEXT NOT NULL, rel TEXT NOT NULL);
    CREATE INDEX note_edges_a ON note_edges(a);
    CREATE INDEX note_edges_b ON note_edges(b);

    CREATE TABLE sessions(
      id TEXT PRIMARY KEY, kind TEXT NOT NULL, title TEXT NOT NULL, spec TEXT NOT NULL,
      started INTEGER NOT NULL, ended INTEGER, total INTEGER NOT NULL, correct INTEGER NOT NULL DEFAULT 0);
    CREATE INDEX sessions_started ON sessions(started DESC);

    CREATE TABLE attempts(
      id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
      item_id TEXT NOT NULL, prompt TEXT NOT NULL, given TEXT NOT NULL, expected TEXT NOT NULL,
      correct INTEGER NOT NULL, ms INTEGER NOT NULL, ts INTEGER NOT NULL);
    CREATE INDEX attempts_session ON attempts(session_id);
    CREATE INDEX attempts_item ON attempts(item_id);
    ",
    // 2: full-text index over quiz sessions (retrieval search)
    "
    CREATE VIRTUAL TABLE session_fts USING fts5(session_id UNINDEXED, body, tokenize='porter unicode61');
    ",
    // 3: row-level sync. Every synced row carries `updated` (ms) for
    // last-writer-wins merging, and deletions become tombstones.
    "
    ALTER TABLE lookups  ADD COLUMN updated INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE lookups  ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE sessions ADD COLUMN updated INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE sessions ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE attempts ADD COLUMN updated INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE attempts ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0;
    UPDATE lookups  SET updated = ts;
    UPDATE sessions SET updated = coalesce(ended, started);
    UPDATE attempts SET updated = ts;
    CREATE INDEX lookups_updated  ON lookups(updated);
    CREATE INDEX sessions_updated ON sessions(updated);
    CREATE INDEX attempts_updated ON attempts(updated);
    CREATE INDEX notes_updated    ON notes(updated);
    -- Every insert/update gets a monotonically increasing sequence number, so
    -- peers can pull everything after sequence N, whichever process wrote it.
    CREATE TABLE sync_counter(n INTEGER NOT NULL);
    INSERT INTO sync_counter VALUES(0);
    CREATE TABLE sync_seq(tbl TEXT NOT NULL, id TEXT NOT NULL, seq INTEGER NOT NULL, PRIMARY KEY(tbl, id)) WITHOUT ROWID;
    CREATE INDEX sync_seq_seq ON sync_seq(seq);
    CREATE TRIGGER notes_ai AFTER INSERT ON notes BEGIN
      UPDATE sync_counter SET n = n + 1;
      INSERT INTO sync_seq(tbl,id,seq) VALUES('notes', NEW.id, (SELECT n FROM sync_counter))
        ON CONFLICT(tbl,id) DO UPDATE SET seq=excluded.seq;
    END;
    CREATE TRIGGER notes_au AFTER UPDATE ON notes BEGIN
      UPDATE sync_counter SET n = n + 1;
      INSERT INTO sync_seq(tbl,id,seq) VALUES('notes', NEW.id, (SELECT n FROM sync_counter))
        ON CONFLICT(tbl,id) DO UPDATE SET seq=excluded.seq;
    END;
    CREATE TRIGGER lookups_ai AFTER INSERT ON lookups BEGIN
      UPDATE sync_counter SET n = n + 1;
      INSERT INTO sync_seq(tbl,id,seq) VALUES('lookups', NEW.id, (SELECT n FROM sync_counter))
        ON CONFLICT(tbl,id) DO UPDATE SET seq=excluded.seq;
    END;
    CREATE TRIGGER lookups_au AFTER UPDATE ON lookups BEGIN
      UPDATE sync_counter SET n = n + 1;
      INSERT INTO sync_seq(tbl,id,seq) VALUES('lookups', NEW.id, (SELECT n FROM sync_counter))
        ON CONFLICT(tbl,id) DO UPDATE SET seq=excluded.seq;
    END;
    CREATE TRIGGER sessions_ai AFTER INSERT ON sessions BEGIN
      UPDATE sync_counter SET n = n + 1;
      INSERT INTO sync_seq(tbl,id,seq) VALUES('sessions', NEW.id, (SELECT n FROM sync_counter))
        ON CONFLICT(tbl,id) DO UPDATE SET seq=excluded.seq;
    END;
    CREATE TRIGGER sessions_au AFTER UPDATE ON sessions BEGIN
      UPDATE sync_counter SET n = n + 1;
      INSERT INTO sync_seq(tbl,id,seq) VALUES('sessions', NEW.id, (SELECT n FROM sync_counter))
        ON CONFLICT(tbl,id) DO UPDATE SET seq=excluded.seq;
    END;
    CREATE TRIGGER attempts_ai AFTER INSERT ON attempts BEGIN
      UPDATE sync_counter SET n = n + 1;
      INSERT INTO sync_seq(tbl,id,seq) VALUES('attempts', NEW.id, (SELECT n FROM sync_counter))
        ON CONFLICT(tbl,id) DO UPDATE SET seq=excluded.seq;
    END;
    CREATE TRIGGER attempts_au AFTER UPDATE ON attempts BEGIN
      UPDATE sync_counter SET n = n + 1;
      INSERT INTO sync_seq(tbl,id,seq) VALUES('attempts', NEW.id, (SELECT n FROM sync_counter))
        ON CONFLICT(tbl,id) DO UPDATE SET seq=excluded.seq;
    END;
    -- Stamp rows that already exist.
    UPDATE notes    SET updated = updated;
    UPDATE lookups  SET updated = updated;
    UPDATE sessions SET updated = updated;
    UPDATE attempts SET updated = updated;
    ",
    // 4: photographed notes waiting to be read and reviewed. The photos are
    // files (sent separately); this row carries their names and the results.
    "
    CREATE TABLE drafts(
      id TEXT PRIMARY KEY, created INTEGER NOT NULL, updated INTEGER NOT NULL, deleted INTEGER NOT NULL DEFAULT 0,
      -- queued | processing | needs_review | accepted | rejected | failed
      status TEXT NOT NULL,
      photos TEXT NOT NULL,
      session_date TEXT NOT NULL DEFAULT '', tags TEXT NOT NULL DEFAULT '',
      ocr_text TEXT NOT NULL DEFAULT '', ocr_engine TEXT NOT NULL DEFAULT '',
      -- '' | requested | running | done | failed; `agents` holds the request, versions and discussion
      agent_status TEXT NOT NULL DEFAULT '', agents TEXT NOT NULL DEFAULT '',
      text TEXT NOT NULL DEFAULT '', note_id TEXT, error TEXT NOT NULL DEFAULT '');
    CREATE INDEX drafts_updated ON drafts(updated);
    CREATE TRIGGER drafts_ai AFTER INSERT ON drafts BEGIN
      UPDATE sync_counter SET n = n + 1;
      INSERT INTO sync_seq(tbl,id,seq) VALUES('drafts', NEW.id, (SELECT n FROM sync_counter))
        ON CONFLICT(tbl,id) DO UPDATE SET seq=excluded.seq;
    END;
    CREATE TRIGGER drafts_au AFTER UPDATE ON drafts BEGIN
      UPDATE sync_counter SET n = n + 1;
      INSERT INTO sync_seq(tbl,id,seq) VALUES('drafts', NEW.id, (SELECT n FROM sync_counter))
        ON CONFLICT(tbl,id) DO UPDATE SET seq=excluded.seq;
    END;
    ",
    // 5: answers from outside services kept for good (related-word lists from Gemini, and whatever comes next). Local to
    // each machine, never synced (no triggers, not in sync::TABLES): the sync server fills it once
    // and every device asks the server, so an update or a reinstall never loses what was paid for.
    "
    CREATE TABLE ref_cache(
      kind TEXT NOT NULL, key TEXT NOT NULL, json TEXT NOT NULL, fetched INTEGER NOT NULL,
      PRIMARY KEY(kind, key)) WITHOUT ROWID;
    ",
    // 6: the conversation with Claude about a photographed note (a JSON list of messages; synced with the draft).
    "
    ALTER TABLE drafts ADD COLUMN chat TEXT NOT NULL DEFAULT '[]';
    ",
    // 7: the general chat with Claude (one conversation, row "main"; messages and their ideas as JSON; synced).
    "
    CREATE TABLE talks(
      id TEXT PRIMARY KEY, created INTEGER NOT NULL, updated INTEGER NOT NULL, deleted INTEGER NOT NULL DEFAULT 0,
      -- '' | asked | running
      state TEXT NOT NULL DEFAULT '', msgs TEXT NOT NULL DEFAULT '[]');
    CREATE INDEX talks_updated ON talks(updated);
    CREATE TRIGGER talks_ai AFTER INSERT ON talks BEGIN
      UPDATE sync_counter SET n = n + 1;
      INSERT INTO sync_seq(tbl,id,seq) VALUES('talks', NEW.id, (SELECT n FROM sync_counter))
        ON CONFLICT(tbl,id) DO UPDATE SET seq=excluded.seq;
    END;
    CREATE TRIGGER talks_au AFTER UPDATE ON talks BEGIN
      UPDATE sync_counter SET n = n + 1;
      INSERT INTO sync_seq(tbl,id,seq) VALUES('talks', NEW.id, (SELECT n FROM sync_counter))
        ON CONFLICT(tbl,id) DO UPDATE SET seq=excluded.seq;
    END;
    ",
];

pub fn open(path: &Path) -> Result<Connection> {
    let existed = path.exists();
    let mut conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
    conn.pragma_update(None, "foreign_keys", true)?;
    migrate(&mut conn, path, existed)?;
    Ok(conn)
}

pub fn migrate(conn: &mut Connection, path: &Path, existed: bool) -> Result<()> {
    migrate_with(conn, path, existed, MIGRATIONS)
}

pub fn migrate_with(conn: &mut Connection, path: &Path, existed: bool, migrations: &[&str]) -> Result<()> {
    let current: usize = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let target = migrations.len();
    if current > target {
        bail!(
            "this database was written by a newer version of the app (schema {current}, this build knows {target}); \
             update the app instead of opening it with an older one"
        );
    }
    if current == target {
        return Ok(());
    }
    if existed && current > 0 {
        let mut backup = path.as_os_str().to_owned();
        backup.push(format!(".bak-v{current}"));
        std::fs::copy(path, &backup).context("backing up the database before migrating")?;
    }
    for (i, sql) in migrations.iter().enumerate().skip(current) {
        // Each step is atomic; a failed migration leaves the previous version intact.
        let tx = conn.transaction()?;
        tx.execute_batch(sql).with_context(|| format!("migration {} failed", i + 1))?;
        tx.pragma_update(None, "user_version", i + 1)?;
        tx.commit()?;
    }
    Ok(())
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("gredict-test-{}-{name}", new_id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn fresh_database_gets_the_latest_schema() {
        let p = tmp("fresh.sqlite");
        let c = open(&p).unwrap();
        let v: usize = c.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(v, MIGRATIONS.len());
        c.execute("INSERT INTO settings VALUES('a','b')", []).unwrap();
        let _ = std::fs::remove_file(&p);
    }

    /// Writes a database exactly as release 1 (schema v1) would have.
    fn release_1(p: &Path) {
        let mut c = Connection::open(p).unwrap();
        migrate_with(&mut c, p, false, &MIGRATIONS[..1]).unwrap();
        c.execute("INSERT INTO notes(id,created,updated,body) VALUES('n1',1,1,'abate = lessen')", []).unwrap();
        c.execute("INSERT INTO lookups(id,ts,key,word,source,device) VALUES('l1',5,'abate','abate','wiktionary','d')", []).unwrap();
        c.execute("INSERT INTO sessions(id,kind,title,spec,started,total) VALUES('s1','barrons','Unit 1',?,7,10)", ["{}"]).unwrap();
    }

    #[test]
    fn upgrading_from_release_1_keeps_every_row_and_leaves_a_backup() {
        let p = tmp("upgrade.sqlite");
        release_1(&p);
        let c = open(&p).unwrap();
        let v: usize = c.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(v, MIGRATIONS.len());
        let body: String = c.query_row("SELECT body FROM notes WHERE id='n1'", [], |r| r.get(0)).unwrap();
        assert_eq!(body, "abate = lessen");
        for (table, n) in [("lookups", 1), ("sessions", 1)] {
            let got: i64 = c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)).unwrap();
            assert_eq!(got, n, "{table}");
        }
        // The v2 full-text table exists and works.
        c.execute("INSERT INTO session_fts(session_id, body) VALUES ('s1','obdurate stubborn')", []).unwrap();
        let hit: String = c.query_row("SELECT session_id FROM session_fts WHERE session_fts MATCH 'stubborn'", [], |r| r.get(0)).unwrap();
        assert_eq!(hit, "s1");
        // The untouched release-1 file is kept beside it.
        let backup = std::path::PathBuf::from(format!("{}.bak-v1", p.display()));
        let b = Connection::open(&backup).unwrap();
        let bv: usize = b.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(bv, 1);
        assert!(b.prepare("SELECT * FROM session_fts").is_err(), "backup predates the new table");
        let _ = (std::fs::remove_file(&p), std::fs::remove_file(&backup));
    }

    #[test]
    fn a_future_release_can_alter_tables_without_losing_rows() {
        let p = tmp("future-alter.sqlite");
        release_1(&p);
        let mut v3: Vec<&str> = MIGRATIONS.to_vec();
        v3.push("ALTER TABLE notes ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0; CREATE TABLE extra(id TEXT PRIMARY KEY);");
        let mut c = Connection::open(&p).unwrap();
        migrate_with(&mut c, &p, true, &v3).unwrap();
        let (body, pinned): (String, i64) = c.query_row("SELECT body, pinned FROM notes WHERE id='n1'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((body.as_str(), pinned), ("abate = lessen", 0));
        let _ = (std::fs::remove_file(&p), std::fs::remove_file(format!("{}.bak-v1", p.display())));
    }

    #[test]
    fn failed_migration_rolls_back() {
        let p = tmp("fail.sqlite");
        let mut c = Connection::open(&p).unwrap();
        migrate_with(&mut c, &p, false, &[MIGRATIONS[0]]).unwrap();
        c.execute("INSERT INTO notes(id,created,updated,body) VALUES('n1',1,1,'x')", []).unwrap();
        let bad: Vec<&str> = vec![MIGRATIONS[0], "ALTER TABLE notes ADD COLUMN ok INTEGER; THIS IS NOT SQL;"];
        assert!(migrate_with(&mut c, &p, true, &bad).is_err());
        let v: usize = c.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(v, 1, "version must not advance");
        assert!(c.prepare("SELECT ok FROM notes").is_err(), "partial migration must be rolled back");
        let _ = (std::fs::remove_file(&p), std::fs::remove_file(format!("{}.bak-v1", p.display())));
    }

    #[test]
    fn refuses_a_database_from_the_future() {
        let p = tmp("future.sqlite");
        {
            let c = Connection::open(&p).unwrap();
            c.pragma_update(None, "user_version", MIGRATIONS.len() + 5).unwrap();
        }
        let err = open(&p).unwrap_err().to_string();
        assert!(err.contains("newer version"), "{err}");
        let _ = std::fs::remove_file(&p);
    }
}
