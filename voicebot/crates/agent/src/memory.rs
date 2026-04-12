use common::types::{Message, Role};

pub struct ConversationMemory {
    messages: Vec<Message>,
    max_turns: usize,
}

impl ConversationMemory {
    pub fn new(max_turns: usize) -> Self {
        Self {
            messages: Vec::new(),
            max_turns,
        }
    }

    pub fn push(&mut self, msg: Message) {
        self.messages.push(msg);
        // Trim oldest, but keep system message at index 0
        while self.messages.len() > self.max_turns * 2 {
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
    }

    /// Borrow the full message history as a slice (no allocation).
    pub fn as_slice(&self) -> &[Message] {
        &self.messages
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_and_messages() {
        let mut mem = ConversationMemory::new(5);
        mem.push(Message::user("hello"));
        mem.push(Message::assistant("hi"));
        assert_eq!(mem.as_slice().len(), 2);
    }

    #[test]
    fn test_max_turns_trimming() {
        let mut mem = ConversationMemory::new(2);
        // 2 turns * 2 = 4 messages max
        for i in 0..6 {
            mem.push(Message::user(&format!("msg {}", i)));
        }
        assert!(mem.as_slice().len() <= 4);
    }

    #[test]
    fn test_system_message_preserved() {
        let mut mem = ConversationMemory::new(2);
        mem.push(Message::system("You are helpful"));
        for i in 0..10 {
            mem.push(Message::user(&format!("msg {}", i)));
        }
        let msgs = mem.as_slice();
        assert_eq!(msgs[0].role, Role::System);
    }

    #[test]
    fn test_clear() {
        let mut mem = ConversationMemory::new(5);
        mem.push(Message::user("hello"));
        mem.clear();
        assert!(mem.as_slice().is_empty());
    }
}
