//! Tray icon in the Windows notification area, with the same menu as on Linux. The icon
//! lives on its own thread with a message loop (Windows needs that); the main loop changes
//! it through `Handle::update`, like the Linux tray. Menu entries send the same actions as
//! the keyboard shortcuts.

use crate::config::{Config, Mode};
use crate::hotkey::{self, Action, Event};
use crate::icon::State;
use crate::mics;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;
use tray_icon::menu::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE};

pub struct VttTray {
    pub dictating: bool,
    pub session: bool,
    hold: bool,
    correct: bool,
    sounds: bool,
    keep_audio: bool,
    speakers: bool,
    mic: String,
    notes: String,
    hotkey: String,
    mics: Vec<(String, String)>,
    shortcut_session: String,
    shortcut_call: String,
    shortcut_correction: String,
    shortcut_mode: String,
    shortcut_settings: String,
}

type Change = Box<dyn FnOnce(&mut VttTray) + Send>;

/// Changes the tray from other threads.
#[derive(Clone)]
pub struct Handle {
    tx: Sender<Change>,
}

impl Handle {
    /// Change the tray state; None if the tray is gone.
    pub fn update(&self, f: impl FnOnce(&mut VttTray) + Send + 'static) -> Option<()> {
        self.tx.send(Box::new(f)).ok()
    }
}

/// Show the tray icon. None if it can't be created (vtt works without it).
pub fn start(cfg: &Config, events: Sender<Event>) -> Option<Handle> {
    let combo = |s: &str| hotkey::Combo::parse(s).map(|c| c.text()).unwrap_or_default();
    let state = VttTray {
        dictating: false,
        session: crate::session::is_running(),
        hold: cfg.mode == Mode::Hold,
        correct: cfg.correct_text,
        sounds: cfg.sounds,
        keep_audio: cfg.session_keep_audio,
        speakers: cfg.session_speakers,
        mic: cfg.mic.clone(),
        notes: cfg.session_notes.clone(),
        hotkey: hotkey::pretty(&cfg.hotkey),
        mics: mics::list().map(|l| l.into_iter().map(|m| (m.name, m.description)).collect()).unwrap_or_default(),
        shortcut_session: combo(&cfg.shortcut_session),
        shortcut_call: combo(&cfg.shortcut_call),
        shortcut_correction: combo(&cfg.shortcut_correction),
        shortcut_mode: combo(&cfg.shortcut_mode),
        shortcut_settings: combo(&cfg.shortcut_settings),
    };
    let (tx, rx) = mpsc::channel::<Change>();
    let (ready_tx, ready_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut state = state;
        let tray = match build(&state) {
            Ok(t) => {
                let _ = ready_tx.send(true);
                t
            }
            Err(e) => {
                eprintln!("no tray icon ({e}); everything works without it");
                let _ = ready_tx.send(false);
                return;
            }
        };
        loop {
            // Windows messages for the icon (clicks, menu).
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            let mut changed = false;
            while let Ok(ev) = MenuEvent::receiver().try_recv() {
                if let Some(a) = action_for(&state, &ev.id.0) {
                    let _ = events.send(Event::Shortcut(a));
                }
                changed = true; // redraw the check marks from the state
            }
            loop {
                match rx.try_recv() {
                    Ok(f) => {
                        f(&mut state);
                        changed = true;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return,
                }
            }
            if changed {
                let _ = tray.set_icon(Some(icon(&state)));
                let _ = tray.set_tooltip(Some(format!("Wortfluss: {}", status(&state))));
                tray.set_menu(Some(Box::new(menu(&state))));
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    });
    ready_rx.recv().ok().filter(|ok| *ok).map(|_| Handle { tx })
}

fn build(state: &VttTray) -> Result<TrayIcon, String> {
    TrayIconBuilder::new()
        .with_menu(Box::new(menu(state)))
        .with_tooltip(format!("Wortfluss: {}", status(state)))
        .with_icon(icon(state))
        .build()
        .map_err(|e| e.to_string())
}

fn icon(t: &VttTray) -> Icon {
    let st = if t.session { State::Session } else if t.dictating { State::Dictating } else { State::Ready };
    let size = 32;
    Icon::from_rgba(crate::icon::rgba(size, st, false), size, size).expect("icon size matches")
}

fn status(t: &VttTray) -> String {
    if t.session {
        crate::i18n::t("tray.status.session").into()
    } else if t.dictating {
        crate::i18n::t("tray.status.listening").into()
    } else if t.hold {
        crate::i18n::tf("tray.status.hold", &[&t.hotkey])
    } else {
        crate::i18n::tf("tray.status.press", &[&t.hotkey])
    }
}

/// "Label<Tab>Ctrl+Alt+S": Windows shows the part after the tab right-aligned.
fn with_key(label: &str, key: &str) -> String {
    if key.is_empty() { label.to_string() } else { format!("{label}\t{key}") }
}

const NOTES: [&str; 5] = ["auto", "lecture", "meeting", "summary", "none"];

fn menu(t: &VttTray) -> Menu {
    let item = |id: &str, label: &str| MenuItem::with_id(id, label, true, None);
    let check = |id: &str, label: &str, on: bool| CheckMenuItem::with_id(id, label, true, on, None);
    let sep = PredefinedMenuItem::separator;

    let mut items: Vec<Box<dyn IsMenuItem>> = vec![Box::new(MenuItem::new(status(t), false, None)), Box::new(sep())];
    if t.session {
        items.push(Box::new(item("stop", crate::i18n::t("tray.stop_session"))));
    } else {
        items.push(Box::new(item("session:system", &with_key(crate::i18n::t("tray.session_system"), &t.shortcut_session))));
        items.push(Box::new(item("session:mic", crate::i18n::t("tray.session_mic"))));
        items.push(Box::new(item("session:both", &with_key(crate::i18n::t("tray.session_call"), &t.shortcut_call))));
    }
    items.push(Box::new(item("open", crate::i18n::t("tray.open"))));
    items.push(Box::new(sep()));
    items.push(Box::new(check("correction", &with_key(crate::i18n::t("tray.correction"), &t.shortcut_correction), t.correct)));

    let hold = check("mode:hold", &crate::i18n::tf("tray.mode_hold", &[&t.hotkey]), t.hold);
    let toggle = check("mode:toggle", &crate::i18n::tf("tray.mode_toggle", &[&t.hotkey]), !t.hold);
    if let Ok(m) = Submenu::with_items(with_key(crate::i18n::t("tray.mode"), &t.shortcut_mode), true, &[&hold, &toggle]) {
        items.push(Box::new(m));
    }

    let mut mic_items = vec![check("mic:", crate::i18n::t("tray.mic_default"), t.mic.is_empty())];
    for (i, (name, desc)) in t.mics.iter().enumerate() {
        mic_items.push(check(&format!("mic:{i}"), desc, *name == t.mic));
    }
    let refs: Vec<&dyn IsMenuItem> = mic_items.iter().map(|m| m as &dyn IsMenuItem).collect();
    if let Ok(m) = Submenu::with_items(crate::i18n::t("tray.mic"), true, &refs) {
        items.push(Box::new(m));
    }

    let note_items: Vec<CheckMenuItem> =
        NOTES.iter().map(|k| check(&format!("notes:{k}"), crate::i18n::t(&format!("notes.{k}")), t.notes == *k)).collect();
    let refs: Vec<&dyn IsMenuItem> = note_items.iter().map(|m| m as &dyn IsMenuItem).collect();
    if let Ok(m) = Submenu::with_items(crate::i18n::t("tray.notes"), true, &refs) {
        items.push(Box::new(m));
    }
    items.push(Box::new(check("speakers", crate::i18n::t("tray.speakers"), t.speakers)));
    items.push(Box::new(check("keep", crate::i18n::t("tray.keep_audio"), t.keep_audio)));
    items.push(Box::new(check("beeps", crate::i18n::t("tray.beeps"), t.sounds)));
    items.push(Box::new(sep()));
    items.push(Box::new(item("settings", &with_key(crate::i18n::t("tray.settings"), &t.shortcut_settings))));
    items.push(Box::new(item("updates", crate::i18n::t("tray.updates"))));
    let version = MenuItem::new(format!("Wortfluss {}", env!("CARGO_PKG_VERSION")), false, None);
    let made_by = MenuItem::new(crate::i18n::tf("tray.made_by", &[crate::credits::AUTHOR]), false, None);
    let website = item("url:website", crate::i18n::t("tray.website"));
    let github = item("url:github", crate::i18n::t("tray.github"));
    let source = item("url:source", crate::i18n::t("tray.source"));
    let mut about: Vec<&dyn IsMenuItem> = vec![&version, &made_by];
    if !crate::credits::WEBSITE.is_empty() {
        about.push(&website);
    }
    about.extend([&github as &dyn IsMenuItem, &source]);
    if let Ok(m) = Submenu::with_items(crate::i18n::t("tray.about"), true, &about) {
        items.push(Box::new(m));
    }
    items.push(Box::new(item("quit", crate::i18n::t("tray.quit"))));

    let refs: Vec<&dyn IsMenuItem> = items.iter().map(|b| b.as_ref()).collect();
    Menu::with_items(&refs).unwrap_or_default()
}

/// What a menu entry does. Check marks are redrawn from the state after the change.
fn action_for(t: &VttTray, id: &str) -> Option<Action> {
    Some(match id {
        "stop" => Action::StopSession,
        "open" => Action::OpenTranscripts,
        "correction" => Action::ToggleCorrection,
        "mode:hold" | "mode:toggle" => {
            if (id == "mode:hold") == t.hold {
                return None;
            }
            Action::ToggleMode
        }
        "speakers" => Action::ToggleSpeakers,
        "keep" => Action::ToggleKeepAudio,
        "beeps" => Action::ToggleSounds,
        "settings" => Action::OpenSettings,
        "quit" => Action::Quit,
        "updates" => Action::CheckUpdates,
        "url:website" => Action::OpenUrl(crate::credits::WEBSITE.into()),
        "url:github" => Action::OpenUrl(crate::credits::GITHUB.into()),
        "url:source" => Action::OpenUrl(crate::credits::REPO.into()),
        "mic:" => Action::SetMic(String::new()),
        _ => {
            if let Some(src) = id.strip_prefix("session:") {
                Action::StartSession(src.to_string())
            } else if let Some(i) = id.strip_prefix("mic:") {
                Action::SetMic(t.mics.get(i.parse::<usize>().ok()?)?.0.clone())
            } else if let Some(k) = id.strip_prefix("notes:") {
                Action::SetNotes(k.to_string())
            } else {
                return None;
            }
        }
    })
}
