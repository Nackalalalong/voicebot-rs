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
    pub xphone: Option<XphoneBackendConfig>,
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
pub struct XphoneBackendConfig {
    pub sip_host: String,
    #[serde(default)]
    pub local_ip: String,
    #[serde(default = "default_sip_port")]
    pub sip_port: u16,
    #[serde(default = "default_sip_transport")]
    pub transport: String,
    pub username: String,
    pub password: String,
    #[serde(default = "default_rtp_port_min")]
    pub rtp_port_min: u16,
    #[serde(default = "default_rtp_port_max")]
    pub rtp_port_max: u16,
    #[serde(default = "default_register_timeout_ms")]
    pub register_timeout_ms: u64,
    #[serde(default = "default_call_timeout_ms")]
    pub call_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignConfig {
    #[serde(default = "default_campaign_name")]
    pub name: String,
    /// "outbound" (default) or "inbound".
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_target_endpoint")]
    pub target_endpoint: String,
    #[serde(default = "default_caller_id")]
    pub caller_id: String,
    #[serde(default = "default_settle_before_playback_ms")]
    pub settle_before_playback_ms: u64,
    #[serde(default = "default_record_after_playback_ms")]
    pub record_after_playback_ms: u64,
    /// Max simultaneous calls. Default: 1.
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    /// Stop after N calls. Default: 1. Set to 0 with soak_duration_secs > 0 for soak mode.
    #[serde(default = "default_total_calls")]
    pub total_calls: usize,
    /// Stop after N seconds even if total_calls not reached. 0 = no time limit.
    #[serde(default)]
    pub soak_duration_secs: u64,
    /// Spread first `concurrency` calls over this window (ms). 0 = no ramp-up.
    #[serde(default)]
    pub ramp_up_ms: u64,
    /// Maximum calls per second across the campaign. 0.0 = unlimited.
    #[serde(default)]
    pub call_rate_per_second: f64,
    /// Timeout (ms) waiting for each inbound INVITE in inbound mode. Default: 60 000.
    #[serde(default = "default_inbound_timeout_ms")]
    pub inbound_timeout_ms: u64,
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
    /// Gaps shorter than this between voiced regions are counted as stutters. Default: 200 ms.
    #[serde(default = "default_stutter_gap_ms")]
    pub stutter_gap_ms: u64,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            silence_threshold: default_silence_threshold(),
            window_ms: default_window_ms(),
            gap_threshold_ms: default_gap_threshold_ms(),
            stutter_gap_ms: default_stutter_gap_ms(),
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
        match self.backend.kind.as_str() {
            "asterisk-external-media" => self.validate_asterisk_backend()?,
            "xphone" => self.validate_xphone_backend()?,
            other => {
                return Err(LoadtestError::InvalidConfig(format!(
                    "unsupported backend.kind: {}",
                    other
                )));
            }
        }
        match self.campaign.mode.as_str() {
            "outbound" | "inbound" => {}
            other => {
                return Err(LoadtestError::InvalidConfig(format!(
                    "unsupported campaign.mode: '{}' (expected 'outbound' or 'inbound')",
                    other
                )));
            }
        }
        if self.campaign.mode == "inbound" && self.backend.kind != "xphone" {
            return Err(LoadtestError::InvalidConfig(
                "inbound mode is only supported with the 'xphone' backend".into(),
            ));
        }
        if self.campaign.mode == "outbound" && self.campaign.target_endpoint.trim().is_empty() {
            return Err(LoadtestError::InvalidConfig(
                "campaign.target_endpoint must be non-empty for outbound mode".into(),
            ));
        }
        if self.campaign.concurrency == 0 {
            return Err(LoadtestError::InvalidConfig(
                "campaign.concurrency must be > 0".into(),
            ));
        }
        if self.campaign.total_calls == 0 && self.campaign.soak_duration_secs == 0 {
            return Err(LoadtestError::InvalidConfig(
                "campaign: at least one of total_calls or soak_duration_secs must be > 0".into(),
            ));
        }
        if self.campaign.call_rate_per_second < 0.0 {
            return Err(LoadtestError::InvalidConfig(
                "campaign.call_rate_per_second must be >= 0.0".into(),
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

    fn validate_asterisk_backend(&self) -> Result<(), LoadtestError> {
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
        Ok(())
    }

    fn validate_xphone_backend(&self) -> Result<(), LoadtestError> {
        let xphone = self.backend.xphone.as_ref().ok_or_else(|| {
            LoadtestError::InvalidConfig(
                "backend.kind is 'xphone' but [backend.xphone] is missing".into(),
            )
        })?;
        if xphone.sip_host.trim().is_empty()
            || xphone.username.trim().is_empty()
            || xphone.password.trim().is_empty()
        {
            return Err(LoadtestError::InvalidConfig(
                "backend.xphone: sip_host, username, password must be non-empty".into(),
            ));
        }
        if xphone.rtp_port_min >= xphone.rtp_port_max {
            return Err(LoadtestError::InvalidConfig(
                "backend.xphone: rtp_port_min must be < rtp_port_max".into(),
            ));
        }
        if xphone.register_timeout_ms == 0 {
            return Err(LoadtestError::InvalidConfig(
                "backend.xphone.register_timeout_ms must be > 0".into(),
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

fn default_concurrency() -> usize {
    1
}

fn default_total_calls() -> usize {
    1
}

fn default_stutter_gap_ms() -> u64 {
    200
}

fn default_mode() -> String {
    "outbound".into()
}

fn default_inbound_timeout_ms() -> u64 {
    60_000
}

fn default_sip_port() -> u16 {
    5060
}

fn default_sip_transport() -> String {
    "udp".into()
}

fn default_rtp_port_min() -> u16 {
    20000
}

fn default_rtp_port_max() -> u16 {
    20100
}

fn default_register_timeout_ms() -> u64 {
    10_000
}

fn default_call_timeout_ms() -> u64 {
    30_000
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

    #[test]
    fn parses_xphone_config_with_defaults() {
        let config: LoadtestConfig = toml::from_str(
            r#"
                [backend]
                kind = "xphone"

                [backend.xphone]
                sip_host = "localhost"
                username = "voicebot"
                password = "voicebot"

                [campaign]

                [media]
                input_wav = "tests/fixtures/hello.wav"
            "#,
        )
        .expect("xphone config should parse");

        assert_eq!(config.backend.kind, "xphone");
        let xphone = config.backend.xphone.as_ref().unwrap();
        assert_eq!(xphone.sip_port, 5060);
        assert_eq!(xphone.rtp_port_min, 20000);
        assert_eq!(xphone.rtp_port_max, 20100);
        assert_eq!(xphone.register_timeout_ms, 10_000);
        assert_eq!(xphone.call_timeout_ms, 30_000);
        config.validate().expect("xphone config should validate");
    }

    #[test]
    fn parses_inbound_config() {
        let config: LoadtestConfig = toml::from_str(
            r#"
                [backend]
                kind = "xphone"

                [backend.xphone]
                sip_host = "localhost"
                username = "voicebot"
                password = "voicebot"

                [campaign]
                mode = "inbound"
                target_endpoint = ""
                total_calls = 5
                inbound_timeout_ms = 30000

                [media]
                input_wav = "tests/fixtures/hello.wav"
            "#,
        )
        .expect("inbound config should parse");

        assert_eq!(config.campaign.mode, "inbound");
        assert_eq!(config.campaign.inbound_timeout_ms, 30_000);
        config.validate().expect("inbound config should validate");
    }

    #[test]
    fn rejects_inbound_mode_with_asterisk_backend() {
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
                mode = "inbound"

                [media]
                input_wav = "tests/fixtures/hello.wav"
            "#,
        )
        .expect("config should parse");

        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("inbound mode is only supported"));
    }
}
