//! Reading media files: discovery, image decoding and video frame sampling.

pub mod image;
pub mod scan;

pub use scan::{MediaKind, ScannedFile};
