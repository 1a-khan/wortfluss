//! Listening indicator: a small bubble with a live sound wave next to the text cursor
//! while Wortfluss listens, and a short "working" animation until the text is typed.
//!
//! No app can change another app's text cursor, so the bubble floats beside it:
//! Windows draws it itself (`windows/overlay.rs`); on GNOME Wayland normal apps can
//! neither place windows nor see the cursor, so a small GNOME Shell extension draws it
//! (`linux/overlay.rs`, `linux/gnome-extension/`). Without one, nothing is shown (the
//! tray icon and beeps still work).

use crate::audio::{self, Level};
use crate::overlay::Overlay;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Number of bars in the wave (the GNOME extension draws the same number).
pub const BARS: usize = 7;
const FRAME: Duration = Duration::from_millis(33);
/// The "working" animation gives up after this long, even if typing never finished.
const WORKING_MAX: Duration = Duration::from_secs(90);

enum Cmd {
    Listen(Level),
    Working,
    Hide,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Listening,
    Working,
}

/// Handle used by the dictation loop. Cheap to clone; does nothing when switched off.
#[derive(Clone)]
pub struct Indicator {
    tx: Option<Sender<Cmd>>,
}

impl Indicator {
    /// `busy`: pieces still being transcribed or typed; the working animation ends at 0.
    pub fn start(enabled: bool, busy: Arc<AtomicUsize>) -> Self {
        if !enabled {
            return Self { tx: None };
        }
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || run(rx, busy));
        Self { tx: Some(tx) }
    }

    pub fn listen(&self, level: Level) {
        self.send(Cmd::Listen(level));
    }

    /// Speech was sent off: show "working" until everything is typed.
    pub fn working(&self) {
        self.send(Cmd::Working);
    }

    pub fn hide(&self) {
        self.send(Cmd::Hide);
    }

    fn send(&self, cmd: Cmd) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(cmd);
        }
    }
}

fn run(rx: Receiver<Cmd>, busy: Arc<AtomicUsize>) {
    let mut overlay: Option<Overlay> = None;
    let mut state: Option<(Mode, Option<Level>)> = None;
    let mut since = Instant::now();
    let mut shown = 0.0f32; // smoothed level, 0..1
    loop {
        // Idle: wait for a command. Active: one frame at a time.
        let cmd = if state.is_some() {
            match rx.recv_timeout(FRAME) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match rx.recv() {
                Ok(c) => Some(c),
                Err(_) => return,
            }
        };
        match cmd {
            Some(Cmd::Listen(level)) => {
                if state.is_none() {
                    since = Instant::now();
                    shown = 0.0;
                }
                state = Some((Mode::Listening, Some(level)));
                if overlay.is_none() {
                    overlay = Overlay::open();
                }
            }
            Some(Cmd::Working) => {
                if state.is_none() && overlay.is_none() {
                    overlay = Overlay::open();
                }
                since = Instant::now();
                state = Some((Mode::Working, None));
            }
            Some(Cmd::Hide) => {
                state = None;
                if let Some(o) = &mut overlay {
                    o.hide();
                }
                continue;
            }
            None => {}
        }
        let Some((mode, level)) = &state else { continue };
        if *mode == Mode::Working && (busy.load(SeqCst) == 0 || since.elapsed() > WORKING_MAX) {
            state = None;
            if let Some(o) = &mut overlay {
                o.hide();
            }
            continue;
        }
        let raw = level.as_ref().map(|l| loudness(audio::read_level(l))).unwrap_or(0.0);
        // Rise quickly with the voice, fall back gently.
        shown += (raw - shown) * if raw > shown { 0.6 } else { 0.15 };
        match &mut overlay {
            Some(o) => {
                if !o.frame(*mode, shown, since.elapsed().as_secs_f32()) {
                    overlay = None; // gone (e.g. extension disabled); try again next time
                }
            }
            None => {}
        }
    }
}

/// `vtt _indicator-demo`: 4 s of a synthetic voice level, then 1.5 s "working" (no
/// microphone involved). For trying the bubble and for tests.
pub fn demo() -> anyhow::Result<()> {
    let busy = Arc::new(AtomicUsize::new(1));
    let ind = Indicator::start(true, busy.clone());
    let level: Level = Arc::default();
    ind.listen(level.clone());
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(4) {
        let t = start.elapsed().as_secs_f32();
        // syllables: loud-quiet at ~4 Hz, around -25 dB
        let rms = 0.06 * (0.5 + 0.5 * (t * 25.0).sin()).powi(2) + 0.002;
        level.store((rms as f32).to_bits(), std::sync::atomic::Ordering::Relaxed);
        thread::sleep(Duration::from_millis(20));
    }
    ind.working();
    thread::sleep(Duration::from_millis(1500));
    busy.store(0, SeqCst); // "typed": the bubble goes away by itself
    thread::sleep(Duration::from_millis(300));
    Ok(())
}

/// Microphone RMS to 0..1 on a decibel scale: -55 dB (room) .. -15 dB (loud voice).
pub fn loudness(rms: f32) -> f32 {
    if rms <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * rms.log10();
    ((db + 55.0) / 40.0).clamp(0.0, 1.0)
}

/// Bar heights 0..1 for one frame. `t`: seconds since the bubble appeared.
/// Kept in sync with `heights()` in the GNOME extension.
#[allow(dead_code)] // used by the Windows overlay
pub fn bars(mode: Mode, level: f32, t: f32) -> [f32; BARS] {
    let mut h = [0.0; BARS];
    let mid = (BARS - 1) as f32 / 2.0;
    for (i, v) in h.iter_mut().enumerate() {
        let x = i as f32;
        *v = match mode {
            Mode::Listening => {
                // A wave that is highest in the middle and moves with the voice.
                let envelope = 1.0 - ((x - mid).abs() / (mid + 1.0)).powf(1.5);
                let wobble = 0.65 + 0.35 * (t * 9.0 + x * 1.3).sin();
                0.18 + 0.82 * level * envelope * wobble
            }
            // A ripple running across small bars: "working on it".
            Mode::Working => 0.18 + 0.32 * (t * 7.0 - x * 0.9).sin().max(0.0),
        };
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loudness_scale() {
        assert_eq!(loudness(0.0), 0.0);
        assert!(loudness(0.0005) < 0.05); // -66 dB: silent room
        assert!(loudness(0.05) > 0.5); // -26 dB: speaking
        assert_eq!(loudness(1.0), 1.0);
    }

    #[test]
    fn bars_stay_in_range() {
        for mode in [Mode::Listening, Mode::Working] {
            for step in 0..200 {
                for level in [0.0, 0.5, 1.0] {
                    for h in bars(mode, level, step as f32 * 0.033) {
                        assert!((0.0..=1.0).contains(&h), "{mode:?} {h}");
                    }
                }
            }
        }
        // silent: small bars; loud: the middle bar grows
        let quiet = bars(Mode::Listening, 0.0, 0.0);
        let loud = bars(Mode::Listening, 1.0, 0.0);
        assert!(quiet.iter().all(|h| *h < 0.2));
        assert!(loud[BARS / 2] > 0.5);
    }
}
