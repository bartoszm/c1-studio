//! Opening the library's redb files: told how far it got, and stopped when asked.

use std::fmt::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use redb::{Builder, Database, DatabaseError};

/// Where opening a library has got.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OpenStep {
    /// Opening one of its parts ("store", "index", …).
    Part(&'static str),
    /// A part left open when the app last ended is being put right; `done` runs 0 to 1.
    Repairing { part: &'static str, done: f64 },
}

/// Opening a library was stopped ([`OpenControl::stop`]) before it was done.
#[derive(Debug, thiserror::Error)]
#[error("opening the library was stopped")]
pub struct Stopped;

/// What one part took to open, and whether it was repaired first.
type Took = (&'static str, Duration, bool);

/// How a library is opened: what it is told as it goes, and a flag that stops it between
/// parts (or mid-repair). Cloned, it shares the flag.
#[derive(Clone, Default)]
pub struct OpenControl {
    stop: Arc<AtomicBool>,
    progress: Option<Arc<dyn Fn(OpenStep) + Send + Sync>>,
    took: Arc<Mutex<Vec<Took>>>,
}

impl OpenControl {
    /// Tells `progress` where opening has got.
    pub fn new(progress: impl Fn(OpenStep) + Send + Sync + 'static) -> Self {
        Self { progress: Some(Arc::new(progress)), ..Self::default() }
    }

    /// Asks the open under way to stop: it ends with [`Stopped`].
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// The stop flag, for what the open makes (a library) to keep.
    pub(crate) fn flag(&self) -> Arc<AtomicBool> {
        self.stop.clone()
    }

    /// [`Stopped`] if asked to stop; otherwise tells the window `part` is next.
    pub(crate) fn part(&self, part: &'static str) -> anyhow::Result<()> {
        if self.stopped() {
            return Err(Stopped.into());
        }
        self.tell(OpenStep::Part(part));
        Ok(())
    }

    fn tell(&self, step: OpenStep) {
        if let Some(progress) = &self.progress {
            progress(step);
        }
    }

    /// Runs `work` as part `part`, timed.
    pub fn timed<T>(&self, part: &'static str, work: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
        self.part(part)?;
        let started = Instant::now();
        let out = work()?;
        self.took(part, started.elapsed(), false);
        Ok(out)
    }

    fn took(&self, part: &'static str, time: Duration, repaired: bool) {
        self.took.lock().unwrap().push((part, time, repaired));
    }

    /// How long the parts that took a while (a tenth of a second or more) took, the slow
    /// ones first, and which were repaired: "edits 1.2 s (repaired), store 0.3 s".
    pub fn report(&self) -> String {
        let mut took = self.took.lock().unwrap().clone();
        took.retain(|&(_, time, repaired)| repaired || time >= Duration::from_millis(100));
        took.sort_by_key(|&(_, time, _)| std::cmp::Reverse(time));
        let mut out = String::new();
        for (part, time, repaired) in took {
            if !out.is_empty() {
                out.push_str(", ");
            }
            let _ = write!(out, "{part} {:.1} s", time.as_secs_f64());
            if repaired {
                out.push_str(" (repaired)");
            }
        }
        out
    }
}

/// Opens (or makes) the redb file at `path` as part `part`. One the app did not close
/// (it ended mid-run) is repaired first, with the window told how far that got.
pub(crate) fn open(path: &Path, part: &'static str, ctl: &OpenControl) -> anyhow::Result<Database> {
    ctl.part(part)?;
    let started = Instant::now();
    // redb works out a new file's free space the same way it repairs one: only a file
    // that holds something was repaired.
    let existed = path.metadata().is_ok_and(|m| m.len() > 0);
    let repaired = Arc::new(AtomicBool::new(false));
    let mut builder = Builder::new();
    {
        let (ctl, repaired) = (ctl.clone(), repaired.clone());
        builder.set_repair_callback(move |session| {
            if ctl.stopped() {
                session.abort();
            }
            if existed {
                repaired.store(true, Ordering::SeqCst);
                ctl.tell(OpenStep::Repairing { part, done: session.progress() });
            }
        });
    }
    let db = match builder.create(path) {
        Err(DatabaseError::RepairAborted) => return Err(Stopped.into()),
        db => db.with_context(|| format!("opening {part} {}", path.display()))?,
    };
    ctl.took(part, started.elapsed(), repaired.load(Ordering::SeqCst));
    Ok(db)
}
