//! Windows side of platform.rs: folders under %APPDATA% / %LOCALAPPDATA%, stopping through
//! a stop file, background processes without a console window, the Run key for autostart.

use anyhow::{anyhow, Context, Result};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use windows_sys::Win32::Foundation::{ERROR_SUCCESS, SYSTEMTIME};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Console::{
    FlushConsoleInputBuffer, GetConsoleWindow, GetStdHandle, SetConsoleCtrlHandler, STD_INPUT_HANDLE,
};
use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_SZ,
    RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY,
};
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APP: &str = "Wortfluss";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn env_dir(var: &str) -> Result<PathBuf> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("{var} is not set"))
}

/// %APPDATA%\Wortfluss: config.toml, secrets.env (roams with the profile).
pub fn config_dir() -> Result<PathBuf> {
    Ok(env_dir("APPDATA")?.join(APP))
}

/// %LOCALAPPDATA%\Wortfluss: usage, failed pieces, icon.
pub fn data_dir() -> Result<PathBuf> {
    Ok(env_dir("LOCALAPPDATA")?.join(APP))
}

pub fn state_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("state"))
}

pub fn runtime_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("run"))
}

/// Write a file through a temporary file and a rename, so a running vtt never sees a
/// half-written config. (The profile folders are private to the user already.)
pub fn write_private(path: &Path, text: &str) -> Result<()> {
    use std::io::Write;
    let dir = path.parent().context("bad path")?;
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::File::create(&tmp).with_context(|| format!("writing {}", tmp.display()))?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

// ---- stopping ----
// Windows has no SIGTERM for background processes. A process is asked to stop with a file
// "stop-<pid>" in the runtime folder, which `stop_requested` watches.

fn stop_file(pid: u32) -> Result<PathBuf> {
    Ok(runtime_dir()?.join(format!("stop-{pid}")))
}

/// Ask process `pid` to stop; it finishes its work first.
pub fn request_stop(pid: u32) -> Result<()> {
    let f = stop_file(pid)?;
    std::fs::create_dir_all(f.parent().unwrap())?;
    std::fs::write(&f, b"").context("could not ask it to stop")?;
    Ok(())
}

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn on_console_event(_kind: u32) -> i32 {
    // Ctrl+C, Ctrl+Break, closing the window: finish the work, then exit.
    STOP_REQUESTED.store(true, SeqCst);
    1
}

/// From now on Ctrl+C and stop requests only set a flag, so the process can finish its
/// work (last piece, notes) before exiting.
pub fn catch_stop_signals() {
    if let Ok(f) = stop_file(std::process::id()) {
        let _ = std::fs::remove_file(f); // left over from an earlier process with this id
    }
    unsafe {
        SetConsoleCtrlHandler(Some(on_console_event), 1);
    }
}

pub fn stop_requested() -> bool {
    if STOP_REQUESTED.load(SeqCst) {
        return true;
    }
    let asked = stop_file(std::process::id()).is_ok_and(|f| f.exists());
    if asked {
        if let Ok(f) = stop_file(std::process::id()) {
            let _ = std::fs::remove_file(f);
        }
        STOP_REQUESTED.store(true, SeqCst);
    }
    asked
}

/// Local date and time: (year, month, day, hour, minute, second).
pub fn local_time() -> (i32, u32, u32, u32, u32, u32) {
    let mut t: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    (t.wYear as i32, t.wMonth as u32, t.wDay as u32, t.wHour as u32, t.wMinute as u32, t.wSecond as u32)
}

/// The Documents folder (also when it is moved to OneDrive).
pub fn documents_dir() -> Result<PathBuf> {
    unsafe {
        let mut p: windows_sys::core::PWSTR = std::ptr::null_mut();
        if SHGetKnownFolderPath(&FOLDERID_Documents, 0, std::ptr::null_mut(), &mut p) == 0 && !p.is_null() {
            let len = (0..).take_while(|&i| *p.add(i) != 0).count();
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
            CoTaskMemFree(p as *const _);
            return Ok(PathBuf::from(s));
        }
        if !p.is_null() {
            CoTaskMemFree(p as *const _);
        }
    }
    Ok(env_dir("USERPROFILE")?.join("Documents"))
}

/// A console program started by a process without a console window would open a new
/// window: prevent that for children of background processes.
pub fn quiet_child(cmd: &mut Command) {
    if unsafe { GetConsoleWindow() }.is_null() {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

/// Start `vtt <args>` in the background (no window), output going to the log file.
pub fn start_background_with(args: &[&str]) -> Result<()> {
    let log_path = super::log_file()?;
    std::fs::create_dir_all(log_path.parent().unwrap())?;
    let big = std::fs::metadata(&log_path).is_ok_and(|m| m.len() > 1_000_000);
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(!big)
        .write(true)
        .truncate(big)
        .open(&log_path)?;
    Command::new(super::exe_path()?)
        .args(args)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
        .spawn()
        .context("could not start vtt in the background")?;
    Ok(())
}

/// Start a fresh `vtt run` and end this process (after the settings changed).
pub fn restart_self() -> anyhow::Error {
    match start_background_with(&["run"]) {
        Ok(()) => std::process::exit(0),
        Err(e) => e,
    }
}

/// Throw away keys typed into the console (e.g. while capturing a hotkey).
pub fn flush_terminal_input() {
    unsafe {
        FlushConsoleInputBuffer(GetStdHandle(STD_INPUT_HANDLE));
    }
}

// ---- registry ----

fn reg_get(root: HKEY, key: &str, value: &str, flags: u32) -> Option<String> {
    let (k, v) = (wide(key), wide(value));
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(root, k.as_ptr(), v.as_ptr(), RRF_RT_REG_SZ | flags, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size)
    };
    if r != ERROR_SUCCESS {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..len.min(buf.len())]))
}

/// The id Windows gave this installation (for the fallback secrets key).
pub fn machine_guid() -> Option<String> {
    reg_get(HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Cryptography", "MachineGuid", RRF_SUBKEY_WOW6464KEY)
}

// ---- start at login ----

/// The command for the Run key: the window-less launcher next to vtt.exe if it is there.
fn autostart_command() -> Result<String> {
    let exe = super::exe_path()?;
    let launcher = exe.with_file_name("wortfluss.exe");
    Ok(if launcher.exists() {
        format!("\"{}\"", launcher.display())
    } else {
        format!("\"{}\" start", exe.display())
    })
}

pub fn autostart_enabled() -> bool {
    reg_get(HKEY_CURRENT_USER, RUN_KEY, APP, 0).is_some()
}

pub fn set_autostart(on: bool) -> Result<()> {
    let (k, v) = (wide(RUN_KEY), wide(APP));
    let r = unsafe {
        if on {
            let data = wide(&autostart_command()?);
            RegSetKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr(), REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32)
        } else {
            match RegDeleteKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr()) {
                2 => ERROR_SUCCESS, // ERROR_FILE_NOT_FOUND: wasn't on
                r => r,
            }
        }
    };
    if r != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(r as i32)).context("could not change the start-at-login setting");
    }
    Ok(())
}

/// After an update: the version Windows shows under "Installed apps" (the installer's
/// per-user uninstall entry).
pub fn set_installed_version(version: &str) {
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{6C3F8A1E-4B2D-4F7A-9E51-57A0D3C2B8F4}_is1");
    let (name, data) = (wide("DisplayVersion"), wide(version));
    unsafe {
        RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32);
    }
}

/// Is this process alone in its console window (so the window closes when it ends)?
pub fn owns_console() -> bool {
    use windows_sys::Win32::System::Console::GetConsoleProcessList;
    let mut ids = [0u32; 4];
    unsafe { GetConsoleProcessList(ids.as_mut_ptr(), ids.len() as u32) == 1 }
}

/// Run `vtt <args>` in a new console window (tray "All settings...", shortcut). The window
/// waits for Enter at the end (see `owns_console`), so the last message can be read.
///
/// Started through `cmd /c start`: a child created directly with CREATE_NEW_CONSOLE gets
/// the background vtt's input and output (no console, a log file) instead of the new
/// window's, so the settings saw no keyboard, stopped at once and the window vanished.
pub fn open_in_terminal(args: &[&str]) -> Result<()> {
    let exe = super::exe_path()?;
    let line = format!("/d /c start \"Wortfluss\" \"{}\" {}", exe.display(), args.join(" "));
    Command::new("cmd.exe")
        .raw_arg(line)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .context("could not open a console window")?;
    Ok(())
}

/// Open a folder or file with its default program (Explorer).
pub fn open_path(path: &Path) -> Result<()> {
    Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .context("could not open Explorer")?;
    Ok(())
}
