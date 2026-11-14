//! Notes from a session transcript: a text model of the configured service reads the
//! whole transcript and writes lecture notes, meeting minutes or a summary. The notes
//! go at the top of the transcript file, between two markers, so they can be redone
//! (`vtt notes <file> <type>`) without touching the transcript.

use crate::config::{self, Active};
use crate::{correct, usage};
use anyhow::{bail, Context, Result};
use std::path::Path;
use std::time::Duration;

pub const START: &str = "<!-- vtt:notes:start -->";
pub const END: &str = "<!-- vtt:notes:end -->";
pub const TRANSCRIPT_HEADING: &str = "## Transcript";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Lecture,
    Meeting,
    Summary,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        match s.trim().to_lowercase().as_str() {
            "lecture" | "lecture-notes" | "notes" | "tutorial" => Some(Kind::Lecture),
            "meeting" | "minutes" | "meeting-minutes" | "call" => Some(Kind::Meeting),
            "summary" | "short" => Some(Kind::Summary),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Lecture => "lecture notes",
            Kind::Meeting => "meeting minutes",
            Kind::Summary => "summary",
        }
    }

    fn structure(self) -> &'static str {
        match self {
            Kind::Lecture => {
                "### Summary\n3 to 6 sentences: what the lecture or video is about and its main message.\n\
                 ### Key points\nBullet points grouped by topic, in the order they came up. Keep facts, \
                 numbers, examples and steps exact.\n\
                 ### Terms and definitions\nBullets 'term: short explanation as given'. Leave out the \
                 section if there are none.\n\
                 ### Questions and things to look up\nOpen questions, unclear parts, references \
                 mentioned. Leave out the section if there are none."
            }
            Kind::Meeting => {
                "Each transcript line names who spoke: 'You' is the user, 'Others' the other \
                 participants together; 'Speaker 1', 'Speaker 2' ... or names are separate people. \
                 Attribute statements, decisions and tasks to these labels; don't guess real names \
                 for 'Speaker N' labels.\n\
                 ### Summary\n2 to 5 sentences.\n\
                 ### Discussion\nThe topics discussed, as bullet points.\n\
                 ### Decisions\nBullets. Write 'None recorded' if there were none.\n\
                 ### Action items\nTask list lines '- [ ] who: what (deadline if one was said)'. Write \
                 'None recorded' if there were none.\n\
                 ### Open questions\nLeave out the section if there are none."
            }
            Kind::Summary => {
                "### Summary\nOne paragraph.\n### Key points\nUp to 7 bullet points."
            }
        }
    }
}

/// The notes for a transcript (Markdown, without a title).
pub fn generate(a: &Active, model: &str, transcript: &str, kind: Kind) -> Result<(String, Option<f64>)> {
    if model.is_empty() {
        bail!("no text model for notes: choose one with `vtt set correction` (it doesn't have to be switched on)");
    }
    let system = format!(
        "You turn a transcript into clear, useful {kind}. The transcript was made by speech \
         recognition, so it may contain recognition errors: correct obvious ones silently, and \
         mark parts that are really unclear. Write in the language the transcript is in, \
         including the headings. Use only information from the transcript; never invent facts, \
         names or numbers. Use Markdown with exactly this structure:\n\n{structure}",
        kind = kind.name(),
        structure = kind.structure()
    );
    let (notes, cost) = correct::chat(a, model, &system, transcript, 3000, Duration::from_secs(180))
        .context("writing the notes failed")?;
    if notes.trim().is_empty() {
        bail!("the model returned no notes");
    }
    Ok((notes, cost))
}

/// The transcript part of a session file (everything after "## Transcript").
pub fn transcript_of(file_text: &str) -> &str {
    file_text
        .find(TRANSCRIPT_HEADING)
        .map(|i| &file_text[i + TRANSCRIPT_HEADING.len()..])
        .unwrap_or(file_text)
        .trim()
}

/// Put `notes` between the markers in the file text (or right after the title if there
/// are no markers).
pub fn insert(file_text: &str, notes: &str) -> String {
    let block = format!("{START}\n{}\n{END}", notes.trim());
    match (file_text.find(START), file_text.find(END)) {
        (Some(a), Some(b)) if b > a => {
            format!("{}{block}{}", &file_text[..a], &file_text[b + END.len()..])
        }
        _ => match file_text.find('\n') {
            Some(i) if file_text.starts_with("# ") => {
                format!("{}\n\n{block}\n{}", &file_text[..i], &file_text[i..])
            }
            _ => format!("{block}\n\n{file_text}"),
        },
    }
}

/// Write notes for the transcript in `path` into the same file. Returns a short status.
pub fn write_for_file(path: &Path, kind: Kind) -> Result<String> {
    let cfg = config::load()?;
    let a = cfg.active();
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let transcript = transcript_of(&text);
    if transcript.split_whitespace().count() < 10 {
        bail!("the transcript is (almost) empty, no notes written");
    }
    let started = std::time::Instant::now();
    let (notes, cost) = generate(&a, &a.correction_model, transcript, kind)?;
    let _ = usage::record(a.provider.id, &a.correction_model, 0.0, cost);
    let footer = format!(
        "\n\n_{} by {} in {:.0} s. Redo: `vtt notes \"{}\" lecture|meeting|summary`_",
        capitalize(kind.name()),
        a.correction_model,
        started.elapsed().as_secs_f32(),
        path.display()
    );
    crate::platform::write_private(path, &insert(&text, &format!("{notes}{footer}")))?;
    Ok(format!("{} written ({} words of transcript)", kind.name(), transcript.split_whitespace().count()))
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// `vtt notes FILE [lecture|meeting|summary]`
pub fn cmd(args: &[String]) -> Result<()> {
    let Some(file) = args.first() else {
        bail!("usage: vtt notes FILE [lecture|meeting|summary]");
    };
    let kind = match args.get(1) {
        Some(k) => Kind::parse(k).with_context(|| format!("unknown notes type `{k}`: lecture, meeting or summary"))?,
        None => Kind::Lecture,
    };
    println!("Writing {} for {} ...", kind.name(), file);
    let status = write_for_file(Path::new(file), kind)?;
    println!("Done: {status}. Open {file} to read them.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "# Lecture\n_meta_\n\n<!-- vtt:notes:start -->\n_Notes will appear here._\n<!-- vtt:notes:end -->\n\n## Transcript\n\n**[00:00:05]** Hello world.\n";

    #[test]
    fn notes_go_between_markers_and_can_be_redone() {
        let once = insert(FILE, "### Summary\nA");
        assert!(once.contains("<!-- vtt:notes:start -->\n### Summary\nA\n<!-- vtt:notes:end -->"));
        assert!(!once.contains("Notes will appear"));
        let twice = insert(&once, "### Summary\nB");
        assert!(twice.contains("### Summary\nB") && !twice.contains("### Summary\nA"));
        assert_eq!(transcript_of(&twice), "**[00:00:05]** Hello world.");
    }

    #[test]
    fn kinds() {
        assert_eq!(Kind::parse("Minutes"), Some(Kind::Meeting));
        assert_eq!(Kind::parse("x"), None);
    }
}
