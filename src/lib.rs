//! Recollect: local, private semantic search over photos and videos.
//!
//! The [`library::Library`] facade is the entry point for both the GUI and the CLI.

pub mod cli;
pub mod config;
pub mod eval;
pub mod gpu;
pub mod indexer;
pub mod library;
pub mod media;
pub mod model;
pub mod search;
pub mod store;
pub mod ui;
mod video_index;

pub use library::Library;
