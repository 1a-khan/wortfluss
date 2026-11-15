//! Linux side of platform.rs: XDG folders, signals, setsid, .desktop autostart, terminals.

use anyhow::{anyhow, Context, Result};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

pub fn runtime_dir() -> Result<PathBuf> {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(d) if !d.is_empty() => Ok(PathBuf::from(d)),
        _ => state_dir(),
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

/// Ask process `pid` to stop (SIGTERM); it finishes its work first.
pub fn request_stop(pid: u32) -> Result<()> {
    if unsafe { libc::kill(pid as i32, libc::SIGTERM) } != 0 {
        return Err(std::io::Error::last_os_error()).context("could not stop it");
    }
    Ok(())
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

/// Only Windows closes a program's own window when it ends.
pub fn owns_console() -> bool {
    false
}

/// Child processes need nothing special on Linux.
pub fn quiet_child(_cmd: &mut Command) {}

/// Start `vtt <args>` detached from this terminal, output going to the log file.
pub fn start_background_with(args: &[&str]) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let log_path = super::log_file()?;
    std::fs::create_dir_all(log_path.parent().unwrap())?;
    // Keep the log small: start over when it passes 1 MB.
    let big = std::fs::metadata(&log_path).is_ok_and(|m| m.len() > 1_000_000);
    let log = OpenOptions::new()
        .create(true)
        .append(!big)
        .write(true)
        .truncate(big)
        .open(&log_path)?;
    let mut cmd = Command::new(super::exe_path()?);
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
    match super::exe_path() {
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
            "[Desktop Entry]\nType=Application\nName=Wortfluss\n\
             Comment=Hold a key, speak, and the text is typed\n\
             Exec=\"{}\" run\nIcon={}\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\nTerminal=false\n",
            super::exe_path()?.display(),
            crate::platform::app_icon_path().map(|p| p.display().to_string()).unwrap_or_default()
        ),
    )?;
    Ok(())
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
        .unwrap_or(false)
}

/// Run `vtt <args>` in a new terminal window (tray "All settings...", shortcut).
pub fn open_in_terminal(args: &[&str]) -> Result<()> {
    let exe = super::exe_path()?;
    let exe = exe.to_string_lossy().to_string();
    // (terminal, arguments that come before the command)
    let terminals: [(&str, &[&str]); 7] = [
        ("ptyxis", &["--new-window", "--"]),
        ("gnome-terminal", &["--"]),
        ("kgx", &["--"]),
        ("konsole", &["-e"]),
        ("xfce4-terminal", &["-x"]),
        ("xterm", &["-e"]),
        ("x-terminal-emulator", &["-e"]),
    ];
    let (term, pre) = terminals
        .iter()
        .find(|(t, _)| on_path(t))
        .ok_or_else(|| anyhow!("no terminal program found; run `vtt {}` in a terminal", args.join(" ")))?;
    Command::new(term)
        .args(*pre)
        .arg(&exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("could not open {term}"))?;
    Ok(())
}

/// Open a folder or file with the desktop's default program.
pub fn open_path(path: &Path) -> Result<()> {
    Command::new("xdg-open")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("could not open the file manager")?;
    Ok(())
}
