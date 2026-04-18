use std::sync::Arc;

use async_trait::async_trait;
use common::types::{Message, Role};
use tracing::warn;
use uuid::Uuid;

#[async_trait]
pub trait ConversationMemoryBackend: Send + Sync {
    async fn load(&self, session_id: Uuid) -> Result<Option<Vec<Message>>, String>;
    async fn save(&self, session_id: Uuid, messages: &[Message]) -> Result<(), String>;
    async fn clear(&self, session_id: Uuid) -> Result<(), String>;
}

pub struct ConversationMemory {
    session_id: Option<Uuid>,
    messages: Vec<Message>,
    max_turns: usize,
    backend: Option<Arc<dyn ConversationMemoryBackend>>,
    loaded: bool,
    system_prompt: Option<String>,
}

impl ConversationMemory {
    pub fn new(max_turns: usize) -> Self {
        Self {
            session_id: None,
            messages: Vec::new(),
            max_turns,
            backend: None,
            loaded: true,
            system_prompt: None,
        }
    }

    pub fn with_backend(
        max_turns: usize,
        session_id: Uuid,
        system_prompt: Option<String>,
        backend: Arc<dyn ConversationMemoryBackend>,
    ) -> Self {
        Self {
            session_id: Some(session_id),
            messages: Vec::new(),
            max_turns,
            backend: Some(backend),
            loaded: false,
            system_prompt,
        }
    }

    pub fn push_local(&mut self, msg: Message) {
        self.messages.push(msg);
        self.trim_to_max_turns();
    }

    pub async fn push(&mut self, msg: Message) {
        self.ensure_loaded().await;
        self.push_local(msg);
        self.persist().await;
    }

    pub async fn snapshot(&mut self) -> Vec<Message> {
        self.ensure_loaded().await;
        self.messages.clone()
    }

    pub async fn set_system_prompt(&mut self, system_prompt: Option<String>) {
        self.ensure_loaded().await;

        let new_prompt = system_prompt.as_deref();
        let existing_prompt = self
            .messages
            .first()
            .and_then(|message| (message.role == Role::System).then_some(message.content.as_deref()))
            .flatten();

        if existing_prompt == new_prompt {
            self.system_prompt = system_prompt;
            return;
        }

        self.system_prompt = system_prompt;
        match self.system_prompt.as_deref() {
            Some(prompt) => {
                if self
                    .messages
                    .first()
                    .map_or(false, |message| message.role == Role::System)
                {
                    if let Some(first) = self.messages.first_mut() {
                        first.content = Some(prompt.to_string());
                    }
                } else {
                    self.messages.insert(0, Message::system(prompt));
                }
            }
            None => {
                if self
                    .messages
                    .first()
                    .map_or(false, |message| message.role == Role::System)
                {
                    self.messages.remove(0);
                }
            }
        }

        self.trim_to_max_turns();
        self.persist().await;
    }

    /// Borrow the full message history as a slice (no allocation).
    pub fn as_slice(&self) -> &[Message] {
        &self.messages
    }

    pub async fn clear(&mut self) {
        self.ensure_loaded().await;
        self.messages.clear();
        self.system_prompt = None;
        self.clear_backend().await;
    }

    fn trim_to_max_turns(&mut self) -> bool {
        let mut trimmed = false;
        while self.messages.len() > self.max_turns * 2 {
            trimmed = true;
            if self
                .messages
                .first()
                .map_or(false, |m| m.role == Role::System)
            {
                if self.messages.len() > 1 {
                    self.messages.remove(1);
                } else {
                    break;
                }
            } else {
                self.messages.remove(0);
            }
        }
        trimmed
    }

    fn apply_system_prompt_if_missing(&mut self) -> bool {
        let Some(prompt) = self.system_prompt.as_deref() else {
            return false;
        };

        if self
            .messages
            .first()
            .map_or(false, |m| m.role == Role::System)
        {
            return false;
        }

        self.messages.insert(0, Message::system(prompt));
        true
    }

    async fn ensure_loaded(&mut self) {
        if self.loaded {
            return;
        }

        let session_id = self.session_id;
        let backend = self.backend.clone();
        if let (Some(session_id), Some(backend)) = (session_id, backend) {
            match backend.load(session_id).await {
                Ok(Some(messages)) => self.messages = messages,
                Ok(None) => {}
                Err(error) => {
                    warn!(%session_id, %error, "failed to load conversation memory from backend; falling back to local cache");
                    self.backend = None;
                }
            }
        }

        self.loaded = true;

        let mut persist_needed = self.apply_system_prompt_if_missing();
        if self.trim_to_max_turns() {
            persist_needed = true;
        }
        if persist_needed {
            self.persist().await;
        }
    }

    async fn persist(&mut self) {
        let session_id = self.session_id;
        let backend = self.backend.clone();
        let messages = self.messages.clone();

        if let (Some(session_id), Some(backend)) = (session_id, backend) {
            if let Err(error) = backend.save(session_id, &messages).await {
                warn!(%session_id, %error, "failed to persist conversation memory; continuing with local cache only");
                self.backend = None;
            }
        }
    }

    async fn clear_backend(&mut self) {
        let session_id = self.session_id;
        let backend = self.backend.clone();
        if let (Some(session_id), Some(backend)) = (session_id, backend) {
            if let Err(error) = backend.clear(session_id).await {
                warn!(%session_id, %error, "failed to clear conversation memory from backend; continuing with local cache only");
                self.backend = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tokio::sync::Mutex;

    #[derive(Default)]
    struct MockBackend {
        messages: Arc<Mutex<Option<Vec<Message>>>>,
        fail_save: bool,
    }

    #[async_trait]
    impl ConversationMemoryBackend for MockBackend {
        async fn load(&self, _session_id: Uuid) -> Result<Option<Vec<Message>>, String> {
            Ok(self.messages.lock().await.clone())
        }

        async fn save(&self, _session_id: Uuid, messages: &[Message]) -> Result<(), String> {
            if self.fail_save {
                return Err("save failed".into());
            }
            *self.messages.lock().await = Some(messages.to_vec());
            Ok(())
        }

        async fn clear(&self, _session_id: Uuid) -> Result<(), String> {
            *self.messages.lock().await = None;
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_push_and_messages() {
        let mut mem = ConversationMemory::new(5);
        mem.push(Message::user("hello")).await;
        mem.push(Message::assistant("hi")).await;
        assert_eq!(mem.as_slice().len(), 2);
    }

    #[tokio::test]
    async fn test_max_turns_trimming() {
        let mut mem = ConversationMemory::new(2);
        // 2 turns * 2 = 4 messages max
        for i in 0..6 {
            mem.push(Message::user(&format!("msg {}", i))).await;
        }
        assert!(mem.as_slice().len() <= 4);
    }

    #[tokio::test]
    async fn test_system_message_preserved() {
        let mut mem = ConversationMemory::new(2);
        mem.push(Message::system("You are helpful")).await;
        for i in 0..10 {
            mem.push(Message::user(&format!("msg {}", i))).await;
        }
        let msgs = mem.as_slice();
        assert_eq!(msgs[0].role, Role::System);
    }

    #[tokio::test]
    async fn test_clear() {
        let mut mem = ConversationMemory::new(5);
        mem.push(Message::user("hello")).await;
        mem.clear().await;
        assert!(mem.as_slice().is_empty());
    }

    #[tokio::test]
    async fn test_set_system_prompt_replaces_existing_prompt() {
        let mut mem = ConversationMemory::new(5);
        mem.push(Message::system("old prompt")).await;
        mem.push(Message::user("hello")).await;

        mem.set_system_prompt(Some("new prompt".into())).await;

        assert_eq!(mem.as_slice()[0].content.as_deref(), Some("new prompt"));
        assert_eq!(mem.as_slice()[1].content.as_deref(), Some("hello"));
    }

    #[tokio::test]
    async fn test_loads_from_backend_on_first_snapshot() {
        let backend = MockBackend {
            messages: Arc::new(Mutex::new(Some(vec![Message::user("persisted")]))),
            fail_save: false,
        };
        let mut mem = ConversationMemory::with_backend(
            5,
            Uuid::new_v4(),
            None,
            Arc::new(backend),
        );

        let messages = mem.snapshot().await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content.as_deref(), Some("persisted"));
    }

    #[tokio::test]
    async fn test_falls_back_to_local_cache_when_backend_save_fails() {
        let mut mem = ConversationMemory::with_backend(
            5,
            Uuid::new_v4(),
            None,
            Arc::new(MockBackend {
                messages: Arc::new(Mutex::new(None)),
                fail_save: true,
            }),
        );

        mem.push(Message::user("hello")).await;
        mem.push(Message::assistant("world")).await;

        assert_eq!(mem.as_slice().len(), 2);
        assert_eq!(mem.as_slice()[1].content.as_deref(), Some("world"));
    }
}
