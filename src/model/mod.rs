//! Embedding models: which ones exist, getting them onto disk, and running them.

pub mod download;
pub mod embedder;
pub mod registry;

pub use embedder::{Device, TextEncoder, VisionEncoder};
pub use registry::{MODELS, ModelSpec};
