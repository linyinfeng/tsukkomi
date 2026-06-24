use std::collections::HashMap;
use std::sync::Mutex;

use rig::agent::Agent;
use rig::client::CompletionClient;
use rig::client::ProviderClient;
use rig::completion::Prompt;
use rig::memory::InMemoryConversationMemory;
use rig::providers::deepseek;

pub struct MessageInfo {
    pub user_id: String,
    pub display_name: String,
    pub text: String,
}

pub struct ChatManager {
    client: deepseek::Client,
    agents: Mutex<HashMap<String, Agent<deepseek::CompletionModel>>>,
}

impl ChatManager {
    pub fn new() -> anyhow::Result<Self> {
        let client = deepseek::Client::from_env()?;
        Ok(Self {
            client,
            agents: Mutex::new(HashMap::new()),
        })
    }

    pub async fn reply(&self, room_id: &str, msg: MessageInfo) -> anyhow::Result<String> {
        let agent = {
            let mut agents = self.agents.lock().unwrap();
            agents
                .entry(room_id.to_string())
                .or_insert_with(|| self.create_agent())
                .clone()
        };

        tracing::info!(
            room_id,
            user_id = msg.user_id,
            display_name = msg.display_name,
            text = msg.text,
            "Incoming message"
        );

        let formatted = format!("<{}> {}: {}", msg.user_id, msg.display_name, msg.text);
        agent.prompt(&formatted).await.map_err(Into::into)
    }

    fn create_agent(&self) -> Agent<deepseek::CompletionModel> {
        self.client
            .agent(deepseek::DEEPSEEK_V4_FLASH)
            .preamble("你是一个群聊吐槽 bot。用简短幽默的中文（50字以内）回应群友消息，语气友善调侃，像朋友间的互怼。不要用敬语，不要长篇大论。")
            .memory(InMemoryConversationMemory::default())
            .build()
    }
}
