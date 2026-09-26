use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use crate::config::ConfigManager;
use crate::model::branch::BranchId;
use crate::model::branch_store::BranchStore;
use crate::model::commit::{CommitStore, IndexStore, MaskObject, RefStore};
use voe_mask::ChunkMask;
use voe_storage_api::{ChunkStore, ObjectStore};
use voe_types::error::{Result, VoeError};
use voe_types::object::{ObjectId, ObjectKind, VoeObject};

type Snapshot = HashMap<PathBuf, Vec<u8>>;

pub trait Repository: Send + Sync {
    fn path(&self) -> &PathBuf;
    fn object_store(&self) -> &dyn ObjectStore;
    fn chunk_store(&self) -> &dyn ChunkStore;
    fn commit_store(&self) -> &dyn CommitStore;
    fn ref_store(&self) -> &dyn RefStore;
    /// High-level branch management — see [`BranchStore`] for full semantics.
    fn branch_store(&self) -> &dyn BranchStore;
    fn index_store(&self) -> &dyn IndexStore;
    fn config_manager(&self) -> &dyn ConfigManager;
    fn config_manager_mut(&mut self) -> &mut dyn ConfigManager;

    /// Scan the repository's working tree and return every visible file as
    /// a `snapshot_key → bytes` map, skipping dotfiles and the `.voe/` dir.
    fn read_working_tree(&self) -> Result<Snapshot>;

    /// Materialise `snapshot` to disk under the repository's working tree,
    /// creating files that don't exist, overwriting files that differ, and
    /// deleting visible files not present in the snapshot.
    fn write_snapshot_to_disk(&self, snapshot: &Snapshot) -> Result<()>;

    fn store_object(&self, kind: ObjectKind, content: Vec<u8>) -> Result<ObjectId> {
        let object = VoeObject::new(kind, content);
        self.object_store().store(&object)
    }

    fn retrieve_object(&self, id: &ObjectId) -> Result<VoeObject> {
        self.object_store().retrieve(id)
    }

    /// Set the head of a release branch (rule 4) after validating that the
    /// new head is a descendant of the current head.  Walks the commit
    /// parent graph via `commit_store()` and refuses to rewrite release
    /// history — returns `ReleaseAppendViolation` when the new head cannot
    /// reach the old head or when `branch_id` is not a release branch.
    ///
    /// Also enforces that no tagged masks (the old "separator" concept) are
    /// lost along the way: every tagged-mask ObjectId that appears anywhere
    /// in the old-head ancestry must also appear somewhere in the new-head
    /// ancestry.  Tagged masks are append-only markers on release branches
    /// — they may be added but never removed or reordered.
    fn set_release_branch_head(&self, branch_id: &BranchId, new_head: ObjectId) -> Result<()> {
        if !branch_id.is_release() {
            return Err(VoeError::ReleaseAppendViolation);
        }

        let branch = self
            .branch_store()
            .get_branch(branch_id)?
            .ok_or_else(|| VoeError::BranchNotFound(branch_id.storage_key()))?;

        if branch.head == new_head {
            return Ok(());
        }

        let cs = self.commit_store();

        // Phase 1 — ancestry check: walk the parent chain starting from
        // `new_head`.  If we ever reach `branch.head`, the update is
        // append-only (safe).  If we exhaust the ancestor set without
        // finding it, it's a rebase/rewrite and we refuse.
        let mut visited: HashSet<ObjectId> = HashSet::new();
        let mut frontier: Vec<ObjectId> = vec![new_head.clone()];
        while let Some(id) = frontier.pop() {
            if !visited.insert(id.clone()) {
                continue;
            }
            if id == branch.head {
                // Phase 2 — tagged-mask integrity check: collect every
                // tagged-mask ObjectId reachable from the old head and from
                // the new head; the new set must be a superset of the old.
                let old_tagged = self.collect_tagged_masks(&branch.head)?;
                let new_tagged = self.collect_tagged_masks(&new_head)?;
                if !old_tagged.is_subset(&new_tagged) {
                    return Err(VoeError::ReleaseAppendViolation);
                }

                return self.branch_store().set_branch_head(branch_id, new_head);
            }
            match cs.retrieve_commit(&id) {
                Ok(commit) => frontier.extend(commit.parents),
                Err(_) => {
                    // Dangling commit object — cannot verify ancestry; refuse
                    // the move rather than silently allowing a rewrite.
                    return Err(VoeError::ReleaseAppendViolation);
                }
            }
        }

        Err(VoeError::ReleaseAppendViolation)
    }

    /// Walk the ancestor chain of `start` and return the set of every mask
    /// ObjectId whose mask carries a non-None tag.  Used by
    /// [`Self::set_release_branch_head`] to verify that tagged (version-
    /// boundary) masks are never removed from a release branch.
    fn collect_tagged_masks(&self, start: &ObjectId) -> Result<HashSet<ObjectId>> {
        let cs = self.commit_store();
        let mut tagged: HashSet<ObjectId> = HashSet::new();
        let mut visited_commits: HashSet<ObjectId> = HashSet::new();
        let mut queue: VecDeque<ObjectId> = VecDeque::new();
        queue.push_back(start.clone());

        while let Some(commit_id) = queue.pop_front() {
            if !visited_commits.insert(commit_id.clone()) {
                continue;
            }
            let commit = match cs.retrieve_commit(&commit_id) {
                Ok(c) => c,
                Err(_) => continue,
            };
            for mask_id in &commit.masks {
                if let Ok(MaskObject::Chunk(mask)) = cs.retrieve_mask(mask_id) {
                    if mask.tag.is_some() {
                        tagged.insert(mask_id.clone());
                    }
                }
            }
            queue.extend(commit.parents.iter().cloned());
        }

        Ok(tagged)
    }

    /// Store a tagged-only mask (the new replacement for "separator") as a
    /// VOE object and return its id.  Tagged masks sit alongside regular
    /// masks in a commit's `masks` Vec — callers distinguish them via
    /// `ChunkMask::tag()`.
    fn create_tagged_mask(&self, mask: &ChunkMask) -> Result<ObjectId> {
        if mask.tag.is_none() {
            return Err(VoeError::Other(
                "create_tagged_mask requires a mask with a non-None tag".to_string(),
            ));
        }
        self.commit_store().store_mask(mask)
    }
}

pub trait RepoManager: Send + Sync {
    fn init(&self, path: PathBuf) -> Result<Box<dyn Repository>>;
    fn open(&self, path: PathBuf) -> Result<Box<dyn Repository>>;
    fn try_open_or_init(&self, path: PathBuf) -> Result<Box<dyn Repository>>;
    fn find_root(&self, start: &std::path::Path) -> Option<PathBuf>;
}
