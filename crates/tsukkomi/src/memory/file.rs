use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use rig::completion::Message;
use rig::memory::{ConversationMemory, MemoryError};
use rig::wasm_compat::WasmBoxedFuture;
use tokio::fs;
use tokio::sync::{Mutex, RwLock};

use super::utils::atomic_write;

pub struct FileMemory {
    base_dir: PathBuf,
    locks: Mutex<HashMap<String, Arc<RwLock<()>>>>,
}

impl FileMemory {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
            locks: Mutex::new(HashMap::new()),
        }
    }

    async fn get_lock(&self, conversation_id: &str) -> Arc<RwLock<()>> {
        let mut locks = self.locks.lock().await;
        locks
            .entry(conversation_id.to_string())
            .or_insert_with(|| Arc::new(RwLock::new(())))
            .clone()
    }

    fn path(&self, conversation_id: &str) -> PathBuf {
        self.base_dir.join(format!("{conversation_id}.jsonl"))
    }

    pub async fn count(&self, conversation_id: &str) -> io::Result<usize> {
        let lock = self.get_lock(conversation_id).await;
        let _guard = lock.read().await;
        let path = self.path(conversation_id);
        let content = match fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        Ok(content.lines().count())
    }

    pub async fn replace_all(&self, conversation_id: &str, messages: &[Message]) -> io::Result<()> {
        let lock = self.get_lock(conversation_id).await;
        let _guard = lock.write().await;
        let path = self.path(conversation_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let mut buf = Vec::new();
        for msg in messages {
            let json = serde_json::to_string(msg)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            buf.extend_from_slice(json.as_bytes());
            buf.extend_from_slice(b"\n");
        }

        atomic_write(&path, &buf).await
    }
}

impl ConversationMemory for FileMemory {
    fn load<'a>(
        &'a self,
        conversation_id: &'a str,
    ) -> WasmBoxedFuture<'a, Result<Vec<Message>, MemoryError>> {
        Box::pin(async move {
            let lock = self.get_lock(conversation_id).await;
            let _guard = lock.read().await;
            let path = self.path(conversation_id);

            let content = match fs::read_to_string(&path).await {
                Ok(c) => c,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    return Ok(Vec::new());
                }
                Err(e) => return Err(MemoryError::Backend(e.into())),
            };

            let mut messages = Vec::new();
            for line in content.lines() {
                let msg: Message =
                    serde_json::from_str(line).map_err(|e| MemoryError::Backend(e.into()))?;
                messages.push(msg);
            }
            Ok(messages)
        })
    }

    fn append<'a>(
        &'a self,
        conversation_id: &'a str,
        messages: Vec<Message>,
    ) -> WasmBoxedFuture<'a, Result<(), MemoryError>> {
        Box::pin(async move {
            let lock = self.get_lock(conversation_id).await;
            let _guard = lock.write().await;
            let path = self.path(conversation_id);

            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .await
                    .map_err(|e| MemoryError::Backend(e.into()))?;
            }

            let mut buf = match fs::read_to_string(&path).await {
                Ok(content) => content.into_bytes(),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
                Err(e) => return Err(MemoryError::Backend(e.into())),
            };

            for msg in messages {
                let json =
                    serde_json::to_string(&msg).map_err(|e| MemoryError::Backend(e.into()))?;
                buf.extend_from_slice(json.as_bytes());
                buf.extend_from_slice(b"\n");
            }

            atomic_write(&path, &buf)
                .await
                .map_err(|e| MemoryError::Backend(e.into()))?;

            Ok(())
        })
    }

    fn clear<'a>(
        &'a self,
        conversation_id: &'a str,
    ) -> WasmBoxedFuture<'a, Result<(), MemoryError>> {
        Box::pin(async move {
            let lock = self.get_lock(conversation_id).await;
            let _guard = lock.write().await;
            let path = self.path(conversation_id);

            match fs::remove_file(&path).await {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(MemoryError::Backend(e.into())),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::completion::message::UserContent;

    fn test_memory() -> (FileMemory, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mem = FileMemory::new(dir.path().to_path_buf());
        (mem, dir)
    }

    fn user_msg(text: &str) -> Message {
        Message::user(text)
    }

    #[tokio::test]
    async fn load_missing_returns_empty() {
        let (mem, _dir) = test_memory();
        let msgs = mem.load("room_missing").await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn append_and_load_roundtrip() {
        let (mem, _dir) = test_memory();
        let msgs = vec![user_msg("hello"), user_msg("world")];
        mem.append("room_a", msgs.clone()).await.unwrap();
        let loaded = mem.load("room_a").await.unwrap();
        assert_eq!(loaded.len(), 2);
    }

    #[tokio::test]
    async fn replace_all_overwrites() {
        let (mem, _dir) = test_memory();
        mem.append("room_b", vec![user_msg("old")]).await.unwrap();
        mem.replace_all("room_b", &[user_msg("new")]).await.unwrap();
        let loaded = mem.load("room_b").await.unwrap();
        assert_eq!(loaded.len(), 1);
    }

    #[tokio::test]
    async fn count_returns_line_count() {
        let (mem, _dir) = test_memory();
        mem.append("room_c", vec![user_msg("a"), user_msg("b")])
            .await
            .unwrap();
        assert_eq!(mem.count("room_c").await.unwrap(), 2);
    }

    #[tokio::test]
    async fn count_missing_returns_zero() {
        let (mem, _dir) = test_memory();
        assert_eq!(mem.count("room_missing").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn clear_removes_file() {
        let (mem, _dir) = test_memory();
        mem.append("room_d", vec![user_msg("x")]).await.unwrap();
        mem.clear("room_d").await.unwrap();
        let loaded = mem.load("room_d").await.unwrap();
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn clear_missing_is_ok() {
        let (mem, _dir) = test_memory();
        mem.clear("room_missing").await.unwrap();
    }

    #[tokio::test]
    async fn multiple_rooms_are_isolated() {
        let (mem, _dir) = test_memory();
        mem.append("room_x", vec![user_msg("x")]).await.unwrap();
        mem.append("room_y", vec![user_msg("y")]).await.unwrap();
        let x = mem.load("room_x").await.unwrap();
        let y = mem.load("room_y").await.unwrap();
        assert_eq!(x.len(), 1);
        assert_eq!(y.len(), 1);
    }

    #[tokio::test]
    async fn concurrent_appends_dont_lose_messages() {
        let (mem, _dir) = test_memory();
        let mem = std::sync::Arc::new(mem);
        let mut handles = Vec::new();
        for i in 0..10 {
            let m = mem.clone();
            handles.push(tokio::spawn(async move {
                m.append("room_concurrent", vec![user_msg(&i.to_string())])
                    .await
                    .unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        let loaded = mem.load("room_concurrent").await.unwrap();
        assert_eq!(loaded.len(), 10);
    }
}
