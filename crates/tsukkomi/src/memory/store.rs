use std::collections::HashMap;
use std::io;
use std::path::PathBuf;

use async_fd_lock::{LockRead, LockWrite};
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
    Io(#[from] io::Error),
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
        let file = match File::open(&self.path(room_id)).await {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(HashMap::new()),
            Err(e) => return Err(e.into()),
        };
        let mut guard = file.lock_read().await.map_err(|e| e.error)?;
        let mut content = String::new();
        guard.read_to_string(&mut content).await?;
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
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .await?;

        // Acquire exclusive lock (async via spawn_blocking internally).
        let mut guard = file.lock_write().await.map_err(|e| e.error)?;

        // Read existing content through the same locked handle.
        let mut content = String::new();
        guard.read_to_string(&mut content).await?;

        let mut memories: HashMap<String, Memory> = if content.is_empty() {
            HashMap::new()
        } else {
            serde_json::from_str(&content)?
        };

        f(&mut memories);

        // Truncate and write back through the same locked handle.
        let json = serde_json::to_string_pretty(&memories)?;
        guard.inner_mut().set_len(0).await?;
        guard.rewind().await?;
        guard.write_all(json.as_bytes()).await?;
        guard.flush().await?;
        // Guard drops here → lock released
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> (MemoryStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path().to_path_buf());
        (store, dir)
    }

    #[tokio::test]
    async fn list_empty_returns_empty() {
        let (store, _dir) = test_store();
        let mems = store.list("room_empty").await.unwrap();
        assert!(mems.is_empty());
    }

    #[tokio::test]
    async fn remember_and_list_roundtrip() {
        let (store, _dir) = test_store();
        store
            .remember("room_a", "mood", "feeling happy")
            .await
            .unwrap();
        let mems = store.list("room_a").await.unwrap();
        assert_eq!(mems.len(), 1);
        assert_eq!(mems["mood"].summary, "feeling happy");
    }

    #[tokio::test]
    async fn remember_multiple_keys() {
        let (store, _dir) = test_store();
        store.remember("room_b", "topic", "rust").await.unwrap();
        store
            .remember("room_b", "mood", "tired")
            .await
            .unwrap();
        let mems = store.list("room_b").await.unwrap();
        assert_eq!(mems.len(), 2);
    }

    #[tokio::test]
    async fn forget_removes_key() {
        let (store, _dir) = test_store();
        store.remember("room_c", "key1", "val1").await.unwrap();
        store.remember("room_c", "key2", "val2").await.unwrap();
        store.forget("room_c", "key1").await.unwrap();
        let mems = store.list("room_c").await.unwrap();
        assert_eq!(mems.len(), 1);
        assert!(mems.contains_key("key2"));
    }

    #[tokio::test]
    async fn remember_updates_existing_key() {
        let (store, _dir) = test_store();
        store
            .remember("room_d", "note", "old")
            .await
            .unwrap();
        store
            .remember("room_d", "note", "new")
            .await
            .unwrap();
        let mems = store.list("room_d").await.unwrap();
        assert_eq!(mems.len(), 1);
        assert_eq!(mems["note"].summary, "new");
    }

    #[tokio::test]
    async fn rooms_are_isolated() {
        let (store, _dir) = test_store();
        store
            .remember("room_x", "secret", "sauce")
            .await
            .unwrap();
        let mems = store.list("room_y").await.unwrap();
        assert!(mems.is_empty());
    }

}
