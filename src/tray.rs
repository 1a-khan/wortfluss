//! Tray icon (system tray / "hidden icons" area) with a right-click menu for the common
//! settings and for sessions. Linux: StatusNotifierItem over D-Bus (KDE natively, GNOME
//! with the AppIndicator extension that Ubuntu enables by default). Menu entries send
//! the same actions as the keyboard shortcuts to the main loop.
//! (A Windows port uses the `tray-icon` crate with the same menu.)

use crate::config::{Config, Mode};
use crate::hotkey::{Action, Event};
use crate::{hotkey, mics};
use ksni::blocking::TrayMethods;
use ksni::menu::{CheckmarkItem, RadioGroup, RadioItem, StandardItem, SubMenu};
use ksni::{MenuItem, ToolTip};
use std::sync::mpsc::Sender;

pub type Handle = ksni::blocking::Handle<VttTray>;

pub struct VttTray {
    tx: Sender<Event>,
    pub dictating: bool,
    pub session: bool,
    hold: bool,
    correct: bool,
    sounds: bool,
    keep_audio: bool,
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

/// Show the tray icon. None if the desktop has no tray (vtt works without it).
pub fn start(cfg: &Config, tx: Sender<Event>) -> Option<Handle> {
    let combo = |s: &str| hotkey::Combo::parse(s).map(|c| c.text()).unwrap_or_default();
    let tray = VttTray {
        tx,
        dictating: false,
        session: crate::session::is_running(),
        hold: cfg.mode == Mode::Hold,
        correct: cfg.correct_text,
        sounds: cfg.sounds,
        keep_audio: cfg.session_keep_audio,
        mic: cfg.mic.clone(),
        notes: cfg.session_notes.clone(),
        hotkey: hotkey::pretty(&cfg.hotkey),
        mics: mics::list()
            .map(|l| l.into_iter().map(|m| (m.name, m.description)).collect())
            .unwrap_or_default(),
        shortcut_session: combo(&cfg.shortcut_session),
        shortcut_call: combo(&cfg.shortcut_call),
        shortcut_correction: combo(&cfg.shortcut_correction),
        shortcut_mode: combo(&cfg.shortcut_mode),
        shortcut_settings: combo(&cfg.shortcut_settings),
    };
    match tray.spawn() {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("no tray icon ({e}); everything works without it");
            None
        }
    }
}

/// "Label   Ctrl+Alt+S" when a shortcut exists.
fn with_key(label: &str, key: &str) -> String {
    if key.is_empty() { label.to_string() } else { format!("{label}\t{key}") }
}

impl VttTray {
    fn send(&self, a: Action) {
        let _ = self.tx.send(Event::Shortcut(a));
    }

    fn status(&self) -> String {
        if self.session {
            "Session running: transcribing".into()
        } else if self.dictating {
            "Listening...".into()
        } else if self.hold {
            format!("Ready: hold {} to dictate", self.hotkey)
        } else {
            format!("Ready: press {} to dictate", self.hotkey)
        }
    }
}

fn item(label: impl Into<String>, action: Action) -> MenuItem<VttTray> {
    StandardItem {
        label: label.into(),
        activate: Box::new(move |t: &mut VttTray| t.send(action.clone())),
        ..Default::default()
    }
    .into()
}

fn check(label: impl Into<String>, checked: bool, action: Action) -> MenuItem<VttTray> {
    CheckmarkItem {
        label: label.into(),
        checked,
        activate: Box::new(move |t: &mut VttTray| t.send(action.clone())),
        ..Default::default()
    }
    .into()
}

impl ksni::Tray for VttTray {
    fn id(&self) -> String {
        "vtt".into()
    }

    fn title(&self) -> String {
        "vtt voice to text".into()
    }

    fn icon_name(&self) -> String {
        if self.session || self.dictating {
            "media-record-symbolic".into()
        } else {
            "audio-input-microphone-symbolic".into()
        }
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "vtt".into(),
            description: self.status(),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut menu: Vec<MenuItem<Self>> = vec![
            StandardItem { label: self.status(), enabled: false, ..Default::default() }.into(),
            MenuItem::Separator,
        ];

        // Sessions
        if self.session {
            menu.push(item("Stop session and write notes", Action::StopSession));
        } else {
            menu.push(item(with_key("Transcribe what the computer plays", &self.shortcut_session), Action::StartSession("system".into())));
            menu.push(item("Transcribe the microphone (room)", Action::StartSession("mic".into())));
            menu.push(item(with_key("Transcribe a call (You / Others, headphones)", &self.shortcut_call), Action::StartSession("both".into())));
        }
        menu.push(item("Open transcripts folder", Action::OpenTranscripts));
        menu.push(MenuItem::Separator);

        // Quick settings
        menu.push(check(with_key("Spelling & grammar correction", &self.shortcut_correction), self.correct, Action::ToggleCorrection));
        menu.push(
            SubMenu {
                label: with_key("Dictation key mode", &self.shortcut_mode),
                submenu: vec![RadioGroup {
                    selected: if self.hold { 0 } else { 1 },
                    select: Box::new(|t: &mut VttTray, i| {
                        if (i == 0) != t.hold {
                            t.send(Action::ToggleMode);
                        }
                    }),
                    options: vec![
                        RadioItem { label: format!("Hold {} while speaking", self.hotkey), ..Default::default() },
                        RadioItem { label: format!("Press {} to start, again to stop", self.hotkey), ..Default::default() },
                    ],
                    ..Default::default()
                }
                .into()],
                ..Default::default()
            }
            .into(),
        );
        let mut mic_names = vec![String::new()];
        mic_names.extend(self.mics.iter().map(|m| m.0.clone()));
        let mut mic_items = vec![RadioItem { label: "System default".into(), ..Default::default() }];
        mic_items.extend(self.mics.iter().map(|m| RadioItem { label: m.1.clone(), ..Default::default() }));
        menu.push(
            SubMenu {
                label: "Microphone".into(),
                submenu: vec![RadioGroup {
                    selected: mic_names.iter().position(|n| *n == self.mic).unwrap_or(0),
                    select: Box::new(move |t: &mut VttTray, i| {
                        if let Some(name) = mic_names.get(i) {
                            t.send(Action::SetMic(name.clone()));
                        }
                    }),
                    options: mic_items,
                    ..Default::default()
                }
                .into()],
                ..Default::default()
            }
            .into(),
        );
        let kinds = ["auto", "lecture", "meeting", "summary", "none"];
        menu.push(
            SubMenu {
                label: "Notes after a session".into(),
                submenu: vec![RadioGroup {
                    selected: kinds.iter().position(|k| *k == self.notes).unwrap_or(0),
                    select: Box::new(move |t: &mut VttTray, i| t.send(Action::SetNotes(kinds[i].into()))),
                    options: ["Automatic", "Lecture notes", "Meeting minutes", "Summary", "None"]
                        .iter()
                        .map(|l| RadioItem { label: (*l).into(), ..Default::default() })
                        .collect(),
                    ..Default::default()
                }
                .into()],
                ..Default::default()
            }
            .into(),
        );
        menu.push(check("Keep session audio (asks to delete)", self.keep_audio, Action::ToggleKeepAudio));
        menu.push(check("Beeps", self.sounds, Action::ToggleSounds));
        menu.push(MenuItem::Separator);
        menu.push(item(with_key("All settings...", &self.shortcut_settings), Action::OpenSettings));
        menu.push(item("Quit vtt", Action::Quit));
        menu
    }
}
