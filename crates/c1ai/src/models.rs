//! The files of a model run on this computer: where they are kept, whether they are all
//! there, and fetching them. Nothing is fetched unless asked: a model is a download of
//! over a gigabyte.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};

use crate::AiError;

/// One file of a model, as its repository holds it.
#[derive(Debug)]
pub struct ModelFile {
    /// Its path in the repository and under the model's folder.
    pub path: &'static str,
    pub size: u64,
    /// Lowercase hex.
    pub sha256: &'static str,
}

/// A model kept as files: a Hugging Face repository at one revision.
#[derive(Debug)]
pub struct Model {
    /// The folder it is kept in, and the model a suggestion is recorded with.
    pub id: &'static str,
    pub name: &'static str,
    pub repo: &'static str,
    pub revision: &'static str,
    pub files: &'static [ModelFile],
}

/// SigLIP 2 base, patch 16, 224 px (Google, Apache-2.0), as Immich exports it to ONNX: an
/// image tower and a multilingual text tower with one embedding space.
pub const SIGLIP2: Model = Model {
    id: "siglip2-b16-224",
    name: "SigLIP 2 B/16",
    repo: "immich-app/ViT-B-16-SigLIP2__webli",
    revision: "5f3fc29104ebcf51e9d893af57604badd51a3ea6",
    files: &[
        ModelFile {
            path: "visual/model.onnx",
            size: 371_674_292,
            sha256: "fee10c729875dd203d94f396ac3d664e301d39c977ef6d92e7f467a2a716f0b0",
        },
        ModelFile {
            path: "textual/model.onnx",
            size: 1_129_435_819,
            sha256: "fc9991b415d7d0fac7ed540e875fd08a647c8a1abba99f12542531b004c4b68f",
        },
        ModelFile {
            path: "textual/tokenizer.json",
            size: 34_362_885,
            sha256: "220c63d496e0c14e63eb656c91e0215e926202e4c74b1f089e09f1920d779b04",
        },
    ],
};

impl Model {
    /// Its folder under `root`, the folder all models are kept in.
    pub fn dir(&self, root: &Path) -> PathBuf {
        root.join(self.id)
    }

    /// The whole download, in bytes.
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// Every file is there at its full size. Hashes are checked when a file is fetched,
    /// not here: reading a gigabyte takes seconds.
    pub fn installed(&self, root: &Path) -> bool {
        let dir = self.dir(root);
        self.files.iter().all(|f| has(&dir.join(f.path), f.size))
    }

    fn url(&self, file: &ModelFile) -> String {
        format!("https://huggingface.co/{}/resolve/{}/{}", self.repo, self.revision, file.path)
    }

    /// Fetches the files not there yet into the model's folder under `root`, telling
    /// `progress` the bytes there so far and the total. A file is written beside its
    /// place and moved in once its hash matches, so one cut short is never taken for
    /// whole. `stop` ends it with [`AiError::Stopped`]; files already fetched stay.
    pub fn download(&self, root: &Path, stop: &AtomicBool, progress: &dyn Fn(u64, u64)) -> Result<(), AiError> {
        let dir = self.dir(root);
        let total = self.size();
        let mut done: u64 = 0;
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        for file in self.files {
            let to = dir.join(file.path);
            if has(&to, file.size) {
                done += file.size;
                progress(done, total);
                continue;
            }
            let folder = to.parent().unwrap_or(&dir);
            fs::create_dir_all(folder).map_err(|e| disk(folder, e))?;
            let mut response = agent.get(self.url(file)).call().map_err(|e| AiError::Network(e.to_string()))?;
            let status = response.status().as_u16();
            if status != 200 {
                return Err(AiError::Http { status, message: format!("fetching {}", file.path), retry_after: None });
            }
            let part = to.with_extension("part");
            let hash = copy(response.body_mut().as_reader(), &part, stop, &mut |n| progress(done + n, total));
            let hash = match hash {
                Ok(hash) => hash,
                Err(e) => {
                    let _ = fs::remove_file(&part);
                    return Err(e);
                }
            };
            if hash != file.sha256 {
                let _ = fs::remove_file(&part);
                return Err(AiError::BadReply(format!("{} is not the file expected: its hash differs", file.path)));
            }
            fs::rename(&part, &to).map_err(|e| disk(&to, e))?;
            done += file.size;
            progress(done, total);
        }
        Ok(())
    }

    /// Removes the model's folder under `root`, with all in it.
    pub fn remove(&self, root: &Path) -> Result<(), AiError> {
        let dir = self.dir(root);
        match fs::remove_dir_all(&dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(disk(&dir, e)),
            _ => Ok(()),
        }
    }
}

fn has(path: &Path, size: u64) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() == size)
}

fn disk(path: &Path, e: std::io::Error) -> AiError {
    AiError::Local(format!("{}: {e}", path.display()))
}

/// Writes what `from` gives to `to`, telling `wrote` the bytes so far; returns their
/// SHA-256 in lowercase hex.
fn copy(mut from: impl Read, to: &Path, stop: &AtomicBool, wrote: &mut dyn FnMut(u64)) -> Result<String, AiError> {
    let mut file = fs::File::create(to).map_err(|e| disk(to, e))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut written: u64 = 0;
    loop {
        if stop.load(Ordering::SeqCst) {
            return Err(AiError::Stopped);
        }
        let n = from.read(&mut buffer).map_err(|e| AiError::Network(e.to_string()))?;
        if n == 0 {
            break;
        }
        file.write_all(&buffer[..n]).map_err(|e| disk(to, e))?;
        hasher.update(&buffer[..n]);
        written += n as u64;
        wrote(written);
    }
    file.flush().map_err(|e| disk(to, e))?;
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: Model = Model {
        id: "small",
        name: "Small",
        repo: "nobody/small",
        revision: "abc",
        files: &[
            ModelFile { path: "a/one.bin", size: 3, sha256: "" },
            ModelFile { path: "two.bin", size: 2, sha256: "" },
        ],
    };

    #[test]
    fn a_model_is_installed_when_every_file_is_there_whole() {
        let root = tempfile::tempdir().unwrap();
        assert!(!SMALL.installed(root.path()));
        let dir = SMALL.dir(root.path());
        fs::create_dir_all(dir.join("a")).unwrap();
        fs::write(dir.join("a/one.bin"), b"123").unwrap();
        assert!(!SMALL.installed(root.path()), "one file is missing");
        fs::write(dir.join("two.bin"), b"1").unwrap();
        assert!(!SMALL.installed(root.path()), "one file is cut short");
        fs::write(dir.join("two.bin"), b"12").unwrap();
        assert!(SMALL.installed(root.path()));
        assert_eq!(SMALL.size(), 5);
        SMALL.remove(root.path()).unwrap();
        assert!(!SMALL.installed(root.path()));
        SMALL.remove(root.path()).unwrap();
    }

    #[test]
    fn a_file_is_copied_with_its_hash_and_stops_when_asked() {
        let root = tempfile::tempdir().unwrap();
        let to = root.path().join("x.part");
        let mut last = 0;
        let hash = copy(&b"abc"[..], &to, &AtomicBool::new(false), &mut |n| last = n).unwrap();
        assert_eq!(hash, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!((last, fs::read(&to).unwrap()), (3, b"abc".to_vec()));
        let stopped = copy(&b"abc"[..], &to, &AtomicBool::new(true), &mut |_| {});
        assert!(matches!(stopped, Err(AiError::Stopped)));
    }

    #[test]
    fn the_address_names_the_pinned_revision() {
        assert_eq!(
            SIGLIP2.url(&SIGLIP2.files[0]),
            "https://huggingface.co/immich-app/ViT-B-16-SigLIP2__webli/resolve/5f3fc29104ebcf51e9d893af57604badd51a3ea6/visual/model.onnx"
        );
        assert!(SIGLIP2.files.iter().all(|f| f.sha256.len() == 64));
    }
}
