mod audio;
mod config;
mod correct;
mod help;
mod hotkey;
mod layout;
mod platform;
mod providers;
mod secrets;
mod setup;
mod stt;
mod typer;
mod usage;

use anyhow::{anyhow, bail, Result};
use config::{Config, Mode};
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;


fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("run") => daemon(),
        Some("setup" | "init" | "config" | "settings") => setup::run(false),
        Some("set" | "change") => setup::set(args.get(1).map(String::as_str)),
        Some("phrases" | "phrase" | "words") => setup::phrases_cmd(&args[1..]),
        Some("check" | "doctor") => setup::check(),
        Some("start") => start(),
        Some("stop") => {
            if platform::stop_running()? {
                println!("vtt stopped.");
            } else {
                println!("vtt wasn't running.");
            }
            Ok(())
        }
        Some("usage") => usage::report(),
        Some("autostart") => {
            let on = args.get(1).map(String::as_str) != Some("off");
            platform::set_autostart(on)?;
            println!(
                "{}",
                if on { "vtt will start when you log in." } else { "vtt won't start at login anymore." }
            );
            Ok(())
        }
        Some("test-type") => test_type(),
        Some("test-mic") => test_mic(),
        Some("-V" | "--version" | "version") => {
            println!("vtt {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("-h" | "--help" | "help") => help::show(args.get(1).map(String::as_str)),
        Some(other) => bail!("unknown command `{other}`. Run `vtt help` for the guide"),
    }
}

fn start() -> Result<()> {
    if let Some(pid) = platform::running_pid() {
        println!("vtt is already running (process {pid}).");
        return Ok(());
    }
    let cfg = config::load()?;
    if let Some(p) = cfg.problem() {
        bail!("vtt isn't set up yet ({p}). Run `vtt setup`");
    }
    setup::start_and_confirm(&cfg)
}

fn daemon() -> Result<()> {
    let Some(_lock) = platform::lock_instance()? else {
        let pid = platform::running_pid()
            .map(|p| format!(" (process {p})"))
            .unwrap_or_default();
        println!(
            "vtt is already running in the background{pid}.\n\
             `vtt setup` changes settings, `vtt stop` stops it."
        );
        return Ok(());
    };
    let interactive = std::io::stdin().is_terminal();

    let mut cfg = config::load();
    if cfg.as_ref().map_or(true, |c| c.problem().is_some()) {
        if !interactive {
            let why = match &cfg {
                Err(e) => format!("{e:#}"),
                Ok(c) => c.problem().unwrap_or_default(),
            };
            report(&format!("vtt is not set up ({why}). Open a terminal and run: vtt setup"));
            bail!("not set up: {why}");
        }
        println!("vtt isn't set up yet, so let's do that first.");
        setup::run(true)?;
        cfg = config::load();
    }
    let cfg = Arc::new(cfg?);
    if let Some(p) = cfg.problem() {
        bail!("{p}. Run `vtt setup`");
    }
    let hotkey = hotkey::parse(&cfg.hotkey)
        .ok_or_else(|| anyhow!("unknown hotkey `{}`: run `vtt setup`", cfg.hotkey))?;

    // Dictations still being transcribed or typed; settings reload waits for them.
    let busy = Arc::new(AtomicUsize::new(0));

    // Typing happens on its own thread so recording is never blocked by it.
    let mut typer = typer::Typer::new(cfg.key_delay_ms)?;
    let (text_tx, text_rx) = mpsc::channel::<String>();
    {
        let (cfg, busy) = (cfg.clone(), busy.clone());
        thread::spawn(move || {
            for text in text_rx {
                // Looked up each time: follows layout switches (German <-> Turkish ...)
                let layout = layout::resolve(&cfg);
                if let Err(e) = typer.type_text(&text, &layout) {
                    report(&format!("typing failed: {e:#}"));
                }
                busy.fetch_sub(1, SeqCst);
            }
        });
    }

    let (tx, rx) = mpsc::channel();
    let keyboards = hotkey::listen(hotkey, tx.clone())?;
    let reload_pending = Arc::new(AtomicBool::new(false));
    watch_config(tx.clone(), reload_pending.clone());

    let a = cfg.active();
    let how = if cfg.mode == Mode::Hold { "Hold" } else { "Press" };
    println!(
        "vtt ready: {} {} · {keyboards} keyboard(s) · layout {}{}.\n{how} {} to dictate. Ctrl+C to quit.",
        a.provider.name,
        a.model,
        layout::resolve(&cfg),
        if cfg.layout_is_auto() { " (auto)" } else { "" },
        hotkey::pretty(&cfg.hotkey),
    );

    let mut recorder: Option<audio::Recorder> = None;
    let mut take: u64 = 0; // numbers each recording so stale timers are ignored
    let after = |delay: Duration, event: hotkey::Event| {
        let tx = tx.clone();
        thread::spawn(move || {
            thread::sleep(delay);
            let _ = tx.send(event);
        });
    };
    for event in rx {
        use hotkey::Event::*;
        match (cfg.mode, event) {
            (Mode::Hold, Down) | (Mode::Toggle, Down) if recorder.is_none() => {
                match audio::Recorder::start() {
                    Ok(r) => {
                        recorder = Some(r);
                        take += 1;
                        after(Duration::from_secs(cfg.max_seconds.max(1)), Timeout(take));
                        // Hold mode: beep only once it's clearly a dictation, not a
                        // Ctrl+C or another shortcut using the same key.
                        let beep_delay = if cfg.mode == Mode::Hold { 250 } else { 0 };
                        after(Duration::from_millis(beep_delay), BeepDue(take));
                    }
                    Err(e) => report(&format!("{e:#}")),
                }
            }
            (_, BeepDue(n)) if n == take && recorder.is_some() && cfg.sounds => {
                audio::beep(880.0, 60);
            }
            (Mode::Hold, Up) | (Mode::Toggle, Down) => {
                if let Some(r) = recorder.take() {
                    finish(r, &cfg, &text_tx, &busy);
                }
            }
            (_, Timeout(n)) if n == take => {
                if let Some(r) = recorder.take() {
                    report(&format!(
                        "recording stopped at the {} s limit (change it with `vtt setup`)",
                        cfg.max_seconds
                    ));
                    finish(r, &cfg, &text_tx, &busy);
                }
            }
            (Mode::Hold, OtherKey) => {
                // The key was part of a shortcut (e.g. Ctrl+C): discard silently.
                recorder = None;
            }
            (_, Reload) if recorder.is_none() && busy.load(SeqCst) == 0 => {
                reload_pending.store(false, SeqCst);
                match config::load() {
                    Ok(_) => {
                        println!("Settings changed, restarting with the new ones...");
                        return Err(platform::restart_self());
                    }
                    Err(e) => report(&format!("new settings not applied: {e:#}")),
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Poll config.toml; when it changes, keep sending Reload until the main loop is idle
/// enough to act on it.
fn watch_config(tx: Sender<hotkey::Event>, pending: Arc<AtomicBool>) {
    let Ok(path) = config::path() else { return };
    let stamp = move || std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let mut last = stamp();
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(2));
        let now = stamp();
        if now != last {
            last = now;
            pending.store(true, SeqCst);
        }
        if pending.load(SeqCst) && tx.send(hotkey::Event::Reload).is_err() {
            return;
        }
    });
}

fn finish(recorder: audio::Recorder, cfg: &Arc<Config>, text_tx: &Sender<String>, busy: &Arc<AtomicUsize>) {
    let mut clip = recorder.stop();
    // Skip taps and silence: providers may bill a minimum per request, and Whisper
    // tends to invent words when given silence.
    if clip.seconds() < 0.4 || clip.peak() < 0.01 {
        return;
    }
    if cfg.sounds {
        audio::beep(587.0, 60);
    }
    clip.trim_silence();
    busy.fetch_add(1, SeqCst);
    let (cfg, text_tx, busy) = (cfg.clone(), text_tx.clone(), busy.clone());
    thread::spawn(move || {
        let started = std::time::Instant::now();
        let a = cfg.active();
        let phrases = cfg.phrases();
        let result = stt::transcribe(&a, &cfg.language, &phrases.join(", "), &clip.to_wav());
        if let Ok(t) = &result {
            if let Err(e) = usage::record(a.provider.id, &a.model, clip.seconds(), t.cost) {
                eprintln!("could not log usage: {e:#}");
            }
        }
        // Dictated text is only shown in a terminal, never written to the log file.
        let show = std::io::stdout().is_terminal();
        let text = match result {
            Ok(t) if !t.text.is_empty() && !is_hallucination(&t.text) => t.text,
            Ok(t) => {
                if !t.text.is_empty() && show {
                    println!("[dropped likely made-up text] {}", t.text);
                }
                busy.fetch_sub(1, SeqCst);
                return;
            }
            Err(e) => {
                report(&format!("{e:#}"));
                busy.fetch_sub(1, SeqCst);
                return;
            }
        };
        let heard = started.elapsed().as_secs_f32();

        // Optional spelling/grammar correction. Any problem: type what was heard.
        let mut fixed_note = String::new();
        let text = if cfg.correct_text && text.split_whitespace().count() >= 2 && !a.correction_model.is_empty() {
            let t0 = std::time::Instant::now();
            match correct::correct(&a, &a.correction_model, &text, &phrases) {
                Ok(c) => {
                    let _ = usage::record(a.provider.id, &a.correction_model, 0.0, c.cost);
                    fixed_note = format!(", corrected in {:.2}s", t0.elapsed().as_secs_f32());
                    if show && c.text != text {
                        println!("  heard:     {text}");
                    }
                    c.text
                }
                Err(e) => {
                    eprintln!("{e:#} (typing the text as heard)");
                    text
                }
            }
        } else {
            text
        };

        let info = format!("[{:.1}s audio, {heard:.2}s{fixed_note}]", clip.seconds());
        if show {
            println!("{info} {text}");
        } else {
            println!("{info} {} characters", text.chars().count());
        }
        let text = if cfg.add_trailing_space { format!("{text} ") } else { text };
        if text_tx.send(text).is_ok() {
            return; // the typing thread marks it done
        }
        busy.fetch_sub(1, SeqCst);
    });
}

/// Phrases Whisper is known to invent from background noise (it was trained on
/// subtitled videos). Only exact whole-output matches are dropped.
fn is_hallucination(text: &str) -> bool {
    let t: String = text
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ')
        .collect();
    matches!(
        t.trim(),
        "thanks for watching"
            | "thank you for watching"
            | "thank you for watching and see you next time"
            | "please subscribe"
            | "like and subscribe"
            | "subtitles by the amaraorg community"
            | "untertitel im auftrag des zdf 2017"
            | "untertitel der amaraorg community"
    )
}

/// Print an error and show it as a desktop notification (vtt usually runs without a terminal).
fn report(msg: &str) {
    eprintln!("error: {msg}");
    let _ = notify_rust::Notification::new()
        .summary("vtt")
        .body(msg)
        .timeout(6000)
        .show();
}

fn test_type() -> Result<()> {
    let cfg = config::load().unwrap_or_default();
    let mut t = typer::Typer::new(5)?;
    let layout = layout::resolve(&cfg);
    println!("Typing with layout {layout}. Click into a text box. Typing in 3 seconds...");
    thread::sleep(Duration::from_secs(3));
    t.type_text(
        "Hello from vtt! Testing 1, 2, 3? (ok) geht's? Zürich, Größe, ß, @, €",
        &layout::resolve(&cfg), // again: you may have switched layout meanwhile
    )?;
    println!("Done.");
    Ok(())
}

fn test_mic() -> Result<()> {
    println!("Recording 4 seconds, say something...");
    let rec = audio::Recorder::start()?;
    thread::sleep(Duration::from_secs(4));
    let clip = rec.stop();
    let path = std::env::current_dir()?.join("vtt-test.wav");
    std::fs::write(&path, clip.to_wav())?;
    let peak = clip.peak();
    println!(
        "Got {:.1}s of audio, peak level {:.0}%{}",
        clip.seconds(),
        peak * 100.0,
        if peak < 0.01 { "  <- silent: check the input device in Settings > Sound" } else { "" }
    );
    println!("Saved {} (open it to hear what gets sent)", path.display());
    Ok(())
}
