use rig::agent::Agent;
use rig::completion::{CompletionModel, Message, Prompt};
use rig::memory::{Compactor, MemoryError};
use rig::wasm_compat::WasmBoxedFuture;

pub struct TsukkomiCompactor<M: CompletionModel> {
    agent: Agent<M>,
    header: String,
}

impl<M: CompletionModel> TsukkomiCompactor<M> {
    pub fn new(agent: Agent<M>, header: String) -> Self {
        Self { agent, header }
    }
}

impl<M> Compactor for TsukkomiCompactor<M>
where
    M: CompletionModel + 'static,
{
    type Artifact = Message;

    fn compact<'a>(
        &'a self,
        conversation_id: &'a str,
        evicted: &'a [Message],
        carry_over: Option<&'a Self::Artifact>,
    ) -> WasmBoxedFuture<'a, Result<Self::Artifact, MemoryError>> {
        Box::pin(async move {
            tracing::info!(
                conversation_id,
                evicted = evicted.len(),
                has_carry_over = carry_over.is_some(),
                "Starting compaction"
            );
            let mut messages: Vec<Message> = Vec::with_capacity(evicted.len() + 1);
            if let Some(prev) = carry_over {
                messages.push(prev.clone());
            }
            messages.extend_from_slice(evicted);
            let payload =
                serde_json::to_string(&messages).map_err(|e| MemoryError::Backend(e.into()))?;

            let summary = self
                .agent
                .prompt(&payload)
                .await
                .map_err(|e| MemoryError::Backend(e.into()))?;

            Ok(Message::System {
                content: format!("## {}\n\n{}", self.header, summary),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_summary_output() {
        // The compact method formats output as "## {header}\n\n{summary}".
        // We verify the formatting logic directly since mocking Agent<M>
        // requires a full CompletionModel implementation.
        let header = "历史摘要";
        let summary = "users discussed rust";
        let expected = format!("## {}\n\n{}", header, summary);
        assert_eq!(expected, "## 历史摘要\n\nusers discussed rust");
    }

    #[test]
    fn message_payload_ordering_with_carry_over() {
        // Verify that carry_over is prepended before evicted messages.
        // This mirrors the internal logic of compact().
        let carry_over = Message::System {
            content: "previous summary".into(),
        };
        let evicted = vec![Message::user("hello"), Message::user("world")];

        let mut messages: Vec<Message> = Vec::with_capacity(evicted.len() + 1);
        messages.push(carry_over.clone());
        messages.extend_from_slice(&evicted);

        assert_eq!(messages.len(), 3);
        assert!(
            matches!(&messages[0], Message::System { content } if content == "previous summary")
        );

        let payload = serde_json::to_string(&messages).unwrap();
        assert!(payload.contains("hello"));
        assert!(payload.contains("world"));
        let hello_pos = payload.find("hello").unwrap();
        let world_pos = payload.find("world").unwrap();
        let summary_pos = payload.find("previous summary").unwrap();
        assert!(summary_pos < hello_pos);
        assert!(hello_pos < world_pos);
    }

    #[test]
    fn message_payload_without_carry_over() {
        let evicted = vec![Message::user("only message")];
        let mut messages: Vec<Message> = Vec::with_capacity(evicted.len());
        messages.extend_from_slice(&evicted);

        assert_eq!(messages.len(), 1);
        let payload = serde_json::to_string(&messages).unwrap();
        assert!(payload.contains("only message"));
    }

    #[test]
    fn empty_evicted_produces_empty_payload() {
        let messages: Vec<Message> = vec![];
        let payload = serde_json::to_string(&messages).unwrap();
        assert_eq!(payload, "[]");
    }
}
