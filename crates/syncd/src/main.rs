use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use gredict_app::Api;
use gredict_syncd::{ensure_tls, serve_enrolling, Enroll, Pairing, Tokens};

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
        /// Addresses handed to devices that join by themselves on the home network.
        #[arg(long, default_value = "192.168.1.7:25")]
        advertise_lan: String,
        #[arg(long, default_value = "lhan.duckdns.org:5025")]
        advertise_wan: String,
        /// Do not let devices on the home network join without a code.
        #[arg(long)]
        no_auto_enroll: bool,
    },
    /// Create a device token and print its pairing code.
    Pair {
        name: String,
        /// A guest device: it sees none of the owner's data (the default is an owner device).
        #[arg(long)]
        guest: bool,
        #[arg(long, default_value = "192.168.1.7:25")]
        lan: String,
        #[arg(long, default_value = "lhan.duckdns.org:5025")]
        wan: String,
    },
    /// List paired devices and their roles.
    Devices,
    /// Set the owner password (read from the first line of standard input); other devices become guests until they enter it.
    Password,
    /// Revoke a device.
    Revoke { name: String },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    std::fs::create_dir_all(&cli.data)?;
    let tokens = Tokens::new(&cli.data);
    match cli.cmd {
        Cmd::Serve { lan, wan, advertise_lan, advertise_wan, no_auto_enroll } => {
            let tls = ensure_tls(&cli.data.join("tls"), &cli.sans)?;
            // A Gemini key for note reading may sit in <data>/gemini.key (readable by the service only).
            if std::env::var_os("GEMINI_API_KEY").is_none() {
                if let Ok(k) = std::fs::read_to_string(cli.data.join("gemini.key")) {
                    if !k.trim().is_empty() {
                        std::env::set_var("GEMINI_API_KEY", k.trim());
                    }
                }
            }
            let api = Arc::new(Api::open_headless_serving(&cli.data, &cli.packs.clone().unwrap_or_else(|| cli.data.join("packs")))?);
            // Reads photographed notes (OCR) and serves agent requests.
            let _ = api.recover_talks();
            gredict_app::spawn_job_runner(api.clone(), std::time::Duration::from_secs(5));
            {
                let api = api.clone();
                std::thread::spawn(move || api.warm_pack_hashes());
            }
            let mut addrs = Vec::new();
            for a in [lan.clone(), wan] {
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
            // Only the home-network port ever enrols devices on its own.
            let enroll = (!no_auto_enroll && addrs.contains(&lan)).then(|| (Arc::new(Enroll::new(&advertise_lan, &advertise_wan, &tls.cert_pem)), lan.clone()));
            for h in serve_enrolling(api, Arc::new(tokens), cli.private.clone(), &tls, &addrs, enroll)? {
                let _ = h.join();
            }
        }
        Cmd::Pair { name, guest, lan, wan } => {
            let tls = ensure_tls(&cli.data.join("tls"), &cli.sans)?;
            let token = tokens.issue_as(&name, !guest)?;
            let p = Pairing { v: 1, name, lan: Some(lan), wan: Some(wan), token, cert: tls.cert_pem };
            println!("{}", p.encode());
        }
        Cmd::Devices => {
            for t in tokens.list() {
                println!("{}\t{}", t.name, if t.owner { "owner" } else { "guest" });
            }
        }
        Cmd::Password => {
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            let pw = line.trim_end_matches(['\r', '\n']);
            anyhow::ensure!(pw.chars().count() >= 6, "the password needs at least 6 characters");
            tokens.set_password(pw)?;
            tokens.demote_others("")?;
            println!("owner password set; every device is a guest until it enters it in Settings");
        }
        Cmd::Revoke { name } => println!("{}", if tokens.revoke(&name)? { "revoked" } else { "no such device" }),
    }
    Ok(())
}
