//! On-device AI in the app: the embedding model, which hardware runs it, and
//! meaning-aware ranking for session search.

use std::sync::Arc;

use anyhow::Result;
use gredict_core::ai::{cosine, Embedder};
use serde_json::{json, Value};

use crate::api::Api;
use crate::net::sha_hex;

const MODEL: &str = "bge-small-en-v1.5-q.onnx";
const TOKENIZER: &str = "bge-small-en-v1.5-tokenizer.json";

impl Api {
    /// The embedding model, opened on first use; `Err` says why it is unavailable.
    pub(crate) fn embedder(&self) -> Option<Arc<Embedder>> {
        self.ai_result().as_ref().ok().cloned()
    }

    fn ai_result(&self) -> &std::result::Result<Arc<Embedder>, String> {
        self.ai.get_or_init(|| {
            let (m, t) = (crate::packs::locate(&self.data_dir, &self.packs_dir, MODEL), crate::packs::locate(&self.data_dir, &self.packs_dir, TOKENIZER));
            if !m.exists() || !t.exists() {
                return Err("not downloaded".into());
            }
            Embedder::open(&m, &t).map(Arc::new).map_err(|e| format!("{e:#}"))
        })
    }

    pub(crate) fn ai_status(&self) -> Result<Value> {
        Ok(match self.ai_result() {
            Ok(e) => json!({"ready": true, "backend": e.backend().id(), "backend_name": e.backend().name(),
                            "skipped": e.skipped.iter().map(|(b, why)| json!({"backend": b.name(), "why": why})).collect::<Vec<_>>()}),
            Err(why) => json!({"ready": false, "why": why}),
        })
    }

    pub(crate) fn ai_test(&self) -> Result<Value> {
        let e = self.embedder().ok_or_else(|| anyhow::anyhow!("the AI model is not installed: download it in Settings → Downloads"))?;
        let t = e.self_test()?;
        Ok(json!({"backend": t.backend.id(), "backend_name": t.backend.name(), "first_ms": t.first_ms.round(), "per_run_ms": (t.per_run_ms * 10.0).round() / 10.0, "dims": t.dims}))
    }

    /// Session ids ranked by how close their meaning is to `query`, best first
    /// (only those close enough to matter). Vectors are cached on the device.
    pub(crate) fn semantic_sessions(&self, query: &str) -> Vec<(String, f32)> {
        let Some(e) = self.embedder() else { return vec![] };
        let docs: Vec<(String, String)> = {
            let db = self.db.lock().unwrap();
            let Ok(mut st) = db.prepare("SELECT f.session_id, f.body FROM session_fts f JOIN sessions s ON s.id=f.session_id WHERE s.deleted=0") else { return vec![] };
            let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) else { return vec![] };
            rows.flatten().collect()
        };
        let Ok(q) = e.embed(&[query]) else { return vec![] };
        let mut scored = Vec::new();
        for (id, body) in docs {
            let hash = sha_hex(body.as_bytes());
            let cached: Option<Vec<f32>> = self.cache.lock().unwrap().query_row("SELECT vec FROM session_vec WHERE session_id=?1 AND hash=?2", [&id, &hash], |r| r.get::<_, Vec<u8>>(0)).ok().map(|b| b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect());
            let v = match cached {
                Some(v) => v,
                None => {
                    let Ok(mut out) = e.embed(&[body.as_str()]) else { continue };
                    let v = out.remove(0);
                    let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
                    let _ = self.cache.lock().unwrap().execute("INSERT OR REPLACE INTO session_vec VALUES(?1,?2,?3)", rusqlite::params![id, hash, bytes]);
                    v
                }
            };
            scored.push((id, cosine(&q[0], &v)));
        }
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored.retain(|x| x.1 >= 0.5);
        scored.truncate(30);
        scored
    }
}
