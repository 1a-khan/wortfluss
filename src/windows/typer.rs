//! Puts text into the focused window. Normally it is pasted: put on the clipboard (kept
//! out of the Win+V history), Ctrl+V, and the user's clipboard put back. Typing it as
//! a stream of KEYEVENTF_UNICODE key events, the obvious way, garbles text in some
//! apps: Windows 11 Notepad repeats or drops characters ("eeeeeeeer") when they arrive
//! faster than it handles them. Typing stays as the fallback when the clipboard can't
//! be used; it works with any keyboard layout and script without a key map.
//! (Windows doesn't let a normal program type into apps running as administrator.)

use crate::layout::Layout;
use anyhow::{bail, Result};
use std::collections::HashMap;
use std::thread::sleep;
use std::time::Duration;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VK_RETURN, VK_SHIFT, VK_TAB,
};

/// Name of the virtual keyboard on Linux; on Windows our input is recognised as injected.
#[allow(dead_code)]
pub const DEVICE_NAME: &str = "vtt virtual keyboard";

pub struct Typer {
    delay: Duration,
}

fn input(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

fn send(inputs: &[INPUT]) -> Result<()> {
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        bail!(
            "Windows blocked the typing ({}). Apps running as administrator can't receive typed text",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

impl Typer {
    pub fn new(delay_ms: u64) -> Result<Self> {
        Ok(Self { delay: Duration::from_millis(delay_ms) })
    }

    pub fn type_text(&mut self, text: &str, _layout: &Layout) -> Result<()> {
        wait_keys_released();
        match paste(text) {
            Ok(()) => Ok(()),
            Err(Pasting::NoClipboard) => self.type_keys(text),
            Err(Pasting::Blocked(e)) => Err(e),
        }
    }

    /// Key by key, as Unicode key events.
    fn type_keys(&mut self, text: &str) -> Result<()> {
        for c in text.chars() {
            match c {
                // Shift+Enter: a new line that doesn't send the message in chat apps.
                '\n' => send(&[
                    input(VK_SHIFT, 0, 0),
                    input(VK_RETURN, 0, 0),
                    input(VK_RETURN, 0, KEYEVENTF_KEYUP),
                    input(VK_SHIFT, 0, KEYEVENTF_KEYUP),
                ])?,
                '\t' => send(&[input(VK_TAB, 0, 0), input(VK_TAB, 0, KEYEVENTF_KEYUP)])?,
                '\r' => {}
                c => {
                    let mut units = [0u16; 2];
                    let mut ev = Vec::with_capacity(4);
                    for &u in c.encode_utf16(&mut units).iter() {
                        ev.push(input(0, u, KEYEVENTF_UNICODE));
                        ev.push(input(0, u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
                    }
                    send(&ev)?;
                }
            }
            sleep(self.delay);
        }
        Ok(())
    }
}

/// Wait (at most 3 s) until no modifier, Enter or Escape key is held, so they don't mix
/// with the keys we send (a held Ctrl or Alt turns them into other shortcuts).
pub fn wait_keys_released() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    let start = std::time::Instant::now();
    while [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN, VK_RETURN, VK_ESCAPE]
        .iter()
        .any(|&k| unsafe { GetAsyncKeyState(k as i32) } as u16 & 0x8000 != 0)
    {
        if start.elapsed() > Duration::from_secs(3) {
            break;
        }
        sleep(Duration::from_millis(20));
    }
}

pub enum Pasting {
    /// The clipboard couldn't be used (another program holds it): type instead.
    NoClipboard,
    /// Windows refused the Ctrl+V (an app running as administrator is in front).
    Blocked(anyhow::Error),
}

enum Saved {
    Text(String),
    Image(arboard::ImageData<'static>),
    Nothing,
}

/// Paste `text` into the focused window and put the clipboard back as it was.
pub fn paste(text: &str) -> std::result::Result<(), Pasting> {
    use arboard::SetExtWindows;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_CONTROL;
    const VK_V: u16 = b'V' as u16;
    let mut cb = arboard::Clipboard::new().map_err(|_| Pasting::NoClipboard)?;
    let saved = match cb.get_text() {
        Ok(t) => Saved::Text(t),
        Err(_) => cb.get_image().map(Saved::Image).unwrap_or(Saved::Nothing),
    };
    cb.set().exclude_from_monitoring().text(text.to_string()).map_err(|_| Pasting::NoClipboard)?;
    send(&[
        input(VK_CONTROL, 0, 0),
        input(VK_V, 0, 0),
        input(VK_V, 0, KEYEVENTF_KEYUP),
        input(VK_CONTROL, 0, KEYEVENTF_KEYUP),
    ])
    .map_err(Pasting::Blocked)?;
    // Apps read the clipboard while they handle the Ctrl+V, a moment later.
    sleep(Duration::from_millis(500));
    let _ = match saved {
        Saved::Text(t) => cb.set_text(t),
        Saved::Image(i) => cb.set_image(i),
        Saved::Nothing => cb.clear(),
    };
    Ok(())
}

/// No key map is needed on Windows (kept for the setup's layout check).
pub fn build_map(_layout: &str, _variant: &str) -> Result<HashMap<char, ()>> {
    Ok(HashMap::new())
}

/// Typing needs no permission on Windows.
pub fn can_type() -> bool {
    true
}
