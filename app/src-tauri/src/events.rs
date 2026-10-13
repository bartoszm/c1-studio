//! The events log: what happened in the background (reading Capture One, writing to it,
//! making thumbnails) and what went wrong, as plain text beside the library, for the user
//! to open. One per catalog; the older half is kept as `events.old.log`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Past this, the log moves aside and a new one starts.
const MAX_BYTES: u64 = 4 << 20;

pub struct Log {
    path: PathBuf,
    lock: Mutex<()>,
}

impl Log {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), lock: Mutex::new(()) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends an event with the time; `details` follow it, one per line, indented. A log
    /// that cannot be written is let go: it must never stop the work it reports on.
    pub fn event(&self, text: &str, details: &[String]) {
        let _one = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if std::fs::metadata(&self.path).is_ok_and(|m| m.len() > MAX_BYTES) {
            let _ = std::fs::rename(&self.path, self.path.with_extension("old.log"));
        }
        let mut out = format!("{}  {text}\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
        for line in details {
            out.push_str("    ");
            out.push_str(line);
            out.push('\n');
        }
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = file.write_all(out.as_bytes());
        }
    }

    /// Opens the log in the system's viewer for text files (it is made first if empty).
    pub fn open(&self) -> std::io::Result<()> {
        if !self.path.is_file() {
            self.event("Events log started", &[]);
        }
        #[cfg(windows)]
        let opener = "explorer";
        #[cfg(target_os = "macos")]
        let opener = "open";
        #[cfg(all(unix, not(target_os = "macos")))]
        let opener = "xdg-open";
        // Explorer reports failure even when it opened the file; only starting it counts.
        std::process::Command::new(opener).arg(&self.path).spawn().map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_append_with_their_details_indented() {
        let dir = tempfile::tempdir().unwrap();
        let log = Log::new(dir.path().join("events.log"));
        log.event("one", &[]);
        log.event("two", &["a".into(), "b".into()]);
        let text = std::fs::read_to_string(log.path()).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 4);
        assert!(lines[0].ends_with("  one"));
        assert!(lines[1].ends_with("  two"));
        assert_eq!(&lines[2..], ["    a", "    b"]);
    }
}
