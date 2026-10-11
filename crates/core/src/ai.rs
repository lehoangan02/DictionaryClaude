//! The one on-device AI runtime. Every model runs through here, and the
//! hardware is chosen in a fixed order: NPU, then GPU, then CPU.
//!
//! * Apple silicon: CoreML (which uses the Neural Engine, then the GPU), then CPU.
//! * Android: NNAPI (the system hands the model to the phone's NPU/DSP, then
//!   GPU, whatever the driver offers), then CPU.
//! * Desktop Linux/Windows: CPU (an NVIDIA GPU would need a different build).
//!
//! `Embedder::open` reports which backend is actually in use, so the app can
//! show the truth instead of the hope.

use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use ort::session::Session;
use ort::value::Tensor;
use tokenizers::Tokenizer;

/// What the model ended up running on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Apple Neural Engine / GPU through CoreML.
    CoreMl,
    /// Android Neural Networks API (NPU, DSP or GPU, as the driver decides).
    Nnapi,
    Cpu,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Backend::CoreMl => "Neural Engine (CoreML)",
            Backend::Nnapi => "NPU/GPU (NNAPI)",
            Backend::Cpu => "CPU",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Backend::CoreMl => "coreml",
            Backend::Nnapi => "nnapi",
            Backend::Cpu => "cpu",
        }
    }
}

/// Providers to try, best first. `LEXIS_AI_BACKEND=cpu` forces the CPU.
fn ladder() -> Vec<Backend> {
    let forced = std::env::var("LEXIS_AI_BACKEND").unwrap_or_default();
    let mut v = Vec::new();
    if forced != "cpu" {
        if cfg!(target_os = "macos") {
            v.push(Backend::CoreMl);
        }
        if cfg!(target_os = "android") {
            v.push(Backend::Nnapi);
        }
    }
    v.push(Backend::Cpu);
    v
}

fn build(model: &Path, backend: Backend) -> Result<Session> {
    let mut b = Session::builder().map_err(|e| anyhow!("{e}"))?;
    match backend {
        #[cfg(target_os = "macos")]
        Backend::CoreMl => {
            use ort::execution_providers::CoreMLExecutionProvider;
            b = b.with_execution_providers([CoreMLExecutionProvider::default().build().error_on_failure()]).map_err(|e| anyhow!("{e}"))?;
        }
        #[cfg(target_os = "android")]
        Backend::Nnapi => {
            use ort::execution_providers::NNAPIExecutionProvider;
            b = b.with_execution_providers([NNAPIExecutionProvider::default().build().error_on_failure()]).map_err(|e| anyhow!("{e}"))?;
        }
        _ => {}
    }
    b.commit_from_file(model).map_err(|e| anyhow!("{e}"))
}

/// Turns text into meaning vectors (bge-small-en, 384 numbers each).
pub struct Embedder {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
    backend: Backend,
    /// Providers that were tried first and why they were skipped.
    pub skipped: Vec<(Backend, String)>,
}

impl Embedder {
    pub fn open(model: &Path, tokenizer: &Path) -> Result<Self> {
        let mut tok = Tokenizer::from_file(tokenizer).map_err(|e| anyhow!("tokenizer: {e}"))?;
        tok.with_truncation(Some(tokenizers::TruncationParams { max_length: 256, ..Default::default() })).map_err(|e| anyhow!("{e}"))?;
        tok.with_padding(Some(tokenizers::PaddingParams::default()));
        let mut skipped = Vec::new();
        for backend in ladder() {
            match build(model, backend) {
                Ok(session) => {
                    let me = Embedder { session: Mutex::new(session), tokenizer: tok, backend, skipped };
                    // A provider can load and still fail on the first run: prove it works.
                    match me.embed(&["warm up"]) {
                        Ok(_) => return Ok(me),
                        Err(e) if backend != Backend::Cpu => {
                            let mut skipped = me.skipped;
                            skipped.push((backend, format!("{e:#}")));
                            let session = build(model, Backend::Cpu)?;
                            let cpu = Embedder { session: Mutex::new(session), tokenizer: me.tokenizer, backend: Backend::Cpu, skipped };
                            cpu.embed(&["warm up"])?;
                            return Ok(cpu);
                        }
                        Err(e) => return Err(e),
                    }
                }
                Err(e) if backend != Backend::Cpu => skipped.push((backend, format!("{e:#}"))),
                Err(e) => return Err(e).context("could not start the AI runtime"),
            }
        }
        Err(anyhow!("no backend available"))
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    /// One unit-length vector per text.
    pub fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        let enc = self.tokenizer.encode_batch(texts.to_vec(), true).map_err(|e| anyhow!("{e}"))?;
        let n = enc.len();
        let len = enc[0].get_ids().len();
        let flat = |f: &dyn Fn(&tokenizers::Encoding) -> Vec<i64>| -> Vec<i64> { enc.iter().flat_map(|e| f(e)).collect() };
        let ids = flat(&|e| e.get_ids().iter().map(|&x| x as i64).collect());
        let mask = flat(&|e| e.get_attention_mask().iter().map(|&x| x as i64).collect());
        let types = flat(&|e| e.get_type_ids().iter().map(|&x| x as i64).collect());
        let shape = [n, len];
        let mut session = self.session.lock().unwrap();
        let outputs = session
            .run(ort::inputs![
                "input_ids" => Tensor::from_array((shape, ids)).map_err(|e| anyhow!("{e}"))?,
                "attention_mask" => Tensor::from_array((shape, mask.clone())).map_err(|e| anyhow!("{e}"))?,
                "token_type_ids" => Tensor::from_array((shape, types)).map_err(|e| anyhow!("{e}"))?
            ])
            .map_err(|e| anyhow!("{e}"))?;
        // The model returns one vector per token; the sentence vector is the [CLS] token, normalized.
        let (dims, data) = outputs[0].try_extract_tensor::<f32>().map_err(|e| anyhow!("{e}"))?;
        let hidden = *dims.last().unwrap() as usize;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let start = i * len * hidden;
            let mut v: Vec<f32> = data[start..start + hidden].to_vec();
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
            v.iter_mut().for_each(|x| *x /= norm);
            out.push(v);
        }
        Ok(out)
    }

    /// Runs a small fixed job and reports which hardware did it and how fast.
    pub fn self_test(&self) -> Result<SelfTest> {
        let texts = ["obdurate means stubbornly refusing to change", "a person who talks too much", "to make something less severe"];
        let t = Instant::now();
        let v = self.embed(&texts)?;
        let first_ms = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        for _ in 0..5 {
            self.embed(&texts)?;
        }
        let per_run_ms = t.elapsed().as_secs_f64() * 1000.0 / 5.0;
        Ok(SelfTest { backend: self.backend, first_ms, per_run_ms, dims: v[0].len() })
    }
}

#[derive(Debug, Clone)]
pub struct SelfTest {
    pub backend: Backend,
    pub first_ms: f64,
    pub per_run_ms: f64,
    pub dims: usize,
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}
