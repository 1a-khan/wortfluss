//! Operating-system plumbing, except keyboard input/output (hotkey, typer, layout): file
//! locations, single instance, stopping, start at login, running in the background.
//! The shared parts are here; linux/platform.rs and windows/platform.rs do the rest:
//!
//!                 Linux                          Windows
//!   folders       ~/.config/vtt, ~/.local/...    %APPDATA%\Wortfluss, %LOCALAPPDATA%\Wortfluss
//!   stop request  SIGTERM                        a stop file the process watches
//!   background    setsid                         DETACHED_PROCESS, no console window
//!   autostart     ~/.config/autostart/*.desktop  HKCU\...\CurrentVersion\Run
//!   settings      a terminal program             a new console window

#[cfg(unix)]
#[path = "linux/platform.rs"]
mod os;
#[cfg(windows)]
#[path = "windows/platform.rs"]
mod os;

pub use os::*;

use anyhow::{anyhow, Result};
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::time::Duration;

pub fn log_file() -> Result<PathBuf> {
    Ok(state_dir()?.join("vtt.log"))
}

/// Lock file of the dictation process ("vtt") or a session ("vtt-session").
fn lock_path_for(name: &str) -> Result<PathBuf> {
    Ok(runtime_dir()?.join(format!("{name}.lock")))
}

/// The process id of the lock holder is kept next to the lock (Windows doesn't let others
/// read a locked file).
fn pid_path_for(name: &str) -> Result<PathBuf> {
    Ok(runtime_dir()?.join(format!("{name}.pid")))
}

/// A small file next to the lock files (e.g. the path of the running session's transcript).
pub fn runtime_file(name: &str) -> Result<PathBuf> {
    Ok(runtime_dir()?.join(name))
}

/// The path of this program, also after it was replaced by a newer build while running.
pub fn exe_path() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let s = exe.to_string_lossy();
    Ok(match s.strip_suffix(" (deleted)") {
        Some(clean) => PathBuf::from(clean),
        None => exe,
    })
}

// ---- single instance ----

/// Held by the running vtt. Two copies would type everything twice.
pub struct InstanceLock(#[allow(dead_code)] File);

/// Some(lock) if no other vtt is running, None if one is.
pub fn lock_instance() -> Result<Option<InstanceLock>> {
    lock_named("vtt")
}

/// Like `lock_instance`, for another kind of process (e.g. "vtt-session").
pub fn lock_named(name: &str) -> Result<Option<InstanceLock>> {
    let path = lock_path_for(name)?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;
    match f.try_lock() {
        Ok(()) => {
            std::fs::write(pid_path_for(name)?, std::process::id().to_string())?;
            Ok(Some(InstanceLock(f)))
        }
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
    }
}

/// Process id of the running vtt, if any.
pub fn running_pid() -> Option<u32> {
    running_pid_named("vtt")
}

pub fn running_pid_named(name: &str) -> Option<u32> {
    let f = File::open(lock_path_for(name).ok()?).ok()?;
    match f.try_lock_shared() {
        Ok(()) => None, // nobody holds it
        Err(_) => std::fs::read_to_string(pid_path_for(name).ok()?).ok()?.trim().parse().ok(),
    }
}

/// Stop the running vtt. Returns false if none was running.
pub fn stop_running() -> Result<bool> {
    stop_named("vtt", Duration::from_secs(5))
}

/// Ask the process holding lock `name` to stop and wait up to `wait` for it.
pub fn stop_named(name: &str, wait: Duration) -> Result<bool> {
    let Some(pid) = running_pid_named(name) else {
        return Ok(false);
    };
    request_stop(pid)?;
    let until = std::time::Instant::now() + wait;
    while std::time::Instant::now() < until {
        if running_pid_named(name).is_none() {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(anyhow!("process {pid} did not stop within {} s", wait.as_secs()))
}

/// Start `vtt run` detached from this terminal, output going to the log file.
pub fn start_background() -> Result<()> {
    start_background_with(&["run"])
}

/// The app icon as a PNG file (for notifications and the desktop entry); written once.
pub fn app_icon_path() -> Option<std::path::PathBuf> {
    // The file name carries the design version, so a new design replaces the old file.
    let path = data_dir().ok()?.join("wortfluss-wf.png");
    if !path.exists() {
        std::fs::create_dir_all(path.parent()?).ok()?;
        crate::icon::write_png(&path, 256, crate::icon::State::Ready, true).ok()?;
    }
    Some(path)
}
