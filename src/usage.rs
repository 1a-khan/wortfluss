//! Local log of every transcription request, so `vtt usage` can show what the service
//! will bill and how much time dictating saved.
//! One line per request in ~/.local/share/vtt/usage.tsv:
//!   unix_time, audio_seconds, model, provider, cost, characters, spoken_seconds
//! The last two only for dictations (text typed into an app): the number of characters,
//! never the text itself, and how long the person spoke. Sessions and corrections leave
//! them empty, so they never count as saved typing.

use crate::providers;
use anyhow::{Context, Result};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn path() -> Result<PathBuf> {
    Ok(crate::platform::data_dir()?.join("usage.tsv"))
}

/// `audio_seconds` is 0 for a correction (text model) request.
pub fn record(provider: &str, model: &str, audio_seconds: f32, cost: Option<f64>) -> Result<()> {
    write(provider, model, audio_seconds, cost, None)
}

/// A dictation: also how many characters it typed and how long the person spoke.
pub fn record_dictation(
    provider: &str,
    model: &str,
    audio_seconds: f32,
    cost: Option<f64>,
    characters: usize,
    spoken_seconds: f32,
) -> Result<()> {
    write(provider, model, audio_seconds, cost, Some((characters, spoken_seconds)))
}

fn write(provider: &str, model: &str, audio_seconds: f32, cost: Option<f64>, typed: Option<(usize, f32)>) -> Result<()> {
    let path = path()?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    let cost = cost.map(|c| format!("{c:.8}")).unwrap_or_default();
    let typed = typed.map(|(c, s)| format!("\t{c}\t{s:.2}")).unwrap_or_default();
    writeln!(f, "{now}\t{audio_seconds:.2}\t{model}\t{provider}\t{cost}{typed}")?;
    Ok(())
}

// ---- time saved ---------------------------------------------------------------------

/// Typing speed assumed when none is set: 40 words per minute. A large study (Dhakal et
/// al. 2018, ~168,000 people) measured 40-52 for copying text; writing your own words is
/// slower, so 40 keeps the estimate on the safe side.
pub const DEFAULT_WPM: u32 = 40;
/// A "word" in typing speeds is 5 characters, spaces included.
const CHARS_PER_WORD: f64 = 5.0;
/// Large totals are shown in working days of this many hours ...
const WORKDAY_HOURS: f64 = 8.0;
/// ... and working months of this many days.
const WORKDAYS_PER_MONTH: f64 = 21.0;

/// Seconds saved by one dictation: the time typing it would have taken minus the time
/// spent speaking. Never negative (a slow, short dictation saved nothing, it lost a bit,
/// but that is not counted against the others either: one bad case shouldn't hide them).
pub fn saved_seconds(characters: usize, spoken_seconds: f64, wpm: u32) -> f64 {
    let per_minute = wpm.max(1) as f64 * CHARS_PER_WORD;
    (characters as f64 / per_minute * 60.0 - spoken_seconds).max(0.0)
}

/// "45 s", "12 min", "3 h 40 min".
pub fn duration(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min", s / 60)
    } else {
        format!("{} h {} min", s / 3600, s / 60 % 60)
    }
}

/// The duration, and for large totals what that is in working days or months:
/// "21 h 5 min (≈ 2.6 working days)".
pub fn duration_long(seconds: f64, l: crate::i18n::Lang) -> String {
    let days = seconds / 3600.0 / WORKDAY_HOURS;
    let base = duration(seconds);
    if days >= WORKDAYS_PER_MONTH {
        let months = format!("{:.1}", days / WORKDAYS_PER_MONTH);
        format!("{base} ({})", crate::i18n::tf_in(l, "saved.months", &[&months]))
    } else if days >= 1.0 {
        format!("{base} ({})", crate::i18n::tf_in(l, "saved.days", &[&format!("{days:.1}")]))
    } else {
        base
    }
}

/// Seconds saved today, this month and all time.
#[derive(Default, Debug, PartialEq)]
pub struct Saved {
    pub today: f64,
    pub month: f64,
    pub all: f64,
    pub dictations: u64,
}

/// Time saved according to the usage log, at `wpm` words per minute.
pub fn saved(wpm: u32) -> Saved {
    let Ok(text) = path().and_then(|p| Ok(std::fs::read_to_string(p)?)) else {
        return Saved::default();
    };
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    saved_in(&text, now, wpm)
}

fn saved_in(log: &str, now: i64, wpm: u32) -> Saved {
    let today = now.div_euclid(86_400);
    let this_month = year_month(today);
    let mut out = Saved::default();
    for line in log.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        let (Some(Ok(ts)), Some(Ok(chars)), Some(Ok(spoken))) = (
            f.first().map(|v| v.parse::<i64>()),
            f.get(5).map(|v| v.parse::<usize>()),
            f.get(6).map(|v| v.parse::<f64>()),
        ) else {
            continue; // not a dictation, or from before vtt counted characters
        };
        let s = saved_seconds(chars, spoken, wpm);
        let d = ts.div_euclid(86_400);
        if d == today {
            out.today += s;
        }
        if year_month(d) == this_month {
            out.month += s;
        }
        out.all += s;
        out.dictations += 1;
    }
    out
}

/// Tray line: "Time saved this month: 3 h 40 min", or None before the first dictation.
pub fn tray_line(wpm: u32) -> Option<String> {
    let s = saved(wpm);
    if s.dictations == 0 {
        return None;
    }
    Some(crate::i18n::tf("tray.saved", &[&duration_long(s.month, crate::i18n::lang())]))
}

#[derive(Default)]
struct Bucket {
    requests: u64,
    audio: f64,
    billed: f64,
    cost: f64,
    unpriced: u64,
    corrections: u64,
}

impl Bucket {
    fn add(&mut self, audio: f64, model: &str, provider: &str, cost: Option<f64>) {
        if audio == 0.0 {
            // spelling correction: tiny, only counted when the service reports the cost
            self.corrections += 1;
            self.cost += cost.unwrap_or(0.0);
            return;
        }
        let p = providers::get(provider);
        let billed = audio.max(p.min_billed_seconds);
        self.requests += 1;
        self.audio += audio;
        self.billed += billed;
        match (cost, providers::model_info(p.id, model)) {
            (Some(c), _) => self.cost += c, // reported by the service: exact
            (None, Some(m)) => self.cost += billed / 3600.0 * m.usd_per_hour,
            (None, None) => self.unpriced += 1,
        }
    }

    fn row(&self, label: &str) -> String {
        format!(
            "{label:<12} {:>8} {:>10.1} {:>10.1} {:>10} {:>11}",
            self.requests,
            self.audio / 60.0,
            self.billed / 60.0,
            format!("${:.4}{}", self.cost, if self.unpriced > 0 { "*" } else { "" }),
            self.corrections
        )
    }
}

pub fn report() -> Result<()> {
    let path = path()?;
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("No dictations recorded yet ({}).", path.display());
            return Ok(());
        }
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };

    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
    let today = now.div_euclid(86_400);
    let this_month = year_month(today);
    let (mut day, mut month, mut all) = (Bucket::default(), Bucket::default(), Bucket::default());

    for line in text.lines() {
        let mut parts = line.split('\t');
        let (Some(ts), Some(secs), model, provider, cost) =
            (parts.next(), parts.next(), parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let (Ok(ts), Ok(secs)) = (ts.parse::<i64>(), secs.parse::<f64>()) else {
            continue;
        };
        let model = model.unwrap_or("");
        let provider = provider.unwrap_or("groq"); // lines from vtt 0.1
        let cost = cost.and_then(|c| c.parse::<f64>().ok());
        let d = ts.div_euclid(86_400);
        if d == today {
            day.add(secs, model, provider, cost);
        }
        if year_month(d) == this_month {
            month.add(secs, model, provider, cost);
        }
        all.add(secs, model, provider, cost);
    }

    println!(
        "{:<12} {:>8} {:>10} {:>10} {:>10} {:>11}",
        "(UTC)", "requests", "audio min", "billed min", "est. cost", "corrections"
    );
    println!("{}", day.row("today"));
    println!("{}", month.row("this month"));
    println!("{}", all.row("all time"));
    println!(
        "\nBilled minutes include minimums (Groq counts every request as at least 10 s).\n\
         Estimates use list prices. On Groq's free plan you pay nothing.\n\
         * = includes models without a known price (custom server or unlisted model).\n\
         Spelling corrections cost a fraction of a cent each; included only where reported (OpenRouter).\n\
         Official numbers are in your provider's dashboard."
    );

    let wpm = crate::config::load().map(|c| c.typing_wpm).unwrap_or(DEFAULT_WPM);
    let saved = saved_in(&text, now, wpm);
    println!("\nTime saved by dictating instead of typing (at {wpm} words per minute):");
    if saved.dictations == 0 {
        println!("  nothing counted yet: dictations are counted from Wortfluss 0.6.0 on.");
    } else {
        println!("  {:<12} {}", "today", duration_long(saved.today, crate::i18n::Lang::En));
        println!("  {:<12} {}", "this month", duration_long(saved.month, crate::i18n::Lang::En));
        println!("  {:<12} {}", "all time", duration_long(saved.all, crate::i18n::Lang::En));
    }
    println!(
        "How: characters typed ÷ your typing speed, minus the time you spent speaking.\n\
         Sessions and rewrites don't count. Dictations are counted from Wortfluss 0.6.0 on\n\
         (older entries can't tell dictations from sessions). Working day = {WORKDAY_HOURS} h.\n\
         Your own speed: `vtt typing-test`, or `vtt set typing-speed`."
    );
    Ok(())
}

/// (year, month) of a day number counted from 1970-01-01 (UTC).
fn year_month(days: i64) -> (i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang::En;

    #[test]
    fn saved_time() {
        // 1000 characters at 40 wpm (200 per minute) = 5 min of typing; spoken in 60 s.
        assert_eq!(saved_seconds(1000, 60.0, 40), 240.0);
        // Faster typist saves less: 1000 at 100 wpm = 2 min, minus 60 s.
        assert_eq!(saved_seconds(1000, 60.0, 100), 60.0);
        // Never negative.
        assert_eq!(saved_seconds(10, 30.0, 40), 0.0);
        assert_eq!(saved_seconds(10, 0.0, 0), 120.0); // 0 wpm treated as 1
    }

    #[test]
    fn saved_from_log() {
        let now = 20_407 * 86_400 + 3600; // 2025-11-15 01:00 UTC
        let earlier_month = 20_380 * 86_400; // 2025-10-19
        let log = format!(
            "{now}\t10.00\twhisper\tgroq\t\t1000\t60.00\n\
             {now}\t30.00\twhisper\tgroq\t\n\
             {now}\t0.00\tqwen\tgroq\t0.0001\n\
             {earlier_month}\t5.00\twhisper\tgroq\t\t400\t0.00\n\
             1700000000\t12.00\twhisper\n\
             garbage\n"
        );
        let s = saved_in(&log, now, 40);
        assert_eq!(s, Saved { today: 240.0, month: 240.0, all: 360.0, dictations: 2 });
    }

    #[test]
    fn durations() {
        assert_eq!(duration(44.6), "45 s");
        assert_eq!(duration(12.0 * 60.0 + 59.0), "12 min");
        assert_eq!(duration(3.0 * 3600.0 + 40.0 * 60.0), "3 h 40 min");
        assert_eq!(duration_long(3600.0, En), "1 h 0 min");
        assert_eq!(duration_long(20.0 * 3600.0, En), "20 h 0 min (≈ 2.5 working days)");
        assert_eq!(duration_long(220.0 * 3600.0, En), "220 h 0 min (≈ 1.3 working months)");
    }

    #[test]
    fn dates() {
        assert_eq!(year_month(0), (1970, 1));
        assert_eq!(year_month(20_407), (2025, 11)); // 2025-11-15
        assert_eq!(year_month(19_782), (2024, 2)); // 2024-02-29
    }
}
