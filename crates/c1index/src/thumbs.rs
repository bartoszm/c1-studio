//! Thumbnails made here for photos whose source has none (Capture One leaves about 4% of
//! variants without a `.cot`). Made from the source's preview, else from its original,
//! framed as [`c1image::load`] frames them, and kept as JPEGs in a folder of their own.
//! The source's own thumbnail always wins once it has one.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use c1model::{ImageFormat, ImageRef, ImageTier, PhotoId, PhotoSource};
use serde::Serialize;

use crate::filter::{Filter, Trashed};
use crate::library::{Library, QueryOptions, Sort};

/// The long edge, as Capture One's own thumbnails.
const LONG_EDGE: u32 = 450;
const QUALITY: u8 = 85;

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Progress {
    /// Photos tried so far, of `total` without a thumbnail.
    pub done: usize,
    pub total: usize,
    /// Thumbnails made.
    pub made: usize,
    /// Photos with nothing to make one from: no preview, and the original is missing
    /// (its drive not connected) or unreadable.
    pub failed: usize,
}

/// A photo no thumbnail could be made for, and why.
#[derive(Clone, Debug)]
pub struct Failure {
    pub row: u64,
    pub why: String,
}

pub struct Thumbs {
    dir: PathBuf,
}

impl Thumbs {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, id: &PhotoId) -> PathBuf {
        let name: String =
            id.ext.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
        self.dir.join(format!("{name}.jpg"))
    }

    /// One made here earlier, ready to show as it is.
    fn made(&self, id: &PhotoId) -> Option<ImageRef> {
        let path = self.path(id);
        path.is_file().then_some(ImageRef {
            path,
            format: ImageFormat::Jpeg,
            white_balance: None,
            rotation: 0,
            monochrome: false,
            look: None,
            straighten: 0.0,
            crop: None,
        })
    }

    /// The source's thumbnail, or else one made here.
    pub fn get(&self, source: &dyn PhotoSource, id: &PhotoId) -> anyhow::Result<Option<ImageRef>> {
        Ok(match source.image(id, ImageTier::Thumb)? {
            Some(r) => Some(r),
            None => self.made(id),
        })
    }

    /// [`Thumbs::get`], making the thumbnail first when there is none yet.
    pub fn get_or_make(&self, source: &dyn PhotoSource, id: &PhotoId) -> anyhow::Result<Option<ImageRef>> {
        match self.get(source, id)? {
            Some(r) => Ok(Some(r)),
            None => Ok(self.make(source, id).ok()),
        }
    }

    /// Makes the thumbnail from the preview, else the original. The error says why neither
    /// served ("no preview; original not found").
    pub fn make(&self, source: &dyn PhotoSource, id: &PhotoId) -> anyhow::Result<ImageRef> {
        let mut pixels = None;
        let mut why = Vec::new();
        for (tier, name) in [(ImageTier::Preview, "preview"), (ImageTier::Original, "original")] {
            match source.image(id, tier) {
                Ok(Some(r)) => match c1image::load_fitted(&r, LONG_EDGE) {
                    Ok(p) => {
                        pixels = Some(p);
                        break;
                    }
                    Err(e) => why.push(format!("{name} unreadable ({e:#})")),
                },
                Ok(None) if tier == ImageTier::Preview => why.push("no preview".into()),
                Ok(None) => why.push("original not found".into()),
                Err(e) => why.push(format!("{name}: {e:#}")),
            }
        }
        let Some(pixels) = pixels else { anyhow::bail!("{}", why.join("; ")) };
        std::fs::create_dir_all(&self.dir)?;
        let path = self.path(id);
        // Written aside and moved in, so a reader never sees half a file; the batch and a
        // tile asking for the same photo each write their own.
        static WRITES: AtomicUsize = AtomicUsize::new(0);
        let part =
            path.with_extension(format!("{}-{}.part", std::process::id(), WRITES.fetch_add(1, Ordering::Relaxed)));
        std::fs::write(&part, c1image::encode_jpeg(&pixels, QUALITY))?;
        std::fs::rename(&part, &path)?;
        self.made(id).ok_or_else(|| anyhow::anyhow!("{} vanished once written", path.display()))
    }

    /// The library's photos that show no thumbnail yet, by row.
    pub fn missing(&self, library: &Library, source: &dyn PhotoSource) -> anyhow::Result<Vec<(u64, PhotoId)>> {
        let all = Filter { trashed: Trashed::Include, ..Filter::default() };
        let rows = library.query(&all, &QueryOptions { sort: Sort::Row, ..QueryOptions::default() })?.rows;
        let mine = source.info().id;
        let mut out = Vec::new();
        for row in rows {
            let Some(id) = library.photo_id(row) else { continue };
            if id.source == mine && matches!(self.get(source, &id), Ok(None)) {
                out.push((row, id));
            }
        }
        Ok(out)
    }

    /// Makes the thumbnails `missing` lists, on `threads` threads. `stop` ends it early;
    /// `report` hears now and then. Returns the last progress and the photos that failed.
    pub fn fill(
        &self,
        source: &dyn PhotoSource,
        missing: &[(u64, PhotoId)],
        threads: usize,
        stop: &AtomicBool,
        report: impl Fn(Progress) + Sync,
    ) -> (Progress, Vec<Failure>) {
        let next = AtomicUsize::new(0);
        let progress = Mutex::new(Progress { total: missing.len(), ..Progress::default() });
        let failures = Mutex::new(Vec::new());
        report(*progress.lock().unwrap());
        std::thread::scope(|scope| {
            for _ in 0..threads.max(1) {
                scope.spawn(|| {
                    while !stop.load(Ordering::Relaxed) {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some((row, id)) = missing.get(i) else { break };
                        let made = self.make(source, id);
                        if let Err(e) = &made {
                            failures.lock().unwrap().push(Failure { row: *row, why: format!("{e:#}") });
                        }
                        let made = made.is_ok();
                        let now = {
                            let mut p = progress.lock().unwrap();
                            p.done += 1;
                            if made {
                                p.made += 1
                            } else {
                                p.failed += 1
                            }
                            *p
                        };
                        if now.done % 25 == 0 || now.done == now.total {
                            report(now);
                        }
                    }
                });
            }
        });
        let mut failures = failures.into_inner().unwrap();
        failures.sort_by_key(|f| f.row);
        (progress.into_inner().unwrap(), failures)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn file_names_keep_to_safe_characters() {
        let thumbs = Thumbs::new("t");
        let id = PhotoId { source: c1model::SourceId("c1:x".into()), ext: "12/34:5".into() };
        assert_eq!(thumbs.path(&id), Path::new("t").join("12_34_5.jpg"));
    }
}
