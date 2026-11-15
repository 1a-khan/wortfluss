mod audio;
mod config;
mod correct;
mod credits;
mod help;
mod hotkey;
mod i18n;
mod icon;
mod indicator;
#[cfg(target_os = "linux")]
#[path = "linux/layout.rs"]
mod layout;
#[cfg(windows)]
#[path = "windows/layout.rs"]
mod layout;
#[cfg(target_os = "linux")]
#[path = "linux/mics.rs"]
mod mics;
#[cfg(windows)]
#[path = "windows/mics.rs"]
mod mics;
mod notes;
#[cfg(target_os = "linux")]
#[path = "linux/overlay.rs"]
mod overlay;
#[cfg(windows)]
#[path = "windows/overlay.rs"]
mod overlay;
mod platform;
mod providers;
mod rewrite;
mod session;
mod speakers;
#[cfg(target_os = "linux")]
#[path = "linux/tray.rs"]
mod tray;
#[cfg(windows)]
#[path = "windows/tray.rs"]
mod tray;
mod secrets;
mod setup;
mod stt;
mod supervisor;
#[cfg(target_os = "linux")]
#[path = "linux/typer.rs"]
mod typer;
#[cfg(windows)]
#[path = "windows/typer.rs"]
mod typer;
mod update;
mod usage;

use anyhow::{anyhow, bail, Result};
use config::{Config, Mode};
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::SeqCst};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;


fn main() {
    let result = run();
    if let Err(e) = &result {
        eprintln!("error: {e:#}");
    }
    // A console window of its own (Windows: started from the Start menu, the tray or the
    // installer) closes when vtt ends: wait, so the last message can be read.
    if platform::owns_console() {
        println!("\nPress Enter to close this window.");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
    if result.is_err() {
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("run") => daemon(),
        Some("worker") => worker(),
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
        Some("session" | "sessions") => session::cmd(&args[1..]),
        Some("notes") => notes::cmd(&args[1..]),
        Some("speakers" | "speaker" | "names") => speakers::cmd(&args[1..]),
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
        Some("_rewrite") => rewrite::window(&args[1..]),
        Some("_indicator-demo") => indicator::demo(),
        Some("update" | "upgrade") => update_now(),
        Some("_release-keygen") => {
            update::keygen(std::path::Path::new(args.get(1).ok_or_else(|| anyhow!("vtt _release-keygen KEYFILE"))?))
        }
        Some("_release-sign") => update::sign_release(&args[1..]),
        Some("_diarize") => {
            // developer test: vtt _diarize FILE.wav [MODEL] -> speaker segments
            let cfg = config::load()?;
            let a = cfg.active();
            let wav = std::fs::read(args.get(1).ok_or_else(|| anyhow::anyhow!("vtt _diarize FILE.wav [MODEL]"))?)?;
            let model = args.get(2).cloned().unwrap_or_else(|| stt::speaker_model(a.provider.id).unwrap_or("").to_string());
            let d = stt::diarize(&a, &model, &cfg.language, &wav)?;
            for s in d.segments {
                println!("{:6.1}-{:6.1} [{}] {}", s.start, s.end, s.speaker.unwrap_or_default(), s.text);
            }
            println!("cost: {:?}", d.cost);
            Ok(())
        }
        Some("_icons") => {
            // preview the icon: vtt _icons DIR
            let dir = std::path::PathBuf::from(args.get(1).map(String::as_str).unwrap_or("."));
            for (name, st) in [("ready", icon::State::Ready), ("dictating", icon::State::Dictating), ("session", icon::State::Session)] {
                for size in [22u32, 64] {
                    icon::write_png(&dir.join(format!("tray-{name}-{size}.png")), size, st, false)?;
                }
            }
            icon::write_png(&dir.join("app-256.png"), 256, icon::State::Ready, true)?;
            icon::write_ico(&dir.join("wortfluss.ico"))?;
            Ok(())
        }
        Some("-V" | "--version" | "version") => {
            println!("vtt {}", env!("CARGO_PKG_VERSION"));
            println!("{}\n{}", credits::line(), credits::links());
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
    let cfg = config::load();
    if cfg.as_ref().map_or(true, |c| c.problem().is_some()) {
        if !interactive {
            let why = match &cfg {
                Err(e) => format!("{e:#}"),
                Ok(c) => c.problem().unwrap_or_default(),
            };
            report(&i18n::tf("err.not_set_up", &[&why]));
            bail!("not set up: {why}");
        }
        println!("vtt isn't set up yet, so let's do that first.");
        setup::run(true)?;
        if let Some(p) = config::load()?.problem() {
            bail!("{p}. Run `vtt setup`");
        }
    }
    // This process is the supervisor; the work runs in a child that is restarted if it
    // crashes (see supervisor.rs).
    let outcome = supervisor::supervise(
        &supervisor::Policy {
            name: "Wortfluss",
            max_restarts: 5,
            within: Duration::from_secs(60),
            grace: Duration::from_secs(5),
        },
        |_| vec!["worker".to_string()],
    )?;
    if outcome == supervisor::Outcome::Updated {
        // The worker installed a new version: start over with it (supervisor included).
        drop(_lock);
        return Err(platform::restart_self());
    }
    Ok(())
}

/// `vtt update`: install a newer version now, then restart the running Wortfluss.
fn update_now() -> Result<()> {
    println!("Wortfluss {} ({}). Looking for a new version...", update::current_version(), update::platform_id());
    let Some(m) = update::check()? else {
        println!("Up to date.");
        return Ok(());
    };
    println!("Downloading {} ...", m.version);
    update::download(&m)?;
    update::swap(&m)?;
    println!("Installed {}.", m.version);
    if platform::running_pid().is_some() {
        if session::is_running() {
            println!("A session is running. Wortfluss switches to the new version after `vtt stop` and `vtt start`.");
        } else {
            platform::stop_running()?;
            platform::start_background_with(&["run"])?;
            println!("Wortfluss restarted with the new version.");
        }
    }
    Ok(())
}

/// Look for a new version; when found, download and check it and tell the main loop.
/// `manual`: the user asked (tray), so also say "up to date" and show errors.
fn find_update(pending: &Mutex<Option<update::Manifest>>, tx: &mpsc::Sender<hotkey::Event>, manual: bool) {
    if manual {
        info(i18n::t("upd.checking"));
    }
    let result = update::check().and_then(|found| match found {
        Some(m) => {
            if manual {
                info(&i18n::tf("upd.downloading", &[&m.version]));
            }
            update::download(&m)?;
            Ok(Some(m))
        }
        None => Ok(None),
    });
    match result {
        Ok(Some(m)) => {
            eprintln!("update {} downloaded; installing when idle", m.version);
            *pending.lock().unwrap() = Some(m);
            let _ = tx.send(hotkey::Event::UpdateReady);
        }
        Ok(None) if manual => info(&i18n::tf("upd.none", &[update::current_version()])),
        Ok(None) => {}
        Err(e) if manual => report(&i18n::tf("upd.failed", &[&format!("{e:#}")])),
        Err(e) => eprintln!("update check failed: {e:#}"),
    }
}

/// The actual work: hotkey, recording, transcription, typing, tray. Normally started by
/// the supervisor (`vtt run`).
fn worker() -> Result<()> {
    let cfg = Arc::new(config::load()?);
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
                // Each text in isolation: a crash while typing loses this text only, and
                // the virtual keyboard is created again.
                let typed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    match cfg.paste.then(|| typer::paste(&text)).flatten() {
                        Some(pasted) => pasted,
                        None => typer.type_text(&text, &layout), // pasting not possible here
                    }
                }));
                match typed {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => report(&i18n::tf("err.typing", &[&format!("{e:#}")])),
                    Err(_) => {
                        report(i18n::t("err.typing_crash"));
                        if let Ok(t) = typer::Typer::new(cfg.key_delay_ms) {
                            typer = t;
                        }
                    }
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
                // Each piece in isolation: a crash loses this piece only.
                let handed_on = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    process(job, &cfg, &text_tx)
                }));
                match handed_on {
                    Ok(true) => {}
                    Ok(false) => {
                        busy.fetch_sub(1, SeqCst);
                    }
                    Err(_) => {
                        report(i18n::t("err.piece_crash"));
                        busy.fetch_sub(1, SeqCst);
                    }
                }
            }
        });
    }

    let (tx, rx) = mpsc::channel();
    let session_key = hotkey::parse(&cfg.session_hotkey).filter(|k| *k != hotkey);
    let keyboards = hotkey::listen(hotkey, session_key, cfg.shortcuts(), tx.clone())?;
    let tray = if cfg.tray { tray::start(&cfg, tx.clone()) } else { None };
    session::cleanup_old_audio(&cfg); // kept session audio past its expiry
    if let Some(t) = tray.clone() {
        // Sessions run in their own process: keep the tray's session state current.
        thread::spawn(move || {
            let mut last = session::is_running();
            loop {
                thread::sleep(Duration::from_secs(2));
                let now = session::is_running();
                if now != last {
                    last = now;
                    if t.update(move |t| t.session = now).is_none() {
                        return;
                    }
                }
            }
        });
    }
    let reload_pending = Arc::new(AtomicBool::new(false));
    // Automatic updates: first look 2 minutes after the start, then every 6 hours.
    update::cleanup();
    let pending_update: Arc<Mutex<Option<update::Manifest>>> = Arc::default();
    if cfg.auto_update {
        let (pending, tx) = (pending_update.clone(), tx.clone());
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(120));
            while pending.lock().unwrap().is_none() {
                find_update(&pending, &tx, false);
                thread::sleep(Duration::from_secs(6 * 3600));
            }
        });
    }
    let rewriting = Arc::new(AtomicBool::new(false));
    watch_config(tx.clone(), reload_pending.clone());
    let indicator = indicator::Indicator::start(cfg.indicator, busy.clone());
    // Just updated: say so (the old version left a note).
    if let Ok(path) = platform::state_dir().map(|d| d.join(UPDATED_MARKER)) {
        if let Ok(version) = std::fs::read_to_string(&path) {
            let _ = std::fs::remove_file(&path);
            info(&i18n::tf("upd.done", &[version.trim()]));
        }
    }
    // GNOME: the bubble is drawn by a small shell extension; keep it installed and up to
    // date while the indicator is on, remove it when switched off.
    #[cfg(target_os = "linux")]
    {
        let on = cfg.indicator || cfg.paste; // the extension also pastes
        thread::spawn(move || {
            let done = if on {
                overlay::ensure_extension().map(|i| {
                    if i == overlay::Install::NeedsLogin {
                        info(i18n::t("ind.relogin"));
                    }
                })
            } else {
                overlay::remove_extension()
            };
            if let Err(e) = done {
                eprintln!("listening indicator: {e:#}");
            }
        });
    }

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
        if let Some(t) = &tray {
            t.update(|t| t.dictating = false);
        }
        let Some(r) = rec.recorder.take() else { return };
        let clip = r.stop();
        let worth = clip.seconds() >= 0.4 && clip.has_speech();
        if worth || rec.pieces > 0 {
            indicator.working(); // until the text is typed
        } else {
            indicator.hide();
        }
        if cfg.sounds && (worth || rec.pieces > 0) {
            audio::beep(587.0, 60);
        }
        if worth {
            send(clip, rec, true);
        } else if rec.pieces == 0 && clip.seconds() >= 0.4 && std::io::stdout().is_terminal() {
            println!("[no speech heard, nothing sent]");
        }
    };

    // Toggle mode reacts to a clean tap (press and release, no other key in between), so
    // a Ctrl hotkey used in Ctrl+C or Ctrl+Alt+S doesn't start or stop dictation.
    let mut tap_clean = false;
    for event in rx {
        use hotkey::Event::*;
        match event {
            Down => key_down.store(cfg.mode == Mode::Hold, SeqCst),
            Up => key_down.store(false, SeqCst),
            _ => {}
        }
        let event = match (cfg.mode, event) {
            (Mode::Toggle, Down) => {
                tap_clean = true;
                continue;
            }
            (Mode::Toggle, OtherKey) => {
                tap_clean = false;
                OtherKey
            }
            (Mode::Toggle, Up) if tap_clean => {
                tap_clean = false;
                Down // the tap: start or stop
            }
            (Mode::Toggle, Up) => continue,
            (_, e) => e,
        };
        match (cfg.mode, event) {
            (Mode::Hold, Down) | (Mode::Toggle, Down) if rec.recorder.is_none() => {
                match audio::Recorder::start() {
                    Ok(r) => {
                        rec = Recording { recorder: Some(r), ..Default::default() };
                        if let Some(t) = &tray {
                            t.update(|t| t.dictating = true);
                        }
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
            (_, BeepDue(n)) if n == take && rec.recorder.is_some() => {
                // Clearly a dictation now (hold mode waits 250 ms): beep and show the bubble.
                if let Some(r) = &rec.recorder {
                    indicator.listen(r.level());
                }
                if cfg.sounds {
                    audio::beep(880.0, 60);
                }
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
                        report(&i18n::tf("msg.no_speech", &[&format!("{QUIET_STOP:.0}")]));
                        stop(&mut rec, &current);
                    }
                }
            }
            (Mode::Hold, Up) | (Mode::Toggle, Down) => stop(&mut rec, &current),
            (_, Timeout(n)) if n == take && rec.recorder.is_some() => {
                report(&i18n::tf("msg.limit", &[&duration_text(cfg.max_seconds)]));
                stop(&mut rec, &current);
            }
            (_, SessionKey) => handle_action(hotkey::Action::Session),
            (_, Shortcut(hotkey::Action::Rewrite)) => {
                // One window at a time; the accepted text is typed like dictated text.
                if !rewriting.swap(true, SeqCst) {
                    let (rewriting, busy, text_tx) = (rewriting.clone(), busy.clone(), text_tx.clone());
                    thread::spawn(move || {
                        match rewrite::ask() {
                            #[cfg(windows)]
                            Ok(Some(accepted)) => {
                                busy.fetch_add(1, SeqCst);
                                if let Err(e) = rewrite::paste(&accepted) {
                                    report(&i18n::tf("err.rewrite", &[&format!("{e:#}")]));
                                }
                                busy.fetch_sub(1, SeqCst);
                                let _ = &text_tx;
                            }
                            #[cfg(not(windows))]
                            Ok(Some(accepted)) => {
                                thread::sleep(Duration::from_millis(250)); // focus back to the app
                                busy.fetch_add(1, SeqCst);
                                if text_tx.send(accepted.text).is_err() {
                                    busy.fetch_sub(1, SeqCst);
                                }
                            }
                            Ok(None) => {}
                            Err(e) => report(&i18n::tf("err.rewrite", &[&format!("{e:#}")])),
                        }
                        rewriting.store(false, SeqCst);
                    });
                }
            }
            (_, Shortcut(hotkey::Action::CheckUpdates)) => {
                let (pending, tx) = (pending_update.clone(), tx.clone());
                thread::spawn(move || find_update(&pending, &tx, true));
            }
            (_, UpdateReady) => {
                // Only when nothing is going on: no dictation, typing, rewrite window or session.
                let idle = rec.recorder.is_none()
                    && busy.load(SeqCst) == 0
                    && !rewriting.load(SeqCst)
                    && !session::is_running();
                if !idle {
                    after(Duration::from_secs(30), UpdateReady);
                } else if let Some(m) = pending_update.lock().unwrap().take() {
                    match update::swap(&m) {
                        Ok(()) => {
                            // Said by the new version once it runs: a notice sent right
                            // before exiting gets lost (Windows drops it).
                            if let Ok(dir) = platform::state_dir() {
                                let _ = std::fs::write(dir.join(UPDATED_MARKER), &m.version);
                            }
                            println!("{}", i18n::tf("upd.done", &[&m.version]));
                            if supervisor::is_supervised() {
                                std::process::exit(supervisor::UPDATED);
                            }
                            return Err(platform::restart_self());
                        }
                        Err(e) => report(&i18n::tf("upd.failed", &[&format!("{e:#}")])),
                    }
                }
            }
            (_, Shortcut(action)) => handle_action(action),
            (Mode::Hold, OtherKey) if rec.recorder.is_some() => {
                if rec.pieces == 0 {
                    // The key was part of a shortcut (e.g. Ctrl+C): discard silently.
                    rec.recorder = None;
                    indicator.hide();
                    current.store(0, SeqCst);
                    if let Some(t) = &tray {
                        t.update(|t| t.dictating = false);
                    }
                } else {
                    stop(&mut rec, &current); // long dictation: keep what was said
                }
            }
            (_, Reload) if rec.recorder.is_none() && busy.load(SeqCst) == 0 => {
                reload_pending.store(false, SeqCst);
                match config::load() {
                    Ok(_) => {
                        println!("Settings changed, restarting with the new ones...");
                        if supervisor::is_supervised() {
                            std::process::exit(supervisor::RESTART); // the supervisor restarts us
                        }
                        return Err(platform::restart_self());
                    }
                    Err(e) => report(&i18n::tf("err.settings_not_applied", &[&format!("{e:#}")])),
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Carry out a shortcut or tray menu action. Settings changes are saved to config.toml;
/// the running vtt then restarts itself with them.
fn handle_action(action: hotkey::Action) {
    use hotkey::Action::*;
    match action {
        Session => session_toggle(None),
        CallSession => session_toggle(Some("both")),
        StartSession(src) => session_toggle(Some(&src)),
        StopSession => session_toggle(None),
        ToggleCorrection => change_setting(|c| {
            c.correct_text = !c.correct_text;
            i18n::switched("tray.correction", c.correct_text)
        }),
        ToggleMode => change_setting(|c| {
            c.mode = if c.mode == Mode::Hold { Mode::Toggle } else { Mode::Hold };
            let key = hotkey::pretty(&c.hotkey);
            if c.mode == Mode::Hold {
                i18n::tf("tray.mode_hold", &[&key])
            } else {
                i18n::tf("tray.mode_toggle", &[&key])
            }
        }),
        ToggleSounds => change_setting(|c| {
            c.sounds = !c.sounds;
            i18n::switched("tray.beeps", c.sounds)
        }),
        ToggleKeepAudio => change_setting(|c| {
            c.session_keep_audio = !c.session_keep_audio;
            i18n::switched("set.keep_audio", c.session_keep_audio)
        }),
        ToggleSpeakers => change_setting(|c| {
            c.session_speakers = !c.session_speakers;
            if c.session_speakers && c.active().speaker_model.is_empty() {
                c.session_speakers = false;
                i18n::tf("set.no_speakers", &[c.active().provider.name])
            } else {
                i18n::switched("set.speakers", c.session_speakers)
            }
        }),
        SetMic(name) => change_setting(move |c| {
            c.mic = name;
            i18n::tf("msg.switched", &[i18n::t("tray.mic"), &mics::describe(&c.mic)])
        }),
        SetNotes(kind) => change_setting(move |c| {
            c.session_notes = kind;
            i18n::tf("msg.switched", &[i18n::t("tray.notes"), i18n::t(&format!("notes.{}", c.session_notes))])
        }),
        OpenTranscripts => {
            let result = config::load()
                .and_then(|c| session::session_dir(&c))
                .and_then(|d| {
                    std::fs::create_dir_all(&d)?;
                    platform::open_path(&d)
                });
            if let Err(e) = result {
                report(&format!("{e:#}"));
            }
        }
        Rewrite | CheckUpdates => {} // need the main loop's state: handled there
        OpenUrl(url) => {
            if let Err(e) = platform::open_path(std::path::Path::new(&url)) {
                report(&format!("{e:#}"));
            }
        }
        OpenSettings => {
            if let Err(e) = platform::open_in_terminal(&["setup"]) {
                report(&format!("{e:#}"));
            }
        }
        Quit => {
            info("vtt stopped. Start it again with `vtt start` (it also starts at login).");
            std::process::exit(0);
        }
    }
}

/// Start a session (with this source), or stop the running one.
fn session_toggle(source: Option<&str>) {
    let source = source.map(str::to_string);
    thread::spawn(move || {
        let result = if session::is_running() {
            platform::stop_named("vtt-session", Duration::from_secs(300)).map(|_| ())
        } else {
            let mut args = vec!["session", "run"];
            if let Some(s) = &source {
                args.extend(["--source", s.as_str()]);
            }
            platform::start_background_with(&args)
        };
        if let Err(e) = result {
            report(&i18n::tf("err.session", &[&format!("{e:#}")]));
        }
    });
}

fn change_setting(f: impl FnOnce(&mut Config) -> String) {
    let result = config::load().and_then(|mut c| {
        let msg = f(&mut c);
        config::save(&c)?;
        Ok(msg)
    });
    match result {
        Ok(msg) => info(&msg),
        Err(e) => report(&i18n::tf("err.setting_not_changed", &[&format!("{e:#}")])),
    }
}

/// A notification with the Wortfluss icon.
pub(crate) fn notification() -> notify_rust::Notification {
    let mut n = notify_rust::Notification::new();
    n.appname("Wortfluss");
    if let Some(p) = platform::app_icon_path() {
        n.icon(&p.to_string_lossy());
    }
    n
}

/// File in the state folder: "updated to this version", for the next start to announce.
const UPDATED_MARKER: &str = "updated-to";

/// A short desktop notification (not an error).
fn info(msg: &str) {
    println!("{msg}");
    let _ = notification().summary("Wortfluss").body(msg).timeout(3000).show();
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
                Ok(path) => report(&i18n::tf("msg.audio_saved", &[&format!("{e:#}"), &path.display().to_string()])),
                Err(save) => report(&i18n::tf("msg.audio_not_saved", &[&format!("{e:#}"), &format!("{save:#}")])),
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

pub(crate) fn save_failed(wav: &[u8], piece: u32) -> Result<std::path::PathBuf> {
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
pub(crate) fn is_hallucination(text: &str) -> bool {
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
    let _ = notification()
        .summary("Wortfluss")
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
    let text = "Hello from vtt! Testing 1, 2, 3? (ok) geht's? Zürich, Größe, ß, @, €";
    // The same way as dictation: pasted if that is on and possible here, else typed.
    match cfg.paste.then(|| typer::paste(text)).flatten() {
        Some(pasted) => {
            pasted?;
            println!("Done (pasted).");
        }
        None => {
            t.type_text(text, &layout::resolve(&cfg))?; // again: the layout may have changed
            println!("Done (typed).");
        }
    }
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
