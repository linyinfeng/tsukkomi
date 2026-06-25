use std::sync::Arc;

use rig::agent::Agent;
use rig::client::CompletionClient;
use rig::client::ProviderClient;
use rig::completion::message::UserContent;
use rig::completion::{Message, Prompt};
use rig::memory::{Compactor, ConversationMemory, MemoryError, MemoryPolicy};
use rig::providers::deepseek;
use rig::schemars::JsonSchema;
use rig::wasm_compat::WasmBoxedFuture;
use serde::{Deserialize, Serialize};

use crate::cli::TsukkomiOptions;
use crate::compactor::TsukkomiCompactor;
use crate::memory::FileMemory;
use crate::store::{CURRENT_ROOM, Forget, MemoryStore, Remember};
use crate::window::BatchedSlidingWindow;

const RETRY_PROMPT: &str =
    "Your response was not valid JSON. Reply with valid JSON matching the ResponsePayload schema.";

#[derive(Debug, Serialize, JsonSchema)]
#[serde(tag = "type", content = "data")]
pub enum MessageBody {
    Text(String),
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MessagePayload {
    pub user_id: String,
    pub display_name: String,
    pub body: MessageBody,
    pub sent_at: chrono::DateTime<chrono::Utc>,
    pub mentions_bot: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Response {
    pub text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "action")]
pub enum ResponsePayload {
    #[serde(rename = "skip")]
    Skip,
    #[serde(rename = "reply")]
    Reply(Response),
}

struct RememberingMemory {
    inner: Arc<FileMemory>,
    store: Arc<MemoryStore>,
}

impl ConversationMemory for RememberingMemory {
    fn load<'a>(
        &'a self,
        conversation_id: &'a str,
    ) -> WasmBoxedFuture<'a, Result<Vec<Message>, MemoryError>> {
        Box::pin(async move {
            let mut messages = self.inner.load(conversation_id).await?;
            let memories = self.store.list(conversation_id).await;
            if !memories.is_empty() {
                let summary = memories
                    .iter()
                    .map(|m| format!("- {}: {}", m.key, m.summary))
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

pub fn default_system_prompt() -> &'static str {
    include_str!("../prompts/default.md")
}

fn format_system_prompt() -> String {
    let input_schema = rig::schemars::schema_for!(MessagePayload);
    let input_json = serde_json::to_string_pretty(&input_schema).unwrap();
    let output_schema = rig::schemars::schema_for!(ResponsePayload);
    let output_json = serde_json::to_string_pretty(&output_schema).unwrap();

    format!(
        "# Input Format / 输入格式\n\n\
         用户消息以 JSON 格式发送，MessagePayload schema 如下：\n{input_json}\n\n\
         # Output Format / 输出格式\n\n\
         你必须以 JSON 格式回复，ResponsePayload schema 如下（只返回 JSON，不要包含其他文字）：\n{output_json}"
    )
}

pub struct ChatManager {
    agent: Agent<deepseek::CompletionModel>,
    memory: Arc<FileMemory>,
    window: BatchedSlidingWindow,
    compactor: TsukkomiCompactor<deepseek::Client>,
    max_retries: u32,
}

impl ChatManager {
    pub fn new(opts: TsukkomiOptions) -> anyhow::Result<Self> {
        let client = Arc::new(deepseek::Client::from_env()?);
        let system_prompt = Self::system_prompt(&opts);
        let max_retries = opts.max_retries;

        let memory = Arc::new(FileMemory::new(&opts.memory_directory));
        let store = Arc::new(MemoryStore::new(std::path::PathBuf::from(
            &opts.memory_directory,
        )));
        let remembering = RememberingMemory {
            inner: Arc::clone(&memory),
            store: Arc::clone(&store),
        };

        let window =
            BatchedSlidingWindow::new(opts.sliding_window as usize, opts.batch_size as usize);
        let compactor = TsukkomiCompactor::new(
            client.clone(),
            opts.summary_model,
            opts.summary_max_chars as usize,
            opts.summary_header,
        );

        let agent = client
            .agent(deepseek::DEEPSEEK_V4_FLASH)
            .preamble(&system_prompt)
            .memory(remembering)
            .tool(Remember {
                store: Arc::clone(&store),
            })
            .tool(Forget {
                store: Arc::clone(&store),
            })
            .additional_params(serde_json::json!({"response_format": {"type": "json_object"}}))
            .build();

        tracing::info!(
            system_prompt,
            max_retries,
            sliding_window = opts.sliding_window,
            "ChatManager initialized"
        );
        Ok(Self {
            agent,
            memory,
            window,
            compactor,
            max_retries,
        })
    }

    pub fn system_prompt(opts: &TsukkomiOptions) -> String {
        let base = if let Some(path) = &opts.system_prompt_file {
            std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("Failed to read system prompt file {path}: {e}"))
        } else {
            opts.system_prompt
                .clone()
                .unwrap_or_else(|| default_system_prompt().to_string())
        };
        let mut prompt = base;
        prompt.push_str("\n\n");
        prompt.push_str(&format_system_prompt());
        prompt
    }

    pub async fn reply(
        &self,
        room_id: &str,
        msg: MessagePayload,
    ) -> anyhow::Result<Option<Response>> {
        CURRENT_ROOM
            .scope(room_id.to_string(), async {
                self.reply_inner(room_id, msg).await
            })
            .await
    }

    async fn reply_inner(
        &self,
        room_id: &str,
        msg: MessagePayload,
    ) -> anyhow::Result<Option<Response>> {
        let _messages = self.compact_before_prompt(room_id).await;

        let mut payload = serde_json::to_string(&msg)?;
        tracing::info!(room_id, ?msg, "Sending payload");

        for attempt in 0..self.max_retries {
            let response = self.agent.prompt(payload).conversation(room_id).await?;
            match serde_json::from_str::<ResponsePayload>(&response) {
                Ok(ResponsePayload::Reply(resp)) => {
                    tracing::info!(room_id, ?resp, "Received reply");
                    return Ok(Some(resp));
                }
                Ok(ResponsePayload::Skip) => {
                    return Ok(None);
                }
                Err(e) => {
                    tracing::warn!(attempt, error = %e, raw = %response, "Failed to parse AI response");
                    payload = format!("{}\nError message: {e}", RETRY_PROMPT);
                }
            }
        }
        tracing::warn!("All retries exhausted");
        Ok(None)
    }

    /// Compact FileMemory before each prompt so the agent always sees a
    /// bounded history.
    ///
    /// We do NOT use rig's `CompactingMemory` because it only controls what
    /// the agent sees in-memory — it never writes the compacted form back to
    /// the underlying `FileMemory`. Without persistence, every restart would
    /// require re-compacting the entire conversation history from scratch.
    ///
    /// Instead we compact and replace the file directly, so the compacted
    /// state survives restarts. The agent's `FileMemory` always loads the
    /// already-compacted form.
    async fn compact_before_prompt(&self, room_id: &str) -> Vec<Message> {
        let messages = match self.memory.load(room_id).await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to load memory");
                return Vec::new();
            }
        };

        let count = messages.len();
        if count < self.window.window_size() + self.window.batch_size() {
            return messages;
        }

        let (mut kept, mut demoted) = match self.window.apply_with_demoted(messages) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to apply window");
                return Vec::new();
            }
        };

        // Move orphan tool results (tool results whose preceding tool call was
        // demoted) into the demoted set so they don't pollute the agent's window.
        if let Some(Message::User { content }) = kept.first()
            && matches!(content.first_ref(), UserContent::ToolResult(_))
        {
            demoted.push(kept.remove(0));
        }

        if demoted.is_empty() {
            return kept;
        }

        tracing::info!(
            room_id,
            total = kept.len() + demoted.len(),
            demoted = demoted.len(),
            "Compacting FileMemory before prompt"
        );

        let summary = match self.compactor.compact(room_id, &demoted, None).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "Compaction failed");
                return kept;
            }
        };

        let mut compacted = vec![summary];
        compacted.extend(kept);
        if let Err(e) = self.memory.replace_all(room_id, &compacted).await {
            tracing::warn!(error = %e, "Failed to persist");
        }
        compacted
    }
}
