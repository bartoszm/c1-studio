//! Where the app runs, and how the Windows paths a catalog stores reach files here.
//!
//! Capture One runs on Windows (or macOS) and records originals as `H:\fotos\x.RAF`. On
//! Windows that is the path. Under WSL the drive is mounted at `/mnt/h`; on other Linux
//! machines the user says where each volume is in `volumes.json` beside the app's
//! settings: `{"H:": "/media/photos", "\\\\nas\\photos": "/mnt/nas"}`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Running under the Windows Subsystem for Linux?
pub fn is_wsl() -> bool {
    static WSL: OnceLock<bool> = OnceLock::new();
    *WSL.get_or_init(|| {
        cfg!(target_os = "linux")
            && (std::env::var_os("WSL_DISTRO_NAME").is_some()
                || Path::new("/proc/sys/fs/binfmt_misc/WSLInterop").exists()
                || std::fs::read_to_string("/proc/sys/kernel/osrelease")
                    .is_ok_and(|r| r.to_ascii_lowercase().contains("microsoft")))
    })
}

/// Where WSL mounts the Windows drives: `[automount] root` in `/etc/wsl.conf`, `/mnt/`
/// by default.
pub fn wsl_mount_root() -> PathBuf {
    let conf = std::fs::read_to_string("/etc/wsl.conf").unwrap_or_default();
    PathBuf::from(automount_root(&conf).unwrap_or_else(|| "/mnt/".to_owned()))
}

fn automount_root(conf: &str) -> Option<String> {
    let mut in_automount = false;
    for line in conf.lines().map(str::trim) {
        if line.starts_with('[') {
            in_automount = line.eq_ignore_ascii_case("[automount]");
        } else if in_automount
            && let Some((key, value)) = line.split_once('=')
            && key.trim().eq_ignore_ascii_case("root")
        {
            let value = value.split('#').next().unwrap_or_default().trim().trim_matches('"');
            return (!value.is_empty()).then(|| value.to_owned());
        }
    }
    None
}

/// The user's volume map: `<config root>/volumes.json`.
pub fn volumes_file() -> PathBuf {
    crate::provider::config_root().join("volumes.json")
}

/// The volume map, read once per run: Windows roots (`H:`, `\\server\share`) to local
/// folders. A missing or broken file is an empty map.
fn volumes() -> &'static [(String, PathBuf)] {
    static VOLUMES: OnceLock<Vec<(String, PathBuf)>> = OnceLock::new();
    VOLUMES.get_or_init(|| {
        let text = std::fs::read_to_string(volumes_file()).unwrap_or_default();
        parse_volumes(&text)
    })
}

fn parse_volumes(text: &str) -> Vec<(String, PathBuf)> {
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str(text) else { return Vec::new() };
    let mut volumes: Vec<(String, PathBuf)> = map
        .into_iter()
        .filter_map(|(root, local)| Some((root_key(&root), PathBuf::from(local.as_str()?))))
        .filter(|(root, _)| !root.is_empty())
        .collect();
    // The longest root first, so `\\nas\photos\2026` wins over `\\nas\photos`.
    volumes.sort_by_key(|(root, _)| std::cmp::Reverse(root.len()));
    volumes
}

/// A Windows root as compared: lower case, backslashes, no trailing separator.
fn root_key(root: &str) -> String {
    root.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// Is this a Windows path (`H:\…`, `\\server\…`) rather than one of this machine's?
pub fn is_windows_path(path: &str) -> bool {
    let b = path.as_bytes();
    (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':') || path.starts_with("\\\\")
}

/// Where a path from the catalog is on this machine, or `None` when it cannot be reached
/// from here (a share nobody mapped, say). Paths that are already local pass through.
pub fn local_path(path: &str) -> Option<PathBuf> {
    if cfg!(windows) || !is_windows_path(path) {
        return Some(PathBuf::from(path));
    }
    let wsl = is_wsl().then(wsl_mount_root);
    map_path(path, volumes(), wsl.as_deref())
}

/// [`local_path`] with its inputs given: the volume map, then WSL's drive mounts.
fn map_path(path: &str, volumes: &[(String, PathBuf)], wsl_root: Option<&Path>) -> Option<PathBuf> {
    let key = path.replace('/', "\\");
    for (root, local) in volumes {
        let n = root.len();
        // A whole component: `H:` matches `H:\x` and `H:` but not `HX:`.
        if key.is_char_boundary(n)
            && key[..n].to_lowercase() == *root
            && key[n..].chars().next().is_none_or(|c| c == '\\')
        {
            return Some(join_windows(local, &key[n..]));
        }
    }
    let b = key.as_bytes();
    if let Some(root) = wsl_root
        && b.len() >= 2
        && b[1] == b':'
        && b[0].is_ascii_alphabetic()
    {
        let drive = (b[0] as char).to_ascii_lowercase().to_string();
        return Some(join_windows(&root.join(drive), &key[2..]));
    }
    None
}

fn join_windows(base: &Path, rest: &str) -> PathBuf {
    let mut path = base.to_path_buf();
    path.extend(rest.split('\\').filter(|s| !s.is_empty()));
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automount_root_is_read_from_its_section() {
        assert_eq!(automount_root("[boot]\nsystemd=true\n"), None);
        assert_eq!(automount_root("[automount]\nroot = /windows/ # here\n"), Some("/windows/".into()));
        assert_eq!(automount_root("[network]\nroot=/no\n[automount]\nenabled=true\n"), None);
        assert_eq!(automount_root("[Automount]\nroot=\"/w/\"\n"), Some("/w/".into()));
    }

    #[test]
    fn windows_paths_are_told_apart() {
        assert!(is_windows_path(r"H:\fotos\A.RAF"));
        assert!(is_windows_path("c:/x"));
        assert!(is_windows_path(r"\\nas\photos\A.RAF"));
        assert!(!is_windows_path("/mnt/h/fotos/A.RAF"));
        assert!(!is_windows_path("fotos"));
    }

    #[test]
    fn wsl_maps_drive_letters() {
        let root = Path::new("/mnt/");
        assert_eq!(map_path(r"H:\fotos\2026\A.RAF", &[], Some(root)), Some(PathBuf::from("/mnt/h/fotos/2026/A.RAF")));
        assert_eq!(map_path("c:/x/y.jpg", &[], Some(Path::new("/windows"))), Some(PathBuf::from("/windows/c/x/y.jpg")));
        assert_eq!(map_path(r"H:\A.RAF", &[], None), None);
        // A share has no mount point of its own.
        assert_eq!(map_path(r"\\nas\photos\A.RAF", &[], Some(root)), None);
    }

    #[test]
    fn the_volume_map_comes_first() {
        let volumes = parse_volumes(
            r#"{"H:": "/media/photos", "\\\\NAS\\photos": "/srv/nas", "\\\\nas\\photos\\2026": "/srv/2026", "bad": 3}"#,
        );
        assert_eq!(volumes.len(), 3);
        let root = Some(Path::new("/mnt/"));
        assert_eq!(map_path(r"h:\fotos\A.RAF", &volumes, root), Some(PathBuf::from("/media/photos/fotos/A.RAF")));
        assert_eq!(map_path(r"\\nas\Photos\x\A.RAF", &volumes, None), Some(PathBuf::from("/srv/nas/x/A.RAF")));
        assert_eq!(map_path(r"\\nas\photos\2026\A.RAF", &volumes, None), Some(PathBuf::from("/srv/2026/A.RAF")));
        // `\\nas\photos2` is another share, not a folder of `\\nas\photos`.
        assert_eq!(map_path(r"\\nas\photos2\A.RAF", &volumes, None), None);
        // Drives not in the map still go through WSL.
        assert_eq!(map_path(r"D:\A.RAF", &volumes, root), Some(PathBuf::from("/mnt/d/A.RAF")));
        assert!(parse_volumes("not json").is_empty());
    }

    #[test]
    fn local_paths_pass_through() {
        assert_eq!(local_path("/home/me/A.RAF"), Some(PathBuf::from("/home/me/A.RAF")));
        if cfg!(windows) {
            assert_eq!(local_path(r"H:\A.RAF"), Some(PathBuf::from(r"H:\A.RAF")));
        }
    }
}
