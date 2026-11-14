//! Global hotkey: reads key events straight from the keyboards in /dev/input.
//! Works on any desktop (GNOME/KDE/Wayland/X11) since it bypasses the compositor.
//! (Linux only. A Windows port uses a low-level keyboard hook instead.)

use crate::typer::DEVICE_NAME;
use anyhow::{bail, Result};
use evdev::{InputEventKind, Key};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone)]
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
    /// A keyboard shortcut or tray menu entry.
    Shortcut(Action),
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

/// Something to do, triggered by a keyboard shortcut or the tray menu.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Start or stop a session with the default source.
    Session,
    /// Start or stop a call session (system audio + microphone).
    CallSession,
    /// Start a session with this source ("system", "mic", "both").
    StartSession(String),
    StopSession,
    ToggleCorrection,
    ToggleMode,
    ToggleSounds,
    ToggleKeepAudio,
    SetMic(String),
    SetNotes(String),
    OpenTranscripts,
    OpenSettings,
    Quit,
}

/// A key combination like Ctrl+Alt+S.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Combo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
    pub key: Option<Key>,
}

impl Combo {
    /// "Ctrl+Alt+S", "ctrl+alt+f9", "Super+Shift+KEY_M". None for "" / "off" / invalid.
    pub fn parse(text: &str) -> Option<Combo> {
        let mut c = Combo::default();
        for part in text.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_lowercase().as_str() {
                "ctrl" | "control" | "strg" => c.ctrl = true,
                "alt" => c.alt = true,
                "shift" => c.shift = true,
                "super" | "meta" | "win" | "windows" => c.meta = true,
                "off" | "none" => return None,
                key => {
                    let name = if key.starts_with("key_") { key.to_uppercase() } else { format!("KEY_{}", key.to_uppercase()) };
                    c.key = Some(name.parse().ok()?);
                }
            }
        }
        // A combination needs a key and at least one of Ctrl/Alt/Super, so normal typing
        // never triggers it.
        (c.key.is_some() && (c.ctrl || c.alt || c.meta)).then_some(c)
    }

    pub fn text(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl { parts.push("Ctrl".to_string()); }
        if self.alt { parts.push("Alt".to_string()); }
        if self.shift { parts.push("Shift".to_string()); }
        if self.meta { parts.push("Super".to_string()); }
        if let Some(k) = self.key { parts.push(pretty(&format!("{k:?}"))); }
        parts.join("+")
    }
}

/// Modifier keys held on one keyboard.
#[derive(Default)]
struct Mods {
    ctrl: u8,
    alt: u8,
    shift: u8,
    meta: u8,
}

impl Mods {
    /// Track a modifier; returns true if `k` is a modifier key.
    fn update(&mut self, k: Key, down: bool) -> bool {
        let slot = match k {
            Key::KEY_LEFTCTRL | Key::KEY_RIGHTCTRL => &mut self.ctrl,
            Key::KEY_LEFTALT | Key::KEY_RIGHTALT => &mut self.alt,
            Key::KEY_LEFTSHIFT | Key::KEY_RIGHTSHIFT => &mut self.shift,
            Key::KEY_LEFTMETA | Key::KEY_RIGHTMETA => &mut self.meta,
            _ => return false,
        };
        *slot = if down { slot.saturating_add(1).min(2) } else { slot.saturating_sub(1) };
        true
    }

    fn matches(&self, c: &Combo, k: Key) -> bool {
        c.key == Some(k)
            && (self.ctrl > 0) == c.ctrl
            && (self.alt > 0) == c.alt
            && (self.shift > 0) == c.shift
            && (self.meta > 0) == c.meta
    }
}

pub fn listen(
    hotkey: Key,
    session_key: Option<Key>,
    shortcuts: Vec<(Combo, Action)>,
    tx: Sender<Event>,
) -> Result<usize> {
    let mut count = 0;
    for mut dev in keyboards() {
        if !dev.supported_keys().is_some_and(|k| k.contains(hotkey)) {
            continue;
        }
        let (tx, shortcuts) = (tx.clone(), shortcuts.clone());
        thread::spawn(move || {
            let mut mods = Mods::default();
            loop {
                let events = match dev.fetch_events() {
                    Ok(events) => events,
                    Err(e) => {
                        eprintln!("stopped reading a keyboard: {e}");
                        return;
                    }
                };
                for ev in events {
                    let InputEventKind::Key(k) = ev.kind() else { continue };
                    if ev.value() != 2 {
                        mods.update(k, ev.value() == 1);
                    }
                    let mut out = Vec::new();
                    if Some(k) == session_key {
                        if ev.value() == 1 {
                            out.push(Event::SessionKey);
                        }
                    } else if k == hotkey {
                        match ev.value() {
                            1 => out.push(Event::Down),
                            0 => out.push(Event::Up),
                            _ => {} // auto-repeat
                        }
                    } else if ev.value() == 1 && k.code() < 0x100 {
                        out.push(Event::OtherKey); // keyboard keys only, not mouse buttons
                        if let Some((_, action)) = shortcuts.iter().find(|(c, _)| mods.matches(c, k)) {
                            out.push(Event::Shortcut(action.clone()));
                        }
                    }
                    for e in out {
                        if tx.send(e).is_err() {
                            return;
                        }
                    }
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

/// Wait for a key combination (setup). Returns when a non-modifier key is pressed.
pub fn capture_combo() -> Result<Combo> {
    let (tx, rx) = mpsc::channel();
    let mut count = 0;
    for mut dev in keyboards() {
        let tx = tx.clone();
        thread::spawn(move || {
            let mut mods = Mods::default();
            loop {
                let Ok(events) = dev.fetch_events() else { return };
                for ev in events {
                    let InputEventKind::Key(k) = ev.kind() else { continue };
                    if ev.value() == 2 {
                        continue;
                    }
                    let is_mod = mods.update(k, ev.value() == 1);
                    if !is_mod && ev.value() == 1 && k.code() < 0x100 {
                        let c = Combo {
                            ctrl: mods.ctrl > 0,
                            alt: mods.alt > 0,
                            shift: mods.shift > 0,
                            meta: mods.meta > 0,
                            key: Some(k),
                        };
                        if tx.send(c).is_err() {
                            return;
                        }
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
    thread::sleep(Duration::from_millis(300));
    while rx.try_recv().is_ok() {}
    let c = rx.recv()?;
    thread::sleep(Duration::from_millis(400));
    crate::platform::flush_terminal_input();
    Ok(c)
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

    #[test]
    fn combos() {
        let c = Combo::parse("Ctrl+Alt+S").unwrap();
        assert!(c.ctrl && c.alt && !c.shift && c.key == Some(Key::KEY_S));
        assert_eq!(c.text(), "Ctrl+Alt+S");
        assert_eq!(Combo::parse("ctrl + alt + f9").unwrap().key, Some(Key::KEY_F9));
        assert!(Combo::parse("S").is_none(), "a plain letter is not a shortcut");
        assert!(Combo::parse("Shift+S").is_none(), "Shift alone would fire while typing");
        assert!(Combo::parse("").is_none() && Combo::parse("off").is_none());

        let mut m = Mods::default();
        m.update(Key::KEY_LEFTCTRL, true);
        m.update(Key::KEY_RIGHTALT, true);
        assert!(m.matches(&c, Key::KEY_S));
        m.update(Key::KEY_LEFTSHIFT, true);
        assert!(!m.matches(&c, Key::KEY_S), "extra Shift: different shortcut");
    }
}

#[cfg(test)]
mod combo_round_trip {
    use super::*;

    #[test]
    fn text_parses_back() {
        for t in ["Ctrl+Alt+S", "Ctrl+Alt+F9", "Super+Shift+M", "Ctrl+Alt+1"] {
            let c = Combo::parse(t).unwrap();
            assert_eq!(Combo::parse(&c.text()), Some(c), "{t} -> {}", c.text());
        }
    }
}
