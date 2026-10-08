//! Is Capture One running? It rewrites the catalog on save, so external writes made
//! while it is open are lost.

use std::process::Command;

const PROCESSES: [&str; 2] = ["CaptureOne.exe", "ImgCoreProcess.exe"];

/// The name of the first Capture One process found, if any.
pub(crate) fn running_capture_one() -> Option<String> {
    PROCESSES.iter().find(|name| is_running(name)).map(|s| (*s).to_owned())
}

#[cfg(windows)]
fn is_running(image: &str) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {image}"), "/NH", "/FO", "CSV"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).to_ascii_lowercase().contains(&image.to_ascii_lowercase()))
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn is_running(image: &str) -> bool {
    // Under WSL the Windows process list is reachable through tasklist.exe.
    Command::new("tasklist.exe")
        .args(["/FI", &format!("IMAGENAME eq {image}"), "/NH", "/FO", "CSV"])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).to_ascii_lowercase().contains(&image.to_ascii_lowercase()))
        .unwrap_or(false)
}
