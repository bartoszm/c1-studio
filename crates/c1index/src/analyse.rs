//! Measuring a library's photos (stage 3): each photo's small render, in parallel and in
//! batches, skipping photos whose render has not changed since it was measured.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Instant, UNIX_EPOCH};

use c1model::{ImageRef, PhotoSource};
use serde::Serialize;

use crate::filter::{Bin, Filter, Hidden, Trashed};
use crate::library::{Library, QueryOptions, Sort};
use crate::thumbs::Thumbs;

/// Photos measured between commits to the library.
const BATCH: usize = 1000;

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Progress {
    /// Photos measured so far in this run, of `total` to measure.
    pub done: usize,
    pub total: usize,
    /// Photos whose render could not be read.
    pub failed: usize,
    pub millis: u128,
}

fn stamp(path: &std::path::Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos() as u64;
    Some((meta.len(), mtime))
}

/// Measures every photo not yet measured (or whose render changed), then works the
/// duplicate groups and series out again. `stop` ends it between batches; `report`
/// hears after each batch.
pub fn run(
    library: &Library,
    source: &dyn PhotoSource,
    thumbs: &Thumbs,
    threads: usize,
    stop: &AtomicBool,
    mut report: impl FnMut(Progress),
) -> anyhow::Result<Progress> {
    let started = Instant::now();
    let analysed = library.analysed();
    let all = Filter { trashed: Trashed::Include, hidden: Hidden::Include, bin: Bin::Include, ..Filter::default() };
    let rows = library.query(&all, &QueryOptions { sort: Sort::Row, ..QueryOptions::default() })?.rows;
    let mut todo: Vec<(String, ImageRef, (u64, u64))> = Vec::new();
    for row in rows {
        let Some(id) = library.photo_id(row) else { continue };
        if id.source != source.info().id {
            continue;
        }
        // The source's thumbnail, or one made for a photo without.
        let Ok(Some(image)) = thumbs.get(source, &id) else { continue };
        let Some(stamp) = stamp(&image.path) else { continue };
        let key = id.key();
        if analysed.get(&key) != Some(&stamp) {
            todo.push((key, image, stamp));
        }
    }
    let mut progress = Progress { total: todo.len(), ..Progress::default() };
    report(progress);
    for chunk in todo.chunks(BATCH) {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let next = AtomicUsize::new(0);
        let failed = AtomicUsize::new(0);
        let out = Mutex::new(Vec::with_capacity(chunk.len()));
        std::thread::scope(|scope| {
            for _ in 0..threads.max(1) {
                scope.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some((key, image, stamp)) = chunk.get(i) else { break };
                        match c1image::load(image) {
                            Ok(pixels) => {
                                let a = c1analysis::analyse(&pixels);
                                out.lock().unwrap().push((key.clone(), *stamp, a));
                            }
                            Err(_) => {
                                failed.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                });
            }
        });
        library.put_analyses(out.into_inner().unwrap())?;
        progress.done += chunk.len();
        progress.failed += failed.into_inner();
        progress.millis = started.elapsed().as_millis();
        report(progress);
    }
    library.regroup()?;
    progress.millis = started.elapsed().as_millis();
    Ok(progress)
}

/// Worker threads that leave a core for everything else.
pub fn default_threads() -> usize {
    std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(1).max(1))
}
