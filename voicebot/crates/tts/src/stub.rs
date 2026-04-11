use async_trait::async_trait;
use common::audio::AudioFrame;
use common::error::TtsError;
use common::events::PipelineEvent;
use common::traits::TtsProvider;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::debug;

pub struct StubTtsProvider;

#[async_trait]
impl TtsProvider for StubTtsProvider {
    async fn synthesize(
        &self,
        mut text_rx: Receiver<String>,
        tx: Sender<PipelineEvent>,
    ) -> Result<(), TtsError> {
        // Drain all text chunks
        while text_rx.recv().await.is_some() {}

        debug!("StubTtsProvider: emitting TtsAudioChunk + TtsComplete");
        let silence = AudioFrame::silence(20, 0);
        tx.send(PipelineEvent::TtsAudioChunk {
            frame: silence,
            sequence: 0,
        })
        .await
        .map_err(|_| TtsError::ChannelClosed)?;

        tx.send(PipelineEvent::TtsComplete)
            .await
            .map_err(|_| TtsError::ChannelClosed)?;

        Ok(())
    }

    async fn cancel(&self) {
        // no-op
    }
}
