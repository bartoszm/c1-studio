//! Suggestions from a vision model (stage 5): stored per photo until the user accepts
//! them into the edit queue, as edits from [`Origin::Ai`], or rejects them.

use std::collections::{HashMap, HashSet};

use c1ai::RunRecord;
use c1model::{KeywordPath, Op};
use serde::{Deserialize, Serialize};

use super::Library;
use crate::ai::{Entry, Status};
use crate::edits::Origin;

/// What the user takes from one photo's suggestion, as edited in the review: any field
/// left out is not touched.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Accept {
    pub row: u64,
    /// Pipe paths; ones the catalog lacks are added to it.
    pub keywords: Vec<String>,
    /// Becomes the caption (IPTC description).
    pub caption: Option<String>,
    pub headline: Option<String>,
    pub rating: Option<u8>,
    /// What the review showed the photo to have.
    pub seen: Seen,
}

/// A photo's caption, headline and rating as the review showed them. A field taken is
/// set only while the photo still has what was shown: one written since (in Capture One,
/// in the panel) is not replaced unseen. Left out, it reads as "the photo had none".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Seen {
    pub caption: Option<String>,
    pub headline: Option<String>,
    pub rating: Option<u8>,
}

/// What accepting did.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Accepted {
    /// The ops that undo it.
    pub undo: Vec<(u64, Op)>,
    /// Photos with a taken field that changed since it was shown: that field was left as
    /// it is, and the suggestion stays to review against what the photo has now.
    pub changed: Vec<u64>,
}

/// A text field as compared: trimmed, empty as none.
fn text(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

impl Library {
    /// The catalog's keyword paths with how many photos carry each, most used first: what
    /// a model is asked to choose from.
    pub fn vocabulary(&self) -> anyhow::Result<Vec<(String, u64)>> {
        let mut out: Vec<(String, u64)> =
            self.keywords()?.into_iter().filter(|n| n.uses > 0).map(|n| (n.path.to_pipe(), n.uses as u64)).collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Ok(out)
    }

    pub fn suggestion(&self, row: u64) -> anyhow::Result<Option<Entry>> {
        match self.key_of(row) {
            Some(key) => self.suggestions.get(&key),
            None => Ok(None),
        }
    }

    /// Of `rows`, those already described with this model and prompt: a run skips them.
    pub fn described(&self, rows: &[u64], model: &str, prompt: u64) -> anyhow::Result<HashSet<u64>> {
        let all = self.suggestions.all()?;
        Ok(rows
            .iter()
            .copied()
            .filter(|&row| {
                self.key_of(row).and_then(|k| all.get(&k)).is_some_and(|e| e.model == model && e.prompt == prompt)
            })
            .collect())
    }

    pub fn put_suggestion(&self, row: u64, entry: &Entry, record: &RunRecord) -> anyhow::Result<()> {
        let Some(key) = self.key_of(row) else { return Ok(()) };
        self.suggestions.put(&key, entry, record)
    }

    /// Records a call that gave no suggestion: it was paid for all the same.
    pub fn record_call(&self, row: u64, record: &RunRecord) -> anyhow::Result<()> {
        let Some(key) = self.key_of(row) else { return Ok(()) };
        self.suggestions.record(&key, record)
    }

    /// Stored suggestions of photos in the library, with `status` or any, newest first.
    pub fn suggestions(&self, status: Option<Status>) -> anyhow::Result<Vec<(u64, Entry)>> {
        let all = self.suggestions.all()?;
        let mut out: Vec<(u64, Entry)> = self
            .store
            .rows_of(all.keys().map(String::as_str))?
            .into_iter()
            .filter_map(|(row, key)| all.get(key).map(|e| (row, e.clone())))
            .filter(|(_, e)| status.is_none_or(|s| e.status == s))
            .collect();
        out.sort_by(|a, b| b.1.at.cmp(&a.1.at).then(a.0.cmp(&b.0)));
        Ok(out)
    }

    pub fn set_suggestion_status(&self, rows: &[u64], status: Status) -> anyhow::Result<()> {
        let keys: Vec<String> = rows.iter().filter_map(|&r| self.key_of(r)).collect();
        self.suggestions.set_status(&keys.iter().map(String::as_str).collect::<Vec<_>>(), status)
    }

    /// Calls made and what they cost in all, in US dollars.
    pub fn ai_spent(&self) -> anyhow::Result<(u64, f64)> {
        self.suggestions.spent()
    }

    /// Queues what the user took from suggestions, logged as coming from the model, and
    /// marks those suggestions accepted. Each photo's ops are made from the photo as it is
    /// now, so nothing edited since is undone; a caption, headline or rating that is no
    /// longer what the review showed is left alone (see [`Seen`]).
    pub fn accept_suggestions(&self, picks: &[Accept]) -> anyhow::Result<Accepted> {
        let by_row: HashMap<u64, &Accept> = picks.iter().map(|p| (p.row, p)).collect();
        let rows: Vec<u64> = by_row.keys().copied().collect();
        let changed = std::sync::Mutex::new(HashSet::new());
        let undo = self.edit_photos_as(
            &rows,
            |row, photo| {
                let pick = by_row[&row];
                let mut ops = Vec::new();
                let mut stale = false;
                let paths: Vec<KeywordPath> = pick
                    .keywords
                    .iter()
                    .map(|k| KeywordPath::from_pipe(k))
                    .filter(|p| !p.0.is_empty() && !photo.keywords.contains(p))
                    .collect();
                if !paths.is_empty() {
                    ops.push(Op::AddKeywords { paths });
                }
                let mut iptc = photo.iptc.clone();
                if let Some(caption) = text(&pick.caption) {
                    if text(&photo.iptc.description) == text(&pick.seen.caption) {
                        iptc.description = Some(caption.to_string());
                    } else {
                        stale = true;
                    }
                }
                if let Some(headline) = text(&pick.headline) {
                    if text(&photo.iptc.headline) == text(&pick.seen.headline) {
                        iptc.headline = Some(headline.to_string());
                    } else {
                        stale = true;
                    }
                }
                if iptc != photo.iptc {
                    ops.push(Op::SetIptc { base: photo.iptc.clone(), value: iptc });
                }
                if let Some(rating) = pick.rating.filter(|r| (1..=5).contains(r))
                    && photo.rating != Some(rating)
                {
                    if photo.rating == pick.seen.rating {
                        ops.push(Op::SetRating { base: photo.rating, value: Some(rating) });
                    } else {
                        stale = true;
                    }
                }
                if stale {
                    changed.lock().unwrap().insert(row);
                }
                Ok(ops)
            },
            Origin::Ai,
        )?;
        let changed = changed.into_inner().unwrap();
        let done: Vec<u64> = rows.iter().copied().filter(|r| !changed.contains(r)).collect();
        self.set_suggestion_status(&done, Status::Accepted)?;
        let mut changed: Vec<u64> = changed.into_iter().collect();
        changed.sort_unstable();
        Ok(Accepted { undo, changed })
    }
}
