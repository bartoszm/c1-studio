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

/// A photo's render as the analysis measures it: its stamp and its pixels.
pub type Render = ((u64, u64), c1image::RgbImage);

/// How the photos at `rows` are framed, measured where no framing is kept for their render
/// now, from the render the analysis measures ([`Thumbs::get`]). A photo not measured yet
/// (one picked by hand, say) is measured on the way: its analysis is kept too, without
/// working the groups out again. Returns how many could not be read.
pub fn frame(
    library: &Library,
    source: &dyn PhotoSource,
    thumbs: &Thumbs,
    rows: &[u64],
    threads: usize,
) -> anyhow::Result<usize> {
    frame_with(library, rows, threads, &|row| {
        let id = library.photo_id(row)?;
        if id.source != source.info().id {
            return None;
        }
        let image = thumbs.get(source, &id).ok()??;
        let stamp = stamp(&image.path)?;
        Some((stamp, c1image::load(&image).ok()?))
    })
}

/// [`frame`], each photo's render given by `load`.
pub fn frame_with(
    library: &Library,
    rows: &[u64],
    threads: usize,
    load: &(dyn Fn(u64) -> Option<Render> + Sync),
) -> anyhow::Result<usize> {
    let have = library.framings(rows)?;
    let todo: Vec<u64> = rows.iter().copied().filter(|row| !have.contains_key(row)).collect();
    if todo.is_empty() {
        return Ok(0);
    }
    let next = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);
    let analyses = Mutex::new(Vec::new());
    let framings = Mutex::new(Vec::with_capacity(todo.len()));
    std::thread::scope(|scope| {
        for _ in 0..threads.clamp(1, todo.len()) {
            scope.spawn(|| {
                while let Some(&row) = todo.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let Some((stamp, pixels)) = load(row) else {
                        failed.fetch_add(1, Ordering::Relaxed);
                        continue;
                    };
                    // Not measured, or measured of another render: the framing is stamped
                    // with the render the analysis has, so that is measured first.
                    if library.look(row).is_none_or(|look| look.stamp != stamp)
                        && let Some(id) = library.photo_id(row)
                    {
                        analyses.lock().unwrap().push((id.key(), stamp, c1analysis::analyse(&pixels)));
                    }
                    framings.lock().unwrap().push((row, c1analysis::framing(&pixels)));
                }
            });
        }
    });
    let analyses = analyses.into_inner().unwrap();
    if !analyses.is_empty() {
        library.put_analyses(analyses)?;
    }
    let framings = framings.into_inner().unwrap();
    let made = framings.len();
    let kept = library.put_framings(framings)?;
    Ok(failed.into_inner() + made - kept)
}

/// Worker threads that leave a core for everything else.
pub fn default_threads() -> usize {
    std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(1).max(1))
}
