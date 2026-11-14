//! Windows launcher without a console window: starts `vtt.exe start` (or the arguments
//! given) next to it. Used by the Start menu entry and start at login, so clients never
//! see a black console window flash up. On other systems it just runs vtt.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::process::Command;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args = if args.is_empty() { vec!["start".to_string()] } else { args };
    let exe = std::env::current_exe()
        .map(|p| p.with_file_name(if cfg!(windows) { "vtt.exe" } else { "vtt" }))
        .unwrap_or_else(|_| "vtt".into());
    let mut cmd = Command::new(exe);
    cmd.args(&args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let _ = cmd.spawn();
}
