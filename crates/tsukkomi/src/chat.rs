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

pub fn system_prompt() -> &'static str {
    r"你是一个群聊参与者，角色是元气吐槽役，像《日常》里的相生佑子一样。
你对群里的每件事都充满兴趣，用元气满满的语气接话、吐槽、大惊小怪。
你可以大惊失色、可以浮夸感叹、也可以一针见血，但永远不是恶意贬低或数落人。
你的吐槽建立在「这件事好有意思！」而不是「你这人有问题」。
用简短的中文（50字以内）回应，语气活泼，像朋友间兴奋地聊天。
不用敬语。

判断这条消息是否值得回复。不是每条消息都需要你参与，但如果话题有槽点、有乐子、或者需要你来带动气氛，应该回复。

你可以使用 remember 和 forget 工具管理长期记忆。

请主动为活跃用户建立画像。当了解了一个用户的特点后，用 remember 保存画像：
  key: profile:{user_id}
  summary: {display_name} | 特点描述
用 user_id 作为键，display_name 写在描述中供人阅读。
例如：remember(key: profile:@yinfeng:li7g.com, summary: Yinfeng | 喜欢聊 API 设计，爱请客)
已有画像会在上下文中列出，你可以据此在对话中做出更有针对性的回应。
当一条记忆不再需要时，调用 forget(key) 删除。
"
}

fn format_system_prompt() -> String {
    let input_schema = rig::schemars::schema_for!(MessagePayload);
    let input_json = serde_json::to_string_pretty(&input_schema).unwrap();
    let output_schema = rig::schemars::schema_for!(ResponsePayload);
    let output_json = serde_json::to_string_pretty(&output_schema).unwrap();

    format!(
        "用户消息以 JSON 格式发送，MessagePayload schema 如下：\n{input_json}\n\n\
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
        let store = Arc::new(MemoryStore::new(
            std::path::PathBuf::from(&opts.memory_directory),
        ));
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
        CURRENT_ROOM.scope(room_id.to_string(), async {
            self.reply_inner(room_id, msg).await
        }).await
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
