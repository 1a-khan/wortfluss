//! Global hotkey, session key and keyboard shortcuts, the same on every system. The
//! operating system part only delivers raw key events from all keyboards:
//!   Linux:   linux/hotkey.rs, reads /dev/input (evdev) directly, works on Wayland and X11
//!   Windows: windows/hotkey.rs, a low-level keyboard hook (no permissions needed)
//! Keys use the Linux key names and numbers on both ("KEY_RIGHTCTRL"), so config.toml
//! means the same everywhere.

use anyhow::Result;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

#[cfg(target_os = "linux")]
#[path = "linux/hotkey.rs"]
mod os;
#[cfg(windows)]
#[path = "windows/hotkey.rs"]
mod os;

pub use os::{keyboard_count, Key};

/// Key event values, as Linux numbers them.
const UP: i32 = 0;
const DOWN: i32 = 1;
const REPEAT: i32 = 2;

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
    ToggleSpeakers,
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

/// Turns raw key events into vtt events (hotkey down/up, session key, shortcuts).
struct Dispatcher {
    hotkey: Key,
    session_key: Option<Key>,
    shortcuts: Vec<(Combo, Action)>,
    mods: Mods,
}

impl Dispatcher {
    fn handle(&mut self, k: Key, value: i32) -> Vec<Event> {
        if value != REPEAT {
            self.mods.update(k, value == DOWN);
        }
        let mut out = Vec::new();
        if Some(k) == self.session_key {
            if value == DOWN {
                out.push(Event::SessionKey);
            }
        } else if k == self.hotkey {
            match value {
                DOWN => out.push(Event::Down),
                UP => out.push(Event::Up),
                _ => {} // auto-repeat
            }
        } else if value == DOWN && k.code() < 0x100 {
            out.push(Event::OtherKey); // keyboard keys only, not mouse buttons
            if let Some((_, action)) = self.shortcuts.iter().find(|(c, _)| self.mods.matches(c, k)) {
                out.push(Event::Shortcut(action.clone()));
            }
        }
        out
    }
}

/// Watch the keyboards and send hotkey, session key and shortcut events to `tx`.
/// Returns the number of keyboards (Windows: 1, the hook sees all of them).
pub fn listen(hotkey: Key, session_key: Option<Key>, shortcuts: Vec<(Combo, Action)>, tx: Sender<Event>) -> Result<usize> {
    let keys = os::subscribe(Some(hotkey))?;
    let mut d = Dispatcher { hotkey, session_key, shortcuts, mods: Mods::default() };
    thread::spawn(move || {
        for (k, value) in keys {
            for e in d.handle(k, value) {
                if tx.send(e).is_err() {
                    return;
                }
            }
        }
    });
    Ok(keyboard_count().max(1))
}

/// Wait for a key combination (setup). Returns when a non-modifier key is pressed.
pub fn capture_combo() -> Result<Combo> {
    let keys = os::subscribe(None)?;
    // Ignore the Enter that answered the previous question.
    thread::sleep(Duration::from_millis(300));
    while keys.try_recv().is_ok() {}
    let mut mods = Mods::default();
    let combo = loop {
        let (k, value) = keys.recv()?;
        if value == REPEAT {
            continue;
        }
        let is_mod = mods.update(k, value == DOWN);
        if !is_mod && value == DOWN && k.code() < 0x100 {
            break Combo { ctrl: mods.ctrl > 0, alt: mods.alt > 0, shift: mods.shift > 0, meta: mods.meta > 0, key: Some(k) };
        }
    };
    thread::sleep(Duration::from_millis(400)); // let it be released
    crate::platform::flush_terminal_input();
    Ok(combo)
}

/// Wait for the user to press a key and return it (for `vtt setup`).
pub fn capture() -> Result<Key> {
    let keys = os::subscribe(None)?;
    thread::sleep(Duration::from_millis(300));
    while keys.try_recv().is_ok() {}
    let key = loop {
        let (k, value) = keys.recv()?;
        if value == DOWN && k.code() < 0x100 {
            break k;
        }
    };
    thread::sleep(Duration::from_millis(400));
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
