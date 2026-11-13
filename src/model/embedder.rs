//! Image and text encoders on top of ONNX Runtime (ADR 0002).
//!
//! Each encoder owns its ORT session and is meant to live on one thread (indexer thread for
//! vision, search thread for text).

use super::registry::{Family, ModelSpec, TEXT_FILE, TOKENIZER_FILE, VISION_FILE};
use anyhow::{Context, Result, anyhow, bail};
use ndarray::{Array2, Array4};
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::TensorRef;
use std::path::Path;
use std::sync::OnceLock;

/// Where inference runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    Cpu,
    Gpu,
}

impl std::fmt::Display for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Device::Cpu => "CPU",
            Device::Gpu => "GPU (WebGPU)",
        })
    }
}

fn environment() -> Result<&'static ort::environment::Environment> {
    static ENV: OnceLock<std::result::Result<ort::environment::Environment, String>> =
        OnceLock::new();
    ENV.get_or_init(|| {
        ort::init()
            .with_name("recollect")
            // Privacy: never send ONNX Runtime telemetry.
            .with_telemetry(false)
            .build()
            .map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(|e| anyhow!("initializing ONNX Runtime: {e}"))
}

fn session(path: &Path, threads: usize, gpu: bool) -> Result<Session> {
    let env = environment()?;
    let mut b = Session::builder(env)
        .map_err(|e| anyhow!("{e}"))?
        // Level 3 (layout transforms) breaks SigLIP on WebGPU; Level 2 is as fast on CPU.
        .with_optimization_level(GraphOptimizationLevel::Level2)
        .map_err(|e| anyhow!("{e}"))?
        .with_intra_threads(threads.max(1))
        .map_err(|e| anyhow!("{e}"))?;
    if gpu {
        b = b
            .with_execution_providers([ort::ep::WebGPU::default().build().error_on_failure()])
            .map_err(|e| anyhow!("{e}"))?;
    }
    b.commit_from_file(path)
        .map_err(|e| anyhow!("loading {}: {e}", path.display()))
}

fn output_name(s: &Session, preferred: &str) -> String {
    let names: Vec<&str> = s.outputs().iter().map(|o| o.name()).collect();
    for want in [preferred, "pooler_output"] {
        if names.contains(&want) {
            return want.to_string();
        }
    }
    names.first().map(|s| s.to_string()).unwrap_or_default()
}

/// L2-normalizes in place.
pub fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 1e-12 {
        v.iter_mut().for_each(|x| *x /= n);
    }
}

/// Converts an RGB8 `size×size` image into a normalized CHW f32 tensor slice.
pub fn rgb_to_chw(spec: &ModelSpec, rgb: &[u8], out: &mut [f32]) {
    let n = (spec.image_size * spec.image_size) as usize;
    debug_assert_eq!(rgb.len(), 3 * n);
    debug_assert_eq!(out.len(), 3 * n);
    for (i, px) in rgb.as_chunks::<3>().0.iter().enumerate() {
        for c in 0..3 {
            out[c * n + i] = (px[c] as f32 / 255.0 - spec.mean[c]) / spec.std[c];
        }
    }
}

/// Image tower.
pub struct VisionEncoder {
    spec: &'static ModelSpec,
    session: Session,
    input: String,
    output: String,
    device: Device,
    batch: usize,
}

impl VisionEncoder {
    /// Loads the vision model, trying the GPU first when `prefer_gpu` and falling back to the
    /// CPU if the GPU session can't be created or fails a warm-up run.
    pub fn load(
        spec: &'static ModelSpec,
        dir: &Path,
        prefer_gpu: bool,
        threads: usize,
    ) -> Result<Self> {
        let path = dir.join(VISION_FILE);
        if prefer_gpu {
            match Self::with_device(spec, &path, Device::Gpu, threads) {
                Ok(v) => return Ok(v),
                Err(e) => tracing::warn!("GPU unavailable for {}, using CPU: {e:#}", spec.id),
            }
        }
        Self::with_device(spec, &path, Device::Cpu, threads)
    }

    fn with_device(
        spec: &'static ModelSpec,
        path: &Path,
        device: Device,
        threads: usize,
    ) -> Result<Self> {
        let session = session(path, threads, device == Device::Gpu)?;
        let input = s_input(&session)?;
        let output = output_name(&session, "image_embeds");
        let batch = match device {
            Device::Gpu => spec.gpu_max_batch,
            Device::Cpu => 16,
        };
        let mut v = Self {
            spec,
            session,
            input,
            output,
            device,
            batch,
        };
        // Warm-up catches EPs that load but cannot run this graph.
        let px = vec![0u8; 3 * (spec.image_size * spec.image_size) as usize];
        let warm: Vec<&[u8]> = vec![&px; 2];
        v.embed(&warm).context("warm-up inference")?;
        Ok(v)
    }

    pub fn device(&self) -> Device {
        self.device
    }

    /// Preferred number of images per [`Self::embed`] call.
    pub fn batch_size(&self) -> usize {
        self.batch
    }

    pub fn spec(&self) -> &'static ModelSpec {
        self.spec
    }

    /// Embeds RGB8 images of `image_size²` pixels; returns one unit vector per image.
    pub fn embed(&mut self, images: &[&[u8]]) -> Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(images.len());
        let batch = self.batch;
        for chunk in images.chunks(batch) {
            out.extend(self.embed_batch(chunk)?);
        }
        Ok(out)
    }

    fn embed_batch(&mut self, images: &[&[u8]]) -> Result<Vec<Vec<f32>>> {
        let s = self.spec.image_size as usize;
        let per = 3 * s * s;
        let mut flat = vec![0f32; images.len() * per];
        for (img, dst) in images.iter().zip(flat.chunks_exact_mut(per)) {
            if img.len() != per {
                bail!("image buffer has {} bytes, expected {per}", img.len());
            }
            rgb_to_chw(self.spec, img, dst);
        }
        let arr = Array4::from_shape_vec((images.len(), 3, s, s), flat)?;
        let outputs = self
            .session
            .run(ort::inputs![self.input.as_str() => TensorRef::from_array_view(&arr)?])?;
        let (shape, data) = outputs[self.output.as_str()].try_extract_tensor::<f32>()?;
        let d = shape[shape.len() - 1] as usize;
        if d != self.spec.dim {
            bail!("model returned {d}-dim vectors, expected {}", self.spec.dim);
        }
        Ok(data
            .chunks_exact(d)
            .map(|c| {
                let mut v = c.to_vec();
                normalize(&mut v);
                v
            })
            .collect())
    }
}

fn s_input(s: &Session) -> Result<String> {
    s.inputs()
        .first()
        .map(|i| i.name().to_string())
        .ok_or_else(|| anyhow!("model has no inputs"))
}

/// Text tower. Runs on the CPU: a single query is cheap, and keeping it off the GPU avoids
/// contention with indexing.
pub struct TextEncoder {
    spec: &'static ModelSpec,
    session: Session,
    tokenizer: tokenizers::Tokenizer,
    output: String,
    needs_mask: bool,
}

impl TextEncoder {
    pub fn load(spec: &'static ModelSpec, dir: &Path, threads: usize) -> Result<Self> {
        let session = session(&dir.join(TEXT_FILE), threads, false)?;
        let tokenizer = tokenizers::Tokenizer::from_file(dir.join(TOKENIZER_FILE))
            .map_err(|e| anyhow!("loading tokenizer: {e}"))?;
        let output = output_name(&session, "text_embeds");
        let needs_mask = session
            .inputs()
            .iter()
            .any(|i| i.name() == "attention_mask");
        Ok(Self {
            spec,
            session,
            tokenizer,
            output,
            needs_mask,
        })
    }

    /// Token ids exactly as the model expects them.
    pub fn tokenize(&self, text: &str) -> Result<Vec<i64>> {
        tokenize(self.spec, &self.tokenizer, text)
    }

    pub fn embed(&mut self, text: &str) -> Result<Vec<f32>> {
        let ids = self.tokenize(text)?;
        let n = ids.len();
        let ids = Array2::from_shape_vec((1, n), ids)?;
        let outputs = if self.needs_mask {
            let mask = Array2::from_elem((1, n), 1i64);
            self.session.run(ort::inputs![
                "input_ids" => TensorRef::from_array_view(&ids)?,
                "attention_mask" => TensorRef::from_array_view(&mask)?
            ])?
        } else {
            self.session
                .run(ort::inputs!["input_ids" => TensorRef::from_array_view(&ids)?])?
        };
        let (_, data) = outputs[self.output.as_str()].try_extract_tensor::<f32>()?;
        let mut v = data.to_vec();
        if v.len() != self.spec.dim {
            bail!(
                "text model returned {} values, expected {}",
                v.len(),
                self.spec.dim
            );
        }
        normalize(&mut v);
        Ok(v)
    }
}

pub(crate) fn tokenize(
    spec: &ModelSpec,
    tok: &tokenizers::Tokenizer,
    text: &str,
) -> Result<Vec<i64>> {
    let text = match spec.family {
        Family::Siglip => text.to_lowercase(),
        Family::Clip => text.to_string(),
    };
    let enc = tok
        .encode(text, true)
        .map_err(|e| anyhow!("tokenizing: {e}"))?;
    let mut ids: Vec<i64> = enc.get_ids().iter().map(|&x| i64::from(x)).collect();
    let max = spec.text_len();
    if ids.len() > max {
        // Keep the end-of-text token: CLIP pools at the highest id (eos), SigLIP at the last.
        let last = *ids.last().unwrap_or(&0);
        ids.truncate(max);
        ids[max - 1] = last;
    }
    if spec.family == Family::Siglip {
        // SigLIP was trained with padding="max_length" (pad id 1) and no attention mask.
        ids.resize(max, 1);
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::registry::{CLIP_B32, SIGLIP_B16};

    #[test]
    fn normalize_unit_length() {
        let mut v = vec![3.0, 4.0];
        normalize(&mut v);
        assert!((v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6);
        let mut z = vec![0.0, 0.0];
        normalize(&mut z);
        assert_eq!(z, vec![0.0, 0.0]);
    }

    #[test]
    fn chw_layout_and_normalization() {
        let n = (224 * 224) as usize;
        let mut rgb = vec![0u8; 3 * n];
        rgb[0] = 255; // R of first pixel
        rgb[4] = 255; // G of second pixel
        let mut out = vec![0f32; 3 * n];
        rgb_to_chw(&SIGLIP_B16, &rgb, &mut out);
        assert_eq!(out[0], 1.0); // (1-0.5)/0.5
        assert_eq!(out[1], -1.0);
        assert_eq!(out[n + 1], 1.0);
        rgb_to_chw(&CLIP_B32, &rgb, &mut out);
        assert!((out[0] - (1.0 - 0.481_454_66) / 0.268_629_54).abs() < 1e-5);
    }
}
