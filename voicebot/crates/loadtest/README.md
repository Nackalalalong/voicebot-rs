# voicebot-loadtest

Phase 1 loadtest harness for the voicebot stack.

Current scope:

- multi-call campaign execution with configurable concurrency/rate/ramp
- Asterisk, WebSocket, and xphone backends
- normalized WAV playback into the call
- received-audio recording
- campaign JSON plus Markdown and HTML reports with first-response, gap, and outcome insights

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
- a dedicated loadtest controller app `voicebot-loadtest` for the controllable Local channel leg
- external-media callback host `172.17.0.1`
- the local Asterisk dial target `Local/1000@dp_entry_call_in`
- speech fixture `tests/fixtures/audio/sample_speech.wav`

Phase 1 currently uses ARI `externalMedia` in the supported RTP/UDP mode. The loadtest backend controls the Local channel leg in its own Stasis app, while the far leg reaches `Stasis(voicebot)` through the existing dialplan.

Artifacts are written under `artifacts/loadtest/<run_id>/`, including `campaign.json`, `report.md`, and `report.html`.

## Phase 2 Direction

Phase 2 is planned to target `xphone` as the native SIP backend so the harness can act like a real registered phone:

- register SIP users
- receive inbound INVITEs
- place outbound calls without depending on ARI origination
- keep the same analysis and reporting layers across both backends
