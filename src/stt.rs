//! Talks to the speech-to-text service (OpenAI-style HTTP API, used by all providers).

use crate::config::Active;
use crate::providers::{self, Provider};
use anyhow::{bail, Result};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct Transcript {
    pub text: String,
    /// Actual cost in USD, when the service reports it (OpenRouter does).
    pub cost: Option<f64>,
}

/// Transcribe a WAV clip, retrying once on network trouble or a server error.
pub fn transcribe(a: &Active, language: &str, prompt: &str, wav: &[u8]) -> Result<Transcript> {
    with_retry(|| send(a, language, prompt, wav))
}

/// A model offered by the service, as setup shows it.
pub struct ModelEntry {
    pub id: String,
    pub name: String,
    pub usd_per_hour: Option<f64>,
    /// Billed per token instead of per second (no simple hourly price).
    pub token_priced: bool,
}

fn get_json(url: &str, api_key: &str) -> Result<serde_json::Value> {
    let mut req = ureq::get(url).timeout(Duration::from_secs(15));
    if !api_key.is_empty() {
        req = req.set("Authorization", &format!("Bearer {api_key}"));
    }
    match req.call() {
        Ok(resp) => Ok(resp.into_json()?),
        Err(ureq::Error::Status(401 | 403, _)) => bail!("the API key was rejected"),
        Err(ureq::Error::Status(code, resp)) => {
            bail!("HTTP {code}: {}", resp.into_string().unwrap_or_default())
        }
        Err(e) => bail!("could not connect: {e}"),
    }
}

/// Is the key accepted?
pub fn check_key(p: &Provider, api_url: &str, api_key: &str) -> Result<()> {
    let path = p.key_check_path.unwrap_or(p.models_path);
    get_json(&format!("{}{path}", api_url.trim_end_matches('/')), api_key).map(|_| ())
}

/// Speech-to-text models the service offers to this key, sorted by id.
pub fn list_models(p: &Provider, api_url: &str, api_key: &str) -> Result<Vec<ModelEntry>> {
    let json = get_json(&format!("{}{}", api_url.trim_end_matches('/'), p.models_path), api_key)?;
    let mut out: Vec<ModelEntry> = json["data"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|m| {
            let id = m["id"].as_str()?.to_string();
            if !p.list_is_stt_only && !providers::is_transcription_model(&id) {
                return None;
            }
            let (usd_per_hour, token_priced) = price(&m["pricing"]);
            Some(ModelEntry {
                name: m["name"].as_str().unwrap_or("").to_string(),
                usd_per_hour: usd_per_hour
                    .or_else(|| providers::model_info(p.id, &id).map(|i| i.usd_per_hour)),
                token_priced,
                id,
            })
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// Text (chat) models the service offers, for the correction setting.
pub fn list_chat_models(p: &Provider, api_url: &str, api_key: &str) -> Result<Vec<ModelEntry>> {
    let json = get_json(&format!("{}/models", api_url.trim_end_matches('/')), api_key)?;
    let mut out: Vec<ModelEntry> = json["data"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|m| {
            let id = m["id"].as_str()?.to_string();
            if !providers::is_chat_model(&id) || id.ends_with(":batch") {
                return None;
            }
            let _ = p;
            Some(ModelEntry {
                name: m["name"].as_str().unwrap_or("").to_string(),
                usd_per_hour: None,
                token_priced: m["pricing"].is_object(),
                id,
            })
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// OpenRouter-style pricing: per-second audio price in `prompt` (completion "0"),
/// or per-token prices for models like GPT-4o Transcribe.
fn price(pricing: &serde_json::Value) -> (Option<f64>, bool) {
    let num = |k: &str| pricing[k].as_str().and_then(|v| v.parse::<f64>().ok());
    match (num("prompt"), num("completion")) {
        (Some(_), Some(c)) if c > 0.0 => (None, true),
        (Some(p), _) if p > 0.0 && p < 0.01 => (Some(p * 3600.0), false), // per second
        (Some(p), _) if p >= 0.01 => (Some(p), false),                      // already per hour
        _ => (None, false),
    }
}

enum Failure {
    Retryable(String),
    Final(String),
}

fn status_failure(name: &str, code: u16, msg: &str, model: &str) -> Failure {
    match code {
        402 => Failure::Final(format!(
            "{name}: not enough credit on the account (HTTP 402). Top up, or switch service with `vtt setup`"
        )),
        401 | 403 => Failure::Final(format!(
            "{name} rejected the API key (HTTP {code}). Run `vtt setup` to enter a new one"
        )),
        429 => Failure::Final(format!(
            "{name} rate limit or quota reached (HTTP 429); wait a bit or check your plan.\n{msg}"
        )),
        404 => Failure::Final(format!(
            "{name} doesn't know model `{model}` or the address is wrong (HTTP 404). Run `vtt setup`.\n{msg}"
        )),
        500..=599 => Failure::Retryable(format!("{name} server error HTTP {code}: {msg}")),
        _ => Failure::Final(format!("{name} returned HTTP {code}: {msg}")),
    }
}

/// Retry once on network trouble or a server error.
fn with_retry<T>(mut f: impl FnMut() -> Result<T, Failure>) -> Result<T> {
    match f() {
        Err(Failure::Retryable(first)) => {
            std::thread::sleep(Duration::from_millis(700));
            match f() {
                Ok(v) => Ok(v),
                Err(Failure::Retryable(e) | Failure::Final(e)) => bail!("{e} (first attempt: {first})"),
            }
        }
        Err(Failure::Final(e)) => bail!("{e}"),
        Ok(v) => Ok(v),
    }
}

fn multipart(boundary: &str, fields: &[(&str, &str)], wav: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(wav.len() + 1024);
    for (name, value) in fields {
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes(),
        );
    }
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(wav);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

// ---- speaker separation (diarization) ----

/// A stretch of speech by one speaker, times in seconds from the start of the audio.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub start: f32,
    pub end: f32,
    pub text: String,
    /// The service's label for the speaker ("0", "A", "speaker_1" ...), if it gave one.
    pub speaker: Option<String>,
}

pub struct Diarized {
    pub segments: Vec<Segment>,
    pub cost: Option<f64>,
}

/// Can this service label speakers, and with which model by default?
pub fn speaker_model(provider: &str) -> Option<&'static str> {
    match provider {
        "openrouter" => Some("elevenlabs/scribe-v2"),
        "openai" => Some("gpt-4o-transcribe-diarize"),
        "mistral" => Some("voxtral-mini-latest"),
        _ => None,
    }
}

/// Models known to label speakers (others may, too; the setting accepts any id).
pub fn speaker_models(provider: &str) -> &'static [(&'static str, &'static str)] {
    match provider {
        // Tested Nov 2026: Scribe v2 separates speakers through OpenRouter; Voxtral 3 there
        // returned no labels and Nova-3 no German text, so they aren't offered here.
        "openrouter" => &[("elevenlabs/scribe-v2", "recommended: separates speakers reliably, 90+ languages, ~$0.22/h")],
        "openai" => &[("gpt-4o-transcribe-diarize", "OpenAI's speaker-labelling model")],
        "mistral" => &[("voxtral-mini-latest", "Voxtral Mini Transcribe with speaker labels")],
        _ => &[],
    }
}

/// Transcribe with speaker labels.
pub fn diarize(a: &Active, model: &str, language: &str, wav: &[u8]) -> Result<Diarized> {
    with_retry(|| send_diarize(a, model, language, wav))
}

fn send_diarize(a: &Active, model: &str, language: &str, wav: &[u8]) -> Result<Diarized, Failure> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    let url = format!("{}/audio/transcriptions", a.api_url.trim_end_matches('/'));
    let name = a.provider.name;
    let language = language.trim();
    let timeout = Duration::from_secs(30 + wav.len() as u64 / 32_000); // + 1 s per second of audio
    let mut req = ureq::post(&url).timeout(timeout);
    if !a.api_key.is_empty() {
        req = req.set("Authorization", &format!("Bearer {}", a.api_key));
    }
    let reply = if a.provider.id == "openrouter" {
        // OpenRouter takes provider options only in the JSON form. The options are keyed by
        // the provider behind the model; unknown ones are ignored.
        let mut body = serde_json::json!({
            "model": model,
            "input_audio": { "data": B64.encode(wav), "format": "wav" },
            "response_format": "verbose_json",
            "timestamp_granularities": ["segment"],
            "provider": { "options": {
                "mistral": { "diarize": true },
                "elevenlabs": { "diarize": true },
                "deepgram": { "diarize": true },
                "azure": { "diarization": { "enabled": true } },
            }},
        });
        if !language.is_empty() {
            body["language"] = language.into();
        }
        req.set("Content-Type", "application/json").send_string(&body.to_string())
    } else {
        let boundary = format!("----vtt{:x}", SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        let mut fields: Vec<(&str, &str)> = vec![("model", model)];
        if !language.is_empty() {
            fields.push(("language", language));
        }
        if a.provider.id == "mistral" {
            fields.push(("diarize", "true"));
            fields.push(("timestamp_granularities", "segment"));
        } else {
            fields.push(("response_format", "diarized_json"));
            fields.push(("chunking_strategy", "auto"));
        }
        req.set("Content-Type", &format!("multipart/form-data; boundary={boundary}"))
            .send_bytes(&multipart(&boundary, &fields, wav))
    };
    match reply {
        Ok(resp) => {
            let json: serde_json::Value = resp
                .into_json()
                .map_err(|e| Failure::Retryable(format!("bad reply from {name}: {e}")))?;
            if std::env::var_os("VTT_DEBUG_DIARIZE").is_some() {
                eprintln!("{}", serde_json::to_string_pretty(&json).unwrap_or_default());
            }
            Ok(Diarized { segments: parse_segments(&json), cost: json["usage"]["cost"].as_f64() })
        }
        Err(ureq::Error::Status(code, resp)) => {
            Err(status_failure(name, code, &resp.into_string().unwrap_or_default(), model))
        }
        Err(e) => Err(Failure::Retryable(format!("could not reach {name}: {e}"))),
    }
}

fn label_of(v: &serde_json::Value) -> Option<String> {
    for k in ["speaker", "speaker_id", "speaker_label"] {
        match &v[k] {
            serde_json::Value::String(s) if !s.trim().is_empty() => return Some(s.trim().to_string()),
            serde_json::Value::Number(n) => return Some(n.to_string()),
            _ => {}
        }
    }
    None
}

/// Segments from the reply. Services differ: the label may be on the segments, only on
/// the words, or missing (then the whole text is one unlabelled segment).
pub fn parse_segments(json: &serde_json::Value) -> Vec<Segment> {
    let num = |v: &serde_json::Value| v.as_f64().unwrap_or(0.0) as f32;
    let words: Vec<(f32, f32, String, Option<String>)> = json["words"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|w| {
            let text = w["word"].as_str().or(w["text"].as_str()).unwrap_or("").to_string();
            (num(&w["start"]), num(&w["end"]), text, label_of(w))
        })
        .collect();
    let mut out: Vec<Segment> = Vec::new();
    if words.iter().any(|w| w.3.is_some()) {
        // Labelled words (the finest information): join runs of the same speaker.
        let words: Vec<_> = words.iter().cloned().filter(|w| !w.2.trim().is_empty()).collect();
        for (start, end, text, speaker) in words {
            match out.last_mut() {
                // a new segment also after a clear pause (keeps parts sent together apart)
                Some(last) if last.speaker == speaker && start - last.end < 0.8 => {
                    last.end = end;
                    if !text.starts_with(' ') && !last.text.is_empty() {
                        last.text.push(' ');
                    }
                    last.text.push_str(&text);
                }
                _ => out.push(Segment { start, end, text: text.trim().to_string(), speaker }),
            }
        }
        for s in &mut out {
            s.text = s.text.split_whitespace().collect::<Vec<_>>().join(" ");
        }
    }
    if out.is_empty() {
        out = json["segments"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|s| {
            let (start, end) = (num(&s["start"]), num(&s["end"]));
            let speaker = label_of(s);
            Segment { start, end, text: s["text"].as_str().unwrap_or("").trim().to_string(), speaker }
        })
        .filter(|s| !s.text.is_empty())
        .collect();
    }
    if out.is_empty() {
        if let Some(t) = json["text"].as_str().filter(|t| !t.trim().is_empty()) {
            out.push(Segment { start: 0.0, end: num(&json["duration"]), text: t.trim().to_string(), speaker: None });
        }
    }
    out
}

fn send(a: &Active, language: &str, prompt: &str, wav: &[u8]) -> Result<Transcript, Failure> {
    let url = format!("{}/audio/transcriptions", a.api_url.trim_end_matches('/'));
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let boundary = format!("----vtt{nanos:x}");

    let mut body = Vec::with_capacity(wav.len() + 1024);
    let mut field = |name: &str, value: &str| {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    };
    field("model", &a.model);
    if !language.trim().is_empty() {
        field("language", language.trim());
    }
    if a.provider.openai_fields {
        field("response_format", "json");
        field("temperature", "0");
        if !prompt.trim().is_empty() {
            field("prompt", prompt.trim());
        }
    }
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(wav);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let mut req = ureq::post(&url)
        .set(
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        )
        .timeout(Duration::from_secs(30));
    if !a.api_key.is_empty() {
        req = req.set("Authorization", &format!("Bearer {}", a.api_key));
    }
    let name = a.provider.name;
    match req.send_bytes(&body) {
        Ok(resp) => {
            let json: serde_json::Value = resp
                .into_json()
                .map_err(|e| Failure::Retryable(format!("bad reply from {name}: {e}")))?;
            Ok(Transcript {
                text: json["text"].as_str().unwrap_or_default().trim().to_string(),
                cost: json["usage"]["cost"].as_f64(),
            })
        }
        Err(ureq::Error::Status(code, resp)) => {
            Err(status_failure(name, code, &resp.into_string().unwrap_or_default(), &a.model))
        }
        Err(e) => Err(Failure::Retryable(format!("could not reach {name}: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::price;
    use serde_json::json;

    #[test]
    fn openrouter_prices() {
        // per second of audio -> per hour
        let (h, tok) = price(&json!({"prompt": "0.00000333", "completion": "0"}));
        assert!((h.unwrap() - 0.012).abs() < 0.001 && !tok);
        // token-priced
        assert_eq!(price(&json!({"prompt": "0.00000125", "completion": "0.000005"})), (None, true));
        assert_eq!(price(&json!(null)), (None, false));
    }
}
