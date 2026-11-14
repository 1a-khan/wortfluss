//! Raw key events on Windows: a low-level keyboard hook (WH_KEYBOARD_LL). It sees every
//! keyboard, needs no permissions, and isn't fooled by our own typing (injected events are
//! skipped). One hook thread per process hands events to all subscribers.
//!
//! Keys use the Linux key numbers and names ("KEY_RIGHTCTRL" = 97), so config.toml is the
//! same on both systems. Most Linux numbers are the PC scan codes; keys with the E0 prefix
//! (right Ctrl/Alt, arrows, Windows keys ...) and a few special ones are mapped below.

use anyhow::{anyhow, Result};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse as vk;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage, HC_ACTION, KBDLLHOOKSTRUCT,
    LLKHF_EXTENDED, LLKHF_INJECTED, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
};

/// A key, numbered like Linux (evdev) numbers it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key(u16);

macro_rules! keys {
    ($($name:ident = $code:expr),* $(,)?) => {
        #[allow(dead_code)]
        impl Key {
            $(pub const $name: Key = Key($code);)*
        }
        const NAMES: &[(&str, u16)] = &[$((stringify!($name), $code)),*];
    };
}

keys! {
    KEY_ESC = 1, KEY_1 = 2, KEY_2 = 3, KEY_3 = 4, KEY_4 = 5, KEY_5 = 6, KEY_6 = 7, KEY_7 = 8,
    KEY_8 = 9, KEY_9 = 10, KEY_0 = 11, KEY_MINUS = 12, KEY_EQUAL = 13, KEY_BACKSPACE = 14,
    KEY_TAB = 15, KEY_Q = 16, KEY_W = 17, KEY_E = 18, KEY_R = 19, KEY_T = 20, KEY_Y = 21,
    KEY_U = 22, KEY_I = 23, KEY_O = 24, KEY_P = 25, KEY_LEFTBRACE = 26, KEY_RIGHTBRACE = 27,
    KEY_ENTER = 28, KEY_LEFTCTRL = 29, KEY_A = 30, KEY_S = 31, KEY_D = 32, KEY_F = 33,
    KEY_G = 34, KEY_H = 35, KEY_J = 36, KEY_K = 37, KEY_L = 38, KEY_SEMICOLON = 39,
    KEY_APOSTROPHE = 40, KEY_GRAVE = 41, KEY_LEFTSHIFT = 42, KEY_BACKSLASH = 43, KEY_Z = 44,
    KEY_X = 45, KEY_C = 46, KEY_V = 47, KEY_B = 48, KEY_N = 49, KEY_M = 50, KEY_COMMA = 51,
    KEY_DOT = 52, KEY_SLASH = 53, KEY_RIGHTSHIFT = 54, KEY_KPASTERISK = 55, KEY_LEFTALT = 56,
    KEY_SPACE = 57, KEY_CAPSLOCK = 58, KEY_F1 = 59, KEY_F2 = 60, KEY_F3 = 61, KEY_F4 = 62,
    KEY_F5 = 63, KEY_F6 = 64, KEY_F7 = 65, KEY_F8 = 66, KEY_F9 = 67, KEY_F10 = 68,
    KEY_NUMLOCK = 69, KEY_SCROLLLOCK = 70, KEY_KP7 = 71, KEY_KP8 = 72, KEY_KP9 = 73,
    KEY_KPMINUS = 74, KEY_KP4 = 75, KEY_KP5 = 76, KEY_KP6 = 77, KEY_KPPLUS = 78, KEY_KP1 = 79,
    KEY_KP2 = 80, KEY_KP3 = 81, KEY_KP0 = 82, KEY_KPDOT = 83, KEY_102ND = 86, KEY_F11 = 87,
    KEY_F12 = 88, KEY_KPENTER = 96, KEY_RIGHTCTRL = 97, KEY_KPSLASH = 98, KEY_SYSRQ = 99,
    KEY_RIGHTALT = 100, KEY_HOME = 102, KEY_UP = 103, KEY_PAGEUP = 104, KEY_LEFT = 105,
    KEY_RIGHT = 106, KEY_END = 107, KEY_DOWN = 108, KEY_PAGEDOWN = 109, KEY_INSERT = 110,
    KEY_DELETE = 111, KEY_MUTE = 113, KEY_VOLUMEDOWN = 114, KEY_VOLUMEUP = 115,
    KEY_PAUSE = 119, KEY_LEFTMETA = 125, KEY_RIGHTMETA = 126, KEY_COMPOSE = 127,
    KEY_NEXTSONG = 163, KEY_PLAYPAUSE = 164, KEY_PREVIOUSSONG = 165, KEY_STOPCD = 166,
    KEY_F13 = 183, KEY_F14 = 184, KEY_F15 = 185, KEY_F16 = 186, KEY_F17 = 187, KEY_F18 = 188,
    KEY_F19 = 189, KEY_F20 = 190, KEY_F21 = 191, KEY_F22 = 192, KEY_F23 = 193, KEY_F24 = 194,
}

impl Key {
    pub fn code(self) -> u16 {
        self.0
    }
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match NAMES.iter().find(|n| n.1 == self.0) {
            Some((name, _)) => f.write_str(name),
            None => write!(f, "unknown key: {}", self.0),
        }
    }
}

impl std::str::FromStr for Key {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        NAMES.iter().find(|n| n.0 == s).map(|n| Key(n.1)).ok_or(())
    }
}

/// The Linux number for a key from the hook (virtual key, scan code, E0 prefix).
fn to_key(vk_code: u32, scan: u32, extended: bool) -> Option<Key> {
    let v = vk_code as u16;
    // Keys whose scan code is ambiguous or missing: by virtual key.
    let by_vk = match v {
        vk::VK_PAUSE => Some(119),
        vk::VK_NUMLOCK => Some(69),
        vk::VK_SNAPSHOT => Some(99),
        vk::VK_F13..=vk::VK_F24 => Some(183 + (v - vk::VK_F13)),
        vk::VK_VOLUME_MUTE => Some(113),
        vk::VK_VOLUME_DOWN => Some(114),
        vk::VK_VOLUME_UP => Some(115),
        vk::VK_MEDIA_NEXT_TRACK => Some(163),
        vk::VK_MEDIA_PLAY_PAUSE => Some(164),
        vk::VK_MEDIA_PREV_TRACK => Some(165),
        vk::VK_MEDIA_STOP => Some(166),
        vk::VK_LWIN => Some(125),
        vk::VK_RWIN => Some(126),
        vk::VK_APPS => Some(127),
        vk::VK_RCONTROL => Some(97),
        vk::VK_RMENU => Some(100),
        _ => None,
    };
    if let Some(c) = by_vk {
        return Some(Key(c));
    }
    let scan = scan as u16;
    let code = if extended {
        match scan {
            0x1C => 96,  // keypad Enter
            0x1D => 97,  // right Ctrl
            0x35 => 98,  // keypad /
            0x38 => 100, // right Alt / AltGr
            0x47 => 102,
            0x48 => 103,
            0x49 => 104,
            0x4B => 105,
            0x4D => 106,
            0x4F => 107,
            0x50 => 108,
            0x51 => 109,
            0x52 => 110,
            0x53 => 111,
            _ => return None,
        }
    } else if (1..=88).contains(&scan) {
        scan
    } else {
        return None;
    };
    Some(Key(code))
}

/// There is always a keyboard hook, whatever is plugged in.
pub fn keyboard_count() -> usize {
    1
}

static SUBSCRIBERS: Mutex<Vec<Sender<(Key, i32)>>> = Mutex::new(Vec::new());
static HOOK: OnceLock<Result<(), String>> = OnceLock::new();
/// Keys held down (to tell auto-repeat from a new press). Only the hook thread uses it.
static HELD: Mutex<[bool; 256]> = Mutex::new([false; 256]);

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let kb = &*(lparam as *const KBDLLHOOKSTRUCT);
        // Skip injected events (our own typing) and the fake Ctrl that AltGr sends.
        let fake_ctrl = kb.scanCode & 0x200 != 0;
        if kb.flags & LLKHF_INJECTED == 0 && !fake_ctrl {
            if let Some(k) = to_key(kb.vkCode, kb.scanCode, kb.flags & LLKHF_EXTENDED != 0) {
                let down = wparam as u32 == WM_KEYDOWN || wparam as u32 == WM_SYSKEYDOWN;
                let value = {
                    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
                    let slot = &mut held[(k.0 as usize).min(255)];
                    let v = match (down, *slot) {
                        (true, true) => 2,
                        (true, false) => 1,
                        (false, _) => 0,
                    };
                    *slot = down;
                    v
                };
                if let Ok(mut subs) = SUBSCRIBERS.lock() {
                    subs.retain(|tx| tx.send((k, value)).is_ok());
                }
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}

/// Key events (key, 0 up / 1 down / 2 repeat) from all keyboards until the receiver is
/// dropped. The hook thread is started on first use and lives as long as the process.
pub fn subscribe(_extra: Option<Key>) -> Result<Receiver<(Key, i32)>> {
    let started = HOOK.get_or_init(|| {
        let (ready_tx, ready_rx) = mpsc::channel();
        std::thread::spawn(move || unsafe {
            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), GetModuleHandleW(std::ptr::null()), 0);
            if hook.is_null() {
                let _ = ready_tx.send(Err(format!("keyboard hook failed: {}", std::io::Error::last_os_error())));
                return;
            }
            let _ = ready_tx.send(Ok(()));
            // The hook only runs while this thread handles messages.
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
        ready_rx.recv().unwrap_or_else(|_| Err("keyboard hook thread ended".into()))
    });
    if let Err(e) = started {
        return Err(anyhow!("{e}"));
    }
    let (tx, rx) = mpsc::channel();
    SUBSCRIBERS.lock().map_err(|_| anyhow!("keyboard hook broken"))?.push(tx);
    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_keys_map_to_linux_numbers() {
        assert_eq!(to_key(0x41, 0x1E, false), Some(Key::KEY_A));
        assert_eq!(to_key(vk::VK_RCONTROL as u32, 0x1D, true), Some(Key::KEY_RIGHTCTRL));
        assert_eq!(to_key(vk::VK_LCONTROL as u32, 0x1D, false), Some(Key::KEY_LEFTCTRL));
        assert_eq!(to_key(vk::VK_F9 as u32, 0x43, false), Some(Key::KEY_F9));
        assert_eq!(to_key(vk::VK_PAUSE as u32, 0x45, false), Some(Key::KEY_PAUSE));
        assert_eq!(to_key(vk::VK_NUMLOCK as u32, 0x45, true), Some(Key::KEY_NUMLOCK));
        assert_eq!(to_key(vk::VK_UP as u32, 0x48, true), Some(Key::KEY_UP));
        assert_eq!(to_key(vk::VK_F13 as u32, 0x64, false), Some(Key::KEY_F13));
        assert_eq!("KEY_RIGHTCTRL".parse::<Key>(), Ok(Key::KEY_RIGHTCTRL));
        assert_eq!(format!("{:?}", Key::KEY_COMPOSE), "KEY_COMPOSE");
    }
}
