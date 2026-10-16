//! [`c1model::PhotoSource::snapshot`] from the store: everything, or what changed since a
//! cursor.
//!
//! The cursor is `<store uuid>:<generation>`. Given one of this store's, only what changed
//! after that generation is sent (`BeginIncremental`, upserts, deletes, and the collections
//! and keywords again if they changed); anything else gets everything (`BeginFull`).
//! The roots are walked first when a walk is due (on open, after `rescan`).

use std::sync::atomic::Ordering;

use c1model::{Cursor, KeywordNode, KeywordPath, Result, SourceEvent, SourceId};

use crate::FolderSource;
use crate::definition::Definition;
use crate::layer::{self, UserLayer};
use crate::scan;
use crate::store::{FileRec, FileState, State};

pub(crate) fn run(
    src: &FolderSource,
    since: Option<&Cursor>,
    sink: &mut dyn FnMut(SourceEvent) -> Result<()>,
) -> Result<()> {
    if src.scan_due.swap(false, Ordering::SeqCst)
        && let Err(e) = scan::scan(src)
    {
        src.scan_due.store(true, Ordering::SeqCst);
        return Err(e);
    }
    let def = src.definition();
    let events = {
        let state = src.store().read();
        match since.and_then(|c| generation_of(&state, c)) {
            Some(since) => incremental(&src.id, &def, &state, since),
            None => full(&src.id, &def, &state),
        }
    };
    for event in events {
        sink(event)?;
    }
    Ok(())
}

/// The generation of a cursor this store gave, not newer than the store.
fn generation_of(state: &State, cursor: &Cursor) -> Option<u64> {
    let (uuid, generation) = cursor.0.split_once(':')?;
    let generation: u64 = generation.parse().ok()?;
    (uuid == state.uuid && generation <= state.generation).then_some(generation)
}

fn cursor(state: &State) -> Cursor {
    Cursor(format!("{}:{}", state.uuid, state.generation))
}

fn shown(rec: &FileRec) -> bool {
    rec.state != FileState::Detached
}

fn full(source: &SourceId, def: &Definition, state: &State) -> Vec<SourceEvent> {
    let mut events = vec![SourceEvent::BeginFull];
    events.extend(collections_and_keywords(def, state));
    for (ext, rec) in state.files.iter().filter(|(_, rec)| shown(rec)) {
        events.push(upsert(source, def, state, ext, rec));
    }
    events.push(SourceEvent::End(cursor(state)));
    events
}

fn incremental(source: &SourceId, def: &Definition, state: &State, since: u64) -> Vec<SourceEvent> {
    let mut changed: Vec<&String> = state.changed.iter().filter(|(_, at)| **at > since).map(|(ext, _)| ext).collect();
    changed.sort();
    let mut gone: Vec<&String> = state
        .tombs
        .iter()
        .filter(|(ext, at)| **at > since && !state.files.get(*ext).is_some_and(shown))
        .map(|(ext, _)| ext)
        .collect();
    gone.sort();
    let upserts: Vec<SourceEvent> = changed
        .into_iter()
        .filter_map(|ext| state.files.get(ext).filter(|rec| shown(rec)).map(|rec| upsert(source, def, state, ext, rec)))
        .collect();

    let mut events = vec![SourceEvent::BeginIncremental];
    // A photo's keywords may have changed with it, so the tree is sent again with any photo.
    if state.albums_gen > since || !upserts.is_empty() || !gone.is_empty() {
        events.extend(collections_and_keywords(def, state));
    }
    events.extend(upserts);
    events.extend(gone.into_iter().map(|ext| SourceEvent::Delete(c1model::PhotoId::new(source, ext.as_str()))));
    events.push(SourceEvent::End(cursor(state)));
    events
}

fn upsert(source: &SourceId, def: &Definition, state: &State, ext: &str, rec: &FileRec) -> SourceEvent {
    SourceEvent::Upsert(Box::new(layer::compose(source, def, ext, rec, state.user.get(ext))))
}

/// The user's albums, then the folders; and the keyword tree of the photos shown.
fn collections_and_keywords(def: &Definition, state: &State) -> [SourceEvent; 2] {
    let mut collections = state.albums.clone();
    collections.extend(layer::folder_collections(def, state.files.values()));
    let keywords =
        state.files.iter().filter(|(_, rec)| shown(rec)).flat_map(|(ext, rec)| keywords(rec, state.user.get(ext)));
    [SourceEvent::Collections(collections), SourceEvent::Keywords(KeywordNode::tree(keywords))]
}

/// A photo's keywords as [`layer::compose`] gives them: the user's, else the file's.
fn keywords<'a>(rec: &'a FileRec, layer: Option<&'a UserLayer>) -> &'a [KeywordPath] {
    layer.and_then(|l| l.keywords.as_deref()).unwrap_or(&rec.meta.keywords)
}
