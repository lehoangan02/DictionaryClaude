//! The sync server: TLS with a pinned self-signed certificate, bearer tokens,
//! and a small JSON API in front of the same merge logic the apps use.
//!
//! Devices pair with a code that carries the server addresses, a token and the
//! certificate to trust, so no certificate authority is involved.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use gredict_app::Api;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tiny_http::{Header, Method, Request, Response, Server, SslConfig, StatusCode};

pub const MAX_BODY: u64 = 32 << 20;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Token {
    pub name: String,
    pub hash: String,
}

pub use gredict_app::pairing::Pairing;

pub struct Tls {
    pub cert_pem: String,
    pub key_pem: String,
}

/// Loads the server certificate, creating it on first run. `sans` must include
/// every name or IP clients connect to (DuckDNS name, LAN address, localhost).
pub fn ensure_tls(dir: &Path, sans: &[String]) -> Result<Tls> {
    let (c, k) = (dir.join("cert.pem"), dir.join("key.pem"));
    if c.exists() && k.exists() {
        return Ok(Tls { cert_pem: fs::read_to_string(c)?, key_pem: fs::read_to_string(k)? });
    }
    fs::create_dir_all(dir)?;
    let ck = rcgen::generate_simple_self_signed(sans.to_vec())?;
    let (cert_pem, key_pem) = (ck.cert.pem(), ck.key_pair.serialize_pem());
    fs::write(&c, &cert_pem)?;
    fs::write(&k, &key_pem)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&k, fs::Permissions::from_mode(0o600))?;
    }
    Ok(Tls { cert_pem, key_pem })
}

fn hash(token: &str) -> String {
    hash_bytes(token.as_bytes())
}

fn hash_bytes(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|b| format!("{b:02x}")).collect()
}

/// Constant-time comparison of two hex digests.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

pub struct Tokens {
    path: PathBuf,
}

impl Tokens {
    pub fn new(dir: &Path) -> Self {
        Tokens { path: dir.join("tokens.json") }
    }
    pub fn list(&self) -> Vec<Token> {
        fs::read(&self.path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    /// Issues a token for a new device; the secret is only ever shown once.
    pub fn issue(&self, name: &str) -> Result<String> {
        let secret = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        let mut all: Vec<Token> = self.list().into_iter().filter(|t| t.name != name).collect();
        all.push(Token { name: name.to_string(), hash: hash(&secret) });
        fs::write(&self.path, serde_json::to_vec_pretty(&all)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(secret)
    }
    pub fn revoke(&self, name: &str) -> Result<bool> {
        let all = self.list();
        let kept: Vec<Token> = all.iter().filter(|t| t.name != name).cloned().collect();
        fs::write(&self.path, serde_json::to_vec_pretty(&kept)?)?;
        Ok(kept.len() != all.len())
    }
    pub fn check(&self, presented: &str) -> Option<String> {
        let h = hash(presented);
        self.list().into_iter().find(|t| same(&t.hash, &h)).map(|t| t.name)
    }
}

fn reply(req: Request, status: u16, body: Value) {
    let h = Header::from_bytes("Content-Type", "application/json").unwrap();
    let _ = req.respond(Response::from_string(body.to_string()).with_status_code(StatusCode(status)).with_header(h));
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

pub fn handle(api: &Api, tokens: &Tokens, private: &Path, mut req: Request) {
    let path = req.url().split('?').next().unwrap_or("/").to_string();
    if path == "/v1/health" {
        return reply(req, 200, json!({"ok": true, "name": "lexis-syncd", "version": env!("CARGO_PKG_VERSION"), "time": now_ms()}));
    }
    let who = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("Authorization"))
        .and_then(|h| h.value.as_str().strip_prefix("Bearer ").map(str::to_owned))
        .and_then(|t| tokens.check(&t));
    let Some(who) = who else { return reply(req, 401, json!({"error": "unauthorized"})) };
    if *req.method() != Method::Post {
        return reply(req, 405, json!({"error": "POST only"}));
    }
    let mut body = Vec::new();
    {
        use std::io::Read;
        if req.as_reader().take(MAX_BODY + 1).read_to_end(&mut body).is_err() || body.len() as u64 > MAX_BODY {
            return reply(req, 413, json!({"error": "request too large"}));
        }
    }
    if path == "/v1/photo/put" {
        let name = req.headers().iter().find(|h| h.field.equiv("X-Photo")).map(|h| h.value.as_str().to_string()).unwrap_or_default();
        return match api.photo_store(&name, &body) {
            Ok(()) => reply(req, 200, json!({"ok": true})),
            Err(e) => reply(req, 400, json!({"error": format!("{e:#}")})),
        };
    }
    let args: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    if path == "/v1/photo/get" {
        let bytes = args.get("name").and_then(Value::as_str).and_then(|n| api.photo_read(n));
        return match bytes {
            Some(b) => {
                let h = Header::from_bytes("Content-Type", "application/octet-stream").unwrap();
                let _ = req.respond(Response::from_data(b).with_header(h));
            }
            None => reply(req, 404, json!({"error": "no such photo"})),
        };
    }
    if path == "/v1/blob/barrons" {
        // The parsed book (never the PDF) goes to the owner's paired devices.
        let Ok(bytes) = fs::read(private.join("barrons.sqlite")) else { return reply(req, 404, json!({"error": "no book on this server"})) };
        let sum = hash_bytes(&bytes);
        if args.get("have").and_then(Value::as_str) == Some(sum.as_str()) {
            return reply(req, 200, json!({"unchanged": true}));
        }
        let h1 = Header::from_bytes("Content-Type", "application/octet-stream").unwrap();
        let h2 = Header::from_bytes("X-Sha256", sum.as_bytes()).unwrap();
        let _ = req.respond(Response::from_data(bytes).with_header(h1).with_header(h2));
        return;
    }
    let out = match path.as_str() {
        "/v1/hello" => Ok(json!({"device": who, "time": now_ms()})),
        "/v1/push" => api.call("sync_apply", json!({"changes": args.get("changes").cloned().unwrap_or(Value::Null)})),
        "/v1/pull" => api.call("sync_pull", json!({"cursor": args.get("cursor").and_then(Value::as_i64).unwrap_or(0), "limit": args.get("limit").and_then(Value::as_i64).unwrap_or(2000)})),
        _ => return reply(req, 404, json!({"error": "not found"})),
    };
    match out {
        Ok(v) => reply(req, 200, v),
        Err(e) => reply(req, 400, json!({"error": format!("{e:#}")})),
    }
}

/// Starts one TLS listener per address and serves until the process exits.
pub fn serve(api: Arc<Api>, tokens: Arc<Tokens>, private: PathBuf, tls: &Tls, addrs: &[String]) -> Result<Vec<std::thread::JoinHandle<()>>> {
    let mut handles = Vec::new();
    for addr in addrs {
        let server = Server::https(addr.as_str(), SslConfig { certificate: tls.cert_pem.clone().into_bytes(), private_key: tls.key_pem.clone().into_bytes() })
            .map_err(|e| anyhow!("{e}"))
            .with_context(|| format!("listening on {addr}"))?;
        let (api, tokens, private) = (api.clone(), tokens.clone(), private.clone());
        handles.push(std::thread::spawn(move || {
            for req in server.incoming_requests() {
                let (api, tokens, private) = (api.clone(), tokens.clone(), private.clone());
                std::thread::spawn(move || handle(&api, &tokens, &private, req));
            }
        }));
    }
    Ok(handles)
}
