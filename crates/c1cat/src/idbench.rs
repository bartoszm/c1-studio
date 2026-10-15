//! `idbench`: how fast photos get their stable ids, and whether two different files ever
//! share one. A photo folder source tells files apart by [`content_key`] (size plus a hash
//! of the first and last 64 KB); this reads a real folder with it, one file at a time (or on
//! all cores with `--parallel`, as a scan does), and checks every shared key against a
//! hash of the whole files.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Context;
use rayon::prelude::*;
use serde_json::json;
use source_folders::ids::{EDGE, content_key, full_hash};
use source_folders::meta::FileKind;

/// What [`run`] measured.
#[derive(Debug, Default)]
pub struct Report {
    pub dir: PathBuf,
    /// Whether the files were keyed on all cores at once.
    pub parallel: bool,
    /// Photo files keyed.
    pub files: u64,
    /// Their sizes together.
    pub total_bytes: u64,
    /// What keying them read: at most [`EDGE`] from each end of each file.
    pub read_bytes: u64,
    /// Seconds spent keying (the walk of the folders not included).
    pub seconds: f64,
    /// Files that could not be read, so were not keyed.
    pub unreadable: u64,
    /// Keys more than one file has.
    pub shared_keys: u64,
    /// Of those, the keys whose files are all the same (copies).
    pub same_content: Collisions,
    /// Of those, the keys whose files differ: the collisions that would merge two photos.
    pub different_content: Collisions,
    /// Seconds spent on the whole-file hashes of the files that share a key.
    pub full_hash_seconds: f64,
    /// Bytes those hashes read.
    pub full_hash_bytes: u64,
}

/// Keys that more than one file has, and the files holding them.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Collisions {
    pub keys: u64,
    pub files: u64,
}

impl Report {
    pub fn files_per_second(&self) -> f64 {
        per_second(self.files as f64, self.seconds)
    }

    /// Megabytes (10^6 bytes) actually read per second.
    pub fn read_mb_per_second(&self) -> f64 {
        per_second(self.read_bytes as f64 / 1e6, self.seconds)
    }

    pub fn to_json(&self) -> serde_json::Value {
        let collisions = |c: &Collisions| json!({ "keys": c.keys, "files": c.files });
        json!({
            "dir": self.dir,
            "parallel": self.parallel,
            "files": self.files,
            "total_bytes": self.total_bytes,
            "read_bytes": self.read_bytes,
            "seconds": self.seconds,
            "files_per_second": self.files_per_second(),
            "read_mb_per_second": self.read_mb_per_second(),
            "unreadable": self.unreadable,
            "shared_keys": self.shared_keys,
            "same_content": collisions(&self.same_content),
            "different_content": collisions(&self.different_content),
            "full_hash_seconds": self.full_hash_seconds,
            "full_hash_bytes": self.full_hash_bytes,
        })
    }
}

fn per_second(amount: f64, seconds: f64) -> f64 {
    if seconds > 0.0 { amount / seconds } else { 0.0 }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "folder      {}", self.dir.display())?;
        writeln!(
            f,
            "files       {} ({:.1} MB), {} unreadable",
            self.files,
            self.total_bytes as f64 / 1e6,
            self.unreadable
        )?;
        writeln!(
            f,
            "key time    {:.2} s{}: {:.0} files/s, {:.1} MB/s read ({:.1} MB of the {:.1} MB)",
            self.seconds,
            if self.parallel { " on all cores" } else { ", one file at a time" },
            self.files_per_second(),
            self.read_mb_per_second(),
            self.read_bytes as f64 / 1e6,
            self.total_bytes as f64 / 1e6
        )?;
        writeln!(f, "shared keys {}", self.shared_keys)?;
        writeln!(
            f,
            "  same content       {} keys, {} files (copies of one file: expected)",
            self.same_content.keys, self.same_content.files
        )?;
        writeln!(
            f,
            "  different content  {} keys, {} files (two photos with one id: the risk)",
            self.different_content.keys, self.different_content.files
        )?;
        write!(
            f,
            "full hashes {:.2} s for {:.1} MB, of the files that share a key",
            self.full_hash_seconds,
            self.full_hash_bytes as f64 / 1e6
        )
    }
}

/// The photo files below `dir` (at most `limit`) with their sizes, in the order found.
/// Links are not followed; a folder that cannot be read is skipped.
fn photos(dir: &Path, limit: Option<usize>) -> Vec<(PathBuf, u64)> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_owned()];
    while let Some(folder) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file()
                && path.extension().is_some_and(|e| FileKind::from_ext(&e.to_string_lossy()).is_some())
            {
                let Ok(meta) = entry.metadata() else { continue };
                found.push((path, meta.len()));
                if limit.is_some_and(|n| found.len() >= n) {
                    return found;
                }
            }
        }
    }
    found
}

/// Keys the photos below `dir`, on all cores if `parallel`, and checks the keys that more
/// than one file has.
pub fn run(dir: &Path, limit: Option<usize>, parallel: bool) -> anyhow::Result<Report> {
    if !dir.is_dir() {
        anyhow::bail!("{} is not a folder", dir.display());
    }
    let files = photos(dir, limit);
    let mut report = Report { dir: dir.to_owned(), parallel, ..Report::default() };
    let mut by_key: HashMap<_, Vec<&Path>> = HashMap::new();
    let started = Instant::now();
    let keyed: Vec<_> = if parallel {
        files.par_iter().map(|(path, size)| (path, *size, content_key(path))).collect()
    } else {
        files.iter().map(|(path, size)| (path, *size, content_key(path))).collect()
    };
    report.seconds = started.elapsed().as_secs_f64();
    for (path, size, key) in keyed {
        match key {
            Ok(key) => {
                report.files += 1;
                report.total_bytes += size;
                report.read_bytes += size.min(2 * EDGE);
                by_key.entry(key).or_default().push(path);
            }
            Err(_) => report.unreadable += 1,
        }
    }

    let started = Instant::now();
    for (key, paths) in by_key.iter().filter(|(_, paths)| paths.len() > 1) {
        report.shared_keys += 1;
        let mut hashes = std::collections::HashSet::new();
        for path in paths {
            report.full_hash_bytes += key.size;
            hashes.insert(full_hash(path).with_context(|| format!("hashing {}", path.display()))?);
        }
        let group = if hashes.len() == 1 { &mut report.same_content } else { &mut report.different_content };
        group.keys += 1;
        group.files += paths.len() as u64;
    }
    report.full_hash_seconds = started.elapsed().as_secs_f64();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_photo_files_are_found_and_the_limit_stops_the_walk() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("a").join("b");
        std::fs::create_dir_all(&deep).unwrap();
        for (folder, name) in [(dir.path(), "x.JPG"), (&deep, "y.raf"), (&deep, "notes.txt"), (dir.path(), "z.xmp")] {
            std::fs::write(folder.join(name), b"12345").unwrap();
        }
        let all = photos(dir.path(), None);
        let mut names: Vec<_> =
            all.iter().map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["x.JPG", "y.raf"]);
        assert!(all.iter().all(|(_, size)| *size == 5));
        assert_eq!(photos(dir.path(), Some(1)).len(), 1);
        assert!(run(&dir.path().join("x.JPG"), None, false).is_err(), "a file is not a folder");
    }

    #[test]
    fn copies_and_collisions_are_counted_the_same_one_at_a_time_and_in_parallel() {
        let dir = tempfile::tempdir().unwrap();
        let size = 3 * EDGE as usize;
        let base: Vec<u8> = (0..size).map(|i| (i % 253) as u8).collect();
        let mut other = base.clone();
        other[size / 2] ^= 1;
        let write = |name: &str, bytes: &[u8]| {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        };
        write("x/a.jpg", &base);
        write("y/a-copy.JPG", &base);
        write("y/b.raf", &other);
        write("c.png", b"unique");
        let third: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        write("z/d1.jpg", &third);
        write("z/d2.jpg", &third);
        write("notes.txt", &base);
        for parallel in [false, true] {
            let report = run(dir.path(), None, parallel).unwrap();
            assert_eq!((report.files, report.parallel), (6, parallel), "{report}");
            assert_eq!(report.total_bytes, 5 * size as u64 + 6);
            assert_eq!(report.read_bytes, 5 * 2 * EDGE + 6);
            // a.jpg, a-copy.JPG and b.raf share a key, and b.raf is not a copy; d1 and d2 are.
            assert_eq!(report.shared_keys, 2);
            assert_eq!(report.same_content, Collisions { keys: 1, files: 2 });
            assert_eq!(report.different_content, Collisions { keys: 1, files: 3 });
            assert_eq!(report.full_hash_bytes, 5 * size as u64);
        }
        assert_eq!(run(dir.path(), Some(2), false).unwrap().files, 2);
    }

    #[test]
    fn the_report_prints_and_serialises_its_rates() {
        let report = Report {
            dir: "photos".into(),
            files: 1000,
            total_bytes: 8_000_000_000,
            read_bytes: 100_000_000,
            seconds: 2.0,
            shared_keys: 3,
            same_content: Collisions { keys: 2, files: 4 },
            different_content: Collisions { keys: 1, files: 2 },
            ..Report::default()
        };
        assert_eq!(report.files_per_second(), 500.0);
        assert_eq!(report.read_mb_per_second(), 50.0);
        let text = report.to_string();
        assert!(
            text.contains("500 files/s") && text.contains("50.0 MB/s") && text.contains("2 keys, 4 files"),
            "{text}"
        );
        let json = report.to_json();
        assert_eq!(json["different_content"]["files"], 2);
        assert_eq!(json["files_per_second"], 500.0);
        assert_eq!(Report::default().files_per_second(), 0.0, "no time, no rate");
    }
}
