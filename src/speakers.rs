//! Speaker separation ("who said what") for sessions.
//!
//! A speaker-labelling service names the voices in one request "0, 1, 2" or "A, B", but
//! a session is sent in pieces, and each piece would start counting again. To keep
//! "Speaker 1" the same person for the whole session, every piece is sent with a short
//! sample (about 3 s) of each speaker already known in front of it:
//!
//!   [sample Speaker 1] pause [sample Speaker 2] pause [the new piece ...]
//!
//! Whatever label the service gives the voice in sample 1 is Speaker 1 in this piece too.
//! Voices that match no sample are new speakers; their first clear stretch becomes their
//! sample. This works with any service that labels speakers and costs only the few
//! seconds of samples per piece. Afterwards `vtt speakers FILE` puts real names in.

use crate::audio::{Clip, TARGET_RATE};
use crate::config::Active;
use crate::stt::{self, Segment};
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::Path;

/// Length of a voice sample, and of the silence between the parts sent together.
const SAMPLE_SECS: f32 = 3.0;
const GAP_SECS: f32 = 1.5;
/// At most this many samples go in front of a piece (more speakers still get names,
/// but may not keep the same one across pieces).
const MAX_SAMPLES: usize = 5;

/// What was said in a piece, by whom.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    /// Seconds from the start of the piece.
    pub at: f32,
    /// "Speaker 1" ...; empty when the service gave no labels.
    pub speaker: String,
    pub text: String,
}

#[derive(Default)]
pub struct Tracker {
    /// (name, voice sample at 16 kHz)
    samples: Vec<(String, Vec<f32>)>,
    /// Number for the next new speaker.
    next: usize,
}

impl Tracker {
    /// Continue numbering after `last` (e.g. after a restart, so names don't repeat).
    pub fn starting_after(last: usize) -> Self {
        Tracker { samples: Vec::new(), next: last }
    }

    /// Transcribe a piece with speaker labels.
    pub fn transcribe(&mut self, a: &Active, model: &str, language: &str, clip: &Clip) -> Result<(Vec<Turn>, Option<f64>)> {
        let rate = TARGET_RATE as f32;
        let gap = vec![0.0f32; (GAP_SECS * rate) as usize];
        let mut audio = Vec::new();
        let mut ranges = Vec::new(); // where each sample is, in seconds
        for (name, s) in &self.samples {
            let start = audio.len() as f32 / rate;
            audio.extend_from_slice(s);
            ranges.push((name.clone(), start, audio.len() as f32 / rate));
            audio.extend_from_slice(&gap);
        }
        let prefix = audio.len() as f32 / rate;
        audio.extend_from_slice(&clip.samples);
        let reply = stt::diarize(a, model, language, &Clip { samples: audio }.to_wav())?;
        let turns = self.assign(&reply.segments, &ranges, prefix, &clip.samples);
        Ok((turns, reply.cost))
    }

    /// Map the service's labels to session names, learn new voices, and return the turns
    /// of the piece itself (the samples' own text is dropped).
    fn assign(&mut self, segs: &[Segment], ranges: &[(String, f32, f32)], prefix: f32, piece: &[f32]) -> Vec<Turn> {
        // 1. Labels heard in the samples belong to those speakers.
        let mut names: HashMap<String, String> = HashMap::new();
        for (name, start, end) in ranges {
            let mut best: Option<(&str, f32)> = None;
            for s in segs {
                let overlap = s.end.min(*end) - s.start.max(*start);
                if let (Some(l), true) = (&s.speaker, overlap > 0.3) {
                    if best.is_none_or(|b| overlap > b.1) {
                        best = Some((l, overlap));
                    }
                }
            }
            if let Some((l, _)) = best {
                names.entry(l.to_string()).or_insert_with(|| name.clone());
            }
        }
        // 2. The piece: new labels are new speakers.
        let mut turns: Vec<Turn> = Vec::new();
        let mut first_seen: Vec<(String, f32, f32)> = Vec::new(); // candidate samples
        for s in segs.iter().filter(|s| (s.start + s.end) / 2.0 >= prefix - 0.2) {
            let speaker = match &s.speaker {
                Some(l) => names
                    .entry(l.clone())
                    .or_insert_with(|| {
                        self.next += 1;
                        format!("Speaker {}", self.next)
                    })
                    .clone(),
                None => String::new(),
            };
            let (start, end) = ((s.start - prefix).max(0.0), (s.end - prefix).max(0.0));
            if !speaker.is_empty() && end - start >= 1.5 {
                match first_seen.iter_mut().find(|c| c.0 == speaker) {
                    Some(c) if end - start > c.2 - c.1 => (c.1, c.2) = (start, end),
                    Some(_) => {}
                    None => first_seen.push((speaker.clone(), start, end)),
                }
            }
            match turns.last_mut() {
                Some(t) if t.speaker == speaker => {
                    t.text.push(' ');
                    t.text.push_str(&s.text);
                }
                _ => turns.push(Turn { at: start, speaker, text: s.text.clone() }),
            }
        }
        // 3. Keep a sample of every new voice (and a better one for a short sample).
        let rate = TARGET_RATE as f32;
        for (name, start, end) in first_seen {
            let from = ((start + 0.1) * rate) as usize;
            let to = (((start + 0.1 + SAMPLE_SECS).min(end)) * rate) as usize;
            let Some(sample) = piece.get(from..to.min(piece.len())).filter(|s| s.len() > rate as usize) else {
                continue;
            };
            let full = self.samples.len() >= MAX_SAMPLES;
            match self.samples.iter_mut().find(|s| s.0 == name) {
                Some(s) if s.1.len() < sample.len() => s.1 = sample.to_vec(),
                Some(_) => {}
                None if !full => self.samples.push((name, sample.to_vec())),
                None => {}
            }
        }
        turns
    }
}

/// The highest "Speaker N" in a transcript (to continue numbering after a restart).
pub fn last_number(text: &str) -> usize {
    labels(text)
        .iter()
        .filter_map(|(l, _)| l.strip_prefix("Speaker ")?.parse::<usize>().ok())
        .max()
        .unwrap_or(0)
}

/// The speaker labels in a transcript ("**[00:01:02] Speaker 1:** ..."), with their
/// number of words, in order of first appearance.
pub fn labels(text: &str) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("**[") else { continue };
        let Some(close) = rest.find("] ") else { continue };
        let rest = &rest[close + 2..];
        let Some(end) = rest.find(":**") else { continue };
        let label = rest[..end].trim().to_string();
        let words = rest[end + 3..].split_whitespace().count();
        match out.iter_mut().find(|l| l.0 == label) {
            Some(l) => l.1 += words,
            None => out.push((label, words)),
        }
    }
    out
}

/// Rename a speaker everywhere in the file: transcript labels and the notes. "Speaker 1"
/// doesn't touch "Speaker 12".
pub fn rename(text: &str, old: &str, new: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(old) {
        let after = &rest[i + old.len()..];
        let before_ok = rest[..i].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let after_ok = after.chars().next().is_none_or(|c| !c.is_alphanumeric());
        out.push_str(&rest[..i]);
        out.push_str(if before_ok && after_ok { new } else { old });
        rest = after;
    }
    out.push_str(rest);
    out
}

/// `vtt speakers FILE [OLD=NEW ...]`: give the speakers in a transcript real names.
pub fn cmd(args: &[String]) -> Result<()> {
    let file = args.first().context("usage: vtt speakers FILE [\"Speaker 1=Anna\" ...]")?;
    let path = Path::new(file);
    let mut text = std::fs::read_to_string(path).with_context(|| format!("reading {file}"))?;
    let found = labels(&text);
    if found.is_empty() {
        bail!("no speaker labels in {file} (was \"Separate speakers\" on for this session?)");
    }
    let mut changes: Vec<(String, String)> = Vec::new();
    if args.len() > 1 {
        for a in &args[1..] {
            let (old, new) = a.split_once('=').with_context(|| format!("`{a}`: write OLD=NEW, e.g. \"Speaker 1=Anna\""))?;
            if !found.iter().any(|f| f.0 == old.trim()) {
                bail!("`{}` doesn't appear in the transcript", old.trim());
            }
            changes.push((old.trim().to_string(), new.trim().to_string()));
        }
    } else {
        println!("Speakers in this transcript (Enter keeps the name):\n");
        let theme = dialoguer::theme::ColorfulTheme::default();
        for (label, words) in &found {
            if let Some(line) = text.lines().find(|l| l.contains(&format!("] {label}:**"))) {
                let quote: String = line.split(":**").nth(1).unwrap_or("").trim().chars().take(90).collect();
                println!("  {label} ({words} words): \"{quote}...\"");
            }
            let name: String = dialoguer::Input::with_theme(&theme)
                .with_prompt(format!("Name for {label}"))
                .default(label.clone())
                .interact_text()?;
            if name.trim() != label && !name.trim().is_empty() {
                changes.push((label.clone(), name.trim().to_string()));
            }
        }
    }
    if changes.is_empty() {
        println!("Nothing changed.");
        return Ok(());
    }
    // Two steps, so swapping names ("Speaker 1" <-> "Speaker 2") works.
    for (i, (old, _)) in changes.iter().enumerate() {
        text = rename(&text, old, &format!("\u{1}{i}\u{1}"));
    }
    for (i, (_, new)) in changes.iter().enumerate() {
        text = text.replace(&format!("\u{1}{i}\u{1}"), new);
    }
    crate::platform::write_private(path, &text)?;
    for (old, new) in &changes {
        println!("{old} -> {new}");
    }
    println!("Saved. The notes use the new names too.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f32, end: f32, who: &str, text: &str) -> Segment {
        Segment { start, end, text: text.into(), speaker: Some(who.into()) }
    }

    #[test]
    fn samples_keep_names_across_pieces() {
        let mut t = Tracker::default();
        let piece = vec![0.1f32; TARGET_RATE as usize * 20];
        // First piece, no samples yet: two voices.
        let turns = t.assign(&[seg(0.0, 4.0, "A", "Hallo."), seg(4.5, 9.0, "B", "Hi."), seg(9.5, 10.0, "A", "Ja.")], &[], 0.0, &piece);
        assert_eq!(turns.iter().map(|t| t.speaker.as_str()).collect::<Vec<_>>(), ["Speaker 1", "Speaker 2", "Speaker 1"]);
        assert_eq!(t.samples.len(), 2);
        // Second piece: the service now calls Speaker 2 "A" and Speaker 1 "B"; a third voice appears.
        let ranges = vec![("Speaker 1".to_string(), 0.0, 3.0), ("Speaker 2".to_string(), 4.0, 7.0)];
        let segs = [
            seg(0.0, 3.0, "B", "sample one"),
            seg(4.0, 7.0, "A", "sample two"),
            seg(8.2, 10.0, "A", "Ich bin zwei."),
            seg(10.5, 12.0, "C", "Ich bin neu."),
            seg(12.5, 13.0, "B", "Eins."),
        ];
        let turns = t.assign(&segs, &ranges, 8.0, &piece);
        let got: Vec<(f32, &str, &str)> = turns.iter().map(|t| ((t.at * 10.0).round() / 10.0, t.speaker.as_str(), t.text.as_str())).collect();
        assert_eq!(got, [(0.2, "Speaker 2", "Ich bin zwei."), (2.5, "Speaker 3", "Ich bin neu."), (4.5, "Speaker 1", "Eins.")]);
    }

    #[test]
    fn labels_and_renaming() {
        let text = "# M\n\n- Speaker 1 decided\n\n## Transcript\n\n**[00:00:01] Speaker 1:** a b\n\n**[00:00:05] Speaker 12:** c\n\n**[00:00:09] Speaker 1:** d\n";
        assert_eq!(labels(text), vec![("Speaker 1".to_string(), 3), ("Speaker 12".to_string(), 1)]);
        assert_eq!(last_number(text), 12);
        let r = rename(text, "Speaker 1", "Anna");
        assert!(r.contains("- Anna decided") && r.contains("] Anna:** a b") && r.contains("] Speaker 12:** c"));
    }
}
