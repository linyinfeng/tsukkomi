use rig::agent::Agent;
use rig::client::CompletionClient;
use rig::client::ProviderClient;
use rig::completion::Prompt;
use rig::memory::InMemoryConversationMemory;
use rig::providers::deepseek;
use rig::schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::cli::TsukkomiOptions;

const RETRY_PROMPT: &str = "Invalid JSON format. Reply with valid JSON matching the schema.";

#[derive(Serialize, JsonSchema)]
#[serde(tag = "type", content = "data")]
pub enum MessageBody {
    Text(String),
}

#[derive(Serialize, JsonSchema)]
pub struct MessagePayload {
    pub user_id: String,
    pub display_name: String,
    pub body: MessageBody,
}

#[derive(Deserialize, JsonSchema)]
pub struct ReplyPayload {
    pub should_reply: bool,
    pub reply: String,
}

pub fn system_prompt() -> &'static str {
    r"你是一个群聊吐槽 bot。
用简短幽默的中文（50字以内）回应群友消息，语气友善调侃，像朋友间的互怼。
不要用敬语，不要长篇大论。

判断消息是否值得回复（有趣、有槽点、需要你参与），否则 should_reply 设为 false。
"
}

fn format_system_prompt() -> String {
    let input_schema = rig::schemars::schema_for!(MessagePayload);
    let input_json = serde_json::to_string_pretty(&input_schema).unwrap();
    let output_schema = rig::schemars::schema_for!(ReplyPayload);
    let output_json = serde_json::to_string_pretty(&output_schema).unwrap();

    format!(
        "用户消息以 JSON 格式发送，schema 如下：\n{input_json}\n\n\
         你必须以 JSON 格式回复，schema 如下（只返回 JSON，不要包含其他文字）：\n{output_json}"
    )
}

pub struct ChatManager {
    agent: Agent<deepseek::CompletionModel>,
    max_retries: u32,
}

impl ChatManager {
    pub fn new(opts: TsukkomiOptions) -> anyhow::Result<Self> {
        let client = deepseek::Client::from_env()?;
        let system_prompt = Self::system_prompt(&opts);
        let max_retries = opts.max_retries;
        let agent = client
            .agent(deepseek::DEEPSEEK_V4_FLASH)
            .preamble(&system_prompt)
            .memory(InMemoryConversationMemory::default())
            // DeepSeek doesn't support output_schema (rig warns and ignores it)
            // .output_schema::<ReplyPayload>()
            .additional_params(serde_json::json!({"response_format": {"type": "json_object"}}))
            .build();
        tracing::info!(system_prompt, max_retries, "ChatManager initialized");
        Ok(Self { agent, max_retries })
    }

    pub fn system_prompt(opts: &TsukkomiOptions) -> String {
        let mut system_prompt = opts.system_prompt.clone();
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&format_system_prompt());
        system_prompt
    }

    pub async fn reply(&self, room_id: &str, msg: MessagePayload) -> anyhow::Result<Option<String>> {
        let payload = serde_json::to_string(&msg)?;
        tracing::info!(room_id, payload, "Sending payload");

        let mut response = self.agent.prompt(&payload).conversation(room_id).await?;

        for attempt in 0..self.max_retries {
            match serde_json::from_str::<ReplyPayload>(&response) {
                Ok(reply) => {
                    return if reply.should_reply {
                        Ok(Some(reply.reply))
                    } else {
                        Ok(None)
                    };
                }
                Err(e) => {
                    tracing::warn!(attempt, error = %e, raw = %response, "Failed to parse AI response");
                    response = self.agent.prompt(RETRY_PROMPT).conversation(room_id).await?;
                }
            }
        }

        match serde_json::from_str::<ReplyPayload>(&response) {
            Ok(reply) => {
                if reply.should_reply {
                    Ok(Some(reply.reply))
                } else {
                    Ok(None)
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, raw = %response, "All retries exhausted");
                Ok(Some(response))
            }
        }
    }
}
