//! Is Capture One running? It rewrites the catalog on save, so external writes made
//! while it is open are lost.

use std::process::Command;

const PROCESSES: [&str; 2] = ["CaptureOne.exe", "ImgCoreProcess.exe"];

/// What holds writes back when the check cannot be made. It follows "locked by", and the
/// app's footer knows it by "Capture One" (with the space; process names have none).
pub(crate) const UNKNOWN: &str =
    "Capture One, which may be running (WSL cannot read Windows' process list: is Windows interop on?)";

/// The name of the first Capture One process found, if any. When the check cannot be
/// made (WSL without Windows interop), it says so: writes then wait, as if it were open.
pub(crate) fn running_capture_one() -> Option<String> {
    first_running(is_running)
}

/// [`running_capture_one`] with the per-process check given.
fn first_running(is_running: impl Fn(&str) -> Option<bool>) -> Option<String> {
    for name in PROCESSES {
        match is_running(name) {
            Some(true) => return Some(name.to_owned()),
            Some(false) => {}
            None => return Some(UNKNOWN.to_owned()),
        }
    }
    None
}

/// Whether `tasklist` lists the image; `None` when it could not be asked.
fn tasklist(program: &str, image: &str) -> Option<bool> {
    let mut command = Command::new(program);
    command.args(["/FI", &format!("IMAGENAME eq {image}"), "/NH", "/FO", "CSV"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let out = command.output().ok().filter(|out| out.status.success())?;
    Some(String::from_utf8_lossy(&out.stdout).to_ascii_lowercase().contains(&image.to_ascii_lowercase()))
}

#[cfg(windows)]
fn is_running(image: &str) -> Option<bool> {
    // A tasklist that does not answer is taken as Capture One closed, as it always was here.
    Some(tasklist("tasklist", image).unwrap_or(false))
}

#[cfg(not(windows))]
fn is_running(image: &str) -> Option<bool> {
    if !c1model::platform::is_wsl() {
        // Plain Linux: Capture One cannot run on this machine, and one on another machine
        // (with the catalog on a share mounted here) cannot be seen from here: nothing
        // guards that case, as the README says.
        return Some(false);
    }
    // Under WSL the Windows process list is reachable through tasklist.exe, at its usual
    // place even when the Windows folders are left off PATH.
    let system32 = c1model::platform::wsl_mount_root().join("c").join("Windows").join("System32").join("tasklist.exe");
    tasklist(&system32.to_string_lossy(), image).or_else(|| tasklist("tasklist.exe", image))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_check_that_cannot_be_made_holds_writes_back() {
        assert_eq!(first_running(|_| Some(false)), None);
        assert_eq!(first_running(|n| Some(n == "ImgCoreProcess.exe")).as_deref(), Some("ImgCoreProcess.exe"));
        assert_eq!(first_running(|_| None).as_deref(), Some(UNKNOWN));
        // The footer tells it from a program that has the catalog open by its name.
        assert!(UNKNOWN.starts_with("Capture One,"));
    }
}
