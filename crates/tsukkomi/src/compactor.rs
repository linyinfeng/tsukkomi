use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rig::client::CompletionClient;
use rig::completion::{Message, Prompt};
use rig::memory::{Compactor, MemoryError};
use rig::providers::deepseek;
use rig::wasm_compat::WasmBoxedFuture;

use crate::cli::CompactorOptions;

pub struct TsukkomiCompactor {
    client: Arc<deepseek::Client>,
    opts: CompactorOptions,
    pending: Mutex<HashMap<String, Vec<Message>>>,
}

impl TsukkomiCompactor {
    pub fn new(client: Arc<deepseek::Client>, opts: CompactorOptions) -> Self {
        Self {
            client,
            opts,
            pending: Mutex::new(HashMap::new()),
        }
    }
}

fn summary_system_prompt(max_chars: usize) -> String {
    format!(
        "你是一个群聊对话摘要机器人。\
         将输入的对话压缩为简洁的中文摘要，保留关键话题和重要上下文。\
         摘要不超过 {} 字。",
        max_chars
    )
}

impl Compactor for TsukkomiCompactor {
    type Artifact = Message;

    fn compact<'a>(
        &'a self,
        conversation_id: &'a str,
        evicted: &'a [Message],
        carry_over: Option<&'a Self::Artifact>,
    ) -> WasmBoxedFuture<'a, Result<Self::Artifact, MemoryError>> {
        Box::pin(async move {
            let batch = {
                let mut pending = self.pending.lock().map_err(|e| {
                    MemoryError::Internal(format!("pending lock: {e}"))
                })?;
                let buf = pending.entry(conversation_id.to_string()).or_default();
                buf.extend_from_slice(evicted);

                if buf.len() < self.opts.interval as usize {
                    return Ok(carry_over.cloned().unwrap_or(Message::System {
                        content: String::new(),
                    }));
                }

                std::mem::take(buf)
            };

            let mut messages: Vec<Message> = Vec::with_capacity(batch.len() + 1);
            if let Some(prev) = carry_over {
                messages.push(prev.clone());
            }
            messages.extend(batch);
            let payload =
                serde_json::to_string(&messages).map_err(|e| MemoryError::Backend(e.into()))?;

            let agent = self
                .client
                .agent(&self.opts.model)
                .preamble(&summary_system_prompt(self.opts.max_chars as usize))
                .build();

            let summary = agent
                .prompt(&payload)
                .await
                .map_err(|e| MemoryError::Backend(e.into()))?;

            Ok(Message::System {
                content: format!("{}：{}", self.opts.header, summary),
            })
        })
    }
}
