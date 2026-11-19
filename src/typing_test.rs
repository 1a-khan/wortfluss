//! `vtt typing-test`: measures how fast the person types, so "time saved" uses their own
//! speed instead of the average.
//!
//! One sentence is shown; the clock runs from Enter to Enter. The result is the usual
//! "net" speed: characters typed ÷ 5 per minute, reduced by the share of mistakes
//! (edit distance to the sentence). Copying is faster than writing your own words, so a
//! measured speed is on the high side: that makes the time saved smaller, never larger.

use anyhow::Result;
use std::io::{BufRead, Write};
use std::time::Instant;

const ENGLISH: [&str; 3] = [
    "Please send me the updated report by Friday, and let me know if the numbers for the second quarter have changed since our last meeting.",
    "The new version is faster and easier to use, but we still need to test it on older computers before we give it to all of our customers.",
    "Thank you for your quick reply. I have looked at the plan and I think we can start next week if the team has enough time to prepare.",
];

const GERMAN: [&str; 3] = [
    "Bitte schicken Sie mir den aktualisierten Bericht bis Freitag und sagen Sie mir, ob sich die Zahlen seit unserem letzten Treffen geändert haben.",
    "Die neue Version ist schneller und einfacher zu bedienen, aber wir müssen sie noch auf älteren Rechnern testen, bevor alle Kunden sie bekommen.",
    "Vielen Dank für Ihre schnelle Antwort. Ich habe mir den Plan angesehen und denke, dass wir nächste Woche anfangen können, wenn das Team Zeit hat.",
];

/// Words per minute (net) and accuracy (0-1) for `typed` against `target` in `seconds`.
pub fn score(target: &str, typed: &str, seconds: f64) -> (u32, f64) {
    let typed = typed.trim();
    let target: Vec<char> = target.chars().collect();
    let got: Vec<char> = typed.chars().collect();
    if got.is_empty() || seconds <= 0.0 {
        return (0, 0.0);
    }
    let errors = distance(&target, &got) as f64;
    let accuracy = (1.0 - errors / target.len().max(got.len()) as f64).max(0.0);
    let gross = got.len() as f64 / 5.0 / (seconds / 60.0);
    ((gross * accuracy).round() as u32, accuracy)
}

/// Levenshtein distance (characters to insert, delete or change).
fn distance(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let change = prev[j] + usize::from(ca != cb);
            cur[j + 1] = change.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Runs the test in the terminal. `language`: the dictation language ("de" gets a German
/// sentence, everything else English). Returns the measured speed, None if skipped.
pub fn run(language: &str) -> Result<Option<u32>> {
    let texts = if language.to_lowercase().starts_with("de") { GERMAN } else { ENGLISH };
    let pick = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as usize)
        .unwrap_or(0)
        % texts.len();
    let target = texts[pick];
    let stdin = std::io::stdin();
    let mut line = String::new();

    println!("\nTyping test: type the sentence below as you normally would, then press Enter.");
    println!("Mistakes count, but don't stop to fix every one. Your text is not stored.");
    print!("\nPress Enter to start (or type q and Enter to cancel) ");
    std::io::stdout().flush()?;
    stdin.lock().read_line(&mut line)?;
    if line.trim().eq_ignore_ascii_case("q") {
        return Ok(None);
    }

    println!("\n  {target}\n");
    print!("> ");
    std::io::stdout().flush()?;
    let start = Instant::now();
    line.clear();
    stdin.lock().read_line(&mut line)?;
    let seconds = start.elapsed().as_secs_f64();

    let (wpm, accuracy) = score(target, &line, seconds);
    if wpm == 0 || accuracy < 0.5 {
        println!("\nThat doesn't look like the sentence (accuracy {:.0}%). Nothing measured.", accuracy * 100.0);
        return Ok(None);
    }
    println!(
        "\nYou typed {wpm} words per minute ({:.0} s, {:.0}% accurate).",
        seconds,
        accuracy * 100.0
    );
    println!("For comparison: most people type 40-50 when copying, fewer when writing their own words.");
    Ok(Some(wpm))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfect_and_sloppy() {
        let target = "a".repeat(100);
        // 100 characters in 30 s = 20 words in half a minute = 40 wpm.
        assert_eq!(score(&target, &target, 30.0), (40, 1.0));
        // 10 wrong characters: 90% accurate, 36 wpm.
        let sloppy = format!("{}{}", "a".repeat(90), "b".repeat(10));
        let (wpm, acc) = score(&target, &sloppy, 30.0);
        assert_eq!(wpm, 36);
        assert!((acc - 0.9).abs() < 1e-9);
        // Nothing typed, or nonsense.
        assert_eq!(score(&target, "  \n", 30.0), (0, 0.0));
        assert_eq!(score(&target, "xyz", 30.0).0, 0);
    }

    #[test]
    fn umlauts_are_one_character() {
        let t = "Grüße aus Köln";
        assert_eq!(score(t, t, 6.0).1, 1.0);
        assert_eq!(distance(&"Köln".chars().collect::<Vec<_>>(), &"Koln".chars().collect::<Vec<_>>()), 1);
    }

    #[test]
    fn sentences_are_long_enough() {
        for t in ENGLISH.iter().chain(GERMAN.iter()) {
            assert!(t.chars().count() > 120, "{t}");
        }
    }
}
