//! Filesystem-backed implementation of [`BranchStore`].
//!
//! Layout inside the `.voe/` directory:
//!
//! ```text
//! .voe/
//!   refs/                       ← FsRefStore (storage_key → head ObjectId)
//!   branches/                   ← Branch metadata (JSON)
//!     main@mainline.json
//!     dev@fix.json
//!   aliases/                    ← Alias mapping (alias_name → storage_key)
//!     dev                       ← content: "main@mainline"
//!   HEAD_BRANCH                 ← Current branch storage_key (empty = detached)
//!   HEAD                        ← HEAD commit ObjectId
//!   objects/                    ← ObjectStore
//! ```

use std::path::{Path, PathBuf};

use std::sync::Arc;

use voe_repo_api::model::branch::{Alias, Branch, BranchId, MergeNode};
use voe_repo_api::model::branch_store::{
    validate_reserved_labels, BranchStore, CreateBranchOptions,
};
use voe_repo_api::model::commit::RefStore;
use voe_storage_api::{ObjectStore, StorageBackend};
use voe_types::error::{Result, VoeError};
use voe_types::object::{ObjectId, ObjectKind, VoeObject};

use crate::backend::LocalFileBackend;
use crate::repo::refs::FsRefStore;

pub const BRANCHES_DIR: &str = "branches";
pub const ALIASES_DIR: &str = "aliases";
pub const HEAD_BRANCH_FILE: &str = "HEAD_BRANCH";

/// Filesystem implementation of [`BranchStore`].
///
/// Delegates low-level ref operations (`RefStore`) to an embedded
/// [`FsRefStore`] while persisting branch metadata as JSON files and
/// aliases as plain-text mappings.  Merge nodes are written via an
/// external object store.
pub struct FsBranchStore {
    ref_store: FsRefStore,
    object_store: Arc<dyn ObjectStore>,
    backend: Arc<dyn StorageBackend>,
    voe_dir: PathBuf,
}

impl FsBranchStore {
    /// Construct a new `FsBranchStore` backed by the given object store.
    pub fn new(voe_dir: PathBuf, object_store: Arc<dyn ObjectStore>) -> Self {
        let refs_dir = voe_dir.join("refs");
        let head_path = voe_dir.join("HEAD");
        Self::with_backend(
            voe_dir,
            FsRefStore::new(refs_dir, head_path),
            object_store,
            Arc::new(LocalFileBackend::new()),
        )
    }

    /// Fully configurable constructor (mostly for testing).
    pub fn with_backend(
        voe_dir: PathBuf,
        ref_store: FsRefStore,
        object_store: Arc<dyn ObjectStore>,
        backend: Arc<dyn StorageBackend>,
    ) -> Self {
        let store = Self {
            ref_store,
            object_store,
            backend,
            voe_dir,
        };
        let _ = store.ensure_dirs();
        store
    }

    /// Return the `.voe/` directory this store uses.
    pub fn voe_dir(&self) -> &Path {
        &self.voe_dir
    }

    fn branches_dir(&self) -> PathBuf {
        self.voe_dir.join(BRANCHES_DIR)
    }

    fn aliases_dir(&self) -> PathBuf {
        self.voe_dir.join(ALIASES_DIR)
    }

    fn head_branch_path(&self) -> PathBuf {
        self.voe_dir.join(HEAD_BRANCH_FILE)
    }

    /// Make sure every sub-directory we need exists.  Idempotent.
    fn ensure_dirs(&self) -> Result<()> {
        for dir in [self.branches_dir(), self.aliases_dir()] {
            if !self.backend.exists(&dir)? {
                self.backend.create_dir_all(&dir)?;
            }
        }
        Ok(())
    }

    fn branch_meta_path(&self, storage_key: &str) -> PathBuf {
        self.branches_dir().join(format!("{}.json", storage_key))
    }

    fn alias_path(&self, alias_name: &str) -> PathBuf {
        self.aliases_dir().join(alias_name)
    }

    fn read_file(&self, path: &Path) -> Result<Option<String>> {
        match self.backend.read_file_to_string(path) {
            Ok(s) if s.trim().is_empty() => Ok(None),
            Ok(s) => Ok(Some(s)),
            Err(VoeError::FileNotFound { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn write_file(&self, path: &Path, content: &str) -> Result<()> {
        self.backend.ensure_parent_dir(path)?;
        self.backend.write_file(path, content.as_bytes())
    }

    fn write_branch_json(&self, branch: &Branch) -> Result<()> {
        let path = self.branch_meta_path(&branch.id.storage_key());
        let json = serde_json::to_string_pretty(branch)
            .map_err(|e| VoeError::Storage(format!("Failed to serialize branch metadata: {e}")))?;
        self.write_file(&path, &json)
    }

    fn read_branch_json(&self, storage_key: &str) -> Result<Option<Branch>> {
        let path = self.branch_meta_path(storage_key);
        match self.read_file(&path)? {
            Some(content) => {
                let branch: Branch = serde_json::from_str(&content).map_err(|e| {
                    VoeError::Storage(format!("Failed to parse branch metadata: {e}"))
                })?;
                Ok(Some(branch))
            }
            None => Ok(None),
        }
    }

    fn delete_branch_json(&self, storage_key: &str) -> Result<()> {
        let path = self.branch_meta_path(storage_key);
        self.backend.delete_file(&path)
    }

    fn write_head_branch(&self, storage_key: &str) -> Result<()> {
        if storage_key.is_empty() {
            // Empty HEAD_BRANCH = detached HEAD
            self.backend.delete_file(&self.head_branch_path())?;
            self.backend.create_file(&self.head_branch_path())?;
        } else {
            self.write_file(&self.head_branch_path(), storage_key)?;
        }
        Ok(())
    }

    fn read_head_branch(&self) -> Result<Option<String>> {
        let content = self.read_file(&self.head_branch_path())?;
        Ok(content.filter(|s| !s.trim().is_empty()))
    }
}

impl RefStore for FsBranchStore {
    fn get_head(&self) -> Result<Option<ObjectId>> {
        self.ref_store.get_head()
    }

    fn set_head(&self, id: &ObjectId) -> Result<()> {
        self.ref_store.set_head(id)
    }

    fn get_ref(&self, name: &str) -> Result<Option<ObjectId>> {
        self.ref_store.get_ref(name)
    }

    fn set_ref(&self, name: &str, id: &ObjectId) -> Result<()> {
        self.ref_store.set_ref(name, id)
    }

    fn delete_ref(&self, name: &str) -> Result<()> {
        self.ref_store.delete_ref(name)
    }

    fn list_refs(&self) -> Result<Vec<(String, ObjectId)>> {
        self.ref_store.list_refs()
    }
}

impl BranchStore for FsBranchStore {
    // ------------------------------------------------------------------
    // Branch CRUD
    // ------------------------------------------------------------------

    fn create_branch(
        &self,
        id: &BranchId,
        head: ObjectId,
        creator_device_id: &str,
        options: &CreateBranchOptions,
    ) -> Result<Branch> {
        validate_reserved_labels(id, options.allow_reserved_labels, "branch creation")?;

        let storage_key = id.storage_key();
        if let Some(existing) = self.get_branch_by_key(&storage_key)? {
            if options.if_not_exists {
                return Ok(existing);
            }
            return Err(VoeError::BranchAlreadyExists(storage_key));
        }

        // Enforce mainline uniqueness (rule 9).
        if id.has_mainline() {
            let existing_mainline = self.get_mainline()?;
            if existing_mainline.is_some() {
                return Err(VoeError::MainlineNotUnique);
            }
        }

        // Write the ref → head pointer.
        self.set_ref(&storage_key, &head)?;

        let branch = Branch::new(
            id.clone(),
            head,
            creator_device_id.to_string(),
            voe_types::author::current_timestamp(),
        );
        self.store_branch_metadata(&branch)?;

        Ok(branch)
    }

    fn get_branch_by_key(&self, storage_key: &str) -> Result<Option<Branch>> {
        self.read_branch_json(storage_key)
    }

    fn list_branches(&self) -> Result<Vec<Branch>> {
        // The refs directory holds storage_key → ObjectId files for every
        // known branch.  We read metadata for each, falling back to a
        // synthetic Branch built from the ref if metadata is missing
        // (defensive).
        let mut out: Vec<Branch> = Vec::new();
        for (storage_key, oid) in self.list_refs()? {
            if let Some(meta) = self.read_branch_json(&storage_key)? {
                out.push(meta);
            } else if let Ok(id) = BranchId::parse(&storage_key) {
                out.push(Branch::new(id, oid, "unknown".to_string(), 0));
            }
        }
        out.sort_by_key(|a| a.id.storage_key());
        Ok(out)
    }

    fn delete_branch(&self, id: &BranchId) -> Result<()> {
        let storage_key = id.storage_key();

        // Refuse to delete the currently checked-out branch.
        if let Some(cur) = self.read_head_branch()? {
            if cur == storage_key {
                return Err(VoeError::Branch(
                    "cannot delete the currently checked-out branch — switch first".to_string(),
                ));
            }
        }

        // Collect and delete any aliases pointing at this branch.
        let aliases = self.list_aliases_for_branch_storage_key(&storage_key)?;
        for alias_name in aliases {
            let path = self.alias_path(&alias_name);
            self.backend.delete_file(&path)?;
        }

        self.delete_ref(&storage_key)?;
        self.delete_branch_json(&storage_key)?;
        Ok(())
    }

    fn set_branch_head(&self, id: &BranchId, new_head: ObjectId) -> Result<()> {
        let storage_key = id.storage_key();
        let mut branch = self
            .get_branch_by_key(&storage_key)?
            .ok_or_else(|| VoeError::BranchNotFound(storage_key.clone()))?;

        self.set_ref(&storage_key, &new_head)?;
        branch.head = new_head;
        branch.last_updated = voe_types::author::current_timestamp();
        self.store_branch_metadata(&branch)?;
        Ok(())
    }

    fn touch_branch(&self, id: &BranchId) -> Result<()> {
        let storage_key = id.storage_key();
        let mut branch = self
            .get_branch_by_key(&storage_key)?
            .ok_or_else(|| VoeError::BranchNotFound(storage_key.clone()))?;
        branch.last_updated = voe_types::author::current_timestamp();
        self.store_branch_metadata(&branch)?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Rename / re-label — storage_key migration
    // ------------------------------------------------------------------

    fn migrate_branch_id(&self, old_id: &BranchId, new_id: &BranchId) -> Result<()> {
        let old_key = old_id.storage_key();
        let new_key = new_id.storage_key();

        // 1. Move the ref file (read ObjectId, write under new key, delete old).
        if let Some(oid) = self.get_ref(&old_key)? {
            self.set_ref(&new_key, &oid)?;
            self.delete_ref(&old_key)?;
        }

        // 2. Delete the old metadata JSON — the caller will write the
        //    updated branch under the new key afterward.
        let old_meta = self.branch_meta_path(&old_key);
        if self.backend.exists(&old_meta)? {
            self.backend.delete_file(&old_meta)?;
        }

        // 3. Re-point every alias that targeted the old key to the new key.
        let aliases = self.list_aliases_for_branch_storage_key(&old_key)?;
        for alias_name in aliases {
            let alias_path = self.alias_path(&alias_name);
            self.write_file(&alias_path, &new_key)?;
        }

        // 4. If this branch is currently checked out, update HEAD_BRANCH.
        if let Some(cur) = self.read_head_branch()? {
            if cur == old_key {
                self.write_head_branch(&new_key)?;
            }
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // Current branch / switching
    // ------------------------------------------------------------------

    fn current_branch(&self) -> Result<Option<Branch>> {
        match self.read_head_branch()? {
            Some(key) => self.get_branch_by_key(&key),
            None => Ok(None),
        }
    }

    fn switch_branch(&self, id: &BranchId) -> Result<()> {
        let storage_key = id.storage_key();
        let branch = self
            .get_branch_by_key(&storage_key)?
            .ok_or_else(|| VoeError::BranchNotFound(storage_key.clone()))?;
        self.set_head(&branch.head)?;
        self.write_head_branch(&storage_key)?;
        Ok(())
    }

    fn detach_head(&self) -> Result<()> {
        self.write_head_branch("")
    }

    // ------------------------------------------------------------------
    // Aliases
    // ------------------------------------------------------------------

    fn add_alias_inner(&self, id: &BranchId, alias: &Alias) -> Result<()> {
        let path = self.alias_path(&alias.name);
        self.ensure_dirs()?;
        self.write_file(&path, &id.storage_key())
    }

    fn resolve_alias(&self, alias_name: &str) -> Result<Option<String>> {
        self.read_file(&self.alias_path(alias_name))
    }

    fn remove_alias(&self, id: &BranchId, alias_name: &str) -> Result<()> {
        let alias = Alias::new(alias_name)?;
        let path = self.alias_path(&alias.name);
        match self.backend.read_file_to_string(&path) {
            Ok(content) => {
                let target = content.trim();
                if target != id.storage_key() {
                    return Err(VoeError::AliasNotFound {
                        alias: format!("#{}", alias.name),
                    });
                }
                self.backend.delete_file(&path)
            }
            Err(VoeError::FileNotFound { .. }) => Err(VoeError::AliasNotFound {
                alias: format!("#{}", alias.name),
            }),
            Err(e) => Err(e),
        }
    }

    // ------------------------------------------------------------------
    // Merge
    // ------------------------------------------------------------------

    fn store_merge_node(&self, node: &MergeNode) -> Result<ObjectId> {
        let data = serde_json::to_vec(node)
            .map_err(|e| VoeError::Storage(format!("Failed to serialize MergeNode: {e}")))?;
        let obj = VoeObject {
            id: ObjectId::from_bytes(&data),
            kind: ObjectKind::Merge,
            content: data,
        };
        self.object_store.store(&obj)
    }

    // ------------------------------------------------------------------
    // Metadata
    // ------------------------------------------------------------------

    fn store_branch_metadata(&self, branch: &Branch) -> Result<()> {
        self.ensure_dirs()?;
        self.write_branch_json(branch)
    }
}

impl FsBranchStore {
    /// Internal helper: list every alias that currently points at a given
    /// branch storage_key.  Used by `delete_branch` to clean up alias files.
    fn list_aliases_for_branch_storage_key(&self, storage_key: &str) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let aliases_dir = self.aliases_dir();
        if !self.backend.exists(&aliases_dir)? {
            return Ok(out);
        }
        let entries = self.backend.list_dir(&aliases_dir)?;
        for entry in entries {
            let meta = self.backend.metadata(&entry)?;
            if !meta.is_file() {
                continue;
            }
            match self.backend.read_file_to_string(&entry) {
                Ok(content) if content.trim() == storage_key => {
                    if let Some(name) = entry.file_name().and_then(|n| n.to_str()) {
                        out.push(name.to_string());
                    }
                }
                _ => continue,
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use voe_repo_api::model::branch_store::MergeOptions;

    fn tempdir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        let nonce = format!("voe_branch_test_{}_{}", name, std::process::id());
        p.push(nonce);
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    struct TestStore {
        store: FsBranchStore,
        dir: PathBuf,
    }

    impl Drop for TestStore {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn make_store(dir: &Path) -> TestStore {
        let voe_dir = dir.join(".voe");
        fs::create_dir_all(&voe_dir).unwrap();
        let obj_dir = voe_dir.join("objects");
        let obj_store = crate::filesystem::FileSystemObjectStore::new(obj_dir);
        let store = FsBranchStore::new(voe_dir, std::sync::Arc::new(obj_store));
        TestStore {
            store,
            dir: dir.to_path_buf(),
        }
    }

    fn oid(s: &str) -> ObjectId {
        ObjectId::from_bytes(s.as_bytes())
    }

    #[test]
    fn test_create_and_get_branch() {
        let dir = tempdir("create_get");
        let ts = make_store(&dir);

        let id = BranchId::parse("dev").unwrap();
        let head = oid("abc");
        let branch = ts
            .store
            .create_branch(
                &id,
                head.clone(),
                "device-1",
                &CreateBranchOptions::default(),
            )
            .unwrap();
        assert_eq!(branch.id.storage_key(), "dev@default");
        assert_eq!(branch.head, head);

        let fetched = ts.store.get_branch_by_key("dev@default").unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().head, head);

        // Verify ref file on disk.
        let ref_content = fs::read_to_string(dir.join(".voe/refs/dev@default")).unwrap();
        assert_eq!(ref_content.trim(), head.to_string());
    }

    #[test]
    fn test_create_branch_mainline_unique() {
        let dir = tempdir("mainline_unique");
        let ts = make_store(&dir);

        let mainline = BranchId::mainline_default();
        ts.store
            .create_branch(
                &mainline,
                oid("init"),
                "device-1",
                &CreateBranchOptions {
                    allow_reserved_labels: true,
                    ..Default::default()
                },
            )
            .unwrap();

        // Second mainline should fail.
        let dup = BranchId::parse("trunk@mainline").unwrap();
        let err = ts
            .store
            .create_branch(
                &dup,
                oid("x"),
                "device-1",
                &CreateBranchOptions {
                    allow_reserved_labels: true,
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert!(matches!(err, VoeError::MainlineNotUnique));
    }

    #[test]
    fn test_create_branch_reserved_label_gate() {
        let dir = tempdir("reserved_gate");
        let ts = make_store(&dir);

        let id = BranchId::parse("hotfix@release").unwrap();
        let err = ts
            .store
            .create_branch(&id, oid("x"), "device-1", &CreateBranchOptions::default())
            .unwrap_err();
        assert!(matches!(err, VoeError::ReservedKeyword { .. }));

        // With allow_reserved_labels it should succeed.
        let b = ts
            .store
            .create_branch(
                &id,
                oid("x"),
                "device-1",
                &CreateBranchOptions {
                    allow_reserved_labels: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(b.id.name, "hotfix");
    }

    #[test]
    fn test_list_branches() {
        let dir = tempdir("list_branches");
        let ts = make_store(&dir);

        ts.store
            .create_branch(
                &BranchId::mainline_default(),
                oid("init"),
                "dev1",
                &CreateBranchOptions {
                    allow_reserved_labels: true,
                    ..Default::default()
                },
            )
            .unwrap();
        ts.store
            .create_branch(
                &BranchId::parse("feature/a").unwrap(),
                oid("f1"),
                "dev2",
                &CreateBranchOptions::default(),
            )
            .unwrap();
        ts.store
            .create_branch(
                &BranchId::parse("feature/b").unwrap(),
                oid("f2"),
                "dev1",
                &CreateBranchOptions::default(),
            )
            .unwrap();

        let list = ts.store.list_branches().unwrap();
        assert_eq!(list.len(), 3);
        // Sorted by storage key.
        assert_eq!(list[0].id.storage_key(), "feature/a@default");
        assert_eq!(list[1].id.storage_key(), "feature/b@default");
        assert_eq!(list[2].id.storage_key(), "main@mainline");

        let by_dev1 = ts.store.list_branches_by_creator("dev1").unwrap();
        assert_eq!(by_dev1.len(), 2);
    }

    #[test]
    fn test_switch_and_current_branch() {
        let dir = tempdir("switch");
        let ts = make_store(&dir);

        let mainline = ts
            .store
            .create_branch(
                &BranchId::mainline_default(),
                oid("init"),
                "dev",
                &CreateBranchOptions {
                    allow_reserved_labels: true,
                    ..Default::default()
                },
            )
            .unwrap();
        let feature = ts
            .store
            .create_branch(
                &BranchId::parse("feature").unwrap(),
                oid("f"),
                "dev",
                &CreateBranchOptions::default(),
            )
            .unwrap();

        // Initially HEAD_BRANCH should be empty.
        assert!(ts.store.current_branch().unwrap().is_none());
        assert!(ts.store.is_detached_head().unwrap());

        ts.store.switch_branch(&mainline.id).unwrap();
        let cur = ts.store.current_branch().unwrap().unwrap();
        assert_eq!(cur.id.storage_key(), "main@mainline");
        let init_head = oid("init");
        assert_eq!(
            fs::read_to_string(dir.join(".voe/HEAD")).unwrap().trim(),
            init_head.to_string()
        );
        assert!(!ts.store.is_detached_head().unwrap());

        ts.store.switch_branch(&feature.id).unwrap();
        let cur2 = ts.store.current_branch().unwrap().unwrap();
        assert_eq!(cur2.id.storage_key(), "feature@default");
    }

    #[test]
    fn test_delete_branch_rejects_current() {
        let dir = tempdir("delete_current");
        let ts = make_store(&dir);

        let mainline = ts
            .store
            .create_branch(
                &BranchId::mainline_default(),
                oid("init"),
                "dev",
                &CreateBranchOptions {
                    allow_reserved_labels: true,
                    ..Default::default()
                },
            )
            .unwrap();
        ts.store.switch_branch(&mainline.id).unwrap();

        let err = ts.store.delete_branch(&mainline.id).unwrap_err();
        assert!(matches!(err, VoeError::Branch(_)));
    }

    #[test]
    fn test_delete_branch_also_removes_aliases() {
        let dir = tempdir("delete_alias_cleanup");
        let ts = make_store(&dir);

        let b = ts
            .store
            .create_branch(
                &BranchId::mainline_default(),
                oid("init"),
                "dev",
                &CreateBranchOptions {
                    allow_reserved_labels: true,
                    ..Default::default()
                },
            )
            .unwrap();
        ts.store.switch_branch(&b.id).unwrap();

        ts.store.add_alias(&b.id, "trunk").unwrap();
        ts.store.add_alias(&b.id, "ml").unwrap();

        // Create another branch and switch to it, so we can delete mainline.
        let other = ts
            .store
            .create_branch(
                &BranchId::parse("other").unwrap(),
                oid("o"),
                "dev",
                &CreateBranchOptions::default(),
            )
            .unwrap();
        ts.store.switch_branch(&other.id).unwrap();

        ts.store.delete_branch(&b.id).unwrap();
        // Aliases should be gone.
        assert!(ts.store.resolve_alias("trunk").unwrap().is_none());
        assert!(ts.store.resolve_alias("ml").unwrap().is_none());
    }

    #[test]
    fn test_alias_crud() {
        let dir = tempdir("alias");
        let ts = make_store(&dir);

        let b = ts
            .store
            .create_branch(
                &BranchId::mainline_default(),
                oid("init"),
                "dev",
                &CreateBranchOptions {
                    allow_reserved_labels: true,
                    ..Default::default()
                },
            )
            .unwrap();

        // Create alias (both with and without `#` prefix should work).
        let a1 = ts.store.add_alias(&b.id, "trunk").unwrap();
        assert_eq!(a1.name, "trunk");
        assert!(ts.store.add_alias(&b.id, "#ml").is_ok()); // leading # stripped

        // Duplicate alias → error.
        let err = ts.store.add_alias(&b.id, "trunk").unwrap_err();
        assert!(matches!(err, VoeError::AliasAlreadyExists { .. }));

        // Resolve.
        let key = ts.store.resolve_alias("trunk").unwrap().unwrap();
        assert_eq!(key, "main@mainline");

        // Remove.
        ts.store.remove_alias(&b.id, "trunk").unwrap();
        assert!(ts.store.resolve_alias("trunk").unwrap().is_none());

        // Remove non-existent → error.
        assert!(matches!(
            ts.store.remove_alias(&b.id, "ghost").unwrap_err(),
            VoeError::AliasNotFound { .. }
        ));

        // Remove wrong target → error.
        let other = ts
            .store
            .create_branch(
                &BranchId::parse("other").unwrap(),
                oid("o"),
                "dev",
                &CreateBranchOptions::default(),
            )
            .unwrap();
        ts.store.add_alias(&other.id, "trick").unwrap();
        let err = ts.store.remove_alias(&b.id, "trick").unwrap_err();
        assert!(matches!(err, VoeError::AliasNotFound { .. }));
    }

    #[test]
    fn test_auto_temp_branch_on_detached() {
        let dir = tempdir("auto_temp");
        let ts = make_store(&dir);

        // Set a detached HEAD commit (no HEAD_BRANCH file).
        ts.store.set_head(&oid("orphan")).unwrap();
        assert!(ts.store.is_detached_head().unwrap());

        let temp = ts.store.auto_temp_branch_on_detached("dev").unwrap();
        assert_eq!(
            temp.id.name,
            format!("temp/{}", voe_types::author::current_timestamp())
        );
        assert_eq!(temp.id.labels, vec!["temp".to_string()]);
        assert_eq!(temp.head, oid("orphan"));

        // Now attached — calling again should fail with DetachedHead.
        assert!(matches!(
            ts.store.auto_temp_branch_on_detached("dev").unwrap_err(),
            VoeError::DetachedHead
        ));
    }

    #[test]
    fn test_merge_branches_creates_merge_node() {
        let dir = tempdir("merge");
        let ts = make_store(&dir);

        let primary = ts
            .store
            .create_branch(
                &BranchId::mainline_default(),
                oid("init"),
                "dev",
                &CreateBranchOptions {
                    allow_reserved_labels: true,
                    ..Default::default()
                },
            )
            .unwrap();
        let sec = ts
            .store
            .create_branch(
                &BranchId::parse("feature").unwrap(),
                oid("f"),
                "dev",
                &CreateBranchOptions::default(),
            )
            .unwrap();

        let node = ts
            .store
            .merge_branches(
                &primary.id,
                &[sec.id.clone()],
                &[oid("base")],
                &MergeOptions {
                    message: Some("merge feature into main".to_string()),
                    patch_mask_ids: vec![],
                },
            )
            .unwrap();

        let node_oid = ts.store.store_merge_node(&node).unwrap();

        assert_eq!(node.primary_branch_storage_key, primary.id.storage_key());
        assert_eq!(
            node.secondary_branch_storage_keys,
            vec![sec.id.storage_key()]
        );
        assert_eq!(node.merge_bases, vec![oid("base")]);
        assert_eq!(node.message, "merge feature into main");

        // Merge node should have been stored in the object store.
        assert!(ts.store.object_store.exists(&node_oid).unwrap());
    }

    #[test]
    fn test_resolve_reference_handles_alias() {
        let dir = tempdir("resolve_ref");
        let ts = make_store(&dir);

        let b = ts
            .store
            .create_branch(
                &BranchId::mainline_default(),
                oid("init"),
                "dev",
                &CreateBranchOptions {
                    allow_reserved_labels: true,
                    ..Default::default()
                },
            )
            .unwrap();
        ts.store.add_alias(&b.id, "trunk").unwrap();

        let id = ts.store.resolve_reference("#trunk").unwrap();
        assert_eq!(id.storage_key(), "main@mainline");
    }

    // ------------------------------------------------------------------
    // ObjectStore sharing
    // ------------------------------------------------------------------

    #[test]
    fn test_fsbranchstore_object_store_arc_is_shared() {
        // Verify that the object_store field is an Arc — clone the store
        // and confirm object ids written before cloning are visible from
        // the cloned instance (which shares the same underlying store).
        let dir = tempdir("arc_shared");
        let ts = make_store(&dir);

        let sep = voe_mask::ChunkMask::tagged(ObjectId::new("sep-v1.0"), "v1.0");
        let obj = sep.to_voe_object().unwrap();
        let sep_oid = ts.store.object_store.store(&obj).unwrap();

        // Simulate how FsRepository clones the Arc — a cloned FsBranchStore
        // that got the same Arc will see the written object.
        let shared_store = FsBranchStore::with_backend(
            ts.store.voe_dir().to_path_buf(),
            // re-use the same refs dir / HEAD from the original
            FsRefStore::with_backend(
                ts.store.voe_dir().join("refs"),
                ts.store.voe_dir().join("HEAD"),
                Arc::new(LocalFileBackend::new()),
            ),
            ts.store.object_store.clone(),
            Arc::new(LocalFileBackend::new()),
        );

        let loaded = shared_store.object_store.retrieve(&sep_oid).unwrap();

        // Also verify existence flag matches between the two views.
        assert!(ts.store.object_store.exists(&sep_oid).unwrap());
        assert!(shared_store.object_store.exists(&sep_oid).unwrap());
    }

    // ------------------------------------------------------------------
    // Tagged-mask round-trip (replaces old Separator tests)
    // ------------------------------------------------------------------

    #[test]
    fn test_tagged_mask_roundtrip_through_object_store() {
        use voe_mask::Mask;

        let dir = tempdir("tagged_rt");
        let ts = make_store(&dir);

        let mask_in = voe_mask::ChunkMask::tagged(ObjectId::new("sep-v1.0.0"), "v1.0.0");
        let obj = mask_in.to_voe_object().unwrap();
        let oid = ts.store.object_store.store(&obj).unwrap();

        let loaded = ts.store.object_store.retrieve(&oid).unwrap();
        // Tagged masks are stored as plain ChunkMask objects (ObjectKind::Blob),
        // the tag lives inside the JSON payload.
        assert_eq!(loaded.kind, ObjectKind::Blob);

        let mask_out = voe_mask::ChunkMask::from_voe_object(&loaded).unwrap();
        assert_eq!(mask_out.tag.as_deref(), Some("v1.0.0"));
        assert!(mask_out.changes.is_empty());
        // A tagged-only mask carries no changes, so mask_type falls back
        // to the default (File) — the tag field is what matters.
    }

    #[test]
    fn test_tagged_mask_errors_on_garbage_payload() {
        let dir = tempdir("tagged_bad_payload");
        let ts = make_store(&dir);

        // Store a Blob with non-JSON content — from_voe_object must fail.
        let obj = VoeObject::new(ObjectKind::Blob, b"not-json".to_vec());
        let oid = ts.store.object_store.store(&obj).unwrap();
        let loaded = ts.store.object_store.retrieve(&oid).unwrap();

        let result = voe_mask::ChunkMask::from_voe_object(&loaded);
        assert!(result.is_err());
    }
}
