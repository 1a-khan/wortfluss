//! Automatic updates, like Teams or Postman: Wortfluss looks for a new version in the
//! background, downloads it, checks it, and switches to it when it is idle.
//!
//! Releases live in the public repository `1a-khan/wortfluss-releases` (only binaries,
//! no source). Each release has `latest.json` (version, files, SHA-256 hashes) and
//! `latest.json.sig`, an Ed25519 signature of it. Wortfluss accepts an update only if
//! the signature matches the public key built into it (`release-key.pub`) and the
//! version is newer than its own, so nobody else can push code to its users and old
//! versions can't be replayed. The private key stays on the release machine.
//!
//! Installing: the program lives in a folder the user owns (Linux: ~/.local/bin,
//! Windows: %LOCALAPPDATA%\Programs\Wortfluss), so no admin rights are needed. New files
//! are written next to the old ones as `NAME.new`, checked, then swapped in by renaming
//! (Windows can rename a running .exe: the old one becomes `NAME.old` and is deleted at
//! the next start). Then Wortfluss restarts itself.

use anyhow::{anyhow, bail, Context, Result};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where the newest release is described.
pub const MANIFEST_URL: &str =
    "https://github.com/1a-khan/wortfluss-releases/releases/latest/download/latest.json";
/// Overrides MANIFEST_URL (a URL or a local file path), for testing a release.
const URL_ENV: &str = "VTT_UPDATE_URL";

/// The release signing key (public half), hex. Generated with `vtt _release-keygen`.
const PUBLIC_KEY_HEX: &str = include_str!("../release-key.pub");

/// No binary of ours is anywhere near this.
const MAX_DOWNLOAD: u64 = 200 * 1024 * 1024;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub date: String,
    /// Platform ("linux-x86_64", "windows-x86_64") -> files to install.
    pub files: std::collections::BTreeMap<String, Vec<FileEntry>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileEntry {
    /// File name in the install folder ("vtt", "vtt.exe", "wortfluss.exe").
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

pub fn platform_id() -> String {
    let os = if cfg!(windows) { "windows" } else { std::env::consts::OS };
    format!("{os}-{}", std::env::consts::ARCH)
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// "1.2.3" -> (1, 2, 3); anything after a '-' or '+' is ignored.
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((parts.next()??, parts.next().unwrap_or(Some(0))?, parts.next().unwrap_or(Some(0))?))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn public_key() -> Result<VerifyingKey> {
    let bytes: [u8; 32] = hex_decode(PUBLIC_KEY_HEX)
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| anyhow!("built-in release key is malformed"))?;
    Ok(VerifyingKey::from_bytes(&bytes)?)
}

/// Checks that `sig_hex` is the release key's signature of `data`.
pub fn verify(key: &VerifyingKey, data: &[u8], sig_hex: &str) -> Result<()> {
    let sig = hex_decode(sig_hex).ok_or_else(|| anyhow!("signature is not hex"))?;
    let sig = Signature::from_slice(&sig).map_err(|_| anyhow!("signature has the wrong length"))?;
    key.verify_strict(data, &sig).map_err(|_| anyhow!("the update's signature is not valid; it was not installed"))
}

/// Bytes from a URL, or from a local path (testing).
fn fetch(location: &str, limit: u64) -> Result<Vec<u8>> {
    if !location.starts_with("http://") && !location.starts_with("https://") {
        return std::fs::read(location.trim_start_matches("file://")).with_context(|| format!("reading {location}"));
    }
    let resp = ureq::get(location)
        .timeout(Duration::from_secs(300))
        .set("User-Agent", "Wortfluss-updater")
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => anyhow!("HTTP {code} for {location}"),
            e => anyhow!("{e}"),
        })?;
    let mut data = Vec::new();
    resp.into_reader().take(limit + 1).read_to_end(&mut data)?;
    if data.len() as u64 > limit {
        bail!("{location} is larger than expected");
    }
    Ok(data)
}

fn manifest_location() -> String {
    std::env::var(URL_ENV).ok().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| MANIFEST_URL.to_string())
}

/// The newest release, if it is newer than this program and correctly signed.
pub fn check() -> Result<Option<Manifest>> {
    let location = manifest_location();
    let data = fetch(&location, 1024 * 1024)?;
    let sig = fetch(&format!("{location}.sig"), 4096)?;
    verify(&public_key()?, &data, &String::from_utf8_lossy(&sig))?;
    let m: Manifest = serde_json::from_slice(&data).context("latest.json is not valid")?;
    if !is_newer(&m.version, current_version()) {
        return Ok(None);
    }
    if !m.files.contains_key(&platform_id()) {
        return Ok(None); // nothing for this platform in this release
    }
    Ok(Some(m))
}

pub fn install_dir() -> Result<PathBuf> {
    let exe = crate::platform::exe_path()?;
    Ok(exe.parent().ok_or_else(|| anyhow!("no folder for {}", exe.display()))?.to_path_buf())
}

fn staged(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.new"))
}

/// Download and check this release's files next to the installed ones (`NAME.new`).
/// Nothing installed changes yet.
pub fn download(m: &Manifest) -> Result<()> {
    let dir = install_dir()?;
    let files = m.files.get(&platform_id()).ok_or_else(|| anyhow!("no files for {}", platform_id()))?;
    for f in files {
        if f.name.contains(['/', '\\']) || f.name.starts_with('.') {
            bail!("unsafe file name in the release: {}", f.name);
        }
        let data = fetch(&f.url, MAX_DOWNLOAD.min(f.size.max(1)))?;
        let got = hex(&Sha256::digest(&data));
        if !got.eq_ignore_ascii_case(f.sha256.trim()) || data.len() as u64 != f.size {
            bail!("{} doesn't match the signed release (hash or size); not installed", f.name);
        }
        let path = staged(&dir, &f.name);
        std::fs::write(&path, &data).with_context(|| format!("writing {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(())
}

/// Put the downloaded files in place. Quick (renames only).
pub fn swap(m: &Manifest) -> Result<()> {
    let dir = install_dir()?;
    let files = m.files.get(&platform_id()).ok_or_else(|| anyhow!("no files for {}", platform_id()))?;
    for f in files {
        if !staged(&dir, &f.name).exists() {
            bail!("{} was not downloaded", f.name);
        }
    }
    for f in files {
        let target = dir.join(&f.name);
        if cfg!(windows) && target.exists() {
            // A running .exe can't be replaced, but it can be renamed.
            let old = dir.join(format!("{}.old", f.name));
            let _ = std::fs::remove_file(&old);
            std::fs::rename(&target, &old).with_context(|| format!("moving {} aside", target.display()))?;
        }
        std::fs::rename(staged(&dir, &f.name), &target).with_context(|| format!("installing {}", target.display()))?;
    }
    #[cfg(windows)]
    crate::platform::set_installed_version(&m.version);
    Ok(())
}

/// Remove what an earlier update left behind (`*.old`, unfinished `*.new`).
pub fn cleanup() {
    let Ok(dir) = install_dir() else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let ours = ["vtt", "vtt.exe", "wortfluss.exe"];
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let base = name.strip_suffix(".old").or_else(|| name.strip_suffix(".new"));
        if base.is_some_and(|b| ours.contains(&b)) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

// ---- release side: `vtt _release-keygen`, `vtt _release-sign` ----------------------

/// `vtt _release-keygen KEYFILE`: a new signing key (private, 0600) and its public half
/// printed in hex (goes into release-key.pub).
pub fn keygen(path: &Path) -> Result<()> {
    if path.exists() {
        bail!("{} exists; not overwritten", path.display());
    }
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| anyhow!("no randomness: {e}"))?;
    let key = ed25519_dalek::SigningKey::from_bytes(&seed);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::platform::write_private(path, &hex(&seed))?;
    println!("{}", hex(&key.verifying_key().to_bytes()));
    Ok(())
}

/// `vtt _release-sign KEYFILE VERSION BASE_URL OUTDIR PLATFORM:NAME=FILE ...`: writes
/// OUTDIR/latest.json and latest.json.sig for these files (uploaded as assets named
/// like the file, under BASE_URL).
pub fn sign_release(args: &[String]) -> Result<()> {
    use ed25519_dalek::Signer;
    let [key_file, version, base_url, out_dir, specs @ ..] = args else {
        bail!("vtt _release-sign KEYFILE VERSION BASE_URL OUTDIR PLATFORM:NAME=FILE ...");
    };
    parse_version(version).ok_or_else(|| anyhow!("bad version {version}"))?;
    let seed: [u8; 32] = hex_decode(&std::fs::read_to_string(key_file)?)
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| anyhow!("{key_file} is not a signing key"))?;
    let key = ed25519_dalek::SigningKey::from_bytes(&seed);
    if key.verifying_key() != public_key()? {
        bail!("{key_file} doesn't match release-key.pub in this build");
    }
    let mut files: std::collections::BTreeMap<String, Vec<FileEntry>> = Default::default();
    for spec in specs {
        let (platform, rest) = spec.split_once(':').ok_or_else(|| anyhow!("bad spec {spec}"))?;
        let (name, file) = rest.split_once('=').ok_or_else(|| anyhow!("bad spec {spec}"))?;
        let data = std::fs::read(file).with_context(|| format!("reading {file}"))?;
        let asset = Path::new(file).file_name().unwrap().to_string_lossy().to_string();
        files.entry(platform.to_string()).or_default().push(FileEntry {
            name: name.to_string(),
            url: format!("{}/{asset}", base_url.trim_end_matches('/')),
            sha256: hex(&Sha256::digest(&data)),
            size: data.len() as u64,
        });
    }
    let m = Manifest { version: version.clone(), date: {
        let (y, mo, d, ..) = crate::platform::local_time();
        format!("{y}-{mo:02}-{d:02}")
    }, files };
    let json = serde_json::to_vec_pretty(&m)?;
    let sig = key.sign(&json);
    let out = Path::new(out_dir);
    std::fs::create_dir_all(out)?;
    std::fs::write(out.join("latest.json"), &json)?;
    std::fs::write(out.join("latest.json.sig"), hex(&sig.to_bytes()))?;
    println!("{}", String::from_utf8_lossy(&json));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(is_newer("0.3.0", "0.2.0"));
        assert!(is_newer("v1.0", "0.9.9"));
        assert!(is_newer("0.2.10", "0.2.9"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
        assert!(!is_newer("garbage", "0.2.0"));
    }

    #[test]
    fn signatures() {
        use ed25519_dalek::Signer;
        let key = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let data = br#"{"version":"9.9.9"}"#;
        let sig = hex(&key.sign(data).to_bytes());
        assert!(verify(&key.verifying_key(), data, &sig).is_ok());
        assert!(verify(&key.verifying_key(), br#"{"version":"9.9.8"}"#, &sig).is_err(), "changed data");
        let other = ed25519_dalek::SigningKey::from_bytes(&[8u8; 32]);
        assert!(verify(&other.verifying_key(), data, &sig).is_err(), "other key");
        assert!(public_key().is_ok(), "built-in key parses");
    }
}
