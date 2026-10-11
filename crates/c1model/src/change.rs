//! Edits flowing from the app back into a source.
//!
//! Every [`Op`] carries the value the app last saw (`base`). A source compares it with
//! what it holds now; a mismatch means someone else changed the field meanwhile and is
//! reported as a [`Conflict`] instead of being overwritten.

use serde::{Deserialize, Serialize};

use crate::photo::{CollectionId, ColorLabel, GeoPoint, Iptc, KeywordPath, Photo, PhotoId, Place};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Op {
    SetRating { base: Option<u8>, value: Option<u8> },
    SetColor { base: Option<ColorLabel>, value: Option<ColorLabel> },
    AddKeywords { paths: Vec<KeywordPath> },
    RemoveKeywords { paths: Vec<KeywordPath> },
    SetIptc { base: Iptc, value: Iptc },
    SetPlace { base: Place, value: Place },
    SetGeo { base: Option<GeoPoint>, value: Option<GeoPoint> },
    AddToCollection { collection: CollectionId },
    RemoveFromCollection { collection: CollectionId },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub photo: PhotoId,
    pub op: Op,
}

/// Outcome of checking one change against a photo's current state.
pub enum Check {
    Apply,
    NoOp,
    Conflict(String),
}

/// Checks a set-field op: the field already holds `value` (nothing to do), still holds
/// `base` (apply), or holds something else (someone changed it meanwhile).
pub fn compare<T: PartialEq + std::fmt::Debug>(base: &T, value: &T, current: &T) -> Check {
    if current == value {
        Check::NoOp
    } else if current != base {
        Check::Conflict(format!("{current:?}"))
    } else {
        Check::Apply
    }
}

impl Op {
    /// Compares `base` with `photo` and decides what applying this op would mean.
    pub fn check(&self, photo: &Photo) -> Check {
        match self {
            Op::SetRating { base, value } => compare(base, value, &photo.rating),
            Op::SetColor { base, value } => compare(base, value, &photo.color),
            Op::SetIptc { base, value } => compare(base, value, &photo.iptc),
            Op::SetPlace { base, value } => compare(base, value, &photo.place),
            Op::SetGeo { base, value } => compare(base, value, &photo.geo),
            Op::AddKeywords { paths } => {
                if paths.iter().all(|p| photo.keywords.contains(p)) {
                    Check::NoOp
                } else {
                    Check::Apply
                }
            }
            Op::RemoveKeywords { paths } => {
                if paths.iter().any(|p| photo.keywords.contains(p)) {
                    Check::Apply
                } else {
                    Check::NoOp
                }
            }
            Op::AddToCollection { collection } => {
                if photo.collections.contains(collection) {
                    Check::NoOp
                } else {
                    Check::Apply
                }
            }
            Op::RemoveFromCollection { collection } => {
                if photo.collections.contains(collection) {
                    Check::Apply
                } else {
                    Check::NoOp
                }
            }
        }
    }

    /// Applies the op to an in-memory photo. Sources that store photos differently
    /// translate ops themselves; this is the reference behaviour they must match.
    pub fn apply_to(&self, photo: &mut Photo) {
        match self {
            Op::SetRating { value, .. } => photo.rating = *value,
            Op::SetColor { value, .. } => photo.color = *value,
            Op::SetIptc { value, .. } => photo.iptc = value.clone(),
            Op::SetPlace { value, .. } => photo.place = value.clone(),
            Op::SetGeo { value, .. } => photo.geo = *value,
            Op::AddKeywords { paths } => {
                for path in paths {
                    if !photo.keywords.contains(path) {
                        photo.keywords.push(path.clone());
                    }
                }
            }
            Op::RemoveKeywords { paths } => photo.keywords.retain(|k| !paths.contains(k)),
            Op::AddToCollection { collection } => {
                if !photo.collections.contains(collection) {
                    photo.collections.push(collection.clone());
                }
            }
            Op::RemoveFromCollection { collection } => photo.collections.retain(|c| c != collection),
        }
    }
}

impl Op {
    /// What a queued edit is about: a field (`rating`, `color`, ...) or one keyword
    /// (`keyword:best|best2018`). Two edits with the same key on the same photo merge into
    /// one entry. `None` for ops that need [`Op::split`] first, and for collections.
    pub fn queue_key(&self) -> Option<String> {
        match self {
            Op::SetRating { .. } => Some("rating".into()),
            Op::SetColor { .. } => Some("color".into()),
            Op::SetIptc { .. } => Some("iptc".into()),
            Op::SetPlace { .. } => Some("place".into()),
            Op::SetGeo { .. } => Some("geo".into()),
            Op::AddKeywords { paths } | Op::RemoveKeywords { paths } if paths.len() == 1 => {
                Some(format!("keyword:{}", paths[0].to_pipe()))
            }
            _ => None,
        }
    }

    /// Keyword ops one path each; other ops as they are.
    pub fn split(&self) -> Vec<Op> {
        match self {
            Op::AddKeywords { paths } => paths.iter().map(|p| Op::AddKeywords { paths: vec![p.clone()] }).collect(),
            Op::RemoveKeywords { paths } => {
                paths.iter().map(|p| Op::RemoveKeywords { paths: vec![p.clone()] }).collect()
            }
            other => vec![other.clone()],
        }
    }

    /// The same set-field op with `value` replaced by what `photo` holds for the field.
    /// Other ops come back unchanged.
    pub fn valued_from(&self, photo: &Photo) -> Op {
        match self {
            Op::SetRating { base, .. } => Op::SetRating { base: *base, value: photo.rating },
            Op::SetColor { base, .. } => Op::SetColor { base: *base, value: photo.color },
            Op::SetIptc { base, .. } => Op::SetIptc { base: base.clone(), value: photo.iptc.clone() },
            Op::SetPlace { base, .. } => Op::SetPlace { base: base.clone(), value: photo.place.clone() },
            Op::SetGeo { base, .. } => Op::SetGeo { base: *base, value: photo.geo },
            other => other.clone(),
        }
    }

    /// The same set-field op with `base` replaced by what `photo` holds for the field.
    pub fn based_on(&self, photo: &Photo) -> Op {
        match self {
            Op::SetRating { value, .. } => Op::SetRating { base: photo.rating, value: *value },
            Op::SetColor { value, .. } => Op::SetColor { base: photo.color, value: *value },
            Op::SetIptc { value, .. } => Op::SetIptc { base: photo.iptc.clone(), value: value.clone() },
            Op::SetPlace { value, .. } => Op::SetPlace { base: photo.place.clone(), value: value.clone() },
            Op::SetGeo { value, .. } => Op::SetGeo { base: photo.geo, value: *value },
            other => other.clone(),
        }
    }

    /// This op's base with `to`'s value, when both set the same field; otherwise `to`.
    pub fn retarget(&self, to: &Op) -> Op {
        match (self, to) {
            (Op::SetRating { base, .. }, Op::SetRating { value, .. }) => Op::SetRating { base: *base, value: *value },
            (Op::SetColor { base, .. }, Op::SetColor { value, .. }) => Op::SetColor { base: *base, value: *value },
            (Op::SetIptc { base, .. }, Op::SetIptc { value, .. }) => {
                Op::SetIptc { base: base.clone(), value: value.clone() }
            }
            (Op::SetPlace { base, .. }, Op::SetPlace { value, .. }) => {
                Op::SetPlace { base: base.clone(), value: value.clone() }
            }
            (Op::SetGeo { base, .. }, Op::SetGeo { value, .. }) => Op::SetGeo { base: *base, value: *value },
            _ => to.clone(),
        }
    }

    /// The op that undoes this one: base and value swapped, keywords and collections
    /// added instead of removed and the other way round.
    pub fn reverted(&self) -> Op {
        match self {
            Op::SetRating { base, value } => Op::SetRating { base: *value, value: *base },
            Op::SetColor { base, value } => Op::SetColor { base: *value, value: *base },
            Op::SetIptc { base, value } => Op::SetIptc { base: value.clone(), value: base.clone() },
            Op::SetPlace { base, value } => Op::SetPlace { base: value.clone(), value: base.clone() },
            Op::SetGeo { base, value } => Op::SetGeo { base: *value, value: *base },
            Op::AddKeywords { paths } => Op::RemoveKeywords { paths: paths.clone() },
            Op::RemoveKeywords { paths } => Op::AddKeywords { paths: paths.clone() },
            Op::AddToCollection { collection } => Op::RemoveFromCollection { collection: collection.clone() },
            Op::RemoveFromCollection { collection } => Op::AddToCollection { collection: collection.clone() },
        }
    }

    /// A set-field op whose value is its base changes nothing.
    pub fn is_identity(&self) -> bool {
        match self {
            Op::SetRating { base, value } => base == value,
            Op::SetColor { base, value } => base == value,
            Op::SetIptc { base, value } => base == value,
            Op::SetPlace { base, value } => base == value,
            Op::SetGeo { base, value } => base == value,
            Op::AddKeywords { paths } | Op::RemoveKeywords { paths } => paths.is_empty(),
            _ => false,
        }
    }
}

/// What changed between two versions of a photo, as ops that turn `old` into `new`:
/// rating, colour, keywords and collections, the fields a history is kept of.
pub fn diff(old: &Photo, new: &Photo) -> Vec<Op> {
    let mut ops = Vec::new();
    if old.rating != new.rating {
        ops.push(Op::SetRating { base: old.rating, value: new.rating });
    }
    if old.color != new.color {
        ops.push(Op::SetColor { base: old.color, value: new.color });
    }
    let added: Vec<_> = new.keywords.iter().filter(|k| !old.keywords.contains(k)).cloned().collect();
    let removed: Vec<_> = old.keywords.iter().filter(|k| !new.keywords.contains(k)).cloned().collect();
    if !added.is_empty() {
        ops.push(Op::AddKeywords { paths: added });
    }
    if !removed.is_empty() {
        ops.push(Op::RemoveKeywords { paths: removed });
    }
    for c in new.collections.iter().filter(|c| !old.collections.contains(c)) {
        ops.push(Op::AddToCollection { collection: c.clone() });
    }
    for c in old.collections.iter().filter(|c| !new.collections.contains(c)) {
        ops.push(Op::RemoveFromCollection { collection: c.clone() });
    }
    ops
}

/// Sorts changes into a plan using [`Op::check`]. `lookup` returns the current photo.
pub fn plan_with<'a>(changes: &[Change], mut lookup: impl FnMut(&PhotoId) -> Option<&'a Photo>) -> ApplyPlan {
    let mut plan = ApplyPlan::default();
    for change in changes {
        match lookup(&change.photo) {
            None => plan.conflicts.push(Conflict { change: change.clone(), current: "deleted".into() }),
            Some(photo) => match change.op.check(photo) {
                Check::Apply => plan.apply.push(change.clone()),
                Check::NoOp => plan.no_ops.push(change.clone()),
                Check::Conflict(current) => plan.conflicts.push(Conflict { change: change.clone(), current }),
            },
        }
    }
    plan
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Conflict {
    pub change: Change,
    /// What the source holds now, rendered for display.
    pub current: String,
}

/// A dry run: what [`crate::PhotoSource::apply`] would do.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ApplyPlan {
    pub apply: Vec<Change>,
    pub conflicts: Vec<Conflict>,
    /// Changes that would not alter anything (value already equals the target).
    pub no_ops: Vec<Change>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ApplyReport {
    pub applied: usize,
    /// Where the source saved a backup before writing, if it does that.
    pub backup: Option<String>,
}
