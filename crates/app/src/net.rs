//! Talking to the sync server: choosing the route (home network or remote),
//! pinned-certificate TLS, push/pull, and the status the UI shows.
//!
//! Route rules, set by the owner: on the home Wi-Fi (default `Anan`, `Anan 5g`)
//! use the LAN port; everywhere else use the remote port. The Wi-Fi name is
//! only a hint: it can be unreadable (permissions) or misleading (a VPN may
//! carry LAN traffic away), so every route is *verified* by a handshake that
//! checks the pinned certificate and the token before it is trusted.

use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use serde_json::{json, Value};
use ureq::tls::{Certificate, RootCerts, TlsConfig};

use crate::api::Api;
use crate::pairing::Pairing;
use crate::userdb::now_ms;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Route {
    Lan,
    Wan,
}

impl Route {
    fn name(self) -> &'static str {
        match self {
            Route::Lan => "lan",
            Route::Wan => "wan",
        }
    }
}

struct Probe {
    route: Route,
    ok: bool,
    ms: u128,
    error: Option<String>,
}

fn agent(cert_pem: &str, timeout: Duration) -> Result<ureq::Agent> {
    let cert = Certificate::from_pem(cert_pem.as_bytes()).map_err(|e| anyhow!("bad server certificate: {e}"))?;
    let tls = TlsConfig::builder().root_certs(RootCerts::new_with_certs(&[cert])).build();
    Ok(ureq::Agent::config_builder().tls_config(tls).timeout_global(Some(timeout)).http_status_as_error(false).build().into())
}

fn post(agent: &ureq::Agent, host: &str, token: &str, path: &str, body: &Value) -> Result<Value> {
    let mut resp = agent
        .post(&format!("https://{host}{path}"))
        .header("Authorization", &format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .send(serde_json::to_vec(body)?)?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string()?;
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status != 200 {
        bail!("{}", v["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("server answered {status}")));
    }
    Ok(v)
}

fn verify(p: &Pairing, route: Route, timeout: Duration) -> Probe {
    let started = Instant::now();
    let host = match route {
        Route::Lan => p.lan.as_deref(),
        Route::Wan => p.wan.as_deref(),
    };
    let result = host
        .ok_or_else(|| anyhow!("no {} address", route.name()))
        .and_then(|h| Ok((h, agent(&p.cert, timeout)?)))
        .and_then(|(h, a)| post(&a, h, &p.token, "/v1/hello", &json!({})));
    Probe { route, ok: result.is_ok(), ms: started.elapsed().as_millis(), error: result.err().map(|e| format!("{e:#}")) }
}

/// Best-effort Wi-Fi name; `None` when unknown or not permitted.
fn wifi_name() -> Option<String> {
    // Test/debug hook: pretend to be on this network (empty = unknown).
    if let Ok(v) = std::env::var("LEXIS_WIFI") {
        return Some(v).filter(|v| !v.is_empty());
    }
    let run = |cmd: &str, args: &[&str]| Command::new(cmd).args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    if cfg!(target_os = "macos") {
        let out = run("networksetup", &["-getairportnetwork", "en0"])?;
        return out.strip_prefix("Current Wi-Fi Network: ").map(str::to_string);
    }
    if let Some(s) = run("iwgetid", &["-r"]).filter(|s| !s.is_empty()) {
        return Some(s);
    }
    run("nmcli", &["-t", "-f", "active,ssid", "dev", "wifi"])?.lines().find_map(|l| l.strip_prefix("yes:").map(str::to_string))
}

/// Whether a VPN looks active (tunnel interfaces on Linux, `scutil` on macOS).
fn vpn_active() -> bool {
    if cfg!(target_os = "macos") {
        return Command::new("scutil").args(["--nc", "list"]).output().map_or(false, |o| String::from_utf8_lossy(&o.stdout).contains("(Connected)"));
    }
    std::fs::read_dir("/sys/class/net").map_or(false, |d| {
        d.flatten().any(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            let tunnel = ["tun", "wg", "ppp", "tailscale", "zt", "ipsec", "vpn"].iter().any(|p| n.starts_with(p));
            tunnel && std::fs::read_to_string(e.path().join("operstate")).map_or(true, |s| s.trim() != "down")
        })
    })
}

fn sha_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

impl Api {
    /// Downloads the parsed Barron's book from the server when this device
    /// lacks it or has an older copy. The PDF can only be parsed on the
    /// machine that has `pdftotext`; every other device receives the result.
    fn fetch_barrons(&self, a: &ureq::Agent, host: &str, token: &str) -> Result<bool> {
        let dir = self.private_dir.clone().ok_or_else(|| anyhow!("no private directory"))?;
        let path = dir.join("barrons.sqlite");
        let have = std::fs::read(&path).map(|b| sha_hex(&b)).unwrap_or_default();
        let mut resp = a
            .post(&format!("https://{host}/v1/blob/barrons"))
            .header("Authorization", &format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .send(serde_json::to_vec(&json!({"have": have}))?)?;
        match resp.status().as_u16() {
            200 => {}
            404 => return Ok(false), // the server has no book to share
            s => bail!("server answered {s}"),
        }
        if resp.headers().get("Content-Type").and_then(|v| v.to_str().ok()).is_some_and(|t| t.contains("json")) {
            return Ok(false); // {"unchanged": true}
        }
        let want = resp.headers().get("X-Sha256").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let bytes = resp.body_mut().with_config().limit(256 << 20).read_to_vec()?;
        if !want.is_empty() && sha_hex(&bytes) != want {
            bail!("the downloaded book is damaged (checksum mismatch)");
        }
        std::fs::create_dir_all(&dir)?;
        let tmp = dir.join("barrons.sqlite.part");
        std::fs::write(&tmp, &bytes)?;
        // Only replace the working copy with something that opens as a book.
        crate::barrons::Barrons::open(&tmp).context("the downloaded book does not open")?;
        std::fs::rename(&tmp, &path)?;
        self.reload_barrons()?;
        Ok(true)
    }

    /// Sends photos the server has not received (before the rows that name them).
    fn upload_photos(&self, a: &ureq::Agent, host: &str, token: &str) -> usize {
        let mut sent = 0;
        for name in self.photos_to_upload() {
            let Some(bytes) = self.photo_read(&name) else { continue };
            let ok = a
                .post(&format!("https://{host}/v1/photo/put"))
                .header("Authorization", &format!("Bearer {token}"))
                .header("X-Photo", &name)
                .header("Content-Type", "application/octet-stream")
                .send(bytes)
                .map(|r| r.status().as_u16() == 200)
                .unwrap_or(false);
            if ok {
                self.photo_mark_sent(&name);
                sent += 1;
            }
        }
        sent
    }

    /// A photo taken on another device: ask the server for it and keep a copy.
    pub(crate) fn photo_fetch_from_server(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let p = self.pairing().ok_or_else(|| anyhow!("not paired"))?;
        let (route, ..) = self.choose_route(&p);
        let host = match route {
            Some(Route::Lan) => p.lan.clone(),
            Some(Route::Wan) => p.wan.clone(),
            None => bail!("the sync server is not reachable"),
        }
        .unwrap();
        let a = agent(&p.cert, Duration::from_secs(60))?;
        let mut resp = a
            .post(&format!("https://{host}/v1/photo/get"))
            .header("Authorization", &format!("Bearer {}", p.token))
            .header("Content-Type", "application/json")
            .send(serde_json::to_vec(&json!({"name": name}))?)?;
        if resp.status().as_u16() != 200 {
            return Ok(None);
        }
        let bytes = resp.body_mut().with_config().limit(12 << 20).read_to_vec()?;
        self.photo_store(name, &bytes)?;
        self.photo_mark_sent(name); // it came from the server, so the server has it
        Ok(Some(bytes))
    }

    pub(crate) fn setting(&self, k: &str) -> Option<String> {
        self.db.lock().unwrap().query_row("SELECT v FROM settings WHERE k=?", [k], |r| r.get(0)).ok()
    }

    pub(crate) fn set_setting(&self, k: &str, v: &str) -> Result<()> {
        self.db.lock().unwrap().execute("INSERT INTO settings VALUES(?1,?2) ON CONFLICT(k) DO UPDATE SET v=excluded.v", [k, v])?;
        Ok(())
    }

    fn pairing(&self) -> Option<Pairing> {
        serde_json::from_str(&self.setting("sync.pairing")?).ok()
    }

    pub(crate) fn sync_pair(&self, code: &str) -> Result<Value> {
        let p = Pairing::decode(code)?;
        self.set_setting("sync.pairing", &serde_json::to_string(&p)?)?;
        self.set_setting("sync.cursor", "0")?;
        self.set_setting("sync.pushed", "0")?;
        Ok(json!({"paired": true, "name": p.name, "lan": p.lan, "wan": p.wan}))
    }

    pub(crate) fn sync_unpair(&self) -> Result<Value> {
        self.db.lock().unwrap().execute("DELETE FROM settings WHERE k LIKE 'sync.%'", [])?;
        Ok(json!({"paired": false}))
    }

    /// Verifies the routes the rules allow, concurrently, and picks one.
    fn choose_route(&self, p: &Pairing) -> (Option<Route>, Vec<Probe>, Option<String>, bool) {
        let ssid = wifi_name();
        let vpn = vpn_active();
        let homes: Vec<String> = self.setting("sync.home_ssids").unwrap_or_else(|| "Anan,Anan 5g".into()).split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect();
        let at_home = ssid.as_ref().map(|s| homes.contains(&s.to_lowercase()));
        // Known to be away from home and no VPN that could tunnel LAN traffic:
        // 192.168.x.x would be somebody else's network, so do not even try it.
        let try_lan = p.lan.is_some() && (at_home != Some(false) || vpn);
        let try_wan = p.wan.is_some();
        let (tx, rx) = mpsc::channel();
        let mut n = 0;
        for (route, wanted) in [(Route::Lan, try_lan), (Route::Wan, try_wan)] {
            if wanted {
                n += 1;
                let (p, tx) = (p.clone(), tx.clone());
                std::thread::spawn(move || {
                    let _ = tx.send(verify(&p, route, Duration::from_millis(if route == Route::Lan { 2500 } else { 6000 })));
                });
            }
        }
        drop(tx);
        let mut probes: Vec<Probe> = rx.iter().take(n).collect();
        probes.sort_by_key(|p| p.route == Route::Wan);
        // At home (or unknown) the LAN wins when it verifies; otherwise remote.
        let pick = if at_home == Some(false) && !vpn {
            probes.iter().find(|p| p.ok && p.route == Route::Wan)
        } else {
            probes.iter().find(|p| p.ok)
        };
        (pick.map(|p| p.route), probes, ssid, vpn)
    }

    /// Status for the header pill and the sync settings.
    pub(crate) fn server_status(&self, probe: bool) -> Result<Value> {
        let Some(p) = self.pairing() else { return Ok(json!({"paired": false})) };
        let pending = self.pending_changes()?;
        let last = self.setting("sync.last").and_then(|s| serde_json::from_str::<Value>(&s).ok());
        let mut out = json!({"paired": true, "name": p.name, "pending": pending, "last_sync": last, "lan": p.lan, "wan": p.wan});
        if probe {
            let (route, probes, ssid, vpn) = self.choose_route(&p);
            out["online"] = json!(route.is_some());
            out["route"] = json!(route.map(Route::name));
            out["latency_ms"] = json!(probes.iter().find(|x| Some(x.route) == route).map(|x| x.ms));
            out["ssid"] = json!(ssid);
            out["vpn"] = json!(vpn);
            out["probes"] = json!(probes.iter().map(|x| json!({"route": x.route.name(), "ok": x.ok, "ms": x.ms, "error": x.error})).collect::<Vec<_>>());
        }
        Ok(out)
    }

    /// Drafts the server has yet to finish, so their results should be fetched soon.
    pub(crate) fn waiting_on_server(&self) -> bool {
        self.db
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM drafts WHERE deleted=0 AND (status IN ('queued','processing') OR agent_status IN ('requested','running'))", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0)
            > 0
    }

    fn pending_changes(&self) -> Result<i64> {
        let since: i64 = self.setting("sync.pushed").and_then(|s| s.parse().ok()).unwrap_or(0);
        let db = self.db.lock().unwrap();
        let mut n = 0;
        for t in crate::sync::TABLES {
            n += db.query_row(&format!("SELECT count(*) FROM {t} WHERE updated >= ?1"), [since], |r| r.get::<_, i64>(0))?;
        }
        Ok(n)
    }

    /// Pushes local changes, then pulls everything new from the server.
    pub(crate) fn sync_now(&self) -> Result<Value> {
        let p = self.pairing().ok_or_else(|| anyhow!("this device is not paired with a sync server"))?;
        let started = Instant::now();
        let (route, probes, _, _) = self.choose_route(&p);
        let Some(route) = route else {
            let why = probes.iter().filter_map(|x| x.error.clone()).collect::<Vec<_>>().join("; ");
            bail!("the sync server is not reachable{}", if why.is_empty() { String::new() } else { format!(" ({why})") });
        };
        let host = match route {
            Route::Lan => p.lan.clone(),
            Route::Wan => p.wan.clone(),
        }
        .unwrap();
        let a = agent(&p.cert, Duration::from_secs(60))?;

        let photos_sent = self.upload_photos(&a, &host, &p.token);

        // Push. Re-sending a little overlap is harmless: rows merge by timestamp.
        let pushed_before: i64 = self.setting("sync.pushed").and_then(|s| s.parse().ok()).unwrap_or(0);
        let mark = now_ms();
        let mut since = (pushed_before - 5_000).max(0);
        let mut pushed = 0usize;
        loop {
            let batch = self.sync_export(since, 1500)?;
            let rows: usize = batch["changes"].as_object().map_or(0, |m| m.values().map(|v| v.as_array().map_or(0, Vec::len)).sum());
            if rows == 0 {
                break;
            }
            post(&a, &host, &p.token, "/v1/push", &json!({"changes": batch["changes"]}))?;
            pushed += rows;
            // Rows come oldest-first per table; move the window past what was sent.
            let newest = batch["changes"].as_object().into_iter().flat_map(|m| m.values()).filter_map(Value::as_array).flatten().filter_map(|r| r["updated"].as_i64()).max().unwrap_or(since);
            if rows < 1500 || newest <= since {
                break;
            }
            since = newest;
        }
        self.set_setting("sync.pushed", &mark.to_string())?;

        // Pull in pages until caught up.
        let mut cursor: i64 = self.setting("sync.cursor").and_then(|s| s.parse().ok()).unwrap_or(0);
        let mut pulled = 0usize;
        loop {
            let page = post(&a, &host, &p.token, "/v1/pull", &json!({"cursor": cursor, "limit": 1500}))?;
            pulled += page["changes"].as_object().map_or(0, |m| m.values().map(|v| v.as_array().map_or(0, Vec::len)).sum());
            self.sync_apply(&page["changes"])?;
            cursor = page["cursor"].as_i64().unwrap_or(cursor);
            self.set_setting("sync.cursor", &cursor.to_string())?;
            if page["more"] != true {
                break;
            }
        }
        // Receive the book if this device lacks it; a failure here never fails the sync.
        let book = if self.private_dir.is_some() { self.fetch_barrons(&a, &host, &p.token).map_err(|e| format!("{e:#}")) } else { Ok(false) };
        let report = json!({"route": route.name(), "pushed": pushed, "pulled": pulled, "photos": photos_sent, "at": now_ms(), "took_ms": started.elapsed().as_millis() as u64,
                            "barrons": match &book { Ok(true) => json!("received"), Ok(false) => json!("up to date"), Err(e) => json!(format!("failed: {e}")) }});
        self.set_setting("sync.last", &report.to_string())?;
        self.dirty.store(false, Ordering::SeqCst);
        Ok(report)
    }
}

/// Keeps a paired device in step without anyone opening anything: soon after
/// a local change, and at least every `every` to pick up other devices' edits.
pub fn spawn_background_sync(api: Arc<Api>, every: Duration) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut last = Instant::now() - every;
        loop {
            std::thread::sleep(Duration::from_secs(15));
            if api.pairing().is_none() {
                continue;
            }
            // Photos waiting to be read (or agents at work) on the server: look often.
            if api.dirty.load(Ordering::SeqCst) || api.waiting_on_server() || last.elapsed() >= every {
                // Failures (offline, server down) are normal; try again next round.
                let _ = api.sync_now();
                last = Instant::now();
            }
        }
    })
}
