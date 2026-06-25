use std::sync::Arc;

use rig::agent::Agent;
use rig::client::CompletionClient;
use rig::client::ProviderClient;
use rig::completion::{Message, Prompt};
use rig::memory::{Compactor, ConversationMemory, MemoryPolicy};
use rig::providers::deepseek;
use rig::schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::cli::TsukkomiOptions;
use crate::compactor::TsukkomiCompactor;
use crate::memory::FileMemory;
use crate::window::BatchedSlidingWindow;

const RETRY_PROMPT: &str =
    "Your response was not valid JSON. Reply with valid JSON matching the Response schema.";

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
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Response {
    pub should_reply: bool,
    pub reply: String,
}

pub fn system_prompt() -> &'static str {
    r"你是一个群聊参与者，角色是活跃群气氛的吐槽役。
你的目标是引导对话健康持续，让群氛围变得有趣。
你可以吐槽、调侃、接梗，也可以一针见血地指出问题关键、给出建设性意见。
用简短的中文（50字以内）回应，语气友善，像朋友间的交流。
不要用敬语。

判断这条消息是否值得回复。不是每条消息都需要你参与，但如果话题需要引导、有槽点、或需要你来活跃气氛，应该回复。
"}

fn format_system_prompt() -> String {
    let input_schema = rig::schemars::schema_for!(MessagePayload);
    let input_json = serde_json::to_string_pretty(&input_schema).unwrap();
    let output_schema = rig::schemars::schema_for!(Response);
    let output_json = serde_json::to_string_pretty(&output_schema).unwrap();

    format!(
        "用户消息以 JSON 格式发送，MessagePayload schema 如下：\n{input_json}\n\n\
         你必须以 JSON 格式回复，Response schema 如下（只返回 JSON，不要包含其他文字）：\n{output_json}"
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
            .memory(Arc::clone(&memory))
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
        let mut system_prompt = opts.system_prompt.clone();
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&format_system_prompt());
        system_prompt
    }

    pub async fn reply(
        &self,
        room_id: &str,
        msg: MessagePayload,
    ) -> anyhow::Result<Option<Response>> {
        let _messages = self.compact_before_prompt(room_id).await;

        let payload = serde_json::to_string(&msg)?;
        tracing::info!(room_id, ?msg, "Sending payload");

        for attempt in 0..self.max_retries {
            let prompt = if attempt == 0 { &payload } else { RETRY_PROMPT };
            let response = self.agent.prompt(prompt).conversation(room_id).await?;
            match serde_json::from_str::<Response>(&response) {
                Ok(reply) => {
                    tracing::info!(room_id, ?reply, "Received reply");
                    if reply.should_reply {
                        return Ok(Some(reply));
                    } else {
                        return Ok(None);
                    }
                }
                Err(e) => {
                    tracing::warn!(attempt, error = %e, raw = %response, "Failed to parse AI response");
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

        let (kept, demoted) = match self.window.apply_with_demoted(messages) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to apply window");
                return Vec::new();
            }
        };

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
