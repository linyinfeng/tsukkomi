use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::time::Instant;

use anyhow::Context;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rig::OneOrMany;
use rig::agent::Agent;
use rig::client::CompletionClient;
use rig::client::ProviderClient;
use rig::completion::{Message, Prompt};
use rig::memory::{Compactor, ConversationMemory, MemoryPolicy};
use rig::message::{DocumentSourceKind, Image as RigImage, ImageMediaType, MimeType, UserContent};
use rig::providers::deepseek;
use rig::providers::xiaomimimo;
use rig::schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::cli::TsukkomiOptions;
use crate::compactor::TsukkomiCompactor;
use crate::memory::file::FileMemory;
use crate::memory::remembering::RememberingMemory;
use crate::memory::store::{CURRENT_ROOM, Forget, MemoryStore, Remember};
use crate::window::BatchedSlidingWindow;

const RETRY_PROMPT: &str =
    "Your response was not valid JSON. Reply with valid JSON matching the ResponsePayload schema.";

const IMAGE_DESC_PROMPT: &str = include_str!("../prompts/describe_image.md");

/// Raw image data with its MIME type.
#[derive(Debug)]
pub struct ImageData {
    pub data: Vec<u8>,
    pub media_type: Option<String>,
}

/// External input from bots to the library.
#[derive(Debug)]
pub struct ChatInput {
    pub user_id: String,
    pub display_name: String,
    pub text: Option<String>,
    pub images: Vec<ImageData>,
    pub sent_at: chrono::DateTime<chrono::Utc>,
    pub reply_to_user_id: Option<String>,
}

/// Internal payload sent to the LLM (JSON-encoded).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MessagePayload {
    pub user_id: String,
    pub display_name: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_descriptions: Option<String>,
    pub sent_at: chrono::DateTime<chrono::Utc>,
    pub reply_to_user_id: Option<String>,
    pub debouncing: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
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

fn default_system_prompt() -> &'static str {
    include_str!("../prompts/default.md")
}

fn summary_system_prompt(max_chars: usize) -> String {
    format!(include_str!("../prompts/summary.md"), max_chars)
}

fn format_system_prompt() -> anyhow::Result<&'static str> {
    use std::sync::OnceLock;

    static SCHEMA: OnceLock<String> = OnceLock::new();
    let schema = SCHEMA.get_or_init(|| {
        let input_schema = rig::schemars::schema_for!(MessagePayload);
        let input_json = serde_json::to_string_pretty(&input_schema)
            .expect("serialize input schema for system prompt is infallible");
        let output_schema = rig::schemars::schema_for!(ResponsePayload);
        let output_json = serde_json::to_string_pretty(&output_schema)
            .expect("serialize output schema for system prompt is infallible");

        format!(
            "# Input Format / 输入格式\n\n\
             用户消息以 JSON 格式发送，MessagePayload schema 如下：\n{input_json}\n\n\
             # Output Format / 输出格式\n\n\
             你必须以 JSON 格式回复，ResponsePayload schema 如下（只返回 JSON，不要包含其他文字）：\n{output_json}"
        )
    });
    Ok(schema.as_str())
}

type DeepSeekModel = <deepseek::Client as CompletionClient>::CompletionModel;
type MiMoModel = <xiaomimimo::AnthropicClient as CompletionClient>::CompletionModel;

/// The default `ChatManager` type using DeepSeek for conversation and MiMo for images.
pub type DefaultChatManager = ChatManager;

pub struct ChatManager {
    agent: Agent<DeepSeekModel>,
    image_agent: Agent<MiMoModel>,
    memory: Arc<FileMemory>,
    window: BatchedSlidingWindow,
    compactor: TsukkomiCompactor<DeepSeekModel>,
    max_retries: u32,
    last_reply: Mutex<HashMap<String, Instant>>,
    debounce_duration: humantime::Duration,
}

impl ChatManager {
    pub fn new(
        opts: TsukkomiOptions,
        bot_user_id: &str,
        bot_display_name: &str,
    ) -> anyhow::Result<Self> {
        let deepseek_client = Arc::new(
            deepseek::Client::from_env()
                .context("failed to create DeepSeek client — check DEEPSEEK_API_KEY env var")?,
        );
        let system_prompt = system_prompt(&opts, bot_user_id, bot_display_name)
            .context("failed to build system prompt")?;

        let max_retries = opts.max_retries;
        let debounce_duration = opts.debounce_duration;

        let memory = Arc::new(FileMemory::new(&opts.memory_directory));
        let store = Arc::new(MemoryStore::new(std::path::PathBuf::from(
            &opts.memory_directory,
        )));
        let remembering = RememberingMemory::new(Arc::clone(&memory), Arc::clone(&store));

        let window = BatchedSlidingWindow::new(opts.sliding_window, opts.batch_size);

        let main_agent = deepseek_client
            .agent(deepseek::DEEPSEEK_V4_FLASH)
            .preamble(&system_prompt)
            .memory(remembering)
            .tool(Remember {
                store: Arc::clone(&store),
            })
            .tool(Forget {
                store: Arc::clone(&store),
            })
            .build();

        let summary_prompt = summary_system_prompt(opts.summary_max_chars);
        let summary_agent = deepseek_client
            .agent(deepseek::DEEPSEEK_V4_FLASH)
            .preamble(&summary_prompt)
            .build();
        let compactor = TsukkomiCompactor::new(summary_agent, opts.summary_header);

        let mimo_client = Arc::new(
            xiaomimimo::AnthropicClient::from_env()
                .context("failed to create MiMo client — check XIAOMI_MIMO_API_KEY env var")?,
        );
        let image_agent = mimo_client
            .agent(xiaomimimo::MIMO_V2_5)
            .preamble(IMAGE_DESC_PROMPT)
            .build();

        tracing::info!("ChatManager initialized");
        Ok(Self {
            agent: main_agent,
            image_agent,
            memory,
            window,
            compactor,
            max_retries,
            last_reply: Mutex::new(HashMap::new()),
            debounce_duration,
        })
    }
}

pub fn system_prompt(
    opts: &TsukkomiOptions,
    bot_user_id: &str,
    bot_display_name: &str,
) -> anyhow::Result<String> {
    let base = if let Some(path) = &opts.system_prompt_file {
        std::fs::read_to_string(path)
            .with_context(|| format!("failed to read system prompt file {path}"))?
    } else {
        opts.system_prompt
            .clone()
            .unwrap_or_else(|| default_system_prompt().to_string())
    };
    let identity = format!(
        "\n\n# Your Identity / 你的身份\n\n\
         user_id: {bot_user_id}\n\
         display_name: {bot_display_name}\n\n\
         当用户 @你、回复你（reply_to_user_id 为你自己的 user_id）、\
         或通过名字提到你时，应当优先回应该消息。\n"
    );
    let mut prompt = base;
    prompt.push_str(&identity);
    prompt.push_str("\n\n");
    prompt.push_str(format_system_prompt()?);
    Ok(prompt)
}

impl ChatManager {
    async fn describe_images(&self, images: &[ImageData]) -> Option<String> {
        if images.is_empty() {
            return None;
        }
        let content: Vec<UserContent> = images
            .iter()
            .map(|img| {
                let media_type = img
                    .media_type
                    .as_deref()
                    .and_then(ImageMediaType::from_mime_type);
                if media_type.is_none() {
                    tracing::warn!(
                        media_type = ?img.media_type,
                        "Unsupported image MIME type, sending without media_type"
                    );
                }
                UserContent::Image(RigImage {
                    data: DocumentSourceKind::Base64(STANDARD.encode(&img.data)),
                    media_type,
                    detail: None,
                    additional_params: None,
                })
            })
            .collect();
        if content.is_empty() {
            return None;
        }
        let msg = Message::User {
            content: OneOrMany::many(content).expect("non-empty"),
        };
        match self.image_agent.prompt(msg).await {
            Ok(desc) => Some(desc),
            Err(e) => {
                tracing::warn!("Image description failed: {e}");
                None
            }
        }
    }

    pub async fn reply(&self, room_id: &str, input: ChatInput) -> anyhow::Result<Option<Response>> {
        let debouncing = {
            let last = self.last_reply.lock().await;
            last.get(room_id)
                .map(|t| t.elapsed() < *self.debounce_duration)
                .unwrap_or(false)
        };

        let image_descriptions = self.describe_images(&input.images).await;

        let msg = MessagePayload {
            user_id: input.user_id,
            display_name: input.display_name,
            body: input.text.unwrap_or_default(),
            image_descriptions,
            sent_at: input.sent_at,
            reply_to_user_id: input.reply_to_user_id,
            debouncing,
        };

        CURRENT_ROOM
            .scope(room_id.to_string(), async move {
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

        let mut payload =
            serde_json::to_string(&msg).context("failed to serialize message payload")?;
        tracing::info!(
            room_id,
            debouncing = msg.debouncing,
            ?msg,
            "Sending payload"
        );

        for attempt in 0..self.max_retries {
            let response = self
                .agent
                .prompt(&payload)
                .conversation(room_id)
                .await
                .with_context(|| format!("LLM prompt failed for room {room_id}"))?;
            match serde_json::from_str::<ResponsePayload>(&response) {
                Ok(ResponsePayload::Reply(resp)) => {
                    tracing::info!(room_id, ?resp, "Received reply");
                    self.last_reply
                        .lock()
                        .await
                        .insert(room_id.to_string(), Instant::now());
                    return Ok(Some(resp));
                }
                Ok(ResponsePayload::Skip) => {
                    return Ok(None);
                }
                Err(e) => {
                    tracing::warn!(attempt, error = %e, raw = %response, "Failed to parse AI response");
                    payload = format!("{RETRY_PROMPT}\nError message: {e}");
                }
            }
        }
        Err(anyhow::anyhow!(
            "all {} retries exhausted for room {room_id}",
            self.max_retries
        ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::TsukkomiOptions;

    fn test_opts() -> TsukkomiOptions {
        TsukkomiOptions {
            system_prompt: None,
            system_prompt_file: None,
            max_retries: 3,
            memory_directory: "memory".into(),
            sliding_window: 200,
            summary_max_chars: 2000,
            summary_header: "历史摘要".into(),
            batch_size: 100,
            debounce_duration: "30s".parse().unwrap(),
        }
    }

    #[test]
    fn system_prompt_contains_bot_identity() {
        let opts = test_opts();
        let prompt = system_prompt(&opts, "bot123", "TestBot").unwrap();
        assert!(prompt.contains("bot123"));
        assert!(prompt.contains("TestBot"));
        assert!(prompt.contains("user_id:"));
        assert!(prompt.contains("display_name:"));
    }

    #[test]
    fn system_prompt_contains_input_output_schemas() {
        let input_schema =
            serde_json::to_string_pretty(&rig::schemars::schema_for!(MessagePayload)).unwrap();
        let output_schema =
            serde_json::to_string_pretty(&rig::schemars::schema_for!(ResponsePayload)).unwrap();

        let opts = test_opts();
        let prompt = system_prompt(&opts, "bot1", "Bot").unwrap();
        assert!(prompt.contains(&input_schema));
        assert!(prompt.contains(&output_schema));
    }

    #[test]
    fn system_prompt_uses_custom_prompt_when_provided() {
        let opts = TsukkomiOptions {
            system_prompt: Some("Custom system prompt".into()),
            ..test_opts()
        };
        let prompt = system_prompt(&opts, "b", "B").unwrap();
        assert!(prompt.starts_with("Custom system prompt"));
    }

    #[test]
    fn message_payload_serializes_all_fields() {
        let payload = MessagePayload {
            user_id: "user1".into(),
            display_name: "User One".into(),
            body: "hello world".into(),
            image_descriptions: Some("a cat photo".into()),
            sent_at: chrono::DateTime::UNIX_EPOCH,
            reply_to_user_id: Some("user2".into()),
            debouncing: true,
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("user1"));
        assert!(json.contains("User One"));
        assert!(json.contains("hello world"));
        assert!(json.contains("a cat photo"));
        assert!(json.contains("user2"));
        assert!(json.contains("true"));
    }

    #[test]
    fn message_payload_omits_none_fields() {
        let payload = MessagePayload {
            user_id: "u".into(),
            display_name: "D".into(),
            body: "b".into(),
            image_descriptions: None,
            sent_at: chrono::DateTime::UNIX_EPOCH,
            reply_to_user_id: None,
            debouncing: false,
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(!json.contains("image_descriptions"));
    }

    #[test]
    fn response_payload_skip_deserializes() {
        let json = r#"{"action":"skip"}"#;
        let payload: ResponsePayload = serde_json::from_str(json).unwrap();
        assert!(matches!(payload, ResponsePayload::Skip));
    }

    #[test]
    fn response_payload_reply_deserializes() {
        let json = r#"{"action":"reply","text":"hello"}"#;
        let payload: ResponsePayload = serde_json::from_str(json).unwrap();
        match payload {
            ResponsePayload::Reply(resp) => assert_eq!(resp.text, "hello"),
            _ => panic!("expected Reply variant"),
        }
    }

    #[test]
    fn chat_input_builds_correctly() {
        let input = ChatInput {
            user_id: "uid".into(),
            display_name: "name".into(),
            text: Some("text".into()),
            images: vec![],
            sent_at: chrono::Utc::now(),
            reply_to_user_id: Some("other".into()),
        };
        assert_eq!(input.user_id, "uid");
        assert_eq!(input.display_name, "name");
        assert_eq!(input.text, Some("text".into()));
        assert!(input.images.is_empty());
        assert_eq!(input.reply_to_user_id, Some("other".into()));
    }
}
