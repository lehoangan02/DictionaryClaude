//! Development/headless host for the app API: `POST /api/<command>` with a
//! JSON body, plus the built UI as static files. The desktop and mobile apps
//! call the same `Api::call` in-process through Tauri instead.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use gredict_app::{Api, Config};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value_t = 8787)]
    port: u16,
    /// Address to bind; keep it on loopback unless you add authentication.
    #[arg(long, default_value = "127.0.0.1")]
    bind: String,
    #[arg(long, default_value = "data/packs")]
    packs: PathBuf,
    #[arg(long, default_value = "data/user")]
    data: PathBuf,
    #[arg(long, default_value = "private")]
    private: PathBuf,
    #[arg(long, default_value = "app/ui/dist")]
    dist: PathBuf,
}

fn content_type(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json",
        Some("woff2") => "font/woff2",
        Some("png") => "image/png",
        _ => "application/octet-stream",
    }
}

fn respond(req: Request, status: u16, ctype: &str, body: Vec<u8>) {
    let h = Header::from_bytes("Content-Type", ctype).unwrap();
    let _ = req.respond(Response::from_data(body).with_status_code(StatusCode(status)).with_header(h));
}

fn handle(api: &Api, dist: &Path, mut req: Request) {
    let url = req.url().split('?').next().unwrap_or("/").to_string();
    if let Some(cmd) = url.strip_prefix("/api/") {
        if *req.method() != Method::Post {
            return respond(req, 405, "text/plain", b"POST only".to_vec());
        }
        let mut body = String::new();
        let _ = req.as_reader().read_to_string(&mut body);
        let args = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
        return match api.call(cmd, args) {
            Ok(v) => respond(req, 200, "application/json", v.to_string().into_bytes()),
            Err(e) => respond(req, 400, "application/json", serde_json::json!({"error": format!("{e:#}")}).to_string().into_bytes()),
        };
    }
    // Static files; unknown paths fall back to the app shell.
    let rel = url.trim_start_matches('/');
    let mut path = dist.join(if rel.is_empty() { "index.html" } else { rel });
    if rel.contains("..") || !path.is_file() {
        path = dist.join("index.html");
    }
    match std::fs::read(&path) {
        Ok(bytes) => respond(req, 200, content_type(&path), bytes),
        Err(_) => respond(req, 404, "text/plain", b"UI not built: run `npm run build` in app/ui".to_vec()),
    }
}

fn main() -> Result<()> {
    use std::io::Read as _;
    let a = Args::parse();
    let api = Arc::new(Api::open(Config { data_dir: a.data, packs_dir: a.packs, private_dir: Some(a.private) })?);
    let server = Server::http((a.bind.as_str(), a.port)).map_err(|e| anyhow::anyhow!("{e}"))?;
    // A paired device keeps itself in sync without anyone opening a sync screen.
    gredict_app::spawn_background_sync(api.clone(), std::time::Duration::from_secs(300));
    eprintln!("gredict-server on http://{}:{}", a.bind, a.port);
    let dist = Arc::new(a.dist);
    for req in server.incoming_requests() {
        let (api, dist) = (api.clone(), dist.clone());
        std::thread::spawn(move || handle(&api, &dist, req));
    }
    Ok(())
}
