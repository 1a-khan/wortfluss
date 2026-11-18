//! Session mode: transcribe what the computer plays (lectures, tutorials, the other side
//! of a call) and/or the microphone into a Markdown file, live, then write notes.
//!
//! Each source is recorded as its own stream, cut into ~30 s pieces at pauses; pieces
//! without speech are skipped. With both sources the lines are labelled "You" (mic) and
//! "Others" (system audio). Only text is kept, never audio (except failed pieces until
//! `vtt retry` or when keeping the audio is switched on).
//!
//! With speaker separation on, the voices are labelled "Speaker 1", "Speaker 2" ... (see
//! speakers.rs); in a call only the other side, since "You" is already known.

use crate::config::{self, Config};
use crate::mics::{self, SourceRecorder};
use crate::notes::{self, Kind};
use crate::speakers::{self, Tracker};
use crate::{audio, correct, platform, stt, usage};
use anyhow::{bail, Context, Result};
use std::io::{IsTerminal, Write};
use std::collections::HashMap;
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

    /// Key of the translated description (notifications).
    pub fn text_key(self) -> &'static str {
        match self {
            Source::System => "source.system",
            Source::Mic => "source.mic",
            Source::Both => "source.both",
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
    /// Restart after a crash: continue this transcript (set by the supervisor).
    continue_file: Option<PathBuf>,
    /// Seconds since the session started, for the timestamps after a restart.
    offset: f32,
    /// Separate speakers.
    speakers: bool,
}

fn parse_options(args: &[String], cfg: &Config) -> Result<Options> {
    let mut source = Source::parse(&cfg.session_source).unwrap_or(Source::System);
    let mut notes_arg: Option<String> = None;
    let mut from = None;
    let mut dir = None;
    let mut keep_audio = cfg.session_keep_audio;
    let mut continue_file = None;
    let mut offset = 0.0f32;
    let mut speakers = cfg.session_speakers;
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
            "--speakers" => speakers = true,
            "--no-speakers" => speakers = false,
            "--continue" => continue_file = Some(PathBuf::from(it.next().context("--continue needs a file")?)),
            "--offset" => offset = it.next().and_then(|v| v.parse().ok()).context("--offset needs seconds")?,
            other => title.push(other.to_string()),
        }
    }
    let notes_setting = notes_arg.unwrap_or_else(|| cfg.session_notes.clone());
    let notes = match notes_setting.trim().to_lowercase().as_str() {
        "none" | "off" | "no" => None,
        "" | "auto" => Some(if source == Source::Both || speakers { Kind::Meeting } else { Kind::Lecture }),
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
    Ok(Options { source, title, notes, from, dir, keep_audio, continue_file, offset, speakers })
}

/// `vtt session [start|run|stop|status] ...`
pub fn cmd(args: &[String]) -> Result<()> {
    let rest = &args[args.len().min(1)..];
    match args.first().map(String::as_str) {
        Some("start") => start(rest),
        Some("run") => run_supervised(rest),
        Some("worker") => run(rest),
        Some("ask-audio") => {
            let file = PathBuf::from(rest.first().context("ask-audio needs a transcript")?);
            let files: Vec<PathBuf> = audio_files_of(&file).into_iter().map(|(p, _)| p).collect();
            ask_delete_audio(&file, &files, &config::load()?, "Session saved.");
            Ok(())
        }
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
    let Some(pid) = platform::running_pid_named(LOCK) else {
        println!("No session is running.");
        return Ok(());
    };
    println!("Stopping: transcribing the last piece and writing notes (can take up to a minute)...");
    platform::request_stop(pid)?;
    // Done when the transcript is complete (state file gone) or the process has ended.
    let until = Instant::now() + Duration::from_secs(300);
    while current_file().is_some() && platform::running_pid_named(LOCK).is_some() {
        if Instant::now() > until {
            bail!("the session did not stop within 5 minutes (process {pid})");
        }
        thread::sleep(Duration::from_millis(200));
    }
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

/// Is a session running? (for the session hotkey and the tray)
pub fn is_running() -> bool {
    platform::running_pid_named(LOCK).is_some()
}

/// The running session's transcript and when it started (unix seconds).
fn read_state() -> Option<(PathBuf, u64)> {
    let text = std::fs::read_to_string(platform::runtime_file(STATE_FILE).ok()?).ok()?;
    let mut lines = text.lines();
    let p = PathBuf::from(lines.next()?.trim());
    let started = lines.next().and_then(|l| l.trim().parse().ok()).unwrap_or(0);
    p.exists().then_some((p, started))
}

fn current_file() -> Option<PathBuf> {
    read_state().map(|s| s.0)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `vtt session run`: supervise the session worker (OTP-style, transient): after a crash
/// it is restarted and continues the same transcript; a normal end finishes the session.
fn run_supervised(args: &[String]) -> Result<()> {
    let cfg = config::load()?;
    if let Some(p) = cfg.problem() {
        bail!("vtt isn't set up yet ({p}). Run `vtt setup`");
    }
    parse_options(args, &cfg)?; // fail early on bad arguments
    let Some(_lock) = platform::lock_named(LOCK)? else {
        bail!("a session is already running: `vtt session status`");
    };
    let _ = std::fs::remove_file(platform::runtime_file(STATE_FILE)?); // stale from a crash
    let base: Vec<String> = args.to_vec();
    let result = crate::supervisor::supervise(
        &crate::supervisor::Policy {
            name: "The session",
            max_restarts: 3,
            within: Duration::from_secs(300),
            grace: Duration::from_secs(300), // time to transcribe the rest and write notes
            retry_after: None,
        },
        |restart| {
            let mut a = vec!["session".to_string(), "worker".to_string()];
            a.extend(base.iter().cloned());
            if restart > 0 {
                if let Some((path, started)) = read_state() {
                    let offset = unix_now().saturating_sub(started);
                    a.extend(["--continue".into(), path.to_string_lossy().into(), "--offset".into(), offset.to_string()]);
                }
            }
            a
        },
    );
    let _ = std::fs::remove_file(platform::runtime_file(STATE_FILE)?);
    result.map(|_| ())
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
    if cfg!(test) {
        return; // no desktop notifications from tests
    }
    let _ = crate::notification().summary(crate::i18n::t("session.title")).body(msg).timeout(8000).show();
}

/// A piece of one stream waiting to be transcribed.
struct Job {
    clip: audio::Clip,
    label: &'static str,
    /// Seconds since the session started.
    at: f32,
}

/// One recorded source of the session.
struct Stream {
    label: &'static str,
    /// Sound-server source / device name, to open it again.
    name: String,
    /// None while the stream is broken and being reopened.
    rec: Option<SourceRecorder>,
    /// Seconds of the session this stream covers (audio taken plus gaps).
    taken: f32,
    /// The kept audio (session_keep_audio).
    wav: Option<WavWriter>,
    /// Broken since, and whether the user was told.
    broken: Option<(Instant, bool)>,
    last_try: Instant,
}

impl Stream {
    /// Queue a recorded piece for transcription (if there is speech in it). True if
    /// there was speech.
    fn hand_over(&mut self, clip: audio::Clip, tx: &mpsc::Sender<Job>, show: bool) -> bool {
        if let Some(w) = &mut self.wav {
            w.append(&clip.samples);
        }
        let (secs, at) = (clip.seconds(), self.taken);
        self.taken += secs;
        if clip.has_speech() {
            let _ = tx.send(Job { clip, label: self.label, at });
            true
        } else {
            if show {
                println!("  [{}] {:.0} s without speech skipped", clock(at), secs);
            }
            false
        }
    }

    /// A broken stream: try to open it again every 2 s; tell the user if that takes
    /// longer than 10 s, and again once it works.
    fn reopen(&mut self) {
        if self.rec.is_some() || self.last_try.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_try = Instant::now();
        match SourceRecorder::start(&self.name) {
            Ok(rec) => {
                let (since, told) = self.broken.take().unwrap_or((Instant::now(), false));
                // The gap counts, so the times of later pieces stay right.
                self.taken += since.elapsed().as_secs_f32();
                eprintln!("recording of {} works again after {:.0} s", self.name, since.elapsed().as_secs_f32());
                if told {
                    notify(crate::i18n::t("session.rec_back"));
                }
                self.rec = Some(rec);
            }
            Err(e) => {
                if let Some((since, told)) = &mut self.broken {
                    if !*told && since.elapsed() > Duration::from_secs(10) {
                        *told = true;
                        eprintln!("cannot record {} again yet: {e:#}", self.name);
                        notify(crate::i18n::t("session.rec_lost"));
                    }
                }
            }
        }
    }
}

/// Pieces that could not be transcribed yet. They are in the transcript as a marker
/// with the path of their kept audio; while the service is unreachable new pieces are
/// not sent but marked right away, and every 5 s, growing to 60 s, the oldest marked
/// piece is tried. Once one works, all markers are filled in, in order.
struct CatchUp {
    pending: usize,
    offline: bool,
    next: Instant,
    wait: Duration,
    told: bool,
}

const FIRST_WAIT: Duration = Duration::from_secs(5);
const LONGEST_WAIT: Duration = Duration::from_secs(60);

impl CatchUp {
    /// Markers already in the file count (a session continued after a restart).
    fn new(path: &Path) -> Self {
        let pending = std::fs::read_to_string(path).map(|t| pending_parts(&t).len()).unwrap_or(0);
        CatchUp { pending, offline: false, next: Instant::now(), wait: FIRST_WAIT, told: false }
    }

    /// How long the worker may wait for the next piece.
    fn wait(&self) -> Duration {
        if self.pending == 0 {
            Duration::from_secs(3600)
        } else {
            self.next.saturating_duration_since(Instant::now())
        }
    }

    /// Don't send new pieces: the service is unreachable and the next try isn't due.
    fn offline(&self) -> bool {
        self.offline && Instant::now() < self.next
    }

    fn due(&self) -> bool {
        self.pending > 0 && Instant::now() >= self.next
    }

    fn worked(&mut self) {
        self.offline = false;
        if self.pending > 0 {
            self.next = Instant::now(); // it works again: catch up right away
        }
    }

    fn failed(&mut self, reason: &str) {
        self.pending += 1;
        if !self.offline {
            self.offline = true;
            self.wait = FIRST_WAIT;
            self.next = Instant::now() + self.wait;
            if !self.told {
                self.told = true;
                notify(&crate::i18n::tf("session.offline", &[reason]));
            }
        }
    }

    /// Try the marked pieces in order, stopping at the first that still fails.
    /// Returns the words filled in.
    fn run(&mut self, cfg: &Config, path: &Path, show: bool) -> usize {
        let done = fill_pending(path, cfg, show, true);
        self.pending = done.left;
        if done.left == 0 {
            if self.told && done.filled > 0 {
                notify(&crate::i18n::tf("session.back", &[&done.filled.to_string()]));
            }
            self.told = false;
            self.offline = false;
            self.wait = FIRST_WAIT;
        } else {
            self.offline = done.filled == 0;
            self.wait = if done.filled > 0 { FIRST_WAIT } else { (self.wait * 2).min(LONGEST_WAIT) };
            self.next = Instant::now() + self.wait;
        }
        done.words
    }
}

/// Marks a piece whose audio is kept for a later try (`vtt retry`, or the session
/// itself once the service works again). The older form is understood too.
const KEPT: &str = "audio kept for `vtt retry`: ";
const KEPT_OLD: &str = "Audio saved for `vtt retry`: ";

/// A marked piece in a transcript.
struct Part {
    /// The whole line, without its line break.
    line: String,
    /// "**[12:30] You:** ", what stays in front of the text.
    head: String,
    wav: PathBuf,
}

fn pending_parts(text: &str) -> Vec<Part> {
    let mut parts = Vec::new();
    for line in text.lines() {
        for tag in [KEPT, KEPT_OLD] {
            let Some(i) = line.find(tag) else { continue };
            let rest = &line[i + tag.len()..];
            let (Some(end), Some(start)) = (rest.rfind("]_"), line[..i].rfind("_[")) else { continue };
            parts.push(Part { line: line.to_string(), head: line[..start].to_string(), wav: PathBuf::from(&rest[..end]) });
            break;
        }
    }
    parts
}

struct Filled {
    filled: usize,
    left: usize,
    words: usize,
}

/// Transcribe the marked pieces of a transcript from their kept audio and put the text
/// in their place. `stop_at_failure`: stop at the first that still fails (no
/// connection: the rest would fail too).
fn fill_pending(path: &Path, cfg: &Config, show: bool, stop_at_failure: bool) -> Filled {
    let mut done = Filled { filled: 0, left: 0, words: 0 };
    let Ok(mut text) = std::fs::read_to_string(path) else { return done };
    let parts = pending_parts(&text);
    let a = cfg.active();
    let phrases = cfg.phrases();
    for (i, p) in parts.iter().enumerate() {
        let Ok(wav) = std::fs::read(&p.wav) else {
            done.left += 1; // the audio is gone; the marker stays
            continue;
        };
        match stt::transcribe(&a, &cfg.language, &phrases.join(", "), &wav) {
            Ok(t) => {
                let secs = wav.len().saturating_sub(44) as f32 / (audio::TARGET_RATE as f32 * 2.0);
                let _ = usage::record(a.provider.id, &a.model, secs, t.cost);
                if t.text.is_empty() || crate::is_hallucination(&t.text) {
                    text = text.replacen(&format!("{}\n\n", p.line), "", 1);
                } else {
                    let new = corrected(cfg, &a, t.text, &phrases);
                    done.words += new.split_whitespace().count();
                    if show {
                        println!("{}{new}  (filled in)\n", p.head);
                    }
                    text = text.replacen(&p.line, &format!("{}{new}", p.head), 1);
                }
                let _ = std::fs::remove_file(&p.wav);
                done.filled += 1;
            }
            Err(e) => {
                eprintln!("{} still not transcribed: {e:#}", p.wav.display());
                done.left += 1;
                if stop_at_failure {
                    done.left += parts.len() - i - 1;
                    break;
                }
            }
        }
    }
    if done.filled > 0 {
        // Written next to it and renamed: a crash can't leave half a transcript.
        let tmp = path.with_extension("md.part");
        let saved = platform::write_private(&tmp, &text).and_then(|_| Ok(std::fs::rename(&tmp, path)?));
        if let Err(e) = saved {
            eprintln!("could not update {}: {e:#}", path.display());
        }
    }
    done
}

/// `vtt retry FILE`: fill in the parts of a transcript that could not be transcribed.
pub fn retry_file(path: &Path) -> Result<()> {
    let cfg = config::load()?;
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let n = pending_parts(&text).len();
    if n == 0 {
        println!("{}: nothing to fill in.", path.display());
        return Ok(());
    }
    println!("{}: {n} part(s) to fill in ...", path.display());
    let done = fill_pending(path, &cfg, true, false);
    println!("{} filled in, {} still missing.", done.filled, done.left);
    Ok(())
}

/// Transcripts in the session folder with parts still to fill in.
pub fn transcripts_with_gaps() -> Vec<PathBuf> {
    let Ok(dir) = config::load().and_then(|c| session_dir(&c)) else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .filter(|p| std::fs::read_to_string(p).is_ok_and(|t| !pending_parts(&t).is_empty()))
        .collect();
    found.sort();
    found
}

/// Run a session in this process until Ctrl+C / `vtt session stop`.
pub fn run(args: &[String]) -> Result<()> {
    let cfg = config::load()?;
    if let Some(p) = cfg.problem() {
        bail!("vtt isn't set up yet ({p}). Run `vtt setup`");
    }
    let mut opts = parse_options(args, &cfg)?;
    let speaker_warning = if opts.speakers && cfg.active().speaker_model.is_empty() {
        opts.speakers = false;
        Some(crate::i18n::tf(
            "session.no_speakers",
            &[cfg.active().provider.name]
        ))
    } else {
        None
    };
    // Under the session supervisor the lock is held there; standalone we take it here.
    let _lock = if crate::supervisor::is_supervised() {
        None
    } else {
        match platform::lock_named(LOCK)? {
            Some(l) => Some(l),
            None => bail!("a session is already running: `vtt session status`"),
        }
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
    let path = match &opts.continue_file {
        Some(p) => p.clone(),
        None => dir.join(format!("{y}-{mo:02}-{d:02} {h:02}-{mi:02} {}.md", safe_name(&opts.title))),
    };
    let a = cfg.active();
    let header = format!(
        "# {}\n\n_{y}-{mo:02}-{d:02} {h:02}:{mi:02} · {} · {} {}{}_\n\n{}\n{}\n{}\n\n{}\n\n",
        opts.title,
        opts.source.describe(),
        a.provider.name,
        if opts.speakers { &a.speaker_model } else { &a.model },
        if opts.speakers { " · speakers separated (rename: `vtt speakers FILE`)" } else { "" },
        notes::START,
        match opts.notes {
            Some(k) => format!("_{} will appear here when the session ends._", capitalize(k.name())),
            None => "_No notes requested. Create them later: `vtt notes FILE lecture|meeting|summary`._".to_string(),
        },
        notes::END,
        notes::TRANSCRIPT_HEADING,
    );
    if opts.continue_file.is_some() {
        let note = format!(
            "_[{}] The session was restarted after an error; up to about 30 seconds before this may be missing._\n\n",
            clock(opts.offset)
        );
        std::fs::OpenOptions::new().append(true).open(&path)?.write_all(note.as_bytes())?;
    } else {
        platform::write_private(&path, &header)?;
    }
    let started = unix_now().saturating_sub(opts.offset as u64);
    std::fs::write(
        platform::runtime_file(STATE_FILE)?,
        format!("{}\n{started}\n", path.to_string_lossy()),
    )?;
    let show = std::io::stdout().is_terminal();
    println!("Session started: {}\nTranscript: {}", opts.source.describe(), path.display());
    if show {
        println!("Press Ctrl+C to stop{}.\n", if opts.notes.is_some() { " and write the notes" } else { "" });
    }
    let no_headphones = opts.source == Source::Both && mics::output_is_headphones() == Some(false);
    notify(&crate::i18n::tf(
        "session.started",
        &[
            crate::i18n::t(opts.source.text_key()),
            if no_headphones { crate::i18n::t("session.headphones") } else { "" },
        ],
    ));
    if no_headphones {
        println!("Tip: use headphones; with loudspeakers the other side may be transcribed twice.");
    }
    if let Some(w) = &speaker_warning {
        println!("Note: {w}");
        notify(w);
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
            Some(if opts.continue_file.is_some() { WavWriter::append_to(&p)? } else { WavWriter::create(&p)? })
        } else {
            None
        };
        streams.push(Stream { label, name, rec: Some(rec), taken: opts.offset, wav, broken: None, last_try: Instant::now() });
    }

    // One worker transcribes pieces in order and appends them to the file. Pieces that
    // can't be transcribed (no internet, service down) get a marker and their audio is
    // kept; they are filled in as soon as it works again (see CatchUp).
    let (tx, rx) = mpsc::channel::<Job>();
    let worker = {
        let (cfg, path) = (cfg.clone(), path.clone());
        let speakers = opts.speakers;
        // After a restart, new voices get numbers after the ones already in the file.
        let first = std::fs::read_to_string(&path).map(|t| speakers::last_number(&t)).unwrap_or(0);
        thread::spawn(move || {
            let mut trackers = Trackers::new(speakers, first);
            let mut words = 0usize;
            let mut catch_up = CatchUp::new(&path);
            loop {
                match rx.recv_timeout(catch_up.wait()) {
                    Ok(job) => {
                        let tracker = trackers.get(job.label);
                        match transcribe_piece(job, &cfg, &path, show, tracker, catch_up.offline()) {
                            Ok(w) => {
                                words += w;
                                catch_up.worked();
                            }
                            Err(reason) => catch_up.failed(&reason),
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
                if catch_up.due() {
                    words += catch_up.run(&cfg, &path, show);
                }
            }
            if catch_up.pending > 0 {
                words += catch_up.run(&cfg, &path, show); // a last try before the notes
            }
            (words, catch_up.pending)
        })
    };

    let (min, max) = piece_seconds(&cfg, opts.speakers);
    let started = Instant::now();
    let mut last_speech = Instant::now();
    let mut stopped_quietly = false;
    // Taken over from a supervisor that is gone, so `vtt session stop` still finds us.
    let mut own_lock = None;
    while !platform::stop_requested() {
        thread::sleep(Duration::from_millis(500));
        for s in streams.iter_mut() {
            if let Some(rec) = &s.rec {
                let mut clips = Vec::new();
                while let Some(clip) = rec.take_piece(min, max) {
                    clips.push(clip);
                }
                if !rec.is_alive() {
                    // The sound server ended the stream (restarted, device removed): keep
                    // what it recorded and open it again.
                    eprintln!("recording of {} ended; reopening it", s.name);
                    clips.push(s.rec.take().unwrap().stop());
                    s.broken = Some((Instant::now(), false));
                }
                for clip in clips {
                    if s.hand_over(clip, &tx, show) {
                        last_speech = Instant::now();
                    }
                }
            }
            s.reopen();
        }
        // Forgotten session: no speech on any stream for a long time.
        if last_speech.elapsed().as_secs_f32() > QUIET_STOP {
            stopped_quietly = true;
            break;
        }
        if own_lock.is_none() && crate::supervisor::supervisor_gone() {
            eprintln!("the session supervisor is gone; this session continues on its own");
            own_lock = platform::lock_named(LOCK).ok().flatten();
        }
    }

    // Finish: the rest of each stream, then the queue.
    if show {
        println!("\nStopping: transcribing the last piece...");
    }
    for mut s in streams {
        if let Some(rec) = s.rec.take() {
            let clip = rec.stop();
            if clip.seconds() >= 0.4 {
                s.hand_over(clip, &tx, show);
            }
        }
        if let Some(w) = s.wav.take() {
            if let Err(e) = w.finish() {
                eprintln!("could not save the session audio: {e:#}");
            }
        }
    }
    drop(tx);
    let (words, left_over) = worker.join().unwrap_or((0, 0));
    let minutes = started.elapsed().as_secs_f32() / 60.0;

    // Notes.
    let mut summary = format!("{minutes:.0} min, {words} words");
    if left_over > 0 {
        summary.push_str(&format!(", {left_over} part(s) not transcribed yet: `vtt retry FILE`"));
    }
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
    drop(_lock);
    if audio_files.is_empty() {
        let why = if stopped_quietly { crate::i18n::t("session.stopped_quiet") } else { "" };
        notify(&crate::i18n::tf("session.saved", &[why, &summary, &path.display().to_string()]));
    } else {
        // Asked by a separate small process, so this session ends now.
        let p = path.to_string_lossy().to_string();
        if platform::start_background_with(&["session", "ask-audio", &p]).is_err() {
            ask_delete_audio(&path, &audio_files, &cfg, &format!("{why}Saved ({summary})."));
        }
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
    let answer = ask_delete(&body);
    if let Some(answer) = answer {
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

/// Windows: a small dialog (Yes = delete). No answer within the hour keeps the audio.
#[cfg(windows)]
fn ask_delete(body: &str) -> Option<String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONQUESTION, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO};
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let text = wide(&format!("{body}\n\n{}", crate::i18n::t("session.delete_now")));
    let title = wide(crate::i18n::t("session.delete_title"));
    let r = unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_YESNO | MB_ICONQUESTION | MB_TOPMOST | MB_SETFOREGROUND)
    };
    Some(if r == IDYES { "delete" } else { "keep" }.to_string())
}

/// Linux: a notification with two buttons. No answer within the hour keeps the audio.
#[cfg(not(windows))]
fn ask_delete(body: &str) -> Option<String> {
    let (tx, rx) = mpsc::channel();
    let shown = crate::notification()
        .summary(crate::i18n::t("session.delete_title"))
        .body(body)
        .action("delete", crate::i18n::t("session.delete"))
        .action("keep", crate::i18n::t("session.keep"))
        .timeout(notify_rust::Timeout::Never)
        .show();
    let handle = shown.ok()?;
    thread::spawn(move || {
        handle.wait_for_action(|action| {
            let _ = tx.send(action.to_string());
        })
    });
    rx.recv_timeout(Duration::from_secs(3600)).ok()
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
    let file = args
        .first()
        .context("usage: vtt session redo FILE [--notes lecture|meeting|summary|none] [--speakers|--no-speakers]")?;
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
    let mut speakers_on = cfg.session_speakers || args.iter().any(|a| a == "--speakers");
    if args.iter().any(|a| a == "--no-speakers") {
        speakers_on = false;
    }
    if speakers_on && cfg.active().speaker_model.is_empty() {
        println!("{} can't separate speakers; transcribing without.", cfg.active().provider.name);
        speakers_on = false;
    }

    // Cut every stream into pieces offline, then transcribe in time order.
    let (min, max) = piece_seconds(&cfg, speakers_on);
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
    let mut trackers = Trackers::new(speakers_on, 0);
    for job in jobs {
        let tracker = trackers.get(job.label);
        words += transcribe_piece(job, &cfg, &path, true, tracker, false).unwrap_or(0);
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
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600); // contains voices
        }
        let mut file = opts
            .open(path)
            .with_context(|| format!("creating {}", path.display()))?;
        file.write_all(&wav_header(0))?;
        Ok(Self { file, samples: 0 })
    }

    /// Continue an existing file (after a restart); a missing file is created.
    fn append_to(path: &Path) -> Result<Self> {
        use std::io::{Seek, SeekFrom};
        if !path.exists() {
            return Self::create(path);
        }
        let mut file = std::fs::OpenOptions::new().read(true).write(true).open(path)?;
        let len = file.metadata()?.len();
        let samples = (len.saturating_sub(44) / 2) as u32;
        file.seek(SeekFrom::End(0))?;
        Ok(Self { file, samples })
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

/// Piece length: longer with speaker separation, so the voice samples sent in front of
/// each piece are a small share of it.
fn piece_seconds(cfg: &Config, speakers: bool) -> (f32, f32) {
    let mut piece = cfg.piece_seconds.clamp(10, 120) as f32;
    if speakers {
        piece = piece.max(60.0);
    }
    (piece * 0.67, piece * 1.5)
}

/// One speaker tracker per stream that gets speaker separation ("You" never does).
struct Trackers {
    on: bool,
    first: usize,
    map: HashMap<&'static str, Tracker>,
}

impl Trackers {
    fn new(on: bool, first: usize) -> Self {
        Trackers { on, first, map: HashMap::new() }
    }

    fn get(&mut self, label: &'static str) -> Option<&mut Tracker> {
        if !self.on || label == "You" {
            return None;
        }
        let first = self.first;
        Some(self.map.entry(label).or_insert_with(|| Tracker::starting_after(first)))
    }
}

/// Correct a text if the setting is on (failures keep the text as it is).
fn corrected(cfg: &Config, a: &config::Active, text: String, phrases: &[String]) -> String {
    if cfg.correct_text && text.split_whitespace().count() >= 2 && !a.correction_model.is_empty() {
        if let Ok(c) = correct::correct(a, &a.correction_model, &text, phrases) {
            let _ = usage::record(a.provider.id, &a.correction_model, 0.0, c.cost);
            return c.text;
        }
    }
    text
}

/// Transcribe one piece and append it to the transcript. Returns the number of words,
/// or why it failed (then a marker is appended and the audio kept, see CatchUp).
/// `offline`: the service is unreachable right now, so the piece is only marked.
fn transcribe_piece(
    job: Job,
    cfg: &Config,
    path: &Path,
    show: bool,
    tracker: Option<&mut Tracker>,
    offline: bool,
) -> Result<usize, String> {
    let Job { mut clip, label, at } = job;
    clip.trim_silence();
    clip.shorten_pauses();
    let a = cfg.active();
    let phrases = cfg.phrases();
    let mut entries: Vec<(f32, String, String)> = Vec::new(); // (time, label, text)

    // With speaker separation; if that fails, the piece is transcribed without, so no
    // text is lost.
    let mut separated = false;
    let mut failure = None;
    if let Some(tracker) = tracker.filter(|_| !offline) {
        match tracker.transcribe(&a, &a.speaker_model, &cfg.language, &clip) {
            Ok((turns, cost)) => {
                let _ = usage::record(a.provider.id, &a.speaker_model, clip.seconds(), cost);
                for t in turns {
                    if t.text.is_empty() || crate::is_hallucination(&t.text) {
                        continue;
                    }
                    let who = if t.speaker.is_empty() { label.to_string() } else { t.speaker };
                    entries.push((at + t.at, who, corrected(cfg, &a, t.text, &phrases)));
                }
                separated = true;
            }
            Err(e) => eprintln!("speaker separation failed at {} ({e:#}); transcribing without", clock(at)),
        }
    }
    if !separated {
        let wav = clip.to_wav();
        let result = if offline {
            Err("waiting for the connection".to_string())
        } else {
            stt::transcribe(&a, &cfg.language, &phrases.join(", "), &wav).map_err(|e| format!("{e:#}"))
        };
        let line = match result {
            Ok(t) => {
                let _ = usage::record(a.provider.id, &a.model, clip.seconds(), t.cost);
                if t.text.is_empty() || crate::is_hallucination(&t.text) {
                    return Ok(0);
                }
                corrected(cfg, &a, t.text, &phrases)
            }
            Err(e) => {
                // One short line: the reason must not break the marker.
                let reason: String = e.replace(['\n', '\r'], " ").replace("]_", "]").replace("_[", "[").chars().take(160).collect();
                eprintln!("piece at {} not transcribed: {reason}", clock(at));
                let kept = crate::save_failed(&wav, (at as u32).max(1))
                    .map(|p| format!("; {KEPT}{}", p.display()))
                    .unwrap_or_default();
                failure = Some(reason.clone());
                format!("_[not transcribed yet ({reason}){kept}]_")
            }
        };
        entries.push((at, label.to_string(), line));
    }

    let mut out = String::new();
    let mut words = 0;
    for (time, who, text) in entries {
        let who = if who.is_empty() { String::new() } else { format!(" {who}:") };
        out.push_str(&format!("**[{}]{who}** {text}\n\n", clock(time)));
        words += text.split_whitespace().count();
    }
    if show {
        print!("{out}");
    }
    let appended = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .and_then(|mut f| f.write_all(out.as_bytes()));
    if let Err(e) = appended {
        eprintln!("could not write to {}: {e}", path.display());
    }
    match failure {
        Some(reason) => Err(reason),
        None => Ok(words),
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

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
        w.finish().unwrap();
        // after a restart the file is continued, not overwritten
        let mut w = WavWriter::append_to(&p).unwrap();
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
    fn markers_are_found() {
        let text = "# T\n\n**[00:00:10] You:** hello\n\n\
            **[00:00:40] Others:** _[not transcribed yet (no route to host); audio kept for `vtt retry`: /tmp/a b/1-040.wav]_\n\n\
            **[00:01:10]** _[this part could not be transcribed: HTTP 500. Audio saved for `vtt retry`: /tmp/2-070.wav]_\n\n\
            **[00:01:40]** _[this part could not be transcribed: HTTP 500.]_\n\n";
        let parts = pending_parts(text);
        assert_eq!(parts.len(), 2, "the last one has no audio");
        assert_eq!(parts[0].head, "**[00:00:40] Others:** ");
        assert_eq!(parts[0].wav, PathBuf::from("/tmp/a b/1-040.wav"));
        assert_eq!(parts[1].head, "**[00:01:10]** ");
        assert_eq!(parts[1].wav, PathBuf::from("/tmp/2-070.wav"));
    }

    /// A transcription server on localhost that can be switched off (the connection is
    /// dropped, like no internet) and on (answers with `text`).
    fn fake_server(up: Arc<std::sync::atomic::AtomicBool>, text: &'static str) -> String {
        use std::io::{BufRead, BufReader, Read};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut conn) = conn else { continue };
                if !up.load(std::sync::atomic::Ordering::SeqCst) {
                    continue; // dropped: connection reset
                }
                let mut reader = BufReader::new(conn.try_clone().unwrap());
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0; length];
                let _ = reader.read_exact(&mut body);
                let json = format!("{{\"text\":\"{text}\"}}");
                let _ = write!(
                    conn,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
                    json.len()
                );
            }
        });
        format!("http://{addr}")
    }

    /// No internet: the piece is marked and its audio kept; once the service answers
    /// again the marker is replaced by the text, in place, and the audio deleted.
    #[test]
    fn gaps_are_filled_once_the_service_is_back() {
        let dir = std::env::temp_dir().join(format!("vtt-gaps-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Kept audio and usage go to the state folder: not the user's.
        std::env::set_var("XDG_STATE_HOME", dir.join("state"));
        std::env::set_var("XDG_DATA_HOME", dir.join("data"));
        let up = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut cfg = Config::default();
        cfg.provider = "custom".into();
        cfg.custom.api_url = fake_server(up.clone(), "back online");
        cfg.custom.model = "whisper".into();
        let path = dir.join("t.md");
        std::fs::write(&path, "# T\n\n**[00:00:00]** before\n\n").unwrap();
        let speech = || audio::Clip {
            samples: (0..48_000).map(|i| ((i as f32) * 0.06).sin() * if (i / 4000) % 2 == 0 { 0.5 } else { 0.05 }).collect(),
        };

        let failed = transcribe_piece(Job { clip: speech(), label: "", at: 30.0 }, &cfg, &path, false, None, false);
        assert!(failed.is_err());
        let after_marked = transcribe_piece(Job { clip: speech(), label: "", at: 60.0 }, &cfg, &path, false, None, true);
        assert_eq!(after_marked, Err("waiting for the connection".to_string()), "offline: not even sent");
        let text = std::fs::read_to_string(&path).unwrap();
        let parts = pending_parts(&text);
        assert_eq!(parts.len(), 2, "{text}");
        assert!(parts.iter().all(|p| p.wav.exists()));

        // Still down: nothing changes, and it stops at the first failure.
        let done = fill_pending(&path, &cfg, false, true);
        assert_eq!((done.filled, done.left), (0, 2));

        up.store(true, std::sync::atomic::Ordering::SeqCst);
        let done = fill_pending(&path, &cfg, false, true);
        assert_eq!((done.filled, done.left, done.words), (2, 0, 4));
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "# T\n\n**[00:00:00]** before\n\n**[00:00:30]** back online\n\n**[00:01:00]** back online\n\n"
        );
        assert!(parts.iter().all(|p| !p.wav.exists()), "kept audio deleted");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn catch_up_waits_longer_while_offline() {
        let mut c = CatchUp { pending: 0, offline: false, next: Instant::now(), wait: FIRST_WAIT, told: true };
        assert!(c.wait() > Duration::from_secs(60), "nothing pending: no hurry");
        c.failed("no route to host");
        assert!(c.offline() && !c.due() && c.pending == 1);
        assert!(c.wait() <= FIRST_WAIT);
        c.worked(); // a new piece went through: catch up now
        assert!(!c.offline() && c.due());
    }

    #[test]
    fn names_and_clock() {
        assert_eq!(safe_name("Math: 1/2 \"intro\""), "Math- 1-2 -intro-");
        assert_eq!(clock(3725.4), "01:02:05");
    }
}
