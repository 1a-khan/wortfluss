//! Local log of every transcription request, so `vtt usage` can show what Groq will bill.
//! One line per request in ~/.local/share/vtt/usage.tsv: unix_time, audio_seconds, model.

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
    let path = path()?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    let cost = cost.map(|c| format!("{c:.8}")).unwrap_or_default();
    writeln!(f, "{now}\t{audio_seconds:.2}\t{model}\t{provider}\t{cost}")?;
    Ok(())
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
    use super::year_month;

    #[test]
    fn dates() {
        assert_eq!(year_month(0), (1970, 1));
        assert_eq!(year_month(20_407), (2025, 11)); // 2025-11-15
        assert_eq!(year_month(19_782), (2024, 2)); // 2024-02-29
    }
}
