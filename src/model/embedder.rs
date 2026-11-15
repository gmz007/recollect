//! Image and text encoders on top of ONNX Runtime (see `docs/design.md`).
//!
//! Each encoder owns its ORT session and is meant to live on one thread (indexer thread for
//! vision, search thread for text).

use super::registry::{Family, ModelSpec, TOKENIZER_FILE, VISION_FILE};
use anyhow::{Context, Result, anyhow, bail};
use ndarray::{Array2, Array4};
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::TensorRef;
use std::path::{Path, PathBuf};
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

fn output_name(s: &Session, preferred: &str) -> Result<String> {
    let names: Vec<&str> = s.outputs().iter().map(ort::value::Outlet::name).collect();
    for want in [preferred, "pooler_output"] {
        if names.contains(&want) {
            return Ok(want.to_string());
        }
    }
    names
        .first()
        .map(|n| (*n).to_string())
        .ok_or_else(|| anyhow!("model has no outputs"))
}

/// Splits a flat output tensor of the given `shape` into `count` unit vectors of `dim`
/// values. A model that returns an unexpected shape is an error, not a panic.
fn unit_rows(shape: &[i64], data: &[f32], dim: usize, count: usize) -> Result<Vec<Vec<f32>>> {
    let last = shape.last().context("model returned a scalar output")?;
    let d = usize::try_from(*last).with_context(|| format!("invalid output dimension {last}"))?;
    if d != dim {
        bail!("model returned {d}-dim vectors, expected {dim}");
    }
    if d == 0 || data.len() < count * d {
        bail!(
            "model returned {} values for {count} vectors of {d}",
            data.len()
        );
    }
    Ok(data
        .chunks_exact(d)
        .take(count)
        .map(|c| {
            let mut v = c.to_vec();
            normalize(&mut v);
            v
        })
        .collect())
}

/// L2-normalizes in place.
pub fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 1e-12 {
        for x in &mut v[..] {
            *x /= n;
        }
    }
}

/// Converts an RGB8 `size×size` image into a normalized CHW f32 tensor slice.
pub fn rgb_to_chw(spec: &ModelSpec, rgb: &[u8], out: &mut [f32]) {
    let n = (spec.image_size * spec.image_size) as usize;
    debug_assert_eq!(rgb.len(), 3 * n);
    debug_assert_eq!(out.len(), 3 * n);
    for (i, px) in rgb.as_chunks::<3>().0.iter().enumerate() {
        for c in 0..3 {
            out[c * n + i] = (f32::from(px[c]) / 255.0 - spec.mean[c]) / spec.std[c];
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
    path: PathBuf,
    threads: usize,
    /// GPU inference failures so far (1st: rebuild the session, 2nd: fall back to CPU).
    gpu_failures: u32,
}

impl std::fmt::Debug for VisionEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VisionEncoder")
            .field("model", &self.spec.id)
            .field("device", &self.device)
            .field("batch", &self.batch)
            .finish_non_exhaustive()
    }
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
        let output = output_name(&session, "image_embeds")?;
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
            path: path.to_path_buf(),
            threads,
            gpu_failures: 0,
        };
        // Warm-up catches EPs that load but cannot run this graph.
        let px = vec![0u8; 3 * (spec.image_size * spec.image_size) as usize];
        let warm: Vec<&[u8]> = vec![&px; 2];
        // Directly, not through `embed`: its GPU recovery would call back into this function.
        v.embed_batch(&warm).context("warm-up inference")?;
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
    ///
    /// WebGPU occasionally fails a run ("Invalid Buffer"). The first failure rebuilds the GPU
    /// session and retries; a second one switches this encoder to the CPU for good.
    pub fn embed(&mut self, images: &[&[u8]]) -> Result<Vec<Vec<f32>>> {
        // Validate inputs first so caller bugs are not mistaken for GPU failures.
        let per = 3 * (self.spec.image_size * self.spec.image_size) as usize;
        if let Some(bad) = images.iter().find(|i| i.len() != per) {
            bail!("image buffer has {} bytes, expected {per}", bad.len());
        }
        let mut out = Vec::with_capacity(images.len());
        let mut start = 0;
        while start < images.len() {
            let chunk = &images[start..(start + self.batch).min(images.len())];
            match self.embed_batch(chunk) {
                Ok(v) => {
                    out.extend(v);
                    start += chunk.len();
                }
                Err(e) if self.device == Device::Gpu => self.recover_from_gpu_error(&e)?,
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    fn recover_from_gpu_error(&mut self, e: &anyhow::Error) -> Result<()> {
        self.gpu_failures += 1;
        let device = if self.gpu_failures == 1 {
            tracing::warn!("GPU inference failed ({e:#}); rebuilding the GPU session");
            Device::Gpu
        } else {
            tracing::warn!("GPU inference failed again ({e:#}); continuing on the CPU");
            Device::Cpu
        };
        let failures = self.gpu_failures;
        let fresh = match Self::with_device(self.spec, &self.path, device, self.threads) {
            Ok(fresh) => fresh,
            Err(e) => {
                tracing::warn!("rebuilding the {device} session failed ({e:#}); using the CPU");
                Self::with_device(self.spec, &self.path, Device::Cpu, self.threads)?
            }
        };
        *self = fresh;
        self.gpu_failures = failures;
        Ok(())
    }

    fn embed_batch(&mut self, images: &[&[u8]]) -> Result<Vec<Vec<f32>>> {
        let s = self.spec.image_size as usize;
        let per = 3 * s * s;
        // On the GPU always run full, fixed-size batches (zero padded): constant shapes let
        // WebGPU reuse its buffers, which avoids sporadic allocation failures.
        let rows = if self.device == Device::Gpu {
            self.batch.max(images.len())
        } else {
            images.len()
        };
        let mut flat = vec![0f32; rows * per];
        for (img, dst) in images.iter().zip(flat.chunks_exact_mut(per)) {
            if img.len() != per {
                bail!("image buffer has {} bytes, expected {per}", img.len());
            }
            rgb_to_chw(self.spec, img, dst);
        }
        let arr = Array4::from_shape_vec((rows, 3, s, s), flat)?;
        let outputs = self
            .session
            .run(ort::inputs![self.input.as_str() => TensorRef::from_array_view(&arr)?])?;
        let (shape, data) = outputs[self.output.as_str()].try_extract_tensor::<f32>()?;
        unit_rows(shape, data, self.spec.dim, images.len())
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

impl std::fmt::Debug for TextEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextEncoder")
            .field("model", &self.spec.id)
            .finish_non_exhaustive()
    }
}

impl TextEncoder {
    pub fn load(spec: &'static ModelSpec, dir: &Path, threads: usize) -> Result<Self> {
        let session = session(&dir.join(spec.text_file), threads, false)?;
        let tokenizer = tokenizers::Tokenizer::from_file(dir.join(TOKENIZER_FILE))
            .map_err(|e| anyhow!("loading tokenizer: {e}"))?;
        let output = output_name(&session, "text_embeds")?;
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

    pub fn spec(&self) -> &'static ModelSpec {
        self.spec
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

fn tokenize(spec: &ModelSpec, tok: &tokenizers::Tokenizer, text: &str) -> Result<Vec<i64>> {
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
    fn malformed_output_shapes_are_errors() {
        let data = [1.0f32, 0.0, 0.0, 1.0];
        let rows = unit_rows(&[2, 2], &data, 2, 2).unwrap();
        assert_eq!(rows, vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
        assert!(unit_rows(&[], &data, 2, 2).is_err(), "scalar output");
        assert!(
            unit_rows(&[2, -1], &data, 2, 2).is_err(),
            "negative dimension"
        );
        assert!(unit_rows(&[2, 3], &data, 2, 2).is_err(), "wrong dimension");
        assert!(unit_rows(&[0, 0], &[], 0, 1).is_err(), "zero dimension");
        assert!(unit_rows(&[2, 2], &data[..3], 2, 2).is_err(), "short data");
    }

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
