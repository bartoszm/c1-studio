//! Checks every [`PhotoSource`] implementation must pass.
//!
//! Each check returns the list of violations it found; empty means conformant. A source
//! crate calls these from its own tests against a real (or fixture) instance.

use std::collections::HashSet;

use crate::change::{Change, Op};
use crate::photo::{Photo, PhotoId};
use crate::source::{Cursor, Field, ImageTier, PhotoSource, Removal, SourceError, SourceEvent, WriteStatus};

/// Collects a full snapshot, checking event order along the way.
pub fn collect(source: &dyn PhotoSource) -> Result<(Vec<Photo>, Vec<String>), SourceError> {
    let mut photos = Vec::new();
    let mut problems = Vec::new();
    let mut began = false;
    let mut ended = false;
    source.snapshot(None, &mut |event| {
        if ended {
            problems.push("event after End".to_owned());
        }
        match event {
            SourceEvent::BeginFull | SourceEvent::BeginIncremental => {
                if began {
                    problems.push("Begin sent twice".to_owned());
                }
                began = true;
            }
            _ if !began => problems.push("event before Begin".to_owned()),
            SourceEvent::Upsert(photo) => photos.push(*photo),
            SourceEvent::End(_) => ended = true,
            _ => {}
        }
        Ok(())
    })?;
    if !ended {
        problems.push("snapshot did not send End".to_owned());
    }
    Ok((photos, problems))
}

/// Snapshot shape, id hygiene and image lookups.
pub fn check_read(source: &dyn PhotoSource) -> Vec<String> {
    let (photos, mut problems) = match collect(source) {
        Ok(result) => result,
        Err(e) => return vec![format!("snapshot failed: {e}")],
    };
    let me = source.info().id;
    let mut seen = HashSet::new();
    for photo in &photos {
        if photo.id.source != me {
            problems.push(format!("{} carries another source's id", photo.id));
        }
        if !seen.insert(photo.id.clone()) {
            problems.push(format!("{} sent twice", photo.id));
        }
        if let Some(rating) = photo.rating
            && !(1..=5).contains(&rating)
        {
            problems.push(format!("{} has rating {rating}; use None for unrated", photo.id));
        }
    }
    for tier in source.capabilities().tiers {
        for photo in photos.iter().take(5) {
            if let Err(e) = source.image(&photo.id, tier) {
                problems.push(format!("image({}, {tier:?}) failed: {e}", photo.id));
            }
        }
    }
    problems
}

/// Round-trips a rating change on `probe`, then checks conflict detection. Leaves the
/// photo as it found it. Skipped (no problems) when the source cannot write ratings.
pub fn check_write(source: &dyn PhotoSource, probe: &PhotoId) -> Vec<String> {
    if !source.capabilities().can_write(Field::Rating) || source.write_status() != WriteStatus::Ready {
        return Vec::new();
    }
    let mut problems = Vec::new();
    let current =
        |source: &dyn PhotoSource| -> Option<Photo> { collect(source).ok()?.0.into_iter().find(|p| &p.id == probe) };
    let Some(before) = current(source) else {
        return vec![format!("probe {probe} not in snapshot")];
    };
    let target = if before.rating == Some(5) { Some(4) } else { Some(5) };
    let set = |base, value| Change { photo: probe.clone(), op: Op::SetRating { base, value } };

    // 1. A plain write lands.
    match source.plan(&[set(before.rating, target)]) {
        Ok(plan) if plan.apply.len() == 1 => {
            if let Err(e) = source.apply(&plan) {
                problems.push(format!("apply failed: {e}"));
            }
        }
        Ok(plan) => problems.push(format!("expected 1 change to apply, plan was {plan:?}")),
        Err(e) => problems.push(format!("plan failed: {e}")),
    }
    if current(source).map(|p| p.rating) != Some(target) {
        problems.push("rating did not change after apply".to_owned());
    }

    // 2. Writing the same value again is a no-op, not a conflict.
    match source.plan(&[set(before.rating, target)]) {
        Ok(plan) if plan.no_ops.len() == 1 => {}
        other => problems.push(format!("re-plan of an applied change should be a no-op: {other:?}")),
    }

    // 3. A change based on a value the source no longer holds is a conflict.
    // The source now holds `target` (4 or 5); claim it held 1 and ask for 2.
    match source.plan(&[set(Some(1), Some(2))]) {
        Ok(plan) if plan.conflicts.len() == 1 => {}
        other => problems.push(format!("stale base should conflict: {other:?}")),
    }

    // Restore.
    if let Ok(plan) = source.plan(&[set(target, before.rating)]) {
        let _ = source.apply(&plan);
    }
    problems
}

/// Incremental snapshots: asked again with the cursor it just gave, an unchanged source
/// sends `BeginIncremental` then `End`; after a rating write to `probe`, a snapshot from the
/// earlier cursor either starts over (`BeginFull`) or carries the probe and no other photo.
/// Leaves the probe as it found it. The write half is skipped when ratings cannot be written.
pub fn check_incremental(source: &dyn PhotoSource, probe: &PhotoId) -> Vec<String> {
    let mut problems = Vec::new();
    let run = |since: Option<&Cursor>| -> Result<Vec<SourceEvent>, SourceError> {
        let mut events = Vec::new();
        source.snapshot(since, &mut |e| {
            events.push(e);
            Ok(())
        })?;
        Ok(events)
    };
    let end = |events: &[SourceEvent]| match events.last() {
        Some(SourceEvent::End(c)) => Some(c.clone()),
        _ => None,
    };
    let first = match run(None) {
        Ok(events) => events,
        Err(e) => return vec![format!("snapshot failed: {e}")],
    };
    let Some(cursor) = end(&first) else {
        return vec!["snapshot did not end with End".to_owned()];
    };
    match run(Some(&cursor)) {
        Ok(events) if matches!(events.as_slice(), [SourceEvent::BeginIncremental, SourceEvent::End(_)]) => {}
        Ok(events) => problems.push(format!("unchanged source should send BeginIncremental, End; sent {events:?}")),
        Err(e) => problems.push(format!("incremental snapshot failed: {e}")),
    }
    if !source.capabilities().can_write(Field::Rating) || source.write_status() != WriteStatus::Ready {
        return problems;
    }
    let Some(before) = first.iter().find_map(|e| match e {
        SourceEvent::Upsert(p) if &p.id == probe => Some(p.rating),
        _ => None,
    }) else {
        problems.push(format!("probe {probe} not in snapshot"));
        return problems;
    };
    let target = if before == Some(5) { Some(4) } else { Some(5) };
    let set = |base, value| Change { photo: probe.clone(), op: Op::SetRating { base, value } };
    if let Err(e) = source.plan(&[set(before, target)]).and_then(|plan| source.apply(&plan)) {
        problems.push(format!("write failed: {e}"));
        return problems;
    }
    match run(Some(&cursor)) {
        Ok(events) => {
            let full = matches!(events.first(), Some(SourceEvent::BeginFull));
            let upserts: Vec<&Photo> = events
                .iter()
                .filter_map(|e| match e {
                    SourceEvent::Upsert(p) => Some(&**p),
                    _ => None,
                })
                .collect();
            match upserts.iter().find(|p| &p.id == probe) {
                Some(p) if p.rating != target => problems.push("changed probe sent with the old rating".to_owned()),
                Some(_) => {}
                None => problems.push("changed probe missing from the snapshot after a write".to_owned()),
            }
            if !full && upserts.iter().any(|p| &p.id != probe) {
                problems.push("incremental snapshot sent photos that did not change".to_owned());
            }
            if !full && events.iter().any(|e| matches!(e, SourceEvent::Delete(_))) {
                problems.push("incremental snapshot deleted photos after a rating write".to_owned());
            }
        }
        Err(e) => problems.push(format!("snapshot after a write failed: {e}")),
    }
    if let Ok(plan) = source.plan(&[set(target, before)]) {
        let _ = source.apply(&plan);
    }
    problems
}

/// Backups: after a rating write to `probe`, restoring the newest backup made before it
/// brings the old rating back, and the restore itself returns a backup of the state it
/// replaced. Skipped (no problems) when the source keeps no backups or cannot write ratings.
/// Leaves the probe as it found it.
pub fn check_backup(source: &dyn PhotoSource, probe: &PhotoId) -> Vec<String> {
    if !source.capabilities().can_write(Field::Rating) || source.write_status() != WriteStatus::Ready {
        return Vec::new();
    }
    let rating = |source: &dyn PhotoSource| -> Option<Option<u8>> {
        collect(source).ok()?.0.into_iter().find(|p| &p.id == probe).map(|p| p.rating)
    };
    let Some(before) = rating(source) else {
        return vec![format!("probe {probe} not in snapshot")];
    };
    let target = if before == Some(5) { Some(4) } else { Some(5) };
    let set = |base, value| Change { photo: probe.clone(), op: Op::SetRating { base, value } };
    let report = match source.plan(&[set(before, target)]).and_then(|plan| source.apply(&plan)) {
        Ok(report) => report,
        Err(e) => return vec![format!("write failed: {e}")],
    };
    let backups = match source.backups() {
        Ok(b) => b,
        Err(e) => return vec![format!("backups() failed: {e}")],
    };
    let mut problems = Vec::new();
    if backups.is_empty() {
        if report.backup.is_some() {
            problems.push("apply reported a backup that backups() does not list".to_owned());
        }
        if let Ok(plan) = source.plan(&[set(target, before)]) {
            let _ = source.apply(&plan);
        }
        return problems;
    }
    if backups.windows(2).any(|w| w[0].created_ms < w[1].created_ms) {
        problems.push("backups() is not newest first".to_owned());
    }
    match source.restore(&backups[0].id) {
        Ok(undo) => {
            if rating(source) != Some(before) {
                problems.push("restoring the newest backup did not bring the old rating back".to_owned());
            }
            match source.backups() {
                Ok(list) if list.iter().any(|b| b.id == undo.id) => {}
                Ok(_) => problems.push("the backup made before restoring is not listed".to_owned()),
                Err(e) => problems.push(format!("backups() failed after restore: {e}")),
            }
        }
        Err(e) => {
            problems.push(format!("restore failed: {e}"));
            if let Ok(plan) = source.plan(&[set(target, before)]) {
                let _ = source.apply(&plan);
            }
        }
    }
    problems
}

/// Capabilities match what the source does: [`PhotoSource::rescan`] succeeds exactly when
/// `capabilities().rescan` says it can, and [`PhotoSource::progress`] is `None` once a
/// snapshot has finished. A source that can rescan is asked to, so its next snapshot reads
/// its files again (and is then taken).
pub fn check_capabilities(source: &dyn PhotoSource) -> Vec<String> {
    let mut problems = Vec::new();
    let can = source.capabilities().rescan;
    match (can, source.rescan()) {
        (true, Err(e)) => problems.push(format!("capabilities().rescan is true, but rescan() failed: {e}")),
        (false, Ok(())) => problems.push("rescan() succeeded, but capabilities().rescan is false".to_owned()),
        (false, Err(SourceError::Unsupported(_))) | (true, Ok(())) => {}
        (false, Err(e)) => problems.push(format!("rescan() without the capability is not Unsupported: {e}")),
    }
    if let Err(e) = collect(source) {
        problems.push(format!("snapshot failed: {e}"));
    }
    if let Some(p) = source.progress() {
        problems.push(format!("progress() after the snapshot ended: {p:?}"));
    }
    problems
}

/// Deleting for good, which takes `probe` for good (use a copy): [`PhotoSource::remove`]
/// is `Unsupported` exactly when `capabilities().removes` is [`Removal::None`]; an id the
/// source does not have lands in `failed`, not in an error; and the next snapshot drops
/// `probe` ([`Removal::Files`]) or sends it trashed, with every variant of its asset
/// ([`Removal::MarksTrashed`]).
pub fn check_remove(source: &dyn PhotoSource, probe: &PhotoId) -> Vec<String> {
    let mut problems = Vec::new();
    let removes = source.capabilities().removes;
    let stranger = PhotoId::new(&source.info().id, "no-such-photo");
    let report = match (removes, source.remove(std::slice::from_ref(&stranger))) {
        (Removal::None, Err(SourceError::Unsupported(_))) => return problems,
        (Removal::None, other) => {
            return vec![format!("remove() without capabilities().removes is not Unsupported: {other:?}")];
        }
        (_, Err(e)) => return vec![format!("remove() of an unknown photo failed: {e}")],
        (_, Ok(report)) => report,
    };
    if !report.removed.is_empty() || report.failed.iter().map(|(id, _)| id).ne([&stranger]) {
        problems.push(format!("an unknown photo should be the only one failed: {report:?}"));
    }
    let (before, _) = match collect(source) {
        Ok(result) => result,
        Err(e) => return vec![format!("snapshot failed: {e}")],
    };
    let Some(asset) = before.iter().find(|p| &p.id == probe).map(|p| p.asset.clone()) else {
        return vec![format!("{probe} is not in the source")];
    };
    match source.remove(std::slice::from_ref(probe)) {
        Ok(report) if report.removed != [probe.clone()] => {
            problems.push(format!("remove() did not report {probe} removed: {report:?}"));
        }
        Ok(_) => {}
        Err(e) => return vec![format!("remove() failed: {e}")],
    }
    let (after, _) = match collect(source) {
        Ok(result) => result,
        Err(e) => return vec![format!("snapshot after remove failed: {e}")],
    };
    match removes {
        Removal::Files if after.iter().any(|p| &p.id == probe) => {
            problems.push(format!("{probe} is still there after remove()"));
        }
        Removal::MarksTrashed => {
            for photo in after.iter().filter(|p| p.asset == asset && !p.trashed) {
                problems.push(format!("{} is not trashed after remove() of {probe}", photo.id));
            }
        }
        _ => {}
    }
    problems
}

/// Images of `probe`: `has_image` agrees with `image` for every tier; a source that
/// does not make images returns `Ok(None)` from `make_image`; one that does makes a
/// thumbnail and a preview when the original is there, and finds that very image with
/// `image` and `image_or_make`.
pub fn check_images(source: &dyn PhotoSource, probe: &PhotoId) -> Vec<String> {
    let mut problems = Vec::new();
    let makes = source.capabilities().makes_images;
    // A source that makes images makes them of any photo it has the original of.
    let original = matches!(source.image(probe, ImageTier::Original), Ok(Some(_)));
    for tier in [ImageTier::Thumb, ImageTier::Preview, ImageTier::Original] {
        match (source.image(probe, tier), source.has_image(probe, tier)) {
            (Ok(image), Ok(has)) if image.is_some() != has => {
                problems.push(format!("{tier:?}: has_image is {has}, image is {image:?}"));
            }
            (Err(e), _) | (_, Err(e)) => problems.push(format!("{tier:?}: lookup failed: {e}")),
            _ => {}
        }
        if tier == ImageTier::Original {
            continue;
        }
        match source.make_image(probe, tier) {
            Ok(Some(made)) if !makes => {
                problems.push(format!("{tier:?}: made {} without capabilities().makes_images", made.path.display()));
            }
            Ok(Some(made)) => {
                let found = source.image(probe, tier).ok().flatten().map(|r| r.path);
                if found.as_ref() != Some(&made.path) {
                    problems.push(format!("{tier:?}: made {}, but image() finds {found:?}", made.path.display()));
                }
                let shown = source.image_or_make(probe, tier).ok().flatten().map(|r| r.path);
                if shown.as_ref() != Some(&made.path) {
                    problems
                        .push(format!("{tier:?}: made {}, but image_or_make() gives {shown:?}", made.path.display()));
                }
            }
            Ok(None) if makes && original => {
                problems.push(format!("{tier:?}: makes_images, the original is there, but make_image made none"));
            }
            Ok(None) => {}
            Err(e) if !makes => problems.push(format!("{tier:?}: make_image without the capability failed: {e}")),
            Err(e) => problems.push(format!("{tier:?}: make_image failed: {e}")),
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemorySource;
    use crate::photo::*;

    fn photo(source: &SourceId, n: u32) -> Photo {
        Photo {
            id: PhotoId::new(source, n.to_string()),
            asset: AssetId(n.to_string()),
            variant_index: 0,
            file: FileInfo { name: format!("DSCF{n:04}.RAF"), ext: "raf".into(), ..Default::default() },
            captured_at: Some(1_700_000_000_000 + i64::from(n) * 86_400_000),
            imported_at: None,
            exif: Exif::default(),
            rating: if n.is_multiple_of(2) { Some(3) } else { None },
            color: None,
            keywords: vec![KeywordPath::from_pipe("street")],
            iptc: Iptc::default(),
            place: Place::default(),
            geo: None,
            collections: vec![],
            edit: EditInfo::default(),
            trashed: false,
        }
    }

    fn source() -> MemorySource {
        let id = SourceId("mem".into());
        MemorySource::new("mem", (1..=10).map(|n| photo(&id, n)).collect(), vec![])
    }

    #[test]
    fn memory_source_passes_read_checks() {
        assert_eq!(check_read(&source()), Vec::<String>::new());
    }

    #[test]
    fn memory_source_passes_write_checks() {
        let s = source();
        let probe = PhotoId::new(&SourceId("mem".into()), "2");
        assert_eq!(check_write(&s, &probe), Vec::<String>::new());
        assert_eq!(s.photo(&probe).unwrap().rating, Some(3), "probe restored");
    }

    #[test]
    fn memory_source_passes_incremental_and_backup_checks() {
        let s = source();
        let probe = PhotoId::new(&SourceId("mem".into()), "2");
        // MemorySource starts over after a write, which the check accepts.
        assert_eq!(check_incremental(&s, &probe), Vec::<String>::new());
        assert_eq!(check_backup(&s, &probe), Vec::<String>::new(), "keeps no backups: skipped");
        assert_eq!(s.photo(&probe).unwrap().rating, Some(3), "probe restored");
    }

    #[test]
    fn memory_source_passes_capability_and_image_checks() {
        let s = source();
        assert_eq!(check_capabilities(&s), Vec::<String>::new());
        assert_eq!(check_images(&s, &PhotoId::new(&SourceId("mem".into()), "2")), Vec::<String>::new());
    }

    #[test]
    fn memory_source_passes_remove_checks() {
        let probe = PhotoId::new(&SourceId("mem".into()), "2");
        for removal in [Removal::None, Removal::Files, Removal::MarksTrashed] {
            let s = source();
            s.set_removal(removal);
            assert_eq!(check_remove(&s, &probe), Vec::<String>::new(), "{removal:?}");
        }
    }

    #[test]
    fn unchanged_source_answers_incrementally() {
        let s = source();
        let mut cursor = None;
        s.snapshot(None, &mut |e| {
            if let SourceEvent::End(c) = e {
                cursor = Some(c);
            }
            Ok(())
        })
        .unwrap();
        let mut events = Vec::new();
        s.snapshot(cursor.as_ref(), &mut |e| {
            events.push(e);
            Ok(())
        })
        .unwrap();
        assert!(matches!(events.as_slice(), [SourceEvent::BeginIncremental, SourceEvent::End(_)]));
    }

    #[test]
    fn locked_source_refuses_to_apply() {
        let s = source();
        let probe = PhotoId::new(&SourceId("mem".into()), "2");
        let plan = s.plan(&[Change { photo: probe, op: Op::SetRating { base: Some(3), value: Some(5) } }]).unwrap();
        s.set_locked(Some("CaptureOne.exe"));
        assert!(matches!(s.apply(&plan), Err(SourceError::Locked { .. })));
    }

    #[test]
    fn external_edit_makes_plan_stale() {
        let s = source();
        let probe = PhotoId::new(&SourceId("mem".into()), "2");
        let plan =
            s.plan(&[Change { photo: probe.clone(), op: Op::SetRating { base: Some(3), value: Some(5) } }]).unwrap();
        s.edit_externally(&probe, |p| p.rating = Some(1));
        assert!(matches!(s.apply(&plan), Err(SourceError::StalePlan)));
    }

    #[test]
    fn color_index_round_trips() {
        for c in ColorLabel::ALL {
            assert_eq!(ColorLabel::from_index(c.index()), Some(c));
        }
        assert_eq!(ColorLabel::from_index(0), None);
    }
}
