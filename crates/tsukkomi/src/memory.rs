use std::path::PathBuf;

use rig::completion::Message;
use rig::memory::{ConversationMemory, MemoryError};
use rig::wasm_compat::WasmBoxedFuture;
use tokio::fs;
use tokio::io::AsyncWriteExt;

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
}

impl ConversationMemory for FileMemory {
    fn load<'a>(
        &'a self,
        conversation_id: &'a str,
    ) -> WasmBoxedFuture<'a, Result<Vec<Message>, MemoryError>> {
        Box::pin(async move {
            let path = self.path(conversation_id);

            let content = match fs::read_to_string(&path).await {
                Ok(c) => c,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
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
            let path = self.path(conversation_id);

            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .await
                    .map_err(|e| MemoryError::Backend(e.into()))?;
            }

            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .await
                .map_err(|e| MemoryError::Backend(e.into()))?;

            for msg in messages {
                let json =
                    serde_json::to_string(&msg).map_err(|e| MemoryError::Backend(e.into()))?;
                file.write_all(json.as_bytes())
                    .await
                    .map_err(|e| MemoryError::Backend(e.into()))?;
                file.write_all(b"\n")
                    .await
                    .map_err(|e| MemoryError::Backend(e.into()))?;
            }

            file.flush()
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
            match fs::remove_file(&path).await {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(MemoryError::Backend(e.into())),
            }
        })
    }
}
