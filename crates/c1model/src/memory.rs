//! An in-memory source: the reference implementation of the contract, used by tests
//! and by anything that needs a source without a real catalog behind it.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::change::{
    ApplyPlan, ApplyReport, Change, CollectionChange, KeywordRemap, apply_with, collection_problem, plan_after_with,
    plan_with,
};
use crate::photo::{Collection, CollectionId, KeywordNode, Photo, PhotoId, SourceId};
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
            rescan: false,
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
        let tree = KeywordNode::tree(state.photos.values().flat_map(|p| &p.keywords));
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
        Ok(plan_after_with(&state.photos, &state.collections, remaps, collections, changes))
    }

    fn check_collection(&self, earlier: &[CollectionChange], change: &CollectionChange) -> Result<Option<String>> {
        let mut albums = self.state.lock().unwrap().collections.clone();
        for c in earlier {
            c.apply_to(&mut albums);
        }
        Ok(collection_problem(&albums, change))
    }

    fn apply(&self, plan: &ApplyPlan) -> Result<ApplyReport> {
        let mut guard = self.state.lock().unwrap();
        let state = &mut *guard;
        if let Some(by) = &state.locked_by {
            return Err(SourceError::Locked { by: by.clone() });
        }
        // Collections made here get the source's own ids, as a real source's would.
        let made = &mut state.made;
        let applied = apply_with(&mut state.photos, &mut state.collections, plan, || {
            *made += 1;
            CollectionId(format!("m{made}"))
        })?;
        state.version += 1;
        Ok(ApplyReport { applied, backup: None, warnings: Vec::new() })
    }
}
