---
name: xphone — SIP Virtual Phone for Load Testing
---

# Skill: xphone — Native SIP Virtual Phone Backend

Use this skill when implementing or modifying the `xphone`-based SIP phone backend in the loadtest crate — SIP registration, inbound/outbound call handling, PCM audio I/O, phone pool management, or integration with the campaign scheduler.

---

## Overview

`xphone` is a Rust SIP telephony library (crate `xphone = "0.4"`) that provides a complete SIP UA + RTP stack. It registers with Asterisk as a real SIP phone, accepts inbound INVITEs, places outbound calls, and exposes decoded PCM audio through `crossbeam-channel`.

The xphone backend replaces the ARI-mediated `AsteriskExternalMediaBackend` (Phase 1) with a direct SIP UA that can:

- Register as one or more SIP endpoints with Asterisk
- Place outbound calls (INVITE → answer → media → BYE)
- Receive inbound calls (INVITE from Asterisk → auto-accept → media → BYE)
- Play scripted WAV audio and record received audio

---

## Crate Dependency

```toml
[dependencies]
xphone = "0.4"
```

**Key facts:**

- Requires Rust 1.87+
- G.711 (PCMU/PCMA) and G.722 are built-in, no system deps
- Opus requires `--features opus-codec` + system libopus
- G.729 requires `--features g729-codec` (pure Rust)
- Uses `std::thread` + `crossbeam-channel` internally — **NOT tokio**
- Uses `tracing` for structured logging (compatible with our subscriber)

---

## xphone API Reference (Subset)

### Phone Mode (SIP Client Registration)

```rust
use xphone::{Phone, Config, Call, DialOptions, Codec};
use std::sync::Arc;
use std::time::Duration;

let phone = Phone::new(Config {
    username: "voicebot".into(),
    password: "voicebot".into(),
    host: "localhost".into(),  // Asterisk host
    port: 5060,
    transport: "udp".into(),   // "udp" | "tcp" | "tls"
    rtp_port_min: 20000,
    rtp_port_max: 20100,
    codec_prefs: vec![Codec::PCMU],  // match Asterisk endpoint !all,ulaw,alaw
    ..Config::default()
});

// Registration callback
phone.on_registered(|| {
    println!("Registered with Asterisk");
});

// Inbound call callback
phone.on_incoming(move |call: Arc<Call>| {
    call.accept().unwrap();
    // ... read/write audio
});

phone.connect().expect("SIP registration failed");
```

### Outbound Call

```rust
let opts = DialOptions {
    early_media: true,
    timeout: Duration::from_secs(30),
    ..Default::default()
};

let call = phone.dial("1000", opts)?;  // dials extension 1000 on registered host

call.on_ended(|reason| {
    println!("Call ended: {:?}", reason);
});
```

### Audio I/O

**PCM frame format:** `Vec<i16>`, mono, **8000 Hz**, 160 samples per frame (20 ms).

```rust
// Read received audio
if let Some(pcm_rx) = call.pcm_reader() {
    // pcm_rx: crossbeam_channel::Receiver<Vec<i16>>
    while let Ok(frame) = pcm_rx.recv() {
        // frame is Vec<i16>, 160 samples, 8kHz mono
    }
}

// Write audio at real-time pace (manual 20ms timing)
if let Some(pcm_tx) = call.pcm_writer() {
    // pcm_tx: crossbeam_channel::Sender<Vec<i16>>
    for chunk in samples_8k.chunks(160) {
        pcm_tx.send(chunk.to_vec()).unwrap();
        std::thread::sleep(Duration::from_millis(20));
    }
}

// Write arbitrary-length audio with automatic pacing (preferred for playback)
if let Some(paced_tx) = call.paced_pcm_writer() {
    paced_tx.send(all_samples_8k).unwrap();
    // xphone handles framing + 20ms pacing internally
}
```

**Important constraints:**

- `pcm_writer()` and `paced_pcm_writer()` are **mutually exclusive** per call
- Inbound buffer holds 256 frames (~5s); slow readers cause silent drops
- Outbound `try_send` drops newest frame on full buffer

### Call Hangup

```rust
call.end().unwrap();
// or wait for remote to hang up via on_ended callback
```

### Call State

```rust
use xphone::CallState;

// Idle -> Ringing (inbound) or Dialing (outbound)
//      -> RemoteRinging -> Active <-> OnHold -> Ended

call.on_state(|state| {
    match state {
        CallState::Active => { /* media is flowing */ }
        CallState::Ended => { /* cleanup */ }
        _ => {}
    }
});
```

### Phone Disconnect

```rust
phone.disconnect();  // sends SIP UNREGISTER
```

---

## Audio Format Bridge

xphone PCM is **8 kHz mono i16**. The loadtest canonical format is **16 kHz mono i16**.

Reuse existing conversions from `audio.rs` and `backend/asterisk.rs`:

| Direction              | Conversion          | Function                                    |
| ---------------------- | ------------------- | ------------------------------------------- |
| RX (xphone → loadtest) | Upsample 8k → 16k   | `upsample_8k_to_16k()` — sample duplication |
| TX (loadtest → xphone) | Downsample 16k → 8k | `downsample_16k_to_8k()` — step by 2        |

These are trivial point-sample conversions already in the codebase.

---

## Threading Model: xphone ↔ tokio Bridge

xphone uses `std::thread` + `crossbeam-channel`. The loadtest crate is tokio-based. Bridge pattern:

```rust
// Outbound (tokio → xphone): run xphone audio write on spawn_blocking
let paced_tx = call.paced_pcm_writer().unwrap();
let samples_8k = downsample_16k_to_8k(&tx_samples);
tokio::task::spawn_blocking(move || {
    paced_tx.send(samples_8k).unwrap();
});

// Inbound (xphone → tokio): collect in spawn_blocking, return via oneshot
let pcm_rx = call.pcm_reader().unwrap();
let (done_tx, done_rx) = tokio::sync::oneshot::channel();
tokio::task::spawn_blocking(move || {
    let mut recorded = Vec::new();
    while let Ok(frame) = pcm_rx.recv() {
        recorded.extend_from_slice(&frame);
    }
    let _ = done_tx.send(recorded);
});
let recorded_8k = done_rx.await.unwrap();
let recorded_16k = upsample_8k_to_16k(&recorded_8k);
```

**Alternatively**, use a `std::thread` for time-critical paths and `tokio::sync::mpsc` to bridge back:

```rust
let (audio_tx, mut audio_rx) = tokio::sync::mpsc::channel::<Vec<i16>>(256);
std::thread::spawn(move || {
    while let Ok(frame) = pcm_rx.recv() {
        if audio_tx.blocking_send(frame).is_err() { break; }
    }
});
// In tokio context:
while let Some(frame) = audio_rx.recv().await {
    recorded.extend_from_slice(&frame);
}
```

---

## Architecture: xphone Backend in Loadtest

### Backend Trait

The xphone backend implements `Phase1Backend` from `loadtest/src/backend.rs`. Each call to `run_single_outbound_call` uses a pre-registered phone.

```
┌─────────────────────┐
│  run_campaign()     │  tokio
│  ├─ spawn tasks     │
│  └─ collect results │
└────────┬────────────┘
         │
    ┌────▼────────────────────┐
    │ XphoneBackend           │
    │ ├─ phone: Arc<Phone>    │  registered at init
    │ └─ run_single_outbound  │
    │    call()               │
    └────────┬────────────────┘
             │
    ┌────────▼────────────────┐
    │ Per-call flow:          │  spawn_blocking
    │ 1. phone.dial(target)   │
    │ 2. paced_pcm_writer()   │  send TX samples
    │ 3. pcm_reader()         │  collect RX samples
    │ 4. call.end()           │
    │ 5. return result        │
    └─────────────────────────┘
```

### Inbound Mode

For inbound mode (`wait_for_inbound_call`):

```
┌──────────────────────────┐
│ phone.on_incoming(...)   │  crossbeam callback
│  └─ call.accept()        │
│  └─ paced_pcm_writer()   │  play TX audio
│  └─ pcm_reader()         │  record RX audio
│  └─ call.end()           │
└──────────────────────────┘
```

The incoming callback sends the `Arc<Call>` to a tokio `mpsc::channel` where the campaign scheduler picks it up.

---

## Asterisk SIP Configuration

### Current Endpoint

From `system/asterisk/etc/pjsip_endpoint.conf`:

- Endpoint: `voicebot`
- Auth: `voicebot` / `voicebot`
- Codecs: `!all,ulaw,alaw`
- Max contacts: 10 (from `_term` template)
- Context: `dp_entry_call_in`
- Hint: `1000`

### Dialplan

From `system/asterisk/etc/extensions_local.conf`:

```
[dp_entry_call_in]
exten => 1000,1,Stasis(voicebot)
```

### How xphone Connects

1. xphone registers to `localhost:5060` as `voicebot`/`voicebot`
2. Asterisk accepts registration (max_contacts=10 allows multiple)
3. Outbound: `phone.dial("1000")` → Asterisk routes to `dp_entry_call_in` → Stasis(voicebot) → voicebot-server handles it
4. Inbound: voicebot originates call to registered contacts → xphone's `on_incoming` fires

### Scaling Beyond 10 Contacts

For large-scale load tests, either:

- Increase `max_contacts` in the AOR template
- Template additional endpoints: `loadtest-001` through `loadtest-N` via `pjsip_wizard.conf`

---

## Config Schema for xphone Backend

```toml
[backend]
kind = "xphone"

[backend.xphone]
sip_host = "localhost"
sip_port = 5060
transport = "udp"           # "udp" | "tcp" | "tls"
username = "voicebot"
password = "voicebot"
rtp_port_min = 20000
rtp_port_max = 20100
codec = "pcmu"              # "pcmu" | "pcma" | "g722"
register_timeout_ms = 10000
call_timeout_ms = 30000
```

### Config Struct

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XphoneBackendConfig {
    pub sip_host: String,
    #[serde(default = "default_sip_port")]
    pub sip_port: u16,               // 5060
    #[serde(default = "default_transport")]
    pub transport: String,            // "udp"
    pub username: String,
    pub password: String,
    #[serde(default = "default_rtp_port_min")]
    pub rtp_port_min: u16,           // 20000
    #[serde(default = "default_rtp_port_max")]
    pub rtp_port_max: u16,           // 20100
    #[serde(default = "default_codec")]
    pub codec: String,               // "pcmu"
    #[serde(default = "default_register_timeout_ms")]
    pub register_timeout_ms: u64,    // 10000
    #[serde(default = "default_call_timeout_ms")]
    pub call_timeout_ms: u64,        // 30000
}
```

---

## Testing Strategy

### Unit Tests — MockPhone / MockCall

xphone ships mocks that don't need a SIP server:

```rust
use xphone::mock::phone::MockPhone;
use xphone::mock::call::MockCall;

#[test]
fn test_outbound_call_flow() {
    let phone = MockPhone::new();
    phone.connect().unwrap();
    // MockPhone API mirrors Phone API
}
```

### Integration Tests with fakepbx (no Docker)

```rust
// fakepbx provides an in-process SIP server
// Use for #[test] (not #[ignore]) CI-safe tests
```

### End-to-End with Docker Asterisk

```rust
#[tokio::test]
#[ignore] // requires running Asterisk
async fn xphone_outbound_call_to_asterisk() {
    // Real SIP registration + call through Docker Asterisk
}
```

---

## Key Constraints and Gotchas

1. **Rust 1.87+ required** — xphone won't compile on older toolchains
2. **crossbeam, not tokio** — all xphone callbacks and audio channels are synchronous; use `spawn_blocking` or `std::thread` to bridge
3. **pcm_writer vs paced_pcm_writer** — mutually exclusive per call; use `paced_pcm_writer` for WAV playback
4. **8 kHz PCM** — always upsample/downsample to/from project canonical 16 kHz
5. **256-frame buffer** — slow readers lose oldest frames; slow writers lose newest
6. **max_contacts** — Asterisk endpoint limits concurrent registrations; check AOR config
7. **RTP port range** — each call needs one even port; size the range for max concurrency: `(max - min) / 2 >= max_concurrent_calls`
8. **on_incoming is synchronous** — the callback fires on xphone's internal thread; do minimal work (just send the `Arc<Call>` to a channel) and handle the call asynchronously
9. **Beta crate (0.4.x)** — the API may change; pin to exact version and wrap behind the backend trait
10. **No `unwrap()` in non-test code** — map xphone `Error` to `LoadtestError`

---

## File Layout

```
voicebot/crates/loadtest/
├── src/
│   ├── backend/
│   │   ├── asterisk.rs          # existing Phase 1 backend
│   │   └── xphone_backend.rs   # new xphone backend
│   ├── backend.rs               # add XphoneBackend + "xphone" kind
│   ├── config.rs                # add XphoneBackendConfig
│   └── ...
```
