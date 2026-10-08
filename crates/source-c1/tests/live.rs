//! Against the real catalog. Read-only; run with `cargo test -p source-c1 -- --ignored --nocapture`.

use std::time::Instant;

use c1model::{PhotoSource, SourceEvent, conformance};
use source_c1::{CaptureOne, data_dir, find_catalog};

fn open() -> Option<CaptureOne> {
    let catalog = find_catalog(None).map_err(|e| eprintln!("skipped: {e}")).ok()?;
    Some(CaptureOne::open(&catalog, data_dir(&catalog)).expect("catalog opens"))
}

#[test]
#[ignore = "needs the live catalog"]
fn live_snapshot_conforms() {
    let Some(source) = open() else { return eprintln!("no catalog, skipped") };
    let started = Instant::now();
    let problems = conformance::check_read(&source);
    eprintln!("check_read took {:.1}s", started.elapsed().as_secs_f64());
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
#[ignore = "needs the live catalog"]
fn live_snapshot_stats() {
    let Some(source) = open() else { return eprintln!("no catalog, skipped") };
    let started = Instant::now();
    let (mut photos, mut collections, mut keywords, mut cursor) = (0usize, 0usize, 0usize, None);
    let (mut rated, mut keyworded, mut geo, mut trashed) = (0, 0, 0, 0);
    source
        .snapshot(None, &mut |event| {
            match event {
                SourceEvent::Upsert(p) => {
                    photos += 1;
                    rated += usize::from(p.rating.is_some());
                    keyworded += usize::from(!p.keywords.is_empty());
                    geo += usize::from(p.geo.is_some());
                    trashed += usize::from(p.trashed);
                }
                SourceEvent::Collections(c) => collections = c.len(),
                SourceEvent::Keywords(k) => keywords = k.len(),
                SourceEvent::End(c) => cursor = Some(c),
                _ => {}
            }
            Ok(())
        })
        .unwrap();
    eprintln!(
        "{} · {photos} photos ({rated} rated, {keyworded} keyworded, {geo} with GPS, {trashed} trashed), \
         {collections} collections, {keywords} keywords in {:.1}s",
        source.info().name,
        started.elapsed().as_secs_f64()
    );
    assert!(photos > 0);

    // Unchanged catalog: the second snapshot is incremental and empty.
    let mut events = Vec::new();
    source
        .snapshot(cursor.as_ref(), &mut |e| {
            events.push(e);
            Ok(())
        })
        .unwrap();
    assert!(matches!(events.as_slice(), [SourceEvent::BeginIncremental, SourceEvent::End(_)]));
}
