//! Reading files through the host. The reader itself lives in `plugin_api::remote` so every plugin
//! that reads through the host shares it; this module keeps the name the preview code uses.

pub use plugin_api::remote::{Remote, RemoteVfs};

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor, Read, Seek, SeekFrom};
    use std::path::Path;
    use std::sync::Arc;

    use plugin_api::{ChunkKind, FileChunk, ReadFile};
    use serde_json::Value;
    use shared::{FileKind, MemVfs, Vfs};

    use super::*;

    /// The host's side of `read_file`, answered from an in-memory backend.
    struct Host(MemVfs);

    impl Remote for Host {
        fn call(&self, _method: &str, params: Value) -> Result<Value, String> {
            let request: ReadFile = serde_json::from_value(params).map_err(|e| e.to_string())?;
            let meta = self.0.metadata(&request.path).map_err(|e| e.to_string())?;
            let kind = match meta.kind {
                FileKind::File => ChunkKind::File,
                FileKind::Dir => ChunkKind::Dir,
                _ => ChunkKind::Other,
            };
            let mut data = Vec::new();
            if kind == ChunkKind::File && request.len > 0 {
                let mut file = self.0.open_read(&request.path).map_err(|e| e.to_string())?;
                file.seek(SeekFrom::Start(request.offset))
                    .map_err(|e| e.to_string())?;
                file.take(request.len)
                    .read_to_end(&mut data)
                    .map_err(|e| e.to_string())?;
            }
            serde_json::to_value(FileChunk {
                kind,
                size: meta.len,
                modified: None,
                local_path: None,
                data,
            })
            .map_err(|e| e.to_string())
        }
    }

    fn remote_with(files: &[(&str, Vec<u8>)]) -> RemoteVfs {
        let mem = MemVfs::new();
        mem.add_dir("/d");
        for (name, bytes) in files {
            mem.add_file(name, bytes.clone());
        }
        RemoteVfs::new(Arc::new(Host(mem)) as Arc<dyn Remote>)
    }

    #[test]
    fn an_archive_is_listed_through_the_host() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("inside.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        io::Write::write_all(&mut writer, b"hi").unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        let vfs = remote_with(&[("/a.zip", bytes)]);

        let listing = crate::archive::list(&vfs, Path::new("/a.zip")).expect("a zip lists");
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].name, "inside.txt");
    }
}
