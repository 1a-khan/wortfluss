//! Types text into the focused window with SendInput and KEYEVENTF_UNICODE: every
//! character is sent as itself, so it works with any keyboard layout and any script
//! (German, Turkish, Urdu, emoji) without a key map. Enter and Tab are sent as keys.
//! (Windows doesn't let a normal program type into apps running as administrator.)

use crate::layout::Layout;
use anyhow::{bail, Result};
use std::collections::HashMap;
use std::thread::sleep;
use std::time::Duration;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VK_RETURN, VK_TAB,
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
        for c in text.chars() {
            match c {
                '\n' => send(&[input(VK_RETURN, 0, 0), input(VK_RETURN, 0, KEYEVENTF_KEYUP)])?,
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

/// No key map is needed on Windows (kept for the setup's layout check).
pub fn build_map(_layout: &str, _variant: &str) -> Result<HashMap<char, ()>> {
    Ok(HashMap::new())
}

/// Typing needs no permission on Windows.
pub fn can_type() -> bool {
    true
}
