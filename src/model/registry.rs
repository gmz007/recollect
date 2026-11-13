//! The embedding models Recollect knows how to use. See `docs/design.md` for how they were chosen.

/// Architecture family; decides preprocessing and text handling details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// OpenAI CLIP: shortest side → 224 then center crop; 77 tokens; eos pooling in-graph.
    Clip,
    /// Google SigLIP: squash to 224×224; lowercase; pad to 64 tokens with id 1, no mask.
    Siglip,
}

/// One file of a model, downloaded from Hugging Face at a pinned revision.
#[derive(Debug, Clone, Copy)]
pub struct ModelFile {
    /// File name inside the local model directory.
    pub local: &'static str,
    /// Path inside the Hugging Face repository.
    pub remote: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    pub repo: &'static str,
    pub revision: &'static str,
    pub license: &'static str,
    pub family: Family,
    pub dim: usize,
    pub image_size: u32,
    pub mean: [f32; 3],
    pub std: [f32; 3],
    pub files: &'static [ModelFile],
    /// Local name of the text tower inside `files` (SigLIP's int8 export has its own name so an
    /// older fp32 `text_model.onnx` is never mistaken for it).
    pub text_file: &'static str,
    /// Largest vision batch known to work on the WebGPU execution provider.
    pub gpu_max_batch: usize,
    /// Scale for i8 quantization of unit vectors in the search index.
    pub quant_scale: f32,
}

pub const VISION_FILE: &str = "vision_model.onnx";
pub const TOKENIZER_FILE: &str = "tokenizer.json";

pub const CLIP_B32: ModelSpec = ModelSpec {
    id: "clip-vit-b32",
    name: "CLIP ViT-B/32 (fast)",
    summary: "OpenAI CLIP. About 6x faster than SigLIP on a CPU, somewhat less accurate.",
    repo: "Xenova/clip-vit-base-patch32",
    revision: "6b7c523703ae26e39be4b03774966f8571c7f523",
    license: "MIT",
    family: Family::Clip,
    dim: 512,
    image_size: 224,
    mean: [0.481_454_66, 0.457_827_5, 0.408_210_73],
    std: [0.268_629_54, 0.261_302_6, 0.275_777_1],
    files: &[
        ModelFile {
            local: VISION_FILE,
            remote: "onnx/vision_model.onnx",
            size: 351_685_709,
            sha256: "fd6e1402a588279d1723c7534d4bcba5bc0b14b47dfab0e46f8c47b8270d7d40",
        },
        ModelFile {
            local: "text_model.onnx",
            remote: "onnx/text_model.onnx",
            size: 254_058_553,
            sha256: "3f6571f5bad13a97c469c1622e1cfc4d9aef78b79fdbfcff804ca357bfada8cc",
        },
        ModelFile {
            local: TOKENIZER_FILE,
            remote: "tokenizer.json",
            size: 2_224_119,
            sha256: "f7f3b7af117d467b58374797691a6438d3e6b9e9cef800dfd5dced7f697a90cd",
        },
    ],
    text_file: "text_model.onnx",
    gpu_max_batch: 32,
    quant_scale: 300.0,
};

pub const SIGLIP_B16: ModelSpec = ModelSpec {
    id: "siglip-base-16",
    name: "SigLIP base/16 (best quality)",
    summary: "Google SigLIP. Clearly better results; best with a GPU.",
    repo: "Xenova/siglip-base-patch16-224",
    revision: "4649052661e53c7000355844105f8a1792088239",
    license: "Apache-2.0",
    family: Family::Siglip,
    dim: 768,
    image_size: 224,
    mean: [0.5, 0.5, 0.5],
    std: [0.5, 0.5, 0.5],
    files: &[
        ModelFile {
            local: VISION_FILE,
            remote: "onnx/vision_model.onnx",
            size: 371_819_850,
            sha256: "f89d41bac7f4d4b87e010a467d93f98689d708916ed22f5a07f96fdfa26f475f",
        },
        ModelFile {
            local: "text_model_int8.onnx",
            // int8 text tower: retrieval quality within noise of fp32 (Recall@10 0.973 vs
            // 0.980 in our measurements), 330 MB less RAM and download.
            remote: "onnx/text_model_quantized.onnx",
            size: 111_475_220,
            sha256: "ad0329b1f35acc66d8953ff2559ce358da8eb0a7011794cf951523d63a4dbce2",
        },
        ModelFile {
            local: TOKENIZER_FILE,
            remote: "tokenizer.json",
            size: 2_398_744,
            sha256: "4a17c975210be5ab4c36b47d8dae4eefb866dbfb1e676e394aad85dc30a3ae08",
        },
    ],
    text_file: "text_model_int8.onnx",
    gpu_max_batch: 16,
    quant_scale: 300.0,
};

pub const MODELS: &[ModelSpec] = &[SIGLIP_B16, CLIP_B32];

/// Default model when a GPU is usable / when running CPU-only (see `docs/design.md`).
pub const DEFAULT_GPU_MODEL: &str = SIGLIP_B16.id;
pub const DEFAULT_CPU_MODEL: &str = CLIP_B32.id;

pub fn get(id: &str) -> Option<&'static ModelSpec> {
    MODELS.iter().find(|m| m.id == id)
}

impl ModelSpec {
    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    pub fn url(&self, file: &ModelFile) -> String {
        format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            self.repo, self.revision, file.remote
        )
    }

    /// Maximum number of text tokens the text tower accepts.
    pub fn text_len(&self) -> usize {
        match self.family {
            Family::Clip => 77,
            Family::Siglip => 64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_consistent() {
        for m in MODELS {
            assert!(get(m.id).is_some());
            assert_eq!(m.files.len(), 3);
            for f in m.files {
                assert_eq!(f.sha256.len(), 64, "{} {}", m.id, f.local);
                assert!(f.sha256.chars().all(|c| c.is_ascii_hexdigit()));
                assert!(m.url(f).starts_with("https://huggingface.co/"));
            }
            assert_eq!(m.revision.len(), 40);
        }
        assert!(get(DEFAULT_GPU_MODEL).is_some() && get(DEFAULT_CPU_MODEL).is_some());
        assert!(get("nope").is_none());
    }
}
