//! Session mode: transcribe what the computer plays (lectures, tutorials, the other side
//! of a call) and/or the microphone into a Markdown file, live, then write notes.
//!
//! Each source is recorded as its own stream, cut into ~30 s pieces at pauses; pieces
//! without speech are skipped. With both sources the lines are labelled "You" (mic) and
//! "Others" (system audio). Only text is kept, never audio (except failed pieces until
//! `vtt retry`).

use crate::config::{self, Config};
use crate::mics::{self, SourceRecorder};
use crate::notes::{self, Kind};
use crate::{audio, correct, platform, stt, usage};
use anyhow::{bail, Context, Result};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const LOCK: &str = "vtt-session";
const STATE_FILE: &str = "vtt-session.txt";
/// Stop a session after this long without any speech (forgotten session).
const QUIET_STOP: f32 = 30.0 * 60.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    System,
    Mic,
    Both,
}

impl Source {
    pub fn parse(s: &str) -> Option<Source> {
        match s.trim().to_lowercase().as_str() {
            "system" | "speaker" | "speakers" | "computer" | "desktop" => Some(Source::System),
            "mic" | "microphone" => Some(Source::Mic),
            "both" | "all" | "meeting" => Some(Source::Both),
            _ => None,
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Source::System => "system audio (what the computer plays)",
            Source::Mic => "microphone",
            Source::Both => "system audio + microphone",
        }
    }
}

struct Options {
    source: Source,
    title: String,
    notes: Option<Kind>,
    /// Record this sound-server source instead of the default output (testing, advanced).
    from: Option<String>,
    /// Save the transcript in this folder instead of the configured one.
    dir: Option<PathBuf>,
    /// Keep the audio (overrides the setting for this session).
    keep_audio: bool,
}

fn parse_options(args: &[String], cfg: &Config) -> Result<Options> {
    let mut source = Source::parse(&cfg.session_source).unwrap_or(Source::System);
    let mut notes_arg: Option<String> = None;
    let mut from = None;
    let mut dir = None;
    let mut keep_audio = cfg.session_keep_audio;
    let mut title = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--source" | "-s" => {
                let v = it.next().context("--source needs system, mic or both")?;
                source = Source::parse(v).with_context(|| format!("unknown source `{v}`: system, mic or both"))?;
            }
            "--notes" | "-n" => notes_arg = Some(it.next().context("--notes needs lecture, meeting, summary or none")?.clone()),
            "--from" => from = Some(it.next().context("--from needs a source name (see `vtt mics`)")?.clone()),
            "--dir" => dir = Some(PathBuf::from(it.next().context("--dir needs a folder")?)),
            "--keep-audio" => keep_audio = true,
            "--no-audio" => keep_audio = false,
            other => title.push(other.to_string()),
        }
    }
    let notes_setting = notes_arg.unwrap_or_else(|| cfg.session_notes.clone());
    let notes = match notes_setting.trim().to_lowercase().as_str() {
        "none" | "off" | "no" => None,
        "" | "auto" => Some(if source == Source::Both { Kind::Meeting } else { Kind::Lecture }),
        k => Some(Kind::parse(k).with_context(|| format!("unknown notes type `{k}`: lecture, meeting, summary or none"))?),
    };
    let title = if title.is_empty() {
        match source {
            Source::Both => "Meeting".to_string(),
            _ => "Session".to_string(),
        }
    } else {
        title.join(" ")
    };
    Ok(Options { source, title, notes, from, dir, keep_audio })
}

/// `vtt session [start|run|stop|status] ...`
pub fn cmd(args: &[String]) -> Result<()> {
    let rest = &args[args.len().min(1)..];
    match args.first().map(String::as_str) {
        Some("start") => start(rest),
        Some("run") => run(rest),
        Some("stop") => stop(),
        Some("status") | None => status(),
        Some("redo") => redo(rest),
        Some("delete-audio") => delete_audio_cmd(rest),
        Some(other) => bail!("unknown `vtt session {other}`: start, stop, status, redo, delete-audio or run"),
    }
}

/// Start a session in the background.
pub fn start(args: &[String]) -> Result<()> {
    if platform::running_pid_named(LOCK).is_some() {
        bail!("a session is already running: `vtt session status`, `vtt session stop`");
    }
    let cfg = config::load()?;
    if let Some(p) = cfg.problem() {
        bail!("vtt isn't set up yet ({p}). Run `vtt setup`");
    }
    let opts = parse_options(args, &cfg)?; // check the arguments before going to the background
    let mut full = vec!["session", "run"];
    full.extend(args.iter().map(String::as_str));
    platform::start_background_with(&full)?;
    for _ in 0..30 {
        thread::sleep(Duration::from_millis(100));
        if let Some(path) = current_file() {
            println!("Session started: {}.", opts.source.describe());
            println!("Transcript: {}", path.display());
            println!("Stop with `vtt session stop`{}.", if opts.notes.is_some() { " (notes are written then)" } else { "" });
            println!("Only record other people with their consent.");
            if opts.source == Source::Both && mics::output_is_headphones() == Some(false) {
                println!("Tip: use headphones. With loudspeakers the microphone also hears the other side, so their words can appear twice.");
            }
            return Ok(());
        }
    }
    bail!("the session did not start; see {}", platform::log_file()?.display())
}

/// Stop the running session and wait until its notes are written.
pub fn stop() -> Result<()> {
    let file = current_file();
    if platform::running_pid_named(LOCK).is_none() {
        println!("No session is running.");
        return Ok(());
    }
    println!("Stopping: transcribing the last piece and writing notes (can take up to a minute)...");
    platform::stop_named(LOCK, Duration::from_secs(300))?;
    match file {
        Some(f) => println!("Session saved: {}", f.display()),
        None => println!("Session stopped."),
    }
    Ok(())
}

pub fn status() -> Result<()> {
    match (platform::running_pid_named(LOCK), current_file()) {
        (Some(pid), Some(f)) => println!("A session is running (process {pid}).\nTranscript: {}\nStop with `vtt session stop`.", f.display()),
        (Some(pid), None) => println!("A session is starting (process {pid})."),
        _ => println!(
            "No session is running. Start one with `vtt session start [title] [--source system|mic|both]`."
        ),
    }
    Ok(())
}

/// Is a session running? (for the session hotkey)
pub fn is_running() -> bool {
    platform::running_pid_named(LOCK).is_some()
}

fn current_file() -> Option<PathBuf> {
    let text = std::fs::read_to_string(platform::runtime_file(STATE_FILE).ok()?).ok()?;
    let p = PathBuf::from(text.trim());
    p.exists().then_some(p)
}

pub fn session_dir(cfg: &Config) -> Result<PathBuf> {
    if !cfg.session_dir.trim().is_empty() {
        let d = cfg.session_dir.trim();
        return Ok(match d.strip_prefix("~/") {
            Some(rest) => PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?).join(rest),
            None => PathBuf::from(d),
        });
    }
    Ok(platform::documents_dir()?.join("vtt"))
}

fn safe_name(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '-' } else { c })
        .collect();
    s.trim().chars().take(80).collect()
}

fn clock(secs: f32) -> String {
    let s = secs.max(0.0) as u64;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

fn notify(msg: &str) {
    let _ = notify_rust::Notification::new().summary("vtt session").body(msg).timeout(8000).show();
}

/// A piece of one stream waiting to be transcribed.
struct Job {
    clip: audio::Clip,
    label: &'static str,
    /// Seconds since the session started.
    at: f32,
}

/// Run a session in this process until Ctrl+C / `vtt session stop`.
pub fn run(args: &[String]) -> Result<()> {
    let cfg = config::load()?;
    if let Some(p) = cfg.problem() {
        bail!("vtt isn't set up yet ({p}). Run `vtt setup`");
    }
    let opts = parse_options(args, &cfg)?;
    let Some(_lock) = platform::lock_named(LOCK)? else {
        bail!("a session is already running: `vtt session status`");
    };
    platform::catch_stop_signals();
    cleanup_old_audio(&cfg);

    // The transcript file.
    let dir = match &opts.dir {
        Some(d) => d.clone(),
        None => session_dir(&cfg)?,
    };
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let (y, mo, d, h, mi, _) = platform::local_time();
    let path = dir.join(format!("{y}-{mo:02}-{d:02} {h:02}-{mi:02} {}.md", safe_name(&opts.title)));
    let a = cfg.active();
    let header = format!(
        "# {}\n\n_{y}-{mo:02}-{d:02} {h:02}:{mi:02} · {} · {} {}_\n\n{}\n{}\n{}\n\n{}\n\n",
        opts.title,
        opts.source.describe(),
        a.provider.name,
        a.model,
        notes::START,
        match opts.notes {
            Some(k) => format!("_{} will appear here when the session ends._", capitalize(k.name())),
            None => "_No notes requested. Create them later: `vtt notes FILE lecture|meeting|summary`._".to_string(),
        },
        notes::END,
        notes::TRANSCRIPT_HEADING,
    );
    platform::write_private(&path, &header)?;
    std::fs::write(platform::runtime_file(STATE_FILE)?, path.to_string_lossy().as_bytes())?;
    let show = std::io::stdout().is_terminal();
    println!("Session started: {}\nTranscript: {}", opts.source.describe(), path.display());
    if show {
        println!("Press Ctrl+C to stop{}.\n", if opts.notes.is_some() { " and write the notes" } else { "" });
    }
    let no_headphones = opts.source == Source::Both && mics::output_is_headphones() == Some(false);
    notify(&format!(
        "Transcribing {}. Only record other people with their consent.{}\nStop: vtt session stop",
        opts.source.describe(),
        if no_headphones {
            "\nUse headphones: with loudspeakers the other side may be transcribed twice."
        } else {
            ""
        }
    ));
    if no_headphones {
        println!("Tip: use headphones; with loudspeakers the other side may be transcribed twice.");
    }

    // The streams.
    let mic = if cfg.mic.trim().is_empty() { mics::DEFAULT_MIC.to_string() } else { cfg.mic.trim().to_string() };
    let system = opts.from.clone().unwrap_or_else(|| mics::SYSTEM_AUDIO.to_string());
    let wanted: Vec<(&'static str, String)> = match opts.source {
        Source::System => vec![("", system)],
        Source::Mic => vec![("", mic)],
        Source::Both => vec![("Others", system), ("You", mic)],
    };
    let mut streams = Vec::new();
    let mut audio_files = Vec::new();
    for (label, name) in wanted {
        let rec = SourceRecorder::start(&name).with_context(|| format!("cannot record {name}"))?;
        let wav = if opts.keep_audio {
            let p = audio_path(&path, label)?;
            audio_files.push(p.clone());
            Some(WavWriter::create(&p)?)
        } else {
            None
        };
        streams.push((label, rec, 0.0f32, wav)); // (label, recorder, seconds taken, kept audio)
    }

    // One worker transcribes pieces in order and appends them to the file.
    let (tx, rx) = mpsc::channel::<Job>();
    let worker = {
        let (cfg, path) = (cfg.clone(), path.clone());
        thread::spawn(move || {
            let mut words = 0usize;
            for job in rx {
                words += transcribe_piece(job, &cfg, &path, show);
            }
            words
        })
    };

    let piece = cfg.piece_seconds.clamp(10, 120) as f32;
    let (min, max) = (piece * 0.67, piece * 1.5);
    let started = Instant::now();
    let mut last_speech = Instant::now();
    let mut stopped_quietly = false;
    while !platform::stop_requested() {
        thread::sleep(Duration::from_millis(500));
        for (label, rec, taken, wav) in streams.iter_mut() {
            while let Some(clip) = rec.take_piece(min, max) {
                if let Some(w) = wav {
                    w.append(&clip.samples);
                }
                let secs = clip.seconds();
                let at = *taken;
                *taken += secs;
                if clip.has_speech() {
                    last_speech = Instant::now();
                    let _ = tx.send(Job { clip, label, at });
                } else if show {
                    println!("  [{}] {:.0} s without speech skipped", clock(at), secs);
                }
            }
        }
        // Forgotten session: no speech on any stream for a long time.
        if last_speech.elapsed().as_secs_f32() > QUIET_STOP {
            stopped_quietly = true;
            break;
        }
    }

    // Finish: the rest of each stream, then the queue.
    if show {
        println!("\nStopping: transcribing the last piece...");
    }
    for (label, rec, taken, wav) in streams {
        let clip = rec.stop();
        if let Some(mut w) = wav {
            w.append(&clip.samples);
            if let Err(e) = w.finish() {
                eprintln!("could not save the session audio: {e:#}");
            }
        }
        if clip.seconds() >= 0.4 && clip.has_speech() {
            let _ = tx.send(Job { clip, label, at: taken });
        }
    }
    drop(tx);
    let words = worker.join().unwrap_or(0);
    let minutes = started.elapsed().as_secs_f32() / 60.0;

    // Notes.
    let mut summary = format!("{minutes:.0} min, {words} words");
    if let Some(kind) = opts.notes {
        if words >= 10 {
            if show {
                println!("Writing {} ...", kind.name());
            }
            match notes::write_for_file(&path, kind) {
                Ok(s) => summary.push_str(&format!(", {s}")),
                Err(e) => {
                    summary.push_str(&format!(", notes failed: {e:#}"));
                    eprintln!("notes failed: {e:#}");
                }
            }
        } else {
            summary.push_str(", too little speech for notes");
        }
    }
    let _ = std::fs::remove_file(platform::runtime_file(STATE_FILE)?);
    let why = if stopped_quietly { "Stopped after 30 minutes without speech. " } else { "" };
    println!("{why}Session saved ({summary}): {}", path.display());
    drop(_lock); // `vtt session stop` returns now; only the audio question is left
    if audio_files.is_empty() {
        notify(&format!("{why}Saved ({summary}):\n{}", path.display()));
    } else {
        ask_delete_audio(&path, &audio_files, &cfg, &format!("{why}Saved ({summary})."));
    }
    Ok(())
}

// ---- kept session audio ----

/// `<folder>/.audio/<transcript name>.<others|you|audio>.wav`
fn audio_path(transcript: &Path, label: &str) -> Result<PathBuf> {
    let dir = transcript.parent().context("bad transcript path")?.join(".audio");
    std::fs::create_dir_all(&dir)?;
    let stem = transcript.file_stem().unwrap_or_default().to_string_lossy();
    let part = if label.is_empty() { "audio".to_string() } else { label.to_lowercase() };
    Ok(dir.join(format!("{stem}.{part}.wav")))
}

/// Kept audio files of a transcript, with their labels.
fn audio_files_of(transcript: &Path) -> Vec<(PathBuf, &'static str)> {
    let Some(dir) = transcript.parent().map(|d| d.join(".audio")) else { return Vec::new() };
    let stem = transcript.file_stem().unwrap_or_default().to_string_lossy().to_string();
    ["others", "you", "audio"]
        .iter()
        .map(|part| {
            let label = match *part {
                "others" => "Others",
                "you" => "You",
                _ => "",
            };
            (dir.join(format!("{stem}.{part}.wav")), label)
        })
        .filter(|(p, _)| p.exists())
        .collect()
}

/// After a session with kept audio: a notification asks whether to delete it.
fn ask_delete_audio(transcript: &Path, files: &[PathBuf], cfg: &Config, saved: &str) {
    let mb: u64 = files.iter().filter_map(|f| std::fs::metadata(f).ok()).map(|m| m.len()).sum::<u64>() / 1_000_000;
    let body = format!(
        "{saved}\n{}\nIs the transcript OK? The audio ({mb} MB) is only needed to redo it \
         (`vtt session redo`). Otherwise it is deleted after {} days.",
        transcript.display(),
        cfg.session_audio_days
    );
    let (tx, rx) = mpsc::channel();
    let shown = notify_rust::Notification::new()
        .summary("vtt session: delete the audio?")
        .body(&body)
        .action("delete", "Delete audio")
        .action("keep", "Keep for now")
        .timeout(notify_rust::Timeout::Never)
        .show();
    let Ok(handle) = shown else { return };
    thread::spawn(move || {
        handle.wait_for_action(|action| {
            let _ = tx.send(action.to_string());
        })
    });
    // Wait for an answer, at most an hour; no answer keeps the audio until it expires.
    if let Ok(answer) = rx.recv_timeout(Duration::from_secs(3600)) {
        if answer == "delete" {
            for f in files {
                let _ = std::fs::remove_file(f);
            }
            println!("Session audio deleted.");
        } else {
            println!("Session audio kept.");
        }
    }
}

/// Delete kept session audio older than `session_audio_days`.
pub fn cleanup_old_audio(cfg: &Config) {
    let Ok(dir) = session_dir(cfg).map(|d| d.join(".audio")) else { return };
    let max_age = Duration::from_secs(cfg.session_audio_days.max(1) * 86_400);
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > max_age);
        if old && p.extension().is_some_and(|x| x == "wav") {
            let _ = std::fs::remove_file(&p);
        }
    }
}

/// `vtt session delete-audio [FILE]`: delete the kept audio of one transcript, or all.
fn delete_audio_cmd(args: &[String]) -> Result<()> {
    let files: Vec<PathBuf> = match args.first() {
        Some(f) => audio_files_of(Path::new(f)).into_iter().map(|(p, _)| p).collect(),
        None => {
            let cfg = config::load()?;
            let dir = session_dir(&cfg)?.join(".audio");
            std::fs::read_dir(&dir)
                .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "wav")).collect())
                .unwrap_or_default()
        }
    };
    for f in &files {
        std::fs::remove_file(f)?;
    }
    println!("{} audio file(s) deleted.", files.len());
    Ok(())
}

/// `vtt session redo FILE [--notes TYPE]`: transcribe a session again from its kept audio
/// (e.g. after changing the model or language), then write the notes again.
fn redo(args: &[String]) -> Result<()> {
    let file = args.first().context("usage: vtt session redo FILE [--notes lecture|meeting|summary|none]")?;
    let path = PathBuf::from(file);
    let cfg = config::load()?;
    let audio = audio_files_of(&path);
    if audio.is_empty() {
        bail!("no kept audio for this transcript (session_keep_audio was off, or it was deleted)");
    }
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let head_end = text
        .find(notes::TRANSCRIPT_HEADING)
        .map(|i| i + notes::TRANSCRIPT_HEADING.len())
        .unwrap_or(0);
    let both = audio.len() > 1;
    let notes_kind = match args.iter().position(|a| a == "--notes").and_then(|i| args.get(i + 1)) {
        Some(k) if k == "none" => None,
        Some(k) => Some(Kind::parse(k).with_context(|| format!("unknown notes type `{k}`"))?),
        None => Some(if both { Kind::Meeting } else { Kind::Lecture }),
    };

    // Cut every stream into pieces offline, then transcribe in time order.
    let piece = cfg.piece_seconds.clamp(10, 120) as f32;
    let (min, max) = (piece * 0.67, piece * 1.5);
    let mut jobs = Vec::new();
    for (file, label) in &audio {
        let bytes = std::fs::read(file)?;
        let mut buf: Vec<f32> = bytes
            .get(44..)
            .unwrap_or_default()
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
            .collect();
        let mut at = 0.0f32;
        while let Some(clip) = audio::cut_piece(&mut buf, min, max) {
            let secs = clip.seconds();
            if clip.has_speech() {
                jobs.push(Job { clip, label, at });
            }
            at += secs;
        }
        let rest = audio::Clip { samples: buf };
        if rest.seconds() >= 0.4 && rest.has_speech() {
            jobs.push(Job { clip: rest, label, at });
        }
    }
    jobs.sort_by(|a, b| a.at.total_cmp(&b.at));
    let a = cfg.active();
    println!("Transcribing {} piece(s) again with {} {} ...", jobs.len(), a.provider.name, a.model);
    platform::write_private(&path, &format!("{}

", &text[..head_end]))?;
    let mut words = 0;
    for job in jobs {
        words += transcribe_piece(job, &cfg, &path, true);
    }
    if let Some(kind) = notes_kind {
        if words >= 10 {
            println!("Writing {} ...", kind.name());
            println!("{}", notes::write_for_file(&path, kind)?);
        }
    }
    println!("Done: {}", path.display());
    Ok(())
}

/// A 16 kHz mono WAV file written piece by piece; the header is completed at the end.
struct WavWriter {
    file: std::fs::File,
    samples: u32,
}

impl WavWriter {
    fn create(path: &Path) -> Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600) // contains voices
            .open(path)
            .with_context(|| format!("creating {}", path.display()))?;
        file.write_all(&wav_header(0))?;
        Ok(Self { file, samples: 0 })
    }

    fn append(&mut self, samples: &[f32]) {
        let bytes: Vec<u8> = samples
            .iter()
            .flat_map(|s| ((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes())
            .collect();
        if self.file.write_all(&bytes).is_ok() {
            self.samples += samples.len() as u32;
        }
    }

    fn finish(mut self) -> Result<()> {
        use std::io::{Seek, SeekFrom};
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&wav_header(self.samples))?;
        Ok(())
    }
}

fn wav_header(samples: u32) -> Vec<u8> {
    let rate = audio::TARGET_RATE;
    let data = samples * 2;
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(36 + data).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&(rate * 2).to_le_bytes());
    h.extend_from_slice(&2u16.to_le_bytes());
    h.extend_from_slice(&16u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data.to_le_bytes());
    h
}

/// Transcribe one piece and append it to the transcript. Returns the number of words.
fn transcribe_piece(job: Job, cfg: &Config, path: &Path, show: bool) -> usize {
    let Job { mut clip, label, at } = job;
    clip.trim_silence();
    clip.shorten_pauses();
    let a = cfg.active();
    let phrases = cfg.phrases();
    let wav = clip.to_wav();
    let line = match stt::transcribe(&a, &cfg.language, &phrases.join(", "), &wav) {
        Ok(t) => {
            let _ = usage::record(a.provider.id, &a.model, clip.seconds(), t.cost);
            let mut text = t.text;
            if text.is_empty() || crate::is_hallucination(&text) {
                return 0;
            }
            if cfg.correct_text && text.split_whitespace().count() >= 2 && !a.correction_model.is_empty() {
                if let Ok(c) = correct::correct(&a, &a.correction_model, &text, &phrases) {
                    let _ = usage::record(a.provider.id, &a.correction_model, 0.0, c.cost);
                    text = c.text;
                }
            }
            text
        }
        Err(e) => {
            let saved = crate::save_failed(&wav, (at as u32).max(1))
                .map(|p| format!(" Audio saved for `vtt retry`: {}", p.display()))
                .unwrap_or_default();
            eprintln!("piece at {} failed: {e:#}", clock(at));
            format!("_[this part could not be transcribed: {e}.{saved}]_")
        }
    };
    let label = if label.is_empty() { String::new() } else { format!(" {label}:") };
    let entry = format!("**[{}]{label}** {line}\n\n", clock(at));
    if show {
        print!("{entry}");
    }
    let appended = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .and_then(|mut f| f.write_all(entry.as_bytes()));
    if let Err(e) = appended {
        eprintln!("could not write to {}: {e}", path.display());
    }
    line.split_whitespace().count()
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options() {
        let cfg = Config::default();
        let o = parse_options(&["Physics".into(), "lecture".into(), "3".into()], &cfg).unwrap();
        assert_eq!((o.source, o.title.as_str(), o.notes), (Source::System, "Physics lecture 3", Some(Kind::Lecture)));
        let o = parse_options(&["--source".into(), "both".into()], &cfg).unwrap();
        assert_eq!((o.source, o.title.as_str(), o.notes), (Source::Both, "Meeting", Some(Kind::Meeting)));
        let o = parse_options(&["-s".into(), "mic".into(), "-n".into(), "none".into()], &cfg).unwrap();
        assert_eq!((o.source, o.notes), (Source::Mic, None));
        assert!(parse_options(&["--source".into(), "radio".into()], &cfg).is_err());
    }

    #[test]
    fn kept_audio_round_trip() {
        let dir = std::env::temp_dir().join(format!("vtt-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let transcript = dir.join("2026-11-14 10-00 Call.md");
        let p = audio_path(&transcript, "Others").unwrap();
        let mut w = WavWriter::create(&p).unwrap();
        w.append(&[0.5; 16_000]);
        w.append(&[0.0; 8_000]);
        w.finish().unwrap();
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(bytes.len(), 44 + 24_000 * 2);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 48_000);
        let found = audio_files_of(&transcript);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1, "Others");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn names_and_clock() {
        assert_eq!(safe_name("Math: 1/2 \"intro\""), "Math- 1-2 -intro-");
        assert_eq!(clock(3725.4), "01:02:05");
    }
}
