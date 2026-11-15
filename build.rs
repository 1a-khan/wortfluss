//! Windows builds: puts version information (product, company, version, description),
//! the Wortfluss icon and an application manifest into vtt.exe and wortfluss.exe. An
//! .exe without them shows an empty "Details" tab and makes virus scanners and
//! SmartScreen more suspicious. Nothing is done for other systems.

#[allow(dead_code)]
#[path = "src/icon.rs"]
mod icon;

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=installer/wortfluss.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico = out.join("wortfluss.ico");
    icon::write_ico(&ico).expect("drawing the icon");

    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico.to_str().unwrap())
        .set("ProductName", "Wortfluss")
        .set("FileDescription", "Wortfluss voice dictation")
        .set("CompanyName", "MIAK IT (Ammad Khan)")
        .set("LegalCopyright", "© 2026 Ammad Khan, MIAK IT. Apache License 2.0")
        .set("Comments", "Source code: https://github.com/1a-khan/wortfluss")
        .set("InternalName", "wortfluss")
        .set("ProductVersion", &version)
        .set("FileVersion", &version)
        .set_manifest_file("installer/wortfluss.manifest");
    res.compile().expect("compiling the Windows resources");
}
