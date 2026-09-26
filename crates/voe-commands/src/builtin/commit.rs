use std::collections::HashMap;

use clap::{Arg, Command as ClapCommand};
use voe_mask::{Mask, MaskResolver, SimpleMaskResolver};
use voe_repo_api::command::{Command, CommandContext, CommandInfo, CommandResult};
use voe_repo_api::model::commit::MaskObject;
use voe_repo_api::snapshot::{build_tree_from_snapshot, SnapshotEngine};
use voe_types::error::VoeError;

use crate::utils::resolve_repo;

pub struct CommitCommand;

impl Command for CommitCommand {
    fn info(&self) -> CommandInfo {
        CommandInfo {
            name: "commit",
            description: "Create a commit from staged masks",
            usage: "voe commit [-m MESSAGE]",
            examples: &["voe commit -m \"feat: add hello\"", "voe commit"],
            aliases: &[],
        }
    }

    fn clap_command(&self) -> Option<ClapCommand> {
        Some(
            ClapCommand::new("commit")
                .about("Create a commit from staged masks")
                .arg(
                    Arg::new("message")
                        .short('m')
                        .long("message")
                        .help("Commit message")
                        .required(false),
                ),
        )
    }

    fn execute(&self, ctx: &mut CommandContext) -> CommandResult {
        let (_root, repo) = resolve_repo(ctx)?;

        let message = ctx
            .args
            .get("message")
            .cloned()
            .unwrap_or_else(|| "no message".to_string());

        let author_name = repo
            .config_manager()
            .get_user_name()
            .unwrap_or_else(|| "Voe User".to_string());
        let author_email = repo
            .config_manager()
            .get_user_email()
            .unwrap_or_else(|| "voe@example.com".to_string());

        let cs = repo.commit_store();
        let refs = repo.ref_store();
        let idx = repo.index_store();

        let state = idx.load()?;
        if state.is_empty() {
            return Err(VoeError::Other(
                "Nothing to commit. Stage changes first with `voe add`.".to_string(),
            ));
        }

        let staged_mask_oids = state.all_mask_ids();

        let effective_head = refs.get_head()?.filter(|h| !h.is_null());

        let mut parent_snapshot = match &effective_head {
            Some(head) => {
                let resolver = SimpleMaskResolver;
                let engine = SnapshotEngine::new(cs, repo.chunk_store(), &resolver);
                engine.snapshot_from(head).unwrap_or_default()
            }
            None => HashMap::new(),
        };

        {
            let mut masks: Vec<Box<dyn Mask>> = Vec::new();
            for oid in &staged_mask_oids {
                let obj = cs.retrieve_mask(oid)?;
                match obj {
                    MaskObject::Chunk(m) => masks.push(Box::new(m)),
                }
            }

            let refs_mask: Vec<&dyn Mask> = masks.iter().map(|m| m.as_ref()).collect();
            let resolver = SimpleMaskResolver;
            let ordered = resolver.resolve_order(&refs_mask)?;
            resolver.apply_all(&ordered, &mut parent_snapshot)?;
        }

        let tree_oid = build_tree_from_snapshot(repo.object_store(), &parent_snapshot)?;

        let parents = effective_head
            .as_ref()
            .map(|h| vec![h.clone()])
            .unwrap_or_default();

        let author = voe_types::Author::new(author_name, author_email);

        let commit = voe_repo_api::model::commit::Commit {
            tree: tree_oid,
            parents,
            author: author.clone(),
            committer: author,
            message,
            masks: staged_mask_oids,
            signature: None,
        };

        let commit_oid = cs.store_commit(&commit)?;
        refs.set_head(&commit_oid)?;
        idx.clear()?;

        if let Some(branch) = repo.branch_store().current_branch()? {
            repo.branch_store()
                .set_branch_head(&branch.id, commit_oid.clone())?;
        }

        println!("[{}] {}", &commit_oid.to_string()[..8], commit.message);
        Ok(())
    }
}
