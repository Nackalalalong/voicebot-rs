use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

use crate::backend::{Phase1Backend, Phase1CallRequest, Phase1CallResult};
use crate::config::WebSocketBackendConfig;
use crate::error::LoadtestError;

const FRAME_SAMPLES: usize = 320;
const FRAME_DURATION_MS: u64 = 20;
const SESSION_READY_TIMEOUT_MS: u64 = 2_000;
/// How many 20 ms silence frames to send after the speech payload so the
/// server-side VAD can detect SpeechEnded and forward audio to ASR.
/// 1 000 ms / 20 ms = 50 frames.
const SILENCE_FRAMES_AFTER_TX: usize = 50;
const SAMPLE_RATE: u64 = 16_000;

/// Events forwarded from the RX task to the main driving loop.
enum RxEvent {
    /// Decoded TTS audio chunk.  `arrival_ms` is the absolute time (ms since
    /// session start) at which this frame was received — used to place the
    /// chunk at the correct position in the final conversation timeline.
    Audio { arrival_ms: u64, samples: Vec<i16> },
    /// Server signaled the pipeline is fully initialized and ready to accept audio.
    SessionReady,
    /// Server sent `{"type":"tts_complete"}` — the current turn's TTS is done.
    TtsComplete,
}

struct TurnTtsAudio {
    first_arrival_ms: Option<u64>,
    samples: Vec<i16>,
}

/// Backend that connects directly to a voicebot-core WebSocket server.
///
/// Each "call" is a single persistent WebSocket session.  Per session the
/// client repeats `turns_per_session` turns:
///
///   1. Stream the input WAV at real-time pace (16 kHz LE-PCM binary frames).
///   2. Follow with silence frames so the VAD detects SpeechEnded.
///   3. Wait for `{"type":"tts_complete"}` (or `turn_timeout_ms`).
///   4. Repeat from step 1 for the next turn.
///
/// After all turns, send `{"type":"session_end"}` and close.
pub struct WsBackend {
    config: WebSocketBackendConfig,
}

impl WsBackend {
    pub fn new(config: WebSocketBackendConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Phase1Backend for WsBackend {
    async fn run_single_outbound_call(
        &self,
        request: Phase1CallRequest,
    ) -> Result<Phase1CallResult, LoadtestError> {
        let url = &self.config.url;
        let turns = self.config.turns_per_session.max(1);
        let turn_timeout_ms = self.config.turn_timeout_ms;
        let started_at = Instant::now();

        // ── Connect ───────────────────────────────────────────────────────────
        debug!(%url, turns, "ws: connecting");
        let (ws, _resp) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|e| LoadtestError::Protocol(format!("WebSocket connect failed: {e}")))?;

        let (mut sink, stream) = ws.split();

        // ── session_start ─────────────────────────────────────────────────────
        let session_start_json = serde_json::json!({
            "type": "session_start",
            "language": self.config.language,
            "asr": self.config.asr,
            "tts": self.config.tts,
            "sample_rate": 16000u32,
        })
        .to_string();

        sink.send(Message::Text(session_start_json.into()))
            .await
            .map_err(|e| LoadtestError::Protocol(format!("failed to send session_start: {e}")))?;

        let connect_ms = started_at.elapsed().as_millis() as u64;
        info!(%url, connect_ms, turns, "ws: connected and session_start sent");

        // ── RX task ───────────────────────────────────────────────────────────
        // Runs for the whole session lifetime.  Forwards:
        //   • binary frames  → RxEvent::Audio
        //   • tts_complete   → RxEvent::TtsComplete
        let (rx_tx, mut rx_rx) = tokio::sync::mpsc::unbounded_channel::<RxEvent>();
        let started_at_clone = started_at;

        let rx_handle = tokio::spawn(async move {
            let mut stream = stream;
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(Message::Binary(bytes)) => {
                        if bytes.len() % 2 != 0 {
                            warn!(
                                "ws rx: binary frame with odd length {}, skipping",
                                bytes.len()
                            );
                            continue;
                        }
                        let samples: Vec<i16> = bytes
                            .chunks_exact(2)
                            .map(|b| i16::from_le_bytes([b[0], b[1]]))
                            .collect();
                        let arrival_ms = started_at_clone.elapsed().as_millis() as u64;
                        let _ = rx_tx.send(RxEvent::Audio {
                            arrival_ms,
                            samples,
                        });
                    }
                    Ok(Message::Text(text)) => {
                        debug!("ws rx: event: {}", text);
                        if text.contains("\"session_ready\"") {
                            let _ = rx_tx.send(RxEvent::SessionReady);
                            continue;
                        }
                        if text.contains("\"tts_complete\"") {
                            let _ = rx_tx.send(RxEvent::TtsComplete);
                        }
                    }
                    Ok(Message::Close(_)) => {
                        debug!("ws rx: server closed connection");
                        break;
                    }
                    Err(e) => {
                        debug!("ws rx: error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
        });

        let session_ready_deadline =
            tokio::time::sleep(Duration::from_millis(SESSION_READY_TIMEOUT_MS));
        tokio::pin!(session_ready_deadline);
        loop {
            tokio::select! {
                _ = &mut session_ready_deadline => {
                    warn!(%url, timeout_ms = SESSION_READY_TIMEOUT_MS, "ws: session_ready not received before timeout; proceeding anyway");
                    break;
                }
                event = rx_rx.recv() => {
                    match event {
                        Some(RxEvent::SessionReady) => {
                            debug!(%url, "ws: session_ready received");
                            break;
                        }
                        Some(RxEvent::Audio { .. }) | Some(RxEvent::TtsComplete) => {
                            // Ignore unexpected pre-turn events and keep waiting for readiness.
                        }
                        None => {
                            warn!(%url, "ws: RX channel closed before session_ready; proceeding anyway");
                            break;
                        }
                    }
                }
            }
        }

        // ── Settle delay ──────────────────────────────────────────────────────
        if request.settle_before_playback_ms > 0 {
            sleep(Duration::from_millis(request.settle_before_playback_ms)).await;
        }

        // ── Multi-turn loop ───────────────────────────────────────────────────
        // Per-turn TTS is anchored at the first chunk arrival and then laid out
        // sequentially. This preserves turn spacing while avoiding duplicated /
        // overlaid audio when multiple chunks arrive in the same millisecond.
        let mut all_tts_turns: Vec<TurnTtsAudio> = Vec::with_capacity(turns);
        let mut tx_started_at_ms = started_at.elapsed().as_millis() as u64;
        // tx_finished_at_ms is anchored to the FIRST turn so latency analysis
        // correctly measures time-to-first-response from the very first TX end.
        let mut tx_finished_at_ms = tx_started_at_ms;
        // Track TX start time for each turn so we can mix user audio into the
        // recorded timeline (producing a full conversation recording).
        let mut turn_tx_start_ms: Vec<u64> = Vec::with_capacity(turns);

        for turn in 0..turns {
            debug!(turn, turns, "ws: starting turn");

            // ── Send speech frames ────────────────────────────────────────────
            tx_started_at_ms = started_at.elapsed().as_millis() as u64;
            turn_tx_start_ms.push(tx_started_at_ms);
            let tx_start_instant = Instant::now();

            for (frame_idx, chunk) in request.tx_samples.chunks(FRAME_SAMPLES).enumerate() {
                let bytes: Vec<u8> = chunk.iter().flat_map(|&s| s.to_le_bytes()).collect();
                sink.send(Message::Binary(bytes.into()))
                    .await
                    .map_err(|e| LoadtestError::Protocol(format!("ws send audio failed: {e}")))?;
                let next_frame_time =
                    Duration::from_millis((frame_idx as u64 + 1) * FRAME_DURATION_MS);
                let elapsed = tx_start_instant.elapsed();
                if elapsed < next_frame_time {
                    sleep(next_frame_time - elapsed).await;
                }
            }

            let turn_tx_finished = started_at.elapsed().as_millis() as u64;
            // Anchor tx_finished_at_ms to the first turn only.
            if turn == 0 {
                tx_finished_at_ms = turn_tx_finished;
            }
            debug!(
                turn,
                turn_tx_finished, "ws: speech TX done, sending silence for VAD"
            );

            // ── Post-speech silence so VAD detects SpeechEnded ───────────────
            let silence_frame: Vec<u8> = vec![0u8; FRAME_SAMPLES * 2];
            let silence_start = Instant::now();
            for i in 0..SILENCE_FRAMES_AFTER_TX {
                sink.send(Message::Binary(silence_frame.clone().into()))
                    .await
                    .map_err(|e| LoadtestError::Protocol(format!("ws send silence failed: {e}")))?;
                let next_frame_time = Duration::from_millis((i as u64 + 1) * FRAME_DURATION_MS);
                let elapsed = silence_start.elapsed();
                if elapsed < next_frame_time {
                    sleep(next_frame_time - elapsed).await;
                }
            }

            // ── Wait for tts_complete (or per-turn timeout) ───────────────────
            let turn_deadline = tokio::time::sleep(Duration::from_millis(turn_timeout_ms));
            tokio::pin!(turn_deadline);

            debug!(turn, turn_timeout_ms, "ws: waiting for tts_complete");
            let mut turn_audio = TurnTtsAudio {
                first_arrival_ms: None,
                samples: Vec::new(),
            };
            'wait: loop {
                tokio::select! {
                    biased;
                    _ = &mut turn_deadline => {
                        warn!(turn, "ws: turn timeout waiting for tts_complete");
                        break 'wait;
                    }
                    event = rx_rx.recv() => {
                        match event {
                            Some(RxEvent::SessionReady) => {}
                            Some(RxEvent::Audio { arrival_ms, samples }) => {
                                if turn_audio.first_arrival_ms.is_none() {
                                    turn_audio.first_arrival_ms = Some(arrival_ms);
                                }
                                turn_audio.samples.extend(samples);
                            }
                            Some(RxEvent::TtsComplete) => {
                                debug!(turn, "ws: tts_complete received");
                                break 'wait;
                            }
                            None => {
                                debug!("ws: RX channel closed mid-session");
                                break 'wait;
                            }
                        }
                    }
                }
            }

            info!(
                turn,
                turns,
                tts_samples_this_turn = turn_audio.samples.len(),
                "ws: turn complete"
            );
            all_tts_turns.push(turn_audio);
        }

        // ── Drain any in-flight audio after the last turn ─────────────────────
        while let Ok(event) = rx_rx.try_recv() {
            if let RxEvent::Audio {
                arrival_ms,
                samples,
            } = event
            {
                if let Some(last_turn) = all_tts_turns.last_mut() {
                    if last_turn.first_arrival_ms.is_none() {
                        last_turn.first_arrival_ms = Some(arrival_ms);
                    }
                    last_turn.samples.extend(samples);
                }
            }
        }

        // ── session_end ───────────────────────────────────────────────────────
        let session_end_json = serde_json::json!({"type": "session_end"}).to_string();
        let _ = sink.send(Message::Text(session_end_json.into())).await;
        let _ = sink.send(Message::Close(None)).await;
        rx_handle.abort();

        // Final drain after abort.
        while let Ok(event) = rx_rx.try_recv() {
            if let RxEvent::Audio {
                arrival_ms,
                samples,
            } = event
            {
                if let Some(last_turn) = all_tts_turns.last_mut() {
                    if last_turn.first_arrival_ms.is_none() {
                        last_turn.first_arrival_ms = Some(arrival_ms);
                    }
                    last_turn.samples.extend(samples);
                }
            }
        }

        // ── Build conversation timeline buffer ────────────────────────────────
        // Every TTS turn and every TX turn are placed at their absolute time
        // offset so the saved rx.wav looks like a real interleaved conversation:
        //   [user speaks] [silence] [bot responds] [user speaks] …
        //
        // First compute the required buffer length.
        let mut total_samples: usize = 0;
        for turn_audio in &all_tts_turns {
            if let Some(arrival_ms) = turn_audio.first_arrival_ms {
                let end = (arrival_ms * SAMPLE_RATE / 1000) as usize + turn_audio.samples.len();
                total_samples = total_samples.max(end);
            }
        }
        for &turn_start in &turn_tx_start_ms {
            let end = (turn_start * SAMPLE_RATE / 1000) as usize + request.tx_samples.len();
            total_samples = total_samples.max(end);
        }

        let mut recorded_samples = vec![0i16; total_samples];

        // Mix in TTS audio at the first-arrival offset for each turn.
        for turn_audio in &all_tts_turns {
            if let Some(arrival_ms) = turn_audio.first_arrival_ms {
                let offset = (arrival_ms * SAMPLE_RATE / 1000) as usize;
                for (i, &s) in turn_audio.samples.iter().enumerate() {
                    recorded_samples[offset + i] = recorded_samples[offset + i].saturating_add(s);
                }
            }
        }

        // Mix in TX (user) audio at absolute positions.
        for &turn_start in &turn_tx_start_ms {
            let offset = (turn_start * SAMPLE_RATE / 1000) as usize;
            for (i, &s) in request.tx_samples.iter().enumerate() {
                recorded_samples[offset + i] = recorded_samples[offset + i].saturating_add(s);
            }
        }

        info!(
            %url,
            connect_ms,
            turns,
            tx_finished_at_ms,
            recorded_samples = recorded_samples.len(),
            "ws: session complete"
        );

        Ok(Phase1CallResult {
            connect_ms,
            tx_started_at_ms,
            tx_finished_at_ms,
            recorded_samples,
            hangup_received: false,
        })
    }

    fn backend_name(&self) -> &'static str {
        "websocket"
    }
}
