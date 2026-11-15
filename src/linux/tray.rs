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
    /// The Wortfluss icon per state, drawn once at several sizes.
    icons: [Vec<ksni::Icon>; 3],
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
    shortcut_rewrite: String,
}

/// Show the tray icon. None if the desktop has no tray (vtt works without it).
pub fn start(cfg: &Config, tx: Sender<Event>) -> Option<Handle> {
    let combo = |s: &str| hotkey::Combo::parse(s).map(|c| c.text()).unwrap_or_default();
    let draw = |st| {
        [22u32, 32, 48, 64]
            .iter()
            .map(|&s| ksni::Icon { width: s as i32, height: s as i32, data: crate::icon::argb(s, st) })
            .collect::<Vec<_>>()
    };
    let tray = VttTray {
        tx,
        icons: [
            draw(crate::icon::State::Ready),
            draw(crate::icon::State::Dictating),
            draw(crate::icon::State::Session),
        ],
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
        mics: mics::list()
            .map(|l| l.into_iter().map(|m| (m.name, m.description)).collect())
            .unwrap_or_default(),
        shortcut_session: combo(&cfg.shortcut_session),
        shortcut_call: combo(&cfg.shortcut_call),
        shortcut_correction: combo(&cfg.shortcut_correction),
        shortcut_mode: combo(&cfg.shortcut_mode),
        shortcut_settings: combo(&cfg.shortcut_settings),
        shortcut_rewrite: combo(&cfg.shortcut_rewrite),
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
            crate::i18n::t("tray.status.session").into()
        } else if self.dictating {
            crate::i18n::t("tray.status.listening").into()
        } else if self.hold {
            crate::i18n::tf("tray.status.hold", &[&self.hotkey])
        } else {
            crate::i18n::tf("tray.status.press", &[&self.hotkey])
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
        "Wortfluss".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let i = if self.session { 2 } else if self.dictating { 1 } else { 0 };
        self.icons[i].clone()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Wortfluss".into(),
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
            menu.push(item(crate::i18n::t("tray.stop_session"), Action::StopSession));
        } else {
            menu.push(item(with_key(crate::i18n::t("tray.session_system"), &self.shortcut_session), Action::StartSession("system".into())));
            menu.push(item(crate::i18n::t("tray.session_mic"), Action::StartSession("mic".into())));
            menu.push(item(with_key(crate::i18n::t("tray.session_call"), &self.shortcut_call), Action::StartSession("both".into())));
        }
        menu.push(item(crate::i18n::t("tray.open"), Action::OpenTranscripts));
        menu.push(MenuItem::Separator);
        menu.push(item(with_key(crate::i18n::t("tray.rewrite"), &self.shortcut_rewrite), Action::Rewrite));
        menu.push(MenuItem::Separator);

        // Quick settings
        menu.push(check(with_key(crate::i18n::t("tray.correction"), &self.shortcut_correction), self.correct, Action::ToggleCorrection));
        menu.push(
            SubMenu {
                label: with_key(crate::i18n::t("tray.mode"), &self.shortcut_mode),
                submenu: vec![RadioGroup {
                    selected: if self.hold { 0 } else { 1 },
                    select: Box::new(|t: &mut VttTray, i| {
                        if (i == 0) != t.hold {
                            t.send(Action::ToggleMode);
                        }
                    }),
                    options: vec![
                        RadioItem { label: crate::i18n::tf("tray.mode_hold", &[&self.hotkey]), ..Default::default() },
                        RadioItem { label: crate::i18n::tf("tray.mode_toggle", &[&self.hotkey]), ..Default::default() },
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
        let mut mic_items = vec![RadioItem { label: crate::i18n::t("tray.mic_default").into(), ..Default::default() }];
        mic_items.extend(self.mics.iter().map(|m| RadioItem { label: m.1.clone(), ..Default::default() }));
        menu.push(
            SubMenu {
                label: crate::i18n::t("tray.mic").into(),
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
                label: crate::i18n::t("tray.notes").into(),
                submenu: vec![RadioGroup {
                    selected: kinds.iter().position(|k| *k == self.notes).unwrap_or(0),
                    select: Box::new(move |t: &mut VttTray, i| t.send(Action::SetNotes(kinds[i].into()))),
                    options: kinds
                        .iter()
                        .map(|k| RadioItem { label: crate::i18n::t(&format!("notes.{k}")).into(), ..Default::default() })
                        .collect(),
                    ..Default::default()
                }
                .into()],
                ..Default::default()
            }
            .into(),
        );
        menu.push(check(crate::i18n::t("tray.speakers"), self.speakers, Action::ToggleSpeakers));
        menu.push(check(crate::i18n::t("tray.keep_audio"), self.keep_audio, Action::ToggleKeepAudio));
        menu.push(check(crate::i18n::t("tray.beeps"), self.sounds, Action::ToggleSounds));
        menu.push(MenuItem::Separator);
        menu.push(item(with_key(crate::i18n::t("tray.settings"), &self.shortcut_settings), Action::OpenSettings));
        menu.push(item(crate::i18n::t("tray.updates"), Action::CheckUpdates));
        let mut about: Vec<MenuItem<Self>> = vec![
            StandardItem {
                label: format!("Wortfluss {}", env!("CARGO_PKG_VERSION")),
                enabled: false,
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: crate::i18n::tf("tray.made_by", &[crate::credits::AUTHOR]),
                enabled: false,
                ..Default::default()
            }
            .into(),
        ];
        if !crate::credits::WEBSITE.is_empty() {
            about.push(item(crate::i18n::t("tray.website"), Action::OpenUrl(crate::credits::WEBSITE.into())));
        }
        about.push(item(crate::i18n::t("tray.github"), Action::OpenUrl(crate::credits::GITHUB.into())));
        about.push(item(crate::i18n::t("tray.source"), Action::OpenUrl(crate::credits::REPO.into())));
        menu.push(SubMenu { label: crate::i18n::t("tray.about").into(), submenu: about, ..Default::default() }.into());
        menu.push(item(crate::i18n::t("tray.quit"), Action::Quit));
        menu
    }
}
