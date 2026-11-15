//! Is Capture One running? It rewrites the catalog on save, so external writes made
//! while it is open are lost.

const PROCESSES: [&str; 2] = ["CaptureOne.exe", "ImgCoreProcess.exe"];

/// What holds writes back when the check cannot be made. It follows "locked by", and the
/// app's footer knows it by "Capture One" (with the space; process names have none).
pub(crate) const UNKNOWN: &str =
    "Capture One, which may be running (WSL cannot read Windows' process list: is Windows interop on?)";

/// The name of the first Capture One process found, if any.
#[cfg(windows)]
pub(crate) fn running_capture_one() -> Option<String> {
    // Read here, without starting a program; a list that cannot be read is taken as
    // Capture One closed, as it always was here.
    let running = windows_processes().unwrap_or_default();
    first_running(|name| Some(running.iter().any(|r| r.eq_ignore_ascii_case(name))))
}

/// The name of the first Capture One process found, if any. When the check cannot be
/// made (WSL without Windows interop), it says so: writes then wait, as if it were open.
#[cfg(not(windows))]
pub(crate) fn running_capture_one() -> Option<String> {
    if !c1model::platform::is_wsl() {
        // Plain Linux: Capture One cannot run on this machine, and one on another machine
        // (with the catalog on a share mounted here) cannot be seen from here: nothing
        // guards that case, as the README says.
        return None;
    }
    // Under WSL the Windows process list is reachable through tasklist.exe, at its usual
    // place even when the Windows folders are left off PATH. One call per name: asking
    // for one image is several times quicker than listing every process.
    let system32 = c1model::platform::wsl_mount_root().join("c").join("Windows").join("System32").join("tasklist.exe");
    let system32 = system32.to_string_lossy();
    first_running(|image| tasklist(&system32, image).or_else(|| tasklist("tasklist.exe", image)))
}

/// The first of [`PROCESSES`] that `is_running` finds, or [`UNKNOWN`] as soon as one
/// cannot be checked.
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
#[cfg(not(windows))]
fn tasklist(program: &str, image: &str) -> Option<bool> {
    let out = std::process::Command::new(program)
        .args(["/FI", &format!("IMAGENAME eq {image}"), "/NH", "/FO", "CSV"])
        .output()
        .ok()
        .filter(|out| out.status.success())?;
    let quoted = format!("\"{}\"", image.to_ascii_lowercase());
    Some(String::from_utf8_lossy(&out.stdout).to_ascii_lowercase().contains(&quoted))
}

/// The executable names of the processes running now, from a Toolhelp snapshot.
#[cfg(windows)]
fn windows_processes() -> Option<Vec<String>> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    // SAFETY: a snapshot of every process; the handle is closed below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut names = Vec::new();
    // SAFETY: an all-zero PROCESSENTRY32W is valid; dwSize is set as the API requires.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    // SAFETY: a live snapshot and an entry of the size given.
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while more {
        let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
        names.push(String::from_utf16_lossy(&entry.szExeFile[..len]));
        // SAFETY: as above.
        more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    // SAFETY: the snapshot opened above, closed once.
    unsafe { CloseHandle(snapshot) };
    Some(names)
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

    #[test]
    fn the_running_check_answers() {
        // Whatever runs here, the check must answer, and not with "cannot tell" off WSL.
        if !c1model::platform::is_wsl() {
            assert_ne!(running_capture_one().as_deref(), Some(UNKNOWN));
        }
    }

    #[cfg(windows)]
    #[test]
    fn this_process_is_in_the_windows_list() {
        let me = std::env::current_exe().unwrap();
        let me = me.file_name().unwrap().to_string_lossy();
        let names = windows_processes().expect("a process snapshot");
        assert!(names.iter().any(|n| n.eq_ignore_ascii_case(&me)), "{me} in {} processes", names.len());
    }
}
