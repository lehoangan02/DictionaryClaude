//! Claude sign-in for other people. A paired device can keep its own Claude login on the server so its chats are
//! answered on that person's own subscription (the server owner's login is used for devices that keep none).
//!
//! Claude Code has no password to type: a subscription gives a long-lived token (`claude setup-token`, run once
//! on any computer where the person is signed in to Claude). The person pastes it in Settings → Claude account;
//! the server tries it once, keeps it in its `settings` table (never synced, never sent back to any device) and
//! passes it to `claude -p` as `CLAUDE_CODE_OAUTH_TOKEN`, with a config folder of its own so nothing of the
//! owner's login or history is shared. A token that is later refused sends the person back to Settings.

use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use crate::api::Api;
use crate::userdb::now_ms;

/// The device name as it is kept in setting keys and folder names.
fn slug(device: &str) -> String {
    device.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).take(64).collect()
}

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

impl Api {
    /// The token and config folder of this device's own Claude login on this server, if it has one.
    pub(crate) fn login_for(&self, device: &str) -> Option<(String, PathBuf)> {
        let d = slug(device);
        let token = self.setting(&format!("claude.token.{d}")).filter(|t| !t.is_empty())?;
        let dir = self.data_dir.join("claude-login").join(&d);
        std::fs::create_dir_all(&dir).ok()?;
        Some((token, dir))
    }

    /// Server side: does this device have its own login (never the token itself).
    pub(crate) fn claude_login_get(&self, device: &str) -> Result<Value> {
        let d = slug(device);
        let own = self.setting(&format!("claude.token.{d}")).is_some_and(|t| !t.is_empty());
        let since: Option<i64> = self.setting(&format!("claude.since.{d}")).and_then(|s| s.parse().ok());
        Ok(json!({"own": own, "since": since, "device": device}))
    }

    /// Server side: tries the token with one tiny call and keeps it only when Claude accepts it.
    pub(crate) fn claude_login_put(&self, device: &str, token: &str) -> Result<Value> {
        let token = clean_token(token)?;
        let d = slug(device);
        let dir = self.data_dir.join("claude-login").join(&d);
        std::fs::create_dir_all(&dir)?;
        let scratch = self.data_dir.join("jobs").join("login");
        std::fs::create_dir_all(&scratch)?;
        let agent = crate::jobs::Agent { name: "claude", bin: self.env_or("LEXIS_CLAUDE_BIN", "claude"), model: None, effort: None, extra: None, login: Some((token.clone(), dir)) };
        agent.check_login(&scratch).map_err(|e| {
            if is_login_failure(&format!("{e:#}")) {
                anyhow!("Claude did not accept that token. Run `claude setup-token` again and paste the new one")
            } else {
                anyhow!("could not check the token with Claude: {e:#}")
            }
        })?;
        self.set_setting(&format!("claude.token.{d}"), &token)?;
        self.set_setting(&format!("claude.since.{d}"), &now_ms().to_string())?;
        self.claude_login_get(device)
    }

    /// Server side: forgets the device's login (also when the device is removed). Its chats go back to the server's login.
    pub(crate) fn claude_login_drop(&self, device: &str) -> Result<Value> {
        let d = slug(device);
        self.db.lock().unwrap().execute("DELETE FROM settings WHERE k IN (?1, ?2)", [format!("claude.token.{d}"), format!("claude.since.{d}")])?;
        let _ = std::fs::remove_dir_all(self.data_dir.join("claude-login").join(&d));
        self.claude_login_get(device)
    }

    /// What the chat says when the person's own login is refused.
    pub(crate) fn login_hint(why: &str, own: bool) -> String {
        if own && is_login_failure(why) {
            "Your Claude login was not accepted (it may have expired or been revoked). Open Settings → Claude account, paste a new token from `claude setup-token`, then try again.".into()
        } else {
            why.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Both tests set process-wide environment variables.
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
    fn a_login_is_kept_only_when_claude_accepts_it_and_is_used_for_that_device_alone() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let api = Api::open_headless(&tmp("data")).unwrap();
        let bins = tmp("bins");
        // The stand-in accepts exactly one token and prints the token and config folder it was given to a file.
        let seen = bins.join("seen");
        let bin = script(&bins, &format!(r#"echo "$CLAUDE_CODE_OAUTH_TOKEN $CLAUDE_CONFIG_DIR" > {}
if [ "$CLAUDE_CODE_OAUTH_TOKEN" = sk-ant-oat01-goodgoodgoodgood ]; then echo '{{"result":"OK"}}'; else echo '{{"is_error":true,"result":"Failed to authenticate. API Error: 401 OAuth access token is invalid."}}'; exit 1; fi"#, seen.display()));
        std::env::set_var("LEXIS_CLAUDE_BIN", &bin);

        assert_eq!(api.claude_login_get("phone-ab12").unwrap()["own"], false);
        let err = api.claude_login_put("phone-ab12", "sk-ant-oat01-badbadbadbadbadbad").unwrap_err().to_string();
        assert!(err.contains("did not accept"), "{err}");
        assert!(!err.contains("badbad"), "the token must not appear in the message");
        assert_eq!(api.claude_login_get("phone-ab12").unwrap()["own"], false, "a refused token is not kept");

        let st = api.claude_login_put("phone-ab12", " sk-ant-oat01-goodgoodgoodgood ").unwrap();
        assert_eq!(st["own"], true);
        assert!(!st.to_string().contains("goodgood"), "the token is never sent back");
        let (token, dir) = api.login_for("phone-ab12").unwrap();
        assert_eq!(token, "sk-ant-oat01-goodgoodgoodgood");
        assert!(dir.ends_with("claude-login/phone-ab12"));
        assert!(api.login_for("laptop-cd34").is_none(), "another device keeps using the server's login");
        assert!(std::fs::read_to_string(&seen).unwrap().contains("claude-login/phone-ab12"));

        // The login is passed to the agent as environment variables, and removed with the device.
        let agent = crate::jobs::Agent { name: "claude", bin: bin.display().to_string(), model: None, effort: None, extra: None, login: api.login_for("phone-ab12") };
        agent.check_login(&bins).unwrap();
        api.claude_login_drop("phone-ab12").unwrap();
        assert!(api.login_for("phone-ab12").is_none());
        std::env::remove_var("LEXIS_CLAUDE_BIN");
    }

    // Chats written by a device with its own login run on that login, away from the app's source, and cannot release.
    #[test]
    fn a_chat_from_a_device_with_its_own_login_runs_on_it_in_a_scratch_folder() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let api = Api::open_headless(&tmp("chatdata")).unwrap();
        let bins = tmp("chatbins");
        let log = bins.join("log");
        let repo = tmp("chatrepo");
        std::fs::create_dir_all(repo.join("docs")).unwrap();
        std::fs::write(repo.join("docs/ROADMAP.md"), "x").unwrap();
        std::env::set_var("LEXIS_REPO", &repo);
        std::env::set_var("LEXIS_CLAUDE_BIN", script(&bins, &format!(r#"echo "$PWD|$CLAUDE_CODE_OAUTH_TOKEN" >> {}
echo '{{"structured_output":{{"reply":"Hello."}}}}'"#, log.display())));
        let me = crate::pairing::Pairing { v: 1, name: "phone-ab12".into(), lan: Some("127.0.0.1:1".into()), wan: None, token: "t".into(), cert: "c".into() };
        api.call("sync_pair", json!({"code": me.encode()})).unwrap();
        api.claude_login_put("phone-ab12", "sk-ant-oat01-goodgoodgoodgood").unwrap();

        let id = api.call("talk_new", json!({"mode": "work"})).unwrap()["id"].as_str().unwrap().to_string();
        let sent = api.call("talk_send", json!({"id": id, "message": "release it"})).unwrap();
        assert_eq!(sent["msgs"][0]["by"], "phone-ab12");
        assert_eq!(sent["state"], "asked", "a person with their own login cannot start a release");
        api.call("run_jobs", json!({})).unwrap();
        let t = api.call("talk_get", json!({"id": id})).unwrap();
        assert_eq!(t["msgs"][1]["text"], "Hello.");
        let used = std::fs::read_to_string(&log).unwrap();
        let last = used.lines().last().unwrap();
        assert!(last.ends_with("|sk-ant-oat01-goodgoodgoodgood"), "{last}");
        assert!(!last.starts_with(repo.to_str().unwrap()), "the app's source is not theirs to read: {last}");
        std::env::remove_var("LEXIS_REPO");
        std::env::remove_var("LEXIS_CLAUDE_BIN");
    }
}
