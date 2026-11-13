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
    match send(a, language, prompt, wav) {
        Err(Failure::Retryable(first)) => {
            std::thread::sleep(Duration::from_millis(700));
            match send(a, language, prompt, wav) {
                Ok(text) => Ok(text),
                Err(Failure::Retryable(e) | Failure::Final(e)) => {
                    bail!("{e} (first attempt: {first})")
                }
            }
        }
        Err(Failure::Final(e)) => bail!("{e}"),
        Ok(text) => Ok(text),
    }
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
            let msg = resp.into_string().unwrap_or_default();
            Err(match code {
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
                    "{name} doesn't know model `{}` or the address is wrong (HTTP 404). Run `vtt setup`.\n{msg}",
                    a.model
                )),
                500..=599 => Failure::Retryable(format!("{name} server error HTTP {code}: {msg}")),
                _ => Failure::Final(format!("{name} returned HTTP {code}: {msg}")),
            })
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
