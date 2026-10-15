//! Telling a file by its content, wherever it moves: its size and a hash of its first and
//! last 64 KB ([`ContentKey`]), and, only where two files share one, a hash of the whole
//! file.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// How much of each end of a file [`content_key`] reads.
pub const EDGE: u64 = 64 * 1024;

/// A file's size and the xxh3-128 of its first and last [`EDGE`] bytes (all of it, when it
/// is no bigger than two of them), as 32 hex digits.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ContentKey {
    pub size: u64,
    pub hash: String,
}

pub fn content_key(path: &Path) -> io::Result<ContentKey> {
    let _ = path;
    todo!("WP2")
}

/// The xxh3-128 of the whole file, as 32 hex digits.
pub fn full_hash(path: &Path) -> io::Result<String> {
    let _ = path;
    todo!("WP2")
}
