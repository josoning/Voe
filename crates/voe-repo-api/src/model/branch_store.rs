//! High-level branch management trait.
//!
//! Unlike the thin [`super::refs::RefStore`] (flat `name -> ObjectId` map),
//! `BranchStore` understands the full VOE branch model: names, labels,
//! reserved keywords, mainline uniqueness, sub-branch inheritance, aliases,
//! release append-only policy, detached HEAD → auto-temp-branch, and merge
//! node creation.  Concrete implementations live in `voe-fs`.
//!
//! `BranchStore` extends `RefStore` so implementations can reuse the existing
//! ref filesytem layout for commit pointers while adding their own metadata
//! (creator, aliases, parent branch, ...).

use voe_types::error::{Result, VoeError};

use super::branch::{Alias, Branch, BranchId, MergeNode, DEFAULT_LABEL, RESERVED_LABELS};
use super::commit::RefStore;
use voe_types::author::current_timestamp;
use voe_types::object::ObjectId;

/// Rejects creating a branch carrying any reserved label unless the caller
/// explicitly opted in (used by `create_branch` and `create_sub_branch`
/// implementations).  This is a free function so it can be invoked from
/// default trait methods without `Self: Sized`.
pub fn validate_reserved_labels(id: &BranchId, allow_reserved: bool, context: &str) -> Result<()> {
    if !allow_reserved {
        for lbl in &id.labels {
            if RESERVED_LABELS.iter().any(|r| lbl.eq_ignore_ascii_case(r)) {
                return Err(VoeError::ReservedKeyword {
                    keyword: lbl.clone(),
                    context: context.to_string(),
                });
            }
        }
    }
    Ok(())
}

/// Flags passed to [`BranchStore::create_branch`].
#[derive(Debug, Clone, Default)]
pub struct CreateBranchOptions {
    /// When `true` the caller is allowed to create branches whose labels
    /// include reserved keywords (`mainline`, `release`, `temp`).  This flag
    /// should only be set by designated VOE commands (rules 1-2).
    pub allow_reserved_labels: bool,
    /// When `true` and the target branch identifier already exists, the call
    /// succeeds as a no-op instead of returning `BranchAlreadyExists`.
    pub if_not_exists: bool,
}

/// Flags passed to [`BranchStore::create_sub_branch`] — rule 8 (sub-branch
/// label inheritance) and rule 2 (reserved-label gate).
#[derive(Debug, Clone, Default)]
pub struct CreateSubBranchOptions {
    /// When `true` the new sub-branch inherits **all** non-reserved labels
    /// from its parent (excluding `mainline` — rule 8 explicitly forbids
    /// inheriting the mainline label).
    pub inherit_parent_labels: bool,
    /// When `true` the caller may add reserved labels to the sub-branch.
    pub allow_reserved_labels: bool,
    /// When `true` and the target already exists → no-op, no error.
    pub if_not_exists: bool,
}

/// Flags passed to [`BranchStore::merge_branches`].
#[derive(Debug, Clone, Default)]
pub struct MergeOptions {
    /// Optional human-readable message stored on the resulting `MergeNode`.
    pub message: Option<String>,
    /// Optional patch masks that conflict resolution introduced (rule 14).
    pub patch_mask_ids: Vec<ObjectId>,
}

/// Repository-wide branch management interface.  `BranchStore` extends
/// [`RefStore`] so implementations can reuse the existing flat-ref
/// infrastructure for commit pointers while layering on the richer VOE
/// branch model (creator, aliases, parent branch, ...).
pub trait BranchStore: RefStore {
    // ------------------------------------------------------------------
    // Single-branch CRUD
    // ------------------------------------------------------------------

    /// Create a new branch pointing at `head`.
    ///
    /// # Errors
    /// * `InvalidBranchId` — the identifier fails parsing / validation.
    /// * `ReservedKeyword` — the identifier carries a reserved label but
    ///   `options.allow_reserved_labels` is `false` (rule 2).
    /// * `MainlineNotUnique` — another branch already carries `mainline` and
    ///   this one tries to too (rule 9).
    /// * `BranchAlreadyExists` — a branch with the same id already exists
    ///   (skipped when `if_not_exists` is `true`).
    fn create_branch(
        &self,
        id: &BranchId,
        head: ObjectId,
        creator_device_id: &str,
        options: &CreateBranchOptions,
    ) -> Result<Branch>;

    /// Retrieve a branch by its storage key (canonical `name@labels` form).
    fn get_branch_by_key(&self, storage_key: &str) -> Result<Option<Branch>>;

    /// Retrieve a branch by `BranchId`.  Equivalent to looking up by
    /// `storage_key()` — label order is irrelevant (rule 3).
    fn get_branch(&self, id: &BranchId) -> Result<Option<Branch>> {
        self.get_branch_by_key(&id.storage_key())
    }

    /// List **all** branches in the repository, sorted by storage key.
    fn list_branches(&self) -> Result<Vec<Branch>>;

    /// List all branches created by `creator_device_id` (rule 12).
    fn list_branches_by_creator(&self, creator_device_id: &str) -> Result<Vec<Branch>> {
        let mut out = Vec::new();
        for b in self.list_branches()? {
            if b.creator_device_id == creator_device_id {
                out.push(b);
            }
        }
        Ok(out)
    }

    /// Delete a branch.  Refuses to delete the branch currently checked out;
    /// callers must switch first.
    fn delete_branch(&self, id: &BranchId) -> Result<()>;

    /// Update the `head` commit of an existing branch.  On `@release`
    /// branches this enforces append-only semantics — the new commit **must**
    /// have the current head as a direct parent, otherwise a
    /// `ReleaseAppendViolation` is returned (rule 4).
    fn set_branch_head(&self, id: &BranchId, new_head: ObjectId) -> Result<()>;

    /// Update only the `last_updated` timestamp of `id` to now.
    fn touch_branch(&self, id: &BranchId) -> Result<()>;

    // ------------------------------------------------------------------
    // Rename / re-label
    // ------------------------------------------------------------------

    /// Rename a branch — change only its `name` portion, labels stay the
    /// same.  The new name must pass [`BranchId::validate_name`].
    ///
    /// Every piece of persistent state that keys off `storage_key()` (the
    /// ref file, the metadata JSON, every alias that pointed at the old
    /// key, and HEAD_BRANCH when this was the current branch) is migrated
    /// atomically to the new key.
    ///
    /// # Errors
    /// * `InvalidBranchName` — the supplied name fails validation.
    /// * `BranchAlreadyExists` — a branch with the resulting id already
    ///   exists.
    /// * `BranchNotFound` — `id` does not exist.
    fn rename_branch(&self, id: &BranchId, new_name: &str) -> Result<Branch> {
        let branch = self
            .get_branch(id)?
            .ok_or_else(|| VoeError::BranchNotFound(id.storage_key()))?;

        let mut new_id = branch.id.clone();
        new_id.name = new_name.to_string();
        new_id.validate_name()?;

        // Refuse to collide with an existing branch.
        if self.get_branch(&new_id)?.is_some() {
            return Err(VoeError::BranchAlreadyExists(new_id.storage_key()));
        }

        self.migrate_branch_id(&branch.id, &new_id)?;
        let mut updated = branch;
        updated.id = new_id;
        updated.last_updated = current_timestamp();
        self.store_branch_metadata(&updated)?;
        Ok(updated)
    }

    /// Add a label to a branch.  `label` must be non-empty and must not
    /// conflict with an existing label on the same branch (case-
    /// insensitive comparison).
    ///
    /// When the label being added is **not** `default` and the branch
    /// currently carries only the implicit `default` label, `default` is
    /// automatically removed so the identity stays clean (e.g. adding
    /// `feature` to `main@default` yields `main@feature`, not
    /// `main@default@feature`).
    ///
    /// Adding the `mainline` label is allowed only when no other branch
    /// already carries it (rule 9).
    ///
    /// Returns the updated branch.
    fn add_label(&self, id: &BranchId, label: &str) -> Result<Branch> {
        let branch = self
            .get_branch(id)?
            .ok_or_else(|| VoeError::BranchNotFound(id.storage_key()))?;

        // Build a one-label-only BranchId just to reuse validation logic.
        let probe = BranchId {
            name: branch.id.name.clone(),
            labels: vec![label.to_string()],
        };
        probe.validate_labels()?;

        // mainline uniqueness gate.
        if label.eq_ignore_ascii_case("mainline") && self.get_mainline()?.is_some() {
            return Err(VoeError::MainlineNotUnique);
        }

        let mut new_labels = branch.id.labels.clone();
        let label_ci = label.to_lowercase();
        if new_labels.iter().any(|l| l.to_lowercase() == label_ci) {
            // Already present — nothing to do, but still return the current
            // branch so callers can chain safely.
            return Ok(branch);
        }

        // If we're adding a non-default label and the only existing label
        // is `default`, drop `default` so the branch id stays canonical.
        let non_default_added = !label.eq_ignore_ascii_case(DEFAULT_LABEL);
        if non_default_added
            && new_labels.len() == 1
            && new_labels[0].eq_ignore_ascii_case(DEFAULT_LABEL)
        {
            new_labels.clear();
        }

        new_labels.push(label.to_string());

        let mut new_id = branch.id.clone();
        new_id.labels = new_labels;

        // Refuse collisions.
        if self.get_branch(&new_id)?.is_some() {
            return Err(VoeError::BranchAlreadyExists(new_id.storage_key()));
        }

        self.migrate_branch_id(&branch.id, &new_id)?;
        let mut updated = branch;
        updated.id = new_id;
        updated.last_updated = current_timestamp();
        self.store_branch_metadata(&updated)?;
        Ok(updated)
    }

    /// Remove a label from a branch.  The `default` label may not be
    /// explicitly removed — it is only dropped when another label exists
    /// to take its place (see [`Self::add_label`]) or implicitly when the
    /// branch is re-created.
    ///
    /// If removing `label` would leave the branch with zero labels, the
    /// call is rejected (every branch must carry at least one label).
    /// Instead, the caller should rename the branch or ensure another
    /// non-default label is present first.
    ///
    /// Returns the updated branch.
    fn remove_label(&self, id: &BranchId, label: &str) -> Result<Branch> {
        let branch = self
            .get_branch(id)?
            .ok_or_else(|| VoeError::BranchNotFound(id.storage_key()))?;

        if label.eq_ignore_ascii_case(DEFAULT_LABEL) {
            return Err(VoeError::Other(
                "the implicit 'default' label cannot be removed — \
                 add another label first (e.g. `voe branch <name> --add-label feature`)"
                    .to_string(),
            ));
        }

        let new_labels: Vec<String> = branch
            .id
            .labels
            .iter()
            .filter(|l| !l.eq_ignore_ascii_case(label))
            .cloned()
            .collect();

        if new_labels.len() == branch.id.labels.len() {
            // Nothing was removed — label wasn't present.  Return as-is.
            return Ok(branch);
        }

        // Refuse to leave the branch with zero labels.
        if new_labels.is_empty() {
            return Err(VoeError::Other(format!(
                "cannot remove last label '@{}' — a branch must carry at least one label",
                label
            )));
        }

        let mut new_id = branch.id.clone();
        new_id.labels = new_labels;

        if self.get_branch(&new_id)?.is_some() {
            return Err(VoeError::BranchAlreadyExists(new_id.storage_key()));
        }

        self.migrate_branch_id(&branch.id, &new_id)?;
        let mut updated = branch;
        updated.id = new_id;
        updated.last_updated = current_timestamp();
        self.store_branch_metadata(&updated)?;
        Ok(updated)
    }

    /// Low-level helper that atomically migrates every piece of persistent
    /// state keyed by `old_id.storage_key()` to `new_id.storage_key()`.
    ///
    /// The default implementation:
    /// 1. Re-points every alias that targets the old key to the new key.
    /// 2. Moves the HEAD_BRANCH pointer when `old_id` was the current
    ///    branch.
    /// 3. Moves the ref file and the metadata JSON.
    ///
    /// Concrete stores may override this to batch the operations
    /// transactionally (e.g. on top of a real filesystem txn).
    fn migrate_branch_id(&self, old_id: &BranchId, new_id: &BranchId) -> Result<()>;

    // ------------------------------------------------------------------
    // Sub-branch (rule 8)
    // ------------------------------------------------------------------

    /// Create a sub-branch of `parent`.  The parent must already exist.
    /// If `options.inherit_parent_labels` is set, every label on the parent
    /// except `mainline` (rule 8) is copied onto the new sub-branch.
    fn create_sub_branch(
        &self,
        parent_id: &BranchId,
        sub_name: &str,
        additional_labels: &[String],
        head: ObjectId,
        creator_device_id: &str,
        options: &CreateSubBranchOptions,
    ) -> Result<Branch> {
        let parent = self
            .get_branch(parent_id)?
            .ok_or_else(|| VoeError::ParentBranchNotFound(parent_id.storage_key()))?;

        // Build sub-branch labels.
        let mut labels: Vec<String> = Vec::new();
        if options.inherit_parent_labels {
            for lbl in &parent.id.labels {
                if lbl.eq_ignore_ascii_case("mainline") {
                    continue;
                }
                if RESERVED_LABELS.iter().any(|r| lbl.eq_ignore_ascii_case(r)) {
                    continue;
                }
                labels.push(lbl.clone());
            }
        }
        for lbl in additional_labels {
            labels.push(lbl.clone());
        }
        if labels.is_empty() {
            labels.push(DEFAULT_LABEL.to_string());
        }

        let sub_id = BranchId {
            name: sub_name.to_string(),
            labels,
        };

        // Gate reserved labels.
        validate_reserved_labels(
            &sub_id,
            options.allow_reserved_labels,
            "sub-branch creation",
        )?;

        // Refuse duplicates unless `if_not_exists`.
        if !options.if_not_exists && self.get_branch(&sub_id)?.is_some() {
            return Err(VoeError::BranchAlreadyExists(sub_id.storage_key()));
        }

        // mainline uniqueness gate.
        if sub_id.has_mainline() && self.get_mainline()?.is_some() {
            return Err(VoeError::MainlineNotUnique);
        }

        let ts = current_timestamp();
        let storage_key = sub_id.storage_key();
        self.set_ref(&storage_key, &head)?;
        let mut branch = Branch::new(sub_id, head, creator_device_id.to_string(), ts);
        branch.parent_storage_key = Some(parent.id.storage_key());
        self.store_branch_metadata(&branch)?;
        Ok(branch)
    }

    // ------------------------------------------------------------------
    // Current / mainline / detached HEAD
    // ------------------------------------------------------------------

    /// Return the currently checked-out branch, or `None` when HEAD is
    /// detached (rule 16).
    fn current_branch(&self) -> Result<Option<Branch>>;

    /// Switch to `id`.  Updates the stored "current branch" pointer.  After
    /// this call HEAD points at `branch.head`.
    fn switch_branch(&self, id: &BranchId) -> Result<()>;

    /// Clear the HEAD_BRANCH pointer so the repository enters detached-HEAD
    /// mode.  The commit pointed to by HEAD is **not** modified — only the
    /// branch-tracking state is erased.
    fn detach_head(&self) -> Result<()>;

    /// Return the unique branch carrying the `mainline` label (rule 1, 9).
    fn get_mainline(&self) -> Result<Option<Branch>> {
        let candidates: Vec<Branch> = self
            .list_branches()?
            .into_iter()
            .filter(|b| b.id.has_mainline())
            .collect();
        match candidates.len() {
            0 => Ok(None),
            1 => Ok(Some(candidates.into_iter().next().unwrap())),
            _ => Err(VoeError::MainlineNotUnique),
        }
    }

    /// Create (or confirm existence of) the default `main@mainline` branch
    /// pointing at `head`.  Returns the resulting branch.
    fn ensure_mainline(&self, head: ObjectId, creator_device_id: &str) -> Result<Branch> {
        if let Some(existing) = self.get_mainline()? {
            return Ok(existing);
        }
        let id = BranchId::mainline_default();
        let branch = self.create_branch(
            &id,
            head,
            creator_device_id,
            &CreateBranchOptions {
                allow_reserved_labels: true,
                if_not_exists: false,
            },
        )?;
        Ok(branch)
    }

    /// Returns `true` when HEAD is detached (no current branch).
    fn is_detached_head(&self) -> Result<bool> {
        Ok(self.current_branch()?.is_none())
    }

    /// Implements rule 16: if HEAD is detached, create a temp sub-branch
    /// rooted at HEAD and switch to it.  Returns the new branch.  If HEAD
    /// is **not** detached, returns `Err(DetachedHead)`.
    fn auto_temp_branch_on_detached(&self, device_id: &str) -> Result<Branch> {
        if !self.is_detached_head()? {
            return Err(VoeError::DetachedHead);
        }
        let ts = current_timestamp();
        let name = format!("temp/{}", ts);
        let head = self
            .get_head()?
            .ok_or_else(|| VoeError::Branch("detached HEAD but no HEAD commit yet".to_string()))?;
        let id = BranchId {
            name,
            labels: vec!["temp".to_string()],
        };
        let branch = self.create_branch(
            &id,
            head,
            device_id,
            &CreateBranchOptions {
                allow_reserved_labels: true,
                if_not_exists: false,
            },
        )?;
        self.switch_branch(&id)?;
        Ok(branch)
    }

    // ------------------------------------------------------------------
    // Aliases (rule 11)
    // ------------------------------------------------------------------

    /// Add an alias to `id`.  The alias name should **not** include the `#`
    /// prefix; this function will strip any leading `#` and validate.
    fn add_alias(&self, id: &BranchId, alias_name: &str) -> Result<Alias> {
        let alias = Alias::new(alias_name)?;
        // Uniqueness check.
        let existing = self.resolve_alias(&alias.name)?;
        if existing.is_some() {
            return Err(VoeError::AliasAlreadyExists {
                alias: alias.display(),
            });
        }
        self.add_alias_inner(id, &alias)?;
        Ok(alias)
    }

    /// Core storage-side helper that actually attaches the alias.
    fn add_alias_inner(&self, id: &BranchId, alias: &Alias) -> Result<()>;

    /// Resolve an alias name (without `#`) to its target branch storage key.
    fn resolve_alias(&self, alias_name: &str) -> Result<Option<String>>;

    /// Remove an alias by its name (without `#`).
    fn remove_alias(&self, id: &BranchId, alias_name: &str) -> Result<()>;

    // ------------------------------------------------------------------
    // Merge (rule 13 & 14)
    // ------------------------------------------------------------------

    /// Merge `primary` and `secondaries` together, returning the resulting
    /// `MergeNode`.  The primary branch's name/labels are preserved (rule 13).
    fn merge_branches(
        &self,
        primary: &BranchId,
        secondaries: &[BranchId],
        merge_bases: &[ObjectId],
        options: &MergeOptions,
    ) -> Result<MergeNode> {
        if secondaries.len() != merge_bases.len() {
            return Err(VoeError::MergeConflict(format!(
                "secondary branches ({}) and merge bases ({}) length mismatch",
                secondaries.len(),
                merge_bases.len()
            )));
        }
        for s in secondaries {
            if self.get_branch(s)?.is_none() {
                return Err(VoeError::BranchNotFound(s.storage_key()));
            }
        }
        let node = MergeNode::new(
            primary.storage_key(),
            secondaries.iter().map(|b| b.storage_key()).collect(),
            merge_bases.to_vec(),
        )
        .with_patch_masks(options.patch_mask_ids.clone())
        .with_message(options.message.clone().unwrap_or_default());
        self.store_merge_node(&node)?;
        Ok(node)
    }

    /// Persist the `MergeNode` to the storage layer (the actual
    /// implementation typically stores it in the object store as a
    /// `ObjectKind::Merge` VOE object).
    fn store_merge_node(&self, node: &MergeNode) -> Result<ObjectId>;

    // ------------------------------------------------------------------
    // Metadata storage
    // ------------------------------------------------------------------

    /// Persist the full `Branch` metadata (not just the ref → head pointer).
    fn store_branch_metadata(&self, branch: &Branch) -> Result<()>;

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    /// Resolve a user-supplied branch reference, which may be either a full
    /// identifier (`main@mainline`), a bare name (gets `DEFAULT_LABEL`
    /// injected), or an alias (`#dev`).  Returns the `BranchId` if matched.
    fn resolve_reference(&self, reference: &str) -> Result<BranchId> {
        if let Some(stripped) = reference.strip_prefix('#') {
            let resolved = self.resolve_alias(stripped)?;
            match resolved {
                Some(key) => return BranchId::parse(&key),
                None => {
                    return Err(VoeError::AliasNotFound {
                        alias: reference.to_string(),
                    })
                }
            }
        }
        BranchId::parse(reference)
    }
}
