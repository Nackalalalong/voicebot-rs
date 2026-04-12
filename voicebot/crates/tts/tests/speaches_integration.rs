use common::events::PipelineEvent;
use common::traits::TtsProvider;
use std::time::Duration;
use tokio::sync::mpsc;

fn speaches_base_url() -> String {
    std::env::var("SPEACHES_BASE_URL").unwrap_or_else(|_| "http://localhost:8000".into())
}

fn speaches_tts_model() -> String {
    std::env::var("SPEACHES_TTS_MODEL").unwrap_or_else(|_| "speaches-ai/Kokoro-82M-v1.0-ONNX".into())
}

fn speaches_tts_voice() -> String {
    std::env::var("SPEACHES_TTS_VOICE").unwrap_or_else(|_| "af_heart".into())
}

#[tokio::test]
#[ignore = "requires running Speaches server"]
async fn test_speaches_tts_synthesizes_text() {
    let provider = tts::speaches::SpeachesTtsProvider::new(
        speaches_base_url(),
        speaches_tts_model(),
        speaches_tts_voice(),
    );

    let (text_tx, text_rx) = mpsc::channel::<String>(10);
    let (event_tx, mut event_rx) = mpsc::channel::<PipelineEvent>(200);

    // Send a short text to synthesize
    text_tx.send("Hello, how are you?".into()).await.unwrap();
    drop(text_tx); // Close the stream so synthesize finishes

    provider
        .synthesize(text_rx, event_tx)
        .await
        .expect("TTS synthesis should succeed");

    // Collect all audio chunks
    let mut audio_chunks = 0;
    let mut total_samples = 0;
    let mut got_complete = false;

    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(100), event_rx.recv()).await
    {
        match event {
            PipelineEvent::TtsAudioChunk { frame, sequence } => {
                audio_chunks += 1;
                total_samples += frame.num_samples();
                assert_eq!(frame.sample_rate, 16000);
                assert_eq!(frame.channels, 1);
                println!("TTS chunk #{sequence}: {} samples", frame.num_samples());
            }
            PipelineEvent::TtsComplete => {
                got_complete = true;
            }
            _ => {}
        }
    }

    assert!(audio_chunks > 0, "should receive at least one audio chunk");
    assert!(got_complete, "should receive TtsComplete event");
    println!(
        "TTS produced {} chunks, {} total samples ({:.1}s of audio)",
        audio_chunks,
        total_samples,
        total_samples as f64 / 16000.0
    );
}

#[tokio::test]
#[ignore = "requires running Speaches server"]
async fn test_speaches_tts_multiple_sentences() {
    let provider = tts::speaches::SpeachesTtsProvider::new(
        speaches_base_url(),
        speaches_tts_model(),
        speaches_tts_voice(),
    );

    let (text_tx, text_rx) = mpsc::channel::<String>(10);
    let (event_tx, mut event_rx) = mpsc::channel::<PipelineEvent>(200);

    // Send multiple sentences
    text_tx.send("First sentence.".into()).await.unwrap();
    text_tx.send("Second sentence.".into()).await.unwrap();
    drop(text_tx);

    provider
        .synthesize(text_rx, event_tx)
        .await
        .expect("TTS multi-sentence should succeed");

    let mut audio_chunks = 0;
    let mut got_complete = false;

    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(100), event_rx.recv()).await
    {
        match event {
            PipelineEvent::TtsAudioChunk { .. } => audio_chunks += 1,
            PipelineEvent::TtsComplete => got_complete = true,
            _ => {}
        }
    }

    assert!(audio_chunks > 0, "should receive audio chunks");
    assert!(got_complete, "should receive TtsComplete");
    println!("Multi-sentence TTS: {audio_chunks} chunks");
}

#[tokio::test]
#[ignore = "requires running Speaches server"]
async fn test_speaches_tts_empty_text_skipped() {
    let provider = tts::speaches::SpeachesTtsProvider::new(
        speaches_base_url(),
        speaches_tts_model(),
        speaches_tts_voice(),
    );

    let (text_tx, text_rx) = mpsc::channel::<String>(10);
    let (event_tx, mut event_rx) = mpsc::channel::<PipelineEvent>(200);

    // Send empty string then real text
    text_tx.send("".into()).await.unwrap();
    text_tx.send("Hello.".into()).await.unwrap();
    drop(text_tx);

    provider
        .synthesize(text_rx, event_tx)
        .await
        .expect("TTS with empty text should succeed");

    let mut audio_chunks = 0;
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(100), event_rx.recv()).await
    {
        if matches!(event, PipelineEvent::TtsAudioChunk { .. }) {
            audio_chunks += 1;
        }
    }

    assert!(audio_chunks > 0, "should still produce audio for non-empty text");
}

#[tokio::test]
#[ignore = "requires running Speaches server"]
async fn test_speaches_tts_cancel() {
    let provider = tts::speaches::SpeachesTtsProvider::new(
        speaches_base_url(),
        speaches_tts_model(),
        speaches_tts_voice(),
    );

    // Cancel before sending any text
    provider.cancel().await;

    let (text_tx, text_rx) = mpsc::channel::<String>(10);
    let (event_tx, _event_rx) = mpsc::channel::<PipelineEvent>(200);

    text_tx
        .send("This should be cancelled.".into())
        .await
        .unwrap();
    drop(text_tx);

    let result = provider.synthesize(text_rx, event_tx).await;
    assert!(result.is_err(), "cancelled TTS should return error");
}
