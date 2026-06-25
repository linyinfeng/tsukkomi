use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use tokio::fs;
use tokio::sync::Mutex as AsyncMutex;

tokio::task_local! {
    pub static CURRENT_ROOM: String;
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Memory {
    pub key: String,
    pub summary: String,
}

pub struct MemoryStore {
    base_dir: PathBuf,
    cache: Mutex<HashMap<String, Vec<Memory>>>,
    file_lock: AsyncMutex<()>,
}

impl MemoryStore {
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            cache: Mutex::new(HashMap::new()),
            file_lock: AsyncMutex::new(()),
        }
    }

    fn path(&self, room_id: &str) -> PathBuf {
        self.base_dir.join(format!("{room_id}_memories.json"))
    }

    async fn load_all(&self, room_id: &str) -> std::io::Result<Vec<Memory>> {
        let path = self.path(room_id);
        let content = match fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    async fn save_all(&self, room_id: &str, memories: &[Memory]) -> std::io::Result<()> {
        let path = self.path(room_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let json = serde_json::to_string_pretty(memories)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        fs::write(&path, &json).await
    }

    pub async fn list(&self, room_id: &str) -> Vec<Memory> {
        if let Some(cached) = self.cache.lock().unwrap().get(room_id) {
            return cached.clone();
        }
        let memories = self.load_all(room_id).await.unwrap_or_default();
        self.cache
            .lock()
            .unwrap()
            .insert(room_id.to_string(), memories.clone());
        memories
    }

    pub async fn remember(
        &self,
        room_id: &str,
        key: &str,
        summary: &str,
    ) -> std::io::Result<()> {
        let _lock = self.file_lock.lock().await;
        let mut memories = self.load_all(room_id).await?;
        if let Some(m) = memories.iter_mut().find(|m| m.key == key) {
            m.summary = summary.to_string();
        } else {
            memories.push(Memory {
                key: key.into(),
                summary: summary.into(),
            });
        }
        self.save_all(room_id, &memories).await?;
        self.cache
            .lock()
            .unwrap()
            .insert(room_id.to_string(), memories);
        Ok(())
    }

    pub async fn forget(&self, room_id: &str, key: &str) -> std::io::Result<()> {
        let _lock = self.file_lock.lock().await;
        let memories = self.load_all(room_id).await?;
        let filtered: Vec<Memory> = memories.into_iter().filter(|m| m.key != key).collect();
        self.save_all(room_id, &filtered).await?;
        self.cache
            .lock()
            .unwrap()
            .insert(room_id.to_string(), filtered);
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

    type Error = std::io::Error;
    type Args = RememberArgs;
    type Output = String;

    async fn definition(
        &self,
        _prompt: String,
    ) -> ToolDefinition {
        ToolDefinition {
            name: "remember".into(),
            description: "保存一条长期记忆，包含关键词 key 和描述 summary。以后对话可以引用这些记忆。".into(),
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
        let room_id = CURRENT_ROOM.try_with(|id| id.clone()).unwrap_or_default();
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

    type Error = std::io::Error;
    type Args = ForgetArgs;
    type Output = String;

    async fn definition(
        &self,
        _prompt: String,
    ) -> ToolDefinition {
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
        let room_id = CURRENT_ROOM.try_with(|id| id.clone()).unwrap_or_default();
        self.store.forget(&room_id, &args.key).await?;
        Ok(format!("已删除：{}", args.key))
    }
}
