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
            None => self.make(source, id),
        }
    }

    /// Makes the thumbnail from the preview, else the original. `None` when neither can be
    /// read.
    pub fn make(&self, source: &dyn PhotoSource, id: &PhotoId) -> anyhow::Result<Option<ImageRef>> {
        let mut pixels = None;
        for tier in [ImageTier::Preview, ImageTier::Original] {
            if let Ok(Some(r)) = source.image(id, tier)
                && let Ok(p) = c1image::load_fitted(&r, LONG_EDGE)
            {
                pixels = Some(p);
                break;
            }
        }
        let Some(pixels) = pixels else { return Ok(None) };
        std::fs::create_dir_all(&self.dir)?;
        let path = self.path(id);
        // Written aside and moved in, so a reader never sees half a file; the batch and a
        // tile asking for the same photo each write their own.
        static WRITES: AtomicUsize = AtomicUsize::new(0);
        let part =
            path.with_extension(format!("{}-{}.part", std::process::id(), WRITES.fetch_add(1, Ordering::Relaxed)));
        std::fs::write(&part, c1image::encode_jpeg(&pixels, QUALITY))?;
        std::fs::rename(&part, &path)?;
        Ok(self.made(id))
    }

    /// The library's photos that show no thumbnail yet.
    pub fn missing(&self, library: &Library, source: &dyn PhotoSource) -> anyhow::Result<Vec<PhotoId>> {
        let all = Filter { trashed: Trashed::Include, ..Filter::default() };
        let rows = library.query(&all, &QueryOptions { sort: Sort::Row, ..QueryOptions::default() })?.rows;
        let mine = source.info().id;
        let mut out = Vec::new();
        for row in rows {
            let Some(id) = library.photo_id(row) else { continue };
            if id.source == mine && matches!(self.get(source, &id), Ok(None)) {
                out.push(id);
            }
        }
        Ok(out)
    }

    /// Makes the thumbnails `missing` lists, on `threads` threads. `stop` ends it early;
    /// `report` hears now and then.
    pub fn fill(
        &self,
        source: &dyn PhotoSource,
        missing: &[PhotoId],
        threads: usize,
        stop: &AtomicBool,
        report: impl Fn(Progress) + Sync,
    ) -> Progress {
        let next = AtomicUsize::new(0);
        let progress = Mutex::new(Progress { total: missing.len(), ..Progress::default() });
        report(*progress.lock().unwrap());
        std::thread::scope(|scope| {
            for _ in 0..threads.max(1) {
                scope.spawn(|| {
                    while !stop.load(Ordering::Relaxed) {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(id) = missing.get(i) else { break };
                        let made = matches!(self.make(source, id), Ok(Some(_)));
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
        progress.into_inner().unwrap()
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
