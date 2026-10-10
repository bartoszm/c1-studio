//! An in-memory source: the reference implementation of the contract, used by tests
//! and by anything that needs a source without a real catalog behind it.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::change::{ApplyPlan, ApplyReport, Change, plan_with};
use crate::photo::{Collection, Photo, PhotoId, SourceId};
use crate::source::{
    Capabilities, Cursor, Field, ImageRef, ImageTier, PhotoSource, Result, SourceError, SourceEvent, SourceInfo,
    WriteStatus,
};

#[derive(Default)]
struct State {
    photos: BTreeMap<PhotoId, Photo>,
    collections: Vec<Collection>,
    /// Bumped on every write; the cursor is this number.
    version: u64,
    locked_by: Option<String>,
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

    pub fn photo(&self, id: &PhotoId) -> Option<Photo> {
        self.state.lock().unwrap().photos.get(id).cloned()
    }
}

impl PhotoSource for MemorySource {
    fn info(&self) -> SourceInfo {
        SourceInfo { id: self.id.clone(), name: format!("memory · {}", self.id.0), version: "1".into() }
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

    fn apply(&self, plan: &ApplyPlan) -> Result<ApplyReport> {
        let mut state = self.state.lock().unwrap();
        if let Some(by) = &state.locked_by {
            return Err(SourceError::Locked { by: by.clone() });
        }
        // Re-check against the current state: if anything moved since the plan was
        // made, refuse rather than overwrite.
        let fresh = plan_with(&plan.apply, |id| state.photos.get(id));
        if fresh.apply.len() != plan.apply.len() {
            return Err(SourceError::StalePlan);
        }
        for change in &plan.apply {
            if let Some(photo) = state.photos.get_mut(&change.photo) {
                change.op.apply_to(photo);
            }
        }
        state.version += 1;
        Ok(ApplyReport { applied: plan.apply.len(), backup: None })
    }
}
