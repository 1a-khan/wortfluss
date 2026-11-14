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

fn runtime_dir() -> Result<PathBuf> {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(d) if !d.is_empty() => Ok(PathBuf::from(d)),
        _ => state_dir(),
    }
}

/// Lock file of the dictation process ("vtt") or a session ("vtt-session").
fn lock_path_for(name: &str) -> Result<PathBuf> {
    Ok(runtime_dir()?.join(format!("{name}.lock")))
}

/// A small file next to the lock files (e.g. the path of the running session's transcript).
pub fn runtime_file(name: &str) -> Result<PathBuf> {
    Ok(runtime_dir()?.join(name))
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
    lock_named("vtt")
}

/// Like `lock_instance`, for another kind of process (e.g. "vtt-session").
pub fn lock_named(name: &str) -> Result<Option<InstanceLock>> {
    let path = lock_path_for(name)?;
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
    running_pid_named("vtt")
}

pub fn running_pid_named(name: &str) -> Option<u32> {
    let mut f = File::open(lock_path_for(name).ok()?).ok()?;
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
    stop_named("vtt", Duration::from_secs(5))
}

/// Ask the process holding lock `name` to stop (SIGTERM) and wait up to `wait` for it.
pub fn stop_named(name: &str, wait: Duration) -> Result<bool> {
    let Some(pid) = running_pid_named(name) else {
        return Ok(false);
    };
    if unsafe { libc::kill(pid as i32, libc::SIGTERM) } != 0 {
        return Err(std::io::Error::last_os_error()).context("could not stop it");
    }
    let until = std::time::Instant::now() + wait;
    while std::time::Instant::now() < until {
        if running_pid_named(name).is_none() {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(anyhow!("process {pid} did not stop within {} s", wait.as_secs()))
}

static STOP_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn on_stop_signal(_: libc::c_int) {
    STOP_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// From now on Ctrl+C and SIGTERM (`vtt session stop`) only set a flag, so the process
/// can finish its work (last piece, notes) before exiting.
pub fn catch_stop_signals() {
    unsafe {
        libc::signal(libc::SIGINT, on_stop_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_stop_signal as *const () as libc::sighandler_t);
    }
}

pub fn stop_requested() -> bool {
    STOP_REQUESTED.load(std::sync::atomic::Ordering::SeqCst)
}

/// Local date and time: (year, month, day, hour, minute, second).
pub fn local_time() -> (i32, u32, u32, u32, u32, u32) {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        (
            tm.tm_year + 1900,
            tm.tm_mon as u32 + 1,
            tm.tm_mday as u32,
            tm.tm_hour as u32,
            tm.tm_min as u32,
            tm.tm_sec as u32,
        )
    }
}

/// ~/Documents (XDG user dir), for session transcripts.
pub fn documents_dir() -> Result<PathBuf> {
    let home = home()?;
    let dirs = std::fs::read_to_string(xdg("XDG_CONFIG_HOME", ".config")?.join("user-dirs.dirs")).unwrap_or_default();
    let configured = dirs.lines().find_map(|l| {
        let v = l.trim().strip_prefix("XDG_DOCUMENTS_DIR=")?.trim_matches('"');
        Some(PathBuf::from(v.replace("$HOME", &home.to_string_lossy())))
    });
    Ok(configured.unwrap_or_else(|| home.join("Documents")))
}

/// Start `vtt run` detached from this terminal, output going to the log file.
pub fn start_background() -> Result<()> {
    start_background_with(&["run"])
}

/// Start `vtt <args>` detached from this terminal, output going to the log file.
pub fn start_background_with(args: &[&str]) -> Result<()> {
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
    cmd.args(args)
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
