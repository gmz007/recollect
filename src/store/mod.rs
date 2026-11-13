//! Persistence (SQLite) and the in-memory vector index.

pub mod db;
pub mod index;

pub use db::Db;
pub use index::VectorIndex;
