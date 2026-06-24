use rig::agent::Agent;
use rig::client::CompletionClient;
use rig::client::ProviderClient;
use rig::completion::Prompt;
use rig::memory::InMemoryConversationMemory;
use rig::providers::deepseek;
use schemars::JsonSchema;
use serde::Serialize;

use crate::cli::TsukkomiOptions;

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

pub fn system_prompt() -> &'static str {
    r"你是一个群聊吐槽 bot。
用简短幽默的中文（50字以内）回应群友消息，语气友善调侃，像朋友间的互怼。
不要用敬语，不要长篇大论。
"
}

fn format_system_prompt() -> String {
    let schema = schemars::schema_for!(MessagePayload);
    let schema_json = serde_json::to_string_pretty(&schema).unwrap();

    format!("用户消息以 JSON 格式发送，schema 如下：{schema_json}")
}

pub struct ChatManager {
    agent: Agent<deepseek::CompletionModel>,
}

impl ChatManager {
    pub fn new(opts: TsukkomiOptions) -> anyhow::Result<Self> {
        let client = deepseek::Client::from_env()?;
        let system_prompt = Self::system_prompt(&opts);
        let agent = client
            .agent(deepseek::DEEPSEEK_V4_FLASH)
            .preamble(&system_prompt)
            .memory(InMemoryConversationMemory::default())
            .build();
        tracing::info!(system_prompt, "ChatManager initialized");
        Ok(Self { agent })
    }

    pub fn system_prompt(opts: &TsukkomiOptions) -> String {
        let mut system_prompt = opts.system_prompt.clone();
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&format_system_prompt());
        system_prompt
    }

    pub async fn reply(&self, room_id: &str, msg: MessagePayload) -> anyhow::Result<String> {
        let payload = serde_json::to_string(&msg)?;
        tracing::info!(room_id, payload, "Sending payload");
        self.agent
            .prompt(&payload)
            .conversation(room_id)
            .await
            .map_err(Into::into)
    }
}
