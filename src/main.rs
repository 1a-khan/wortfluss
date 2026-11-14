mod audio;
mod config;
mod correct;
mod help;
mod hotkey;
mod layout;
mod mics;
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
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::SeqCst};
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
        Some("retry") => retry(),
        Some("mics" | "microphones") => {
            for m in mics::list()? {
                println!("{}{}{}\n    {}", m.description, if m.is_default { "  [system default]" } else { "" }, if m.muted { "  [muted]" } else { "" }, m.name);
            }
            Ok(())
        }
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
    mics::select(&cfg.mic); // before any other thread starts
    if let Some(p) = cfg.problem() {
        bail!("{p}. Run `vtt setup`");
    }
    let hotkey = hotkey::parse(&cfg.hotkey)
        .ok_or_else(|| anyhow!("unknown hotkey `{}`: run `vtt setup`", cfg.hotkey))?;

    // Pieces still being transcribed or typed; settings reload waits for them.
    let busy = Arc::new(AtomicUsize::new(0));
    // Held hotkey: typing waits until it's released (a held Ctrl would turn typed
    // letters into shortcuts).
    let key_down = Arc::new(AtomicBool::new(false));

    // Typing happens on its own thread so recording is never blocked by it.
    let mut typer = typer::Typer::new(cfg.key_delay_ms)?;
    let (text_tx, text_rx) = mpsc::channel::<String>();
    {
        let (cfg, busy, key_down) = (cfg.clone(), busy.clone(), key_down.clone());
        thread::spawn(move || {
            for text in text_rx {
                while key_down.load(SeqCst) {
                    thread::sleep(Duration::from_millis(50));
                }
                // Looked up each time: follows layout switches (German <-> Turkish ...)
                let layout = layout::resolve(&cfg);
                if let Err(e) = typer.type_text(&text, &layout) {
                    report(&format!("typing failed: {e:#}"));
                }
                busy.fetch_sub(1, SeqCst);
            }
        });
    }

    // Pieces are transcribed one after another, so the text is typed in order.
    let (job_tx, job_rx) = mpsc::channel::<Job>();
    {
        let (cfg, busy, text_tx) = (cfg.clone(), busy.clone(), text_tx.clone());
        thread::spawn(move || {
            for job in job_rx {
                if !process(job, &cfg, &text_tx) {
                    busy.fetch_sub(1, SeqCst);
                }
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
        "vtt ready: {} {} · mic: {} · {keyboards} keyboard(s) · layout {}{}.\n{how} {} to dictate. Ctrl+C to quit.",
        a.provider.name,
        a.model,
        mics::describe(&cfg.mic),
        layout::resolve(&cfg),
        if cfg.layout_is_auto() { " (auto)" } else { "" },
        hotkey::pretty(&cfg.hotkey),
    );

    // Pieces of about `piece_seconds`, cut at a pause.
    let piece = cfg.piece_seconds.clamp(10, 120) as f32;
    let (piece_min, piece_max) = (piece * 0.67, piece * 1.5);
    let mut rec = Recording::default();
    let mut take: u64 = 0; // numbers each recording so stale timers are ignored
    let current = Arc::new(AtomicU64::new(0));
    let after = |delay: Duration, event: hotkey::Event| {
        let tx = tx.clone();
        thread::spawn(move || {
            thread::sleep(delay);
            let _ = tx.send(event);
        });
    };
    let send = |clip: audio::Clip, rec: &mut Recording, last: bool| {
        rec.pieces += 1;
        busy.fetch_add(1, SeqCst);
        let _ = job_tx.send(Job { clip, last, piece: rec.pieces });
    };
    let stop = |rec: &mut Recording, current: &AtomicU64| {
        current.store(0, SeqCst);
        let Some(r) = rec.recorder.take() else { return };
        let clip = r.stop();
        let worth = clip.seconds() >= 0.4 && clip.has_speech();
        if cfg.sounds && (worth || rec.pieces > 0) {
            audio::beep(587.0, 60);
        }
        if worth {
            send(clip, rec, true);
        } else if rec.pieces == 0 && clip.seconds() >= 0.4 && std::io::stdout().is_terminal() {
            println!("[no speech heard, nothing sent]");
        }
    };

    for event in rx {
        use hotkey::Event::*;
        match event {
            Down => key_down.store(cfg.mode == Mode::Hold, SeqCst),
            Up => key_down.store(false, SeqCst),
            _ => {}
        }
        match (cfg.mode, event) {
            (Mode::Hold, Down) | (Mode::Toggle, Down) if rec.recorder.is_none() => {
                match audio::Recorder::start() {
                    Ok(r) => {
                        rec = Recording { recorder: Some(r), ..Default::default() };
                        take += 1;
                        current.store(take, SeqCst);
                        after(Duration::from_secs(cfg.max_seconds.max(1)), Timeout(take));
                        // Hold mode: beep only once it's clearly a dictation, not a
                        // Ctrl+C or another shortcut using the same key.
                        let beep_delay = if cfg.mode == Mode::Hold { 250 } else { 0 };
                        after(Duration::from_millis(beep_delay), BeepDue(take));
                        // Check once a second whether a piece is ready to send.
                        let (tx, current, n) = (tx.clone(), current.clone(), take);
                        thread::spawn(move || loop {
                            thread::sleep(Duration::from_secs(1));
                            if current.load(SeqCst) != n || tx.send(Tick(n)).is_err() {
                                return;
                            }
                        });
                    }
                    Err(e) => report(&format!("{e:#}")),
                }
            }
            (_, BeepDue(n)) if n == take && rec.recorder.is_some() && cfg.sounds => {
                audio::beep(880.0, 60);
            }
            (_, Tick(n)) if n == take => {
                let Some(clip) = rec.recorder.as_ref().and_then(|r| r.take_piece(piece_min, piece_max)) else {
                    continue;
                };
                if clip.has_speech() {
                    rec.quiet = 0.0;
                    send(clip, &mut rec, false);
                } else {
                    // Only pauses, breathing or background noise: not sent.
                    rec.quiet += clip.seconds();
                    if std::io::stdout().is_terminal() {
                        println!("[skipped {:.0} s without speech]", clip.seconds());
                    }
                    if cfg.mode == Mode::Toggle && rec.quiet >= QUIET_STOP {
                        report(&format!("recording stopped: no speech for {QUIET_STOP:.0} seconds"));
                        stop(&mut rec, &current);
                    }
                }
            }
            (Mode::Hold, Up) | (Mode::Toggle, Down) => stop(&mut rec, &current),
            (_, Timeout(n)) if n == take && rec.recorder.is_some() => {
                report(&format!(
                    "recording stopped at the {} limit (change it with `vtt set mode`)",
                    duration_text(cfg.max_seconds)
                ));
                stop(&mut rec, &current);
            }
            (Mode::Hold, OtherKey) if rec.recorder.is_some() => {
                if rec.pieces == 0 {
                    // The key was part of a shortcut (e.g. Ctrl+C): discard silently.
                    rec.recorder = None;
                    current.store(0, SeqCst);
                } else {
                    stop(&mut rec, &current); // long dictation: keep what was said
                }
            }
            (_, Reload) if rec.recorder.is_none() && busy.load(SeqCst) == 0 => {
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

/// In toggle mode, stop when nobody spoke for this long (forgotten recording).
const QUIET_STOP: f32 = 60.0;

#[derive(Default)]
struct Recording {
    recorder: Option<audio::Recorder>,
    /// Pieces sent so far.
    pieces: u32,
    /// Seconds without speech in a row.
    quiet: f32,
}

/// A piece of a recording waiting to be transcribed.
struct Job {
    clip: audio::Clip,
    /// The final piece of the recording.
    last: bool,
    piece: u32,
}

fn duration_text(secs: u64) -> String {
    if secs % 60 == 0 { format!("{} min", secs / 60) } else { format!("{secs} s") }
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

/// Transcribe (and maybe correct) one piece and hand the text to the typing thread.
/// Returns true if the typing thread took over the `busy` count.
fn process(job: Job, cfg: &Config, text_tx: &Sender<String>) -> bool {
    let Job { mut clip, last, piece } = job;
    let recorded = clip.seconds();
    clip.trim_silence();
    clip.shorten_pauses();
    let started = std::time::Instant::now();
    let a = cfg.active();
    let phrases = cfg.phrases();
    let wav = clip.to_wav();
    let show = std::io::stdout().is_terminal(); // dictated text never goes to the log file
    let text = match stt::transcribe(&a, &cfg.language, &phrases.join(", "), &wav) {
        Ok(t) => {
            if let Err(e) = usage::record(a.provider.id, &a.model, clip.seconds(), t.cost) {
                eprintln!("could not log usage: {e:#}");
            }
            if t.text.is_empty() || is_hallucination(&t.text) {
                if !t.text.is_empty() && show {
                    println!("[dropped likely made-up text] {}", t.text);
                }
                return false;
            }
            t.text
        }
        Err(e) => {
            // Keep the audio so nothing said is lost.
            match save_failed(&wav, piece) {
                Ok(path) => report(&format!(
                    "{e:#}\nThe audio was saved ({}); `vtt retry` transcribes it later.",
                    path.display()
                )),
                Err(save) => report(&format!("{e:#} (could not save the audio: {save:#})")),
            }
            return false;
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

    let part = if last && piece == 1 { String::new() } else { format!("piece {piece}, ") };
    let info = format!(
        "[{part}{recorded:.1}s recorded, {:.1}s sent, {heard:.2}s{fixed_note}]",
        clip.seconds()
    );
    if show {
        println!("{info} {text}");
    } else {
        println!("{info} {} characters", text.chars().count());
    }
    // Pieces of one dictation are separated by a space.
    let text = if !last || cfg.add_trailing_space { format!("{text} ") } else { text };
    text_tx.send(text).is_ok()
}

fn failed_dir() -> Result<std::path::PathBuf> {
    Ok(platform::state_dir()?.join("failed"))
}

fn save_failed(wav: &[u8], piece: u32) -> Result<std::path::PathBuf> {
    let dir = failed_dir()?;
    std::fs::create_dir_all(&dir)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let path = dir.join(format!("{now}-{piece:03}.wav"));
    std::fs::write(&path, wav)?;
    Ok(path)
}

/// `vtt retry`: transcribe audio that failed earlier (e.g. no internet) and print it.
fn retry() -> Result<()> {
    let dir = failed_dir()?;
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    files.retain(|p| p.extension().is_some_and(|e| e == "wav"));
    files.sort();
    if files.is_empty() {
        println!("Nothing to retry: no saved recordings.");
        return Ok(());
    }
    let cfg = config::load()?;
    let a = cfg.active();
    println!("{} saved recording(s); transcribing with {} {}:\n", files.len(), a.provider.name, a.model);
    for path in files {
        let wav = std::fs::read(&path)?;
        match stt::transcribe(&a, &cfg.language, &cfg.phrases().join(", "), &wav) {
            Ok(t) => {
                let secs = wav.len().saturating_sub(44) as f32 / (audio::TARGET_RATE as f32 * 2.0);
                let _ = usage::record(a.provider.id, &a.model, secs, t.cost);
                println!("{}:\n{}\n", path.file_name().unwrap_or_default().to_string_lossy(), t.text);
                std::fs::remove_file(&path)?;
            }
            Err(e) => println!("{}: still failing: {e:#}\n", path.display()),
        }
    }
    println!("Copy the text from here. Successful recordings were deleted.");
    Ok(())
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
    let cfg = config::load().unwrap_or_default();
    mics::select(&cfg.mic);
    println!("Microphone: {}", mics::describe(&cfg.mic));
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
