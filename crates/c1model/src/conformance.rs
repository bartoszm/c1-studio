//! Checks every [`PhotoSource`] implementation must pass.
//!
//! Each check returns the list of violations it found; empty means conformant. A source
//! crate calls these from its own tests against a real (or fixture) instance.

use std::collections::HashSet;

use crate::change::{Change, Op};
use crate::photo::{Photo, PhotoId};
use crate::source::{Field, PhotoSource, SourceError, SourceEvent, WriteStatus};

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
