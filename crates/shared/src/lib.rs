//! Core, read-only infrastructure shared by every Minuteman feature crate.
//!
//! This crate must never depend on a feature crate (`browser`, `file_ops`, `theming`, ...) —
//! dependencies always point inward, toward `shared`.

pub mod error;
pub mod vfs;

pub use error::VfsError;
pub use vfs::{DirEntryInfo, LocalVfs, Vfs};
