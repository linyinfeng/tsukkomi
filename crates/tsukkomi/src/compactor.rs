use rig::client::CompletionClient;
use rig::completion::message::{AssistantContent, UserContent};
use rig::completion::{Message, Prompt};
use rig::memory::{Compactor, MemoryError};
use rig::providers::deepseek;
use rig::schemars::JsonSchema;
use rig::wasm_compat::WasmBoxedFuture;
use serde::{Deserialize, Serialize};

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

#[derive(Deserialize, JsonSchema)]
struct SummaryOutput {
    summary: String,
}

fn format_summary_input(evicted: &[Message], carry_over: Option<&str>) -> SummaryInput {
    let conversation: Vec<SummaryMessage> = evicted
        .iter()
        .filter_map(|msg| {
            let (role, name, content) = match msg {
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
                    ("user".into(), None, text.join(" "))
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
                    ("assistant".into(), None, text.join(" "))
                }
                Message::System { content } => {
                    if content.is_empty() {
                        return None;
                    }
                    ("system".into(), None, content.clone())
                }
            };
            Some(SummaryMessage {
                role,
                name,
                content,
            })
        })
        .collect();

    SummaryInput {
        previous_summary: carry_over.unwrap_or("").to_string(),
        conversation,
    }
}

pub struct TsukkomiCompactor {
    client: deepseek::Client,
    model: String,
    header: String,
    max_chars: usize,
}

impl TsukkomiCompactor {
    pub fn new(client: deepseek::Client, model: String, header: String, max_chars: usize) -> Self {
        Self {
            client,
            model,
            header,
            max_chars,
        }
    }
}

#[derive(Clone)]
pub struct SummaryArtifact(pub String);

impl From<SummaryArtifact> for Message {
    fn from(value: SummaryArtifact) -> Self {
        Message::System { content: value.0 }
    }
}

fn summary_system_prompt(max_chars: usize) -> String {
    let output_schema = rig::schemars::schema_for!(SummaryOutput);
    let schema_json = serde_json::to_string_pretty(&output_schema).unwrap();
    format!(
        "你是一个群聊对话摘要机器人。\
         将输入的对话压缩为简洁的中文摘要，保留关键话题和重要上下文。\
         摘要不超过 {} 字。\
         必须以 JSON 格式回复，schema 如下：\n{schema_json}",
        max_chars
    )
}

impl Compactor for TsukkomiCompactor {
    type Artifact = SummaryArtifact;

    fn compact<'a>(
        &'a self,
        _conversation_id: &'a str,
        evicted: &'a [Message],
        carry_over: Option<&'a Self::Artifact>,
    ) -> WasmBoxedFuture<'a, Result<Self::Artifact, MemoryError>> {
        Box::pin(async move {
            let input = format_summary_input(evicted, carry_over.map(|a| a.0.as_str()));
            let payload = serde_json::to_string(&input)
                .map_err(|e| MemoryError::Backend(e.into()))?;

            let agent = self
                .client
                .agent(&self.model)
                .preamble(&summary_system_prompt(self.max_chars))
                .additional_params(serde_json::json!({"response_format": {"type": "json_object"}}))
                .build();

            let response = agent
                .prompt(&payload)
                .await
                .map_err(|e| MemoryError::Backend(e.into()))?;

            let output: SummaryOutput = serde_json::from_str(&response)
                .map_err(|e| MemoryError::Backend(e.into()))?;

            Ok(SummaryArtifact(format!("{}：{}", self.header, output.summary)))
        })
    }
}
