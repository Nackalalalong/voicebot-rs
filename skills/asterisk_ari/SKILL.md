---
name: Asterisk ARI (Stasis App)
---

# Skill: Asterisk ARI — Stasis Application Transport

Use this whenever building or modifying the `transport/asterisk` crate — ARI WebSocket event handling, REST channel control, AudioSocket audio bridging, DTMF mapping, or Asterisk dialplan configuration.

---

## Architecture Overview

```
Caller ──SIP/RTP──► Asterisk ──Stasis()──► ARI WS event stream
                         │                       │
                    AudioSocket (TCP)        Our ARI client
                         │                  (answer, bridge,
                    Our TCP server          externalMedia)
                         │
                    PipelineSession (VAD→ASR→Agent→TTS)
```

The transport is **not** a provider — it is an independent Stasis application that:

1. Connects to Asterisk ARI WebSocket to receive events.
2. On `StasisStart`: answers the channel, opens an AudioSocket TCP server port, calls `POST /channels/externalMedia` to attach Asterisk's audio to that port, then bridges the two channels.
3. Bridges audio bidirectionally: AudioSocket TCP ↔ `PipelineSession`.
4. On `StasisEnd` / `ChannelHangupRequest`: sends `SessionEnd`, tears down session.

---

## ARI Connection

### WebSocket (event stream)

```
ws://asterisk:8088/ari/events?api_key=user:password&app=voicebot&subscribeAll=false
```

- Connect with basic auth embedded in URL (`api_key=user:password`) — this is the ARI standard.
- All channel events for app `voicebot` arrive here as JSON text frames.
- Must be connected before calls arrive; Asterisk buffers events for ~30s if the WS reconnects quickly.

### REST (channel control)

Base URL: `http://asterisk:8088/ari`  
Auth: `Authorization: Basic base64(user:password)`  
Content-Type: `application/json` for POST bodies; channel/query params preferred.

---

## Stasis App Lifecycle (event types)

| Event type | Trigger | Action |
| --- | --- | --- |
| `StasisStart` | Call enters `Stasis(voicebot)` in dialplan | Answer, open AudioSocket, externalMedia, bridge → spawn `PipelineSession` |
| `StasisEnd` | Channel leaves Stasis (hangup or `continue`) | Send `SessionEnd`, tear down session |
| `ChannelHangupRequest` | Caller hung up | Same as `StasisEnd` (usually precedes it) |
| `ChannelDtmfReceived` | DTMF digit pressed | Map digit to `PipelineEvent::Cancel` or interrupt |
| `ChannelDestroyed` | Channel fully destroyed | Cleanup if session still alive |

### Key ARI event fields

```json
{
    "type": "StasisStart",
    "application": "voicebot",
    "timestamp": "2024-01-01T00:00:00.000+0000",
    "channel": {
        "id": "1704067200.1",
        "name": "PJSIP/alice-00000001",
        "caller": {"name": "Alice", "number": "1234"},
        "state": "Ring",
        "language": "en"
    },
    "args": []
}
```

---

## Call Setup Sequence

```
1. StasisStart received for channel_id
2. POST /channels/{channel_id}/answer
3. Bind TCP server (port from config) → wait for AudioSocket connection
4. POST /channels/externalMedia
     ?app=voicebot
     &external_host=<our-host>:<our-port>
     &transport=tcp
     &encapsulation=audiosocket
     &format=slin16
     &direction=both
   → returns { "id": "ext-media-channel-id" }
5. POST /bridges  ?type=mixing  → returns { "id": "bridge-id" }
6. POST /bridges/{bridge-id}/addChannel?channel={channel_id},{ext-media-channel-id}
7. Accept AudioSocket TCP connection (Asterisk connects to us)
8. Read UUID packet (kind=0x01) — log it
9. Spawn PipelineSession with audio_rx ← AudioSocket audio frames
10. Forward TTS audio → write AudioSocket audio packets
```

---

## AudioSocket Protocol (chan_audiosocket, TCP)

AudioSocket is a simple framing protocol. Asterisk connects TCP to our server.

### Packet format

```
[kind: u8][length: u16 big-endian][payload: bytes]
```

### Packet kinds

| Kind (hex) | Direction | Payload |
| --- | --- | --- |
| `0x00` | Both | Hangup — no payload, length=0. Connection should close. |
| `0x01` | Asterisk→us | UUID — ASCII string identifying this connection (correlates to channel). |
| `0x10` | Both | Audio — raw slin16 PCM (16-bit signed LE, 16kHz mono). |

### Audio frame size

Asterisk sends one Asterisk media frame per packet. At 16kHz with 10ms ptime:

- 160 samples × 2 bytes = **320 bytes** per packet (10ms)
- At 20ms ptime: 320 samples = 640 bytes

### Rust read loop

```rust
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub async fn read_packet(stream: &mut TcpStream) -> Result<AudioSocketPacket, AriError> {
    let kind = stream.read_u8().await?;
    let length = stream.read_u16().await?;  // big-endian
    let mut payload = vec![0u8; length as usize];
    stream.read_exact(&mut payload).await?;
    Ok(AudioSocketPacket { kind, payload })
}

pub async fn write_audio(stream: &mut TcpStream, pcm_bytes: &[u8]) -> Result<(), AriError> {
    stream.write_u8(0x10).await?;
    stream.write_u16(pcm_bytes.len() as u16).await?;  // big-endian
    stream.write_all(pcm_bytes).await?;
    Ok(())
}

pub async fn write_hangup(stream: &mut TcpStream) -> Result<(), AriError> {
    stream.write_u8(0x00).await?;
    stream.write_u16(0).await?;
    Ok(())
}
```

### Converting slin16 packets to AudioFrame

```rust
use common::audio::AudioFrame;
use std::sync::Arc;

pub fn pcm_bytes_to_frame(bytes: &[u8], timestamp_ms: u64) -> AudioFrame {
    // slin16: little-endian 16-bit signed samples
    let samples: Vec<i16> = bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    AudioFrame {
        data: Arc::from(samples.as_slice()),
        sample_rate: 16000,
        channels: 1,
        timestamp_ms,
    }
}

pub fn frame_to_pcm_bytes(frame: &AudioFrame) -> Vec<u8> {
    frame.data.iter()
        .flat_map(|s| s.to_le_bytes())
        .collect()
}
```

---

## Channel Control REST Calls

### Answer a channel

```
POST /ari/channels/{channelId}/answer
```

No body. Returns 200 OK or 412 if already answered.

### Create ExternalMedia channel (AudioSocket)

```
POST /ari/channels/externalMedia
  app=voicebot
  external_host=192.168.1.10:9092   (host:port our TCP server binds)
  transport=tcp
  encapsulation=audiosocket
  format=slin16
  direction=both
```

Returns a `Channel` object. The `external_host` must be reachable FROM Asterisk.

### Create a mixing bridge

```
POST /ari/bridges?type=mixing&name=voicebot-{session_id}
```

Returns a `Bridge` object with `id`.

### Add channels to bridge

```
POST /ari/bridges/{bridgeId}/addChannel?channel={id1},{id2}
```

Both `channel_id` and `ext_media_channel_id` must be added.

### Hang up a channel

```
DELETE /ari/channels/{channelId}?reason=normal
```

### Delete a bridge

```
DELETE /ari/bridges/{bridgeId}
```

---

## Asterisk Dialplan Configuration

`/etc/asterisk/extensions.conf`:

```ini
[voicebot-inbound]
; Route all inbound calls to the Stasis voicebot app
exten => _X.,1,NoOp(Voicebot call from ${CALLERID(num)})
 same =>     n,Answer()
 same =>     n,Stasis(voicebot)
 same =>     n,Hangup()
```

**Important**: The dialplan should NOT answer before Stasis — let the Rust code answer via ARI so we control timing.

Remove the `Answer()` line above if answering in ARI:

```ini
[voicebot-inbound]
exten => _X.,1,Stasis(voicebot)
 same =>     n,Hangup()
```

`/etc/asterisk/ari.conf`:

```ini
[general]
enabled = yes
pretty = yes           ; pretty-print JSON (optional)
allowed_origins = *    ; restrict in production

[voicebot]
type = user
read_only = no
password = secret
password_format = plain
```

---

## DTMF Handling

Map DTMF digits from `ChannelDtmfReceived` to pipeline events:

```rust
fn dtmf_to_pipeline_event(digit: &str) -> Option<PipelineEvent> {
    match digit {
        "#" | "*" => Some(PipelineEvent::Cancel),  // # or * = cancel/interrupt
        "0" => Some(PipelineEvent::Cancel),
        _ => None,  // digits 1-9, A-D: reserved for future use
    }
}
```

---

## Event Deserialization Pattern (Rust)

Use `serde_json::Value` first-pass, then typed parse on known event types:

```rust
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct AriEvent {
    #[serde(rename = "type")]
    pub kind: String,
    pub application: Option<String>,
    pub channel: Option<AriChannel>,
    pub digit: Option<String>,
    pub duration_ms: Option<u32>,
    pub cause: Option<i32>,
    pub cause_txt: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AriChannel {
    pub id: String,
    pub name: String,
    pub caller: Option<AriCaller>,
    pub language: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AriCaller {
    pub name: String,
    pub number: String,
}
```

Then dispatch on `event.kind.as_str()`:

- `"StasisStart"` → start session
- `"StasisEnd"` → end session
- `"ChannelDtmfReceived"` → map digit
- `"ChannelHangupRequest"` | `"ChannelDestroyed"` → end session
- everything else → ignore

---

## ARI REST Client (reqwest)

```rust
use reqwest::Client;

pub struct AriRestClient {
    client: Client,
    base_url: String,
    username: String,
    password: String,
}

impl AriRestClient {
    pub fn new(base_url: &str, username: &str, password: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.to_string(),
            username: username.to_string(),
            password: password.to_string(),
        }
    }

    pub async fn answer_channel(&self, channel_id: &str) -> Result<(), AriError> {
        let url = format!("{}/channels/{}/answer", self.base_url, channel_id);
        let resp = self.client
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send().await?;
        if !resp.status().is_success() {
            return Err(AriError::RestError(resp.status().as_u16(), url));
        }
        Ok(())
    }

    pub async fn create_external_media(
        &self,
        app: &str,
        external_host: &str,
        format: &str,
    ) -> Result<String, AriError> {
        let url = format!("{}/channels/externalMedia", self.base_url);
        let resp = self.client
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .query(&[
                ("app", app),
                ("external_host", external_host),
                ("transport", "tcp"),
                ("encapsulation", "audiosocket"),
                ("format", format),
                ("direction", "both"),
            ])
            .send().await?;
        let body: serde_json::Value = resp.json().await?;
        let id = body["id"].as_str()
            .ok_or_else(|| AriError::Protocol("externalMedia response missing id".into()))?
            .to_string();
        Ok(id)
    }

    pub async fn create_bridge(&self, name: &str) -> Result<String, AriError> {
        let url = format!("{}/bridges", self.base_url);
        let resp = self.client
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .query(&[("type", "mixing"), ("name", name)])
            .send().await?;
        let body: serde_json::Value = resp.json().await?;
        Ok(body["id"].as_str().unwrap_or("").to_string())
    }

    pub async fn add_to_bridge(
        &self,
        bridge_id: &str,
        channel_ids: &[&str],
    ) -> Result<(), AriError> {
        let url = format!("{}/bridges/{}/addChannel", self.base_url, bridge_id);
        let channels = channel_ids.join(",");
        self.client
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .query(&[("channel", channels.as_str())])
            .send().await?;
        Ok(())
    }

    pub async fn hangup_channel(&self, channel_id: &str) -> Result<(), AriError> {
        let url = format!("{}/channels/{}", self.base_url, channel_id);
        self.client
            .delete(&url)
            .basic_auth(&self.username, Some(&self.password))
            .query(&[("reason", "normal")])
            .send().await?;
        Ok(())
    }

    pub async fn destroy_bridge(&self, bridge_id: &str) -> Result<(), AriError> {
        let url = format!("{}/bridges/{}", self.base_url, bridge_id);
        self.client
            .delete(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send().await?;
        Ok(())
    }
}
```

---

## ARI WebSocket Client

```rust
use tokio_tungstenite::{connect_async, tungstenite::Message};
use futures::{SinkExt, StreamExt};

pub async fn connect_ari_ws(
    ari_url: &str,    // "http://asterisk:8088"
    username: &str,
    password: &str,
    app_name: &str,
) -> Result<impl StreamExt<Item = Result<Message, _>>, AriError> {
    // ARI WS URL format
    let ws_url = format!(
        "ws://{}/ari/events?api_key={}:{}&app={}",
        ari_url.trim_start_matches("http://"),
        username,
        password,
        app_name,
    );
    let (ws_stream, _) = connect_async(&ws_url).await
        .map_err(|e| AriError::WebSocket(e.to_string()))?;
    Ok(ws_stream)
}
```

---

## ARI Config in config.toml

```toml
[asterisk]
ari_url = "http://localhost:8088"
username = "voicebot"
password = "secret"
app_name = "voicebot"
audio_host = "192.168.1.10"   # host reachable from Asterisk
audio_port = 9092              # TCP port for AudioSocket
```

---

## Key Constraints

- AudioSocket format **must** be `slin16` (16kHz signed linear) to avoid codec conversion — matches pipeline's canonical format.
- The `external_host` in `externalMedia` must be reachable FROM Asterisk (not `localhost` if Asterisk is in a different container/host).
- Each incoming call gets a **fresh** TCP connection from Asterisk's `chan_audiosocket`. The UUID packet (0x01) correlates the TCP connection to the ARI channel.
- Do NOT hardcode channels directly to AudioSocket without a bridge — bridging is required for bidirectional audio.
- Bridges must be destroyed on session end to avoid orphaned bridges accumulating in Asterisk.
- `StasisEnd` fires AFTER `ChannelHangupRequest` — handle both but only clean up once (use a `CancellationToken`).
- Audio packets flowing from our TTS output must be split into ≤320-byte chunks (10ms at 16kHz) to avoid overloading Asterisk's AudioSocket buffer.
