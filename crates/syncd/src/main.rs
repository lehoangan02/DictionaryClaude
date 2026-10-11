use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use gredict_app::Api;
use gredict_syncd::{ensure_tls, serve, Pairing, Tokens};

#[derive(Parser)]
#[command(about = "LadyBird sync server")]
struct Cli {
    /// Holds user.sqlite, tls/ and tokens.json.
    #[arg(long, default_value = "data/syncd", global = true)]
    data: PathBuf,
    /// Names and addresses clients connect to; baked into the certificate on first run.
    #[arg(long = "san", global = true, default_values_t = ["lhan.duckdns.org".to_string(), "192.168.1.7".to_string(), "localhost".to_string(), "127.0.0.1".to_string()])]
    sans: Vec<String>,
    /// Where barrons.sqlite lives; it is offered to paired devices.
    #[arg(long, default_value = "private", global = true)]
    private: PathBuf,
    /// Dictionary packs offered to paired devices (default: <data>/packs).
    #[arg(long, global = true)]
    packs: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the server.
    Serve {
        /// Port for devices on the home network.
        #[arg(long, default_value = "0.0.0.0:25")]
        lan: String,
        /// Port for devices elsewhere (forward this one on the router).
        #[arg(long, default_value = "0.0.0.0:5025")]
        wan: String,
    },
    /// Create a device token and print its pairing code.
    Pair {
        name: String,
        #[arg(long, default_value = "192.168.1.7:25")]
        lan: String,
        #[arg(long, default_value = "lhan.duckdns.org:5025")]
        wan: String,
    },
    /// List paired devices.
    Devices,
    /// Revoke a device.
    Revoke { name: String },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    std::fs::create_dir_all(&cli.data)?;
    let tokens = Tokens::new(&cli.data);
    match cli.cmd {
        Cmd::Serve { lan, wan } => {
            let tls = ensure_tls(&cli.data.join("tls"), &cli.sans)?;
            let api = Arc::new(Api::open_headless_serving(&cli.data, &cli.packs.clone().unwrap_or_else(|| cli.data.join("packs")))?);
            // Reads photographed notes (OCR) and serves agent requests.
            gredict_app::spawn_job_runner(api.clone(), std::time::Duration::from_secs(5));
            let mut addrs = Vec::new();
            for a in [lan, wan] {
                // Ports below 1024 need CAP_NET_BIND_SERVICE; keep serving the others.
                match std::net::TcpListener::bind(&a) {
                    Ok(l) => {
                        drop(l);
                        addrs.push(a)
                    }
                    Err(e) => eprintln!("cannot listen on {a}: {e} (skipping)"),
                }
            }
            anyhow::ensure!(!addrs.is_empty(), "no address could be bound");
            eprintln!("lexis-syncd listening on {}", addrs.join(", "));
            for h in serve(api, Arc::new(tokens), cli.private.clone(), &tls, &addrs)? {
                let _ = h.join();
            }
        }
        Cmd::Pair { name, lan, wan } => {
            let tls = ensure_tls(&cli.data.join("tls"), &cli.sans)?;
            let token = tokens.issue(&name)?;
            let p = Pairing { v: 1, name, lan: Some(lan), wan: Some(wan), token, cert: tls.cert_pem };
            println!("{}", p.encode());
        }
        Cmd::Devices => {
            for t in tokens.list() {
                println!("{}", t.name);
            }
        }
        Cmd::Revoke { name } => println!("{}", if tokens.revoke(&name)? { "revoked" } else { "no such device" }),
    }
    Ok(())
}
