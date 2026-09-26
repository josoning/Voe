use std::path::{Path, PathBuf};
use std::sync::Arc;

use voe_repo_api::model::commit::RefStore;
use voe_storage_api::StorageBackend;
use voe_types::error::{Result, VoeError};
use voe_types::object::ObjectId;

use crate::backend::LocalFileBackend;

pub struct FsRefStore {
    refs_dir: PathBuf,
    head_path: PathBuf,
    backend: Arc<dyn StorageBackend>,
}

impl FsRefStore {
    pub fn new(refs_dir: PathBuf, head_path: PathBuf) -> Self {
        Self::with_backend(refs_dir, head_path, Arc::new(LocalFileBackend::new()))
    }

    pub fn with_backend(
        refs_dir: PathBuf,
        head_path: PathBuf,
        backend: Arc<dyn StorageBackend>,
    ) -> Self {
        Self {
            refs_dir,
            head_path,
            backend,
        }
    }

    pub fn refs_dir(&self) -> &PathBuf {
        &self.refs_dir
    }

    fn ensure_dirs(&self) -> Result<()> {
        if !self.backend.exists(&self.refs_dir)? {
            self.backend.create_dir_all(&self.refs_dir)?;
        }
        Ok(())
    }

    fn ref_path(&self, name: &str) -> PathBuf {
        self.refs_dir.join(name)
    }

    fn read_file(&self, path: &Path) -> Result<Option<String>> {
        match self.backend.read_file_to_string(path) {
            Ok(content) => Ok(Some(content.trim().to_string())),
            Err(VoeError::FileNotFound { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn write_file(&self, path: &Path, content: &str) -> Result<()> {
        self.backend.ensure_parent_dir(path)?;
        self.backend.write_file(path, content.as_bytes())
    }
}

impl RefStore for FsRefStore {
    fn get_head(&self) -> Result<Option<ObjectId>> {
        let content = self.read_file(&self.head_path)?;
        Ok(content.map(ObjectId::new))
    }

    fn set_head(&self, id: &ObjectId) -> Result<()> {
        self.write_file(&self.head_path, &id.to_string())
    }

    fn get_ref(&self, name: &str) -> Result<Option<ObjectId>> {
        let path = self.ref_path(name);
        let content = self.read_file(&path)?;
        Ok(content.map(ObjectId::new))
    }

    fn set_ref(&self, name: &str, id: &ObjectId) -> Result<()> {
        self.ensure_dirs()?;
        let path = self.ref_path(name);
        self.write_file(&path, &id.to_string())
    }

    fn delete_ref(&self, name: &str) -> Result<()> {
        let path = self.ref_path(name);
        self.backend.delete_file(&path)
    }

    fn list_refs(&self) -> Result<Vec<(String, ObjectId)>> {
        let mut result: Vec<(String, ObjectId)> = Vec::new();
        if !self.backend.exists(&self.refs_dir)? {
            return Ok(result);
        }

        let mut stack: Vec<PathBuf> = vec![self.refs_dir.clone()];
        let base_prefix = self.refs_dir.to_string_lossy().to_string();

        while let Some(dir) = stack.pop() {
            let entries = self.backend.list_dir(&dir)?;
            for entry_path in entries {
                let meta = self.backend.metadata(&entry_path)?;
                if meta.is_dir() {
                    stack.push(entry_path);
                } else {
                    let stripped = entry_path
                        .to_string_lossy()
                        .trim_start_matches(&base_prefix)
                        .trim_start_matches('/')
                        .to_string();
                    let content = self.read_file(&entry_path)?;
                    if let Some(oid) = content {
                        result.push((stripped, ObjectId::new(oid)));
                    }
                }
            }
        }

        result.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(result)
    }
}
