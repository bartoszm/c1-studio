//! Telling a file by its content, wherever it moves: its size and a hash of its first and
//! last 64 KB ([`ContentKey`]), and, only where two files share one, a hash of the whole
//! file.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::Xxh3;

/// How much of each end of a file [`content_key`] reads.
pub const EDGE: u64 = 64 * 1024;

/// How much [`full_hash`] reads at a time.
const CHUNK: usize = 1024 * 1024;

/// A file's size and the xxh3-128 of its first and last [`EDGE`] bytes (all of it, when it
/// is no bigger than two of them), as 32 hex digits.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ContentKey {
    pub size: u64,
    pub hash: String,
}

/// The [`ContentKey`] of the file at `path`. Reads at most `2 * EDGE` bytes.
pub fn content_key(path: &Path) -> io::Result<ContentKey> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut hasher = Xxh3::new();
    if size <= 2 * EDGE {
        let mut all = Vec::with_capacity(size as usize);
        file.read_to_end(&mut all)?;
        hasher.update(&all);
    } else {
        let mut buf = vec![0u8; EDGE as usize];
        file.read_exact(&mut buf)?;
        hasher.update(&buf);
        file.seek(SeekFrom::End(-(EDGE as i64)))?;
        file.read_exact(&mut buf)?;
        hasher.update(&buf);
    }
    Ok(ContentKey { size, hash: hex(hasher.digest128()) })
}

/// The xxh3-128 of the whole file, as 32 hex digits.
pub fn full_hash(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Xxh3::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(hex(hasher.digest128()))
}

fn hex(value: u128) -> String {
    format!("{value:032x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn small_files_are_hashed_whole_and_big_ones_by_their_ends() {
        let dir = tempfile::tempdir().unwrap();
        let small = dir.path().join("s.jpg");
        write(&small, b"hello");
        let key = content_key(&small).unwrap();
        assert_eq!(key.size, 5);
        assert_eq!(key.hash, hex(xxhash_rust::xxh3::xxh3_128(b"hello")));
        assert_eq!(key.hash.len(), 32);
        assert_eq!(full_hash(&small).unwrap(), key.hash);

        // Two big files differing only in the middle share a key, not a full hash.
        let size = 3 * EDGE as usize;
        let mut a: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let (pa, pb) = (dir.path().join("a.jpg"), dir.path().join("b.jpg"));
        write(&pa, &a);
        a[size / 2] ^= 0xff;
        write(&pb, &a);
        let (ka, kb) = (content_key(&pa).unwrap(), content_key(&pb).unwrap());
        assert_eq!(ka, kb);
        assert_ne!(full_hash(&pa).unwrap(), full_hash(&pb).unwrap());
        let mut both = a[..EDGE as usize].to_vec();
        both.extend_from_slice(&a[size - EDGE as usize..]);
        assert_eq!(kb.hash, hex(xxhash_rust::xxh3::xxh3_128(&both)));
        // A change at an end changes the key.
        a[1] ^= 0xff;
        write(&pb, &a);
        assert_ne!(content_key(&pb).unwrap(), ka);
    }
}
