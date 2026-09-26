use voe_repo_api::command::{Command, CommandContext, CommandInfo, CommandResult};
use voe_types::error::VoeError;

use crate::utils::{resolve_oid, resolve_repo};

/// Reset mode selected by the user. Defaults to `Mixed` when neither flag
/// is provided, matching Git's `git reset <commit>` behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResetMode {
    /// Move HEAD only. Index and working tree stay untouched, so any staged
    /// changes from the old HEAD become staged relative to the new HEAD.
    Soft,
    /// Move HEAD and clear the index. Working tree stays untouched, so the
    /// previously staged changes become unstaged relative to the new HEAD.
    Mixed,
}

impl ResetMode {
    fn from_args(ctx: &CommandContext) -> Result<Self, VoeError> {
        let soft = ctx.args.contains_key("soft");
        let mixed = ctx.args.contains_key("mixed");

        if soft && mixed {
            return Err(VoeError::Other(
                "--soft and --mixed are mutually exclusive".to_string(),
            ));
        }

        if soft {
            Ok(ResetMode::Soft)
        } else {
            Ok(ResetMode::Mixed)
        }
    }
}

pub struct ResetCommand;

impl Command for ResetCommand {
    fn info(&self) -> CommandInfo {
        CommandInfo {
            name: "reset",
            description: "Move HEAD to a previous commit, optionally unstaging changes",
            usage: "voe reset [--soft|--mixed] <COMMIT>",
            examples: &[
                "voe reset HEAD~1",
                "voe reset --soft abc123",
                "voe reset --mixed HEAD~2",
            ],
            aliases: &[],
        }
    }

    fn execute(&self, ctx: &mut CommandContext) -> CommandResult {
        let (_root, repo) = resolve_repo(ctx)?;

        let mode = ResetMode::from_args(ctx)?;

        let target = ctx.args.get("target").cloned().ok_or_else(|| {
            VoeError::Other(
                "Usage: voe reset [--soft|--mixed] <commit>\n\
                     Specify a target commit (e.g. HEAD~1, abc123)."
                    .to_string(),
            )
        })?;

        let commit_oid = resolve_oid(repo.as_ref(), &target)?;

        let head_oid_opt = repo.ref_store().get_head()?;
        if Some(&commit_oid) == head_oid_opt.as_ref() {
            println!("HEAD is already at {}", commit_oid);
            return Ok(());
        }

        let current_head = head_oid_opt
            .as_ref()
            .ok_or_else(|| VoeError::Other("No HEAD — nothing to reset".to_string()))?;

        // Update HEAD ref first.
        repo.ref_store().set_head(&commit_oid)?;

        // When on a branch (not detached), also update the branch pointer.
        // Release branches have append-only semantics — resetting backwards
        // will be rejected by set_branch_head with ReleaseAppendViolation.
        if let Some(branch) = repo.branch_store().current_branch()? {
            if let Err(e) = repo
                .branch_store()
                .set_branch_head(&branch.id, commit_oid.clone())
            {
                // If the branch is a release branch and we're moving backward,
                // give a friendlier error than the raw ReleaseAppendViolation.
                match &e {
                    VoeError::ReleaseAppendViolation => {
                        // Roll back the HEAD change we just made so the repo
                        // is not left half-updated.
                        let _ = repo.ref_store().set_head(current_head);
                        return Err(VoeError::Other(format!(
                            "Cannot reset branch '{}' — release branches are append-only.\n\
                             Use `voe switch <commit>` for a detached exploration, \
                             or `voe rollback --force` if you really want to discard history.",
                            branch.id.canonical()
                        )));
                    }
                    _ => return Err(e),
                }
            }
        }

        match mode {
            ResetMode::Soft => {
                println!(
                    "Soft reset: HEAD moved from {} to {}.\n\
                     Index and working tree unchanged — staged changes kept relative to new HEAD.",
                    current_head, commit_oid
                );
            }
            ResetMode::Mixed => {
                repo.index_store().clear()?;
                println!(
                    "Mixed reset: HEAD moved from {} to {}.\n\
                     Index cleared — staged changes are now unstaged in the working tree.",
                    current_head, commit_oid
                );
            }
        }

        Ok(())
    }
}
