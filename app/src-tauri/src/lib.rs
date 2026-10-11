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
#[cfg(not(target_os = "android"))]
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
#[cfg(not(target_os = "android"))]
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

/// The version this app was built as (release.sh sets it), shown in Settings.
#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Installs a downloaded update and restarts into it. The file must be one the app itself
/// downloaded (inside its `updates` folder).
#[tauri::command]
async fn update_install(app: tauri::AppHandle, state: tauri::State<'_, State>, path: String) -> Result<(), String> {
    let dir = state.1.data_dir.join("updates").canonicalize().map_err(|e| e.to_string())?;
    let file = PathBuf::from(&path).canonicalize().map_err(|e| e.to_string())?;
    if !file.starts_with(&dir) || !file.file_name().is_some_and(|n| n.to_string_lossy().starts_with("LadyBird-")) {
        return Err("that is not a downloaded update".into());
    }
    install_update(&app, &file).map_err(|e| format!("{e:#}"))
}

#[cfg(target_os = "macos")]
fn install_update(app: &tauri::AppHandle, file: &std::path::Path) -> anyhow::Result<()> {
    use std::process::{Command, Stdio};
    let exe = std::env::current_exe()?;
    let bundle = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).ok_or_else(|| anyhow::anyhow!("this copy is not running from an app bundle"))?.to_path_buf();
    // A helper outlives this process: waits for it to quit, swaps the bundle for the one in the
    // disk image, clears the download quarantine flag and opens the new app.
    let script = r#"#!/bin/sh
PID="$1"; DMG="$2"; APP="$3"
while kill -0 "$PID" 2>/dev/null; do sleep 0.3; done
MNT="$(mktemp -d)"
hdiutil attach -nobrowse -readonly -mountpoint "$MNT" "$DMG" >/dev/null || exit 1
rm -rf "$APP.new"
if cp -R "$MNT/LadyBird.app" "$APP.new"; then rm -rf "$APP" && mv "$APP.new" "$APP"; fi
hdiutil detach "$MNT" >/dev/null 2>&1
xattr -dr com.apple.quarantine "$APP" 2>/dev/null
open "$APP"
"#;
    let sh = std::env::temp_dir().join("ladybird-update.sh");
    std::fs::write(&sh, script)?;
    Command::new("/bin/sh").arg(&sh).arg(std::process::id().to_string()).arg(file).arg(&bundle).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    app.exit(0);
    Ok(())
}

#[cfg(all(target_os = "linux", not(target_os = "android")))]
fn install_update(app: &tauri::AppHandle, file: &std::path::Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let target = PathBuf::from(std::env::var("APPIMAGE").map_err(|_| anyhow::anyhow!("self-update needs the AppImage build"))?);
    let staged = target.with_extension("AppImage.new");
    std::fs::copy(file, &staged)?;
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    std::fs::rename(&staged, &target)?;
    std::process::Command::new(&target).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn()?;
    app.exit(0);
    Ok(())
}

/// Android: the system installer takes over (MainActivity.installApk).
#[cfg(target_os = "android")]
fn install_update(app: &tauri::AppHandle, file: &std::path::Path) -> anyhow::Result<()> {
    use jni::objects::JValue;
    let window = app.get_webview_window("main").ok_or_else(|| anyhow::anyhow!("no window"))?;
    let path = file.to_string_lossy().to_string();
    window.with_webview(move |webview| {
        webview.jni_handle().exec(move |env, activity, _webview| {
            if let Ok(p) = env.new_string(&path) {
                let _ = env.call_method(activity, "installApk", "(Ljava/lang/String;)V", &[JValue::Object(&p)]);
            }
        });
    })?;
    Ok(())
}

#[tauri::command]
async fn play_audio(state: tauri::State<'_, State>, url: String) -> Result<(), String> {
    let api = state.api();
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = api.audio_bytes(&url).map_err(|e| format!("{e:#}"))?;
        #[cfg(not(target_os = "android"))]
        return play_bytes(bytes);
        #[cfg(target_os = "android")]
        {
            // Android's web view plays audio itself; the page never asks for this.
            let _ = bytes;
            Err("native playback is not used on Android".to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn speak_text(word: String) -> Result<(), String> {
    #[cfg(not(target_os = "android"))]
    return tauri::async_runtime::spawn_blocking(move || say(&word)).await.map_err(|e| e.to_string())?;
    #[cfg(target_os = "android")]
    {
        let _ = word;
        Err("speech is done by the web view on Android".to_string())
    }
}

/// Reopens the Api when its packs changed, so every search sees them.
fn reopen_if_needed(state: &State) -> anyhow::Result<()> {
    if state.api().take_reload() {
        let fresh = Arc::new(Api::open(state.1.clone())?);
        gredict_app::spawn_background_sync(fresh.clone(), Duration::from_secs(300));
        let warm = fresh.clone();
        std::thread::spawn(move || warm.warm_counts());
        *state.0.write().unwrap() = fresh;
    }
    Ok(())
}

#[tauri::command]
async fn call(state: tauri::State<'_, State>, cmd: String, args: Option<Value>) -> Result<Value, String> {
    let api = state.api();
    // The API blocks on SQLite and the network; keep it off the UI thread.
    let out = tauri::async_runtime::spawn_blocking(move || api.call(&cmd, args.unwrap_or(Value::Null)))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))?;
    // A dictionary arrived or left (by hand here, or by the background setup): reopen.
    reopen_if_needed(&state).map_err(|e| format!("{e:#}"))?;
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
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
            eprintln!("LadyBird setup begins");
            let cfg = config(app.handle());
            eprintln!("LadyBird data {:?} packs {:?}", cfg.data_dir, cfg.packs_dir);
            let api = Arc::new(Api::open(cfg.clone()).map_err(|e| {
                eprintln!("LadyBird could not start: {e:#}");
                e
            })?);
            api.seed_defaults();
            gredict_app::spawn_background_sync(api.clone(), Duration::from_secs(300));
            if agent {
                // No window: only the sync loop keeps running.
                if let Some(w) = app.get_webview_window("main") {
                    w.hide()?;
                }
            }
            let warm = api.clone();
            std::thread::spawn(move || warm.warm_counts());
            app.manage(State(std::sync::RwLock::new(api), cfg));
            // Dictionaries downloaded by the background setup need the Api reopened too.
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(3));
                let state = handle.state::<State>();
                if let Err(e) = reopen_if_needed(&state) {
                    eprintln!("LadyBird could not reopen its data: {e:#}");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![call, play_audio, speak_text, app_version, update_install])
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
