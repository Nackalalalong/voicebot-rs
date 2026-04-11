use serde::Deserialize;
use thiserror::Error;

/// Top-level application configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub session_defaults: SessionDefaultsConfig,
    pub vad: crate::events::VadConfig,
    pub asr: AsrConfigGroup,
    pub llm: LlmConfigGroup,
    pub tts: TtsConfigGroup,
    #[serde(default)]
    pub channels: ChannelConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionDefaultsConfig {
    pub language: String,
    pub asr_provider: String,
    pub tts_provider: String,
    pub llm_provider: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AsrConfigGroup {
    pub primary: String,
    pub fallback: Option<String>,
    pub deepgram: Option<DeepgramConfig>,
    pub whisper: Option<WhisperConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeepgramConfig {
    pub api_key: String,
    pub model: String,
    pub language: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WhisperConfig {
    pub model_path: String,
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
    pub audio_ingress_capacity: usize,
    pub event_bus_capacity: usize,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            audio_ingress_capacity: 50,
            event_bus_capacity: 200,
        }
    }
}

/// Configuration errors.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config file not found: {path}: {source}")]
    FileNotFound {
        path: String,
        source: std::io::Error,
    },
    #[error("config parse error: {0}")]
    ParseError(String),
    #[error("missing required environment variables: {0:?}")]
    MissingEnvVars(Vec<String>),
    #[error("invalid configuration: {0}")]
    Invalid(String),
}

/// Load and validate config from a TOML file, substituting `${VAR}` from env.
pub fn load_config(path: &str) -> Result<AppConfig, ConfigError> {
    let raw = std::fs::read_to_string(path).map_err(|e| ConfigError::FileNotFound {
        path: path.into(),
        source: e,
    })?;

    let resolved = substitute_env_vars(&raw)?;

    let config: AppConfig =
        toml::from_str(&resolved).map_err(|e| ConfigError::ParseError(e.to_string()))?;

    validate_config(&config)?;

    Ok(config)
}

/// Replace all `${VAR_NAME}` patterns with environment variable values.
fn substitute_env_vars(input: &str) -> Result<String, ConfigError> {
    let re = regex::Regex::new(r"\$\{([^}]+)\}").expect("valid regex");
    let mut result = input.to_string();
    let mut missing = Vec::new();

    for cap in re.captures_iter(input) {
        let var_name = &cap[1];
        match std::env::var(var_name) {
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

/// Validate configuration for consistency.
fn validate_config(config: &AppConfig) -> Result<(), ConfigError> {
    if config.server.port == 0 {
        return Err(ConfigError::Invalid("server.port must be non-zero".into()));
    }

    // Validate primary ASR provider has config
    match config.asr.primary.as_str() {
        "deepgram" => {
            if config.asr.deepgram.is_none() {
                return Err(ConfigError::Invalid(
                    "asr.primary is 'deepgram' but [asr.deepgram] section is missing".into(),
                ));
            }
        }
        "whisper" => {
            if config.asr.whisper.is_none() {
                return Err(ConfigError::Invalid(
                    "asr.primary is 'whisper' but [asr.whisper] section is missing".into(),
                ));
            }
        }
        other => {
            return Err(ConfigError::Invalid(format!(
                "unknown asr.primary: {}",
                other
            )));
        }
    }

    // Validate primary LLM provider has config
    match config.llm.primary.as_str() {
        "openai" => {
            if config.llm.openai.is_none() {
                return Err(ConfigError::Invalid(
                    "llm.primary is 'openai' but [llm.openai] section is missing".into(),
                ));
            }
        }
        "anthropic" => {
            if config.llm.anthropic.is_none() {
                return Err(ConfigError::Invalid(
                    "llm.primary is 'anthropic' but [llm.anthropic] section is missing".into(),
                ));
            }
        }
        other => {
            return Err(ConfigError::Invalid(format!(
                "unknown llm.primary: {}",
                other
            )));
        }
    }

    // Validate VAD thresholds
    if config.vad.energy_threshold < 0.0 || config.vad.energy_threshold > 1.0 {
        return Err(ConfigError::Invalid(
            "vad.energy_threshold must be between 0.0 and 1.0".into(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_substitute_env_vars_success() {
        std::env::set_var("TEST_CONFIG_VAR_123", "secret_value");
        let input = "key = \"${TEST_CONFIG_VAR_123}\"";
        let result = substitute_env_vars(input).unwrap();
        assert_eq!(result, "key = \"secret_value\"");
        std::env::remove_var("TEST_CONFIG_VAR_123");
    }

    #[test]
    fn test_substitute_env_vars_missing() {
        let input = "key = \"${DEFINITELY_MISSING_VAR_XYZ}\"";
        let result = substitute_env_vars(input);
        assert!(result.is_err());
        match result.unwrap_err() {
            ConfigError::MissingEnvVars(vars) => {
                assert!(vars.contains(&"DEFINITELY_MISSING_VAR_XYZ".to_string()));
            }
            _ => panic!("expected MissingEnvVars"),
        }
    }

    #[test]
    fn test_channel_config_defaults() {
        let config = ChannelConfig::default();
        assert_eq!(config.audio_ingress_capacity, 50);
        assert_eq!(config.event_bus_capacity, 200);
    }
}
