//! Who made Wortfluss, shown in the tray ("About Wortfluss"), `vtt help`, `vtt version`
//! and the Windows installer. Licensed under Apache 2.0: the NOTICE file with these
//! credits must travel with every copy and derived work.

pub const AUTHOR: &str = "Ammad Khan";
pub const COMPANY: &str = "MIAK IT";
pub const GITHUB: &str = "https://github.com/1a-khan";
pub const REPO: &str = "https://github.com/1a-khan/wortfluss";
/// The author's website.
pub const WEBSITE: &str = "https://miak-it.de";
pub const LICENSE: &str = "Apache License 2.0";

/// "Wortfluss 0.3.0 · Ammad Khan (MIAK IT)"
pub fn line() -> String {
    format!("Wortfluss {} · {AUTHOR} ({COMPANY})", env!("CARGO_PKG_VERSION"))
}

/// Links, one per line, for the terminal.
pub fn links() -> String {
    let mut out = String::new();
    if !WEBSITE.is_empty() {
        out.push_str(&format!("Website: {WEBSITE}\n"));
    }
    out.push_str(&format!("GitHub:  {GITHUB}\nSource:  {REPO}\nLicense: {LICENSE} (free to use and change; keep the NOTICE)\n"));
    out
}
