use std::sync::Arc;

use rig::client::CompletionClient;
use rig::completion::{Message, Prompt};
use rig::memory::{Compactor, MemoryError};
use rig::wasm_compat::WasmBoxedFuture;

pub struct TsukkomiCompactor<C: CompletionClient> {
    client: Arc<C>,
    model: String,
    max_chars: usize,
    header: String,
}

impl<C: CompletionClient> TsukkomiCompactor<C> {
    pub fn new(client: Arc<C>, model: String, max_chars: usize, header: String) -> Self {
        Self {
            client,
            model,
            max_chars,
            header,
        }
    }
}

fn summary_system_prompt(max_chars: usize) -> String {
    format!(
        include_str!("../prompts/summary.md"),
        max_chars
    )
}

impl<C> Compactor for TsukkomiCompactor<C>
where
    C: CompletionClient + Send + Sync + 'static,
    C::CompletionModel: 'static,
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

            let agent = self
                .client
                .agent(&self.model)
                .preamble(&summary_system_prompt(self.max_chars))
                .build();

            let summary = agent
                .prompt(&payload)
                .await
                .map_err(|e| MemoryError::Backend(e.into()))?;

            Ok(Message::System {
                content: format!("{}：{}", self.header, summary),
            })
        })
    }
}
