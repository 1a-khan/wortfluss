//! Speech-to-text services vtt can use, with the models we recommend.
//! All speak the OpenAI-style `/audio/transcriptions` API. Prices are on-demand list
//! prices as of Nov 2026 and only used for estimates (`vtt usage`, setup descriptions).
//! `vtt setup` also asks each service which models the key can actually use, so new
//! models show up without changing this file.

pub const GROQ_URL: &str = "https://api.groq.com/openai/v1";

pub struct ModelInfo {
    pub id: &'static str,
    pub note: &'static str,
    /// US dollars per hour of audio.
    pub usd_per_hour: f64,
}

pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    pub blurb: &'static str,
    pub api_url: &'static str,
    pub key_url: &'static str,
    pub key_env: Option<&'static str>,
    pub needs_key: bool,
    /// Each request is billed as at least this long.
    pub min_billed_seconds: f64,
    /// Supports the `prompt`, `temperature` and `response_format` fields.
    pub openai_fields: bool,
    /// Path (after api_url) listing the models, incl. any query string.
    pub models_path: &'static str,
    /// The models list already contains only speech-to-text models.
    pub list_is_stt_only: bool,
    /// Path that fails with 401 for a bad key (when the models list is public).
    pub key_check_path: Option<&'static str>,
    /// Recommended first.
    pub models: &'static [ModelInfo],
}

pub const PROVIDERS: &[Provider] = &[
    Provider {
        id: "groq",
        name: "Groq",
        blurb: "fastest and cheapest, free plan available (recommended)",
        api_url: GROQ_URL,
        key_url: "https://console.groq.com/keys",
        key_env: Some("GROQ_API_KEY"),
        needs_key: true,
        min_billed_seconds: 10.0,
        openai_fields: true,
        models_path: "/models",
        list_is_stt_only: false,
        key_check_path: None,
        models: &[
            ModelInfo {
                id: "whisper-large-v3-turbo",
                note: "recommended: fastest, cheapest, very good accuracy",
                usd_per_hour: 0.04,
            },
            ModelInfo {
                id: "whisper-large-v3",
                note: "a bit more accurate for accents and mixed languages, ~3x the price",
                usd_per_hour: 0.111,
            },
        ],
    },
    Provider {
        id: "openai",
        name: "OpenAI",
        blurb: "very accurate, good punctuation, paid only",
        api_url: "https://api.openai.com/v1",
        key_url: "https://platform.openai.com/api-keys",
        key_env: Some("OPENAI_API_KEY"),
        needs_key: true,
        min_billed_seconds: 0.0,
        openai_fields: true,
        models_path: "/models",
        list_is_stt_only: false,
        key_check_path: None,
        models: &[
            ModelInfo {
                id: "gpt-4o-mini-transcribe",
                note: "recommended: accurate and cheap",
                usd_per_hour: 0.18,
            },
            ModelInfo {
                id: "gpt-transcribe",
                note: "newest OpenAI model, if your account lists it",
                usd_per_hour: 0.27,
            },
            ModelInfo {
                id: "gpt-4o-transcribe",
                note: "most accurate of the older generation",
                usd_per_hour: 0.36,
            },
            ModelInfo {
                id: "whisper-1",
                note: "older, being retired; only for compatibility",
                usd_per_hour: 0.36,
            },
        ],
    },
    Provider {
        id: "openrouter",
        name: "OpenRouter",
        blurb: "one key for 30+ speech models (OpenAI, ElevenLabs, Deepgram, Google ...)",
        api_url: "https://openrouter.ai/api/v1",
        key_url: "https://openrouter.ai/settings/keys",
        key_env: Some("OPENROUTER_API_KEY"),
        needs_key: true,
        min_billed_seconds: 0.0,
        openai_fields: true, // `prompt` is accepted but ignored
        models_path: "/models?output_modalities=transcription",
        list_is_stt_only: true,
        key_check_path: Some("/key"),
        models: &[
            ModelInfo {
                id: "openai/whisper-large-v3-turbo",
                note: "recommended: fast, very cheap, very good accuracy",
                usd_per_hour: 0.012,
            },
            ModelInfo {
                id: "openai/gpt-4o-mini-transcribe",
                note: "accurate, good punctuation",
                usd_per_hour: 0.18,
            },
            ModelInfo {
                id: "elevenlabs/scribe-v2",
                note: "top accuracy, 90+ languages",
                usd_per_hour: 0.22,
            },
            ModelInfo {
                id: "mistralai/voxtral-mini-transcribe",
                note: "strong in European languages",
                usd_per_hour: 0.18,
            },
            ModelInfo {
                id: "deepgram/nova-3",
                note: "fast, strong for English",
                usd_per_hour: 0.26,
            },
        ],
    },
    Provider {
        id: "mistral",
        name: "Mistral",
        blurb: "Voxtral, strong in European languages, EU company",
        api_url: "https://api.mistral.ai/v1",
        key_url: "https://console.mistral.ai/api-keys",
        key_env: Some("MISTRAL_API_KEY"),
        needs_key: true,
        min_billed_seconds: 0.0,
        openai_fields: false,
        models_path: "/models",
        list_is_stt_only: false,
        key_check_path: None,
        models: &[ModelInfo {
            id: "voxtral-mini-latest",
            note: "recommended: Voxtral Mini Transcribe, always the newest version",
            usd_per_hour: 0.18,
        }],
    },
    Provider {
        id: "custom",
        name: "Custom server",
        blurb: "any OpenAI-compatible server, e.g. a self-hosted Whisper (private, no per-use cost)",
        api_url: "",
        key_url: "",
        key_env: None,
        needs_key: false,
        min_billed_seconds: 0.0,
        openai_fields: true,
        models_path: "/models",
        list_is_stt_only: true,
        key_check_path: None,
        models: &[],
    },
];

/// Does the service use the phrases (prompt) while transcribing? OpenRouter accepts but
/// ignores them, Mistral has no such field; there the correction step applies them.
pub fn uses_prompt(id: &str) -> bool {
    !matches!(id, "openrouter" | "mistral")
}

pub fn get(id: &str) -> &'static Provider {
    PROVIDERS
        .iter()
        .find(|p| p.id == id)
        .unwrap_or(&PROVIDERS[0])
}

pub fn model_info(provider: &str, model: &str) -> Option<&'static ModelInfo> {
    get(provider).models.iter().find(|m| m.id == model)
}

/// Does a model id from the provider's /models list look like speech-to-text?
pub fn is_transcription_model(id: &str) -> bool {
    let id = id.to_lowercase();
    !id.contains("tts")
        && (id.contains("whisper")
            || id.contains("transcribe")
            || id.contains("voxtral-mini")
            || id.contains("stt"))
}

// ---- spelling & grammar correction (optional, off by default) ----

/// A text model used to correct the transcript, via the service's /chat/completions.
pub struct ChatModel {
    pub id: &'static str,
    pub note: &'static str,
    /// Extra request fields (JSON object) this model needs, e.g. to switch off "thinking"
    /// which would only add delay.
    pub extra: &'static str,
}

/// Recommended correction models per service, best first. Chosen for speed and for
/// keeping the speaker's wording (tested with German and English, Nov 2026).
pub fn chat_models(provider: &str) -> &'static [ChatModel] {
    match provider {
        "groq" => &[
            ChatModel {
                id: "qwen/qwen3.8-27b",
                note: "recommended: fast (~0.4 s), keeps your wording",
                extra: r#"{"reasoning_effort": "none"}"#,
            },
            ChatModel {
                id: "openai/gpt-oss-20b",
                note: "good, a little slower and may reword slightly",
                extra: r#"{"reasoning_effort": "low", "include_reasoning": false}"#,
            },
            ChatModel {
                id: "openai/gpt-oss-120b",
                note: "strongest on Groq, slowest",
                extra: r#"{"reasoning_effort": "low", "include_reasoning": false}"#,
            },
        ],
        "openrouter" => &[
            ChatModel {
                id: "anthropic/claude-haiku-5.5",
                note: "recommended: careful with wording, many languages",
                extra: "",
            },
            ChatModel {
                id: "openai/gpt-6-luna",
                note: "fast and cheap",
                extra: "",
            },
            ChatModel {
                id: "google/gemini-3.1-flash-lite",
                note: "fast and cheap",
                extra: "",
            },
            ChatModel {
                id: "mistralai/mistral-small-2603",
                note: "good for European languages",
                extra: "",
            },
        ],
        "openai" => &[ChatModel {
            id: "gpt-6-luna",
            note: "recommended: fast and cheap",
            extra: "",
        }],
        "mistral" => &[ChatModel {
            id: "mistral-small-latest",
            note: "recommended: fast, good for European languages",
            extra: "",
        }],
        _ => &[],
    }
}

/// Does a model id from a /models list look like a text (chat) model?
pub fn is_chat_model(id: &str) -> bool {
    let id = id.to_lowercase();
    ![
        "whisper", "transcribe", "tts", "stt", "asr", "guard", "embed", "orpheus", "image",
        "moderation", "realtime", "audio", "dall-e", "sora", "voxtral", "parakeet", "scribe",
        "chirp", "nova-3", "universal-3", "speech", "vision-only",
    ]
    .iter()
    .any(|w| id.contains(w))
}
