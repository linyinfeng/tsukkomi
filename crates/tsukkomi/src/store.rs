use std::collections::HashMap;
use std::path::PathBuf;

use fs4::tokio::AsyncFileExt;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use thiserror::Error;
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

tokio::task_local! {
    pub static CURRENT_ROOM: String;
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("No room context available")]
    NoRoomContext,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Memory {
    pub summary: String,
}

pub struct MemoryStore {
    base_dir: PathBuf,
}

impl MemoryStore {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    fn path(&self, room_id: &str) -> PathBuf {
        self.base_dir.join(format!("{room_id}_memories.json"))
    }

    pub async fn list(&self, room_id: &str) -> Result<HashMap<String, Memory>, StoreError> {
        let mut file = match File::open(&self.path(room_id)).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
            Err(e) => return Err(e.into()),
        };
        let _ = file.lock_shared()?;
        let mut content = String::new();
        file.read_to_string(&mut content).await?;
        Ok(serde_json::from_str(&content)?)
    }

    pub async fn remember(
        &self,
        room_id: &str,
        key: &str,
        summary: &str,
    ) -> Result<(), StoreError> {
        self.modify(room_id, |memories| {
            memories.insert(
                key.into(),
                Memory {
                    summary: summary.into(),
                },
            );
        })
        .await
    }

    pub async fn forget(&self, room_id: &str, key: &str) -> Result<(), StoreError> {
        self.modify(room_id, |memories| {
            memories.remove(key);
        })
        .await
    }

    async fn modify(
        &self,
        room_id: &str,
        f: impl FnOnce(&mut HashMap<String, Memory>),
    ) -> Result<(), StoreError> {
        let path = self.path(room_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }

        // Open with read+write+create; do NOT truncate so we can read first.
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .await?;

        // Acquire exclusive lock on the data file itself.
        // This is a brief blocking flock syscall — microseconds.
        file.lock()?;

        // Read existing content through the same locked handle.
        let mut content = String::new();
        file.read_to_string(&mut content).await?;

        let mut memories: HashMap<String, Memory> = if content.is_empty() {
            HashMap::new()
        } else {
            serde_json::from_str(&content).unwrap_or_default()
        };

        f(&mut memories);

        // Truncate and write back through the same locked handle.
        let json = serde_json::to_string_pretty(&memories)?;
        file.set_len(0).await?;
        file.rewind().await?;
        file.write_all(json.as_bytes()).await?;
        file.flush().await?;
        // File drops here → lock released
        Ok(())
    }
}

#[derive(serde::Deserialize)]
pub struct RememberArgs {
    pub key: String,
    pub summary: String,
}

pub struct Remember {
    pub store: std::sync::Arc<MemoryStore>,
}

impl Tool for Remember {
    const NAME: &'static str = "remember";

    type Error = StoreError;
    type Args = RememberArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "remember".into(),
            description:
                "保存一条长期记忆，包含关键词 key 和描述 summary。以后对话可以引用这些记忆。".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": {
                        "type": "string",
                        "description": "关键词，用于以后回忆此记忆"
                    },
                    "summary": {
                        "type": "string",
                        "description": "记忆内容简述"
                    }
                },
                "required": ["key", "summary"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let room_id = CURRENT_ROOM
            .try_with(|id| id.clone())
            .map_err(|_| StoreError::NoRoomContext)?;
        self.store
            .remember(&room_id, &args.key, &args.summary)
            .await?;
        Ok(format!("已记住：{}", args.key))
    }
}

#[derive(serde::Deserialize)]
pub struct ForgetArgs {
    pub key: String,
}

pub struct Forget {
    pub store: std::sync::Arc<MemoryStore>,
}

impl Tool for Forget {
    const NAME: &'static str = "forget";

    type Error = StoreError;
    type Args = ForgetArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "forget".into(),
            description: "删除一条长期记忆，通过 key 指定。".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": {
                        "type": "string",
                        "description": "要删除的记忆关键词"
                    }
                },
                "required": ["key"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let room_id = CURRENT_ROOM
            .try_with(|id| id.clone())
            .map_err(|_| StoreError::NoRoomContext)?;
        self.store.forget(&room_id, &args.key).await?;
        Ok(format!("已删除：{}", args.key))
    }
}
