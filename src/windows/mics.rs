//! Choosing the microphone and recording sources on Windows, through cpal (WASAPI).
//! Devices have readable names there ("Microphone (Jabra Evolve2 65)"), stored as is.
//! "What the computer plays" is recorded with WASAPI loopback on the default output, so
//! it follows a switch to headphones at the start of a session.

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait};
use std::sync::Mutex;

#[derive(Clone, Debug)]
pub struct Mic {
    /// The device name, stored in config.toml.
    pub name: String,
    /// Readable name (the same on Windows).
    pub description: String,
    pub is_default: bool,
    pub muted: bool,
}

/// Same names as on Linux, so config.toml and session options mean the same.
pub const SYSTEM_AUDIO: &str = "@DEFAULT_MONITOR@";
pub const DEFAULT_MIC: &str = "@DEFAULT_SOURCE@";

static CHOSEN: Mutex<String> = Mutex::new(String::new());

fn name_of(d: &cpal::Device) -> String {
    d.name().unwrap_or_default()
}

/// All microphones (input devices).
pub fn list() -> Result<Vec<Mic>> {
    let host = cpal::default_host();
    let default = host.default_input_device().map(|d| name_of(&d)).unwrap_or_default();
    let mut out: Vec<Mic> = host
        .input_devices()
        .context("can't list the microphones")?
        .map(|d| {
            let name = name_of(&d);
            Mic { description: name.clone(), is_default: name == default, muted: false, name }
        })
        .filter(|m| !m.name.is_empty())
        .collect();
    out.dedup_by(|a, b| a.name == b.name);
    Ok(out)
}

/// Record from this microphone from now on ("" = the system default).
pub fn select(name: &str) {
    *CHOSEN.lock().unwrap_or_else(|e| e.into_inner()) = name.trim().to_string();
}

/// The chosen microphone, or the default one if it isn't connected.
pub fn chosen_input(host: &cpal::Host) -> Option<cpal::Device> {
    let chosen = CHOSEN.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if !chosen.is_empty() {
        if let Some(d) = host.input_devices().ok()?.find(|d| name_of(d) == chosen) {
            return Some(d);
        }
    }
    host.default_input_device()
}

/// Readable name for a stored mic setting.
pub fn describe(name: &str) -> String {
    let host = cpal::default_host();
    if name.trim().is_empty() {
        let now = host.default_input_device().map(|d| format!(" (now: {})", name_of(&d))).unwrap_or_default();
        return format!("system default{now}");
    }
    match list() {
        Ok(l) if l.iter().any(|m| m.name == name) => name.to_string(),
        Ok(_) => format!("{name} (not connected: the default mic is used)"),
        Err(_) => name.to_string(),
    }
}

/// Records one source as 16 kHz mono, so several can be recorded at once (sessions).
/// SYSTEM_AUDIO = what the default output plays; DEFAULT_MIC; or a device by name
/// (a microphone, or an output device to record what it plays).
pub struct SourceRecorder {
    rec: crate::audio::Recorder,
}

impl SourceRecorder {
    pub fn start(source: &str) -> Result<Self> {
        let host = cpal::default_host();
        let rec = match source {
            SYSTEM_AUDIO => {
                let out = host.default_output_device().context("no sound output found")?;
                crate::audio::Recorder::from_device(&out, true)?
            }
            DEFAULT_MIC => crate::audio::Recorder::from_device(&host.default_input_device().context("no microphone found")?, false)?,
            name => {
                if let Some(d) = host.input_devices()?.find(|d| name_of(d) == name) {
                    crate::audio::Recorder::from_device(&d, false)?
                } else if let Some(d) = host.output_devices()?.find(|d| name_of(d) == name) {
                    crate::audio::Recorder::from_device(&d, true)?
                } else {
                    anyhow::bail!("no sound device called `{name}`")
                }
            }
        };
        Ok(Self { rec })
    }

    /// See `audio::Recorder::take_piece`.
    pub fn take_piece(&self, min: f32, max: f32) -> Option<crate::audio::Clip> {
        self.rec.take_piece(min, max)
    }

    pub fn stop(self) -> crate::audio::Clip {
        self.rec.stop()
    }

    /// Is the stream still delivering? False once the device is gone (unplugged,
    /// disabled).
    pub fn is_alive(&self) -> bool {
        self.rec.is_alive()
    }
}

/// Is the current output headphones or a headset? None if the name doesn't tell.
pub fn output_is_headphones() -> Option<bool> {
    let name = name_of(&cpal::default_host().default_output_device()?).to_lowercase();
    let phones = ["headphone", "headset", "kopfhörer", "earbuds", "buds", "airpods", "hands-free"];
    let speakers = ["speaker", "lautsprecher", "hoparlör", "monitor", "tv", "display"];
    if phones.iter().any(|w| name.contains(w)) {
        Some(true)
    } else if speakers.iter().any(|w| name.contains(w)) {
        Some(false)
    } else {
        None
    }
}
