//! Thin shell: one command, `call`, that forwards to the shared Rust API, plus
//! a windowless `--agent` mode so syncing keeps working with no app open.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gredict_app::{Api, Config};
use serde_json::Value;
use tauri::Manager;

/// The open API. Installing or removing a dictionary reopens it, so the
/// pointer is swappable.
struct State(std::sync::RwLock<Arc<Api>>, Config);

impl State {
    fn api(&self) -> Arc<Api> {
        self.0.read().unwrap().clone()
    }
}

/// Only the newest request plays: starting a new word stops the previous one.
static PLAY_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Decodes and plays an MP3/Ogg recording through the system audio output.
pub fn play_bytes(bytes: Vec<u8>) -> Result<(), String> {
    let gen = PLAY_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    // The output stream must live on the thread that plays it.
    std::thread::spawn(move || {
        let run = || -> Result<(), String> {
            let (_stream, handle) = rodio::OutputStream::try_default().map_err(|e| format!("no audio output ({e})"))?;
            let sink = rodio::Sink::try_new(&handle).map_err(|e| e.to_string())?;
            let source = rodio::Decoder::new(std::io::Cursor::new(bytes)).map_err(|e| format!("cannot decode the recording ({e})"))?;
            sink.append(source);
            let _ = tx.send(Ok(()));
            while !sink.empty() && PLAY_GEN.load(std::sync::atomic::Ordering::SeqCst) == gen {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(())
        };
        if let Err(e) = run() {
            let _ = tx.send(Err(e));
        }
    });
    rx.recv_timeout(Duration::from_secs(5)).map_err(|_| "audio did not start".to_string())?
}

/// Says a word with the system's speech program when no recording is available.
pub fn say(word: &str) -> Result<(), String> {
    let word: String = word.chars().filter(|c| c.is_alphanumeric() || " '-".contains(*c)).collect();
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("say", &[])]
    } else {
        &[("spd-say", &["-w", "-l", "en"]), ("espeak-ng", &["-v", "en-us"]), ("espeak", &["-v", "en-us"])]
    };
    for (prog, args) in candidates {
        if std::process::Command::new(prog).args(*args).arg(&word).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().is_ok() {
            return Ok(());
        }
    }
    Err("no speech program is installed".into())
}

#[tauri::command]
async fn play_audio(state: tauri::State<'_, State>, url: String) -> Result<(), String> {
    let api = state.api();
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = api.audio_bytes(&url).map_err(|e| format!("{e:#}"))?;
        play_bytes(bytes)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn speak_text(word: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || say(&word)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn call(state: tauri::State<'_, State>, cmd: String, args: Option<Value>) -> Result<Value, String> {
    let api = state.api();
    // The API blocks on SQLite and the network; keep it off the UI thread.
    let out = tauri::async_runtime::spawn_blocking(move || api.call(&cmd, args.unwrap_or(Value::Null)))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))?;
    if out["reload"] == true {
        // A dictionary arrived or left: reopen so every search sees it.
        let fresh = Arc::new(Api::open(state.1.clone()).map_err(|e| format!("{e:#}"))?);
        gredict_app::spawn_background_sync(fresh.clone(), Duration::from_secs(300));
        *state.0.write().unwrap() = fresh;
    }
    Ok(out)
}

/// Where things live. Environment variables override for development; in an
/// installed app the packs ship inside the bundle and user data sits in the
/// platform's app-data directory.
fn config(app: &tauri::AppHandle) -> Config {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let data = env("LEXIS_DATA").or_else(|| app.path().app_data_dir().ok()).unwrap_or_else(|| PathBuf::from("data/user"));
    let packs = env("LEXIS_PACKS")
        .or_else(|| app.path().resource_dir().ok().map(|d| d.join("packs")).filter(|p| p.exists()))
        .unwrap_or_else(|| PathBuf::from("data/packs"));
    // Barron's is private: it is never bundled, only received from the owner's server.
    let private = env("LEXIS_PRIVATE").unwrap_or_else(|| data.join("private"));
    Config { data_dir: data, packs_dir: packs, private_dir: Some(private) }
}

pub fn run() {
    let agent = std::env::args().any(|a| a == "--agent");
    if std::env::args().any(|a| a == "--install-agent") {
        match install_agent() {
            Ok(p) => println!("background sync installed: {}", p.display()),
            Err(e) => eprintln!("could not install: {e:#}"),
        }
        return;
    }
    tauri::Builder::default()
        .setup(move |app| {
            let cfg = config(app.handle());
            let api = Arc::new(Api::open(cfg.clone())?);
            api.seed_defaults();
            gredict_app::spawn_background_sync(api.clone(), Duration::from_secs(300));
            if agent {
                // No window: only the sync loop keeps running.
                if let Some(w) = app.get_webview_window("main") {
                    w.hide()?;
                }
            }
            app.manage(State(std::sync::RwLock::new(api), cfg));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![call, play_audio, speak_text])
        .run(tauri::generate_context!())
        .expect("error while running LadyBird");
}

/// Starts `lexis --agent` at login (a LaunchAgent on macOS, an autostart entry
/// on Linux), so devices sync without anyone opening the app.
fn install_agent() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let home = PathBuf::from(std::env::var("HOME")?);
    if cfg!(target_os = "macos") {
        let path = home.join("Library/LaunchAgents/dev.ladybird.agent.plist");
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>dev.ladybird.agent</string>\n<key>ProgramArguments</key><array><string>{}</string><string>--agent</string></array>\n<key>RunAtLoad</key><true/>\n<key>KeepAlive</key><true/>\n<key>ProcessType</key><string>Background</string>\n</dict></plist>\n",
            exe.display()
        ))?;
        Ok(path)
    } else {
        let path = home.join(".config/autostart/ladybird-agent.desktop");
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, format!("[Desktop Entry]\nType=Application\nName=LadyBird sync\nExec={} --agent\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n", exe.display()))?;
        Ok(path)
    }
}
