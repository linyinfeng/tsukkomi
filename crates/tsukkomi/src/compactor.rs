use rig::client::CompletionClient;
use rig::completion::message::{AssistantContent, UserContent};
use rig::completion::{Message, Prompt};
use rig::memory::{Compactor, MemoryError};
use rig::providers::deepseek;
use rig::wasm_compat::WasmBoxedFuture;

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

impl Compactor for TsukkomiCompactor {
    type Artifact = SummaryArtifact;

    fn compact<'a>(
        &'a self,
        _conversation_id: &'a str,
        evicted: &'a [Message],
        carry_over: Option<&'a Self::Artifact>,
    ) -> WasmBoxedFuture<'a, Result<Self::Artifact, MemoryError>> {
        Box::pin(async move {
            let mut prompt =
                String::from("将以下群聊对话压缩为一段简洁的中文摘要，保留关键话题和重要上下文。");

            if let Some(co) = carry_over {
                prompt.push_str("\n\n此前摘要：\n");
                prompt.push_str(&co.0);
            }

            prompt.push_str("\n\n新对话：\n");
            for msg in evicted {
                match msg {
                    Message::User { content } => {
                        for part in content.iter() {
                            if let UserContent::Text(t) = part {
                                prompt.push_str("用户: ");
                                prompt.push_str(&t.text);
                                prompt.push('\n');
                            }
                        }
                    }
                    Message::Assistant { content, .. } => {
                        for part in content.iter() {
                            if let AssistantContent::Text(t) = part {
                                prompt.push_str("助手: ");
                                prompt.push_str(&t.text);
                                prompt.push('\n');
                            }
                        }
                    }
                    Message::System { content } => {
                        prompt.push_str("系统: ");
                        prompt.push_str(content);
                        prompt.push('\n');
                    }
                }
            }

            prompt.push_str(&format!(
                "\n生成更新后的摘要，不超过 {} 字。",
                self.max_chars
            ));

            let agent = self
                .client
                .agent(&self.model)
                .preamble("你是一个群聊对话摘要机器人。")
                .build();

            let summary = agent
                .prompt(&prompt)
                .await
                .map_err(|e| MemoryError::Backend(e.into()))?;

            Ok(SummaryArtifact(format!("{}：{}", self.header, summary)))
        })
    }
}
