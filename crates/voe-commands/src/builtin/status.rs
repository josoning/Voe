use std::collections::HashMap;
use std::path::PathBuf;

use clap::Command as ClapCommand;
use voe_mask::{Mask, MaskResolver, SimpleMaskResolver};
use voe_repo_api::command::{Command, CommandContext, CommandInfo, CommandResult};
use voe_repo_api::model::commit::{IndexEntry, MaskObject};
use voe_repo_api::snapshot::SnapshotEngine;
use voe_types::error::VoeError;

use crate::utils::resolve_repo;

pub struct StatusCommand;

impl Command for StatusCommand {
    fn info(&self) -> CommandInfo {
        CommandInfo {
            name: "status",
            description: "Show the working tree status",
            usage: "voe status",
            examples: &["voe status"],
            aliases: &[],
        }
    }

    fn clap_command(&self) -> Option<ClapCommand> {
        Some(ClapCommand::new("status").about("Show the working tree status"))
    }

    fn execute(&self, ctx: &mut CommandContext) -> CommandResult {
        let (root, repo) = resolve_repo(ctx)?;

        repo.config_manager().refresh()?;
        let cm = repo.config_manager();

        let bs = repo.branch_store();
        let refs = repo.ref_store();
        let cs = repo.commit_store();
        let chunk_store = repo.chunk_store();
        let idx = repo.index_store();

        // Resolve HEAD for snapshot.  `get_head()` may return `Some(NULL)`
        // (branch exists but no commit yet) which we treat the same as None.
        let head_snapshot = match refs.get_head()? {
            Some(head) if !head.is_null() => {
                let resolver = SimpleMaskResolver;
                let engine = SnapshotEngine::new(cs, chunk_store, &resolver);
                engine.snapshot_from(&head).unwrap_or_default()
            }
            _ => HashMap::new(),
        };

        let working = repo.read_working_tree()?;

        let index_state = idx.load()?;

        let staged_snapshot = {
            let mut state = head_snapshot.clone();
            let mut masks: Vec<Box<dyn Mask>> = Vec::new();
            for oid in index_state.all_mask_ids() {
                if let Ok(obj) = cs.retrieve_mask(&oid) {
                    match obj {
                        MaskObject::Chunk(m) => masks.push(Box::new(m)),
                    }
                }
            }
            let refs_mask: Vec<&dyn Mask> = masks.iter().map(|m| m.as_ref()).collect();
            let resolver = SimpleMaskResolver;
            if let Ok(ordered) = resolver.resolve_order(&refs_mask) {
                let _ = resolver.apply_all(&ordered, &mut state);
            }
            state
        };

        let mut staged_paths: Vec<String> = Vec::new();
        let mut unstaged_paths: Vec<String> = Vec::new();
        let mut untracked_paths: Vec<String> = Vec::new();

        for entry in &index_state.entries {
            staged_paths.push(entry.path.clone());
        }

        for (path, data) in &working {
            let key = path.to_string_lossy().to_string();
            if key == "/voeconfig.toml" {
                continue;
            }
            let rel = key.trim_start_matches('/');
            if cm.is_ignored(rel) {
                continue;
            }

            let in_staged = staged_snapshot.get(path);
            let in_head = head_snapshot.get(path);

            match (in_staged, in_head) {
                (Some(staged_data), _) => {
                    if staged_data != data {
                        unstaged_paths.push(key.clone());
                    }
                }
                (None, _) => {
                    if !staged_paths.contains(&key) {
                        untracked_paths.push(key.clone());
                    }
                }
            }
        }

        for path in staged_snapshot.keys() {
            let key = path.to_string_lossy().to_string();
            let rel = key.trim_start_matches('/');
            if cm.is_ignored(rel) {
                continue;
            }
            if !working.contains_key(path) {
                unstaged_paths.push(key);
            }
        }

        let has_head = refs.get_head()?.is_some_and(|h| !h.is_null());

        match bs.current_branch()? {
            Some(branch) => println!("On branch {}", branch.id.canonical()),
            None => println!("On detached HEAD"),
        }
        println!("Repository root: {}", root.display());
        println!();

        if !has_head
            && index_state.is_empty()
            && staged_snapshot.is_empty()
            && untracked_paths.is_empty()
        {
            println!("No commits yet and nothing to commit.");
            return Ok(());
        }

        let mut has_changes = false;

        if !index_state.is_empty() {
            has_changes = true;
            println!("Changes to be committed:");
            println!("  (use \"voe reset HEAD <file>...\" to unstage)");
            println!();

            let mut staged_entries: Vec<(String, char)> = Vec::new();
            for entry in &index_state.entries {
                let key = entry.path.clone();
                if key == "/voeconfig.toml" {
                    continue;
                }
                let rel = key.trim_start_matches('/');
                if cm.is_ignored(rel) {
                    continue;
                }

                let status = determine_staged_status(entry, cs, &head_snapshot)?;
                staged_entries.push((entry.path.clone(), status));
            }
            staged_entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (path, status_char) in staged_entries {
                let rel = path.trim_start_matches('/');
                println!("\t{}  {}", status_char, rel);
            }
            println!();
        }

        let has_unstaged = !unstaged_paths.is_empty();
        if has_unstaged {
            has_changes = true;
            println!("Changes not staged for commit:");
            println!("  (use \"voe add <file>...\" to update what will be committed)");
            println!("  (use \"voe switch <file>...\" to discard local changes)");
            println!();

            unstaged_paths.sort();
            for key in &unstaged_paths {
                let p = PathBuf::from(key);
                let rel = key.trim_start_matches('/');
                let has_in_working = working.contains_key(&p);
                let has_in_staged = staged_snapshot.contains_key(&p);

                let status_char = match (has_in_working, has_in_staged) {
                    (false, true) => 'D',
                    (true, true) => 'M',
                    (true, false) => 'M',
                    _ => 'M',
                };
                println!("\t{}  {}", status_char, rel);
            }
            println!();
        }

        if !untracked_paths.is_empty() {
            has_changes = true;
            println!("Untracked files:");
            println!("  (use \"voe add <file>...\" to include in what will be committed)");
            println!();

            untracked_paths.sort();
            for key in &untracked_paths {
                let rel = key.trim_start_matches('/');
                println!("\t{}", rel);
            }
            println!();
        }

        if !has_changes {
            println!("nothing to commit, working tree clean");
        } else if !index_state.is_empty() && (has_unstaged || !untracked_paths.is_empty()) {
            let staged_count = index_state.entries.len();
            let unstaged_count = unstaged_paths.len();
            let untracked_count = untracked_paths.len();
            println!(
                "{} file(s) changed, {} to be committed, {} unstaged, {} untracked",
                staged_count + unstaged_count,
                staged_count,
                unstaged_count,
                untracked_count
            );
        }

        Ok(())
    }
}

fn determine_staged_status(
    entry: &IndexEntry,
    cs: &dyn voe_repo_api::model::commit::CommitStore,
    head_snapshot: &HashMap<PathBuf, Vec<u8>>,
) -> Result<char, VoeError> {
    let path = PathBuf::from(&entry.path);
    let has_head = head_snapshot.contains_key(&path);

    let first_mask_id = entry
        .mask_ids
        .first()
        .ok_or_else(|| VoeError::Other(format!("No masks for staged path: {}", entry.path)))?;
    let mask_obj = cs.retrieve_mask(first_mask_id)?;

    let is_deleted = match &mask_obj {
        MaskObject::Chunk(m) => m.changes.iter().any(|c| c.content.is_deleted()),
    };

    if is_deleted {
        return Ok('D');
    }

    if has_head {
        Ok('M')
    } else {
        Ok('A')
    }
}
