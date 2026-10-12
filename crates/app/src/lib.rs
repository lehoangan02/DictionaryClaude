//! Application layer: opens the packs and user data and exposes one
//! transport-agnostic command surface used by the Tauri shell and the dev server.

pub mod api;
mod ai;
mod audio;
mod drafts;
mod jobs;
pub mod barrons;
mod mw;
mod net;
mod packs;
pub mod pairing;
mod quiz;
mod sync;
mod oxford;
mod related;
mod update;
pub mod userdb;

pub use api::{Api, Config};
pub use jobs::spawn_job_runner;
pub use net::spawn_background_sync;
pub(crate) use barrons::BarronsEntry;
