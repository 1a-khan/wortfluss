//! Finds out which keyboard layout the desktop is using *right now*, so typed text comes
//! out right even after the user switches layouts (e.g. German <-> Turkish with Super+Space).
//!
//! GNOME records the active input source first in `mru-sources` in its settings database
//! (~/.config/dconf/user, a GVDB file), which we read directly. Elsewhere we fall back to
//! the system default from /etc/default/keyboard, then to US.
//! (Linux only. On Windows the port types with KEYEVENTF_UNICODE, which needs no layout.)

use crate::config::Config;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Layout {
    pub layout: String,
    pub variant: String,
}

impl std::fmt::Display for Layout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.variant.is_empty() {
            write!(f, "{}", self.layout)
        } else {
            write!(f, "{} ({})", self.layout, self.variant)
        }
    }
}

/// The layout to type with: the one set in config, or the detected one when set to auto.
pub fn resolve(cfg: &Config) -> Layout {
    if cfg.layout_is_auto() {
        detect().0
    } else {
        Layout {
            layout: cfg.keyboard_layout.trim().to_string(),
            variant: cfg.keyboard_variant.trim().to_string(),
        }
    }
}

/// Detected layout and where it came from (for `vtt check` / setup).
pub fn detect() -> (Layout, &'static str) {
    if let Some(l) = gnome_active() {
        return (l, "GNOME, follows layout switches live");
    }
    if let Some(l) = system_default() {
        return (l, "system default in /etc/default/keyboard");
    }
    (
        Layout {
            layout: "us".into(),
            variant: String::new(),
        },
        "fallback",
    )
}

pub fn dconf_user_db() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    Some(base.join("dconf").join("user"))
}

fn gnome_active() -> Option<Layout> {
    let file = gvdb::read::File::from_file(&dconf_user_db()?).ok()?;
    let table = file.hash_table().ok()?;
    let sources: Vec<(String, String)> = ["mru-sources", "sources"]
        .iter()
        .find_map(|k| {
            table
                .get::<Vec<(String, String)>>(&format!("/org/gnome/desktop/input-sources/{k}"))
                .ok()
                .filter(|v| !v.is_empty())
        })?;
    // Entries look like ("xkb", "de+nodeadkeys"). IBus input methods (("ibus", "anthy"))
    // don't map to a key layout, so take the first xkb one.
    let (_, id) = sources.into_iter().find(|(kind, _)| kind == "xkb")?;
    let (layout, variant) = id.split_once('+').unwrap_or((&id, ""));
    Some(Layout {
        layout: layout.to_string(),
        variant: variant.to_string(),
    })
}

fn system_default() -> Option<Layout> {
    let text = std::fs::read_to_string("/etc/default/keyboard").ok()?;
    let get = |name: &str| {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(name)?.strip_prefix('='))
            .map(|v| {
                let v = v.trim().trim_matches('"');
                v.split(',').next().unwrap_or("").trim().to_string()
            })
            .unwrap_or_default()
    };
    let layout = get("XKBLAYOUT");
    (!layout.is_empty()).then(|| Layout {
        layout,
        variant: get("XKBVARIANT"),
    })
}
