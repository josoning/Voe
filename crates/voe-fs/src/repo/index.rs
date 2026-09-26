use std::path::PathBuf;
use std::sync::Arc;

use voe_repo_api::model::commit::{IndexEntry, IndexState, IndexStore};
use voe_storage_api::StorageBackend;
use voe_types::error::{Result, VoeError};
use voe_types::object::ObjectId;

use crate::backend::LocalFileBackend;

pub struct FsIndexStore {
    index_path: PathBuf,
    backend: Arc<dyn StorageBackend>,
}

impl FsIndexStore {
    pub fn new(index_path: PathBuf) -> Self {
        Self::with_backend(index_path, Arc::new(LocalFileBackend::new()))
    }

    pub fn with_backend(index_path: PathBuf, backend: Arc<dyn StorageBackend>) -> Self {
        Self {
            index_path,
            backend,
        }
    }

    pub fn ensure_file(&self) -> Result<()> {
        if !self.backend.exists(&self.index_path)? {
            self.backend.ensure_parent_dir(&self.index_path)?;
            let empty = IndexState::default();
            let json = serde_json::to_string_pretty(&empty).map_err(|e| {
                VoeError::Storage(format!("Failed to serialize empty index: {}", e))
            })?;
            self.backend.write_file(&self.index_path, json.as_bytes())?;
        }
        Ok(())
    }
}

impl IndexStore for FsIndexStore {
    fn load(&self) -> Result<IndexState> {
        self.ensure_file()?;
        let content = self.backend.read_file_to_string(&self.index_path)?;
        if content.trim().is_empty() {
            return Ok(IndexState::default());
        }
        serde_json::from_str(&content).map_err(|e| {
            VoeError::Storage(format!(
                "Failed to parse index {}: {}",
                self.index_path.display(),
                e
            ))
        })
    }

    fn save(&self, state: &IndexState) -> Result<()> {
        self.ensure_file()?;
        let json = serde_json::to_string_pretty(state)
            .map_err(|e| VoeError::Storage(format!("Failed to serialize index: {}", e)))?;
        self.backend.write_file(&self.index_path, json.as_bytes())
    }

    fn add_mask(&self, path: &str, mask_id: &ObjectId) -> Result<()> {
        let mut state = self.load()?;
        let entry = IndexEntry::with_mask(path.to_string(), mask_id.clone());
        state.add_entry(entry);
        self.save(&state)
    }

    fn remove_mask(&self, path: &str, mask_id: &ObjectId) -> Result<()> {
        let mut state = self.load()?;
        state.remove_mask(path, mask_id);
        self.save(&state)
    }

    fn clear(&self) -> Result<()> {
        let empty = IndexState::default();
        self.save(&empty)
    }
}
