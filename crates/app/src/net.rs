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

static SYNC_ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Route {
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

pub(crate) struct Probe {
    route: Route,
    ok: bool,
    ms: u128,
    error: Option<String>,
}

/// Whether this device's own address on the route to `host` (an `ip:port`) shares the
/// first three bytes with it, i.e. both sit on the same home /24. Sends nothing.
fn on_subnet_of(host: &str) -> bool {
    let Ok(server) = host.parse::<std::net::SocketAddr>() else { return false };
    let (std::net::IpAddr::V4(srv), Some(local)) = (server.ip(), std::net::UdpSocket::bind("0.0.0.0:0").ok().and_then(|u| u.connect(server).ok().map(|_| u)).and_then(|u| u.local_addr().ok())) else { return false };
    matches!(local.ip(), std::net::IpAddr::V4(l) if l.octets()[..3] == srv.octets()[..3])
}

/// Where a new device looks for the server on the home network.
const DEFAULT_LAN: &str = "192.168.1.7:25";

pub(crate) fn agent(cert_pem: &str, timeout: Duration) -> Result<ureq::Agent> {
    let cert = Certificate::from_pem(cert_pem.as_bytes()).map_err(|e| anyhow!("bad server certificate: {e}"))?;
    let tls = TlsConfig::builder().root_certs(RootCerts::new_with_certs(&[cert])).build();
    Ok(ureq::Agent::config_builder().tls_config(tls).timeout_global(Some(timeout)).http_status_as_error(false).build().into())
}

pub(crate) fn post(agent: &ureq::Agent, host: &str, token: &str, path: &str, body: &Value) -> Result<Value> {
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

pub(crate) fn sha_hex(bytes: &[u8]) -> String {
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
            404 | 403 => return Ok(false), // the server has no book to share, or this device is a guest
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

    pub(crate) fn pairing(&self) -> Option<Pairing> {
        serde_json::from_str(&self.setting("sync.pairing")?).ok()
    }

    pub(crate) fn sync_pair(&self, code: &str) -> Result<Value> {
        let p = Pairing::decode(code)?;
        self.set_setting("sync.pairing", &serde_json::to_string(&p)?)?;
        self.set_setting("sync.cursor", "0")?;
        self.set_setting("sync.pushed", "0")?;
        self.db.lock().unwrap().execute("DELETE FROM settings WHERE k='sync.role'", [])?;
        Ok(json!({"paired": true, "name": p.name, "lan": p.lan, "wan": p.wan}))
    }

    /// One authenticated call to the sync server over whichever route works.
    pub(crate) fn server_post(&self, path: &str, body: Value) -> Result<Value> {
        let (host, p, a) = self.server_conn(Duration::from_secs(20))?;
        crate::net::post(&a, &host, &p.token, path, &body)
    }

    pub(crate) fn sync_unpair(&self) -> Result<Value> {
        self.db.lock().unwrap().execute("DELETE FROM settings WHERE k LIKE 'sync.%'", [])?;
        // Someone who unpairs on purpose must not be paired again behind their back.
        self.set_setting("autopair.off", "1")?;
        Ok(json!({"paired": false}))
    }

    /// Where a device may look for the server unprompted: on a known home Wi-Fi, or,
    /// when the system will not tell us the Wi-Fi name (macOS hides it from apps), on
    /// the server's own subnet. Never with a VPN up, and never on a Wi-Fi known to be
    /// someone else's (192.168.x.x there belongs to a stranger).
    fn at_home(&self) -> bool {
        if vpn_active() {
            return false;
        }
        let homes: Vec<String> = self.setting("sync.home_ssids").unwrap_or_else(|| "Anan,Anan 5g".into()).split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect();
        match wifi_name() {
            Some(s) => homes.contains(&s.to_lowercase()),
            None => on_subnet_of(self.setting("autopair.host").as_deref().unwrap_or(DEFAULT_LAN)),
        }
    }

    /// Whether the background loop should try to pair this device by itself now.
    pub(crate) fn autopair_due(&self) -> bool {
        self.pairing().is_none() && self.setting("autopair.off").as_deref() != Some("1") && self.at_home()
    }

    fn device_label(&self) -> String {
        let host = if cfg!(target_os = "android") { None } else { Command::new("hostname").output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()) };
        let base: String = host.filter(|h| !h.is_empty()).unwrap_or_else(|| std::env::consts::OS.to_string());
        let base = base.split('.').next().unwrap_or("device");
        format!("{base}-{}", self.device.chars().filter(|c| c.is_ascii_alphanumeric()).take(4).collect::<String>())
    }

    /// Pairs with the server on the home network without a code: asks its LAN port to
    /// enrol this device, then pins the certificate it hands back. The first contact
    /// cannot be verified (there is nothing to pin yet), which is why the background
    /// loop only does this on a known home Wi-Fi; the button does it on request.
    /// The result is checked with the new token over the pinned certificate before it is kept.
    pub(crate) fn sync_autopair(&self) -> Result<Value> {
        if let Some(p) = self.pairing() {
            return Ok(json!({"paired": true, "name": p.name}));
        }
        let host = self.setting("autopair.host").unwrap_or_else(|| DEFAULT_LAN.to_string());
        let tls = TlsConfig::builder().disable_verification(true).build();
        let first: ureq::Agent = ureq::Agent::config_builder().tls_config(tls).timeout_global(Some(Duration::from_millis(4000))).http_status_as_error(false).build().into();
        let mut resp = first
            .post(&format!("https://{host}/v1/enroll"))
            .header("Content-Type", "application/json")
            .send(serde_json::to_vec(&json!({"name": self.device_label()}))?)
            .map_err(|e| {
                let hint = if cfg!(target_os = "macos") && format!("{e}").contains("No route to host") {
                    " macOS may be blocking local network access: allow LadyBird in System Settings → Privacy & Security → Local Network."
                } else {
                    ""
                };
                anyhow!("could not reach the server at {host} ({e}). Is this device on the home Wi-Fi?{hint}")
            })?;
        let status = resp.status().as_u16();
        let v: Value = serde_json::from_str(&resp.body_mut().read_to_string()?).unwrap_or(Value::Null);
        if status != 200 {
            bail!("{}", v["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("the server answered {status}")));
        }
        let code = v["code"].as_str().ok_or_else(|| anyhow!("the server did not send a pairing"))?;
        let p = Pairing::decode(code)?;
        let check = verify(&p, Route::Lan, Duration::from_millis(4000));
        if !check.ok {
            bail!("the server's answer did not check out: {}", check.error.unwrap_or_default());
        }
        let out = self.sync_pair(code)?;
        self.db.lock().unwrap().execute("DELETE FROM settings WHERE k='autopair.off'", [])?;
        Ok(out)
    }

    /// Verifies the routes the rules allow, concurrently, and picks one.
    pub(crate) fn choose_route(&self, p: &Pairing) -> (Option<Route>, Vec<Probe>, Option<String>, bool) {
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

    /// A verified route to the server: its host, the pairing and a TLS agent
    /// that trusts only the paired certificate.
    pub(crate) fn server_conn(&self, timeout: Duration) -> Result<(String, Pairing, ureq::Agent)> {
        let p = self.pairing().ok_or_else(|| anyhow!("this device is not paired with a sync server"))?;
        let (route, ..) = self.choose_route(&p);
        let host = match route {
            Some(Route::Lan) => p.lan.clone(),
            Some(Route::Wan) => p.wan.clone(),
            None => bail!("the sync server is not reachable"),
        }
        .unwrap();
        let a = agent(&p.cert, timeout)?;
        Ok((host, p, a))
    }

    /// Status for the header pill and the sync settings.
    pub(crate) fn server_status(&self, probe: bool) -> Result<Value> {
        let Some(p) = self.pairing() else {
            return Ok(json!({"paired": false, "auto": self.setting("autopair.off").as_deref() != Some("1")}));
        };
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
        // One sync at a time: the chat screen, a sent message and the background loop all ask for one,
        // and overlapping runs piled up on a slow link while Claude answered, making every chat lag.
        let _one = SYNC_ONE.lock().unwrap_or_else(|e| e.into_inner());
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

        // Which store on the server this device talks to (the owner's, or its own as a guest) decides what is sent and
        // where pulling starts.
        if let Some(role) = post(&a, &host, &p.token, "/v1/hello", &json!({})).ok().and_then(|h| h["role"].as_str().map(str::to_string)) {
            self.note_role(&role);
        }

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
    // Holding only a weak handle lets the app drop and reopen the Api (after a
    // dictionary download) without leaving this loop on the old one.
    let weak = Arc::downgrade(&api);
    drop(api);
    std::thread::spawn(move || {
        let mut last = Instant::now() - every;
        let mut last_try = Instant::now() - Duration::from_secs(60);
        loop {
            std::thread::sleep(Duration::from_secs(5));
            let Some(api) = weak.upgrade() else { return };
            if api.pairing().is_none() {
                // Not paired: on the home Wi-Fi, join by ourselves (at most once a minute).
                if last_try.elapsed() >= Duration::from_secs(60) && api.autopair_due() {
                    last_try = Instant::now();
                    if api.sync_autopair().is_ok() {
                        let _ = api.sync_now();
                        last = Instant::now();
                    }
                }
                continue;
            }
            if api.packs_now_requested() {
                api.packs_auto_with(true);
                continue;
            }
            // Photos waiting to be read (or agents at work) on the server: look often.
            if api.dirty.load(Ordering::SeqCst) || api.waiting_on_server() || last.elapsed() >= every {
                // Failures (offline, server down) are normal; try again next round.
                if api.sync_now().is_ok() {
                    // Fetch (or refresh) the dictionaries by itself, on the home network.
                    api.packs_auto();
                }
                last = Instant::now();
            }
        }
    })
}
