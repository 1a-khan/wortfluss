//! Types text into the focused window through a virtual keyboard (/dev/uinput).
//! Works on Wayland and X11 alike because the compositor sees a real keyboard.
//!
//! The desktop turns our key presses into characters using the user's keyboard layout,
//! so we ask libxkbcommon (the same library GNOME/KDE use) which key + Shift/AltGr gives
//! each character on that layout. Characters with no key at all (e.g. Urdu script on a
//! German keyboard) are entered with the GTK/IBus Unicode shortcut Ctrl+Shift+U <hex> Space.

use anyhow::{Context, Result};
use evdev::uinput::{VirtualDevice, VirtualDeviceBuilder};
use evdev::{AttributeSet, EventType, InputEvent, Key};
use std::collections::HashMap;
use std::thread::sleep;
use std::time::Duration;
use crate::layout::Layout;
use xkbcommon::xkb;

pub const DEVICE_NAME: &str = "vtt virtual keyboard";

#[derive(Clone, Copy, Debug)]
pub struct Stroke {
    key: Key,
    shift: bool,
    altgr: bool,
}

pub struct Typer {
    dev: VirtualDevice,
    delay: Duration,
    map: HashMap<char, Stroke>,
    /// Key maps built so far, so switching layouts back and forth costs nothing.
    maps: HashMap<Layout, HashMap<char, Stroke>>,
    current: Option<Layout>,
}

impl Typer {
    pub fn new(delay_ms: u64) -> Result<Self> {
        let mut keys = AttributeSet::<Key>::new();
        for code in 1..=248u16 {
            keys.insert(Key::new(code));
        }
        let dev = VirtualDeviceBuilder::new()
            .context(
                "cannot open /dev/uinput (needed to type text). \
                 Run the one-time permission setup from README.md, then log out and back in",
            )?
            .name(DEVICE_NAME)
            .with_keys(&keys)?
            .build()
            .context("cannot create virtual keyboard")?;
        // Give the desktop a moment to notice the new keyboard before we use it.
        sleep(Duration::from_millis(600));
        Ok(Self {
            dev,
            delay: Duration::from_millis(delay_ms),
            map: HashMap::new(),
            maps: HashMap::new(),
            current: None,
        })
    }

    /// Use the key map for this layout (built on first use).
    fn use_layout(&mut self, layout: &Layout) -> Result<()> {
        if self.current.as_ref() == Some(layout) {
            return Ok(());
        }
        if let Some(cur) = self.current.take() {
            self.maps.insert(cur, std::mem::take(&mut self.map));
        }
        self.map = match self.maps.remove(layout) {
            Some(m) => m,
            None => build_map(&layout.layout, &layout.variant)?,
        };
        self.current = Some(layout.clone());
        Ok(())
    }

    fn key(&mut self, key: Key, value: i32) -> Result<()> {
        self.dev
            .emit(&[InputEvent::new(EventType::KEY, key.code(), value)])?;
        sleep(self.delay);
        Ok(())
    }

    fn press(&mut self, s: Stroke) -> Result<()> {
        if s.altgr {
            self.key(Key::KEY_RIGHTALT, 1)?;
        }
        if s.shift {
            self.key(Key::KEY_LEFTSHIFT, 1)?;
        }
        self.key(s.key, 1)?;
        self.key(s.key, 0)?;
        if s.shift {
            self.key(Key::KEY_LEFTSHIFT, 0)?;
        }
        if s.altgr {
            self.key(Key::KEY_RIGHTALT, 0)?;
        }
        Ok(())
    }

    fn unicode(&mut self, c: char) -> Result<()> {
        let Some(&u) = self.map.get(&'u') else {
            return Ok(()); // layout without a `u` key: nothing sensible to do
        };
        self.key(Key::KEY_LEFTCTRL, 1)?;
        self.key(Key::KEY_LEFTSHIFT, 1)?;
        self.key(u.key, 1)?;
        self.key(u.key, 0)?;
        self.key(Key::KEY_LEFTSHIFT, 0)?;
        self.key(Key::KEY_LEFTCTRL, 0)?;
        for h in format!("{:x}", c as u32).chars() {
            if let Some(&s) = self.map.get(&h) {
                self.press(s)?;
            }
        }
        self.press(self.map[&' '])
    }

    pub fn type_text(&mut self, text: &str, layout: &Layout) -> Result<()> {
        self.use_layout(layout)?;
        for c in normalize(text).chars() {
            if c == '\r' {
                continue;
            }
            if c == '\n' {
                // Shift+Enter: a new line that doesn't send the message in chat apps.
                self.key(Key::KEY_LEFTSHIFT, 1)?;
                self.key(Key::KEY_ENTER, 1)?;
                self.key(Key::KEY_ENTER, 0)?;
                self.key(Key::KEY_LEFTSHIFT, 0)?;
                continue;
            }
            match self.map.get(&c) {
                Some(&s) => self.press(s)?,
                None => self.unicode(c)?,
            }
        }
        Ok(())
    }
}

/// For every character the layout can produce, the key and modifiers that produce it.
/// Simulates holding nothing, Shift, AltGr and Shift+AltGr and asks xkbcommon what
/// each key then gives. Fewer modifiers win, then lower key codes (main block before keypad).
pub fn build_map(layout: &str, variant: &str) -> Result<HashMap<char, Stroke>> {
    let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = xkb::Keymap::new_from_names(
        &ctx,
        "",
        "pc105",
        layout,
        variant,
        None,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .with_context(|| {
        format!("unknown keyboard layout `{layout}` variant `{variant}` (keyboard_layout in config.toml)")
    })?;
    let xkb_code = |k: Key| xkb::Keycode::new(k.code() as u32 + 8); // evdev -> xkb numbering

    let mut map = HashMap::new();
    for (shift, altgr) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut state = xkb::State::new(&keymap);
        if shift {
            state.update_key(xkb_code(Key::KEY_LEFTSHIFT), xkb::KeyDirection::Down);
        }
        if altgr {
            state.update_key(xkb_code(Key::KEY_RIGHTALT), xkb::KeyDirection::Down);
        }
        for code in 1..=248u16 {
            let key = Key::new(code);
            let cp = state.key_get_utf32(xkb_code(key));
            if cp < 0x20 || cp == 0x7f {
                continue; // nothing, or a control character (Enter, Tab, Backspace ...)
            }
            if let Some(ch) = char::from_u32(cp) {
                map.entry(ch).or_insert(Stroke { key, shift, altgr });
            }
        }
    }
    let plain = |key| Stroke { key, shift: false, altgr: false };
    map.insert('\n', plain(Key::KEY_ENTER));
    map.insert('\t', plain(Key::KEY_TAB));
    Ok(map)
}

/// Swap typographic punctuation for plain keys so it can be typed directly.
fn normalize(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' => "'".to_string(),
            '\u{201C}' | '\u{201D}' => "\"".to_string(),
            '\u{2013}' | '\u{2014}' => "-".to_string(),
            '\u{2026}' => "...".to_string(),
            '\u{00A0}' => " ".to_string(),
            c => c.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn german_layout() {
        let m = build_map("de", "nodeadkeys").unwrap();
        let k = |c: char| m.get(&c).map(|s| (s.key, s.shift, s.altgr));
        assert_eq!(k('z'), Some((Key::KEY_Y, false, false))); // QWERTZ
        assert_eq!(k('ö'), Some((Key::KEY_SEMICOLON, false, false)));
        assert_eq!(k('ä'), Some((Key::KEY_APOSTROPHE, false, false)));
        assert_eq!(k('?'), Some((Key::KEY_MINUS, true, false)));
        assert_eq!(k('@'), Some((Key::KEY_Q, false, true)));
        assert_eq!(k('\''), Some((Key::KEY_BACKSLASH, true, false)));
        assert!(m.contains_key(&'ß') && m.contains_key(&'€'));
    }

    #[test]
    fn us_layout() {
        let m = build_map("us", "").unwrap();
        let s = m[&'?'];
        assert_eq!((s.key, s.shift), (Key::KEY_SLASH, true));
        assert!(!m.contains_key(&'ö'));
    }
}

/// Can we create the virtual keyboard? (permission check for setup)
pub fn can_type() -> bool {
    std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .is_ok()
}
