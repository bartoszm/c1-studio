//! The optional XMP export after a write: sidecars beside RAWs, and XMP embedded in JPEG,
//! TIFF and DNG files when changing originals is allowed.
//!
//! Lightroom reads XMP only on import or "Read Metadata from Files", so this is an export,
//! not a sync. A failure is reported, never undoes the write to the store.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use c1model::Photo;

use crate::definition::Definition;
use crate::meta::{FileKind, embed, xmp};

/// Writes the XMP for each `(photo as the user sees it, its file, the file's kind)`, as
/// `def` says (nothing unless `def.write_xmp`). Returns what could not be written, with why.
pub(crate) fn export(def: &Definition, photos: &[(Photo, PathBuf, FileKind)]) -> Vec<String> {
    if !def.write_xmp {
        return Vec::new();
    }
    let problem = |(photo, path, kind): &(Photo, PathBuf, FileKind)| {
        export_one(def, photo, path, *kind).err().map(|why| format!("{}: {why}", path.display()))
    };
    photos.iter().filter_map(problem).collect()
}

fn export_one(def: &Definition, photo: &Photo, path: &Path, kind: FileKind) -> Result<(), String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if kind == FileKind::Png {
        return Ok(());
    }
    if kind.embeds_xmp(ext) {
        // Lightroom ignores a sidecar next to these, so the choice is the packet or nothing
        return if def.modify_originals { embed_in(photo, path, kind) } else { Ok(()) };
    }
    write_sidecar(def, photo, path)
}

/// Writes (or merges into) the sidecar of the RAW at `path`.
fn write_sidecar(def: &Definition, photo: &Photo, path: &Path) -> Result<(), String> {
    let sidecar = def.sidecar.sidecar(path);
    let existing = match fs::read(&sidecar) {
        Ok(bytes) => Some(String::from_utf8(bytes).map_err(|_| format!("{} is not UTF-8 text", sidecar.display()))?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("reading {}: {e}", sidecar.display())),
    };
    let xml = match existing.as_deref().filter(|x| !x.trim().is_empty()) {
        // an existing sidecar that cannot be edited is left alone: it may hold someone's edits
        Some(x) => xmp::try_render(photo, x).map_err(|e| format!("{}: {e}", sidecar.display()))?,
        None => xmp::render(photo, None),
    };
    if existing.as_deref() == Some(&xml) {
        return Ok(());
    }
    write_atomic(&sidecar, xml.as_bytes(), None).map_err(|e| format!("writing {}: {e}", sidecar.display()))
}

/// Writes the XMP into the JPEG, TIFF or DNG at `path`.
fn embed_in(photo: &Photo, path: &Path, kind: FileKind) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|e| format!("reading: {e}"))?;
    let jpeg = kind == FileKind::Jpeg;
    let existing = if jpeg { xmp::packet_from_jpeg(&bytes) } else { xmp::packet_from_tiff(&bytes) };
    let xml = xmp::render(photo, existing.as_deref());
    let out = if jpeg { embed::embed_jpeg(&bytes, &xml)? } else { embed::embed_tiff(&bytes, &xml)? };
    if out == bytes {
        return Ok(());
    }
    let permissions = fs::metadata(path).ok().map(|m| m.permissions());
    write_atomic(path, &out, permissions).map_err(|e| format!("writing: {e}"))
}

/// Writes `bytes` to a temporary file beside `path`, then moves it over `path`.
fn write_atomic(path: &Path, bytes: &[u8], permissions: Option<fs::Permissions>) -> io::Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let unique = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let tmp = path.with_file_name(format!(".{name}.{unique:x}.tmp"));
    let write = || -> io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if let Some(permissions) = permissions {
            fs::set_permissions(&tmp, permissions)?;
        }
        fs::rename(&tmp, path)
    };
    write().inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::definition::SidecarNaming;
    use crate::meta::embed::tests::{plain_jpeg, plain_tiff};
    use crate::meta::xmp::tests::{LIGHTROOM, full_photo};

    fn definition(write_xmp: bool, modify_originals: bool) -> Definition {
        let mut def = Definition::new("test", &[]);
        def.write_xmp = write_xmp;
        def.modify_originals = modify_originals;
        def
    }

    fn entry(path: &Path, kind: FileKind) -> (Photo, PathBuf, FileKind) {
        (full_photo(), path.to_path_buf(), kind)
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn nothing_without_write_xmp() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("a.raf");
        fs::write(&raw, b"raw").unwrap();
        assert!(export(&definition(false, true), &[entry(&raw, FileKind::Raw)]).is_empty());
        assert_eq!(files_in(dir.path()), ["a.raf"]);
    }

    #[test]
    fn a_raw_gets_a_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("DSCF0001.RAF");
        fs::write(&raw, b"raw").unwrap();
        assert!(export(&definition(true, false), &[entry(&raw, FileKind::Raw)]).is_empty());
        assert_eq!(files_in(dir.path()), ["DSCF0001.RAF", "DSCF0001.xmp"]);
        let fields = xmp::parse(&fs::read_to_string(dir.path().join("DSCF0001.xmp")).unwrap());
        assert_eq!(fields.rating, Some(4));
        assert_eq!(fields.iptc.title.as_deref(), Some("Dom nad rzeką"));
        assert_eq!(fs::read(&raw).unwrap(), b"raw");

        let mut named = definition(true, false);
        named.sidecar = SidecarNaming::Name;
        export(&named, &[entry(&raw, FileKind::Raw)]);
        assert!(dir.path().join("DSCF0001.RAF.xmp").exists());
    }

    #[test]
    fn an_existing_sidecar_is_merged_into() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("a.nef");
        fs::write(&raw, b"raw").unwrap();
        let sidecar = dir.path().join("a.xmp");
        fs::write(&sidecar, LIGHTROOM).unwrap();
        assert!(export(&definition(true, false), &[entry(&raw, FileKind::Raw)]).is_empty());
        let xml = fs::read_to_string(&sidecar).unwrap();
        assert!(xml.contains("crs:Exposure2012=\"+0.35\"") && xml.contains("<crs:ToneCurvePV2012>"));
        assert_eq!(xmp::parse(&xml).rating, Some(4));
        assert_eq!(files_in(dir.path()), ["a.nef", "a.xmp"]);

        // again: nothing to change, nothing rewritten
        let modified = fs::metadata(&sidecar).unwrap().modified().unwrap();
        export(&definition(true, false), &[entry(&raw, FileKind::Raw)]);
        assert_eq!(fs::metadata(&sidecar).unwrap().modified().unwrap(), modified);
    }

    #[test]
    fn a_sidecar_that_cannot_be_edited_is_kept_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("a.nef");
        fs::write(&raw, b"raw").unwrap();
        fs::write(dir.path().join("a.xmp"), "<broken").unwrap();
        let problems = export(&definition(true, false), &[entry(&raw, FileKind::Raw)]);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].starts_with(&format!("{}: ", raw.display())), "{problems:?}");
        assert_eq!(fs::read_to_string(dir.path().join("a.xmp")).unwrap(), "<broken");
        assert_eq!(files_in(dir.path()), ["a.nef", "a.xmp"]);
    }

    #[test]
    fn originals_are_only_changed_when_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let jpeg = dir.path().join("a.jpg");
        let tiff = dir.path().join("b.tif");
        let png = dir.path().join("c.png");
        let (original_jpeg, original_tiff) = (plain_jpeg(), plain_tiff());
        fs::write(&jpeg, &original_jpeg).unwrap();
        fs::write(&tiff, &original_tiff).unwrap();
        fs::write(&png, b"png").unwrap();
        let photos = [entry(&jpeg, FileKind::Jpeg), entry(&tiff, FileKind::Tiff), entry(&png, FileKind::Png)];

        assert!(export(&definition(true, false), &photos).is_empty());
        assert_eq!(fs::read(&jpeg).unwrap(), original_jpeg);
        assert_eq!(fs::read(&tiff).unwrap(), original_tiff);
        assert_eq!(files_in(dir.path()), ["a.jpg", "b.tif", "c.png"]);

        assert!(export(&definition(true, true), &photos).is_empty());
        let packet = xmp::packet_from_jpeg(&fs::read(&jpeg).unwrap()).unwrap();
        assert_eq!(xmp::parse(&packet).keywords, full_photo().keywords);
        assert!(image::open(&jpeg).is_ok());
        let packet = xmp::packet_from_tiff(&fs::read(&tiff).unwrap()).unwrap();
        assert_eq!(xmp::parse(&packet).rating, Some(4));
        assert!(image::open(&tiff).is_ok());
        assert_eq!(fs::read(&png).unwrap(), b"png");
        // no sidecars, no leftovers
        assert_eq!(files_in(dir.path()), ["a.jpg", "b.tif", "c.png"]);
    }

    #[test]
    fn an_embedded_packet_is_merged_and_a_dng_is_embedded() {
        let dir = tempfile::tempdir().unwrap();
        let jpeg = dir.path().join("a.jpg");
        fs::write(&jpeg, embed::embed_jpeg(&plain_jpeg(), LIGHTROOM).unwrap()).unwrap();
        let dng = dir.path().join("b.DNG");
        fs::write(&dng, plain_tiff()).unwrap();
        let photos = [entry(&jpeg, FileKind::Jpeg), entry(&dng, FileKind::Raw)];
        assert!(export(&definition(true, true), &photos).is_empty());
        let packet = xmp::packet_from_jpeg(&fs::read(&jpeg).unwrap()).unwrap();
        assert!(packet.contains("crs:Exposure2012=\"+0.35\""));
        assert!(xmp::packet_from_tiff(&fs::read(&dng).unwrap()).is_some());
        assert_eq!(files_in(dir.path()), ["a.jpg", "b.DNG"]);
    }

    #[test]
    fn failures_are_reported_per_file() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.jpg");
        fs::write(&bad, b"not a jpeg").unwrap();
        let missing = dir.path().join("missing.tif");
        let problems = export(&definition(true, true), &[entry(&bad, FileKind::Jpeg), entry(&missing, FileKind::Tiff)]);
        assert_eq!(problems.len(), 2);
        assert!(problems[0].starts_with(&format!("{}: ", bad.display())));
        assert!(problems[1].starts_with(&format!("{}: ", missing.display())));
        assert_eq!(fs::read(&bad).unwrap(), b"not a jpeg");
    }
}
