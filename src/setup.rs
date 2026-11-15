//! `vtt setup`: step-by-step settings in the terminal, and `vtt check`: the self-test.
//! Setup can be run again at any time; current values are preselected, so Enter keeps them.

use crate::config::{self, Config, Mode};
use crate::{audio, correct, hotkey, layout, mics, platform, providers, secrets, stt, typer};
use anyhow::{bail, Result};
use crate::stt::ModelEntry;
use dialoguer::{theme::ColorfulTheme, Confirm, FuzzySelect, Input, MultiSelect, Password, Select};
use std::io::{IsTerminal, Write};
use std::time::Duration;

const LANGUAGES: &[(&str, &str)] = &[
    ("", "Detect automatically (recommended if you switch languages)"),
    ("en", "English"),
    ("de", "German / Deutsch"),
    ("tr", "Turkish / Türkçe"),
    ("ur", "Urdu"),
    ("ar", "Arabic"),
    ("hi", "Hindi"),
    ("fr", "French"),
    ("es", "Spanish"),
    ("it", "Italian"),
    ("nl", "Dutch"),
];

const PERMISSION_FIX: &str = "\
  Fix (one time, needs your password), then log out and back in:
    sudo usermod -aG input \"$USER\"
    echo 'KERNEL==\"uinput\", GROUP=\"input\", MODE=\"0660\", OPTIONS+=\"static_node=uinput\"' \\
      | sudo tee /etc/udev/rules.d/80-vtt-uinput.rules
    sudo udevadm control --reload-rules && sudo udevadm trigger";

fn ok(good: bool, text: &str) {
    println!("  {} {text}", if good { "[ok]  " } else { "[FAIL]" });
}

fn heading(n: usize, text: &str) {
    println!("\n── {n}. {text} ──");
}

fn mask(key: &str) -> String {
    let n = key.chars().count();
    if n <= 8 {
        return "••••".into();
    }
    let start: String = key.chars().take(4).collect();
    let end: String = key.chars().skip(n - 4).collect();
    format!("{start}…{end}")
}

/// Keyboard, typing and microphone. Returns false if vtt can't work yet.
fn system_checks() -> bool {
    let keyboards = hotkey::keyboard_count();
    let typing = typer::can_type();
    ok(keyboards > 0, &format!("read the hotkey ({keyboards} keyboard(s) found)"));
    ok(typing, "type text (virtual keyboard)");
    if keyboards == 0 || !typing {
        println!("{PERMISSION_FIX}");
    }
    let mic = audio::Recorder::start().map(|r| {
        std::thread::sleep(Duration::from_millis(500));
        r.stop()
    });
    match mic {
        Ok(clip) if clip.seconds() > 0.1 => ok(true, "microphone"),
        Ok(_) => ok(false, "microphone gives no sound: check Settings > Sound > Input"),
        Err(e) => ok(false, &format!("microphone: {e:#}")),
    }
    let (l, source) = layout::detect();
    ok(true, &format!("keyboard layout {l} ({source})"));
    keyboards > 0 && typing
}

/// `vtt check`: everything vtt needs, with a fix for whatever is missing.
pub fn check() -> Result<()> {
    if let Ok(cfg) = config::load() {
        mics::select(&cfg.mic);
        println!("Microphone: {}", mics::describe(&cfg.mic));
    }
    println!("Computer:");
    system_checks();

    println!("\nSettings ({}):", config::path()?.display());
    let cfg = match config::load() {
        Ok(c) => c,
        Err(e) => {
            ok(false, &format!("{e:#}"));
            return Ok(());
        }
    };
    if !config::exists() {
        ok(false, "not set up yet: run `vtt setup`");
        return Ok(());
    }
    let a = cfg.active();
    println!("  service {} · model {} · {}", a.provider.name, a.model, a.api_url);
    if let Some(p) = cfg.problem() {
        ok(false, &format!("{p}: run `vtt setup`"));
    } else {
        match stt::check_key(a.provider, &a.api_url, &a.api_key) {
            Err(e) => ok(false, &format!("{e:#}")),
            Ok(()) => match stt::list_models(a.provider, &a.api_url, &a.api_key) {
                Ok(list) if !list.is_empty() && !list.iter().any(|m| m.id == a.model) => ok(
                    false,
                    &format!("key works, but `{}` isn't offered: run `vtt setup`", a.model),
                ),
                _ => ok(true, "API key works and the model is available"),
            },
        }
    }
    if cfg.plaintext_keys {
        ok(false, "an API key is still in plain text in config.toml: run `vtt setup` and save to encrypt it");
    } else {
        match secrets::protection() {
            secrets::Protection::Keyring => ok(true, "API keys encrypted (master key in your login keyring)"),
            secrets::Protection::Machine(why) => ok(
                false,
                &format!("API keys encrypted with a weaker machine-bound key (no keyring: {why})"),
            ),
        }
    }
    match hotkey::parse(&cfg.hotkey) {
        Some(_) => ok(true, &format!("dictation key: {}", hotkey::pretty(&cfg.hotkey))),
        None => ok(false, &format!("unknown hotkey `{}`: run `vtt setup`", cfg.hotkey)),
    }

    println!("\nRunning:");
    ok(platform::autostart_enabled(), "starts at login");
    match platform::running_pid() {
        Some(pid) => ok(true, &format!("vtt is running (process {pid})")),
        None => ok(false, "vtt is not running: start it with `vtt start`"),
    }
    Ok(())
}

/// One thing that can be changed on its own (`vtt set <name>` or the setup menu).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Setting {
    Service,
    Model,
    Key,
    Correction,
    Phrases,
    Language,
    Hotkey,
    Mic,
    Mode,
    Sounds,
    Indicator,
    Paste,
    Layout,
    Autostart,
    Session,
    Speakers,
    UiLanguage,
    Shortcuts,
    Advanced,
}

/// (setting, name for `vtt set`, other accepted names, menu label)
const SETTINGS: &[(Setting, &str, &[&str], &str)] = &[
    (Setting::Service, "service", &["provider"], "Service"),
    (Setting::Model, "model", &[], "Model"),
    (Setting::Key, "key", &["api-key", "apikey"], "API key"),
    (Setting::Correction, "correction", &["correct", "spelling", "grammar"], "Spelling & grammar correction"),
    (Setting::Phrases, "phrases", &["words", "names", "prompt"], "Phrases (names, words to spell right)"),
    (Setting::Language, "language", &["lang"], "Language"),
    (Setting::Hotkey, "hotkey", &["shortcut", "button"], "Dictation key"),
    (Setting::Mic, "mic", &["microphone", "input", "mics"], "Microphone"),
    (Setting::Mode, "mode", &["long", "toggle", "hold", "max-seconds"], "Hold/toggle & long dictations"),
    (Setting::Sounds, "sounds", &["beep", "beeps"], "Beeps"),
    (Setting::Indicator, "indicator", &["bubble", "wave", "listening"], "Listening bubble at the text cursor"),
    (Setting::Paste, "paste", &["typing", "insert"], "Put text in by pasting (fast)"),
    (Setting::Layout, "layout", &["keyboard"], "Keyboard layout"),
    (Setting::Autostart, "autostart", &["login", "startup"], "Start at login"),
    (Setting::Session, "session", &["sessions", "notes", "lecture", "meeting"], "Session mode (lectures, meetings)"),
    (Setting::Speakers, "speakers", &["speaker", "diarization", "who"], "Separate speakers in sessions"),
    (Setting::UiLanguage, "ui-language", &["interface", "ui", "sprache"], "Interface language (tray, notifications)"),
    (Setting::Shortcuts, "shortcuts", &["shortcut", "keys", "combos", "tray"], "Keyboard shortcuts & tray icon"),
    (Setting::Advanced, "advanced", &["spacing", "typing-speed", "pieces"], "Advanced (spacing, typing speed, pieces)"),
];

pub fn setting_names() -> String {
    SETTINGS.iter().map(|s| s.1).collect::<Vec<_>>().join(", ")
}

fn parse_setting(name: &str) -> Option<Setting> {
    let n = name.trim().to_lowercase();
    SETTINGS
        .iter()
        .find(|(_, main, other, _)| *main == n || other.contains(&n.as_str()))
        .map(|s| s.0)
}

fn current_value(cfg: &Config, s: Setting) -> String {
    let a = cfg.active();
    match s {
        Setting::Service => a.provider.name.to_string(),
        Setting::Model => a.model,
        Setting::Key => {
            if a.api_key.is_empty() { "not set".into() } else { mask(&a.api_key) }
        }
        Setting::Correction => {
            if cfg.correct_text { format!("on ({})", a.correction_model) } else { "off".into() }
        }
        Setting::Phrases => match cfg.phrases().len() {
            0 => "none".into(),
            n => format!("{n}: {}", cfg.phrases().join(", ")),
        },
        Setting::Language => {
            if cfg.language.is_empty() { "automatic".into() } else { cfg.language.clone() }
        }
        Setting::Hotkey => hotkey::pretty(&cfg.hotkey),
        Setting::Mic => mics::describe(&cfg.mic),
        Setting::Mode => format!(
            "{}, max {}",
            if cfg.mode == Mode::Hold { "hold" } else { "toggle" },
            duration_text(cfg.max_seconds)
        ),
        Setting::Sounds => if cfg.sounds { "on".into() } else { "off".into() },
        Setting::Indicator => if cfg.indicator { "on".into() } else { "off".into() },
        Setting::Paste => if cfg.paste { "on".into() } else { "off, typed key by key".into() },
        Setting::Layout => {
            if cfg.layout_is_auto() { format!("automatic (now {})", layout::detect().0) } else { layout::resolve(cfg).to_string() }
        }
        Setting::Autostart => if platform::autostart_enabled() { "on".into() } else { "off".into() },
        Setting::Shortcuts => {
            let n = cfg.shortcuts().len();
            format!("{n} shortcut(s), tray {}", if cfg.tray { "on" } else { "off" })
        }
        Setting::UiLanguage => {
            let name = |l| crate::i18n::LANGS.iter().find(|x| x.0 == l).map(|x| x.2).unwrap_or("English");
            match crate::i18n::from_code(&cfg.ui_language) {
                Some(l) => name(l).to_string(),
                None => format!("automatic ({})", name(crate::i18n::system().unwrap_or(crate::i18n::Lang::En))),
            }
        }
        Setting::Speakers => {
            if cfg.session_speakers { format!("on ({})", cfg.active().speaker_model) } else { "off".into() }
        }
        Setting::Session => format!(
            "{}, notes {}, key {}",
            cfg.session_source,
            cfg.session_notes,
            if cfg.session_hotkey.is_empty() { "none".to_string() } else { hotkey::pretty(&cfg.session_hotkey) }
        ),
        Setting::Advanced => format!(
            "space after text {}, {} ms per key, pieces ~{} s",
            if cfg.add_trailing_space { "on" } else { "off" },
            cfg.key_delay_ms,
            cfg.piece_seconds
        ),
    }
}

fn duration_text(secs: u64) -> String {
    if secs % 60 == 0 { format!("{} min", secs / 60) } else { format!("{secs} s") }
}

fn load_for_setup() -> Config {
    match config::load() {
        Ok(c) => c,
        Err(e) => {
            println!("Your settings have a problem ({e:#}); starting from defaults.");
            Config::default()
        }
    }
}

fn need_terminal() -> Result<()> {
    if !std::io::stdin().is_terminal() {
        bail!("this needs to run in a terminal");
    }
    Ok(())
}

/// Save and say when it takes effect.
fn save_now(cfg: &Config) -> Result<()> {
    config::save(cfg)?;
    if platform::running_pid().is_some() {
        println!("  Saved. vtt switches to it within a few seconds.");
    } else {
        println!("  Saved. (vtt isn't running: `vtt start`)");
    }
    Ok(())
}

/// `vtt setup`: the full walk-through the first time, afterwards a menu to change one
/// thing at a time. `in_foreground`: called by `vtt run`, which continues afterwards.
pub fn run(in_foreground: bool) -> Result<()> {
    need_terminal()?;
    let incomplete = !config::exists() || load_for_setup().problem().is_some();
    if incomplete {
        return full(in_foreground);
    }
    menu()
}

/// `vtt set [setting]`: change one setting, or show the menu.
pub fn set(name: Option<&str>) -> Result<()> {
    let setting = name
        .map(|n| {
            parse_setting(n)
                .ok_or_else(|| anyhow::anyhow!("unknown setting `{n}`. Settings: {}", setting_names()))
        })
        .transpose()?;
    need_terminal()?;
    let Some(setting) = setting else { return menu() };
    let t = ColorfulTheme::default();
    let mut cfg = load_for_setup();
    change(&t, &mut cfg, setting)?;
    save_now(&cfg)
}

/// Menu lines cut to the terminal's width: a line that wraps makes the menu redraw
/// itself below the old one again and again on every key press.
fn fit(labels: &[String]) -> Vec<String> {
    let width = dialoguer::console::Term::stdout().size().1 as usize;
    let max = width.saturating_sub(6).max(20); // "  ❯ " in front, one spare column
    labels
        .iter()
        .map(|l| {
            if dialoguer::console::measure_text_width(l) <= max {
                l.clone()
            } else {
                dialoguer::console::truncate_str(l, max, "…").into_owned()
            }
        })
        .collect()
}

/// How many menu lines fit in the terminal (prompt, page line and a spare line besides).
fn rows(wanted: usize) -> usize {
    let height = dialoguer::console::Term::stdout().size().0 as usize;
    wanted.min(height.saturating_sub(4)).max(3)
}

fn menu() -> Result<()> {
    let t = ColorfulTheme::default();
    let mut cfg = load_for_setup();
    println!("\nvtt settings: pick what to change. Each change is saved right away.");
    loop {
        let mut labels: Vec<String> = SETTINGS
            .iter()
            .map(|(s, _, _, label)| format!("{label:<38} {}", current_value(&cfg, *s)))
            .collect();
        labels.push("Run the full setup again".into());
        labels.push("Done".into());
        let i = Select::with_theme(&t)
            .with_prompt("What do you want to change?")
            .items(fit(&labels))
            .default(labels.len() - 1)
            .max_length(rows(16))
            .interact()?;
        if i == labels.len() - 1 {
            return Ok(());
        }
        if i == labels.len() - 2 {
            return full(false);
        }
        println!();
        change(&t, &mut cfg, SETTINGS[i].0)?;
        save_now(&cfg)?;
        println!();
    }
}

/// Ask about one setting (used by the menu and `vtt set`).
fn change(t: &ColorfulTheme, cfg: &mut Config, s: Setting) -> Result<()> {
    let p = providers::get(&cfg.provider);
    match s {
        Setting::Service => change_service(t, cfg)?,
        Setting::Model => {
            let a = cfg.active();
            print!("  Loading the models {} offers... ", p.name);
            let _ = std::io::stdout().flush();
            let list = stt::list_models(p, &a.api_url, &a.api_key).ok();
            println!("{}", list.as_ref().map_or("not available".into(), |l| format!("{}.", l.len())));
            ask_model(t, cfg, p, list.as_deref())?;
        }
        Setting::Key => {
            ask_key(t, cfg, p, true)?;
        }
        Setting::Correction => ask_correction(t, cfg, p)?,
        Setting::Phrases => ask_phrases(t, cfg)?,
        Setting::Language => ask_language(t, cfg)?,
        Setting::Hotkey => ask_hotkey(t, cfg, true)?,
        Setting::Mic => ask_mic(t, cfg, true)?,
        Setting::Mode => ask_mode(t, cfg)?,
        Setting::Sounds => {
            cfg.sounds = Confirm::with_theme(t)
                .with_prompt("Beep when recording starts and stops?")
                .default(cfg.sounds)
                .interact()?
        }
        Setting::Indicator => {
            cfg.indicator = Confirm::with_theme(t)
                .with_prompt("Show a small sound wave next to the text cursor while listening?")
                .default(cfg.indicator)
                .interact()?;
            #[cfg(target_os = "linux")]
            if cfg.indicator && crate::overlay::is_gnome() {
                println!("  GNOME draws it with a small Wortfluss extension. Log out and back in once to see it.");
            }
        }
        Setting::Paste => {
            cfg.paste = Confirm::with_theme(t)
                .with_prompt("Put the text in by pasting it? (fast; your clipboard is put back) No = type it key by key")
                .default(cfg.paste)
                .interact()?;
            #[cfg(target_os = "linux")]
            if cfg.paste && crate::overlay::is_gnome() {
                println!("  GNOME pastes with the small Wortfluss extension. After an update, log out and back in once.");
            }
        }
        Setting::Layout => ask_layout(t, cfg)?,
        Setting::Autostart => {
            let on = Confirm::with_theme(t)
                .with_prompt("Start vtt automatically when you log in? (recommended)")
                .default(true)
                .interact()?;
            platform::set_autostart(on)?;
        }
        Setting::Advanced => ask_advanced(t, cfg)?,
        Setting::Session => ask_session(t, cfg)?,
        Setting::Speakers => ask_speakers(t, cfg)?,
        Setting::UiLanguage => ask_ui_language(t, cfg)?,
        Setting::Shortcuts => ask_shortcuts(t, cfg)?,
    }
    Ok(())
}

/// Switch service. The key is only asked for if this service has none yet, and the
/// recommended model is chosen automatically (keeping an earlier choice for that service).
fn change_service(t: &ColorfulTheme, cfg: &mut Config) -> Result<()> {
    let p = choose_service(t, cfg)?;
    let a = cfg.active();
    let list = if a.api_key.is_empty() && p.needs_key {
        heading(2, &format!("{} API key", p.name));
        ask_key(t, cfg, p, false)?
    } else {
        stt::list_models(p, &a.api_url, &a.api_key).ok()
    };
    let offered = |id: &str| list.as_ref().is_none_or(|l| l.is_empty() || l.iter().any(|m| m.id == id));
    let saved = cfg.settings(p.id).model.trim().to_string();
    if saved.is_empty() || !offered(&saved) {
        match p.models.iter().find(|m| offered(m.id)) {
            Some(m) => {
                cfg.settings_mut(p.id).model = m.id.to_string();
                println!("  Model: {} (recommended: {}).", m.id, m.note.trim_start_matches("recommended: "));
            }
            None => ask_model(t, cfg, p, list.as_deref())?,
        }
    } else {
        println!("  Model: {saved} (your earlier choice for {}).", p.name);
    }
    if cfg.correct_text {
        println!("  Correction: on with {}.", cfg.active().correction_model);
    }
    println!("  Change the model with `vtt set model`, correction with `vtt set correction`.");
    Ok(())
}

fn choose_service(t: &ColorfulTheme, cfg: &mut Config) -> Result<&'static providers::Provider> {
    let items: Vec<String> = providers::PROVIDERS
        .iter()
        .map(|p| format!("{:<14} {}", p.name, p.blurb))
        .collect();
    let cur = providers::PROVIDERS
        .iter()
        .position(|p| p.id == cfg.provider)
        .unwrap_or(0);
    let p = &providers::PROVIDERS[Select::with_theme(t)
        .with_prompt("Which service should turn your speech into text?")
        .items(&items)
        .default(cur)
        .interact()?];
    cfg.provider = p.id.to_string();
    if p.id == "custom" {
        let current = cfg.custom.api_url.clone();
        cfg.custom.api_url = Input::with_theme(t)
            .with_prompt("Server address (usually ends in /v1)")
            .default(if current.is_empty() { "http://localhost:8000/v1".to_string() } else { current })
            .interact_text()?;
    }
    Ok(p)
}

/// The complete walk-through (first run, or "Run the full setup again").
fn full(in_foreground: bool) -> Result<()> {
    let t = ColorfulTheme::default();
    let first_time = !config::exists();
    let mut cfg = load_for_setup();
    mics::select(&cfg.mic);

    println!("\nvtt setup: Enter keeps the highlighted choice, arrow keys move, Ctrl+C cancels.");
    println!("\nChecking this computer:");
    if !system_checks() {
        println!("\n  You can finish the setup now, but dictation only works after the fix above.");
    }

    heading(1, "Speech recognition service");
    let p = choose_service(&t, &mut cfg)?;

    heading(2, "API key");
    let available = ask_key(&t, &mut cfg, p, false)?;

    heading(3, "Model");
    ask_model(&t, &mut cfg, p, available.as_deref())?;

    heading(4, "Spelling & grammar correction");
    ask_correction(&t, &mut cfg, p)?;

    heading(5, "Language");
    ask_language(&t, &mut cfg)?;

    heading(6, "Phrases: names and words to spell right (optional)");
    ask_phrases(&t, &mut cfg)?;

    heading(7, "Microphone");
    ask_mic(&t, &mut cfg, false)?;

    heading(8, "Dictation key");
    let want_change = first_time && hotkey::parse(&cfg.hotkey).is_none();
    ask_hotkey(&t, &mut cfg, want_change)?;
    ask_mode(&t, &mut cfg)?;
    cfg.sounds = Confirm::with_theme(&t)
        .with_prompt("Beep when recording starts and stops?")
        .default(cfg.sounds)
        .interact()?;

    heading(9, "Keyboard layout for typing");
    ask_layout(&t, &mut cfg)?;

    heading(10, "Start automatically");
    let autostart = Confirm::with_theme(&t)
        .with_prompt("Start vtt automatically when you log in? (recommended)")
        .default(true)
        .interact()?;

    if Confirm::with_theme(&t)
        .with_prompt("Change advanced settings (spacing, typing speed, piece size)?")
        .default(false)
        .interact()?
    {
        ask_advanced(&t, &mut cfg)?;
    }

    println!("\n── Summary ──");
    for s in [Setting::Service, Setting::Model, Setting::Correction, Setting::Phrases, Setting::Language, Setting::Mic, Setting::Hotkey, Setting::Mode, Setting::Layout] {
        let label = SETTINGS.iter().find(|x| x.0 == s).map(|x| x.3).unwrap_or("");
        println!("  {label:<38} {}", current_value(&cfg, s));
    }
    println!("  {:<38} {}", "Start at login", if autostart { "on" } else { "off" });
    if !Confirm::with_theme(&t)
        .with_prompt("Save these settings?")
        .default(true)
        .interact()?
    {
        println!("Nothing was changed.");
        return Ok(());
    }

    let path = config::save(&cfg)?;
    platform::set_autostart(autostart)?;
    println!("Saved to {}", path.display());
    println!(
        "API keys saved encrypted in {}{}",
        secrets::path()?.display(),
        match secrets::protection() {
            secrets::Protection::Keyring => " (master key in your login keyring)".to_string(),
            secrets::Protection::Machine(_) => " (no keyring found: using a weaker machine-bound key)".to_string(),
        }
    );
    println!("Later, change single settings with `vtt setup` (menu) or e.g. `vtt set model`.");

    if in_foreground {
        return Ok(());
    }
    if platform::running_pid().is_some() {
        println!("vtt is running and switches to the new settings within a few seconds.");
    } else if Confirm::with_theme(&t)
        .with_prompt("Start vtt now?")
        .default(true)
        .interact()?
    {
        start_and_confirm(&cfg)?;
    } else {
        println!("Start it later with `vtt start`.");
    }
    Ok(())
}

/// Pick the microphone and try it. `always_ask`: also when there is only one.
fn ask_mic(t: &ColorfulTheme, cfg: &mut Config, always_ask: bool) -> Result<()> {
    let list = match mics::list() {
        Ok(l) => l,
        Err(e) => {
            println!("  Can't list microphones ({e:#}); the system default is used.");
            return Ok(());
        }
    };
    let default_desc = list.iter().find(|m| m.is_default).map(|m| m.description.clone()).unwrap_or_default();
    if list.len() <= 1 && !always_ask {
        println!("  Using: {}", if default_desc.is_empty() { "system default".to_string() } else { default_desc });
        cfg.mic.clear();
        return Ok(());
    }
    loop {
        let mut labels = vec![format!(
            "System default (recommended): follows Settings > Sound{}",
            if default_desc.is_empty() { String::new() } else { format!(", now {default_desc}") }
        )];
        labels.extend(list.iter().map(|m| {
            format!("{}{}", m.description, if m.muted { "  [muted]" } else { "" })
        }));
        let current = list.iter().position(|m| m.name == cfg.mic).map_or(0, |i| i + 1);
        let i = Select::with_theme(t)
            .with_prompt("Which microphone should vtt use?")
            .items(&labels)
            .default(current)
            .interact()?;
        let chosen = if i == 0 { String::new() } else { list[i - 1].name.clone() };
        if i > 0 && list[i - 1].muted {
            println!("  This microphone is muted in the system sound settings.");
        }

        // Try it before keeping it.
        mics::select(&chosen);
        println!("  Say something for 3 seconds...");
        let result = audio::Recorder::start().map(|r| {
            std::thread::sleep(Duration::from_secs(3));
            r.stop()
        });
        match result {
            Ok(clip) => {
                let level = (clip.peak() * 100.0).round() as usize;
                let bar: String = "█".repeat((level / 5).min(20)) + &"░".repeat(20 - (level / 5).min(20));
                println!(
                    "  Level {bar} {level}%  {}",
                    if clip.has_speech() {
                        "speech heard".to_string()
                    } else if clip.peak() < 0.01 {
                        "silent: muted or wrong microphone?".to_string()
                    } else {
                        "no clear speech".to_string()
                    }
                );
            }
            Err(e) => println!("  Recording failed: {e:#}"),
        }
        match Select::with_theme(t)
            .items(&["Use this microphone", "Choose another one / test again"])
            .default(0)
            .interact()?
        {
            0 => {
                cfg.mic = chosen;
                return Ok(());
            }
            _ => continue,
        }
    }
}

fn ask_mode(t: &ColorfulTheme, cfg: &mut Config) -> Result<()> {
    let modes = [
        "Hold to talk: hold the key while you speak, let go to type (best for short texts)",
        "Toggle: press once to start, press again to stop (best for long dictations)",
    ];
    cfg.mode = match Select::with_theme(t)
        .with_prompt("How should the key work?")
        .items(&modes)
        .default(if cfg.mode == Mode::Toggle { 1 } else { 0 })
        .interact()?
    {
        1 => Mode::Toggle,
        _ => Mode::Hold,
    };
    println!("  Long dictations are sent in pieces of ~{} s while you speak, so the text", cfg.piece_seconds);
    println!("  appears as you go. Pauses and silence are never sent; toggle mode stops by");
    println!("  itself after 60 s without speech.");
    let options: [(u64, &str); 7] = [
        (60, "1 minute"),
        (120, "2 minutes (default)"),
        (300, "5 minutes"),
        (600, "10 minutes"),
        (1800, "30 minutes (good for toggle mode)"),
        (3600, "1 hour"),
        (0, "Other"),
    ];
    let labels: Vec<&str> = options.iter().map(|o| o.1).collect();
    let default = options
        .iter()
        .position(|o| o.0 == cfg.max_seconds)
        .unwrap_or(if cfg.mode == Mode::Toggle { 4 } else { 1 });
    let i = Select::with_theme(t)
        .with_prompt("Longest recording before vtt stops it automatically")
        .items(&labels)
        .default(default)
        .interact()?;
    cfg.max_seconds = if options[i].0 > 0 {
        options[i].0
    } else {
        Input::with_theme(t)
            .with_prompt("Seconds (10 to 7200)")
            .default(cfg.max_seconds)
            .validate_with(|v: &u64| if (10..=7200).contains(v) { Ok(()) } else { Err("between 10 and 7200") })
            .interact_text()?
    };
    Ok(())
}

/// Defaults for `vtt session start` and the session key.
/// Speaker separation for sessions, and the model that does it.
fn ask_ui_language(t: &ColorfulTheme, cfg: &mut Config) -> Result<()> {
    let mut items = vec!["Automatic (the system's language)".to_string()];
    items.extend(crate::i18n::LANGS.iter().map(|l| l.2.to_string()));
    let current = crate::i18n::LANGS.iter().position(|l| Some(l.0) == crate::i18n::from_code(&cfg.ui_language));
    let i = Select::with_theme(t)
        .with_prompt("Language of the tray menu, notifications and rewrite window")
        .items(&items)
        .default(current.map_or(0, |c| c + 1))
        .interact()?;
    cfg.ui_language = if i == 0 { "auto".into() } else { crate::i18n::LANGS[i - 1].1.into() };
    Ok(())
}

fn ask_speakers(t: &ColorfulTheme, cfg: &mut Config) -> Result<()> {
    let p = providers::get(&cfg.provider);
    let known = stt::speaker_models(p.id);
    if known.is_empty() && cfg.settings(p.id).speaker_model.trim().is_empty() {
        println!("  {} can't tell speakers apart. To get \"Speaker 1: ... / Speaker 2: ...\" in sessions,", p.name);
        println!("  switch to OpenRouter, OpenAI or Mistral (`vtt set service`).");
        cfg.session_speakers = false;
        return Ok(());
    }
    cfg.session_speakers = Confirm::with_theme(t)
        .with_prompt(
            "Separate speakers in sessions? (\"Speaker 1: ... Speaker 2: ...\", rename them afterwards with \
             `vtt speakers FILE`; a bit more expensive)",
        )
        .default(cfg.session_speakers)
        .interact()?;
    if !cfg.session_speakers {
        return Ok(());
    }
    let current = cfg.active().speaker_model;
    let mut items: Vec<String> = known.iter().map(|(id, note)| format!("{id:<40} {note}")).collect();
    items.push("Other model (enter its id)".into());
    let i = Select::with_theme(t)
        .with_prompt("Which model separates the speakers?")
        .items(&items)
        .default(known.iter().position(|m| m.0 == current).unwrap_or(0))
        .interact()?;
    let model = if i == known.len() {
        Input::<String>::with_theme(t).with_prompt("Model id").default(current).interact_text()?
    } else {
        known[i].0.to_string()
    };
    // Keep the setting empty for the recommended model, so a better default later applies.
    cfg.settings_mut(p.id).speaker_model =
        if Some(model.trim()) == stt::speaker_model(p.id) { String::new() } else { model.trim().to_string() };
    Ok(())
}

fn ask_session(t: &ColorfulTheme, cfg: &mut Config) -> Result<()> {
    println!("  Session mode writes a transcript of a lecture, video or meeting to a file and");
    println!("  notes at the end. Start: `vtt session start [title]` or the session key.");
    println!("  Only record other people with their consent.");
    let sources = [
        ("system", "System audio: what the computer plays (online lectures, videos, tutorials)"),
        ("mic", "Microphone: a lecture or talk in the room"),
        ("both", "Both: calls and online meetings, labelled You / Others (use headphones)"),
    ];
    let labels: Vec<&str> = sources.iter().map(|s| s.1).collect();
    let i = Select::with_theme(t)
        .with_prompt("What should a session record by default?")
        .items(&labels)
        .default(sources.iter().position(|s| s.0 == cfg.session_source).unwrap_or(0))
        .interact()?;
    cfg.session_source = sources[i].0.to_string();

    let kinds = [
        ("auto", "Automatic: meeting minutes for calls, lecture notes otherwise (recommended)"),
        ("lecture", "Lecture notes: summary, key points, terms, open questions"),
        ("meeting", "Meeting minutes: decisions, action items, open questions"),
        ("summary", "Short summary"),
        ("none", "No notes (transcript only)"),
    ];
    let labels: Vec<&str> = kinds.iter().map(|k| k.1).collect();
    let i = Select::with_theme(t)
        .with_prompt("Which notes at the end?")
        .items(&labels)
        .default(kinds.iter().position(|k| k.0 == cfg.session_notes).unwrap_or(0))
        .interact()?;
    cfg.session_notes = kinds[i].0.to_string();
    if cfg.session_notes != "none" && cfg.active().correction_model.is_empty() {
        println!("  Notes need a text model: choose one with `vtt set correction` (it can stay switched off).");
    }

    let current = crate::session::session_dir(cfg).map(|d| d.display().to_string()).unwrap_or_default();
    let dir: String = Input::with_theme(t)
        .with_prompt("Folder for transcripts")
        .default(current.clone())
        .interact_text()?;
    cfg.session_dir = if dir.trim() == current { cfg.session_dir.clone() } else { dir.trim().to_string() };

    ask_speakers(t, cfg)?;

    cfg.session_keep_audio = Confirm::with_theme(t)
        .with_prompt(format!(
            "Keep the session audio, e.g. to redo a transcript? (asked to delete after each session, \
             deleted after {} days)",
            cfg.session_audio_days
        ))
        .default(cfg.session_keep_audio)
        .interact()?;

    let key_now = if cfg.session_hotkey.is_empty() { "none".to_string() } else { hotkey::pretty(&cfg.session_hotkey) };
    let choice = Select::with_theme(t)
        .with_prompt(format!("Session key (press to start, press again to stop). Now: {key_now}"))
        .items(&["Keep", "Choose a key by pressing it", "No session key (use the command)"])
        .default(0)
        .interact()?;
    match choice {
        1 => loop {
            println!("  Press the key you want to use (e.g. F10, Pause, Scroll Lock)...");
            let key = hotkey::capture()?;
            let name = format!("{key:?}");
            if hotkey::parse(&cfg.hotkey) == Some(key) {
                println!("  That's already the dictation key. Choose another one.");
                continue;
            }
            println!("  You pressed: {}", hotkey::pretty(&name));
            if let Some(w) = hotkey::warning(key) {
                println!("  Careful: {w}.");
            }
            if Confirm::with_theme(t).with_prompt("Use this key?").default(true).interact()? {
                cfg.session_hotkey = name;
                break;
            }
        },
        2 => cfg.session_hotkey.clear(),
        _ => {}
    }
    Ok(())
}

/// Keyboard shortcuts (pressed, not typed) and the tray icon.
fn ask_shortcuts(t: &ColorfulTheme, cfg: &mut Config) -> Result<()> {
    println!("  Shortcuts work everywhere while vtt runs. The focused app also receives them,");
    println!("  so use combinations your apps don't need (Ctrl+Alt+letter usually works).");
    loop {
        let entries: [(&str, String); 6] = [
            ("Start/stop a session", cfg.shortcut_session.clone()),
            ("Start/stop a call session (You / Others)", cfg.shortcut_call.clone()),
            ("Spelling correction on/off", cfg.shortcut_correction.clone()),
            ("Hold <-> toggle", cfg.shortcut_mode.clone()),
            ("Open the settings", cfg.shortcut_settings.clone()),
            ("Rewrite the selected text (tone, correction)", cfg.shortcut_rewrite.clone()),
        ];
        let mut labels: Vec<String> = entries
            .iter()
            .map(|(what, combo)| {
                let shown = hotkey::Combo::parse(combo).map(|c| c.text()).unwrap_or_else(|| "off".into());
                format!("{what:<44} {shown}")
            })
            .collect();
        labels.push(format!("Tray icon: {}", if cfg.tray { "on" } else { "off" }));
        labels.push("Done".into());
        let i = Select::with_theme(t)
            .with_prompt("Which one?")
            .items(&labels)
            .default(labels.len() - 1)
            .interact()?;
        if i == labels.len() - 1 {
            return Ok(());
        }
        if i == labels.len() - 2 {
            cfg.tray = !cfg.tray;
            continue;
        }
        let choice = Select::with_theme(t)
            .items(&["Press a new combination", "Switch this shortcut off", "Back"])
            .default(0)
            .interact()?;
        let value = match choice {
            0 => {
                println!("  Press the combination now (e.g. Ctrl+Alt+S)...");
                let c = hotkey::capture_combo()?;
                if !(c.ctrl || c.alt || c.meta) {
                    println!("  {} has no Ctrl, Alt or Super, so it would fire while typing. Not used.", c.text());
                    continue;
                }
                println!("  Set to {}.", c.text());
                c.text()
            }
            1 => String::new(),
            _ => continue,
        };
        match i {
            0 => cfg.shortcut_session = value,
            1 => cfg.shortcut_call = value,
            2 => cfg.shortcut_correction = value,
            3 => cfg.shortcut_mode = value,
            4 => cfg.shortcut_settings = value,
            _ => cfg.shortcut_rewrite = value,
        }
    }
}

fn ask_advanced(t: &ColorfulTheme, cfg: &mut Config) -> Result<()> {
    cfg.add_trailing_space = Confirm::with_theme(t)
        .with_prompt("Add a space after each dictation?")
        .default(cfg.add_trailing_space)
        .interact()?;
    cfg.key_delay_ms = Input::with_theme(t)
        .with_prompt("Milliseconds between typed keys (raise if an app drops letters)")
        .default(cfg.key_delay_ms)
        .interact_text()?;
    cfg.piece_seconds = Input::with_theme(t)
        .with_prompt("Send long dictations in pieces of about how many seconds? (10-120)")
        .default(cfg.piece_seconds)
        .validate_with(|v: &u64| if (10..=120).contains(v) { Ok(()) } else { Err("between 10 and 120") })
        .interact_text()?;
    Ok(())
}

fn phrases_note(cfg: &Config) {
    let p = providers::get(&cfg.provider);
    if !providers::uses_prompt(p.id) {
        if cfg.correct_text {
            println!("  ({} ignores these while transcribing; spelling correction applies them.)", p.name);
        } else {
            println!(
                "  Note: {} ignores these while transcribing. Turn on spelling correction\n  (`vtt set correction`) so they are applied.",
                p.name
            );
        }
    }
}

fn show_phrases(cfg: &Config) {
    let list = cfg.phrases();
    if list.is_empty() {
        println!("  No phrases yet.");
    } else {
        println!("  Phrases ({}):", list.len());
        for ph in &list {
            println!("    - {ph}");
        }
    }
}

fn ask_phrases(t: &ColorfulTheme, cfg: &mut Config) -> Result<()> {
    println!("  Names, brands and words vtt should always spell exactly like this,");
    println!("  e.g. your name, your company, product names, greetings.");
    phrases_note(cfg);
    loop {
        show_phrases(cfg);
        let has = !cfg.phrases().is_empty();
        let mut items = vec!["Add phrases"];
        if has {
            items.push("Remove phrases");
            items.push("Remove all");
        }
        items.push("Done");
        let choice = items[Select::with_theme(t)
            .items(&items)
            .default(if has { items.len() - 1 } else { 0 })
            .interact()?];
        match choice {
            "Add phrases" => {
                let input: String = Input::with_theme(t)
                    .with_prompt("Phrases to add, separated by commas")
                    .allow_empty(true)
                    .interact_text()?;
                let mut list = cfg.phrases();
                list.extend(config::split_phrases(&input));
                cfg.set_phrases(list);
            }
            "Remove phrases" => {
                let list = cfg.phrases();
                let picked = MultiSelect::with_theme(t)
                    .with_prompt("Space marks a phrase, Enter removes the marked ones")
                    .items(&list)
                    .interact()?;
                cfg.set_phrases(
                    list.into_iter()
                        .enumerate()
                        .filter(|(i, _)| !picked.contains(i))
                        .map(|(_, p)| p)
                        .collect(),
                );
            }
            "Remove all" => {
                if Confirm::with_theme(t).with_prompt("Remove all phrases?").default(false).interact()? {
                    cfg.set_phrases(Vec::new());
                }
            }
            _ => return Ok(()),
        }
    }
}

/// `vtt phrases`, `vtt phrases add A, B`, `vtt phrases remove A`, `vtt phrases clear`.
pub fn phrases_cmd(args: &[String]) -> Result<()> {
    let mut cfg = config::load()?;
    let rest = args.iter().skip(1).cloned().collect::<Vec<_>>().join(",");
    match args.first().map(String::as_str) {
        None | Some("list") => {
            show_phrases(&cfg);
            phrases_note(&cfg);
            println!("  Change: vtt phrases add \"Name, Other Name\" | vtt phrases remove \"Name\" | vtt phrases clear");
            return Ok(());
        }
        Some("add") => {
            let mut list = cfg.phrases();
            let new = config::split_phrases(&rest);
            if new.is_empty() {
                bail!("nothing to add. Example: vtt phrases add \"Ammad, Salamalaikum\"");
            }
            list.extend(new);
            cfg.set_phrases(list);
        }
        Some("remove" | "rm" | "delete") => {
            let gone: Vec<String> = config::split_phrases(&rest).iter().map(|p| p.to_lowercase()).collect();
            let before = cfg.phrases().len();
            let list: Vec<String> = cfg.phrases().into_iter().filter(|p| !gone.contains(&p.to_lowercase())).collect();
            if list.len() == before {
                bail!("no such phrase. Current: {}", cfg.phrases().join(", "));
            }
            cfg.set_phrases(list);
        }
        Some("clear") => cfg.set_phrases(Vec::new()),
        Some(other) => bail!("unknown `vtt phrases {other}`. Use: list, add, remove, clear"),
    }
    save_now(&cfg)?;
    show_phrases(&cfg);
    phrases_note(&cfg);
    Ok(())
}

/// Start the background copy and tell the user whether it came up.
pub fn start_and_confirm(cfg: &Config) -> Result<()> {
    platform::start_background()?;
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(100));
        if platform::running_pid().is_some() {
            let how = if cfg.mode == Mode::Hold { "Hold" } else { "Press" };
            println!(
                "vtt is running. {how} {} and speak. (`vtt stop` stops it)",
                hotkey::pretty(&cfg.hotkey)
            );
            return Ok(());
        }
    }
    bail!(
        "vtt did not start; see {}",
        platform::log_file()?.display()
    )
}

/// Ask for (or confirm) the key and test it. Returns the speech models the service
/// offers, or None if they couldn't be fetched.
fn ask_key(
    t: &ColorfulTheme,
    cfg: &mut Config,
    p: &providers::Provider,
    prefer_new: bool,
) -> Result<Option<Vec<ModelEntry>>> {
    let mut prefer_new = prefer_new;
    loop {
        let saved = cfg.settings(p.id).api_key.clone();
        let mut key = saved.clone();
        if !saved.is_empty()
            && !Confirm::with_theme(t)
                .with_prompt(format!("Keep the saved key {}?", mask(&saved)))
                .default(!std::mem::take(&mut prefer_new))
                .interact()?
        {
            key.clear();
        }
        if key.is_empty() {
            if !p.key_url.is_empty() {
                println!("  Create a key here: {}", p.key_url);
            }
            key = Password::with_theme(t)
                .with_prompt(if p.needs_key {
                    "Paste your API key (it stays hidden)"
                } else {
                    "API key (leave empty if the server needs none)"
                })
                .allow_empty_password(!p.needs_key)
                .interact()?
                .trim()
                .to_string();
        }
        cfg.settings_mut(p.id).api_key = key.clone();
        let url = cfg.active().api_url;

        print!("  Checking with {}... ", p.name);
        let _ = std::io::stdout().flush();
        match stt::check_key(p, &url, &key) {
            Ok(()) => {
                let models = stt::list_models(p, &url, &key).ok();
                match &models {
                    Some(m) if !m.is_empty() => println!("works. {} speech models available.", m.len()),
                    _ => println!("works."),
                }
                return Ok(models);
            }
            Err(e) => {
                println!("failed: {e:#}");
                let choice = Select::with_theme(t)
                    .items(&["Enter the key again", "Keep it anyway (e.g. no internet right now)"])
                    .default(0)
                    .interact()?;
                if choice == 1 {
                    // Some services (OpenRouter) list their models without a valid key.
                    return Ok(stt::list_models(p, &url, &key).ok());
                }
                cfg.settings_mut(p.id).api_key.clear();
            }
        }
    }
}

fn price_text(usd_per_hour: Option<f64>, token_priced: bool) -> String {
    match usd_per_hour {
        Some(x) if x < 0.1 => format!("~${x:.3}/hour"),
        Some(x) => format!("~${x:.2}/hour"),
        None if token_priced => "billed per token".into(),
        None => "price unknown".into(),
    }
}

/// Recommended model by default; or pick from everything the service offers; or type one.
fn ask_model(
    t: &ColorfulTheme,
    cfg: &mut Config,
    p: &providers::Provider,
    available: Option<&[ModelEntry]>,
) -> Result<()> {
    let current = cfg.settings(p.id).model.trim().to_string();
    let offered = |id: &str| available.is_none_or(|a| a.is_empty() || a.iter().any(|m| m.id == id));
    let live = |id: &str| available.and_then(|a| a.iter().find(|m| m.id == id));

    // Full list: our curated models first (with advice), then the rest alphabetically.
    let mut entries: Vec<(String, String)> = Vec::new();
    for m in p.models.iter().filter(|m| offered(m.id)) {
        let price = live(m.id).and_then(|e| e.usd_per_hour).unwrap_or(m.usd_per_hour);
        entries.push((m.id.to_string(), format!("{:<36} {} · {}", m.id, m.note, price_text(Some(price), false))));
    }
    for e in available.unwrap_or_default() {
        if entries.iter().any(|(id, _)| *id == e.id) {
            continue;
        }
        let name = if e.name.is_empty() { String::new() } else { format!("{} · ", e.name) };
        entries.push((e.id.clone(), format!("{:<36} {name}{}", e.id, price_text(e.usd_per_hour, e.token_priced))));
    }

    enum Pick {
        Keep,
        Recommended(&'static str),
        List,
        Type,
    }
    let mut picks: Vec<(Pick, String)> = Vec::new();
    let recommended = p.models.iter().find(|m| offered(m.id));
    if !current.is_empty() && Some(current.as_str()) != recommended.map(|r| r.id) {
        picks.push((Pick::Keep, format!("Keep current: {current}")));
    }
    if let Some(r) = recommended {
        picks.push((
            Pick::Recommended(r.id),
            format!("Recommended: {} ({}, {})", r.id, r.note.trim_start_matches("recommended: "), price_text(Some(r.usd_per_hour), false)),
        ));
    }
    if !entries.is_empty() {
        picks.push((Pick::List, format!("Choose from all {} speech models {} offers", entries.len(), p.name)));
    }
    picks.push((Pick::Type, "Type a model name".into()));

    let labels: Vec<&str> = picks.iter().map(|(_, l)| l.as_str()).collect();
    let i = Select::with_theme(t)
        .with_prompt("Which model?")
        .items(&labels)
        .default(0)
        .interact()?;
    let model = match picks[i].0 {
        Pick::Keep => current,
        Pick::Recommended(id) => id.to_string(),
        Pick::List => {
            let labels: Vec<String> = entries.iter().map(|(_, l)| l.clone()).collect();
            let j = FuzzySelect::with_theme(t)
                .with_prompt("Type to filter, arrows to move, Enter to choose")
                .items(fit(&labels))
                .default(entries.iter().position(|(id, _)| *id == current).unwrap_or(0))
                .max_length(rows(15))
                .interact()?;
            entries[j].0.clone()
        }
        Pick::Type => Input::with_theme(t)
            .with_prompt("Model name")
            .default(current)
            .interact_text()?,
    };
    cfg.settings_mut(p.id).model = model.trim().to_string();
    Ok(())
}

/// Off by default; or the recommended text model; or pick from the service's list.
fn ask_correction(t: &ColorfulTheme, cfg: &mut Config, p: &providers::Provider) -> Result<()> {
    println!("  Fixes spelling, capital letters, commas and grammar before typing,");
    println!("  without changing your words. Adds ~0.3-1 s and a fraction of a cent per dictation.");
    let recs = providers::chat_models(p.id);
    let current = cfg.settings(p.id).correction_model.trim().to_string();
    let rec = recs.first();

    enum Pick {
        Off,
        Keep,
        Recommended(&'static str),
        List,
        Type,
    }
    let mut picks: Vec<(Pick, String)> = vec![(Pick::Off, "Off: type exactly what was heard (fastest)".into())];
    if cfg.correct_text && !current.is_empty() && Some(current.as_str()) != rec.map(|r| r.id) {
        picks.push((Pick::Keep, format!("On, keep current model: {current}")));
    }
    if let Some(r) = rec {
        picks.push((
            Pick::Recommended(r.id),
            format!("On, recommended: {} ({})", r.id, r.note.trim_start_matches("recommended: ")),
        ));
    }
    picks.push((Pick::List, format!("On, choose from the text models {} offers", p.name)));
    picks.push((Pick::Type, "On, type a model name".into()));
    let default = if cfg.correct_text { 1.min(picks.len() - 1) } else { 0 };

    let labels: Vec<&str> = picks.iter().map(|(_, l)| l.as_str()).collect();
    let i = Select::with_theme(t)
        .with_prompt("Correct spelling and grammar?")
        .items(&labels)
        .default(default)
        .interact()?;
    let model = match picks[i].0 {
        Pick::Off => {
            cfg.correct_text = false;
            return Ok(());
        }
        Pick::Keep => current,
        Pick::Recommended(id) => id.to_string(),
        Pick::Type => Input::with_theme(t)
            .with_prompt("Model name")
            .default(current)
            .interact_text()?,
        Pick::List => {
            let a = cfg.active();
            print!("  Loading the list... ");
            let _ = std::io::stdout().flush();
            let live = stt::list_chat_models(p, &a.api_url, &a.api_key).unwrap_or_default();
            println!("{} models.", live.len());
            let mut entries: Vec<(String, String)> = recs
                .iter()
                .filter(|m| live.is_empty() || live.iter().any(|e| e.id == m.id))
                .map(|m| (m.id.to_string(), format!("{:<34} {}", m.id, m.note)))
                .collect();
            for e in &live {
                if !entries.iter().any(|(id, _)| *id == e.id) {
                    entries.push((e.id.clone(), format!("{:<34} {}", e.id, e.name)));
                }
            }
            if entries.is_empty() {
                Input::with_theme(t).with_prompt("Model name").default(current).interact_text()?
            } else {
                let labels: Vec<String> = entries.iter().map(|(_, l)| l.clone()).collect();
                let j = FuzzySelect::with_theme(t)
                    .with_prompt("Type to filter, arrows to move, Enter to choose")
                    .items(fit(&labels))
                    .default(entries.iter().position(|(id, _)| *id == current).unwrap_or(0))
                    .max_length(rows(15))
                    .interact()?;
                entries[j].0.clone()
            }
        }
    };
    cfg.settings_mut(p.id).correction_model = model.trim().to_string();
    cfg.correct_text = true;

    // Try it, so a wrong model or missing access shows up now and not while dictating.
    let sample = "das ist ein kurzer test wie gehts dir ich hoffe gut";
    let a = cfg.active();
    print!("  Testing with \"{sample}\"... ");
    let _ = std::io::stdout().flush();
    let t0 = std::time::Instant::now();
    match correct::correct(&a, &a.correction_model, sample, &[]) {
        Ok(c) => println!("{:.1}s\n  -> {}", t0.elapsed().as_secs_f32(), c.text),
        Err(e) => {
            println!("failed: {e:#}");
            if Select::with_theme(t)
                .items(&["Turn correction off", "Keep it on anyway"])
                .default(0)
                .interact()?
                == 0
            {
                cfg.correct_text = false;
            }
        }
    }
    Ok(())
}

fn ask_language(t: &ColorfulTheme, cfg: &mut Config) -> Result<()> {
    let mut labels: Vec<String> = LANGUAGES
        .iter()
        .map(|(code, name)| {
            if code.is_empty() { name.to_string() } else { format!("{name} ({code})") }
        })
        .collect();
    labels.push("Other (type a 2-letter language code)".into());
    let current = cfg.language.trim().to_string();
    let default = LANGUAGES
        .iter()
        .position(|(c, _)| *c == current)
        .unwrap_or(LANGUAGES.len());
    let i = Select::with_theme(t)
        .with_prompt("Which language do you speak? (a fixed language is a bit faster and more reliable)")
        .items(&labels)
        .default(default)
        .interact()?;
    cfg.language = if i < LANGUAGES.len() {
        LANGUAGES[i].0.to_string()
    } else {
        Input::<String>::with_theme(t)
            .with_prompt("Language code, e.g. pl, pt, ru, ja, zh")
            .default(current)
            .interact_text()?
            .trim()
            .to_lowercase()
    };
    Ok(())
}

fn ask_hotkey(t: &ColorfulTheme, cfg: &mut Config, want_change: bool) -> Result<()> {
    println!("  Current key: {}", hotkey::pretty(&cfg.hotkey));
    println!("  Good choices: Right Ctrl, Menu, Pause, Scroll Lock, Insert, F9-F12.");
    if !Confirm::with_theme(t)
        .with_prompt("Choose a different key?")
        .default(want_change)
        .interact()?
    {
        return Ok(());
    }
    loop {
        println!("  Press the key you want to use now...");
        let key = match hotkey::capture() {
            Ok(k) => k,
            Err(e) => {
                println!("  Can't read the keyboard: {e:#}");
                return Ok(());
            }
        };
        let name = format!("{key:?}");
        println!("  You pressed: {}", hotkey::pretty(&name));
        if let Some(w) = hotkey::warning(key) {
            println!("  Careful: {w}.");
        }
        let keep_label = format!("Keep {}", hotkey::pretty(&cfg.hotkey));
        match Select::with_theme(t)
            .items(&["Use this key", "Press a different key", keep_label.as_str()])
            .default(0)
            .interact()?
        {
            0 => {
                cfg.hotkey = name;
                return Ok(());
            }
            1 => continue,
            _ => return Ok(()),
        }
    }
}

fn ask_layout(t: &ColorfulTheme, cfg: &mut Config) -> Result<()> {
    if cfg!(windows) {
        println!("  Nothing to set on Windows: text is typed as characters, so every layout works.");
        return Ok(());
    }
    let (detected, source) = layout::detect();
    let items = [
        format!("Automatic (recommended): now {detected}, {source}"),
        "Always use one fixed layout".to_string(),
    ];
    let fixed = Select::with_theme(t)
        .with_prompt("Which keyboard layout should vtt type with?")
        .items(&items)
        .default(if cfg.layout_is_auto() { 0 } else { 1 })
        .interact()?
        == 1;
    if !fixed {
        cfg.keyboard_layout = "auto".into();
        cfg.keyboard_variant.clear();
        return Ok(());
    }
    loop {
        let start = if cfg.layout_is_auto() { detected.clone() } else { layout::resolve(cfg) };
        let l: String = Input::with_theme(t)
            .with_prompt("Layout code (e.g. us, gb, de, tr, fr)")
            .default(start.layout.clone())
            .interact_text()?;
        let v: String = Input::with_theme(t)
            .with_prompt("Variant (often empty; e.g. nodeadkeys)")
            .default(start.variant.clone())
            .allow_empty(true)
            .interact_text()?;
        match typer::build_map(l.trim(), v.trim()) {
            Ok(_) => {
                cfg.keyboard_layout = l.trim().to_string();
                cfg.keyboard_variant = v.trim().to_string();
                return Ok(());
            }
            Err(e) => println!("  {e:#}. Try again."),
        }
    }
}
