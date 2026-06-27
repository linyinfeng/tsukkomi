use std::path::Path;
use tokio::fs;
use tokio::io::AsyncWriteExt;

/// Write `data` to `path` atomically.
///
/// Writes to a temporary file in the same directory, fsyncs it,
/// then renames it over `path`.  This ensures that readers always
/// see either the old complete file or the new complete file,
/// never a partially-written one.
pub async fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp)
        .await?;
    file.write_all(data).await?;
    file.flush().await?;
    file.sync_all().await?;
    drop(file);
    fs::rename(&tmp, path).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn atomic_write_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        atomic_write(&path, b"hello").await.unwrap();
        let content = fs::read_to_string(&path).await.unwrap();
        assert_eq!(content, "hello");
    }

    #[tokio::test]
    async fn atomic_write_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        fs::write(&path, "old").await.unwrap();
        atomic_write(&path, b"new").await.unwrap();
        let content = fs::read_to_string(&path).await.unwrap();
        assert_eq!(content, "new");
    }

    #[tokio::test]
    async fn atomic_write_leaves_no_tmp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        atomic_write(&path, b"hello").await.unwrap();
        let tmp = path.with_extension("tmp");
        assert!(!tmp.exists(), "temp file should be removed after rename");
    }
}
