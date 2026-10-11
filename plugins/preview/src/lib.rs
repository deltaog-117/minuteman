//! The preview column, as a plugin. The host tells it which file the column is about (the
//! `preview` event); it reads that file through the host (`read_file`, so a folder on another
//! machine or inside an archive previews like the local disk does) and answers with a finished
//! document (`set_preview`): the source with its tokens classified, the first bytes of a binary
//! file, or an archive's listing. The interface does the drawing, the scrolling and the colors, so
//! moving through a long file never waits on this process.
//!
//! `remote` is the host seen as a read-only `Vfs`, `load` decides what a file is and reads it
//! (with `archive`, `head` and `highlight` doing the work for each kind, and `hook` for a
//! configured text extractor), and `run` drives it all from the lines on stdin.

pub mod archive;
pub mod head;
pub mod highlight;
pub mod hook;
pub mod load;
pub mod remote;
pub mod run;

pub use run::{Config, run};
