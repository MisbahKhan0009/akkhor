use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use memmap2::Mmap;

use crate::Result;

/// The bytes behind a [`Source`]: either a memory map or an owned buffer.
///
/// Implements `AsRef<[u8]>` so it can be handed to parsers that want a
/// shared, cheaply clonable byte slice.
pub enum Bytes {
    Mapped(Mmap),
    Owned(Vec<u8>),
    Static(&'static [u8]),
}

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        match self {
            Bytes::Mapped(m) => m,
            Bytes::Owned(v) => v,
            Bytes::Static(s) => s,
        }
    }
}

/// A book (or font) as a shared, read-only byte slice.
///
/// Opening a file maps it into memory instead of reading it. A 1 GB PDF costs
/// no RAM up front: the OS loads 4 KB pages only when a parser touches them,
/// and can drop them again under memory pressure because they are backed by
/// the file. Cloning a `Source` is a reference-count bump.
#[derive(Clone)]
pub struct Source(Arc<Bytes>);

impl Source {
    /// Memory-maps the file at `path`.
    ///
    /// The file must not be truncated while it is open: reading a mapped
    /// page past the new end raises SIGBUS and ends the process. That is the
    /// price of zero-copy access, paid by every mmap-based reader. Apps
    /// should open files they control (their own library folder, or a copy
    /// of a shared file) rather than files another app may rewrite.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let file = File::open(path)?;
        Self::from_file(&file)
    }

    /// Memory-maps an already open file. On Android this is how a file handed
    /// over by another app (a `content://` URI's file descriptor) is read.
    pub fn from_file(file: &File) -> Result<Self> {
        let len = file.metadata()?.len();
        if len == 0 {
            // Mapping an empty file fails on some platforms; an empty buffer
            // behaves the same for every parser.
            return Ok(Self::from_vec(Vec::new()));
        }
        // SAFETY: the map is read-only. If another process truncates the file
        // while it is mapped, reads past the new end fault; readers open their
        // own copies or files they control, which is the accepted trade-off
        // every mmap-based reader makes for zero-copy access.
        let map = unsafe { Mmap::map(file)? };
        Ok(Self(Arc::new(Bytes::Mapped(map))))
    }

    /// Wraps bytes already in memory (downloads, tests).
    pub fn from_vec(bytes: Vec<u8>) -> Self {
        Self(Arc::new(Bytes::Owned(bytes)))
    }

    /// Wraps bytes compiled into the binary.
    pub fn from_static(bytes: &'static [u8]) -> Self {
        Self(Arc::new(Bytes::Static(bytes)))
    }

    pub fn bytes(&self) -> &[u8] {
        self.0.as_ref().as_ref()
    }

    pub fn len(&self) -> usize {
        self.bytes().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The shared backing, for parsers that keep their own handle.
    pub fn shared(&self) -> Arc<Bytes> {
        self.0.clone()
    }

    /// Hints that access will jump around (PDF objects, zip entries), so the
    /// OS should not read ahead much. No-op for in-memory sources.
    pub fn advise_random(&self) {
        #[cfg(unix)]
        if let Bytes::Mapped(m) = self.0.as_ref() {
            let _ = m.advise(memmap2::Advice::Random);
        }
    }
}

impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.0.as_ref() {
            Bytes::Mapped(_) => "mapped",
            Bytes::Owned(_) => "owned",
            Bytes::Static(_) => "static",
        };
        write!(f, "Source({kind}, {} bytes)", self.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn maps_a_file() {
        let path = std::env::temp_dir().join("akkhor-source-test.bin");
        File::create(&path)
            .unwrap()
            .write_all(b"hello akkhor")
            .unwrap();
        let src = Source::open(&path).unwrap();
        assert_eq!(src.bytes(), b"hello akkhor");
        let clone = src.clone();
        assert_eq!(clone.len(), 12);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn empty_file_is_fine() {
        let path = std::env::temp_dir().join("akkhor-source-empty.bin");
        File::create(&path).unwrap();
        let src = Source::open(&path).unwrap();
        assert!(src.is_empty());
        std::fs::remove_file(path).ok();
    }
}
