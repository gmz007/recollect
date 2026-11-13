//! Reading media files: discovery, image decoding and video frame sampling.

pub mod image;
pub mod metadata;
pub mod scan;
pub mod video;

pub use scan::{MediaKind, ScannedFile};
