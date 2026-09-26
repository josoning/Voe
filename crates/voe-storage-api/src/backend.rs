use std::path::{Path, PathBuf};
use std::time::SystemTime;

use voe_types::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageEntryType {
    File,
    Directory,
    Symlink,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct StorageMetadata {
    pub entry_type: StorageEntryType,
    pub size: u64,
    pub created: Option<SystemTime>,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub is_hidden: bool,
}

impl StorageMetadata {
    pub fn is_file(&self) -> bool {
        self.entry_type == StorageEntryType::File
    }

    pub fn is_dir(&self) -> bool {
        self.entry_type == StorageEntryType::Directory
    }
}

pub trait StorageBackend: Send + Sync {
    fn exists(&self, path: &Path) -> Result<bool>;

    fn is_file(&self, path: &Path) -> Result<bool>;

    fn is_dir(&self, path: &Path) -> Result<bool>;

    fn metadata(&self, path: &Path) -> Result<StorageMetadata>;

    fn create_dir(&self, path: &Path) -> Result<()>;

    fn create_dir_all(&self, path: &Path) -> Result<()>;

    fn remove_dir(&self, path: &Path) -> Result<()>;

    fn remove_dir_all(&self, path: &Path) -> Result<()>;

    fn list_dir(&self, path: &Path) -> Result<Vec<PathBuf>>;

    fn create_file(&self, path: &Path) -> Result<()>;

    fn read_file(&self, path: &Path) -> Result<Vec<u8>>;

    fn read_file_to_string(&self, path: &Path) -> Result<String> {
        let bytes = self.read_file(path)?;
        String::from_utf8(bytes).map_err(|e| voe_types::error::VoeError::Storage(e.to_string()))
    }

    fn write_file(&self, path: &Path, data: &[u8]) -> Result<()>;

    fn append_to_file(&self, path: &Path, data: &[u8]) -> Result<()>;

    fn delete_file(&self, path: &Path) -> Result<()>;

    fn rename_file(&self, from: &Path, to: &Path) -> Result<()>;

    fn move_file(&self, from: &Path, to: &Path) -> Result<()>;

    fn copy_file(&self, from: &Path, to: &Path) -> Result<()>;

    fn copy_dir_all(&self, from: &Path, to: &Path) -> Result<()> {
        self.create_dir_all(to)?;
        let entries = self.list_dir(from)?;
        for entry in entries {
            let dest = to.join(entry.file_name().ok_or_else(|| {
                voe_types::error::VoeError::Storage("Invalid entry path".to_string())
            })?);
            let meta = self.metadata(&entry)?;
            if meta.is_dir() {
                self.copy_dir_all(&entry, &dest)?;
            } else {
                self.copy_file(&entry, &dest)?;
            }
        }
        Ok(())
    }

    fn ensure_parent_dir(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !self.exists(parent)? {
                self.create_dir_all(parent)?;
            }
        }
        Ok(())
    }
}
