//! What choosing the best of alike photos leaves behind: the photos the user picked as
//! the ones to keep, and how a model saw each photo it looked at. Kept apart from the
//! store like the other marks of C1 Studio's own: a pick is the user's work, a model's
//! view is slow to make again, and both outlive the store being rebuilt. Photos are named
//! by key.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::db::OpenControl;
use c1ai::pick;
use redb::{Database, ReadOnlyTable, ReadableDatabase, ReadableTable, Table, TableDefinition, TableHandle};
use serde::{Deserialize, Serialize};

/// photo key → when it was picked (epoch ms).
const KEEPERS: TableDefinition<&str, i64> = TableDefinition::new("keepers");
/// photo key → postcard [`View`].
const VIEWS: TableDefinition<&str, &[u8]> = TableDefinition::new("views");

/// How the local model saw a photo: its embedding, and what that came to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct View {
    /// The version of the scoring (`c1ai::aesthetic::VERSION`) that made `scores`: another
    /// version's are worked out again from the embedding.
    pub version: u32,
    /// Size and modification time of the image looked at, as the analysis stamps it: a new
    /// render is looked at again.
    pub stamp: (u64, u64),
    /// The picture's embedding: what the scores are worked out from again when the way
    /// of scoring changes, without the model looking a second time.
    pub embedding: Vec<f32>,
    /// One score per axis in the order of `c1ai::aesthetic::AXES`, each 0 to 1.
    pub scores: [f32; 4],
}

/// A view as the model made it, of the photo at `row`: [`crate::Library::put_views`] adds
/// the stamp of the render it is of.
#[derive(Clone, Debug, PartialEq)]
pub struct NewView {
    pub row: u64,
    /// See [`View::version`].
    pub version: u32,
    pub embedding: Vec<f32>,
    /// See [`View::scores`].
    pub scores: [f32; 4],
}

/// An answer's number → postcard [`Verdict`].
const ANSWERS: TableDefinition<u64, &[u8]> = TableDefinition::new("answers");
/// photo key → the number of the answer the photo was last shown for. An answer is found
/// from the photos it was about, whatever else is in their group or under the filter by
/// now: see [`stands`].
const ASKED: TableDefinition<&str, u64> = TableDefinition::new("asked");
/// How an earlier build kept answers: under all the keys of the group as the filter showed
/// it, sorted and joined, so another filter or a frame gone missed the answer. Read once,
/// into the tables above.
const BY_GROUP: TableDefinition<&str, &[u8]> = TableDefinition::new("verdicts");

/// What a model that compared a group's frames said, as it is given to be kept:
/// [`crate::Library::put_verdict`] adds the frames it was shown and the question's version.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NewVerdict {
    /// When it was asked, epoch ms.
    pub at: i64,
    pub provider: String,
    pub model: String,
    /// Indexes into the frames shown, the one to keep first.
    pub order: Vec<u32>,
    /// Why, per frame shown; empty where none was given.
    pub reasons: Vec<String>,
    /// How clear the choice is, in the model's words.
    pub note: String,
}

/// What a model that compared a group's frames said.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    /// The version of the question (`c1ai::pick::VERSION`): another's is asked again.
    pub version: u32,
    /// When it was asked, epoch ms.
    pub at: i64,
    pub provider: String,
    pub model: String,
    /// The frames shown, by photo key, in the order shown: at most 16 of a bigger group.
    pub sent: Vec<String>,
    /// Indexes into `sent`, the one to keep first.
    pub order: Vec<u32>,
    /// Why, per frame of `sent`; empty where none was given.
    pub reasons: Vec<String>,
    /// How clear the choice is, in the model's words.
    pub note: String,
}

/// Per row of a group, its place in a model's order (0 is the one to keep) and why.
pub type Places = HashMap<u64, (usize, String)>;

impl Verdict {
    /// What a model said (`said`) of the frames `sent`, by photo key in the order shown,
    /// asked the question as it is now.
    pub fn new(sent: Vec<String>, said: NewVerdict) -> Self {
        let NewVerdict { at, provider, model, order, reasons, note } = said;
        Verdict { version: pick::VERSION, at, provider, model, sent, order, reasons, note }
    }
}

/// Whether an answer about `sent` frames still answers for a group of `shown` photos,
/// `covered` of which it was about. It does while it tells two of them apart at least and
/// the model saw all there is to see: every photo shown now, or as many frames as a
/// question holds (of a bigger group the rest follow, as they did when it was asked). So
/// a filter that shows fewer of the group, or a frame trashed or hidden since, keeps the
/// answer; a photo the model never saw (one added, a wider filter) has it asked again.
fn stands(sent: usize, covered: usize, shown: usize) -> bool {
    covered >= 2 && (covered == shown || sent >= pick::MAX_FRAMES)
}

/// Answers read in one go, by number: `None` for one that does not read, or answers
/// another version of the question.
type Read = HashMap<u64, Option<Verdict>>;

/// The number of the answer that stands for the photos `keys` ([`stands`]): the one about
/// the most of them, the later of two about as many. It is in `read` afterwards.
fn standing(
    answers: &ReadOnlyTable<u64, &'static [u8]>,
    asked: &ReadOnlyTable<&'static str, u64>,
    keys: &[&str],
    read: &mut Read,
) -> anyhow::Result<Option<u64>> {
    let mut covered: HashMap<u64, usize> = HashMap::new();
    for key in keys {
        if let Some(id) = asked.get(key)? {
            *covered.entry(id.value()).or_default() += 1;
        }
    }
    let mut by_cover: Vec<(u64, usize)> = covered.into_iter().collect();
    by_cover.sort_unstable_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)));
    for (id, covered) in by_cover {
        if !read.contains_key(&id) {
            let verdict = answers.get(id)?.and_then(|bytes| postcard::from_bytes::<Verdict>(bytes.value()).ok());
            read.insert(id, verdict.filter(|v| v.version == pick::VERSION));
        }
        if read[&id].as_ref().is_some_and(|v| stands(v.sent.len(), covered, keys.len())) {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// Keeps `verdict` and points the frames it was about to it. An answer none of its frames
/// points to any more (each was asked about again since) goes.
fn keep(answers: &mut Table<u64, &[u8]>, asked: &mut Table<&str, u64>, verdict: &Verdict) -> anyhow::Result<()> {
    let id = answers.last()?.map_or(0, |(id, _)| id.value() + 1);
    answers.insert(id, postcard::to_stdvec(verdict)?.as_slice())?;
    let mut before: HashSet<u64> = HashSet::new();
    for key in &verdict.sent {
        if let Some(old) = asked.insert(key.as_str(), id)? {
            before.insert(old.value());
        }
    }
    for old in before {
        let sent = answers
            .get(old)?
            .and_then(|bytes| postcard::from_bytes::<Verdict>(bytes.value()).ok())
            .map_or_else(Vec::new, |v| v.sent);
        let mut pointed = false;
        for key in &sent {
            pointed |= asked.get(key.as_str())?.is_some_and(|id| id.value() == old);
        }
        if !pointed {
            answers.remove(old)?;
        }
    }
    Ok(())
}

pub struct CullBook {
    db: Database,
}

impl CullBook {
    pub fn open(path: &Path, ctl: &OpenControl) -> anyhow::Result<Self> {
        let db = crate::db::open(path, "cull", ctl)?;
        let tx = db.begin_write()?;
        tx.open_table(KEEPERS)?;
        tx.open_table(VIEWS)?;
        // Answers an earlier build kept under their group: they were paid for, and are
        // kept under their frames now, oldest first so that a later one wins.
        let mut by_group: Vec<Verdict> = Vec::new();
        if tx.list_tables()?.any(|table| table.name() == BY_GROUP.name()) {
            for entry in tx.open_table(BY_GROUP)?.iter()? {
                by_group.extend(postcard::from_bytes::<Verdict>(entry?.1.value()).ok());
            }
            tx.delete_table(BY_GROUP)?;
        }
        by_group.sort_by_key(|v| v.at);
        {
            let mut answers = tx.open_table(ANSWERS)?;
            let mut asked = tx.open_table(ASKED)?;
            for verdict in &by_group {
                keep(&mut answers, &mut asked, verdict)?;
            }
        }
        tx.commit()?;
        Ok(Self { db })
    }

    /// What a model said that still answers for the photos `keys`, a group as it is shown
    /// now ([`stands`]), if it was asked the question as it is asked now.
    pub fn verdict(&self, keys: &[&str]) -> anyhow::Result<Option<Verdict>> {
        let tx = self.db.begin_read()?;
        let (answers, asked) = (tx.open_table(ANSWERS)?, tx.open_table(ASKED)?);
        let mut read = Read::new();
        let id = standing(&answers, &asked, keys, &mut read)?;
        Ok(id.and_then(|id| read.remove(&id).flatten()))
    }

    /// Per group of `groups` (each its photos' keys), whether an answer stands for it:
    /// [`CullBook::verdict`] of many groups from one read.
    pub fn asked(&self, groups: &[Vec<&str>]) -> anyhow::Result<Vec<bool>> {
        let tx = self.db.begin_read()?;
        let (answers, asked) = (tx.open_table(ANSWERS)?, tx.open_table(ASKED)?);
        let mut read = Read::new();
        groups.iter().map(|keys| Ok(standing(&answers, &asked, keys, &mut read)?.is_some())).collect()
    }

    pub fn put_verdict(&self, verdict: &Verdict) -> anyhow::Result<()> {
        let tx = self.db.begin_write()?;
        keep(&mut tx.open_table(ANSWERS)?, &mut tx.open_table(ASKED)?, verdict)?;
        tx.commit()?;
        Ok(())
    }

    /// The model's view of each of these photos it has looked at, by key.
    pub fn views<'a>(&self, keys: impl IntoIterator<Item = &'a str>) -> anyhow::Result<HashMap<String, View>> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(VIEWS)?;
        let mut out = HashMap::new();
        for key in keys {
            // One that no longer reads is looked at again.
            if let Some(bytes) = table.get(key)?
                && let Ok(view) = postcard::from_bytes::<View>(bytes.value())
            {
                out.insert(key.to_owned(), view);
            }
        }
        Ok(out)
    }

    pub fn put_views(&self, views: &[(String, View)]) -> anyhow::Result<()> {
        if views.is_empty() {
            return Ok(());
        }
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(VIEWS)?;
            for (key, view) in views {
                table.insert(key.as_str(), postcard::to_stdvec(view)?.as_slice())?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Every photo picked as the one to keep of its group, by key.
    pub fn keepers(&self) -> anyhow::Result<HashSet<String>> {
        let tx = self.db.begin_read()?;
        let mut out = HashSet::new();
        for entry in tx.open_table(KEEPERS)?.iter()? {
            out.insert(entry?.0.value().to_owned());
        }
        Ok(out)
    }

    /// Picks (`true`) these photos, by key, or takes the pick back (`false`).
    pub fn set_keepers(&self, keys: &[String], kept: bool, at: i64) -> anyhow::Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(KEEPERS)?;
            for key in keys {
                if kept {
                    table.insert(key.as_str(), at)?;
                } else {
                    table.remove(key.as_str())?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_are_kept_until_taken_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cull.redb");
        let book = CullBook::open(&path, &Default::default()).unwrap();
        book.set_keepers(&["a".into(), "b".into()], true, 1).unwrap();
        book.set_keepers(&["a".into()], false, 2).unwrap();
        drop(book);
        let book = CullBook::open(&path, &Default::default()).unwrap();
        assert_eq!(book.keepers().unwrap(), HashSet::from(["b".to_owned()]));
    }

    /// An answer about the frames `sent`, asked at `at`: the last of them is the one to keep.
    fn said(sent: &[&str], at: i64) -> Verdict {
        let order = (0..sent.len() as u32).rev().collect();
        let said = NewVerdict { at, provider: "claude".into(), model: "m".into(), order, ..NewVerdict::default() };
        Verdict::new(sent.iter().map(|k| k.to_string()).collect(), said)
    }

    #[test]
    fn an_answer_stands_while_the_model_saw_every_photo_shown() {
        // Three frames asked about: all shown, fewer shown (a filter, one in the trash).
        assert!(stands(3, 3, 3) && stands(3, 2, 2));
        assert!(!stands(3, 3, 4), "a photo it never saw: asked again");
        assert!(!stands(3, 1, 1) && !stands(3, 1, 2), "one frame left of it tells nothing apart");
        // Shown as many as a question holds, it saw all it could: the rest follow.
        let most = pick::MAX_FRAMES;
        assert!(stands(most, most, most + 4) && stands(most, most - 3, most + 1));
        assert!(!stands(most - 1, most - 1, most + 4));
    }

    #[test]
    fn an_answer_is_found_from_the_frames_it_was_about() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cull.redb");
        let book = CullBook::open(&path, &Default::default()).unwrap();
        let first = said(&["b", "a", "c"], 5);
        book.put_verdict(&first).unwrap();
        assert_eq!(book.verdict(&["a", "b", "c"]).unwrap(), Some(first.clone()), "whatever the order");
        assert_eq!(book.verdict(&["c", "a"]).unwrap(), Some(first.clone()), "fewer of them shown");
        assert_eq!(book.verdict(&["a", "b", "c", "d"]).unwrap(), None, "a photo more is asked about again");
        assert_eq!(book.verdict(&["a"]).unwrap(), None);
        assert_eq!(book.verdict(&["x", "y"]).unwrap(), None);
        let groups = [vec!["a", "b"], vec!["a", "d"], vec!["x", "y"], vec!["c", "b", "a"]];
        assert_eq!(book.asked(&groups).unwrap(), [true, false, false, true]);

        // Asked again with the fourth: that answer is the group's, and the first is gone.
        let second = said(&["a", "b", "c", "d"], 9);
        book.put_verdict(&second).unwrap();
        assert_eq!(book.verdict(&["a", "b", "c", "d"]).unwrap(), Some(second.clone()));
        assert_eq!(book.verdict(&["a", "b", "c"]).unwrap(), Some(second.clone()));
        drop(book);
        let book = CullBook::open(&path, &Default::default()).unwrap();
        assert_eq!(book.verdict(&["b", "d"]).unwrap(), Some(second));
        let tx = book.db.begin_read().unwrap();
        let answers = redb::ReadableTableMetadata::len(&tx.open_table(ANSWERS).unwrap()).unwrap();
        assert_eq!(answers, 1, "no answer is left with nothing pointing to it");
    }

    #[test]
    fn an_answer_to_another_version_of_the_question_does_not_stand() {
        let dir = tempfile::tempdir().unwrap();
        let book = CullBook::open(&dir.path().join("cull.redb"), &Default::default()).unwrap();
        let old = Verdict { version: pick::VERSION + 1, ..said(&["a", "b"], 5) };
        book.put_verdict(&old).unwrap();
        assert_eq!(book.verdict(&["a", "b"]).unwrap(), None);
        assert_eq!(book.asked(&[vec!["a", "b"]]).unwrap(), [false]);
    }

    #[test]
    fn answers_kept_under_their_group_by_an_earlier_build_are_found_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cull.redb");
        let (early, late) = (said(&["a", "b"], 5), said(&["b", "a", "c"], 7));
        {
            let db = crate::db::open(&path, "cull", &Default::default()).unwrap();
            let tx = db.begin_write().unwrap();
            {
                let mut table = tx.open_table(BY_GROUP).unwrap();
                // The later answer comes first by key: the order asked is what counts.
                table.insert("a\nb\nc", postcard::to_stdvec(&late).unwrap().as_slice()).unwrap();
                table.insert("a\nb\nz", postcard::to_stdvec(&early).unwrap().as_slice()).unwrap();
            }
            tx.commit().unwrap();
        }
        let book = CullBook::open(&path, &Default::default()).unwrap();
        assert_eq!(book.verdict(&["a", "b", "c"]).unwrap(), Some(late.clone()));
        assert_eq!(book.verdict(&["a", "b"]).unwrap(), Some(late), "the later answer about them");
        let tx = book.db.begin_read().unwrap();
        assert!(!tx.list_tables().unwrap().any(|table| table.name() == BY_GROUP.name()), "read once");
    }

    #[test]
    fn a_view_is_kept_and_read_back_by_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cull.redb");
        let book = CullBook::open(&path, &Default::default()).unwrap();
        let view = View { version: 1, stamp: (10, 20), embedding: vec![0.25, -0.5], scores: [0.1, 0.2, 0.3, 0.4] };
        book.put_views(&[("a".into(), view.clone())]).unwrap();
        drop(book);
        let book = CullBook::open(&path, &Default::default()).unwrap();
        let read = book.views(["a", "missing"]).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read["a"], view);
    }
}
