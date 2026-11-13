//! Operating-system specific plumbing, except keyboard input/output (hotkey.rs, typer.rs,
//! layout.rs): file locations, start at login, single instance, running in the background.
//!
//! Linux implementation. A Windows port provides the same functions:
//!   config_dir/data_dir/state_dir -> %APPDATA%\vtt, %LOCALAPPDATA%\vtt
//!   autostart                     -> HKCU\Software\Microsoft\Windows\CurrentVersion\Run
//!   InstanceLock                  -> named mutex
//!   start_background              -> spawn with CREATE_NO_WINDOW | DETACHED_PROCESS

use anyhow::{anyhow, Context, Result};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("HOME is not set"))
}

fn xdg(var: &str, fallback: &str) -> Result<PathBuf> {
    match std::env::var_os(var) {
        Some(d) if !d.is_empty() => Ok(PathBuf::from(d)),
        _ => Ok(home()?.join(fallback)),
    }
}

pub fn config_dir() -> Result<PathBuf> {
    Ok(xdg("XDG_CONFIG_HOME", ".config")?.join("vtt"))
}

pub fn data_dir() -> Result<PathBuf> {
    Ok(xdg("XDG_DATA_HOME", ".local/share")?.join("vtt"))
}

pub fn state_dir() -> Result<PathBuf> {
    Ok(xdg("XDG_STATE_HOME", ".local/state")?.join("vtt"))
}

pub fn log_file() -> Result<PathBuf> {
    Ok(state_dir()?.join("vtt.log"))
}

fn lock_path() -> Result<PathBuf> {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(d) if !d.is_empty() => Ok(PathBuf::from(d).join("vtt.lock")),
        _ => Ok(state_dir()?.join("vtt.lock")),
    }
}

/// Write a file only the current user can read (it may hold API keys). Written to a
/// temporary file and renamed, so a running vtt never sees a half-written config.
pub fn write_private(path: &Path, text: &str) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let dir = path.parent().context("bad path")?;
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension("tmp");
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .with_context(|| format!("writing {}", tmp.display()))?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
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
    let path = lock_path()?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;
    match f.try_lock() {
        Ok(()) => {
            f.set_len(0)?;
            write!(f, "{}", std::process::id())?;
            Ok(Some(InstanceLock(f)))
        }
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
    }
}

/// Process id of the running vtt, if any.
pub fn running_pid() -> Option<u32> {
    let mut f = File::open(lock_path().ok()?).ok()?;
    match f.try_lock_shared() {
        Ok(()) => None, // nobody holds it
        Err(_) => {
            let mut s = String::new();
            f.read_to_string(&mut s).ok()?;
            s.trim().parse().ok()
        }
    }
}

/// Stop the running vtt. Returns false if none was running.
pub fn stop_running() -> Result<bool> {
    let Some(pid) = running_pid() else {
        return Ok(false);
    };
    if unsafe { libc::kill(pid as i32, libc::SIGTERM) } != 0 {
        return Err(std::io::Error::last_os_error()).context("could not stop vtt");
    }
    for _ in 0..50 {
        if running_pid().is_none() {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(anyhow!("vtt (pid {pid}) did not stop"))
}

/// Start `vtt run` detached from this terminal, output going to the log file.
pub fn start_background() -> Result<()> {
    use std::os::unix::process::CommandExt;
    let log_path = log_file()?;
    std::fs::create_dir_all(log_path.parent().unwrap())?;
    // Keep the log small: start over when it passes 1 MB.
    let big = std::fs::metadata(&log_path).is_ok_and(|m| m.len() > 1_000_000);
    let log = OpenOptions::new()
        .create(true)
        .append(!big)
        .write(true)
        .truncate(big)
        .open(&log_path)?;
    let mut cmd = Command::new(exe_path()?);
    cmd.arg("run")
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid(); // own session: survives closing the terminal
            Ok(())
        });
    }
    cmd.spawn().context("could not start vtt in the background")?;
    Ok(())
}

/// Replace this process with a fresh `vtt run` (after the settings changed).
pub fn restart_self() -> anyhow::Error {
    use std::os::unix::process::CommandExt;
    match exe_path() {
        Ok(exe) => anyhow!("could not restart: {}", Command::new(exe).arg("run").exec()),
        Err(e) => e,
    }
}

/// Throw away keys typed into the terminal (e.g. while capturing a hotkey).
pub fn flush_terminal_input() {
    unsafe {
        libc::tcflush(0, libc::TCIFLUSH);
    }
}

// ---- start at login ----

fn autostart_file() -> Result<PathBuf> {
    Ok(xdg("XDG_CONFIG_HOME", ".config")?
        .join("autostart")
        .join("vtt.desktop"))
}

pub fn autostart_enabled() -> bool {
    autostart_file().is_ok_and(|p| p.exists())
}

pub fn set_autostart(on: bool) -> Result<()> {
    let file = autostart_file()?;
    if !on {
        if file.exists() {
            std::fs::remove_file(&file)?;
        }
        return Ok(());
    }
    std::fs::create_dir_all(file.parent().unwrap())?;
    std::fs::write(
        &file,
        format!(
            "[Desktop Entry]\nType=Application\nName=vtt (voice to text)\n\
             Comment=Hold a key, speak, and the text is typed\n\
             Exec=\"{}\" run\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\nTerminal=false\n",
            exe_path()?.display()
        ),
    )?;
    Ok(())
}
