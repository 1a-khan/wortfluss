//! `vtt setup`: step-by-step settings in the terminal, and `vtt check`: the self-test.
//! Setup can be run again at any time; current values are preselected, so Enter keeps them.

use crate::config::{self, Config, Mode};
use crate::{audio, correct, hotkey, layout, platform, providers, secrets, stt, typer};
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
    Mode,
    Sounds,
    Layout,
    Autostart,
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
    (Setting::Mode, "mode", &["long", "toggle", "hold", "max-seconds"], "Hold/toggle & long dictations"),
    (Setting::Sounds, "sounds", &["beep", "beeps"], "Beeps"),
    (Setting::Layout, "layout", &["keyboard"], "Keyboard layout"),
    (Setting::Autostart, "autostart", &["login", "startup"], "Start at login"),
    (Setting::Advanced, "advanced", &["spacing", "typing-speed"], "Advanced (spacing, typing speed)"),
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
        Setting::Mode => format!(
            "{}, max {}",
            if cfg.mode == Mode::Hold { "hold" } else { "toggle" },
            duration_text(cfg.max_seconds)
        ),
        Setting::Sounds => if cfg.sounds { "on".into() } else { "off".into() },
        Setting::Layout => {
            if cfg.layout_is_auto() { format!("automatic (now {})", layout::detect().0) } else { layout::resolve(cfg).to_string() }
        }
        Setting::Autostart => if platform::autostart_enabled() { "on".into() } else { "off".into() },
        Setting::Advanced => format!(
            "space after text {}, {} ms per key",
            if cfg.add_trailing_space { "on" } else { "off" },
            cfg.key_delay_ms
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
            .items(&labels)
            .default(labels.len() - 1)
            .max_length(16)
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
        Setting::Mode => ask_mode(t, cfg)?,
        Setting::Sounds => {
            cfg.sounds = Confirm::with_theme(t)
                .with_prompt("Beep when recording starts and stops?")
                .default(cfg.sounds)
                .interact()?
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

    heading(7, "Dictation key");
    let want_change = first_time && hotkey::parse(&cfg.hotkey).is_none();
    ask_hotkey(&t, &mut cfg, want_change)?;
    ask_mode(&t, &mut cfg)?;
    cfg.sounds = Confirm::with_theme(&t)
        .with_prompt("Beep when recording starts and stops?")
        .default(cfg.sounds)
        .interact()?;

    heading(8, "Keyboard layout for typing");
    ask_layout(&t, &mut cfg)?;

    heading(9, "Start automatically");
    let autostart = Confirm::with_theme(&t)
        .with_prompt("Start vtt automatically when you log in? (recommended)")
        .default(true)
        .interact()?;

    if Confirm::with_theme(&t)
        .with_prompt("Change advanced settings (spacing, typing speed)?")
        .default(false)
        .interact()?
    {
        ask_advanced(&t, &mut cfg)?;
    }

    println!("\n── Summary ──");
    for s in [Setting::Service, Setting::Model, Setting::Correction, Setting::Phrases, Setting::Language, Setting::Hotkey, Setting::Mode, Setting::Layout] {
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
    println!("  The whole recording is transcribed after you stop, then typed in one go.");
    println!("  Very long recordings take a few seconds longer; the limit is about 13 minutes.");
    let options: [(u64, &str); 5] = [
        (60, "1 minute"),
        (120, "2 minutes (default)"),
        (300, "5 minutes (good for toggle mode)"),
        (600, "10 minutes"),
        (0, "Other"),
    ];
    let labels: Vec<&str> = options.iter().map(|o| o.1).collect();
    let default = options
        .iter()
        .position(|o| o.0 == cfg.max_seconds)
        .unwrap_or(if cfg.mode == Mode::Toggle { 2 } else { 1 });
    let i = Select::with_theme(t)
        .with_prompt("Longest recording before vtt stops it automatically")
        .items(&labels)
        .default(default)
        .interact()?;
    cfg.max_seconds = if options[i].0 > 0 {
        options[i].0
    } else {
        Input::with_theme(t)
            .with_prompt("Seconds (10 to 780)")
            .default(cfg.max_seconds)
            .validate_with(|v: &u64| if (10..=780).contains(v) { Ok(()) } else { Err("between 10 and 780") })
            .interact_text()?
    };
    Ok(())
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
            let labels: Vec<&str> = entries.iter().map(|(_, l)| l.as_str()).collect();
            let j = FuzzySelect::with_theme(t)
                .with_prompt("Type to filter, arrows to move, Enter to choose")
                .items(&labels)
                .default(entries.iter().position(|(id, _)| *id == current).unwrap_or(0))
                .max_length(15)
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
                let labels: Vec<&str> = entries.iter().map(|(_, l)| l.as_str()).collect();
                let j = FuzzySelect::with_theme(t)
                    .with_prompt("Type to filter, arrows to move, Enter to choose")
                    .items(&labels)
                    .default(entries.iter().position(|(id, _)| *id == current).unwrap_or(0))
                    .max_length(15)
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
