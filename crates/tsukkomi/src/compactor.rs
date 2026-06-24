use std::collections::HashMap;
use std::sync::Mutex;

use rig::client::CompletionClient;
use rig::completion::message::{AssistantContent, UserContent};
use rig::completion::{Message, Prompt};
use rig::memory::{Compactor, MemoryError};
use rig::providers::deepseek;
use rig::schemars::JsonSchema;
use rig::wasm_compat::WasmBoxedFuture;
use serde::Serialize;

#[derive(Serialize, JsonSchema)]
struct SummaryMessage {
    role: String,
    name: Option<String>,
    content: String,
}

#[derive(Serialize, JsonSchema)]
struct SummaryInput {
    previous_summary: String,
    conversation: Vec<SummaryMessage>,
}

fn format_messages(evicted: &[Message]) -> Vec<SummaryMessage> {
    evicted
        .iter()
        .filter_map(|msg| {
            let (role, content) = match msg {
                Message::User { content } => {
                    let text: Vec<&str> = content
                        .iter()
                        .filter_map(|c| match c {
                            UserContent::Text(t) => Some(t.text.as_str()),
                            _ => None,
                        })
                        .collect();
                    if text.is_empty() {
                        return None;
                    }
                    ("user", text.join(" "))
                }
                Message::Assistant { content, .. } => {
                    let text: Vec<&str> = content
                        .iter()
                        .filter_map(|c| match c {
                            AssistantContent::Text(t) => Some(t.text.as_str()),
                            _ => None,
                        })
                        .collect();
                    if text.is_empty() {
                        return None;
                    }
                    ("assistant", text.join(" "))
                }
                Message::System { content } => {
                    if content.is_empty() {
                        return None;
                    }
                    return Some(SummaryMessage {
                        role: "system".into(),
                        name: None,
                        content: content.clone(),
                    });
                }
            };
            Some(SummaryMessage {
                role: role.into(),
                name: None,
                content,
            })
        })
        .collect()
}

pub struct TsukkomiCompactor {
    client: deepseek::Client,
    model: String,
    header: String,
    max_chars: usize,
    interval: usize,
    pending: Mutex<HashMap<String, Vec<Message>>>,
}

impl TsukkomiCompactor {
    pub fn new(
        client: deepseek::Client,
        model: String,
        header: String,
        max_chars: usize,
        interval: usize,
    ) -> Self {
        Self {
            client,
            model,
            header,
            max_chars,
            interval,
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
            let previous = carry_over.and_then(|m| match m {
                Message::System { content } => Some(content.clone()),
                _ => None,
            });

            let batch = {
                let mut pending = self.pending.lock().map_err(|e| {
                    MemoryError::Internal(format!("pending lock: {e}"))
                })?;
                let buf = pending.entry(conversation_id.to_string()).or_default();
                buf.extend_from_slice(evicted);

                if buf.len() < self.interval {
                    return Ok(Message::System {
                        content: previous.unwrap_or_default(),
                    });
                }

                std::mem::take(buf)
            };

            let conversation = format_messages(&batch);
            let input = SummaryInput {
                previous_summary: previous.unwrap_or_default(),
                conversation,
            };
            let payload =
                serde_json::to_string(&input).map_err(|e| MemoryError::Backend(e.into()))?;

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
