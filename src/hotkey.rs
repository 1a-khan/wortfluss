//! Global hotkey: reads key events straight from the keyboards in /dev/input.
//! Works on any desktop (GNOME/KDE/Wayland/X11) since it bypasses the compositor.
//! (Linux only. A Windows port uses a low-level keyboard hook instead.)

use crate::typer::DEVICE_NAME;
use anyhow::{bail, Result};
use evdev::{InputEventKind, Key};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub enum Event {
    Down,
    Up,
    /// Some other keyboard key was pressed (used to cancel on shortcuts like Ctrl+C).
    OtherKey,
    /// Recording number N reached the max_seconds limit.
    Timeout(u64),
    /// Recording number N has been held long enough to be a dictation: play the start beep.
    BeepDue(u64),
    /// Recording number N: time to check whether a piece is ready to send.
    Tick(u64),
    /// config.toml changed.
    Reload,
    /// The session key was pressed (start/stop a session).
    SessionKey,
}

const PERMISSION_HELP: &str = "no keyboard could be opened. Your user needs read access to \
    /dev/input (the `input` group). Run the one-time permission setup from README.md, \
    then log out and back in";

fn keyboards() -> Vec<evdev::Device> {
    evdev::enumerate()
        .map(|(_, d)| d)
        .filter(|d| d.name() != Some(DEVICE_NAME))
        .filter(|d| {
            d.supported_keys()
                .is_some_and(|k| k.contains(Key::KEY_A) || k.contains(Key::KEY_F9))
        })
        .collect()
}

/// Number of keyboards vtt can read (0 = no permission).
pub fn keyboard_count() -> usize {
    keyboards().len()
}

pub fn listen(hotkey: Key, session_key: Option<Key>, tx: Sender<Event>) -> Result<usize> {
    let mut count = 0;
    for mut dev in keyboards() {
        if !dev.supported_keys().is_some_and(|k| k.contains(hotkey)) {
            continue;
        }
        let tx = tx.clone();
        thread::spawn(move || loop {
            let events = match dev.fetch_events() {
                Ok(events) => events,
                Err(e) => {
                    eprintln!("stopped reading a keyboard: {e}");
                    return;
                }
            };
            for ev in events {
                let InputEventKind::Key(k) = ev.kind() else { continue };
                let event = if Some(k) == session_key {
                    if ev.value() != 1 {
                        continue;
                    }
                    Event::SessionKey
                } else if k == hotkey {
                    match ev.value() {
                        1 => Event::Down,
                        0 => Event::Up,
                        _ => continue, // auto-repeat
                    }
                } else if ev.value() == 1 && k.code() < 0x100 {
                    Event::OtherKey // keyboard keys only, not mouse buttons
                } else {
                    continue;
                };
                if tx.send(event).is_err() {
                    return;
                }
            }
        });
        count += 1;
    }
    if count == 0 {
        bail!(PERMISSION_HELP);
    }
    Ok(count)
}

/// Wait for the user to press a key and return it (for `vtt setup`).
pub fn capture() -> Result<Key> {
    let (tx, rx) = mpsc::channel();
    let mut count = 0;
    for mut dev in keyboards() {
        let tx = tx.clone();
        thread::spawn(move || loop {
            let Ok(events) = dev.fetch_events() else { return };
            for ev in events {
                if let InputEventKind::Key(k) = ev.kind() {
                    if ev.value() == 1 && k.code() < 0x100 && tx.send(k).is_err() {
                        return;
                    }
                }
            }
        });
        count += 1;
    }
    if count == 0 {
        bail!(PERMISSION_HELP);
    }
    drop(tx);
    // Ignore the Enter that answered the previous question.
    thread::sleep(Duration::from_millis(300));
    while rx.try_recv().is_ok() {}
    let key = rx.recv()?;
    thread::sleep(Duration::from_millis(400)); // let it be released
    crate::platform::flush_terminal_input();
    Ok(key)
}

pub fn parse(name: &str) -> Option<Key> {
    name.trim().parse().ok()
}

/// "KEY_RIGHTCTRL" -> "Right Ctrl"
pub fn pretty(name: &str) -> String {
    let n = name.trim().trim_start_matches("KEY_");
    let words: &[(&str, &str)] = &[
        ("RIGHTCTRL", "Right Ctrl"),
        ("LEFTCTRL", "Left Ctrl"),
        ("RIGHTALT", "Right Alt / AltGr"),
        ("LEFTALT", "Left Alt"),
        ("RIGHTSHIFT", "Right Shift"),
        ("LEFTSHIFT", "Left Shift"),
        ("RIGHTMETA", "Right Super/Windows"),
        ("LEFTMETA", "Left Super/Windows"),
        ("COMPOSE", "Menu"),
        ("CAPSLOCK", "Caps Lock"),
        ("SCROLLLOCK", "Scroll Lock"),
        ("PAUSE", "Pause"),
        ("INSERT", "Insert"),
        ("SYSRQ", "Print Screen"),
    ];
    words
        .iter()
        .find(|(k, _)| *k == n)
        .map(|(_, v)| v.to_string())
        .unwrap_or_else(|| n.to_string())
}

/// Why a key is a poor hotkey, if it is.
pub fn warning(key: Key) -> Option<&'static str> {
    use Key as K;
    let code = key.code();
    let typing = (K::KEY_1.code()..=K::KEY_EQUAL.code()).contains(&code)
        || (K::KEY_Q.code()..=K::KEY_RIGHTBRACE.code()).contains(&code)
        || (K::KEY_A.code()..=K::KEY_GRAVE.code()).contains(&code)
        || (K::KEY_BACKSLASH.code()..=K::KEY_SLASH.code()).contains(&code)
        || [K::KEY_SPACE, K::KEY_ENTER, K::KEY_TAB, K::KEY_BACKSPACE, K::KEY_ESC, K::KEY_102ND]
            .contains(&key);
    if typing {
        return Some("this key is used for normal typing; you would lose it");
    }
    match key {
        K::KEY_RIGHTALT => Some(
            "on German, Turkish and many other layouts this is AltGr (needed for @, €, { } ...)",
        ),
        K::KEY_LEFTSHIFT | K::KEY_RIGHTSHIFT => {
            Some("Shift is pressed constantly while typing; every capital letter would start a recording")
        }
        K::KEY_CAPSLOCK => Some("Caps Lock will still switch caps on and off when pressed"),
        K::KEY_LEFTMETA | K::KEY_RIGHTMETA => Some("the desktop also uses this key (e.g. opens the app overview)"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_key_name_round_trips() {
        for k in [Key::KEY_RIGHTCTRL, Key::KEY_F9, Key::KEY_PAUSE, Key::KEY_COMPOSE] {
            let name = format!("{k:?}");
            assert_eq!(parse(&name), Some(k), "{name}");
        }
        assert_eq!(pretty("KEY_RIGHTCTRL"), "Right Ctrl");
        assert!(warning(Key::KEY_A).is_some());
        assert!(warning(Key::KEY_RIGHTALT).is_some());
        assert!(warning(Key::KEY_F9).is_none());
    }
}
