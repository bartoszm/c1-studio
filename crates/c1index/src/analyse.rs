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
    /// Photos measured whose framing was not kept: their render was measured again
    /// meanwhile, or they left the library.
    pub unframed: usize,
    pub millis: u128,
}

fn stamp(path: &std::path::Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos() as u64;
    Some((meta.len(), mtime))
}

/// `work` done on each of `items`, on up to `threads` threads at once: what it gave, in no
/// order, and how many it gave nothing for.
fn each<T: Sync, R: Send>(items: &[T], threads: usize, work: impl Fn(&T) -> Option<R> + Sync) -> (Vec<R>, usize) {
    let next = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);
    let out = Mutex::new(Vec::with_capacity(items.len()));
    std::thread::scope(|scope| {
        for _ in 0..threads.clamp(1, items.len().max(1)) {
            scope.spawn(|| {
                while let Some(item) = items.get(next.fetch_add(1, Ordering::Relaxed)) {
                    match work(item) {
                        Some(done) => out.lock().unwrap().push(done),
                        None => {
                            failed.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            });
        }
    });
    (out.into_inner().unwrap(), failed.into_inner())
}

/// The render of a photo the analysis measures (the source's thumbnail, or one made for a
/// photo without), with its stamp; none for a photo of another source, or with no render.
fn render_of(library: &Library, source: &dyn PhotoSource, thumbs: &Thumbs, row: u64) -> Option<(ImageRef, (u64, u64))> {
    let id = library.photo_id(row)?;
    if id.source != source.info().id {
        return None;
    }
    let image = thumbs.get(source, &id).ok()??;
    let stamp = stamp(&image.path)?;
    Some((image, stamp))
}

/// Measures every photo not yet measured (or whose render changed), how it is framed too,
/// then works the duplicate groups and series out again. `stop` ends it between batches;
/// `report` hears after each batch.
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
    let mut todo: Vec<(u64, String, ImageRef, (u64, u64))> = Vec::new();
    for row in rows {
        let Some((image, stamp)) = render_of(library, source, thumbs, row) else { continue };
        let Some(key) = library.photo_id(row).map(|id| id.key()) else { continue };
        if analysed.get(&key) != Some(&stamp) {
            todo.push((row, key, image, stamp));
        }
    }
    let mut progress = Progress { total: todo.len(), ..Progress::default() };
    report(progress);
    for chunk in todo.chunks(BATCH) {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        // The render is read once for both: choosing the best of a group finds the framing kept.
        let (measured, failed) = each(chunk, threads, |(row, key, image, stamp)| {
            let pixels = c1image::load(image).ok()?;
            Some(((key.clone(), *stamp, c1analysis::analyse(&pixels)), (*row, *stamp, c1analysis::framing(&pixels))))
        });
        let (analyses, framings): (Vec<_>, Vec<_>) = measured.into_iter().unzip();
        let framed = framings.len();
        library.put_analyses(analyses)?;
        progress.unframed += framed - library.put_framings(framings)?;
        progress.done += chunk.len();
        progress.failed += failed;
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
/// now, from the render the analysis measures ([`Thumbs::get`]): photos measured before
/// framing was, or by another version of it. A photo not measured yet (one picked by hand,
/// say) is measured on the way: its analysis is kept too, without working the groups out
/// again. Returns the rows that still have no framing: their render could not be read.
pub fn frame(
    library: &Library,
    source: &dyn PhotoSource,
    thumbs: &Thumbs,
    rows: &[u64],
    threads: usize,
) -> anyhow::Result<Vec<u64>> {
    frame_with(library, rows, threads, &|row| {
        let (image, stamp) = render_of(library, source, thumbs, row)?;
        Some((stamp, c1image::load(&image).ok()?))
    })
}

/// [`frame`], each photo's render given by `load`.
pub fn frame_with(
    library: &Library,
    rows: &[u64],
    threads: usize,
    load: &(dyn Fn(u64) -> Option<Render> + Sync),
) -> anyhow::Result<Vec<u64>> {
    let have = library.framings(rows)?;
    let todo: Vec<u64> = rows.iter().copied().filter(|row| !have.contains_key(row)).collect();
    if todo.is_empty() {
        return Ok(todo);
    }
    let (measured, _) = each(&todo, threads, |&row| {
        let (stamp, pixels) = load(row)?;
        // Not measured, or measured of another render: a framing is kept only of the
        // render the analysis has, so that is measured first.
        let analysis = library
            .look(row)
            .is_none_or(|look| look.stamp != stamp)
            .then(|| library.photo_id(row).map(|id| (id.key(), stamp, c1analysis::analyse(&pixels))))
            .flatten();
        Some((analysis, (row, stamp, c1analysis::framing(&pixels))))
    });
    let (analyses, framings): (Vec<_>, Vec<_>) = measured.into_iter().unzip();
    let analyses: Vec<_> = analyses.into_iter().flatten().collect();
    if !analyses.is_empty() {
        library.put_analyses(analyses)?;
    }
    // How many were kept is not asked: the rows left without a framing are read below.
    library.put_framings(framings)?;
    // One with no render to read, or measured again meanwhile.
    let have = library.framings(&todo)?;
    Ok(todo.into_iter().filter(|row| !have.contains_key(row)).collect())
}

/// Worker threads that leave a core for everything else.
pub fn default_threads() -> usize {
    std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(1).max(1))
}
