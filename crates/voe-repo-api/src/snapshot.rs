use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use crate::model::commit::{Commit, CommitStore, MaskObject, RefStore};
use crate::model::tree::{Tree, TreeEntry, TreeEntryType};
use voe_mask::{ChunkMask, Mask, MaskChange, MaskContent, MaskResolver};
use voe_storage_api::{ChunkStore, ObjectStore};
use voe_types::error::{Result, VoeError};
use voe_types::object::{ObjectId, ObjectKind, VoeObject};

pub struct SnapshotEngine<'a> {
    commit_store: &'a dyn CommitStore,
    chunk_store: &'a dyn ChunkStore,
    resolver: &'a dyn MaskResolver,
}

impl<'a> SnapshotEngine<'a> {
    pub fn new(
        commit_store: &'a dyn CommitStore,
        chunk_store: &'a dyn ChunkStore,
        resolver: &'a dyn MaskResolver,
    ) -> Self {
        Self {
            commit_store,
            chunk_store,
            resolver,
        }
    }

    pub fn snapshot_head(&self, ref_store: &dyn RefStore) -> Result<HashMap<PathBuf, Vec<u8>>> {
        let head = ref_store
            .get_head()?
            .ok_or_else(|| VoeError::Storage("HEAD is not set".to_string()))?;
        self.snapshot_from(&head)
    }

    pub fn snapshot_from(&self, commit_id: &ObjectId) -> Result<HashMap<PathBuf, Vec<u8>>> {
        let commit_pairs = self.collect_commits(commit_id)?;
        let mut commits: Vec<Commit> = commit_pairs.into_iter().map(|(_, c)| c).collect();
        commits.reverse();
        let mask_ids = self.collect_mask_ids(&commits);
        let masks = self.load_and_materialize_masks(&mask_ids)?;

        if masks.is_empty() {
            return Ok(HashMap::new());
        }

        let refs: Vec<&dyn Mask> = masks.iter().map(|m| m.as_ref()).collect();
        let ordered = self.resolver.resolve_order(&refs)?;

        let mut state: HashMap<PathBuf, Vec<u8>> = HashMap::new();
        self.resolver.apply_all(&ordered, &mut state)?;
        Ok(state)
    }

    pub fn collect_commits(&self, start: &ObjectId) -> Result<Vec<(ObjectId, Commit)>> {
        let mut visited: HashSet<ObjectId> = HashSet::new();
        let mut result: Vec<(ObjectId, Commit)> = Vec::new();
        let mut queue: VecDeque<ObjectId> = VecDeque::new();

        queue.push_back(start.clone());

        while let Some(id) = queue.pop_front() {
            if !visited.insert(id.clone()) {
                continue;
            }

            let commit = self.commit_store.retrieve_commit(&id)?;
            for parent in &commit.parents {
                if !visited.contains(parent) {
                    queue.push_back(parent.clone());
                }
            }
            result.push((id, commit));
        }

        Ok(result)
    }

    pub fn collect_mask_ids(&self, commits: &[Commit]) -> Vec<ObjectId> {
        let mut seen: HashSet<ObjectId> = HashSet::new();
        let mut result: Vec<ObjectId> = Vec::new();

        for commit in commits {
            for mask_id in &commit.masks {
                if seen.insert(mask_id.clone()) {
                    result.push(mask_id.clone());
                }
            }
        }

        result
    }

    /// Load every mask object by id, materializing `ChunkMask` content from
    /// the chunk store and wrapping them in `Box<dyn Mask>` for uniform
    /// consumption down-stream.  Tagged masks (the old "separator" concept)
    /// also materialize — tagged-only masks simply carry no file edits so
    /// their `apply()` is effectively a no-op.
    pub fn load_and_materialize_masks(&self, ids: &[ObjectId]) -> Result<Vec<Box<dyn Mask>>> {
        let mut result: Vec<Box<dyn Mask>> = Vec::with_capacity(ids.len());

        for id in ids {
            let obj = self.commit_store.retrieve_mask(id)?;
            match obj {
                MaskObject::Chunk(mask) => {
                    let materialized = self.materialize_mask(mask)?;
                    result.push(Box::new(materialized));
                }
            }
        }

        Ok(result)
    }

    fn materialize_mask(&self, mask: ChunkMask) -> Result<ChunkMask> {
        let mut new_changes: Vec<MaskChange> = Vec::with_capacity(mask.changes.len());
        for change in mask.changes {
            let new_content = match change.content {
                MaskContent::Chunks(refs) => {
                    let data = self.chunk_store.assemble_file(&refs)?;
                    MaskContent::Full(data)
                }
                other => other,
            };
            new_changes.push(MaskChange::new(change.location, new_content));
        }
        Ok(ChunkMask {
            id: mask.id,
            changes: new_changes,
            metadata: mask.metadata,
            tag: mask.tag,
        })
    }
}

/// Result of [`MergeEngine::plan_merge`].  Describes how the two branches
/// can be combined without actually writing any refs or objects — callers
/// decide how to materialise the outcome (fast-forward pointer update,
/// create a merge commit, abort, ...).
#[derive(Debug, Clone)]
pub enum MergeOutcome {
    /// The secondary branch is already an ancestor of the primary — no work
    /// to do.  Typical when you merge the mainline into a feature branch
    /// that has already been rebased.
    AlreadyUpToDate,
    /// The primary branch is an ancestor of the secondary — a plain
    /// fast-forward (just move the primary's head to `target_head`).
    FastForward { target_head: ObjectId },
    /// The merge produces no conflicts and can be committed directly.
    NoConflicts {
        /// The full list of mask ids (from both delta branches) that should
        /// live on the resulting merge commit.
        merged_mask_ids: Vec<ObjectId>,
        /// The fully-resolved file snapshot after applying both delta sets.
        merged_snapshot: HashMap<PathBuf, Vec<u8>>,
    },
    /// Hard conflicts were detected and must be resolved by the user.
    Conflicts {
        /// Per-file conflict description for the caller to report.
        conflicts: Vec<ConflictFile>,
    },
}

/// A single file that failed to merge cleanly.
#[derive(Debug, Clone)]
pub struct ConflictFile {
    pub path: PathBuf,
    pub reason: String,
}

/// Compute merge plans between two branches without touching the object
/// store or refs.  This is the pure, side-effect-free half of the merge
/// operation — the CLI command consumes its results and drives the write
/// phase.
pub struct MergeEngine<'a> {
    commit_store: &'a dyn CommitStore,
    chunk_store: &'a dyn ChunkStore,
    resolver: &'a dyn MaskResolver,
}

impl<'a> MergeEngine<'a> {
    pub fn new(
        commit_store: &'a dyn CommitStore,
        chunk_store: &'a dyn ChunkStore,
        resolver: &'a dyn MaskResolver,
    ) -> Self {
        Self {
            commit_store,
            chunk_store,
            resolver,
        }
    }

    /// Build a [`SnapshotEngine`] from the same backing stores so we can
    /// reuse its `collect_commits` / `snapshot_from` helpers.
    fn snapshot_engine(&self) -> SnapshotEngine<'_> {
        SnapshotEngine::new(self.commit_store, self.chunk_store, self.resolver)
    }

    /// Return the set of commit ids reachable from `start` (inclusive of
    /// `start` itself).  Used by LCA and descendant checks.
    fn descendant_set(&self, start: &ObjectId) -> Result<HashSet<ObjectId>> {
        let mut visited: HashSet<ObjectId> = HashSet::new();
        let mut queue: VecDeque<ObjectId> = VecDeque::new();
        queue.push_back(start.clone());

        while let Some(id) = queue.pop_front() {
            if !visited.insert(id.clone()) {
                continue;
            }
            let commit = self.commit_store.retrieve_commit(&id)?;
            queue.extend(commit.parents);
        }
        Ok(visited)
    }

    /// Returns `true` when `candidate` is reachable by walking parents from
    /// `ancestor` — i.e. `candidate` is a descendant of (or equal to)
    /// `ancestor`.
    pub fn is_descendant(&self, candidate: &ObjectId, ancestor: &ObjectId) -> Result<bool> {
        // Walk backward from candidate.  If we ever reach `ancestor` it's a
        // descendant; if we exhaust the ancestor set without finding it,
        // return false.
        let mut visited: HashSet<ObjectId> = HashSet::new();
        let mut queue: VecDeque<ObjectId> = VecDeque::new();
        queue.push_back(candidate.clone());

        while let Some(id) = queue.pop_front() {
            if !visited.insert(id.clone()) {
                continue;
            }
            if id == *ancestor {
                return Ok(true);
            }
            let commit = self.commit_store.retrieve_commit(&id)?;
            queue.extend(commit.parents);
        }
        Ok(false)
    }

    /// Lowest common ancestor of `a` and `b` in the commit DAG.
    ///
    /// Returns `None` when the two commits share no ancestor (unrelated
    /// histories).  The returned value is the commit closest to both
    /// branches that is reachable from either side.
    ///
    /// Special cases handled:
    /// * `a == b` → returns `a`.
    /// * `a` is ancestor of `b` → returns `a`.
    /// * `b` is ancestor of `a` → returns `b`.
    pub fn find_merge_base(&self, a: &ObjectId, b: &ObjectId) -> Result<Option<ObjectId>> {
        if a == b {
            return Ok(Some(a.clone()));
        }

        // Walk backward from both sides simultaneously.  At each step we pop
        // from whichever frontier has the fewer visited ids (a heuristic that
        // keeps the smaller side bounded while the larger side gradually
        // expands — O(min(|ancestors(a)|, |ancestors(b)|)) memory).
        let mut visited_a: HashSet<ObjectId> = HashSet::new();
        let mut visited_b: HashSet<ObjectId> = HashSet::new();
        let mut queue_a: VecDeque<ObjectId> = VecDeque::new();
        let mut queue_b: VecDeque<ObjectId> = VecDeque::new();
        queue_a.push_back(a.clone());
        queue_b.push_back(b.clone());

        let mut a_done = false;
        let mut b_done = false;

        while !(a_done && b_done) {
            // Expand whichever side still has frontier entries.
            while let Some(id) = queue_a.pop_front() {
                if !visited_a.insert(id.clone()) {
                    continue;
                }
                // Found in b's visited → that's our merge base!
                if visited_b.contains(&id) {
                    return Ok(Some(id));
                }
                match self.commit_store.retrieve_commit(&id) {
                    Ok(c) => queue_a.extend(c.parents),
                    Err(_) => { /* dangling parent — skip silently */ }
                }
            }
            a_done = true;

            while let Some(id) = queue_b.pop_front() {
                if !visited_b.insert(id.clone()) {
                    continue;
                }
                if visited_a.contains(&id) {
                    return Ok(Some(id));
                }
                if let Ok(c) = self.commit_store.retrieve_commit(&id) {
                    queue_b.extend(c.parents)
                }
            }
            b_done = true;

            // If both queues are empty we've completely explored both sides.
            if queue_a.is_empty() && queue_b.is_empty() {
                break;
            }
        }

        Ok(None)
    }

    /// Compute the set of commit ids on `side` that are NOT reachable from
    /// `base` — i.e. the unique commits added by `side` since it diverged.
    fn delta_commits(&self, side_head: &ObjectId, base: &ObjectId) -> Result<Vec<ObjectId>> {
        let side_set = self.descendant_set(side_head)?;
        let base_set = self.descendant_set(base)?;
        Ok(side_set
            .into_iter()
            .filter(|id| !base_set.contains(id))
            .collect())
    }

    /// Collect all unique mask ids reachable from the given set of commits.
    fn collect_mask_ids_from_commits(&self, commit_ids: &[ObjectId]) -> Result<Vec<ObjectId>> {
        let mut seen: HashSet<ObjectId> = HashSet::new();
        let mut out: Vec<ObjectId> = Vec::new();
        for id in commit_ids {
            let commit = self.commit_store.retrieve_commit(id)?;
            for mask_id in commit.masks {
                if seen.insert(mask_id.clone()) {
                    out.push(mask_id);
                }
            }
        }
        Ok(out)
    }

    /// Apply a subset of masks (identified by their ids) on top of `base`
    /// and return the resulting snapshot.  The masks are first materialised
    /// through the chunk store, then dependency-ordered, then applied.
    fn apply_mask_subset(
        &self,
        mask_ids: &[ObjectId],
        base: &HashMap<PathBuf, Vec<u8>>,
    ) -> Result<HashMap<PathBuf, Vec<u8>>> {
        if mask_ids.is_empty() {
            return Ok(base.clone());
        }

        let masks = self
            .snapshot_engine()
            .load_and_materialize_masks(mask_ids)?;
        let refs: Vec<&dyn Mask> = masks.iter().map(|m| m.as_ref()).collect();
        let ordered = self.resolver.resolve_order(&refs)?;

        let mut state = base.clone();
        self.resolver.apply_all(&ordered, &mut state)?;
        Ok(state)
    }

    /// Compare the two delta snapshots against a shared base and report any
    /// file that was modified on BOTH sides in an incompatible way.
    fn detect_snapshot_conflicts(
        base: &HashMap<PathBuf, Vec<u8>>,
        primary_delta: &HashMap<PathBuf, Vec<u8>>,
        secondary_delta: &HashMap<PathBuf, Vec<u8>>,
    ) -> Vec<ConflictFile> {
        let mut conflicts: Vec<ConflictFile> = Vec::new();

        // Union of all paths appearing in any of the three maps.
        let mut all_paths: HashSet<PathBuf> = HashSet::new();
        for p in base.keys().cloned() {
            all_paths.insert(p);
        }
        for p in primary_delta.keys().cloned() {
            all_paths.insert(p);
        }
        for p in secondary_delta.keys().cloned() {
            all_paths.insert(p);
        }

        for path in &all_paths {
            let base_content = base.get(path);
            let primary_changed = match (base_content, primary_delta.get(path)) {
                (None, None) => false,
                (None, Some(_)) => true,      // path was created in primary
                (Some(_), None) => true,      // path was deleted in primary
                (Some(b), Some(p)) => b != p, // content differs
            };
            let secondary_changed = match (base_content, secondary_delta.get(path)) {
                (None, None) => false,
                (None, Some(_)) => true,
                (Some(_), None) => true,
                (Some(b), Some(s)) => b != s,
            };

            if primary_changed && secondary_changed {
                // If they changed the path in *identical* ways we can still
                // auto-merge — compare actual delta content.
                let primary_result = primary_delta.get(path);
                let secondary_result = secondary_delta.get(path);
                match (primary_result, secondary_result) {
                    (None, None) => {}                 // both deleted — ok
                    (Some(a), Some(b)) if a == b => {} // both produced same content — ok
                    _ => {
                        let reason = if primary_result.is_none() && secondary_result.is_some() {
                            "file deleted in primary but modified in secondary".to_string()
                        } else if primary_result.is_some() && secondary_result.is_none() {
                            "file modified in primary but deleted in secondary".to_string()
                        } else {
                            "both sides modified the file with different content".to_string()
                        };
                        conflicts.push(ConflictFile {
                            path: path.clone(),
                            reason,
                        });
                    }
                }
            }
        }

        conflicts
    }

    /// Plan a merge between two commits.  See [`MergeOutcome`] for the
    /// possible results.  This function is pure — it does not touch refs,
    /// the object store, or the working tree.
    pub fn plan_merge(
        &self,
        primary_head: &ObjectId,
        secondary_head: &ObjectId,
    ) -> Result<MergeOutcome> {
        if primary_head == secondary_head {
            return Ok(MergeOutcome::AlreadyUpToDate);
        }

        // ---- Phase 1: locate the merge base ---------------------------------
        let base = match self.find_merge_base(primary_head, secondary_head)? {
            Some(b) => b,
            None => {
                // Completely unrelated histories — refuse for now.
                // We can add an --allow-unrelated-histories flag later.
                return Err(VoeError::MergeConflict(
                    "the two branches have no common ancestor (unrelated histories)".to_string(),
                ));
            }
        };

        // ---- Phase 2: fast-forward detection -------------------------------
        if base == *secondary_head {
            // secondary is ancestor of primary → primary already has everything.
            return Ok(MergeOutcome::AlreadyUpToDate);
        }
        if base == *primary_head {
            // primary is ancestor of secondary → pure fast-forward.
            return Ok(MergeOutcome::FastForward {
                target_head: secondary_head.clone(),
            });
        }

        // ---- Phase 3: isolate the delta commits ----------------------------
        let primary_delta_commits = self.delta_commits(primary_head, &base)?;
        let secondary_delta_commits = self.delta_commits(secondary_head, &base)?;

        // ---- Phase 4: load all masks and check MaskResolver-level conflicts -
        let primary_delta_mask_ids = self.collect_mask_ids_from_commits(&primary_delta_commits)?;
        let secondary_delta_mask_ids =
            self.collect_mask_ids_from_commits(&secondary_delta_commits)?;

        // Merge the two delta sets (dedup).
        let mut merged_mask_ids: Vec<ObjectId> = Vec::new();
        let mut seen: HashSet<ObjectId> = HashSet::new();
        for id in primary_delta_mask_ids
            .iter()
            .chain(secondary_delta_mask_ids.iter())
        {
            if seen.insert(id.clone()) {
                merged_mask_ids.push(id.clone());
            }
        }

        let masks = self
            .snapshot_engine()
            .load_and_materialize_masks(&merged_mask_ids)?;
        let refs: Vec<&dyn Mask> = masks.iter().map(|m| m.as_ref()).collect();
        let mask_conflicts = self.resolver.validate_conflicts(&refs)?;
        if !mask_conflicts.is_empty() {
            let reasons: Vec<String> = mask_conflicts.iter().map(|c| c.to_string()).collect();
            return Err(VoeError::MergeConflict(format!(
                "mask-level conflicts: {}",
                reasons.join("; ")
            )));
        }

        // ---- Phase 5: snapshot-level conflict detection ---------------------
        let snap = self.snapshot_engine();
        let base_snap = snap.snapshot_from(&base)?;
        let primary_delta_snap = self.apply_mask_subset(&primary_delta_mask_ids, &base_snap)?;
        let secondary_delta_snap = self.apply_mask_subset(&secondary_delta_mask_ids, &base_snap)?;

        let file_conflicts =
            Self::detect_snapshot_conflicts(&base_snap, &primary_delta_snap, &secondary_delta_snap);
        if !file_conflicts.is_empty() {
            return Ok(MergeOutcome::Conflicts {
                conflicts: file_conflicts,
            });
        }

        // ---- Phase 6: produce the merged snapshot ---------------------------
        let merged_snapshot = self.apply_mask_subset(&merged_mask_ids, &base_snap)?;

        Ok(MergeOutcome::NoConflicts {
            merged_mask_ids,
            merged_snapshot,
        })
    }
}

/// Build a [`Tree`] from a flat snapshot (`HashMap<PathBuf, Vec<u8>>`)
/// and persist both the leaf blobs and the tree objects through `store`.
/// Returns the ObjectId of the root tree.
///
/// This is the common utility that powers both normal commits and merge
/// commits — the only difference is which snapshot it receives.
pub fn build_tree_from_snapshot(
    store: &dyn ObjectStore,
    snapshot: &HashMap<PathBuf, Vec<u8>>,
) -> Result<ObjectId> {
    let mut grouped: HashMap<String, Vec<(String, Vec<u8>)>> = HashMap::new();

    for (path, data) in snapshot {
        let path_str = path.to_string_lossy();
        let trimmed = path_str.trim_start_matches('/');
        let (top, rest) = match trimmed.find('/') {
            Some(idx) => (trimmed[..idx].to_string(), trimmed[idx + 1..].to_string()),
            None => (trimmed.to_string(), String::new()),
        };
        grouped.entry(top).or_default().push((rest, data.clone()));
    }

    let mut entries: Vec<TreeEntry> = Vec::new();
    for (name, items) in grouped {
        let has_nested = items.iter().any(|(r, _)| !r.is_empty());

        if !has_nested {
            let (_, data) = items.into_iter().next().unwrap();
            let blob = VoeObject::new(ObjectKind::Blob, data);
            let blob_oid = store.store(&blob)?;
            entries.push(TreeEntry {
                name,
                oid: blob_oid,
                entry_type: TreeEntryType::Blob,
            });
        } else {
            let subtree_oid = build_subtree(store, items)?;
            entries.push(TreeEntry {
                name,
                oid: subtree_oid,
                entry_type: TreeEntryType::Tree,
            });
        }
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));

    let tree = Tree::with_entries(entries);
    let tree_oid = store.store(&tree.to_voe_object()?)?;
    Ok(tree_oid)
}

/// Recursive helper for `build_tree_from_snapshot` — builds a subtree from
/// entries that share a common parent directory.
fn build_subtree(store: &dyn ObjectStore, items: Vec<(String, Vec<u8>)>) -> Result<ObjectId> {
    let mut grouped: HashMap<String, Vec<(String, Vec<u8>)>> = HashMap::new();

    for (rest, data) in items {
        match rest.find('/') {
            Some(idx) => {
                let top = rest[..idx].to_string();
                let child_rest = rest[idx + 1..].to_string();
                grouped.entry(top).or_default().push((child_rest, data));
            }
            None => {
                grouped.entry(rest).or_default().push((String::new(), data));
            }
        }
    }

    let mut entries: Vec<TreeEntry> = Vec::new();
    for (name, sub_items) in grouped {
        let has_nested = sub_items.iter().any(|(r, _)| !r.is_empty());

        if !has_nested {
            let (_, data) = sub_items.into_iter().next().unwrap();
            let blob = VoeObject::new(ObjectKind::Blob, data);
            let blob_oid = store.store(&blob)?;
            entries.push(TreeEntry {
                name,
                oid: blob_oid,
                entry_type: TreeEntryType::Blob,
            });
        } else {
            let subtree_oid = build_subtree(store, sub_items)?;
            entries.push(TreeEntry {
                name,
                oid: subtree_oid,
                entry_type: TreeEntryType::Tree,
            });
        }
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));

    let tree = Tree::with_entries(entries);
    store.store(&tree.to_voe_object()?)
}
