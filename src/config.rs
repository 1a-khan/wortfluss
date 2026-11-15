//! Settings in config.toml. `vtt setup` writes this file; it can also be edited by hand.
//! Each provider has its own section, so switching provider keeps the other keys.

use crate::platform;
use crate::providers;
use crate::secrets;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Record while the hotkey is held down.
    Hold,
    /// Press once to start recording, press again to stop.
    Toggle,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct ProviderSettings {
    pub api_key: String,
    pub model: String,
    /// Empty = the provider's standard address.
    pub api_url: String,
    /// Text model for spelling/grammar correction. Empty = recommended one.
    pub correction_model: String,
    /// Speech model that labels speakers (sessions with speaker separation). Empty =
    /// recommended one.
    pub speaker_model: String,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub provider: String,
    pub language: String,
    pub prompt: String,
    /// Correct spelling, grammar and punctuation with a text model before typing.
    pub correct_text: bool,
    pub hotkey: String,
    /// Microphone (sound-server source name). Empty = system default.
    pub mic: String,
    pub mode: Mode,
    pub sounds: bool,
    pub keyboard_layout: String,
    pub keyboard_variant: String,
    /// Session mode: default source ("system", "mic", "both"), notes ("auto", "lecture",
    /// "meeting", "summary", "none"), transcript folder ("" = ~/Documents/vtt) and an
    /// optional hotkey that starts/stops a session.
    pub session_source: String,
    pub session_notes: String,
    pub session_dir: String,
    pub session_hotkey: String,
    /// Keep a session's audio next to the transcript (asked to delete it afterwards).
    pub session_keep_audio: bool,
    /// Kept session audio is deleted automatically after this many days.
    pub session_audio_days: u64,
    /// Separate speakers in sessions ("Speaker 1: ...", "Speaker 2: ...").
    pub session_speakers: bool,
    /// Keyboard shortcuts ("Ctrl+Alt+S"; "" = off).
    pub shortcut_session: String,
    pub shortcut_call: String,
    pub shortcut_correction: String,
    pub shortcut_mode: String,
    pub shortcut_settings: String,
    /// Rewrite the selected text (correct it or change its tone).
    pub shortcut_rewrite: String,
    /// Interface language: "auto" (the system's) or en, de, tr, es, pl, it.
    pub ui_language: String,
    /// Install new versions automatically (checked every few hours).
    pub auto_update: bool,
    /// Show the tray icon.
    pub tray: bool,
    /// Show a small sound wave next to the text cursor while listening.
    pub indicator: bool,
    pub max_seconds: u64,
    /// Long recordings are sent in pieces of about this many seconds, cut at a pause.
    pub piece_seconds: u64,
    pub add_trailing_space: bool,
    pub key_delay_ms: u64,

    pub groq: ProviderSettings,
    pub openai: ProviderSettings,
    pub openrouter: ProviderSettings,
    pub mistral: ProviderSettings,
    pub custom: ProviderSettings,

    /// Some key is still written in plain text in config.toml (moved out on next save).
    #[serde(skip)]
    pub plaintext_keys: bool,

    // Settings from vtt 0.1 (one flat Groq config), moved into [groq] on load.
    groq_api_key: Option<String>,
    api_url: Option<String>,
    model: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            provider: "groq".into(),
            language: String::new(),
            prompt: String::new(),
            correct_text: false,
            hotkey: "KEY_RIGHTCTRL".into(),
            mic: String::new(),
            mode: Mode::Hold,
            sounds: true,
            keyboard_layout: "auto".into(),
            keyboard_variant: String::new(),
            session_source: "system".into(),
            session_notes: "auto".into(),
            session_dir: String::new(),
            session_hotkey: String::new(),
            session_keep_audio: false,
            session_audio_days: 7,
            session_speakers: false,
            shortcut_session: "Ctrl+Alt+S".into(),
            shortcut_call: "Ctrl+Alt+M".into(),
            shortcut_correction: "Ctrl+Alt+K".into(),
            shortcut_mode: "Ctrl+Alt+H".into(),
            shortcut_settings: "Ctrl+Alt+V".into(),
            shortcut_rewrite: "Ctrl+Alt+R".into(),
            ui_language: "auto".into(),
            auto_update: true,
            tray: true,
            indicator: true,
            max_seconds: 120,
            piece_seconds: 30,
            add_trailing_space: true,
            key_delay_ms: 3,
            groq: ProviderSettings::default(),
            openai: ProviderSettings::default(),
            openrouter: ProviderSettings::default(),
            mistral: ProviderSettings::default(),
            custom: ProviderSettings::default(),
            plaintext_keys: false,
            groq_api_key: None,
            api_url: None,
            model: None,
        }
    }
}

/// Everything needed to send a request to the chosen provider.
#[derive(Clone)]
pub struct Active {
    pub provider: &'static providers::Provider,
    pub api_url: String,
    pub api_key: String,
    pub model: String,
    pub correction_model: String,
    /// Empty when the service can't label speakers.
    pub speaker_model: String,
}

impl Config {
    pub fn settings(&self, provider: &str) -> &ProviderSettings {
        match provider {
            "openai" => &self.openai,
            "openrouter" => &self.openrouter,
            "mistral" => &self.mistral,
            "custom" => &self.custom,
            _ => &self.groq,
        }
    }

    pub fn settings_mut(&mut self, provider: &str) -> &mut ProviderSettings {
        match provider {
            "openai" => &mut self.openai,
            "openrouter" => &mut self.openrouter,
            "mistral" => &mut self.mistral,
            "custom" => &mut self.custom,
            _ => &mut self.groq,
        }
    }

    pub fn active(&self) -> Active {
        let provider = providers::get(&self.provider);
        let settings = self.settings(provider.id);
        let api_url = if settings.api_url.trim().is_empty() {
            provider.api_url.to_string()
        } else {
            settings.api_url.trim().to_string()
        };
        // An environment variable (e.g. GROQ_API_KEY) overrides the stored key. It may
        // hold a plain key or an encrypted value copied from secrets.env.
        let api_key = provider
            .key_env
            .and_then(|var| {
                let v = std::env::var(var).ok().filter(|k| !k.trim().is_empty())?;
                secrets::decrypt(var, &v).ok()
            })
            .unwrap_or_else(|| settings.api_key.clone())
            .trim()
            .to_string();
        let model = if settings.model.trim().is_empty() {
            provider.models.first().map(|m| m.id).unwrap_or("").to_string()
        } else {
            settings.model.trim().to_string()
        };
        let correction_model = if settings.correction_model.trim().is_empty() {
            providers::chat_models(provider.id)
                .first()
                .map(|m| m.id)
                .unwrap_or("")
                .to_string()
        } else {
            settings.correction_model.trim().to_string()
        };
        let speaker_model = if settings.speaker_model.trim().is_empty() {
            crate::stt::speaker_model(provider.id).unwrap_or("").to_string()
        } else {
            settings.speaker_model.trim().to_string()
        };
        Active {
            provider,
            api_url,
            api_key,
            model,
            correction_model,
            speaker_model,
        }
    }

    /// The phrases (names, words to spell right), stored comma-separated in `prompt`.
    pub fn phrases(&self) -> Vec<String> {
        split_phrases(&self.prompt)
    }

    pub fn set_phrases(&mut self, list: Vec<String>) {
        self.prompt = split_phrases(&list.join(", ")).join(", ");
    }

    /// The configured keyboard shortcuts that are valid.
    pub fn shortcuts(&self) -> Vec<(crate::hotkey::Combo, crate::hotkey::Action)> {
        use crate::hotkey::{Action, Combo};
        [
            (&self.shortcut_session, Action::Session),
            (&self.shortcut_call, Action::CallSession),
            (&self.shortcut_correction, Action::ToggleCorrection),
            (&self.shortcut_mode, Action::ToggleMode),
            (&self.shortcut_settings, Action::OpenSettings),
            (&self.shortcut_rewrite, Action::Rewrite),
        ]
        .into_iter()
        .filter_map(|(text, action)| Some((Combo::parse(text)?, action)))
        .collect()
    }

    pub fn layout_is_auto(&self) -> bool {
        matches!(self.keyboard_layout.trim(), "" | "auto")
    }

    /// What's missing before vtt can transcribe, in plain words.
    pub fn problem(&self) -> Option<String> {
        let a = self.active();
        if a.provider.needs_key && a.api_key.is_empty() {
            return Some(format!("no API key for {}", a.provider.name));
        }
        if a.model.is_empty() {
            return Some(format!("no model chosen for {}", a.provider.name));
        }
        if a.api_url.is_empty() {
            return Some(format!("no server address for {}", a.provider.name));
        }
        None
    }

    fn migrate(&mut self) {
        if let Some(key) = self.groq_api_key.take() {
            if self.groq.api_key.is_empty() {
                self.groq.api_key = key;
            }
        }
        if let Some(model) = self.model.take() {
            if self.groq.model.is_empty() {
                self.groq.model = model;
            }
        }
        if let Some(url) = self.api_url.take() {
            if self.groq.api_url.is_empty() && url.trim_end_matches('/') != providers::GROQ_URL {
                self.groq.api_url = url;
            }
        }
        if self.keyboard_layout.trim().is_empty() {
            self.keyboard_layout = "auto".into();
        }
    }
}

/// "(Ammad, Salamalaikum, salamalaikum)" -> ["Ammad", "Salamalaikum"]: split on commas,
/// semicolons and new lines, drop brackets/quotes around items and case-insensitive repeats.
pub fn split_phrases(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in text.split([',', ';', '\n']) {
        let p = part
            .trim()
            .trim_matches(|c: char| "()[]{}\"'".contains(c))
            .trim()
            .to_string();
        if !p.is_empty() && !out.iter().any(|o| o.to_lowercase() == p.to_lowercase()) {
            out.push(p);
        }
    }
    out
}

pub fn path() -> Result<PathBuf> {
    Ok(platform::config_dir()?.join("config.toml"))
}

pub fn exists() -> bool {
    path().map(|p| p.exists()).unwrap_or(false)
}

/// Read config.toml plus the API keys from secrets.env; defaults if nothing exists yet.
pub fn load() -> Result<Config> {
    let mut cfg = load_settings()?;
    let stored = secrets::read_all()?;
    let active = cfg.provider.clone();
    for p in providers::PROVIDERS {
        let s = cfg.settings_mut(p.id);
        if !s.api_key.trim().is_empty() {
            continue; // plain key in config.toml (older setup) wins until it is moved
        }
        let name = secrets::var_name(p.id);
        let Some(value) = stored.get(&name) else { continue };
        match secrets::decrypt(&name, value) {
            Ok(key) => s.api_key = key,
            // Only the service in use must work; others can be fixed when chosen.
            Err(e) if p.id == active => return Err(e),
            Err(e) => eprintln!("note: {e:#}"),
        }
    }
    Ok(cfg)
}

/// Only config.toml (no keyring access).
fn load_settings() -> Result<Config> {
    let path = path()?;
    let mut cfg: Config = match std::fs::read_to_string(&path) {
        Ok(text) => {
            toml::from_str(&text).with_context(|| format!("error in {}", path.display()))?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    cfg.migrate();
    cfg.plaintext_keys = providers::PROVIDERS
        .iter()
        .any(|p| !cfg.settings(p.id).api_key.trim().is_empty());
    Ok(cfg)
}

/// Write config.toml, and the API keys encrypted to secrets.env. Keys are never
/// written to config.toml.
pub fn save(cfg: &Config) -> Result<PathBuf> {
    let keys = providers::PROVIDERS
        .iter()
        .map(|p| (secrets::var_name(p.id), cfg.settings(p.id).api_key.trim().to_string()))
        .filter(|(_, k)| !k.is_empty())
        .collect();
    secrets::store(&keys)?; // first, so a failure can't lose the only copy of a key
    let path = path()?;
    platform::write_private(&path, &render(cfg))?;
    Ok(path)
}

fn q(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

/// The config file, with explanations, as `vtt setup` writes it.
fn render(c: &Config) -> String {
    let mode = match c.mode {
        Mode::Hold => "hold",
        Mode::Toggle => "toggle",
    };
    let mut out = format!(
        r#"# vtt settings. Easiest way to change them: run `vtt setup`.
# You can also edit this file; a running vtt picks up changes within a few seconds.

# Speech recognition service: "groq", "openai", "openrouter", "mistral" or "custom" (any
# OpenAI-compatible server). Each one has its own section at the bottom.
provider = {provider}

# Language you speak as a 2-letter code ("en", "de", "tr", "ur" ...).
# Empty = detect automatically (best if you switch languages).
language = {language}

# Language of the tray menu, notifications and rewrite window: "auto" (the system's)
# or "en", "de", "tr", "es", "pl", "it".
ui_language = {ui_language}

# Install new versions automatically in the background (signed releases only;
# switches when idle, never during a dictation or session). `vtt update` checks now.
auto_update = {auto_update}

# Phrases to spell exactly like this (names, brands, jargon), comma separated.
# Easier: `vtt phrases add "Ahmad Khan, Groq"`. Example: "Ahmad Khan, Groq, Kubernetes"
prompt = {prompt}

# Fix spelling, grammar and punctuation with a small text model before typing
# (adds ~0.3-1 s and a tiny cost; the model is `correction_model` in the service's section)
correct_text = {correct}

# Key that starts dictation (Linux key name). `vtt setup` lets you just press it.
hotkey = {hotkey}

# Microphone to record from. Empty = the system default (Settings > Sound).
# Easiest: `vtt set mic`. `vtt mics` lists the names.
mic = {mic}

# "hold" = hold the key while you speak; "toggle" = press to start, press again to stop
mode = "{mode}"

# Short beep when recording starts and stops
sounds = {sounds}

# Small sound wave next to the text cursor while listening (on GNOME through a small
# extension that vtt installs; it appears after logging out and in once)
indicator = {indicator}

# Keyboard layout used to type the text. "auto" = follow the desktop, including
# switching layouts. Or set one, e.g. keyboard_layout = "de", keyboard_variant = "nodeadkeys"
keyboard_layout = {layout}
keyboard_variant = {variant}

# SESSION MODE (`vtt session start`, or the session key): transcribe what the computer
# plays and/or the microphone into a Markdown file, then write notes.
# session_source: "system" (lectures, videos), "mic" (a room), "both" (calls: You/Others)
# session_notes: "auto" (meeting minutes for "both", else lecture notes), "lecture",
#   "meeting", "summary" or "none"
# session_dir: where transcripts go ("" = ~/Documents/vtt)
# session_hotkey: a key that starts/stops a session (e.g. "KEY_F10"), "" = none
session_source = {s_source}
session_notes = {s_notes}
session_dir = {s_dir}
session_hotkey = {s_key}

# Keep the session audio next to the transcript (in a hidden .audio folder), e.g. to redo
# a transcript with `vtt session redo FILE`. Off by default: it contains voices. After a
# session vtt asks whether to delete it; kept audio is deleted after session_audio_days.
session_keep_audio = {keep}
session_audio_days = {days}

# Separate speakers in sessions: "Speaker 1: ...", "Speaker 2: ..." (rename them later with
# `vtt speakers FILE`). Needs a service that labels voices (OpenRouter, OpenAI, Mistral);
# the model is `speaker_model` in the service's section. Costs a bit more per hour.
session_speakers = {speakers}

# KEYBOARD SHORTCUTS ("Ctrl+Alt+S", "Super+F9" ...; "" = off). Easiest: `vtt set shortcuts`.
# The focused app also sees the keys, so pick combinations your apps don't use.
shortcut_session = {sc_session}        # start/stop a session (default source)
shortcut_call = {sc_call}              # start/stop a call session (You / Others)
shortcut_correction = {sc_corr}        # spelling correction on/off
shortcut_mode = {sc_mode}              # hold <-> toggle
shortcut_settings = {sc_set}           # open the settings in a terminal
shortcut_rewrite = {sc_rewrite}          # rewrite the selected text (correct, polite, casual ...)

# Icon in the system tray with a menu (right-click) for the common settings
tray = {tray}

# Stop recording automatically after this many seconds (stuck key, forgotten toggle).
# Toggle mode also stops by itself after 60 s without speech.
max_seconds = {max_seconds}

# While you speak, the recording is sent in pieces of about this many seconds, cut at
# a pause, so text appears while you keep talking. Pieces without speech are not sent.
piece_seconds = {piece}

# Add a space after each dictation so consecutive ones don't run together
add_trailing_space = {space}

# Milliseconds between simulated key presses (raise it if an app drops characters)
key_delay_ms = {delay}
"#,
        provider = q(&c.provider),
        language = q(&c.language),
        ui_language = q(&c.ui_language),
        auto_update = c.auto_update,
        prompt = q(&c.prompt),
        correct = c.correct_text,
        hotkey = q(&c.hotkey),
        mic = q(&c.mic),
        sounds = c.sounds,
        indicator = c.indicator,
        layout = q(&c.keyboard_layout),
        variant = q(&c.keyboard_variant),
        max_seconds = c.max_seconds,
        s_source = q(&c.session_source),
        s_notes = q(&c.session_notes),
        s_dir = q(&c.session_dir),
        s_key = q(&c.session_hotkey),
        keep = c.session_keep_audio,
        days = c.session_audio_days,
        speakers = c.session_speakers,
        sc_session = q(&c.shortcut_session),
        sc_call = q(&c.shortcut_call),
        sc_corr = q(&c.shortcut_correction),
        sc_mode = q(&c.shortcut_mode),
        sc_set = q(&c.shortcut_settings),
        sc_rewrite = q(&c.shortcut_rewrite),
        tray = c.tray,
        piece = c.piece_seconds,
        space = c.add_trailing_space,
        delay = c.key_delay_ms,
    );
    for p in providers::PROVIDERS {
        let s = c.settings(p.id);
        out.push_str(&format!(
            "\n# {} - {}\n[{}]\n# API key: stored encrypted in secrets.env as {} (set it with `vtt setup`)\n\
             model = {}\ncorrection_model = {}\nspeaker_model = {}\n",
            p.name,
            p.blurb,
            p.id,
            secrets::var_name(p.id),
            q(&s.model),
            q(&s.correction_model),
            q(&s.speaker_model)
        ));
        if p.id == "custom" {
            out.push_str("# Server address, e.g. \"http://localhost:8000/v1\"\n");
        } else {
            out.push_str("# Empty = standard address. Only change for a proxy or a regional endpoint.\n");
        }
        out.push_str(&format!("api_url = {}\n", q(&s.api_url)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_config_is_migrated() {
        let mut c: Config = toml::from_str(
            r#"groq_api_key = "gsk_abc"
               api_url = "https://api.groq.com/openai/v1"
               model = "whisper-large-v3"
               keyboard_layout = """#,
        )
        .unwrap();
        c.migrate();
        assert_eq!(c.groq.api_key, "gsk_abc");
        assert_eq!(c.groq.model, "whisper-large-v3");
        assert_eq!(c.groq.api_url, "");
        assert_eq!(c.provider, "groq");
        assert!(c.layout_is_auto());
    }

    #[test]
    fn phrases_are_cleaned() {
        assert_eq!(
            split_phrases("(Ammad, Salamalaikum, salamalaikum, salam)"),
            vec!["Ammad", "Salamalaikum", "salam"]
        );
        let mut c = Config::default();
        c.set_phrases(vec!["Groq".into(), " Ahmad Khan ".into(), "groq".into()]);
        assert_eq!(c.prompt, "Groq, Ahmad Khan");
    }

    #[test]
    fn written_file_reads_back() {
        let mut c = Config::default();
        c.provider = "openai".into();
        c.openai.api_key = "sk-\"quoted\"".into();
        c.prompt = "Ahmad Khan, Groq".into();
        c.correct_text = true;
        c.openai.correction_model = "gpt-6-luna".into();
        let back: Config = toml::from_str(&render(&c)).unwrap();
        assert_eq!(back.provider, "openai");
        assert_eq!(back.openai.api_key, "", "keys never go into config.toml");
        assert!(!render(&c).contains("quoted"));
        assert_eq!(back.prompt, "Ahmad Khan, Groq");
        assert_eq!(back.active().provider.id, "openai");
        assert!(back.correct_text);
        assert_eq!(back.active().correction_model, "gpt-6-luna");
        assert_eq!(Config::default().active().correction_model, "qwen/qwen3.8-27b");
    }
}
