//! Keyboard layout on Windows: nothing to detect. Text is typed as Unicode characters
//! (KEYEVENTF_UNICODE), which comes out right with every layout.

use crate::config::Config;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Layout {
    pub layout: String,
    pub variant: String,
}

impl std::fmt::Display for Layout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("any (Windows types characters directly)")
    }
}

fn any() -> Layout {
    Layout { layout: "windows".into(), variant: String::new() }
}

pub fn resolve(_cfg: &Config) -> Layout {
    any()
}

pub fn detect() -> (Layout, &'static str) {
    (any(), "no setting needed on Windows")
}
