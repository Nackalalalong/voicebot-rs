use async_trait::async_trait;
use common::error::AsrError;
use common::events::PipelineEvent;
use common::traits::{AsrProvider, AudioInputStream};
use tokio::sync::mpsc::Sender;
use tracing::debug;

pub struct StubAsrProvider;

#[async_trait]
impl AsrProvider for StubAsrProvider {
    async fn stream(
        &self,
        mut audio: Box<dyn AudioInputStream>,
        tx: Sender<PipelineEvent>,
    ) -> Result<(), AsrError> {
        // Drain all audio frames
        while audio.recv().await.is_some() {}

        debug!("StubAsrProvider: emitting FinalTranscript");
        tx.send(PipelineEvent::FinalTranscript {
            text: "stub transcript".into(),
            language: "en".into(),
        })
        .await
        .map_err(|_| AsrError::ChannelClosed)?;

        Ok(())
    }

    async fn cancel(&self) {
        // no-op
    }
}
