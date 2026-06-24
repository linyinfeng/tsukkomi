use rig::completion::Message;
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
        let demoted: Vec<Message> = (&mut iter).take(demote_count).collect();
        let kept: Vec<Message> = iter.collect();
        Ok((kept, demoted))
    }
}
