//! API keys, encrypted, in their own file (secrets.env next to config.toml).
//!
//! Each key is encrypted with ChaCha20-Poly1305 using a random 256-bit master key that
//! lives in the user's OS keyring (GNOME Keyring / KWallet via the Secret Service on
//! Linux, Credential Manager on Windows, Keychain on macOS). The keyring is itself
//! encrypted and unlocked at login. So a leaked secrets.env (git, backup, shared folder)
//! is useless: the master key never leaves the keyring.
//!
//! What this does NOT protect against: someone who is logged in as the same user on the
//! same computer. They can unlock the keyring just like vtt does. No local program can
//! prevent that.
//!
//! Without a keyring (headless systems), the master key is derived from this computer's
//! machine id and the user id instead. That still stops a leaked file from working on
//! another machine, but anyone who can read /etc/machine-id there can recompute it.
//!
//! Values look like `enc:v1:<base64 nonce+ciphertext>` (keyring) or `enc:m1:...`
//! (machine fallback). The variable name is authenticated too, so values can't be swapped.

use crate::platform;
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

const SERVICE: &str = "vtt";
const ACCOUNT: &str = "secrets-master-key";
const KEYRING: &str = "enc:v1:";
const MACHINE: &str = "enc:m1:";

const HEADER: &str = "\
# vtt API keys, encrypted. Change them with `vtt setup`.
# Only your user on this computer can decrypt them: the master key is in your login
# keyring, not in this file. Keep this file out of git anyway.
";

pub fn path() -> Result<PathBuf> {
    Ok(platform::config_dir()?.join("secrets.env"))
}

/// Name of a provider's key in secrets.env, e.g. GROQ_API_KEY.
pub fn var_name(provider_id: &str) -> String {
    format!("{}_API_KEY", provider_id.to_uppercase())
}

/// Where the master key comes from, for `vtt check`.
pub enum Protection {
    Keyring,
    Machine(String),
}

pub fn protection() -> Protection {
    match keyring_key(false) {
        Ok(_) => Protection::Keyring,
        Err(e) => Protection::Machine(format!("{e:#}")),
    }
}

// ---- master keys ----

static KEYRING_CACHE: Mutex<Option<[u8; 32]>> = Mutex::new(None);

/// The master key from the OS keyring. `create`: make one if there is none yet.
/// Ok(None) = keyring works but holds no key.
fn keyring_key(create: bool) -> Result<Option<[u8; 32]>> {
    if let Some(k) = *KEYRING_CACHE.lock().unwrap() {
        return Ok(Some(k));
    }
    // Right after login the keyring may still be starting: retry a little.
    let mut last = None;
    for attempt in 0..5 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs(1));
        }
        match try_keyring_key(create) {
            Ok(k) => {
                if let Some(k) = k {
                    *KEYRING_CACHE.lock().unwrap() = Some(k);
                }
                return Ok(k);
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap())
}

fn try_keyring_key(create: bool) -> Result<Option<[u8; 32]>> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| anyhow!("no keyring available: {e}"))?;
    match entry.get_secret() {
        Ok(bytes) => {
            let key: [u8; 32] = bytes
                .try_into()
                .map_err(|_| anyhow!("the vtt entry in your keyring is damaged"))?;
            Ok(Some(key))
        }
        Err(keyring::Error::NoEntry) if create => {
            let mut key = [0u8; 32];
            getrandom::fill(&mut key).map_err(|e| anyhow!("no random numbers: {e}"))?;
            entry
                .set_secret(&key)
                .map_err(|e| anyhow!("could not save the master key in your keyring: {e}"))?;
            Ok(Some(key))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(anyhow!("keyring: {e}")),
    }
}

/// This computer's id and the user's, for the fallback key.
#[cfg(unix)]
fn machine_and_user() -> Result<(String, u32)> {
    let id = ["/etc/machine-id", "/var/lib/dbus/machine-id"]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .context("this computer has no machine id")?;
    Ok((id, unsafe { libc::getuid() }))
}

/// Windows: the installation's MachineGuid, and a number derived from the user's profile
/// folder (the fallback is only used when the Credential Manager doesn't work).
#[cfg(windows)]
fn machine_and_user() -> Result<(String, u32)> {
    let id = crate::platform::machine_guid().context("this computer has no MachineGuid")?;
    let profile = std::env::var("USERPROFILE").unwrap_or_default().to_lowercase();
    let user = profile.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    Ok((id, user))
}

/// Fallback master key, bound to this computer and user.
fn machine_key() -> Result<[u8; 32]> {
    use sha2::{Digest, Sha256};
    let (id, uid) = machine_and_user()?;
    let mut h = Sha256::new();
    h.update(b"vtt machine-bound secrets v1\0");
    h.update(id.as_bytes());
    h.update(b"\0");
    h.update(uid.to_le_bytes());
    Ok(h.finalize().into())
}

// ---- encrypt / decrypt one value ----

fn seal(key: &[u8; 32], name: &str, plain: &str) -> Result<String> {
    let cipher = ChaCha20Poly1305::new_from_slice(key).expect("32-byte key");
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).map_err(|e| anyhow!("no random numbers: {e}"))?;
    let ct = cipher
        .encrypt(
            &Nonce::from(nonce),
            Payload { msg: plain.as_bytes(), aad: name.as_bytes() },
        )
        .map_err(|_| anyhow!("encryption failed"))?;
    let mut blob = nonce.to_vec();
    blob.extend_from_slice(&ct);
    Ok(B64.encode(blob))
}

fn open(key: &[u8; 32], name: &str, b64: &str) -> Option<String> {
    let blob = B64.decode(b64.trim()).ok()?;
    if blob.len() < 12 + 16 {
        return None;
    }
    let (nonce, ct) = blob.split_at(12);
    let nonce: [u8; 12] = nonce.try_into().ok()?;
    let cipher = ChaCha20Poly1305::new_from_slice(key).expect("32-byte key");
    let plain = cipher
        .decrypt(&Nonce::from(nonce), Payload { msg: ct, aad: name.as_bytes() })
        .ok()?;
    String::from_utf8(plain).ok()
}

/// Encrypt with the keyring master key, or the machine key if there's no keyring.
pub fn encrypt(name: &str, plain: &str) -> Result<String> {
    match keyring_key(true) {
        Ok(Some(k)) => Ok(format!("{KEYRING}{}", seal(&k, name, plain)?)),
        Ok(None) | Err(_) => Ok(format!("{MACHINE}{}", seal(&machine_key()?, name, plain)?)),
    }
}

/// Decrypt a stored value. Values without an `enc:` prefix are returned as they are,
/// so plain keys (e.g. in an environment variable) keep working.
pub fn decrypt(name: &str, value: &str) -> Result<String> {
    let value = value.trim();
    let fail = || {
        anyhow!(
            "cannot decrypt {name}: it was saved on another computer or by another user, \
             or the keyring was reset. Run `vtt setup` and enter the key again"
        )
    };
    if let Some(b64) = value.strip_prefix(KEYRING) {
        let key = keyring_key(false)
            .context("cannot reach your keyring to decrypt the API key")?
            .ok_or_else(fail)?;
        open(&key, name, b64).ok_or_else(fail)
    } else if let Some(b64) = value.strip_prefix(MACHINE) {
        open(&machine_key()?, name, b64).ok_or_else(fail)
    } else if value.starts_with("enc:") {
        bail!("{name} uses an unknown encryption format (made by a newer vtt?)")
    } else {
        Ok(value.to_string())
    }
}

// ---- the file ----

/// NAME -> stored (still encrypted) value.
pub fn read_all() -> Result<BTreeMap<String, String>> {
    let path = path()?;
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().trim_matches('"').to_string()))
        .collect())
}

/// Encrypt and store these keys (NAME -> plain key). Other entries in the file are kept.
pub fn store(keys: &BTreeMap<String, String>) -> Result<()> {
    let mut all = read_all()?;
    for (name, plain) in keys {
        all.insert(name.clone(), encrypt(name, plain)?);
    }
    let mut text = HEADER.to_string();
    for (name, value) in &all {
        text.push_str(&format!("{name}={value}\n"));
    }
    platform::write_private(&path()?, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_tamper() {
        let key = [7u8; 32];
        let sealed = seal(&key, "GROQ_API_KEY", "gsk_secret").unwrap();
        assert_eq!(open(&key, "GROQ_API_KEY", &sealed).as_deref(), Some("gsk_secret"));
        // wrong master key, or value moved to another name: refused
        assert!(open(&[8u8; 32], "GROQ_API_KEY", &sealed).is_none());
        assert!(open(&key, "OPENAI_API_KEY", &sealed).is_none());
        // the ciphertext doesn't contain the key
        assert!(!sealed.contains("gsk_secret"));
    }

    #[test]
    fn machine_fallback_round_trip() {
        let v = format!("{MACHINE}{}", seal(&machine_key().unwrap(), "X", "k").unwrap());
        assert_eq!(decrypt("X", &v).unwrap(), "k");
        assert_eq!(decrypt("X", "plain-key").unwrap(), "plain-key");
    }
}
