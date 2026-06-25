use std::io;
use std::path::PathBuf;

use async_fd_lock::{LockRead, LockWrite};
use rig::completion::Message;
use rig::memory::{ConversationMemory, MemoryError};
use rig::wasm_compat::WasmBoxedFuture;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub struct FileMemory {
    base_dir: PathBuf,
}

impl FileMemory {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    fn path(&self, conversation_id: &str) -> PathBuf {
        self.base_dir.join(format!("{conversation_id}.jsonl"))
    }

    pub async fn count(&self, conversation_id: &str) -> io::Result<usize> {
        let path = self.path(conversation_id);
        let file = match fs::File::open(&path).await {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        let mut guard = file.lock_read().await?;
        let mut content = String::new();
        guard.read_to_string(&mut content).await?;
        Ok(content.lines().count())
    }

    pub async fn replace_all(&self, conversation_id: &str, messages: &[Message]) -> io::Result<()> {
        let path = self.path(conversation_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .await?;

        let mut guard = file.lock_write().await?;

        guard.inner_mut().set_len(0).await?;
        guard.rewind().await?;
        for msg in messages {
            let json = serde_json::to_string(msg)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            guard.write_all(json.as_bytes()).await?;
            guard.write_all(b"\n").await?;
        }
        guard.flush().await?;
        Ok(())
    }
}

impl ConversationMemory for FileMemory {
    fn load<'a>(
        &'a self,
        conversation_id: &'a str,
    ) -> WasmBoxedFuture<'a, Result<Vec<Message>, MemoryError>> {
        Box::pin(async move {
            let path = self.path(conversation_id);

            let file = match fs::File::open(&path).await {
                Ok(f) => f,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    return Ok(Vec::new());
                }
                Err(e) => return Err(MemoryError::Backend(e.into())),
            };

            let mut guard = file
                .lock_read()
                .await
                .map_err(|e| MemoryError::Backend(e.error.into()))?;

            let mut content = String::new();
            guard
                .read_to_string(&mut content)
                .await
                .map_err(|e| MemoryError::Backend(e.into()))?;

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
            let path = self.path(conversation_id);

            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .await
                    .map_err(|e| MemoryError::Backend(e.into()))?;
            }

            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .await
                .map_err(|e| MemoryError::Backend(e.into()))?;

            let mut guard = file
                .lock_write()
                .await
                .map_err(|e| MemoryError::Backend(e.error.into()))?;

            guard
                .seek(io::SeekFrom::End(0))
                .await
                .map_err(|e| MemoryError::Backend(e.into()))?;

            for msg in messages {
                let json =
                    serde_json::to_string(&msg).map_err(|e| MemoryError::Backend(e.into()))?;
                guard
                    .write_all(json.as_bytes())
                    .await
                    .map_err(|e| MemoryError::Backend(e.into()))?;
                guard
                    .write_all(b"\n")
                    .await
                    .map_err(|e| MemoryError::Backend(e.into()))?;
            }

            guard
                .flush()
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
            let path = self.path(conversation_id);

            let _locked = match fs::File::open(&path).await {
                Ok(file) => {
                    let locked = file
                        .lock_write()
                        .await
                        .map_err(|e| MemoryError::Backend(e.error.into()))?;
                    Some(locked)
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                Err(e) => return Err(MemoryError::Backend(e.into())),
            };

            match fs::remove_file(&path).await {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(MemoryError::Backend(e.into())),
            }
        })
    }
}
