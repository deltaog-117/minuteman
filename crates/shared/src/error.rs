use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum VfsError {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("not a directory: {0}")]
    NotADirectory(PathBuf),
}
