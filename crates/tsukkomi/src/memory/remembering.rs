use std::sync::Arc;

use rig::completion::Message;
use rig::memory::{ConversationMemory, MemoryError};
use rig::wasm_compat::WasmBoxedFuture;

use super::file::FileMemory;
use super::store::MemoryStore;

pub(crate) struct RememberingMemory {
    inner: Arc<FileMemory>,
    store: Arc<MemoryStore>,
}

impl RememberingMemory {
    pub(crate) fn new(inner: Arc<FileMemory>, store: Arc<MemoryStore>) -> Self {
        Self { inner, store }
    }
}

impl ConversationMemory for RememberingMemory {
    fn load<'a>(
        &'a self,
        conversation_id: &'a str,
    ) -> WasmBoxedFuture<'a, Result<Vec<Message>, MemoryError>> {
        Box::pin(async move {
            let mut messages = self.inner.load(conversation_id).await?;
            let memories = self.store.list(conversation_id).await
                .map_err(|e| MemoryError::Backend(e.into()))?;
            if !memories.is_empty() {
                let summary = memories
                    .iter()
                    .map(|(key, mem)| format!("- {key}: {}", mem.summary))
                    .collect::<Vec<_>>()
                    .join("\n");
                messages.insert(
                    0,
                    Message::System {
                        content: format!("长期记忆：\n{summary}"),
                    },
                );
            }
            Ok(messages)
        })
    }

    fn append<'a>(
        &'a self,
        conversation_id: &'a str,
        messages: Vec<Message>,
    ) -> WasmBoxedFuture<'a, Result<(), MemoryError>> {
        self.inner.append(conversation_id, messages)
    }

    fn clear<'a>(
        &'a self,
        conversation_id: &'a str,
    ) -> WasmBoxedFuture<'a, Result<(), MemoryError>> {
        self.inner.clear(conversation_id)
    }
}
