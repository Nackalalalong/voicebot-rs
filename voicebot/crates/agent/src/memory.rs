use std::collections::VecDeque;

use common::types::{Message, Role};

pub struct ConversationMemory {
    messages: VecDeque<Message>,
    max_turns: usize,
}

impl ConversationMemory {
    pub fn new(max_turns: usize) -> Self {
        Self {
            messages: VecDeque::new(),
            max_turns,
        }
    }

    pub fn push(&mut self, msg: Message) {
        self.messages.push_back(msg);
        // Trim oldest, but keep system message at index 0
        while self.messages.len() > self.max_turns * 2 {
            if self
                .messages
                .front()
                .map_or(false, |m| m.role == Role::System)
            {
                if self.messages.len() > 1 {
                    self.messages.remove(1);
                } else {
                    break;
                }
            } else {
                self.messages.pop_front();
            }
        }
    }

    pub fn messages(&self) -> Vec<Message> {
        self.messages.iter().cloned().collect()
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
        assert_eq!(mem.messages().len(), 2);
    }

    #[test]
    fn test_max_turns_trimming() {
        let mut mem = ConversationMemory::new(2);
        // 2 turns * 2 = 4 messages max
        for i in 0..6 {
            mem.push(Message::user(&format!("msg {}", i)));
        }
        assert!(mem.messages().len() <= 4);
    }

    #[test]
    fn test_system_message_preserved() {
        let mut mem = ConversationMemory::new(2);
        mem.push(Message::system("You are helpful"));
        for i in 0..10 {
            mem.push(Message::user(&format!("msg {}", i)));
        }
        let msgs = mem.messages();
        assert_eq!(msgs[0].role, Role::System);
    }

    #[test]
    fn test_clear() {
        let mut mem = ConversationMemory::new(5);
        mem.push(Message::user("hello"));
        mem.clear();
        assert!(mem.messages().is_empty());
    }
}
