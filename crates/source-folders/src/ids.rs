//! Telling a file by its content, wherever it moves: its size and a hash of its first and
//! last 64 KB ([`ContentKey`]), and, only where two files share one, a hash of the whole
//! file.

use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Instant;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::Xxh3;

use crate::meta::FileKind;

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

/// How fast content keys are made over a folder of photos, and how often two files share
/// one.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct BenchReport {
    /// Photo files hashed (unreadable ones are left out).
    pub files: usize,
    /// Their total size.
    pub bytes: u64,
    /// Time spent making content keys.
    pub seconds: f64,
    pub files_per_sec: f64,
    /// Files whose key and whole content equal an earlier file's: true duplicates.
    pub collisions_same_content: usize,
    /// Files sharing a key with an earlier file whose whole content differs from every one
    /// before it: what the full hash had to tell apart.
    pub collisions_different_content: usize,
    /// Time spent hashing whole files that share a key.
    pub full_hash_seconds: f64,
}

impl fmt::Display for BenchReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mb = self.bytes as f64 / (1024.0 * 1024.0);
        writeln!(f, "{} photo files, {mb:.1} MB", self.files)?;
        writeln!(f, "content keys: {:.2} s, {:.0} files/s", self.seconds, self.files_per_sec)?;
        writeln!(f, "same key, same content (duplicates): {}", self.collisions_same_content)?;
        writeln!(f, "same key, different content: {}", self.collisions_different_content)?;
        write!(f, "full hashes of shared keys: {:.2} s", self.full_hash_seconds)
    }
}

/// Makes the content key of every photo file under `dir` (the first `limit` of them, if
/// given), timing it, then hashes whole the files that share a key to count duplicates and
/// false collisions.
pub fn bench(dir: &Path, limit: Option<usize>) -> io::Result<BenchReport> {
    if !dir.is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("{} is not a folder", dir.display())));
    }
    let files: Vec<PathBuf> = walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()).and_then(FileKind::from_ext).is_some())
        .map(|e| e.into_path())
        .take(limit.unwrap_or(usize::MAX))
        .collect();

    let started = Instant::now();
    let keys: Vec<(PathBuf, ContentKey)> =
        files.into_par_iter().filter_map(|path| content_key(&path).ok().map(|key| (path, key))).collect();
    let seconds = started.elapsed().as_secs_f64();

    let mut report = BenchReport {
        files: keys.len(),
        bytes: keys.iter().map(|(_, k)| k.size).sum(),
        seconds,
        files_per_sec: if seconds > 0.0 { keys.len() as f64 / seconds } else { 0.0 },
        ..BenchReport::default()
    };
    let mut groups: HashMap<&ContentKey, Vec<&Path>> = HashMap::new();
    for (path, key) in &keys {
        groups.entry(key).or_default().push(path);
    }
    let started = Instant::now();
    for paths in groups.values().filter(|p| p.len() > 1) {
        let fulls: Vec<String> = paths.par_iter().filter_map(|p| full_hash(p).ok()).collect();
        let mut distinct: Vec<&String> = fulls.iter().collect();
        distinct.sort();
        distinct.dedup();
        report.collisions_same_content += fulls.len() - distinct.len();
        report.collisions_different_content += distinct.len().saturating_sub(1);
    }
    report.full_hash_seconds = started.elapsed().as_secs_f64();
    Ok(report)
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

    #[test]
    fn bench_counts_duplicates_and_collisions() {
        let dir = tempfile::tempdir().unwrap();
        let size = 3 * EDGE as usize;
        let base: Vec<u8> = (0..size).map(|i| (i % 253) as u8).collect();
        let mut other = base.clone();
        other[size / 2] ^= 1;
        write(&dir.path().join("x/a.jpg"), &base);
        write(&dir.path().join("y/a-copy.JPG"), &base);
        write(&dir.path().join("y/b.raf"), &other);
        write(&dir.path().join("c.png"), b"unique");
        write(&dir.path().join("notes.txt"), &base);
        let report = bench(dir.path(), None).unwrap();
        assert_eq!(report.files, 4, "{report}");
        assert_eq!(report.collisions_same_content, 1);
        assert_eq!(report.collisions_different_content, 1);
        assert_eq!(report.bytes, 3 * size as u64 + 6);
        assert!(report.to_string().contains("4 photo files"));
        assert_eq!(bench(dir.path(), Some(2)).unwrap().files, 2);
        assert!(bench(&dir.path().join("nowhere"), None).is_err());
    }
}
