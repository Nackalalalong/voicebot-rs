use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::LoadtestError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadtestConfig {
    pub backend: BackendConfig,
    pub campaign: CampaignConfig,
    pub media: MediaConfig,
    #[serde(default)]
    pub analysis: AnalysisConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    #[serde(default = "default_backend_kind")]
    pub kind: String,
    pub asterisk: Option<AsteriskBackendConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsteriskBackendConfig {
    pub ari_host: String,
    #[serde(default = "default_ari_port")]
    pub ari_port: u16,
    pub username: String,
    pub password: String,
    #[serde(default = "default_app_name")]
    pub app_name: String,
    pub audio_host: String,
    #[serde(default = "default_accept_timeout_ms")]
    pub accept_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignConfig {
    #[serde(default = "default_campaign_name")]
    pub name: String,
    #[serde(default = "default_target_endpoint")]
    pub target_endpoint: String,
    #[serde(default = "default_caller_id")]
    pub caller_id: String,
    #[serde(default = "default_settle_before_playback_ms")]
    pub settle_before_playback_ms: u64,
    #[serde(default = "default_record_after_playback_ms")]
    pub record_after_playback_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaConfig {
    pub input_wav: PathBuf,
    #[serde(default = "default_artifact_dir")]
    pub artifact_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisConfig {
    #[serde(default = "default_silence_threshold")]
    pub silence_threshold: f32,
    #[serde(default = "default_window_ms")]
    pub window_ms: u32,
    #[serde(default = "default_gap_threshold_ms")]
    pub gap_threshold_ms: u64,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            silence_threshold: default_silence_threshold(),
            window_ms: default_window_ms(),
            gap_threshold_ms: default_gap_threshold_ms(),
        }
    }
}

impl LoadtestConfig {
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, LoadtestError> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)?;
        let resolved = substitute_env_vars(&raw)?;
        let config: Self = toml::from_str(&resolved)?;
        config.validate()?;
        Ok(config)
    }

    pub fn to_toml_string(&self) -> Result<String, LoadtestError> {
        Ok(toml::to_string_pretty(self)?)
    }

    fn validate(&self) -> Result<(), LoadtestError> {
        if self.backend.kind != "asterisk-external-media" {
            return Err(LoadtestError::InvalidConfig(format!(
                "unsupported backend.kind: {}",
                self.backend.kind
            )));
        }

        let asterisk = self.backend.asterisk.as_ref().ok_or_else(|| {
            LoadtestError::InvalidConfig(
                "backend.kind is 'asterisk-external-media' but [backend.asterisk] is missing"
                    .into(),
            )
        })?;

        if asterisk.ari_host.trim().is_empty()
            || asterisk.username.trim().is_empty()
            || asterisk.password.trim().is_empty()
            || asterisk.app_name.trim().is_empty()
            || asterisk.audio_host.trim().is_empty()
        {
            return Err(LoadtestError::InvalidConfig(
                "backend.asterisk fields must be non-empty".into(),
            ));
        }
        if asterisk.accept_timeout_ms == 0 {
            return Err(LoadtestError::InvalidConfig(
                "backend.asterisk.accept_timeout_ms must be > 0".into(),
            ));
        }
        if self.campaign.target_endpoint.trim().is_empty() {
            return Err(LoadtestError::InvalidConfig(
                "campaign.target_endpoint must be non-empty".into(),
            ));
        }
        if self.campaign.settle_before_playback_ms > 30_000 {
            return Err(LoadtestError::InvalidConfig(
                "campaign.settle_before_playback_ms is unreasonably large".into(),
            ));
        }
        if self.analysis.silence_threshold <= 0.0 || self.analysis.silence_threshold >= 1.0 {
            return Err(LoadtestError::InvalidConfig(
                "analysis.silence_threshold must be between 0.0 and 1.0".into(),
            ));
        }
        if self.analysis.window_ms == 0 {
            return Err(LoadtestError::InvalidConfig(
                "analysis.window_ms must be > 0".into(),
            ));
        }
        Ok(())
    }
}

fn substitute_env_vars(input: &str) -> Result<String, LoadtestError> {
    let regex = Regex::new(r"\$\{([^}]+)\}").expect("valid regex");
    let mut missing = Vec::new();

    for capture in regex.captures_iter(input) {
        let key = &capture[1];
        if std::env::var(key).is_err() {
            missing.push(key.to_string());
        }
    }

    if !missing.is_empty() {
        return Err(LoadtestError::MissingEnvVars(missing));
    }

    let mut output = String::with_capacity(input.len());
    let mut last_end = 0;
    for capture in regex.captures_iter(input) {
        let matched = capture.get(0).expect("full regex match");
        output.push_str(&input[last_end..matched.start()]);
        let value = std::env::var(&capture[1]).expect("env var checked above");
        output.push_str(&value);
        last_end = matched.end();
    }
    output.push_str(&input[last_end..]);
    Ok(output)
}

fn default_backend_kind() -> String {
    "asterisk-external-media".into()
}

fn default_ari_port() -> u16 {
    8088
}

fn default_app_name() -> String {
    "voicebot-loadtest".into()
}

fn default_accept_timeout_ms() -> u64 {
    10_000
}

fn default_campaign_name() -> String {
    "phase1-outbound".into()
}

fn default_target_endpoint() -> String {
    "Local/1000@dp_entry_call_in".into()
}

fn default_caller_id() -> String {
    "voicebot-loadtest".into()
}

fn default_settle_before_playback_ms() -> u64 {
    500
}

fn default_record_after_playback_ms() -> u64 {
    3_000
}

fn default_artifact_dir() -> PathBuf {
    PathBuf::from("artifacts/loadtest")
}

fn default_silence_threshold() -> f32 {
    0.02
}

fn default_window_ms() -> u32 {
    20
}

fn default_gap_threshold_ms() -> u64 {
    250
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config_with_defaults() {
        let config: LoadtestConfig = toml::from_str(
            r#"
                [backend]
                kind = "asterisk-external-media"

                [backend.asterisk]
                ari_host = "localhost"
                username = "voicebot"
                password = "voicebot"
                audio_host = "172.17.0.1"

                [campaign]

                [media]
                input_wav = "tests/fixtures/hello.wav"
            "#,
        )
        .expect("config should parse");

        assert_eq!(
            config.campaign.target_endpoint,
            "Local/1000@dp_entry_call_in"
        );
        assert_eq!(config.analysis.window_ms, 20);
        config.validate().expect("config should validate");
    }
}
