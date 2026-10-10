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

impl Op {
    /// Compares `base` with `photo` and decides what applying this op would mean.
    pub fn check(&self, photo: &Photo) -> Check {
        fn compare<T: PartialEq + std::fmt::Debug>(base: &T, value: &T, current: &T) -> Check {
            if current == value {
                Check::NoOp
            } else if current != base {
                Check::Conflict(format!("{current:?}"))
            } else {
                Check::Apply
            }
        }
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
