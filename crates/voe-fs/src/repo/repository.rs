use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use voe_repo_api::config::ConfigManager;
use voe_repo_api::model::branch_store::BranchStore;
use voe_repo_api::model::commit::{CommitStore, IndexStore, RefStore};
use voe_repo_api::repository::Repository;
use voe_storage_api::{ChunkStore, ObjectStore, StorageBackend};
use voe_types::error::Result;
use voe_types::object::ObjectId;

use crate::backend::LocalFileBackend;
use crate::config_backend::{FsConfigManager, TomlConfigSource};
use crate::filesystem::FileSystemObjectStore;

use super::branch_store::FsBranchStore;
use super::index::FsIndexStore;

pub const VOE_DIR: &str = ".voe";
pub const OBJECTS_DIR: &str = "objects";
pub const REFS_DIR: &str = "refs";
pub const HEAD_FILE: &str = "HEAD";
pub const INDEX_FILE: &str = "index.json";
pub const CONFIG_TOML: &str = "voeconfig.toml";
pub const CONFIG_LOCK: &str = "voeconfig.lock";

pub struct FsRepository {
    path: PathBuf,
    objects: Arc<FileSystemObjectStore>,
    branch_store: FsBranchStore,
    index: FsIndexStore,
    config_mgr: FsConfigManager<TomlConfigSource>,
    backend: Arc<dyn StorageBackend>,
}

impl FsRepository {
    pub fn voe_dir(&self) -> PathBuf {
        self.path.join(VOE_DIR)
    }

    pub fn objects_dir(&self) -> PathBuf {
        self.voe_dir().join(OBJECTS_DIR)
    }

    pub fn refs_dir(&self) -> PathBuf {
        self.voe_dir().join(REFS_DIR)
    }

    pub fn head_path(&self) -> PathBuf {
        self.voe_dir().join(HEAD_FILE)
    }

    pub fn config_toml_path(&self) -> PathBuf {
        self.path.join(CONFIG_TOML)
    }

    pub fn config_lock_path(&self) -> PathBuf {
        self.voe_dir().join(CONFIG_LOCK)
    }

    pub fn index_path(&self) -> PathBuf {
        self.voe_dir().join(INDEX_FILE)
    }

    pub fn backend(&self) -> &dyn StorageBackend {
        &*self.backend
    }

    pub fn with_backend(path: PathBuf, backend: Arc<dyn StorageBackend>) -> Result<Self> {
        let voe_dir = path.join(VOE_DIR);
        if backend.exists(&voe_dir)? {
            return Err(voe_types::error::VoeError::RepoAlreadyExists { path });
        }
        backend.create_dir_all(&voe_dir)?;
        let objects_dir = voe_dir.join(OBJECTS_DIR);
        backend.create_dir_all(&objects_dir)?;
        let refs_dir = voe_dir.join(REFS_DIR);
        backend.create_dir_all(&refs_dir)?;

        let toml_path = path.join(CONFIG_TOML);
        let lock_path = voe_dir.join(CONFIG_LOCK);
        let source = TomlConfigSource::with_backend(toml_path, backend.clone());
        let config_mgr = FsConfigManager::with_backend(source, lock_path, backend.clone());
        config_mgr.init_defaults()?;

        let objects = Arc::new(FileSystemObjectStore::with_backend(
            objects_dir.clone(),
            backend.clone(),
        ));

        let ref_store = crate::repo::refs::FsRefStore::with_backend(
            refs_dir,
            voe_dir.join(HEAD_FILE),
            backend.clone(),
        );
        let branch_store = FsBranchStore::with_backend(
            voe_dir.clone(),
            ref_store,
            objects.clone(),
            backend.clone(),
        );

        let index = FsIndexStore::with_backend(voe_dir.join(INDEX_FILE), backend.clone());
        index.ensure_file()?;

        let device_id = config_mgr.lock_state().device_id;
        let device_id = if device_id.is_empty() {
            "unknown"
        } else {
            &device_id
        };
        let mainline = branch_store.ensure_mainline(ObjectId::NULL, device_id)?;
        branch_store.switch_branch(&mainline.id)?;

        Ok(Self {
            path,
            objects,
            branch_store,
            index,
            config_mgr,
            backend,
        })
    }

    pub fn open_with_backend(path: PathBuf, backend: Arc<dyn StorageBackend>) -> Result<Self> {
        let voe_dir = path.join(VOE_DIR);
        if !backend.exists(&voe_dir)? {
            return Err(voe_types::error::VoeError::RepoNotFound { path });
        }

        let objects_dir = voe_dir.join(OBJECTS_DIR);
        let objects = Arc::new(FileSystemObjectStore::with_backend(
            objects_dir.clone(),
            backend.clone(),
        ));

        let ref_store = crate::repo::refs::FsRefStore::with_backend(
            voe_dir.join(REFS_DIR),
            voe_dir.join(HEAD_FILE),
            backend.clone(),
        );
        let branch_store = FsBranchStore::with_backend(
            voe_dir.clone(),
            ref_store,
            objects.clone(),
            backend.clone(),
        );

        let index = FsIndexStore::with_backend(voe_dir.join(INDEX_FILE), backend.clone());
        index.ensure_file()?;

        let toml_path = path.join(CONFIG_TOML);
        let lock_path = voe_dir.join(CONFIG_LOCK);
        let source = TomlConfigSource::with_backend(toml_path, backend.clone());
        let config_mgr = FsConfigManager::with_backend(source, lock_path, backend.clone());
        config_mgr.refresh_internal()?;

        Ok(Self {
            path,
            objects,
            branch_store,
            index,
            config_mgr,
            backend,
        })
    }

    pub fn create_new(path: PathBuf) -> Result<Self> {
        Self::with_backend(path, Arc::new(LocalFileBackend::new()))
    }

    pub fn open_existing(path: PathBuf) -> Result<Self> {
        Self::open_with_backend(path, Arc::new(LocalFileBackend::new()))
    }
}

impl Repository for FsRepository {
    fn path(&self) -> &PathBuf {
        &self.path
    }

    fn object_store(&self) -> &dyn ObjectStore {
        &*self.objects
    }

    fn chunk_store(&self) -> &dyn ChunkStore {
        &*self.objects
    }

    fn commit_store(&self) -> &dyn CommitStore {
        &*self.objects
    }

    fn ref_store(&self) -> &dyn RefStore {
        // FsBranchStore implements RefStore by delegating to its internal
        // FsRefStore, so we simply hand it out here.
        &self.branch_store
    }

    fn branch_store(&self) -> &dyn BranchStore {
        &self.branch_store
    }

    fn index_store(&self) -> &dyn IndexStore {
        &self.index
    }

    fn config_manager(&self) -> &dyn ConfigManager {
        &self.config_mgr
    }

    fn config_manager_mut(&mut self) -> &mut dyn ConfigManager {
        &mut self.config_mgr
    }

    fn read_working_tree(&self) -> Result<HashMap<PathBuf, Vec<u8>>> {
        crate::working_tree::read_working_tree_with_backend(&self.path, &*self.backend)
    }

    fn write_snapshot_to_disk(&self, snapshot: &HashMap<PathBuf, Vec<u8>>) -> Result<()> {
        crate::working_tree::write_snapshot_to_disk_with_backend(
            &self.path,
            snapshot,
            &*self.backend,
        )
    }
}
