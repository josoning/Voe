use std::path::{Path, PathBuf};

use voe_repo_api::repository::{RepoManager, Repository};
use voe_storage_api::StorageBackend;
use voe_types::error::Result;

use crate::backend::LocalFileBackend;

use super::repository::{FsRepository, VOE_DIR};

pub struct FsRepoManager;

impl FsRepoManager {
    pub fn new() -> Self {
        Self
    }
}

impl Default for FsRepoManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RepoManager for FsRepoManager {
    fn init(&self, path: PathBuf) -> Result<Box<dyn Repository>> {
        Ok(Box::new(FsRepository::create_new(path)?))
    }

    fn open(&self, path: PathBuf) -> Result<Box<dyn Repository>> {
        Ok(Box::new(FsRepository::open_existing(path)?))
    }

    fn try_open_or_init(&self, path: PathBuf) -> Result<Box<dyn Repository>> {
        let voe_dir = path.join(VOE_DIR);
        let backend = LocalFileBackend::new();
        if backend.exists(&voe_dir)? {
            self.open(path)
        } else {
            self.init(path)
        }
    }

    fn find_root(&self, start: &Path) -> Option<PathBuf> {
        let mut current = start.to_path_buf();
        loop {
            if current.join(VOE_DIR).exists() {
                return Some(current);
            }
            if !current.pop() {
                return None;
            }
        }
    }
}
