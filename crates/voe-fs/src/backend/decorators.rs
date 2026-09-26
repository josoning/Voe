use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use voe_storage_api::{StorageBackend, StorageMetadata};
use voe_types::error::Result;

pub struct LoggingStorageBackend {
    inner: Box<dyn StorageBackend>,
}

impl LoggingStorageBackend {
    pub fn new(inner: Box<dyn StorageBackend>) -> Self {
        Self { inner }
    }
}

impl StorageBackend for LoggingStorageBackend {
    fn exists(&self, path: &Path) -> Result<bool> {
        let r = self.inner.exists(path);
        match &r {
            Ok(v) => tracing::debug!(target: "storage", "exists({}) -> {}", path.display(), v),
            Err(e) => {
                tracing::warn!(target: "storage", "exists({}) -> FAILED: {}", path.display(), e)
            }
        }
        r
    }

    fn is_file(&self, path: &Path) -> Result<bool> {
        let r = self.inner.is_file(path);
        match &r {
            Ok(v) => tracing::debug!(target: "storage", "is_file({}) -> {}", path.display(), v),
            Err(e) => {
                tracing::warn!(target: "storage", "is_file({}) -> FAILED: {}", path.display(), e)
            }
        }
        r
    }

    fn is_dir(&self, path: &Path) -> Result<bool> {
        let r = self.inner.is_dir(path);
        match &r {
            Ok(v) => tracing::debug!(target: "storage", "is_dir({}) -> {}", path.display(), v),
            Err(e) => {
                tracing::warn!(target: "storage", "is_dir({}) -> FAILED: {}", path.display(), e)
            }
        }
        r
    }

    fn metadata(&self, path: &Path) -> Result<StorageMetadata> {
        let r = self.inner.metadata(path);
        match &r {
            Ok(_) => tracing::debug!(target: "storage", "metadata({}) -> OK", path.display()),
            Err(e) => {
                tracing::warn!(target: "storage", "metadata({}) -> FAILED: {}", path.display(), e)
            }
        }
        r
    }

    fn create_dir(&self, path: &Path) -> Result<()> {
        let r = self.inner.create_dir(path);
        match &r {
            Ok(_) => tracing::debug!(target: "storage", "create_dir({}) -> OK", path.display()),
            Err(e) => {
                tracing::warn!(target: "storage", "create_dir({}) -> FAILED: {}", path.display(), e)
            }
        }
        r
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        let r = self.inner.create_dir_all(path);
        match &r {
            Ok(_) => tracing::debug!(target: "storage", "create_dir_all({}) -> OK", path.display()),
            Err(e) => {
                tracing::warn!(target: "storage", "create_dir_all({}) -> FAILED: {}", path.display(), e)
            }
        }
        r
    }

    fn remove_dir(&self, path: &Path) -> Result<()> {
        let r = self.inner.remove_dir(path);
        match &r {
            Ok(_) => tracing::debug!(target: "storage", "remove_dir({}) -> OK", path.display()),
            Err(e) => {
                tracing::warn!(target: "storage", "remove_dir({}) -> FAILED: {}", path.display(), e)
            }
        }
        r
    }

    fn remove_dir_all(&self, path: &Path) -> Result<()> {
        let r = self.inner.remove_dir_all(path);
        match &r {
            Ok(_) => tracing::debug!(target: "storage", "remove_dir_all({}) -> OK", path.display()),
            Err(e) => {
                tracing::warn!(target: "storage", "remove_dir_all({}) -> FAILED: {}", path.display(), e)
            }
        }
        r
    }

    fn list_dir(&self, path: &Path) -> Result<Vec<PathBuf>> {
        let r = self.inner.list_dir(path);
        match &r {
            Ok(entries) => {
                tracing::debug!(target: "storage", "list_dir({}) -> {} entries", path.display(), entries.len())
            }
            Err(e) => {
                tracing::warn!(target: "storage", "list_dir({}) -> FAILED: {}", path.display(), e)
            }
        }
        r
    }

    fn create_file(&self, path: &Path) -> Result<()> {
        let r = self.inner.create_file(path);
        match &r {
            Ok(_) => tracing::debug!(target: "storage", "create_file({}) -> OK", path.display()),
            Err(e) => {
                tracing::warn!(target: "storage", "create_file({}) -> FAILED: {}", path.display(), e)
            }
        }
        r
    }

    fn read_file(&self, path: &Path) -> Result<Vec<u8>> {
        let r = self.inner.read_file(path);
        match &r {
            Ok(data) => {
                tracing::debug!(target: "storage", "read_file({}) -> {} bytes", path.display(), data.len())
            }
            Err(e) => {
                tracing::warn!(target: "storage", "read_file({}) -> FAILED: {}", path.display(), e)
            }
        }
        r
    }

    fn write_file(&self, path: &Path, data: &[u8]) -> Result<()> {
        let r = self.inner.write_file(path, data);
        tracing::debug!(target: "storage", "write_file({}) -> {} bytes {}", path.display(), data.len(), if r.is_ok() { "OK" } else { "FAILED" });
        r
    }

    fn append_to_file(&self, path: &Path, data: &[u8]) -> Result<()> {
        let r = self.inner.append_to_file(path, data);
        tracing::debug!(target: "storage", "append_to_file({}) -> {} bytes {}", path.display(), data.len(), if r.is_ok() { "OK" } else { "FAILED" });
        r
    }

    fn delete_file(&self, path: &Path) -> Result<()> {
        let r = self.inner.delete_file(path);
        match &r {
            Ok(_) => tracing::debug!(target: "storage", "delete_file({}) -> OK", path.display()),
            Err(e) => {
                tracing::warn!(target: "storage", "delete_file({}) -> FAILED: {}", path.display(), e)
            }
        }
        r
    }

    fn rename_file(&self, from: &Path, to: &Path) -> Result<()> {
        let r = self.inner.rename_file(from, to);
        tracing::debug!(target: "storage", "rename_file({} -> {}) {}", from.display(), to.display(), if r.is_ok() { "OK" } else { "FAILED" });
        r
    }

    fn move_file(&self, from: &Path, to: &Path) -> Result<()> {
        let r = self.inner.move_file(from, to);
        tracing::debug!(target: "storage", "move_file({} -> {}) {}", from.display(), to.display(), if r.is_ok() { "OK" } else { "FAILED" });
        r
    }

    fn copy_file(&self, from: &Path, to: &Path) -> Result<()> {
        let r = self.inner.copy_file(from, to);
        tracing::debug!(target: "storage", "copy_file({} -> {}) {}", from.display(), to.display(), if r.is_ok() { "OK" } else { "FAILED" });
        r
    }
}

pub struct CachedStorageBackend {
    inner: Box<dyn StorageBackend>,
    read_cache: Mutex<HashMap<PathBuf, Vec<u8>>>,
    metadata_cache: Mutex<HashMap<PathBuf, StorageMetadata>>,
}

impl CachedStorageBackend {
    pub fn new(inner: Box<dyn StorageBackend>) -> Self {
        Self {
            inner,
            read_cache: Mutex::new(HashMap::new()),
            metadata_cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn clear_cache(&self) {
        self.read_cache.lock().unwrap().clear();
        self.metadata_cache.lock().unwrap().clear();
    }

    fn invalidate(&self, path: &Path) {
        self.read_cache.lock().unwrap().remove(path);
        self.metadata_cache.lock().unwrap().remove(path);
    }
}

impl StorageBackend for CachedStorageBackend {
    fn exists(&self, path: &Path) -> Result<bool> {
        self.inner.exists(path)
    }

    fn is_file(&self, path: &Path) -> Result<bool> {
        self.inner.is_file(path)
    }

    fn is_dir(&self, path: &Path) -> Result<bool> {
        self.inner.is_dir(path)
    }

    fn metadata(&self, path: &Path) -> Result<StorageMetadata> {
        let key = path.to_path_buf();
        if let Some(cached) = self.metadata_cache.lock().unwrap().get(&key) {
            return Ok(cached.clone());
        }
        let meta = self.inner.metadata(path)?;
        self.metadata_cache
            .lock()
            .unwrap()
            .insert(key, meta.clone());
        Ok(meta)
    }

    fn create_dir(&self, path: &Path) -> Result<()> {
        self.inner.create_dir(path)
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        self.inner.create_dir_all(path)
    }

    fn remove_dir(&self, path: &Path) -> Result<()> {
        self.invalidate(path);
        self.inner.remove_dir(path)
    }

    fn remove_dir_all(&self, path: &Path) -> Result<()> {
        self.invalidate(path);
        self.inner.remove_dir_all(path)
    }

    fn list_dir(&self, path: &Path) -> Result<Vec<PathBuf>> {
        self.inner.list_dir(path)
    }

    fn create_file(&self, path: &Path) -> Result<()> {
        self.inner.create_file(path)
    }

    fn read_file(&self, path: &Path) -> Result<Vec<u8>> {
        let key = path.to_path_buf();
        if let Some(cached) = self.read_cache.lock().unwrap().get(&key) {
            return Ok(cached.clone());
        }
        let data = self.inner.read_file(path)?;
        self.read_cache.lock().unwrap().insert(key, data.clone());
        Ok(data)
    }

    fn write_file(&self, path: &Path, data: &[u8]) -> Result<()> {
        let r = self.inner.write_file(path, data);
        if r.is_ok() {
            self.read_cache
                .lock()
                .unwrap()
                .insert(path.to_path_buf(), data.to_vec());
            self.metadata_cache.lock().unwrap().remove(path);
        }
        r
    }

    fn append_to_file(&self, path: &Path, data: &[u8]) -> Result<()> {
        let r = self.inner.append_to_file(path, data);
        if r.is_ok() {
            self.invalidate(path);
        }
        r
    }

    fn delete_file(&self, path: &Path) -> Result<()> {
        self.invalidate(path);
        self.inner.delete_file(path)
    }

    fn rename_file(&self, from: &Path, to: &Path) -> Result<()> {
        let r = self.inner.rename_file(from, to);
        if r.is_ok() {
            let mut cache = self.read_cache.lock().unwrap();
            if let Some(data) = cache.remove(&from.to_path_buf()) {
                cache.insert(to.to_path_buf(), data);
            }
            self.metadata_cache.lock().unwrap().remove(from);
            self.metadata_cache.lock().unwrap().remove(to);
        }
        r
    }

    fn move_file(&self, from: &Path, to: &Path) -> Result<()> {
        let r = self.inner.move_file(from, to);
        if r.is_ok() {
            let mut cache = self.read_cache.lock().unwrap();
            if let Some(data) = cache.remove(&from.to_path_buf()) {
                cache.insert(to.to_path_buf(), data);
            }
            self.metadata_cache.lock().unwrap().remove(from);
            self.metadata_cache.lock().unwrap().remove(to);
        }
        r
    }

    fn copy_file(&self, from: &Path, to: &Path) -> Result<()> {
        let r = self.inner.copy_file(from, to);
        if r.is_ok() {
            let data_opt = self
                .read_cache
                .lock()
                .unwrap()
                .get(&from.to_path_buf())
                .cloned();
            if let Some(data) = data_opt {
                self.read_cache
                    .lock()
                    .unwrap()
                    .insert(to.to_path_buf(), data);
            }
        }
        r
    }
}
