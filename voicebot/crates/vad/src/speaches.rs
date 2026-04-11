use common::audio::AudioFrame;
use reqwest::multipart;
use serde::Deserialize;
use tracing::debug;

/// A speech segment detected by Speaches Silero VAD.
#[derive(Debug, Clone, Deserialize)]
pub struct SpeechTimestamp {
    /// Segment start in milliseconds.
    pub start: u64,
    /// Segment end in milliseconds.
    pub end: u64,
}

/// Speaches VAD client using the `/v1/audio/speech/timestamps` endpoint.
/// This performs batch VAD — post audio, get speech segment timestamps back.
pub struct SpeachesVadClient {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    threshold: f32,
    min_silence_duration_ms: u32,
}

impl SpeachesVadClient {
    pub fn new(base_url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
            api_key: None,
            threshold: 0.75,
            min_silence_duration_ms: 1000,
        }
    }

    pub fn with_api_key(mut self, api_key: String) -> Self {
        self.api_key = Some(api_key);
        self
    }

    pub fn with_threshold(mut self, threshold: f32) -> Self {
        self.threshold = threshold;
        self
    }

    pub fn with_min_silence_duration_ms(mut self, ms: u32) -> Self {
        self.min_silence_duration_ms = ms;
        self
    }

    /// Detect speech segments in the given audio frames.
    pub async fn detect(
        &self,
        audio: &[AudioFrame],
    ) -> Result<Vec<SpeechTimestamp>, SpeachesVadError> {
        if audio.is_empty() {
            return Ok(Vec::new());
        }

        let pcm_bytes: Vec<u8> = audio.iter().flat_map(|f| f.to_pcm_bytes()).collect();

        let file_part = multipart::Part::bytes(pcm_bytes)
            .file_name("audio.pcm")
            .mime_str("audio/L16;rate=16000;channels=1")
            .map_err(|e| SpeachesVadError::RequestBuild(e.to_string()))?;

        let form = multipart::Form::new()
            .part("file", file_part)
            .text("model", "silero_vad")
            .text("threshold", self.threshold.to_string())
            .text(
                "min_silence_duration_ms",
                self.min_silence_duration_ms.to_string(),
            );

        let mut req = self
            .client
            .post(format!("{}/v1/audio/speech/timestamps", self.base_url))
            .multipart(form)
            .timeout(std::time::Duration::from_secs(30));

        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let resp = req.send().await.map_err(|e| {
            if e.is_timeout() {
                SpeachesVadError::Timeout
            } else {
                SpeachesVadError::ConnectionFailed(e.to_string())
            }
        })?;

        if !resp.status().is_success() {
            return Err(SpeachesVadError::ServerError(format!(
                "Speaches VAD returned {}",
                resp.status()
            )));
        }

        let timestamps: Vec<SpeechTimestamp> = resp
            .json()
            .await
            .map_err(|e| SpeachesVadError::InvalidResponse(e.to_string()))?;

        debug!(segments = timestamps.len(), "Speaches VAD detected speech");
        Ok(timestamps)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SpeachesVadError {
    #[error("VAD connection failed: {0}")]
    ConnectionFailed(String),
    #[error("VAD request timed out")]
    Timeout,
    #[error("VAD server error: {0}")]
    ServerError(String),
    #[error("VAD invalid response: {0}")]
    InvalidResponse(String),
    #[error("VAD request build error: {0}")]
    RequestBuild(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_speech_timestamp_parse() {
        let json = r#"[{"start": 500, "end": 3200}, {"start": 4100, "end": 8900}]"#;
        let timestamps: Vec<SpeechTimestamp> = serde_json::from_str(json).unwrap();
        assert_eq!(timestamps.len(), 2);
        assert_eq!(timestamps[0].start, 500);
        assert_eq!(timestamps[0].end, 3200);
        assert_eq!(timestamps[1].start, 4100);
        assert_eq!(timestamps[1].end, 8900);
    }

    #[test]
    fn test_client_builder() {
        let client = SpeachesVadClient::new("http://localhost:8000".into())
            .with_api_key("test-key".into())
            .with_threshold(0.5)
            .with_min_silence_duration_ms(500);

        assert_eq!(client.base_url, "http://localhost:8000");
        assert_eq!(client.api_key.as_deref(), Some("test-key"));
        assert!((client.threshold - 0.5).abs() < f32::EPSILON);
        assert_eq!(client.min_silence_duration_ms, 500);
    }

    #[test]
    fn test_empty_parse() {
        let json = "[]";
        let timestamps: Vec<SpeechTimestamp> = serde_json::from_str(json).unwrap();
        assert!(timestamps.is_empty());
    }
}
