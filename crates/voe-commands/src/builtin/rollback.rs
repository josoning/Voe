use voe_repo_api::command::{Command, CommandContext, CommandInfo, CommandResult};
use voe_types::error::VoeError;

use crate::utils::{resolve_oid, resolve_repo, snapshot_for};

/// Hard-reset — the dangerous "discard everything" variant that lives under
/// its own command name so users have to type `voe rollback` rather than
/// accidentally adding `--hard` to `voe reset`.
///
/// Semantics (mirrors `git reset --hard <target>`):
///   1. Move HEAD (and the current branch, if any) to `target`.
///   2. Clear the index — all staged changes are discarded.
///   3. Overwrite the working tree with `target`'s snapshot — any local
///      modifications and untracked files are lost.
///
/// Requires `--force` as a safety rail.
pub struct RollbackCommand;

impl Command for RollbackCommand {
    fn info(&self) -> CommandInfo {
        CommandInfo {
            name: "rollback",
            description: "HARD reset — discard all local changes and move HEAD to a commit",
            usage: "voe rollback <COMMIT> --force",
            examples: &["voe rollback HEAD~1 --force", "voe rollback abc123 --force"],
            aliases: &[],
        }
    }

    fn execute(&self, ctx: &mut CommandContext) -> CommandResult {
        let (_root, repo) = resolve_repo(ctx)?;

        if !ctx.args.contains_key("force") {
            return Err(VoeError::Other(
                "voe rollback is destructive — use --force to confirm.\n\
                 This will discard all uncommitted changes in the index and working tree."
                    .to_string(),
            ));
        }

        let target =
            ctx.args.get("target").cloned().ok_or_else(|| {
                VoeError::Other("Usage: voe rollback <commit> --force".to_string())
            })?;

        let commit_oid = resolve_oid(repo.as_ref(), &target)?;

        let head_oid_opt = repo.ref_store().get_head()?;
        let current_head = head_oid_opt.as_ref();

        if Some(&commit_oid) == current_head {
            println!("HEAD is already at {}", commit_oid);
            return Ok(());
        }

        // Phase 1: Materialise the target snapshot BEFORE we touch anything.
        // If snapshot computation fails (missing object, corrupt mask, …),
        // we bail out with the repo untouched.
        let target_snap = snapshot_for(repo.as_ref(), &commit_oid)?;

        // Phase 2: Move HEAD and branch pointer.
        repo.ref_store().set_head(&commit_oid)?;

        if let Some(branch) = repo.branch_store().current_branch()? {
            // Unlike ResetCommand, rollback with --force explicitly opts in
            // to destruction, so we allow overwriting release branches too.
            // set_branch_head on a release branch still enforces append-only
            // by default — we bypass by updating the ref directly and then
            // re-storing the Branch metadata so its `head` field stays in
            // sync with the ref file.
            //
            // SAFETY: The user already passed --force, so we honour that
            // intent rather than second-guessing them with ReleaseAppendViolation.
            repo.ref_store()
                .set_ref(&branch.id.storage_key(), &commit_oid)?;

            // Fetch the current Branch metadata, update its head and
            // last_updated timestamp, then persist it back.
            let mut meta = branch.clone();
            meta.head = commit_oid.clone();
            meta.last_updated = voe_types::author::current_timestamp();
            repo.branch_store().store_branch_metadata(&meta)?;
        }

        // Phase 3: Clear the index — all staged masks are discarded.
        repo.index_store().clear()?;

        // Phase 4: Overwrite the working tree.
        repo.write_snapshot_to_disk(&target_snap)?;

        let from = current_head
            .map(|h| h.to_string())
            .unwrap_or_else(|| "<none>".to_string());
        println!(
            "Rollback complete: HEAD moved from {} to {}.\n\
             Index cleared and working tree restored — all uncommitted changes discarded.",
            from, commit_oid
        );

        Ok(())
    }
}
