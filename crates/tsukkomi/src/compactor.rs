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
        Self {
            agent,
            header,
        }
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

            let summary = self.agent
                .prompt(&payload)
                .await
                .map_err(|e| MemoryError::Backend(e.into()))?;

            Ok(Message::System {
                content: format!("## {}\n\n{}", self.header, summary),
            })
        })
    }
}
