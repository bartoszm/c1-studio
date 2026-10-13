//! An in-memory source: the reference implementation of the contract, used by tests
//! and by anything that needs a source without a real catalog behind it.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use crate::change::{
    ApplyPlan, ApplyReport, Change, CollectionChange, Conflict, KeywordRemap, Op, collection_problem, descendants,
    membership_problem, plan_with,
};
use crate::photo::{Collection, CollectionId, KeywordNode, KeywordPath, Photo, PhotoId, SourceId};
use crate::source::{
    Capabilities, Compatibility, Cursor, Field, ImageRef, ImageTier, PhotoSource, Result, SourceError, SourceEvent,
    SourceInfo, WriteStatus,
};

#[derive(Default)]
struct State {
    photos: BTreeMap<PhotoId, Photo>,
    collections: Vec<Collection>,
    /// Bumped on every write; the cursor is this number.
    version: u64,
    locked_by: Option<String>,
    /// Collections made so far, for their ids (`m1`, `m2`, ...).
    made: u64,
}

pub struct MemorySource {
    id: SourceId,
    state: Mutex<State>,
}

impl MemorySource {
    pub fn new(id: &str, photos: Vec<Photo>, collections: Vec<Collection>) -> Self {
        let photos = photos.into_iter().map(|p| (p.id.clone(), p)).collect();
        Self { id: SourceId(id.to_owned()), state: Mutex::new(State { photos, collections, ..State::default() }) }
    }

    /// Simulates another program holding the source open.
    pub fn set_locked(&self, by: Option<&str>) {
        self.state.lock().unwrap().locked_by = by.map(str::to_owned);
    }

    /// Simulates an edit made outside the app, as Capture One itself would.
    pub fn edit_externally(&self, id: &PhotoId, edit: impl FnOnce(&mut Photo)) {
        let mut state = self.state.lock().unwrap();
        if let Some(photo) = state.photos.get_mut(id) {
            edit(photo);
            state.version += 1;
        }
    }

    /// Simulates a photo imported outside the app.
    pub fn add_externally(&self, photo: Photo) {
        let mut state = self.state.lock().unwrap();
        state.photos.insert(photo.id.clone(), photo);
        state.version += 1;
    }

    /// Simulates a photo deleted outside the app.
    pub fn remove_externally(&self, id: &PhotoId) {
        let mut state = self.state.lock().unwrap();
        state.photos.remove(id);
        state.version += 1;
    }

    /// Simulates albums changed outside the app.
    pub fn edit_collections_externally(&self, edit: impl FnOnce(&mut Vec<Collection>)) {
        let mut state = self.state.lock().unwrap();
        edit(&mut state.collections);
        state.version += 1;
    }

    pub fn photo(&self, id: &PhotoId) -> Option<Photo> {
        self.state.lock().unwrap().photos.get(id).cloned()
    }

    pub fn collections(&self) -> Vec<Collection> {
        self.state.lock().unwrap().collections.clone()
    }
}

impl PhotoSource for MemorySource {
    fn info(&self) -> SourceInfo {
        SourceInfo {
            id: self.id.clone(),
            name: format!("memory · {}", self.id.0),
            version: "1".into(),
            compatibility: Compatibility::tested("in memory"),
        }
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            writable: vec![
                Field::Rating,
                Field::Color,
                Field::Keywords,
                Field::Iptc,
                Field::Place,
                Field::Geo,
                Field::Collections,
            ],
            tiers: vec![],
            geo_per_asset: false,
            collections: true,
        }
    }

    fn snapshot(&self, since: Option<&Cursor>, sink: &mut dyn FnMut(SourceEvent) -> Result<()>) -> Result<()> {
        let state = self.state.lock().unwrap();
        let cursor = Cursor(state.version.to_string());
        if since == Some(&cursor) {
            sink(SourceEvent::BeginIncremental)?;
            return sink(SourceEvent::End(cursor));
        }
        sink(SourceEvent::BeginFull)?;
        sink(SourceEvent::Collections(state.collections.clone()))?;
        // The keyword tree: every keyword on a photo and every level above it.
        let mut tree: Vec<KeywordNode> = Vec::new();
        for path in state.photos.values().flat_map(|p| &p.keywords) {
            for n in 1..=path.0.len() {
                let level = KeywordPath(path.0[..n].to_vec());
                let uses = u32::from(n == path.0.len());
                match tree.iter_mut().find(|k| k.path == level) {
                    Some(node) => node.uses += uses,
                    None => tree.push(KeywordNode { path: level, uses }),
                }
            }
        }
        sink(SourceEvent::Keywords(tree))?;
        for photo in state.photos.values() {
            sink(SourceEvent::Upsert(Box::new(photo.clone())))?;
        }
        sink(SourceEvent::End(cursor))
    }

    fn image(&self, _id: &PhotoId, _tier: ImageTier) -> Result<Option<ImageRef>> {
        Ok(None)
    }

    fn write_status(&self) -> WriteStatus {
        match &self.state.lock().unwrap().locked_by {
            Some(by) => WriteStatus::Locked { by: by.clone() },
            None => WriteStatus::Ready,
        }
    }

    fn plan(&self, changes: &[Change]) -> Result<ApplyPlan> {
        let state = self.state.lock().unwrap();
        Ok(plan_with(changes, |id| state.photos.get(id)))
    }

    fn plan_after(
        &self,
        remaps: &[KeywordRemap],
        collections: &[CollectionChange],
        changes: &[Change],
    ) -> Result<ApplyPlan> {
        let state = self.state.lock().unwrap();
        let mut after = state.photos.clone();
        for photo in after.values_mut() {
            for remap in remaps {
                remap.apply(&mut photo.keywords);
            }
        }
        let mut albums = state.collections.clone();
        for change in collections {
            change.apply_to(&mut albums);
        }
        // Photos cannot go into an album that is gone, or into a smart album.
        let (refused, rest): (Vec<_>, Vec<_>) = changes
            .iter()
            .cloned()
            .partition(|c| membership(&c.op).is_some_and(|id| membership_problem(&albums, id).is_some()));
        let mut plan = plan_with(&rest, |id| after.get(id));
        for change in refused {
            let why = membership(&change.op).and_then(|id| membership_problem(&albums, id)).unwrap_or_default();
            plan.conflicts.push(Conflict { change, current: why });
        }
        plan.remaps = remaps.to_vec();
        plan.collections = collections.to_vec();
        Ok(plan)
    }

    fn check_collection(&self, earlier: &[CollectionChange], change: &CollectionChange) -> Result<Option<String>> {
        let mut albums = self.state.lock().unwrap().collections.clone();
        for c in earlier {
            c.apply_to(&mut albums);
        }
        Ok(collection_problem(&albums, change))
    }

    fn apply(&self, plan: &ApplyPlan) -> Result<ApplyReport> {
        let mut state = self.state.lock().unwrap();
        if let Some(by) = &state.locked_by {
            return Err(SourceError::Locked { by: by.clone() });
        }
        // Everything is done on copies, kept only if all of it holds.
        let mut photos = state.photos.clone();
        for photo in photos.values_mut() {
            for remap in &plan.remaps {
                remap.apply(&mut photo.keywords);
            }
        }
        // Collections made here get the source's own ids, as a real source's would.
        let mut albums = state.collections.clone();
        let mut made: HashMap<CollectionId, CollectionId> = HashMap::new();
        let real = |made: &HashMap<CollectionId, CollectionId>, id: &CollectionId| made.get(id).unwrap_or(id).clone();
        for change in &plan.collections {
            let mut change = match change.clone() {
                CollectionChange::Create { id, name, kind, parent, rule } => {
                    CollectionChange::Create { id, name, kind, parent: parent.map(|p| real(&made, &p)), rule }
                }
                CollectionChange::Rename { id, name } => CollectionChange::Rename { id: real(&made, &id), name },
                CollectionChange::SetRule { id, rule } => CollectionChange::SetRule { id: real(&made, &id), rule },
                CollectionChange::Delete { id } => CollectionChange::Delete { id: real(&made, &id) },
                CollectionChange::Move { id, parent } => {
                    CollectionChange::Move { id: real(&made, &id), parent: parent.map(|p| real(&made, &p)) }
                }
            };
            if let Some(why) = collection_problem(&albums, &change) {
                return Err(SourceError::Other(format!("{}: {why}", change.label(&albums))));
            }
            if let CollectionChange::Create { id, .. } = &mut change {
                state.made += 1;
                let own = CollectionId(format!("m{}", state.made));
                made.insert(std::mem::replace(id, own.clone()), own);
            }
            if let CollectionChange::Delete { id } = &change {
                let gone = descendants(&albums, id);
                for photo in photos.values_mut() {
                    photo.collections.retain(|c| !gone.contains(c));
                }
            }
            change.apply_to(&mut albums);
        }
        let changes: Vec<Change> = plan
            .apply
            .iter()
            .map(|c| {
                let op = match &c.op {
                    Op::AddToCollection { collection } => Op::AddToCollection { collection: real(&made, collection) },
                    Op::RemoveFromCollection { collection } => {
                        Op::RemoveFromCollection { collection: real(&made, collection) }
                    }
                    op => op.clone(),
                };
                Change { photo: c.photo.clone(), op }
            })
            .collect();
        // If anything moved since the plan was made, refuse rather than overwrite.
        let fresh = plan_with(&changes, |id| photos.get(id));
        if fresh.apply.len() != changes.len() {
            return Err(SourceError::StalePlan);
        }
        for change in &changes {
            if let Some(why) = membership(&change.op).and_then(|id| membership_problem(&albums, id)) {
                return Err(SourceError::Other(why));
            }
            if let Some(photo) = photos.get_mut(&change.photo) {
                change.op.apply_to(photo);
            }
        }
        state.photos = photos;
        state.collections = albums;
        state.version += 1;
        Ok(ApplyReport { applied: plan.remaps.len() + plan.collections.len() + plan.apply.len(), backup: None })
    }
}

/// The album an op puts a photo into or takes it out of.
fn membership(op: &Op) -> Option<&CollectionId> {
    match op {
        Op::AddToCollection { collection } | Op::RemoveFromCollection { collection } => Some(collection),
        _ => None,
    }
}
