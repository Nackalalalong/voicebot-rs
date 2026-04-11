---
name: Configuration
---

# Skill: Configuration

Use this whenever working on config loading, environment variable substitution, startup validation, or provider factory setup in `voicebot/crates/common` or `voicebot/crates/core`.

## Config file location

`config.toml` at the project root. Parsed at startup, never reloaded at runtime.

## Config struct hierarchy

```rust
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub session_defaults: SessionDefaultsConfig,
    pub vad: VadConfig,
    pub asr: AsrConfigGroup,
    pub llm: LlmConfigGroup,
    pub tts: TtsConfigGroup,
    pub channels: ChannelConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub host: String,    // "0.0.0.0"
    pub port: u16,       // 8080
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionDefaultsConfig {
    pub language: String,        // "auto" | "th" | "en"
    pub asr_provider: String,    // "deepgram" | "whisper"
    pub tts_provider: String,    // "elevenlabs" | "coqui"
    pub llm_provider: String,    // "openai" | "anthropic"
}

#[derive(Debug, Clone, Deserialize)]
pub struct VadConfig {
    pub silence_ms: u32,          // default 800
    pub min_speech_ms: u32,       // default 200
    pub energy_threshold: f32,    // default 0.02
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            silence_ms: 800,
            min_speech_ms: 200,
            energy_threshold: 0.02,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AsrConfigGroup {
    pub primary: String,                   // "deepgram"
    pub fallback: Option<String>,          // "whisper"
    pub deepgram: Option<DeepgramConfig>,
    pub whisper: Option<WhisperConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeepgramConfig {
    pub api_key: String,    // "${DEEPGRAM_API_KEY}" — resolved at load time
    pub model: String,      // "nova-2"
    pub language: String,   // "th"
}

#[derive(Debug, Clone, Deserialize)]
pub struct WhisperConfig {
    pub model_path: String,  // "./models/ggml-base.bin"
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfigGroup {
    pub primary: String,
    pub fallback: Option<String>,
    pub openai: Option<OpenAiConfig>,
    pub anthropic: Option<AnthropicConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiConfig {
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnthropicConfig {
    pub api_key: String,
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TtsConfigGroup {
    pub elevenlabs: Option<ElevenLabsConfig>,
    pub coqui: Option<CoquiConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ElevenLabsConfig {
    pub api_key: String,
    pub voice_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CoquiConfig {
    pub model_path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChannelConfig {
    pub audio_ingress_capacity: usize,  // default 50
    pub event_bus_capacity: usize,      // default 200
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            audio_ingress_capacity: 50,
            event_bus_capacity: 200,
        }
    }
}
```

## Loading and env var substitution

`${VAR}` tokens in TOML values MUST be resolved from environment variables at startup.

```rust
use std::env;

pub fn load_config(path: &str) -> Result<AppConfig, ConfigError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| ConfigError::FileNotFound(path.into(), e))?;

    // Substitute ${VAR} patterns with environment variable values
    let resolved = substitute_env_vars(&raw)?;

    let config: AppConfig = toml::from_str(&resolved)
        .map_err(|e| ConfigError::ParseError(e.to_string()))?;

    validate_config(&config)?;

    Ok(config)
}

fn substitute_env_vars(input: &str) -> Result<String, ConfigError> {
    let re = regex::Regex::new(r"\$\{([^}]+)\}").unwrap();
    let mut result = input.to_string();
    let mut missing = Vec::new();

    for cap in re.captures_iter(input) {
        let var_name = &cap[1];
        match env::var(var_name) {
            Ok(value) => {
                result = result.replace(&cap[0], &value);
            }
            Err(_) => {
                missing.push(var_name.to_string());
            }
        }
    }

    if !missing.is_empty() {
        return Err(ConfigError::MissingEnvVars(missing));
    }

    Ok(result)
}
```

## Fail-fast validation

Panic at startup if required values are missing or invalid. Never proceed with a broken config.

```rust
fn validate_config(config: &AppConfig) -> Result<(), ConfigError> {
    // Server
    if config.server.port == 0 {
        return Err(ConfigError::Invalid("server.port must be non-zero".into()));
    }

    // Validate that the selected primary providers have config sections
    match config.session_defaults.asr_provider.as_str() {
        "deepgram" => {
            if config.asr.deepgram.is_none() {
                return Err(ConfigError::Invalid(
                    "asr_provider is 'deepgram' but [asr.deepgram] section is missing".into()
                ));
            }
        }
        "whisper" => {
            if config.asr.whisper.is_none() {
                return Err(ConfigError::Invalid(
                    "asr_provider is 'whisper' but [asr.whisper] section is missing".into()
                ));
            }
        }
        other => {
            return Err(ConfigError::Invalid(format!("unknown asr_provider: {}", other)));
        }
    }

    // ... similar for llm_provider, tts_provider

    // VAD thresholds
    if config.vad.energy_threshold < 0.0 || config.vad.energy_threshold > 1.0 {
        return Err(ConfigError::Invalid(
            "vad.energy_threshold must be between 0.0 and 1.0".into()
        ));
    }

    Ok(())
}
```

## Error type

```rust
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config file not found: {0}: {1}")]
    FileNotFound(String, std::io::Error),

    #[error("config parse error: {0}")]
    ParseError(String),

    #[error("missing required environment variables: {0:?}")]
    MissingEnvVars(Vec<String>),

    #[error("invalid configuration: {0}")]
    Invalid(String),
}
```

## Env vars override TOML values

Environment variables take priority. Check env before falling back to TOML.

```rust
// Pattern: env var overrides TOML
let host = env::var("VOICEBOT_HOST")
    .unwrap_or_else(|_| config.server.host.clone());
let port = env::var("VOICEBOT_PORT")
    .ok()
    .and_then(|p| p.parse().ok())
    .unwrap_or(config.server.port);
```

## SessionConfig construction from defaults + per-session overrides

When a WebSocket client sends `session_start`, merge its settings with defaults:

```rust
pub fn build_session_config(
    session_id: Uuid,
    defaults: &SessionDefaultsConfig,
    client_overrides: &ClientSessionStart,
    vad_config: &VadConfig,
) -> SessionConfig {
    SessionConfig {
        session_id,
        language: client_overrides.language
            .as_ref()
            .and_then(|l| Language::from_str(l).ok())
            .unwrap_or_else(|| Language::from_str(&defaults.language).unwrap_or(Language::Auto)),
        asr_provider: AsrProvider::from_str(
            client_overrides.asr.as_deref().unwrap_or(&defaults.asr_provider)
        ).unwrap_or(AsrProvider::Deepgram),
        tts_provider: TtsProvider::from_str(
            client_overrides.tts.as_deref().unwrap_or(&defaults.tts_provider)
        ).unwrap_or(TtsProvider::ElevenLabs),
        llm_provider: LlmProvider::from_str(&defaults.llm_provider)
            .unwrap_or(LlmProvider::OpenAi),
        vad_config: vad_config.clone(),
    }
}
```

## Secrets handling

- **All secrets come from environment variables.** Never hardcode API keys.
- `${VAR}` tokens in TOML are resolved at startup.
- If a required secret is missing, fail fast with a clear error message listing the missing vars.
- Never log secret values. Log the key name only.

```rust
// Correct logging
tracing::info!("loaded API key for provider: deepgram (from DEEPGRAM_API_KEY)");

// Forbidden
tracing::info!("API key: {}", api_key); // ← never log secrets
```

## What NOT to do

```rust
// Never hardcode secrets
let api_key = "sk-1234567890"; // ← forbidden

// Never use lazy env reads in provider code
let key = std::env::var("OPENAI_API_KEY").unwrap(); // ← read from config, not env directly

// Never allow startup with missing required config
if config.asr.deepgram.is_none() {
    tracing::warn!("deepgram not configured"); // ← forbidden: should fail fast
}

// Never reload config at runtime
let config = load_config("config.toml")?; // ← only at startup

// Never put secrets in config.toml directly
// api_key = "sk-real-key"  ← forbidden; use api_key = "${ENV_VAR}"
```
