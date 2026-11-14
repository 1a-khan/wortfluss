//! Raw key events on Linux: read straight from the keyboards in /dev/input (evdev).
//! Works on any desktop (GNOME/KDE/Wayland/X11) since it bypasses the compositor.
//!
//! Supervision (OTP-style, at thread level): a manager attaches a reader thread to every
//! keyboard and re-attaches it when its reader ends (keyboard unplugged, read error,
//! crash). It also picks up keyboards plugged in later.

use crate::typer::DEVICE_NAME;
use anyhow::{bail, Result};
pub use evdev::Key;
use evdev::InputEventKind;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const PERMISSION_HELP: &str = "no keyboard could be opened. Your user needs read access to \
    /dev/input (the `input` group). Run the one-time permission setup from README.md, \
    then log out and back in";

fn is_keyboard(dev: &evdev::Device, extra: Option<Key>) -> bool {
    dev.name() != Some(DEVICE_NAME)
        && dev.supported_keys().is_some_and(|k| {
            k.contains(Key::KEY_A) || k.contains(Key::KEY_F9) || extra.is_some_and(|e| k.contains(e))
        })
}

/// Number of keyboards vtt can read (0 = no permission).
pub fn keyboard_count() -> usize {
    evdev::enumerate().filter(|(_, d)| is_keyboard(d, None)).count()
}

/// Key events (key, 0 up / 1 down / 2 repeat) from all keyboards, also ones plugged in
/// later, until the receiver is dropped. `extra`: also read devices that only have this
/// key (e.g. a foot pedal used as the hotkey).
pub fn subscribe(extra: Option<Key>) -> Result<Receiver<(Key, i32)>> {
    let (tx, rx) = mpsc::channel();
    let watched: Arc<Mutex<HashSet<PathBuf>>> = Arc::default();
    let closed = Arc::new(AtomicBool::new(false));
    let attach = {
        let (watched, closed) = (watched.clone(), closed.clone());
        move || -> usize {
            let mut added = 0;
            for (path, dev) in evdev::enumerate() {
                if !is_keyboard(&dev, extra) || !watched.lock().unwrap().insert(path.clone()) {
                    continue;
                }
                let (tx, watched, closed) = (tx.clone(), watched.clone(), closed.clone());
                thread::spawn(move || {
                    // Removes the keyboard from the watched set however the reader ends,
                    // so the manager attaches it again.
                    struct Detach(Arc<Mutex<HashSet<PathBuf>>>, PathBuf);
                    impl Drop for Detach {
                        fn drop(&mut self) {
                            if let Ok(mut w) = self.0.lock() {
                                w.remove(&self.1);
                            }
                        }
                    }
                    let _detach = Detach(watched, path);
                    read_keyboard(dev, &tx, &closed);
                });
                added += 1;
            }
            added
        }
    };
    if attach() == 0 {
        bail!(PERMISSION_HELP);
    }
    thread::spawn(move || {
        while !closed.load(SeqCst) {
            thread::sleep(Duration::from_secs(3));
            let added = attach();
            if added > 0 && !closed.load(SeqCst) {
                eprintln!("keyboard(s) attached: {added}");
            }
        }
    });
    Ok(rx)
}

/// Read one keyboard until it fails, disappears, or nobody listens anymore.
fn read_keyboard(mut dev: evdev::Device, tx: &Sender<(Key, i32)>, closed: &AtomicBool) {
    loop {
        let events = match dev.fetch_events() {
            Ok(events) => events,
            Err(e) => {
                eprintln!("keyboard detached: {e}");
                return;
            }
        };
        for ev in events {
            let InputEventKind::Key(k) = ev.kind() else { continue };
            if tx.send((k, ev.value())).is_err() {
                closed.store(true, SeqCst);
                return;
            }
        }
    }
}
