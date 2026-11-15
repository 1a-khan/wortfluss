//! Linux side of the listening indicator: GNOME Wayland lets only the desktop shell place
//! a bubble next to another app's text cursor, so a small GNOME Shell extension (embedded
//! below, installed by `ensure_extension`) draws it; this module sends it frames over the
//! session D-Bus. Other desktops: nothing is shown.

use crate::indicator::Mode;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use zbus::blocking::Connection;

const UUID: &str = "wortfluss@miak-it.de";
const BUS_NAME: &str = "de.miakit.Wortfluss";
const PATH: &str = "/de/miakit/Wortfluss/Indicator";
const IFACE: &str = "de.miakit.Wortfluss.Indicator";

const FILES: [(&str, &str); 3] = [
    ("metadata.json", include_str!("gnome-extension/metadata.json")),
    ("extension.js", include_str!("gnome-extension/extension.js")),
    ("stylesheet.css", include_str!("gnome-extension/stylesheet.css")),
];

fn session() -> Option<&'static Connection> {
    static CONN: OnceLock<Option<Connection>> = OnceLock::new();
    CONN.get_or_init(|| Connection::session().ok()).as_ref()
}

fn extension_running(conn: &Connection) -> bool {
    let Ok(proxy) = zbus::blocking::fdo::DBusProxy::new(conn) else { return false };
    let Ok(name) = BUS_NAME.try_into() else { return false };
    proxy.name_has_owner(name).unwrap_or(false)
}

pub struct Overlay {
    conn: &'static Connection,
}

impl Overlay {
    /// None if the extension isn't running (not GNOME, not installed, not loaded yet).
    pub fn open() -> Option<Self> {
        let conn = session()?;
        extension_running(conn).then_some(Self { conn })
    }

    /// Draw one frame. False if the extension is gone.
    pub fn frame(&mut self, mode: Mode, level: f32, t: f32) -> bool {
        let working = mode == Mode::Working;
        self.conn
            .call_method(Some(BUS_NAME), PATH, Some(IFACE), "Frame", &(working, level as f64, t as f64))
            .is_ok()
    }

    pub fn hide(&mut self) {
        let _ = self.conn.call_method(Some(BUS_NAME), PATH, Some(IFACE), "Hide", &());
    }
}

pub fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|d| d.split(':').any(|p| p.eq_ignore_ascii_case("GNOME")))
        .unwrap_or(false)
}

fn extension_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_DATA_HOME") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".local/share"),
    };
    Some(base.join("gnome-shell/extensions").join(UUID))
}

/// What `ensure_extension` did.
#[derive(Debug, PartialEq)]
pub enum Install {
    /// Installed, enabled and running.
    Ready,
    /// Just installed or updated: GNOME on Wayland loads it at the next login.
    NeedsLogin,
    /// Installed earlier, still waiting for that login (no need to say it again).
    Waiting,
    /// Not a GNOME session.
    NotGnome,
}

/// Install or update the GNOME Shell extension and enable it. Cheap when nothing changed.
pub fn ensure_extension() -> Result<Install> {
    if !is_gnome() {
        return Ok(Install::NotGnome);
    }
    let dir = extension_dir().context("no home folder")?;
    let mut changed = false;
    for (name, content) in FILES {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(content) {
            std::fs::create_dir_all(&dir).with_context(|| format!("could not create {}", dir.display()))?;
            std::fs::write(&path, content).with_context(|| format!("could not write {}", path.display()))?;
            changed = true;
        }
    }
    changed |= enable_in_settings()?;
    let running = session().is_some_and(extension_running);
    Ok(match (running, changed) {
        (true, false) => Install::Ready,
        (_, true) => Install::NeedsLogin,
        (false, false) => Install::Waiting,
    })
}

/// Remove the extension (files and setting). Takes effect at the next login.
pub fn remove_extension() -> Result<()> {
    if let Some(dir) = extension_dir() {
        if dir.exists() {
            std::fs::remove_dir_all(&dir).with_context(|| format!("could not remove {}", dir.display()))?;
        }
    }
    let mut enabled = read_list("enabled-extensions");
    if enabled.iter().any(|u| u == UUID) {
        enabled.retain(|u| u != UUID);
        write_lists(&[("enabled-extensions", enabled)])?;
    }
    Ok(())
}

/// Add the extension to GNOME's enabled list (and take it off the disabled list), written
/// through the dconf service like the Settings app does. True if something changed.
fn enable_in_settings() -> Result<bool> {
    let mut enabled = read_list("enabled-extensions");
    let mut disabled = read_list("disabled-extensions");
    let mut changes = Vec::new();
    if !enabled.iter().any(|u| u == UUID) {
        enabled.push(UUID.to_string());
        changes.push(("enabled-extensions", enabled));
    }
    if disabled.iter().any(|u| u == UUID) {
        disabled.retain(|u| u != UUID);
        changes.push(("disabled-extensions", disabled));
    }
    if changes.is_empty() {
        return Ok(false);
    }
    write_lists(&changes)?;
    Ok(true)
}

fn read_list(key: &str) -> Vec<String> {
    let Some(db) = crate::layout::dconf_user_db() else { return Vec::new() };
    let Ok(file) = gvdb::read::File::from_file(&db) else { return Vec::new() };
    let Ok(table) = file.hash_table() else { return Vec::new() };
    table.get::<Vec<String>>(&format!("/org/gnome/shell/{key}")).unwrap_or_default()
}

/// dconf's writer takes a serialized change set, GVariant type a{smv}.
fn write_lists(lists: &[(&str, Vec<String>)]) -> Result<()> {
    let conn = session().context("no session D-Bus")?;
    let set = changeset(lists)?;
    conn.call_method(
        Some("ca.desrt.dconf"),
        "/ca/desrt/dconf/Writer/user",
        Some("ca.desrt.dconf.Writer"),
        "Change",
        &(set,),
    )
    .context("could not change the GNOME settings")?;
    Ok(())
}

#[allow(deprecated)] // GVariant encoding: kept in zvariant 5, which this build pins
fn changeset(lists: &[(&str, Vec<String>)]) -> Result<Vec<u8>> {
    let map: HashMap<String, Option<zvariant::Value>> = lists
        .iter()
        .map(|(k, v)| (format!("/org/gnome/shell/{k}"), Some(zvariant::Value::from(v.clone()))))
        .collect();
    let ctxt = zvariant::serialized::Context::new_gvariant(zvariant::NATIVE_ENDIAN, 0);
    Ok(zvariant::to_bytes(ctxt, &map)?.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_files_are_consistent() {
        let meta: serde_json::Value = serde_json::from_str(FILES[0].1).unwrap();
        assert_eq!(meta["uuid"], UUID);
        let js = FILES[1].1;
        assert!(js.contains(&format!("'{BUS_NAME}'")) && js.contains(&format!("'{PATH}'")));
        assert!(js.contains(&format!("interface name=\"{IFACE}\"")));
        assert!(js.contains(&format!("const BARS = {};", crate::indicator::BARS)));
    }

    #[test]
    #[allow(deprecated)]
    fn changeset_is_gvariant_a_smv() {
        // Read it back as GVariant a{smv}: the same bytes dconf will see.
        let bytes = changeset(&[("enabled-extensions", vec!["a@b".into(), UUID.into()])]).unwrap();
        let ctxt = zvariant::serialized::Context::new_gvariant(zvariant::NATIVE_ENDIAN, 0);
        let data = zvariant::serialized::Data::new(&bytes, ctxt);
        let (back, _): (HashMap<String, Option<zvariant::OwnedValue>>, _) = data.deserialize().unwrap();
        let v = back["/org/gnome/shell/enabled-extensions"].clone().unwrap();
        let list: Vec<String> = v.try_into().unwrap();
        assert_eq!(list, vec!["a@b".to_string(), UUID.to_string()]);
    }
}
