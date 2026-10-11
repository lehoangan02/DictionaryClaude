//! Thin shell: one command, `call`, that forwards to the shared Rust API, plus
//! a windowless `--agent` mode so syncing keeps working with no app open.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gredict_app::{Api, Config};
use serde_json::Value;
use tauri::Manager;

struct State(Arc<Api>);

#[tauri::command]
async fn call(state: tauri::State<'_, State>, cmd: String, args: Option<Value>) -> Result<Value, String> {
    let api = state.0.clone();
    // The API blocks on SQLite and the network; keep it off the UI thread.
    tauri::async_runtime::spawn_blocking(move || api.call(&cmd, args.unwrap_or(Value::Null)))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))
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
            let api = Arc::new(Api::open(config(app.handle()))?);
            gredict_app::spawn_background_sync(api.clone(), Duration::from_secs(300));
            if agent {
                // No window: only the sync loop keeps running.
                if let Some(w) = app.get_webview_window("main") {
                    w.hide()?;
                }
            }
            app.manage(State(api));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![call])
        .run(tauri::generate_context!())
        .expect("error while running Lexis");
}

/// Starts `lexis --agent` at login (a LaunchAgent on macOS, an autostart entry
/// on Linux), so devices sync without anyone opening the app.
fn install_agent() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let home = PathBuf::from(std::env::var("HOME")?);
    if cfg!(target_os = "macos") {
        let path = home.join("Library/LaunchAgents/dev.lexis.agent.plist");
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>dev.lexis.agent</string>\n<key>ProgramArguments</key><array><string>{}</string><string>--agent</string></array>\n<key>RunAtLoad</key><true/>\n<key>KeepAlive</key><true/>\n<key>ProcessType</key><string>Background</string>\n</dict></plist>\n",
            exe.display()
        ))?;
        Ok(path)
    } else {
        let path = home.join(".config/autostart/lexis-agent.desktop");
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, format!("[Desktop Entry]\nType=Application\nName=Lexis sync\nExec={} --agent\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n", exe.display()))?;
        Ok(path)
    }
}
