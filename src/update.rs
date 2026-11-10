//! In-game updates. At start-up a background thread asks GitHub which
//! release is the latest; if it is newer than this build the menu offers it.
//! Accepting downloads the new game beside the running one, swaps the two
//! files and starts the new one.
//!
//! The download is done by `curl`, which ships with Windows 10 and later (and
//! nearly every Linux), so the game needs no HTTPS library of its own. Both
//! requests go to github.com over HTTPS and carry nothing about the player.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, channel};

const REPO: &str = "https://github.com/nissi15/breakpoint-billiards";
const FILE: &str = "breakpoint-billiards.exe";

pub enum State {
    /// Nothing to offer (not checked, up to date, or offline).
    Idle,
    Checking(Receiver<Option<String>>),
    /// A newer version exists; holds its number, like "1.3.0".
    Available(String),
    Downloading(String, Receiver<Result<(), String>>),
    Failed(String),
    /// The new game is in place: start it and close this one.
    Ready,
}

pub struct Updater {
    pub state: State,
}

/// This build's version. BPB_FAKE_VERSION overrides it, to test updating.
pub fn current() -> String {
    std::env::var("BPB_FAKE_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string())
}

fn parts(v: &str) -> Vec<u32> {
    v.trim().trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// Is version `a` newer than version `b`?
pub fn newer(a: &str, b: &str) -> bool {
    let (a, b) = (parts(a), parts(b));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// The version named by the redirect that `/releases/latest` answers with.
fn version_in_headers(headers: &str) -> Option<String> {
    let line = headers.lines().find(|l| l.to_ascii_lowercase().starts_with("location:"))?;
    let v = line.rsplit_once("/tag/v")?.1.trim();
    let ok = !v.is_empty() && v.len() <= 20 && v.chars().all(|c| c.is_ascii_digit() || c == '.');
    ok.then(|| v.to_string())
}

fn curl() -> Command {
    let mut c = Command::new("curl");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console window flashes up behind the game.
        c.creation_flags(0x0800_0000);
    }
    c
}

fn latest() -> Option<String> {
    let out = curl()
        .args(["-sI", "-m", "10", &format!("{REPO}/releases/latest")])
        .output()
        .ok()?;
    version_in_headers(&String::from_utf8_lossy(&out.stdout))
}

fn beside(exe: &Path, ext: &str) -> PathBuf {
    exe.with_extension(ext)
}

/// Download `version` and put it in place of the running game. A running
/// program cannot be overwritten on Windows, but it can be renamed, so the
/// old file steps aside and is deleted the next time the game starts.
fn install(version: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let fresh = beside(&exe, "new");
    let old = beside(&exe, "old");
    let _ = std::fs::remove_file(&fresh);
    let url = format!("{REPO}/releases/download/v{version}/{FILE}");
    let status = curl()
        .args(["-sfL", "-m", "600", "-o"])
        .arg(&fresh)
        .arg(&url)
        .status()
        .map_err(|_| "Could not start the download".to_string())?;
    if !status.success() {
        let _ = std::fs::remove_file(&fresh);
        return Err("The download failed - check your connection".into());
    }
    // Make sure it really is a program and not an error page.
    let head = std::fs::read(&fresh).map_err(|e| e.to_string())?;
    if head.len() < 500_000 || !head.starts_with(b"MZ") {
        let _ = std::fs::remove_file(&fresh);
        return Err("The download was incomplete".into());
    }
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|_| "Could not replace the game file".to_string())?;
    if std::fs::rename(&fresh, &exe).is_err() {
        // Put the old game back so nothing is lost.
        let _ = std::fs::rename(&old, &exe);
        return Err("Could not replace the game file".into());
    }
    Ok(())
}

impl Updater {
    /// `check` is false for automated runs, which never look for updates.
    pub fn new(check: bool) -> Updater {
        if !check {
            return Updater { state: State::Idle };
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            // Tidy up after a previous update; that copy of the game may
            // still be closing, so try for a moment.
            if let Ok(exe) = std::env::current_exe() {
                let old = beside(&exe, "old");
                for _ in 0..10 {
                    if !old.exists() || std::fs::remove_file(&old).is_ok() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(300));
                }
            }
            let found = latest().filter(|v| newer(v, &current()));
            let _ = tx.send(found);
        });
        Updater { state: State::Checking(rx) }
    }

    /// Move background work along; call once a frame.
    pub fn poll(&mut self) {
        match &self.state {
            State::Checking(rx) => match rx.try_recv() {
                Ok(Some(v)) => self.state = State::Available(v),
                Ok(None) | Err(std::sync::mpsc::TryRecvError::Disconnected) => self.state = State::Idle,
                Err(_) => {}
            },
            State::Downloading(_, rx) => match rx.try_recv() {
                Ok(Ok(())) => self.state = State::Ready,
                Ok(Err(e)) => self.state = State::Failed(e),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.state = State::Failed("The update stopped unexpectedly".into())
                }
                Err(_) => {}
            },
            _ => {}
        }
    }

    /// The player accepted the update.
    pub fn start(&mut self) {
        if let State::Available(v) = &self.state {
            let v = v.clone();
            let (tx, rx) = channel();
            let version = v.clone();
            std::thread::spawn(move || {
                let _ = tx.send(install(&version));
            });
            self.state = State::Downloading(v, rx);
        }
    }

    /// Start the freshly installed game. Returns true if it started, and
    /// this copy should now close.
    pub fn restart(&mut self) -> bool {
        let started = std::env::current_exe()
            .ok()
            .and_then(|exe| {
                // The testing overrides must not carry over, or the new copy
                // would think it is old and update again.
                Command::new(exe).env_remove("BPB_FAKE_VERSION").env_remove("BPB_UPDATE_AUTO").spawn().ok()
            })
            .is_some();
        if !started {
            self.state = State::Failed("Updated - please start the game again".into());
        }
        started
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number_not_by_text() {
        assert!(newer("1.10.0", "1.9.3"));
        assert!(newer("v2.0.0", "1.99.99"));
        assert!(newer("1.2.1", "1.2"));
        assert!(!newer("1.2.0", "1.2.0"));
        assert!(!newer("1.1.9", "1.2.0"));
        assert!(!newer("garbage", "1.0.0"));
    }

    #[test]
    fn the_latest_version_is_read_from_the_redirect() {
        let h = "HTTP/2 302\r\nserver: GitHub.com\r\nLocation: https://github.com/nissi15/breakpoint-billiards/releases/tag/v1.4.2\r\n\r\n";
        assert_eq!(version_in_headers(h).as_deref(), Some("1.4.2"));
        // No release yet, or a page that is not a redirect.
        assert_eq!(version_in_headers("HTTP/2 404\r\n\r\n"), None);
        // Nothing but digits and dots is ever accepted as a version.
        let evil = "location: https://github.com/x/y/releases/tag/v1.0/../../evil\r\n";
        assert_eq!(version_in_headers(evil), None);
    }
}
