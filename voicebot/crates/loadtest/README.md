# voicebot-loadtest

Phase 1 loadtest harness for the voicebot stack.

Current scope:

- one outbound call at a time
- Asterisk external-media backend
- normalized WAV playback into the call
- received-audio recording
- summary JSON with first-response and silence-gap metrics

Current non-goals:

- SIP registration
- inbound virtual-phone mode
- campaign scheduler
- stutter and smoothness composite scoring

## Run Phase 1

Start the voicebot server and Asterisk first, then run:

```bash
cd voicebot
cargo run -p voicebot-loadtest -- loadtest.phase1.toml
```

The sample config uses:

- Asterisk ARI on `localhost:8088`
- external-media callback host `172.17.0.1`
- the local Asterisk dial target `Local/1000@dp_entry_call_in`
- fixture WAV `crates/loadtest/fixtures/tone_1s_16k.wav`

Artifacts are written under `artifacts/loadtest/<run_id>/`.

## Phase 2 Direction

Phase 2 is planned to target `xphone` as the native SIP backend so the harness can act like a real registered phone:

- register SIP users
- receive inbound INVITEs
- place outbound calls without depending on ARI origination
- keep the same analysis and reporting layers across both backends
