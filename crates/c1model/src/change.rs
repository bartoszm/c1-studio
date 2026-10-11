//! Edits flowing from the app back into a source.
//!
//! Every [`Op`] carries the value the app last saw (`base`). A source compares it with
//! what it holds now; a mismatch means someone else changed the field meanwhile and is
//! reported as a [`Conflict`] instead of being overwritten.

use serde::{Deserialize, Serialize};

use crate::photo::{
    Collection, CollectionId, CollectionKind, ColorLabel, GeoPoint, Iptc, KeywordPath, Photo, PhotoId, Place, SmartRule,
};

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

/// Where a geo point is, for comparing: two points at the same spot are the same
/// whatever their origin (a manual position reads back from a source as stored data).
pub fn spot(geo: &Option<GeoPoint>) -> Option<(f64, f64)> {
    geo.map(|g| (g.lat, g.lon))
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
            Op::SetGeo { base, value } => compare(&spot(base), &spot(value), &spot(&photo.geo)),
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
    /// (`keyword:best|best2018`) or album (`collection:906`). Two edits with the same key
    /// on the same photo merge into one entry. `None` for ops that need [`Op::split`] first.
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
            Op::AddToCollection { collection } | Op::RemoveFromCollection { collection } => {
                Some(format!("collection:{}", collection.0))
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
            Op::SetGeo { base, value } => spot(base) == spot(value),
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

#[cfg(test)]
mod collection_tests {
    use super::*;

    fn c(id: &str, name: &str, kind: CollectionKind, parent: Option<&str>) -> Collection {
        Collection {
            id: CollectionId(id.into()),
            name: name.into(),
            kind,
            parent: parent.map(|p| CollectionId(p.into())),
            path: String::new(),
            rule: (kind == CollectionKind::Smart).then(|| SmartRule { match_all: true, conditions: vec![] }),
            system: false,
        }
    }

    fn id(s: &str) -> CollectionId {
        CollectionId(s.into())
    }

    #[test]
    fn changes_apply_and_paths_follow() {
        let mut all =
            vec![c("1", "trips", CollectionKind::Group, None), c("2", "japan", CollectionKind::Album, Some("1"))];
        let new = CollectionId::new_local("u-1");
        assert_eq!(new.local_uuid(), Some("u-1"));
        let create = CollectionChange::Create {
            id: new.clone(),
            name: "tokyo".into(),
            kind: CollectionKind::Album,
            parent: Some(id("1")),
            rule: None,
        };
        assert_eq!(collection_problem(&all, &create), None);
        create.apply_to(&mut all);
        assert_eq!(all[2].path, "trips / tokyo");
        assert_eq!(create.label(&all), "New album “tokyo” in “trips”");
        CollectionChange::Rename { id: id("1"), name: "travel".into() }.apply_to(&mut all);
        assert_eq!(all[1].path, "travel / japan");
        assert_eq!(all[2].path, "travel / tokyo");
        CollectionChange::Move { id: new.clone(), parent: None }.apply_to(&mut all);
        assert_eq!(all[2].path, "tokyo");
        CollectionChange::Move { id: new.clone(), parent: Some(id("1")) }.apply_to(&mut all);
        // A group goes with everything in it.
        CollectionChange::Delete { id: id("1") }.apply_to(&mut all);
        assert!(all.is_empty());
    }

    #[test]
    fn what_cannot_be_done() {
        let all = vec![
            c("1", "trips", CollectionKind::Group, None),
            c("2", "japan", CollectionKind::Album, Some("1")),
            c("3", "best", CollectionKind::Smart, None),
        ];
        let create = |parent: Option<&str>, kind, rule| CollectionChange::Create {
            id: CollectionId::new_local("x"),
            name: "x".into(),
            kind,
            parent: parent.map(id),
            rule,
        };
        assert!(collection_problem(&all, &create(Some("2"), CollectionKind::Album, None)).is_some(), "not in an album");
        assert!(collection_problem(&all, &create(Some("9"), CollectionKind::Album, None)).is_some(), "parent gone");
        assert!(collection_problem(&all, &create(None, CollectionKind::Smart, None)).is_some(), "smart needs a rule");
        let rule = SmartRule { match_all: true, conditions: vec![] };
        assert!(collection_problem(&all, &create(None, CollectionKind::Smart, Some(rule.clone()))).is_none());
        assert!(collection_problem(&all, &CollectionChange::SetRule { id: id("2"), rule }).is_some());
        assert!(collection_problem(&all, &CollectionChange::Rename { id: id("2"), name: " ".into() }).is_some());
        assert!(collection_problem(&all, &CollectionChange::Delete { id: id("9") }).is_some());
        // Moving: not into itself or what is under it, not where it is.
        let mv = |what: &str, to: Option<&str>| CollectionChange::Move { id: id(what), parent: to.map(id) };
        assert!(collection_problem(&all, &mv("2", None)).is_none());
        assert!(collection_problem(&all, &mv("2", Some("1"))).is_some(), "there already");
        assert!(collection_problem(&all, &mv("1", Some("1"))).is_some(), "into itself");
        assert!(collection_problem(&all, &mv("3", Some("2"))).is_some(), "an album holds no albums");
        // Only albums hold photos.
        assert_eq!(membership_problem(&all, &id("2")), None);
        assert!(membership_problem(&all, &id("3")).unwrap().contains("smart album"));
        assert!(membership_problem(&all, &id("1")).is_some());
        assert!(membership_problem(&all, &id("9")).is_some());
    }

    #[test]
    fn membership_edits_queue_by_album() {
        let add = Op::AddToCollection { collection: id("7") };
        assert_eq!(add.queue_key().as_deref(), Some("collection:7"));
        assert_eq!(add.reverted(), Op::RemoveFromCollection { collection: id("7") });
    }
}

#[cfg(test)]
mod remap_tests {
    use super::*;

    fn k(p: &str) -> KeywordPath {
        KeywordPath::from_pipe(p)
    }

    #[test]
    fn subtrees_move_and_duplicates_fold() {
        let join = KeywordRemap {
            map: vec![(k("streets"), k("street")), (k("a"), k("g|a"))],
            prune: vec![],
            label: String::new(),
        };
        assert_eq!(join.path(&k("streets|night")), k("street|night"));
        assert_eq!(join.path(&k("a|x")), k("g|a|x"));
        assert_eq!(join.path(&k("ab")), k("ab"), "a prefix of a name is not a parent");
        let mut kws = vec![k("street"), k("b&w"), k("streets")];
        assert!(join.apply(&mut kws));
        assert_eq!(kws, [k("street"), k("b&w")]);
        assert!(!join.apply(&mut kws));
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
    /// Keyword tree changes, applied first and in order.
    #[serde(default)]
    pub remaps: Vec<KeywordRemap>,
    /// Album changes, applied in order after the keyword tree and before the photo edits
    /// (which may put photos into an album created here).
    #[serde(default)]
    pub collections: Vec<CollectionChange>,
    pub apply: Vec<Change>,
    pub conflicts: Vec<Conflict>,
    /// Changes that would not alter anything (value already equals the target).
    pub no_ops: Vec<Change>,
}

/// A change to the keyword tree that rewrites every photo carrying the keywords it moves.
/// Renaming, joining, grouping and ungrouping all come down to this.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeywordRemap {
    /// Each `from` keyword, and everything under it, moves to `to`: with `a → g|a`,
    /// `a|x` becomes `g|a|x`. The first entry that matches a path decides.
    pub map: Vec<(KeywordPath, KeywordPath)>,
    /// Keywords dropped from the tree afterwards if they are left with no children and
    /// no photo carries them (the group an ungroup empties).
    #[serde(default)]
    pub prune: Vec<KeywordPath>,
    /// What the user asked for, for showing: `Join "streets" into "street"`.
    pub label: String,
}

impl KeywordRemap {
    /// A keyword gets a new name, in the same place.
    pub fn rename(path: KeywordPath, name: &str) -> Self {
        let mut to = path.0.clone();
        to.pop();
        to.push(name.to_owned());
        let label = format!("Rename “{}” to “{name}”", path.to_pipe());
        Self { map: vec![(path, KeywordPath(to))], prune: vec![], label }
    }

    /// Several keywords become one, existing or new (replacing one keyword with another
    /// is joining one). Keywords under them move along.
    pub fn join(from: Vec<KeywordPath>, into: KeywordPath) -> Self {
        let names: Vec<String> = from.iter().map(|p| format!("“{}”", p.to_pipe())).collect();
        let label = format!("Join {} into “{}”", names.join(", "), into.to_pipe());
        let map = from.into_iter().filter(|p| *p != into).map(|p| (p, into.clone())).collect();
        Self { map, prune: vec![], label }
    }

    /// Keywords go under a parent, existing or new; `None` moves them to the top level.
    pub fn group(paths: Vec<KeywordPath>, under: Option<KeywordPath>) -> Self {
        let names: Vec<String> = paths.iter().map(|p| format!("“{}”", p.to_pipe())).collect();
        let label = match &under {
            Some(u) => format!("Group {} under “{}”", names.join(", "), u.to_pipe()),
            None => format!("Move {} to the top level", names.join(", ")),
        };
        let map = paths
            .into_iter()
            .map(|p| {
                let mut to = under.as_ref().map(|u| u.0.clone()).unwrap_or_default();
                to.push(p.0.last().cloned().unwrap_or_default());
                (p, KeywordPath(to))
            })
            .filter(|(from, to)| from != to)
            .collect();
        Self { map, prune: vec![], label }
    }

    /// A group's keywords (`children`, as the tree has them now) move up a level, and
    /// the group goes if no photo carries it itself.
    pub fn ungroup(group: KeywordPath, children: Vec<KeywordPath>) -> Self {
        let up = group.0[..group.0.len().saturating_sub(1)].to_vec();
        let map = children
            .into_iter()
            .map(|c| {
                let mut to = up.clone();
                to.push(c.0.last().cloned().unwrap_or_default());
                (c, KeywordPath(to))
            })
            .collect();
        let label = format!("Ungroup “{}”", group.to_pipe());
        Self { map, prune: vec![group], label }
    }

    /// Where a keyword ends up.
    pub fn path(&self, path: &KeywordPath) -> KeywordPath {
        for (from, to) in &self.map {
            if path.0.starts_with(&from.0) {
                let mut moved = to.0.clone();
                moved.extend_from_slice(&path.0[from.0.len()..]);
                return KeywordPath(moved);
            }
        }
        path.clone()
    }

    pub fn moves(&self, path: &KeywordPath) -> bool {
        self.map.iter().any(|(from, _)| path.0.starts_with(&from.0))
    }

    /// Rewrites a photo's keywords; one landing on a keyword already there is dropped.
    /// Returns whether anything changed.
    pub fn apply(&self, keywords: &mut Vec<KeywordPath>) -> bool {
        if !keywords.iter().any(|k| self.moves(k)) {
            return false;
        }
        let mut out: Vec<KeywordPath> = Vec::with_capacity(keywords.len());
        for k in keywords.iter() {
            let moved = self.path(k);
            if !out.contains(&moved) {
                out.push(moved);
            }
        }
        let changed = out != *keywords;
        *keywords = out;
        changed
    }
}

/// A change to a source's albums, smart albums and groups. Photos are put into albums
/// and taken out of them by [`Op::AddToCollection`] and [`Op::RemoveFromCollection`];
/// a smart album has no members of its own, only its rule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CollectionChange {
    /// A new album, smart album (with `rule`) or group. The app makes `id` up
    /// ([`CollectionId::new_local`]); it names the collection until a sync after the
    /// write brings the source's own.
    Create {
        id: CollectionId,
        name: String,
        kind: CollectionKind,
        parent: Option<CollectionId>,
        rule: Option<SmartRule>,
    },
    Rename {
        id: CollectionId,
        name: String,
    },
    /// A smart album's rule.
    SetRule {
        id: CollectionId,
        rule: SmartRule,
    },
    /// The collection and everything under it. Photos stay where they are; only their
    /// place in it goes.
    Delete {
        id: CollectionId,
    },
    /// Into a group or project (`None`: the top level), with everything under it.
    Move {
        id: CollectionId,
        parent: Option<CollectionId>,
    },
}

impl CollectionChange {
    pub fn id(&self) -> &CollectionId {
        match self {
            Self::Create { id, .. }
            | Self::Rename { id, .. }
            | Self::SetRule { id, .. }
            | Self::Delete { id }
            | Self::Move { id, .. } => id,
        }
    }

    /// What it does, for showing: `New album “street” in “trips”`.
    pub fn label(&self, collections: &[Collection]) -> String {
        let name = |id: &CollectionId| {
            collections.iter().find(|c| c.id == *id).map_or_else(|| format!("#{}", id.0), |c| format!("“{}”", c.path))
        };
        match self {
            Self::Create { name: new, kind, parent, .. } => {
                let what = match kind {
                    CollectionKind::Smart => "smart album",
                    CollectionKind::Group => "group",
                    CollectionKind::Project => "project",
                    _ => "album",
                };
                match parent {
                    Some(p) => format!("New {what} “{new}” in {}", name(p)),
                    None => format!("New {what} “{new}”"),
                }
            }
            Self::Rename { id, name: new } => format!("Rename {} to “{new}”", name(id)),
            Self::SetRule { id, .. } => format!("Change the rule of {}", name(id)),
            Self::Delete { id } => format!("Delete {}", name(id)),
            Self::Move { id, parent: Some(p) } => format!("Move {} into {}", name(id), name(p)),
            Self::Move { id, parent: None } => format!("Move {} to the top level", name(id)),
        }
    }

    /// Applies the change to a list of collections, paths made again: the reference
    /// behaviour a source's own must match.
    pub fn apply_to(&self, collections: &mut Vec<Collection>) {
        match self {
            Self::Create { id, name, kind, parent, rule } => collections.push(Collection {
                id: id.clone(),
                name: name.clone(),
                kind: *kind,
                parent: parent.clone(),
                path: String::new(),
                rule: rule.clone(),
                system: false,
            }),
            Self::Rename { id, name } => {
                if let Some(c) = collections.iter_mut().find(|c| c.id == *id) {
                    c.name = name.clone();
                }
            }
            Self::SetRule { id, rule } => {
                if let Some(c) = collections.iter_mut().find(|c| c.id == *id) {
                    c.rule = Some(rule.clone());
                }
            }
            Self::Delete { id } => {
                let gone = descendants(collections, id);
                collections.retain(|c| !gone.contains(&c.id));
            }
            Self::Move { id, parent } => {
                if let Some(c) = collections.iter_mut().find(|c| c.id == *id) {
                    c.parent = parent.clone();
                }
            }
        }
        set_paths(collections);
    }
}

/// Why `change` cannot be applied to `collections`, or `None` if it can.
pub fn collection_problem(collections: &[Collection], change: &CollectionChange) -> Option<String> {
    let find = |id: &CollectionId| collections.iter().find(|c| c.id == *id);
    match change {
        CollectionChange::Create { id, name, kind, parent, rule } => {
            if find(id).is_some() {
                return Some("it exists already".into());
            }
            if name.trim().is_empty() {
                return Some("it has no name".into());
            }
            if !matches!(kind, CollectionKind::Album | CollectionKind::Smart | CollectionKind::Group) {
                return Some(format!("a {kind:?} cannot be made here"));
            }
            if (*kind == CollectionKind::Smart) != rule.is_some() {
                return Some("only a smart album has a rule, and it must have one".into());
            }
            place_problem(collections, parent.as_ref())
        }
        CollectionChange::Rename { id, name } => match find(id) {
            None => Some("the album is gone".into()),
            Some(c) if c.system => Some(format!("“{}” is Capture One's own", c.path)),
            Some(_) if name.trim().is_empty() => Some("the new name is empty".into()),
            Some(_) => None,
        },
        CollectionChange::SetRule { id, .. } => match find(id) {
            None => Some("the album is gone".into()),
            Some(c) if c.kind != CollectionKind::Smart => Some(format!("“{}” is not a smart album", c.path)),
            Some(_) => None,
        },
        CollectionChange::Delete { id } => {
            if find(id).is_none() {
                return Some("the album is gone".into());
            }
            let under: Vec<&Collection> = descendants(collections, id).iter().filter_map(find).collect();
            if let Some(c) = under.iter().find(|c| c.system) {
                return Some(format!("“{}” is Capture One's own", c.path));
            }
            under
                .iter()
                .find(|c| c.kind == CollectionKind::Project)
                .map(|c| format!("“{}” is a project; projects are deleted in Capture One", c.path))
        }
        CollectionChange::Move { id, parent } => match find(id) {
            None => Some("the album is gone".into()),
            Some(c) if c.system => Some(format!("“{}” is Capture One's own", c.path)),
            Some(c) if c.kind == CollectionKind::Project => {
                Some(format!("“{}” is a project; projects are moved in Capture One", c.path))
            }
            Some(c) if c.parent == *parent => Some(format!("“{}” is there already", c.path)),
            Some(_) if parent.as_ref().is_some_and(|p| descendants(collections, id).contains(p)) => {
                Some("a group cannot go into itself".into())
            }
            Some(_)
                if descendants(collections, id).iter().filter_map(find).any(|c| c.kind == CollectionKind::Project) =>
            {
                Some("it holds a project; projects are moved in Capture One".into())
            }
            Some(_) => place_problem(collections, parent.as_ref()),
        },
    }
}

/// Why a collection cannot go into `parent` (`None`: the top level), or `None` if it can.
fn place_problem(collections: &[Collection], parent: Option<&CollectionId>) -> Option<String> {
    match parent.map(|p| collections.iter().find(|c| c.id == *p)) {
        Some(None) => Some("the group it goes in is gone".into()),
        Some(Some(p)) if p.system => Some(format!("“{}” is Capture One's own", p.path)),
        Some(Some(p)) if !matches!(p.kind, CollectionKind::Group | CollectionKind::Project) => {
            Some(format!("“{}” cannot hold albums", p.path))
        }
        _ => None,
    }
}

/// Why a photo cannot go into (or leave) `collection`, or `None` if it can: only albums
/// hold photos of their own; a smart album's come from its rule.
pub fn membership_problem(collections: &[Collection], collection: &CollectionId) -> Option<String> {
    match collections.iter().find(|c| c.id == *collection) {
        None => Some("the album is gone".into()),
        Some(c) if c.kind == CollectionKind::Smart => {
            Some(format!("“{}” is a smart album: its photos come from its rule", c.path))
        }
        Some(c) if c.kind != CollectionKind::Album => Some(format!("“{}” is not an album", c.path)),
        Some(_) => None,
    }
}

/// `id` and every collection under it.
pub fn descendants(collections: &[Collection], id: &CollectionId) -> Vec<CollectionId> {
    let mut out = vec![id.clone()];
    let mut i = 0;
    while i < out.len() {
        let parent = out[i].clone();
        out.extend(collections.iter().filter(|c| c.parent.as_ref() == Some(&parent)).map(|c| c.id.clone()));
        i += 1;
    }
    out
}

/// Each collection's display path, `Group / Album`, from its parents' names.
pub fn set_paths(collections: &mut [Collection]) {
    let paths: Vec<String> = collections
        .iter()
        .map(|c| {
            let mut names = vec![c.name.as_str()];
            let mut seen = vec![&c.id];
            let mut parent = c.parent.as_ref();
            while let Some(p) = parent.and_then(|id| collections.iter().find(|x| x.id == *id)) {
                if seen.contains(&&p.id) {
                    break;
                }
                seen.push(&p.id);
                names.push(&p.name);
                parent = p.parent.as_ref();
            }
            names.reverse();
            names.join(" / ")
        })
        .collect();
    for (c, path) in collections.iter_mut().zip(paths) {
        c.path = path;
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ApplyReport {
    pub applied: usize,
    /// Where the source saved a backup before writing, if it does that.
    pub backup: Option<String>,
}
