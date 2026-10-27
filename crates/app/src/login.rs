//! Who a device is on the sync server, and how a guest signs in to Claude.
//!
//! The owner proves who they are once, with the owner password (kept on the server, entered in Settings). That
//! device is then an *owner* device: it syncs the owner's notes, history and chats and chats on the server's own
//! Claude login. Every other paired device is a *guest*. The server keeps a private store for each guest (chats
//! only, `Api::open_guest`), never shows them the owner's data, and answers their chats on their own Claude
//! subscription, in a locked-down run with no access to the server's files or to the owner's login.
//!
//! A guest signs in to Claude by pressing a link (`claude auth login`, driven by the server) and pasting back the
//! code Claude shows, or by pasting the token that `claude setup-token` prints. The token or the sign-in lives only
//! in the guest's store on the server and is never sent back to any device or synced.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use crate::api::Api;
use crate::userdb::now_ms;

/// A pasted token, checked for shape only (the server checks the rest by trying it).
fn clean_token(raw: &str) -> Result<String> {
    let t = raw.trim();
    if t.is_empty() {
        bail!("paste the token that `claude setup-token` printed");
    }
    if t.starts_with("sk-ant-api") {
        bail!("that is an API key, which is billed per use. For your Claude subscription run `claude setup-token` and paste the token it prints");
    }
    if !t.starts_with("sk-ant-") || t.len() < 20 || t.len() > 600 || !t.chars().all(|c| c.is_ascii_graphic()) {
        bail!("that does not look like a Claude token (it starts with sk-ant- and has no spaces); run `claude setup-token` and paste what it prints");
    }
    Ok(t.to_string())
}

/// Whether an agent failure means the sign-in was refused (as opposed to a quota, a timeout or a bad reply).
pub(crate) fn is_login_failure(why: &str) -> bool {
    let w = why.to_lowercase();
    ["authenticate", "401", "oauth", "not logged in", "/login", "invalid api key", "unauthorized", "token has expired", "token is invalid"].iter().any(|k| w.contains(k))
}

/// The text without terminal escape sequences (colours, hyperlinks).
fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('[') => {
                for n in it.by_ref() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(n) = it.next() {
                    if n == '\u{7}' || (n == '\u{1b}' && it.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The sign-in link in what `claude auth login` printed.
fn find_link(out: &str) -> Option<String> {
    let t = strip_ansi(out);
    let i = t.find("https://")?;
    let url: String = t[i..].chars().take_while(|c| c.is_ascii_graphic()).collect();
    // The whole line must have arrived (it ends with the state parameter and then a newline).
    t[i + url.len()..].starts_with(['\r', '\n']).then_some(url)
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// A `claude auth login` waiting for the code, one per guest.
struct Link {
    child: Child,
    stdin: ChildStdin,
    seen: Arc<Mutex<String>>,
    started: Instant,
}

static LINKS: Mutex<Option<HashMap<PathBuf, Link>>> = Mutex::new(None);
const LINK_LIFE: Duration = Duration::from_secs(900);

fn kill(mut l: Link) {
    let _ = l.child.kill();
    let _ = l.child.wait();
}

impl Api {
    /// Server side, inside a guest's own store only.
    fn guest_only(&self) -> Result<()> {
        if !self.guest {
            bail!("a Claude sign-in belongs to a guest account on the sync server");
        }
        Ok(())
    }

    fn login_dir(&self) -> PathBuf {
        self.data_dir.join("claude-login")
    }

    /// The guest's own Claude sign-in: a token (empty when the sign-in is in the config folder) and that folder.
    pub(crate) fn login_for(&self) -> Option<(String, PathBuf)> {
        self.setting("claude.since")?;
        let token = match self.setting("claude.mode").as_deref() {
            Some("token") => self.setting("claude.token").filter(|t| !t.is_empty())?,
            Some("link") => String::new(),
            _ => return None,
        };
        let dir = self.login_dir();
        std::fs::create_dir_all(&dir).ok()?;
        Some((token, dir))
    }

    /// Does the guest have a Claude sign-in (never the token itself).
    pub(crate) fn claude_login_get(&self) -> Result<Value> {
        self.guest_only()?;
        let since: Option<i64> = self.setting("claude.since").and_then(|s| s.parse().ok());
        Ok(json!({"signed_in": self.login_for().is_some(), "since": since, "mode": self.setting("claude.mode")}))
    }

    /// Tries a token with one tiny call and keeps it only when Claude accepts it.
    pub(crate) fn claude_login_put(&self, token: &str) -> Result<Value> {
        self.guest_only()?;
        let token = clean_token(token)?;
        self.check_claude(Some((token.clone(), self.login_dir())))?;
        self.set_setting("claude.token", &token)?;
        self.keep_login("token")
    }

    fn keep_login(&self, mode: &str) -> Result<Value> {
        self.set_setting("claude.mode", mode)?;
        self.set_setting("claude.since", &now_ms().to_string())?;
        self.claude_login_get()
    }

    fn check_claude(&self, login: Option<(String, PathBuf)>) -> Result<()> {
        let dir = login.as_ref().map(|l| l.1.clone()).unwrap_or_default();
        std::fs::create_dir_all(&dir)?;
        let scratch = self.data_dir.join("jobs").join("login");
        std::fs::create_dir_all(&scratch)?;
        let agent = crate::jobs::Agent { name: "claude", bin: self.env_or("LEXIS_CLAUDE_BIN", "claude"), model: None, effort: None, extra: None, login };
        agent.check_login(&scratch).map_err(|e| {
            if is_login_failure(&format!("{e:#}")) {
                anyhow!("Claude did not accept that sign-in. Start again from Settings → Claude account")
            } else {
                anyhow!("could not check the sign-in with Claude: {e:#}")
            }
        })
    }

    /// Forgets the guest's sign-in (also when the device is removed): their chats stop until they sign in again.
    pub(crate) fn claude_login_drop(&self) -> Result<Value> {
        self.guest_only()?;
        self.db.lock().unwrap().execute("DELETE FROM settings WHERE k IN ('claude.token','claude.mode','claude.since')", [])?;
        if let Some(l) = LINKS.lock().unwrap().as_mut().and_then(|m| m.remove(&self.data_dir)) {
            kill(l);
        }
        let _ = std::fs::remove_dir_all(self.login_dir());
        self.claude_login_get()
    }

    /// Starts `claude auth login` (in a terminal of its own, which the CLI insists on) and returns the link the
    /// guest must open; the code Claude shows there comes back through `claude_link_end`.
    pub(crate) fn claude_link_begin(&self) -> Result<Value> {
        self.guest_only()?;
        let dir = self.login_dir();
        std::fs::create_dir_all(&dir)?;
        let cmd = format!("{} auth login --claudeai", shell_quote(&self.env_or("LEXIS_CLAUDE_BIN", "claude")));
        let mut child = Command::new("script")
            .args(["-qfec", &cmd, "/dev/null"])
            .env("CLAUDE_CONFIG_DIR", &dir)
            .env("BROWSER", "true")
            .env_remove("CLAUDE_CODE_OAUTH_TOKEN")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("ANTHROPIC_AUTH_TOKEN")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| anyhow!("cannot start the Claude sign-in on this server ({e})"))?;
        let (stdin, mut stdout) = (child.stdin.take().unwrap(), child.stdout.take().unwrap());
        let seen = Arc::new(Mutex::new(String::new()));
        let reader = seen.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 {
                    break;
                }
                reader.lock().unwrap().push_str(&String::from_utf8_lossy(&buf[..n]));
            }
        });
        let began = Instant::now();
        let link = loop {
            if let Some(u) = find_link(&seen.lock().unwrap()) {
                break Some(u);
            }
            if began.elapsed() > Duration::from_secs(25) || child.try_wait().ok().flatten().is_some() {
                break None;
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        let l = Link { child, stdin, seen: seen.clone(), started: Instant::now() };
        let Some(url) = link else {
            let why = strip_ansi(&seen.lock().unwrap()).lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no output").trim().to_string();
            kill(l);
            bail!("Claude did not give a sign-in link ({why})");
        };
        let mut map = LINKS.lock().unwrap();
        let map = map.get_or_insert_with(HashMap::new);
        for (_, old) in map.extract_if(|_, l| l.started.elapsed() > LINK_LIFE) {
            kill(old);
        }
        if let Some(old) = map.insert(self.data_dir.clone(), l) {
            kill(old);
        }
        Ok(json!({"url": url}))
    }

    /// Gives `claude auth login` the code the guest pasted and keeps the sign-in if Claude accepts it.
    pub(crate) fn claude_link_end(&self, code: &str) -> Result<Value> {
        self.guest_only()?;
        let code = code.trim();
        if code.is_empty() || code.len() > 2000 || !code.chars().all(|c| c.is_ascii_graphic()) {
            bail!("paste the code that Claude shows after you sign in (no spaces)");
        }
        let Some(mut l) = LINKS.lock().unwrap().as_mut().and_then(|m| m.remove(&self.data_dir)).filter(|l| l.started.elapsed() <= LINK_LIFE) else {
            bail!("that sign-in link has expired; press Sign in with Claude again to get a new one");
        };
        let sent = l.stdin.write_all(code.as_bytes()).and_then(|_| l.stdin.flush()).and_then(|_| {
            std::thread::sleep(Duration::from_millis(400));
            l.stdin.write_all(b"\r")
        });
        let waited = Instant::now();
        while sent.is_ok() && waited.elapsed() < Duration::from_secs(45) && l.child.try_wait().ok().flatten().is_none() {
            std::thread::sleep(Duration::from_millis(150));
        }
        let said = strip_ansi(&l.seen.lock().unwrap());
        kill(l);
        if sent.is_err() {
            bail!("the sign-in link has expired; press Sign in with Claude again to get a new one");
        }
        self.check_claude(Some((String::new(), self.login_dir()))).map_err(|e| {
            let hint = said.lines().rev().find(|x| x.to_lowercase().contains("error") || x.to_lowercase().contains("invalid")).unwrap_or("").trim().to_string();
            anyhow!("{e:#}{}", if hint.is_empty() { String::new() } else { format!(" ({hint})") })
        })?;
        self.keep_login("link")
    }

    /// What the chat says when the guest's own sign-in is refused.
    pub(crate) fn login_hint(why: &str, own: bool) -> String {
        if own && is_login_failure(why) {
            "Your Claude sign-in was not accepted (it may have expired or been revoked). Open Settings → Claude account, sign in again, then try again.".into()
        } else {
            why.to_string()
        }
    }

    /// Device side: records the role the server gave this device. A change of role means a different store on the
    /// server, so the next sync starts from the beginning (rows merge by timestamp, so nothing is lost or doubled).
    pub(crate) fn note_role(&self, role: &str) {
        let before = self.setting("sync.role");
        if before.as_deref() == Some(role) {
            return;
        }
        if before.is_some() || role == "guest" {
            let _ = self.set_setting("sync.cursor", "0");
            let _ = self.set_setting("sync.pushed", "0");
            self.dirty.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let _ = self.set_setting("sync.role", role);
    }

    /// Device side: a guest has no work chats, merges or releases.
    pub(crate) fn is_guest_device(&self) -> bool {
        self.setting("sync.role").as_deref() == Some("guest")
    }

    /// Device side: what the server knows this device as.
    pub(crate) fn owner_status(&self) -> Result<Value> {
        let r = self.server_post("/v1/me", json!({}))?;
        if let Some(role) = r["role"].as_str() {
            self.note_role(role);
        }
        Ok(r)
    }

    /// Device side: proves to the server that this is the owner's device (the password is sent once and not kept here).
    pub(crate) fn owner_login(&self, password: &str) -> Result<Value> {
        let r = self.server_post("/v1/owner/login", json!({"password": password}))?;
        if let Some(role) = r["role"].as_str() {
            self.note_role(role);
        }
        Ok(r)
    }

    /// Device side: sets or changes the owner password (only an owner device may).
    pub(crate) fn owner_set(&self, password: &str, old: &str) -> Result<Value> {
        self.server_post("/v1/owner/set", json!({"password": password, "old": old}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// The tests set process-wide environment variables.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("gredict-login-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn script(dir: &PathBuf, body: &str) -> PathBuf {
        let p = dir.join("claude");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn tokens_are_checked_for_shape() {
        assert!(clean_token("  sk-ant-oat01-abcdefghijklmnop \n").is_ok());
        assert!(clean_token("").is_err());
        assert!(clean_token("hunter2").is_err());
        assert!(clean_token("sk-ant-oat01-abc def ghijklmnop").is_err());
        assert!(clean_token("sk-ant-api03-abcdefghijklmnop").unwrap_err().to_string().contains("API key"));
    }

    #[test]
    fn refused_logins_are_recognised() {
        assert!(is_login_failure("claude says: Failed to authenticate. API Error: 401 OAuth access token is invalid."));
        assert!(!is_login_failure("claude says: usage limit reached"));
        assert!(Api::login_hint("Failed to authenticate", true).contains("Settings"));
        assert_eq!(Api::login_hint("Failed to authenticate", false), "Failed to authenticate");
    }

    #[test]
    fn the_sign_in_link_is_found_in_what_the_terminal_printed() {
        // As `claude auth login` prints it: a hyperlink escape, the coloured visible link, then the prompt.
        let out = "Opening browser to sign in\u{2026}\r\nIf the browser didn't open, visit: \u{1b}]8;;https://claude.com/cai/oauth/authorize?code=true&state=abc\u{7}\u{1b}[94mhttps://claude.com/cai/oauth/authorize?code=true&state=abc\u{1b}[39m\u{1b}]8;;\u{7}\r\nPaste code here if prompted > ";
        assert_eq!(find_link(out).unwrap(), "https://claude.com/cai/oauth/authorize?code=true&state=abc");
        assert!(find_link("If the browser didn't open, visit: https://claude.com/cai/oau").is_none(), "a half-printed link is not a link");
        assert_eq!(strip_ansi("\u{1b}[1mhi\u{1b}[0m"), "hi");
    }

    #[test]
    fn a_login_is_kept_only_when_claude_accepts_it_and_dies_with_the_guest() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let api = Api::open_guest(&tmp("data")).unwrap();
        let bins = tmp("bins");
        // The stand-in accepts exactly one token and writes the token and config folder it was given to a file.
        let seen = bins.join("seen");
        let bin = script(&bins, &format!(r#"echo "$CLAUDE_CODE_OAUTH_TOKEN $CLAUDE_CONFIG_DIR" > {}
if [ "$CLAUDE_CODE_OAUTH_TOKEN" = sk-ant-oat01-goodgoodgoodgood ]; then echo '{{"result":"OK"}}'; else echo '{{"is_error":true,"result":"Failed to authenticate. API Error: 401 OAuth access token is invalid."}}'; exit 1; fi"#, seen.display()));
        std::env::set_var("LEXIS_CLAUDE_BIN", &bin);

        assert_eq!(api.claude_login_get().unwrap()["signed_in"], false);
        let err = api.claude_login_put("sk-ant-oat01-badbadbadbadbadbad").unwrap_err().to_string();
        assert!(err.contains("did not accept"), "{err}");
        assert!(!err.contains("badbad"), "the token must not appear in the message");
        assert_eq!(api.claude_login_get().unwrap()["signed_in"], false, "a refused token is not kept");

        let st = api.claude_login_put(" sk-ant-oat01-goodgoodgoodgood ").unwrap();
        assert_eq!(st["signed_in"], true);
        assert!(!st.to_string().contains("goodgood"), "the token is never sent back");
        let (token, dir) = api.login_for().unwrap();
        assert_eq!(token, "sk-ant-oat01-goodgoodgoodgood");
        assert!(dir.ends_with("claude-login"));
        assert!(std::fs::read_to_string(&seen).unwrap().contains("claude-login"));

        api.claude_login_drop().unwrap();
        assert!(api.login_for().is_none());
        std::env::remove_var("LEXIS_CLAUDE_BIN");
        // Only a guest's store has a sign-in; the owner's server uses its own login.
        assert!(Api::open_headless(&tmp("owner")).unwrap().claude_login_get().is_err());
    }

    #[test]
    fn a_guest_signs_in_by_a_link_and_a_pasted_code() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        if Command::new("script").arg("--version").output().is_err() {
            return; // util-linux `script` is how the CLI gets the terminal it insists on
        }
        let api = Api::open_guest(&tmp("linkdata")).unwrap();
        let bins = tmp("linkbins");
        // `auth login` prints a link and waits for the code; the sign-in it leaves is a file in the config folder,
        // which a later `-p` run accepts only when it is there (and no token is passed in).
        let bin = script(&bins, r#"if [ "$1" = auth ]; then
  printf 'Opening browser to sign in\nIf the browser didn'"'"'t open, visit: https://claude.example/oauth/authorize?state=xyz\nPaste code here if prompted > '
  read code
  if [ "$code" = goodcode ]; then echo signed-in > "$CLAUDE_CONFIG_DIR/credentials"; exit 0; fi
  echo "Error: invalid code"; exit 1
fi
if [ -f "$CLAUDE_CONFIG_DIR/credentials" ] && [ -z "$CLAUDE_CODE_OAUTH_TOKEN" ]; then echo '{"result":"OK"}'; else echo '{"is_error":true,"result":"Failed to authenticate. API Error: 401"}'; exit 1; fi"#);
        std::env::set_var("LEXIS_CLAUDE_BIN", &bin);

        assert!(api.claude_link_end("goodcode").unwrap_err().to_string().contains("expired"), "no link was started");
        let url = api.claude_link_begin().unwrap()["url"].as_str().unwrap().to_string();
        assert_eq!(url, "https://claude.example/oauth/authorize?state=xyz");
        let err = api.claude_link_end("wrongcode").unwrap_err().to_string();
        assert!(err.contains("did not accept"), "{err}");
        assert_eq!(api.claude_login_get().unwrap()["signed_in"], false);

        api.claude_link_begin().unwrap();
        let st = api.claude_link_end("goodcode").unwrap();
        assert_eq!(st["signed_in"], true);
        assert_eq!(st["mode"], "link");
        let (token, dir) = api.login_for().unwrap();
        assert!(token.is_empty(), "the sign-in is in the config folder, not in a token");
        assert!(dir.join("credentials").is_file());
        std::env::remove_var("LEXIS_CLAUDE_BIN");
    }

    // A guest's chat runs on their own sign-in, in an empty folder, reading nothing but its photos, and never as work.
    #[test]
    fn a_guests_chat_runs_on_their_sign_in_in_an_empty_folder() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let api = Api::open_guest(&tmp("chatdata")).unwrap();
        let bins = tmp("chatbins");
        let log = bins.join("log");
        let repo = tmp("chatrepo");
        std::fs::create_dir_all(repo.join("docs")).unwrap();
        std::fs::write(repo.join("docs/ROADMAP.md"), "x").unwrap();
        std::env::set_var("LEXIS_REPO", &repo);
        std::env::set_var("LEXIS_CLAUDE_BIN", script(&bins, &format!(r#"ask="$2"; shift 2
echo "$PWD|$CLAUDE_CODE_OAUTH_TOKEN|$*" >> {}
if echo "$ask" | grep -q 'single word OK'; then echo '{{"result":"OK"}}'; else echo '{{"structured_output":{{"reply":"Hello."}}}}'; fi"#, log.display())));

        // Not signed in: no answer, and above all not on the server owner's subscription.
        let id = api.call("talk_new", json!({"mode": "work"})).unwrap()["id"].as_str().unwrap().to_string();
        api.call("talk_send", json!({"id": id, "message": "hi"})).unwrap();
        api.call("run_jobs", json!({})).unwrap();
        let t = api.call("talk_get", json!({"id": id})).unwrap();
        assert!(t["msgs"][1]["text"].as_str().unwrap().contains("Sign in with your Claude account"), "{t}");
        assert!(!log.exists(), "Claude must not run without the guest's own sign-in");

        api.claude_login_put("sk-ant-oat01-goodgoodgoodgood").unwrap();
        api.call("talk_send", json!({"id": id, "message": "hi again"})).unwrap();
        api.call("run_jobs", json!({})).unwrap();
        let t = api.call("talk_get", json!({"id": id})).unwrap();
        assert_eq!(t["msgs"].as_array().unwrap().last().unwrap()["text"], "Hello.");
        let used = std::fs::read_to_string(&log).unwrap();
        let last = used.lines().last().unwrap();
        assert!(last.contains("|sk-ant-oat01-goodgoodgoodgood|"), "{last}");
        assert!(!last.starts_with(repo.to_str().unwrap()), "the app's source is not theirs to read: {last}");
        assert!(last.contains("--tools Read") && last.contains("--allowedTools Read(//"), "reads are confined: {last}");
        assert!(!last.contains("--permission-mode"), "{last}");

        // On a guest's own device "release it" is just a question...
        let before = used.lines().count();
        api.set_setting("sync.role", "guest").unwrap();
        api.call("talk_send", json!({"id": id, "message": "release it"})).unwrap();
        api.call("run_jobs", json!({})).unwrap();
        assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), before + 1, "it was answered as a plain question");
        assert!(api.call("talk_finish", json!({"id": id, "how": "merge"})).is_err());
        // ...and a chat that a device claims to be in a release state is never acted on by the guest's store.
        api.db.lock().unwrap().execute("UPDATE talks SET state='release_patch' WHERE id=?1", [&id]).unwrap();
        api.call("run_jobs", json!({})).unwrap();
        assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), before + 1);
        assert_eq!(api.call("talk_get", json!({"id": id})).unwrap()["state"], "release_patch");
        std::env::remove_var("LEXIS_REPO");
        std::env::remove_var("LEXIS_CLAUDE_BIN");
    }
}
