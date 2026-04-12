use serde::Deserialize;

/// A parsed ARI event from the WebSocket stream.
/// Flat struct to handle all event types uniformly.
#[derive(Debug, Deserialize)]
pub struct AriEvent {
    #[serde(rename = "type")]
    pub kind: String,
    pub channel: Option<AriChannel>,
    pub digit: Option<String>,
    pub duration_ms: Option<u32>,
    pub cause: Option<i32>,
    pub cause_txt: Option<String>,
    pub application: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AriChannel {
    pub id: String,
    pub name: String,
    pub caller: Option<AriCaller>,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AriCaller {
    pub name: String,
    pub number: String,
}
