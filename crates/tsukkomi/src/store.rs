use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use thiserror::Error;
use tokio::fs;
use tokio::sync::Mutex as AsyncMutex;

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
    cache: Mutex<HashMap<String, HashMap<String, Memory>>>,
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

    async fn load_all(&self, room_id: &str) -> Result<HashMap<String, Memory>, StoreError> {
        let path = self.path(room_id);
        let content = match fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
            Err(e) => return Err(e.into()),
        };
        let map: HashMap<String, Memory> = serde_json::from_str(&content)?;
        Ok(map)
    }

    async fn save_all(
        &self,
        room_id: &str,
        memories: &HashMap<String, Memory>,
    ) -> Result<(), StoreError> {
        let path = self.path(room_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let json = serde_json::to_string_pretty(memories)?;
        fs::write(&path, &json).await?;
        Ok(())
    }

    pub async fn list(&self, room_id: &str) -> Vec<(String, Memory)> {
        if let Some(cached) = self.cache.lock().unwrap().get(room_id) {
            let mut pairs: Vec<(String, Memory)> =
                cached.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            return pairs;
        }
        let memories = self.load_all(room_id).await.unwrap_or_default();
        let mut pairs: Vec<(String, Memory)> =
            memories.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        self.cache
            .lock()
            .unwrap()
            .insert(room_id.to_string(), memories);
        pairs
    }

    pub async fn remember(
        &self,
        room_id: &str,
        key: &str,
        summary: &str,
    ) -> Result<(), StoreError> {
        let _lock = self.file_lock.lock().await;
        let mut memories = self.load_all(room_id).await?;
        memories.insert(key.into(), Memory {
            summary: summary.into(),
        });
        self.save_all(room_id, &memories).await?;
        self.cache
            .lock()
            .unwrap()
            .insert(room_id.to_string(), memories);
        Ok(())
    }

    pub async fn forget(&self, room_id: &str, key: &str) -> Result<(), StoreError> {
        let _lock = self.file_lock.lock().await;
        let mut memories = self.load_all(room_id).await?;
        memories.remove(key);
        self.save_all(room_id, &memories).await?;
        self.cache
            .lock()
            .unwrap()
            .insert(room_id.to_string(), memories);
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
        let room_id = CURRENT_ROOM.try_with(|id| id.clone()).map_err(|_| StoreError::NoRoomContext)?;
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
        let room_id = CURRENT_ROOM.try_with(|id| id.clone()).map_err(|_| StoreError::NoRoomContext)?;
        self.store.forget(&room_id, &args.key).await?;
        Ok(format!("已删除：{}", args.key))
    }
}
