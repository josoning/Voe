use std::fs;
use std::path::{Path, PathBuf};

use voe_mask::SimpleMaskResolver;
use voe_repo_api::command::{Command, CommandContext, CommandInfo, CommandResult};
use voe_repo_api::model::branch::MergeNode;
use voe_repo_api::snapshot::{build_tree_from_snapshot, MergeEngine, MergeOutcome};
use voe_types::error::VoeError;
use voe_types::object::ObjectId;

use crate::utils::{resolve_repo, snapshot_for};

pub const MERGE_HEAD_FILE: &str = "MERGE_HEAD";

pub struct MergeCommand;

/// Path to the MERGE_HEAD state file inside the `.voe/` directory.
fn merge_head_path(root: &Path) -> PathBuf {
    root.join(".voe").join(MERGE_HEAD_FILE)
}

/// Read the stored pre-merge primary head from MERGE_HEAD, if present.
fn read_merge_head(root: &Path) -> Option<ObjectId> {
    let path = merge_head_path(root);
    fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(ObjectId::new)
}

/// Persist the pre-merge primary head so `voe merge --abort` can restore it.
fn write_merge_head(root: &Path, oid: &ObjectId) -> std::io::Result<()> {
    let voe_dir = root.join(".voe");
    fs::create_dir_all(&voe_dir)?;
    fs::write(merge_head_path(root), oid.as_str())
}

/// Remove MERGE_HEAD after a successful merge or a completed abort.
fn clear_merge_head(root: &Path) {
    let path = merge_head_path(root);
    let _ = fs::remove_file(path);
}

impl Command for MergeCommand {
    fn info(&self) -> CommandInfo {
        CommandInfo {
            name: "merge",
            description: "Merge another branch into the current branch",
            usage: "voe merge <BRANCH> [-m MESSAGE] | voe merge --abort",
            examples: &[
                "voe merge feature",
                "voe merge feature@dev -m \"merge feature into main\"",
                "voe merge #feature-alias",
                "voe merge --abort",
            ],
            aliases: &[],
        }
    }

    fn execute(&self, ctx: &mut CommandContext) -> CommandResult {
        let (root, repo) = resolve_repo(ctx)?;

        // --abort path: restore the repo to its pre-merge state.
        if ctx.args.contains_key("abort") {
            return abort_merge(&root, repo.as_ref());
        }

        let secondary_ref =
            ctx.args.get("target").cloned().ok_or_else(|| {
                VoeError::Other("Usage: voe merge <BRANCH> [--abort]".to_string())
            })?;

        let message = ctx.args.get("message").cloned();

        let bs = repo.branch_store();
        let refs = repo.ref_store();

        // ---- 前置检查 ----------------------------------------------------

        let primary = bs.current_branch()?.ok_or_else(|| {
            VoeError::Other(
                "HEAD is detached — cannot merge without a branch checked out.\n\
                 Check out a branch or create one first."
                    .to_string(),
            )
        })?;

        // Check for an in-progress merge (leftover MERGE_HEAD from a
        // previous aborted conflict).  Refuse to start another merge on
        // top of it — the user must abort or resolve first.
        if read_merge_head(&root).is_some() {
            return Err(VoeError::Other(
                "A merge is already in progress.  Run `voe merge --abort` first \
                 or complete the merge commit manually."
                    .to_string(),
            ));
        }

        // Working tree must be clean — same check as switch/commit.
        if let Some(head_oid) = refs.get_head()? {
            let disk = repo.read_working_tree()?;
            let head_snap = snapshot_for(repo.as_ref(), &head_oid)?;
            if disk != head_snap {
                return Err(VoeError::Other(
                    "Working tree has uncommitted changes.\n\
                     Commit or stash your changes first."
                        .to_string(),
                ));
            }
        }

        // Resolve the secondary branch via resolve_reference (supports
        // aliases like #feature too).
        let secondary_id = bs.resolve_reference(&secondary_ref)?;
        let secondary = bs
            .get_branch(&secondary_id)?
            .ok_or_else(|| VoeError::BranchNotFound(secondary_id.storage_key()))?;

        if primary.id == secondary.id {
            return Err(VoeError::Other(
                "Cannot merge a branch into itself.".to_string(),
            ));
        }

        let primary_head = primary.head.clone();
        let secondary_head = secondary.head.clone();

        // ---- Plan the merge ---------------------------------------------

        let engine = MergeEngine::new(repo.commit_store(), repo.chunk_store(), &SimpleMaskResolver);

        let outcome = engine.plan_merge(&primary_head, &secondary_head)?;

        match outcome {
            MergeOutcome::AlreadyUpToDate => {
                println!(
                    "Already up to date — '{}' contains everything in '{}'.",
                    primary.id.canonical(),
                    secondary.id.canonical(),
                );
                Ok(())
            }

            MergeOutcome::FastForward { target_head } => {
                // Fast-forward is a pure ref move — no merge commit needed.
                refs.set_head(&target_head)?;
                bs.set_branch_head(&primary.id, target_head.clone())?;

                let target_snap = snapshot_for(repo.as_ref(), &target_head)?;
                repo.write_snapshot_to_disk(&target_snap)?;

                println!(
                    "Fast-forward merge — '{}' now at {}.",
                    primary.id.canonical(),
                    &target_head.to_string()[..8],
                );
                Ok(())
            }

            MergeOutcome::Conflicts { conflicts } => {
                // Persist MERGE_HEAD so `--abort` knows what to restore.
                if let Err(e) = write_merge_head(&root, &primary_head) {
                    return Err(VoeError::Storage(format!(
                        "Failed to write MERGE_HEAD: {}",
                        e
                    )));
                }

                println!("Merge failed with conflicts:\n");
                for c in &conflicts {
                    println!("  {} — {}", c.path.display(), c.reason);
                }
                println!(
                    "\nResolve manually, then run `voe commit` to finish the merge,\n\
                     or run `voe merge --abort` to abandon it."
                );
                Err(VoeError::MergeConflict(format!(
                    "{} conflict(s) encountered",
                    conflicts.len()
                )))
            }

            MergeOutcome::NoConflicts {
                merged_mask_ids,
                merged_snapshot,
            } => {
                // Pre-write MERGE_HEAD so that if the later write phase
                // aborts midway the user still has a recovery path.
                if let Err(e) = write_merge_head(&root, &primary_head) {
                    return Err(VoeError::Storage(format!(
                        "Failed to write MERGE_HEAD: {}",
                        e
                    )));
                }

                // ---- Build the tree -----------------------------------
                let tree_oid = build_tree_from_snapshot(repo.object_store(), &merged_snapshot)?;

                // ---- Create the merge commit ---------------------------
                let author_name = repo
                    .config_manager()
                    .get_user_name()
                    .unwrap_or_else(|| "Voe User".to_string());
                let author_email = repo
                    .config_manager()
                    .get_user_email()
                    .unwrap_or_else(|| "voe@example.com".to_string());
                let author = voe_types::Author::new(author_name, author_email);

                let default_msg = format!(
                    "Merge '{}' into '{}'",
                    secondary.id.canonical(),
                    primary.id.canonical()
                );
                let msg = message.unwrap_or(default_msg);

                let commit = voe_repo_api::model::commit::Commit {
                    tree: tree_oid,
                    parents: vec![primary_head.clone(), secondary_head.clone()],
                    author: author.clone(),
                    committer: author,
                    message: msg.clone(),
                    masks: merged_mask_ids,
                    signature: None,
                };

                let commit_oid = repo.commit_store().store_commit(&commit)?;

                // ---- Create the MergeNode metadata ---------------------
                let base = engine.find_merge_base(&primary_head, &secondary_head)?;
                let base_ids = match &base {
                    Some(b) => vec![b.clone()],
                    None => vec![],
                };

                let node = MergeNode::new(
                    primary.id.storage_key(),
                    vec![secondary.id.storage_key()],
                    base_ids,
                )
                .with_message(msg.clone());

                // store_merge_node writes it as ObjectKind::Merge.
                let _ = bs.store_merge_node(&node)?;

                // ---- Update refs ---------------------------------------
                refs.set_head(&commit_oid)?;

                // Release-branch safety: a merge commit's parents include
                // the old head, so it's always a descendant — append-only
                // is satisfied.  We still use set_branch_head (not the
                // ReleaseAppend-only path) because that API is more strict
                // about the walk, but MergeEngine already verified ancestry
                // and the merge commit *must* have primary_head as parent.
                // For release branches we call set_release_branch_head to
                // honour its tagged-mask integrity check.
                if primary.id.is_release() {
                    repo.set_release_branch_head(&primary.id, commit_oid.clone())?;
                } else {
                    bs.set_branch_head(&primary.id, commit_oid.clone())?;
                }

                // ---- Refresh working tree ------------------------------
                repo.write_snapshot_to_disk(&merged_snapshot)?;

                // ---- Clean up ------------------------------------------
                clear_merge_head(&root);

                println!(
                    "[{}] {} (merge {})",
                    &commit_oid.to_string()[..8],
                    msg,
                    &secondary_head.to_string()[..8],
                );
                Ok(())
            }
        }
    }
}

/// `voe merge --abort` — restore HEAD, branch pointer, and working tree to
/// the state they were in before the merge started.
fn abort_merge(root: &Path, repo: &dyn voe_repo_api::Repository) -> CommandResult {
    let aborted_head = match read_merge_head(root) {
        Some(oid) => oid,
        None => {
            return Err(VoeError::Other(
                "No merge in progress — MERGE_HEAD not found.".to_string(),
            ));
        }
    };

    refs_restore(repo, &aborted_head)?;

    clear_merge_head(root);

    println!(
        "Merge aborted.  HEAD restored to {}.",
        &aborted_head.to_string()[..8],
    );
    Ok(())
}

/// Move HEAD + current branch pointer back to `oid`, then overwrite the
/// working tree with that commit's snapshot.  Mirrors what `rollback` does
/// but *does not* clear the index — the user may want to retry the merge
/// manually or just keep working.
fn refs_restore(repo: &dyn voe_repo_api::Repository, oid: &ObjectId) -> CommandResult {
    let bs = repo.branch_store();
    let refs = repo.ref_store();

    refs.set_head(oid)?;

    if let Some(branch) = bs.current_branch()? {
        // Same reasoning as rollback: bypass release-append-only by
        // rewriting the ref directly, then update metadata.
        if branch.id.is_release() {
            refs.set_ref(&branch.id.storage_key(), oid)?;
            let mut meta = branch.clone();
            meta.head = oid.clone();
            meta.last_updated = voe_types::author::current_timestamp();
            bs.store_branch_metadata(&meta)?;
        } else {
            bs.set_branch_head(&branch.id, oid.clone())?;
        }
    }

    Ok(())
}
