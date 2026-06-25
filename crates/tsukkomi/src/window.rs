use rig::completion::Message;
use rig::completion::message::UserContent;
use rig::memory::{MemoryError, MemoryPolicy};

/// A sliding window that demotes old messages in batches.
///
/// Once the conversation exceeds `window_size` messages, older messages are
/// accumulated in the agent's window until they fill one `batch_size` batch.
/// The entire batch is then demoted at once, triggering compaction via
/// [`CompactingMemory`](rig::memory::CompactingMemory).
///
/// This avoids per-message compaction calls when messages overflow one at a time.
#[derive(Debug, Clone, Copy)]
pub struct BatchedSlidingWindow {
    window_size: usize,
    batch_size: usize,
}

impl BatchedSlidingWindow {
    pub fn new(window_size: usize, batch_size: usize) -> Self {
        Self {
            window_size,
            batch_size,
        }
    }

    pub fn window_size(&self) -> usize {
        self.window_size
    }

    pub fn batch_size(&self) -> usize {
        self.batch_size
    }
}

impl MemoryPolicy for BatchedSlidingWindow {
    fn apply(&self, messages: Vec<Message>) -> Result<Vec<Message>, MemoryError> {
        Ok(self.apply_with_demoted(messages)?.0)
    }

    fn apply_with_demoted(
        &self,
        messages: Vec<Message>,
    ) -> Result<(Vec<Message>, Vec<Message>), MemoryError> {
        let excess = messages.len().saturating_sub(self.window_size);
        let demote_count = (excess / self.batch_size) * self.batch_size;

        if demote_count == 0 {
            return Ok((messages, Vec::new()));
        }

        tracing::info!(
            total = messages.len(),
            demoted = demote_count,
            kept = messages.len() - demote_count,
            "Demoting batch of old messages"
        );

        let mut iter = messages.into_iter();
        let mut demoted: Vec<Message> = (&mut iter).take(demote_count).collect();
        let mut kept: Vec<Message> = iter.collect();

        // If the first kept message is an orphan tool result (its preceding
        // tool call was demoted), move it to demoted to keep the pair intact.
        if let Some(Message::User { content }) = kept.first()
            && matches!(content.first_ref(), UserContent::ToolResult(_))
        {
            demoted.push(kept.remove(0));
        }

        Ok((kept, demoted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_text(text: &str) -> Message {
        Message::user(text)
    }

    #[test]
    fn window_under_limit_returns_all() {
        let w = BatchedSlidingWindow::new(10, 5);
        let msgs: Vec<Message> = (0..8).map(|i| user_text(&i.to_string())).collect();
        let (kept, demoted) = w.apply_with_demoted(msgs.clone()).unwrap();
        assert_eq!(kept.len(), 8);
        assert!(demoted.is_empty());
    }

    #[test]
    fn window_exact_limit_returns_all() {
        let w = BatchedSlidingWindow::new(10, 5);
        let msgs: Vec<Message> = (0..10).map(|i| user_text(&i.to_string())).collect();
        let (kept, demoted) = w.apply_with_demoted(msgs.clone()).unwrap();
        assert_eq!(kept.len(), 10);
        assert!(demoted.is_empty());
    }

    #[test]
    fn window_exceeds_by_one_batch() {
        let w = BatchedSlidingWindow::new(10, 5);
        let msgs: Vec<Message> = (0..15).map(|i| user_text(&i.to_string())).collect();
        let (kept, demoted) = w.apply_with_demoted(msgs).unwrap();
        assert_eq!(kept.len(), 10);
        assert_eq!(demoted.len(), 5);
    }

    #[test]
    fn window_exceeds_by_less_than_batch_returns_all() {
        let w = BatchedSlidingWindow::new(10, 5);
        let msgs: Vec<Message> = (0..12).map(|i| user_text(&i.to_string())).collect();
        let (kept, demoted) = w.apply_with_demoted(msgs).unwrap();
        assert_eq!(kept.len(), 12);
        assert!(demoted.is_empty());
    }

    #[test]
    fn window_exceeds_by_two_batches() {
        let w = BatchedSlidingWindow::new(10, 5);
        let msgs: Vec<Message> = (0..20).map(|i| user_text(&i.to_string())).collect();
        let (kept, demoted) = w.apply_with_demoted(msgs).unwrap();
        assert_eq!(kept.len(), 10);
        assert_eq!(demoted.len(), 10);
    }

    #[test]
    fn window_empty_returns_empty() {
        let w = BatchedSlidingWindow::new(10, 5);
        let msgs = Vec::new();
        let (kept, demoted) = w.apply_with_demoted(msgs).unwrap();
        assert!(kept.is_empty());
        assert!(demoted.is_empty());
    }

    #[test]
    fn apply_keeps_window_size_only() {
        let w = BatchedSlidingWindow::new(10, 5);
        let msgs: Vec<Message> = (0..15).map(|i| user_text(&i.to_string())).collect();
        let kept = w.apply(msgs).unwrap();
        assert_eq!(kept.len(), 10);
    }

    #[test]
    fn orphan_tool_result_is_demoted_with_its_batch() {
        let w = BatchedSlidingWindow::new(5, 5);
        let mut msgs: Vec<Message> = (0..5).map(|i| user_text(&i.to_string())).collect();
        // Add a tool result message right after the demotion boundary
        msgs.push(Message::tool_result("tool_1", "result"));
        msgs.push(user_text("6"));
        msgs.push(user_text("7"));
        msgs.push(user_text("8"));
        msgs.push(user_text("9"));

        let total = msgs.len(); // 10
        assert_eq!(total, 10);

        let (kept, demoted) = w.apply_with_demoted(msgs).unwrap();
        // With window=5, batch=5, excess=5, demote_count=5
        // The first 5 get demoted + the orphan tool result at index 5
        assert_eq!(demoted.len(), 6);
        assert_eq!(kept.len(), 4);
    }

    #[test]
    fn window_uses_batch_size_for_demotion_granularity() {
        let w = BatchedSlidingWindow::new(10, 3);
        // 14 messages: excess = 4, demote_count = 3 (only full batch)
        let msgs: Vec<Message> = (0..14).map(|i| user_text(&i.to_string())).collect();
        let (kept, demoted) = w.apply_with_demoted(msgs).unwrap();
        assert_eq!(demoted.len(), 3);
        assert_eq!(kept.len(), 11);
    }
}
