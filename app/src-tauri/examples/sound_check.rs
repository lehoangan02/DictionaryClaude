//! Plays one cached recording through the system audio, to check the sound path:
//!   cargo run --example sound_check -- <path-to-mp3>
fn main() {
    let path = std::env::args().nth(1).expect("path to an mp3");
    let bytes = std::fs::read(path).expect("readable file");
    match ladybird_lib::play_bytes(bytes) {
        Ok(()) => {
            std::thread::sleep(std::time::Duration::from_secs(3));
            println!("played");
        }
        Err(e) => {
            eprintln!("FAILED: {e}");
            std::process::exit(1);
        }
    }
}
