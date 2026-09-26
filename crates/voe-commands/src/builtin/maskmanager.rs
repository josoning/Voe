//! Maskmanager — view and edit masks, regardless of staging status.
//!
//! Responsibilities:
//! - `list`   — show every known mask (unstaged + staged) in a single
//!   view with source markers
//! - `split`  — break one multi-change mask into several single-change masks
//! - `merge`  — fold several masks into one multi-change mask
//! - `reload` — refresh the view
//!
//! Staging/unstaging lives in the sibling `stagemanager` command — but when
//! `split` / `merge` operate on a **staged** mask this command keeps the
//! index in sync automatically (drop the old OIDs, store the new ones,
//! re-add them).  Operating on an **unstaged** mask only touches the
//! commit store; the mask remains unstaged afterwards.

use std::path::Path;

use owo_colors::OwoColorize;

use voe_mask::{ChunkMask, MaskChange};
use voe_repo_api::command::{Command, CommandContext, CommandInfo, CommandResult};
use voe_types::error::{Result, VoeError};
use voe_types::object::ObjectId;

use crate::builtin::shared::mask_listing::{
    compute_staged, compute_unstaged, print_all_masks, refresh_both,
};
use crate::utils::resolve_repo;

pub struct MaskManagerCommand;

const SUBCOMMAND_ALIASES: &[(&str, &str)] = &[
    ("list", "list"),
    ("ls", "list"),
    ("l", "list"),
    ("split", "split"),
    ("s", "split"),
    ("merge", "merge"),
    ("m", "merge"),
    ("refresh", "reload"),
    ("reload", "reload"),
    ("help", "help"),
    ("h", "help"),
    ("?", "help"),
];

fn normalize_subcommand(raw: &str) -> &'static str {
    let lower = raw.to_ascii_lowercase();
    for (alias, canonical) in SUBCOMMAND_ALIASES {
        if *alias == lower.as_str() {
            return canonical;
        }
    }
    ""
}

fn resolve_subcommand(ctx: &CommandContext) -> Result<(&'static str, Vec<String>)> {
    if let Some(raw) = ctx.args.get("subcommand") {
        let canon = normalize_subcommand(raw);
        if canon.is_empty() {
            return Err(VoeError::Other(format!(
                "Unknown maskmanager subcommand: '{}'.  Use 'voe maskmanager help'.",
                raw
            )));
        }
        return Ok((canon, Vec::new()));
    }

    if let Some(raw) = ctx.args.get("__pos") {
        let tokens: Vec<String> = raw.split('\x1f').map(|s| s.to_string()).collect();
        if let Some(first) = tokens.first() {
            let canon = normalize_subcommand(first);
            if canon.is_empty() {
                return Err(VoeError::Other(format!(
                    "Unknown maskmanager subcommand: '{}'.  Type 'help'.",
                    first
                )));
            }
            return Ok((canon, tokens.into_iter().skip(1).collect()));
        }
    }

    Ok(("list", Vec::new()))
}

fn resolve_usize(ctx: &CommandContext, key: &str, shell_tokens: &[String]) -> Option<usize> {
    if let Some(raw) = ctx.args.get(key) {
        return raw.parse().ok();
    }
    shell_tokens.first().and_then(|s| s.parse().ok())
}

fn parse_index_list(tokens: &[&str]) -> Result<Vec<usize>> {
    let mut out: Vec<usize> = Vec::new();
    for t in tokens {
        let n: usize = t
            .parse()
            .map_err(|_| VoeError::Other(format!("'{}' is not a valid number", t)))?;
        if n == 0 {
            return Err(VoeError::Other("Indices are 1-based".to_string()));
        }
        out.push(n - 1);
    }
    Ok(out)
}

/// Build the merged mask list that `maskmanager list` prints.  Returns
/// `(source_label, optional_oid, mask)` tuples so callers can tell
/// which side of the fence a given index belongs to.
fn merged_masks(
    unstaged: &[ChunkMask],
    staged: &[(ObjectId, ChunkMask)],
) -> Vec<(&'static str, Option<ObjectId>, ChunkMask)> {
    let mut out: Vec<(&'static str, Option<ObjectId>, ChunkMask)> = Vec::new();
    for m in unstaged {
        out.push(("unstaged", None, m.clone()));
    }
    for (oid, m) in staged {
        out.push(("staged", Some(oid.clone()), m.clone()));
    }
    out
}

fn cmd_list(root: &Path, repo: &dyn voe_repo_api::Repository) -> CommandResult {
    let unstaged = compute_unstaged(root, repo)?;
    let staged = compute_staged(repo)?;
    print_all_masks(&unstaged, &staged);
    Ok(())
}

fn cmd_split(
    ctx: &CommandContext,
    n: usize,
    root: &Path,
    repo: &dyn voe_repo_api::Repository,
) -> CommandResult {
    let unstaged = compute_unstaged(root, repo)?;
    let staged = compute_staged(repo)?;
    let all = merged_masks(&unstaged, &staged);

    if n == 0 || n > all.len() {
        return Err(VoeError::Other(format!(
            "Index out of range (have {} masks total, 1-based)",
            all.len()
        )));
    }

    let (source, old_oid_opt, old_mask) = &all[n - 1];
    if old_mask.changes.len() <= 1 {
        let label = old_oid_opt
            .as_ref()
            .map(|o| o.to_string())
            .unwrap_or_else(|| old_mask.id.to_string());
        println!(
            "  {} already has only one change — nothing to split.",
            label.bold()
        );
        return Ok(());
    }

    // If the target is staged, remove the old OID from the index first.
    let is_staged = *source == "staged";
    if is_staged {
        let old_oid = old_oid_opt.as_ref().unwrap();
        let index_state = repo.index_store().load()?;
        let mut paths_to_clean: Vec<String> = Vec::new();
        for entry in &index_state.entries {
            if entry.mask_ids.contains(old_oid) {
                paths_to_clean.push(entry.path.clone());
            }
        }
        for p in &paths_to_clean {
            repo.index_store().remove_mask(p, old_oid)?;
        }
    }

    let mut new_oids: Vec<ObjectId> = Vec::new();
    for change in &old_mask.changes {
        let new_mask = ChunkMask::file(
            ObjectId::new("split-placeholder"),
            change.location.path().clone(),
            change.content.clone(),
        );
        let new_oid = repo.commit_store().store_mask(&new_mask)?;
        // Only re-add to the index if the original was staged; otherwise
        // the split masks are still part of the "unstaged" set (the
        // change is the same, just now chunked differently).
        if is_staged {
            let path_str = change.location.path().to_string_lossy().into_owned();
            repo.index_store().add_mask(&path_str, &new_oid)?;
        }
        new_oids.push(new_oid);
    }

    let label = old_oid_opt
        .as_ref()
        .map(|o| o.to_string())
        .unwrap_or_else(|| old_mask.id.to_string());

    println!(
        "  {} {} into {} single-change mask(s): {}",
        "Split".blue().bold(),
        label,
        new_oids.len().bold(),
        new_oids
            .iter()
            .map(|o| o.to_string())
            .collect::<Vec<_>>()
            .join(", ")
            .dimmed()
    );

    if is_staged {
        println!("  {}", "(index updated automatically)".dimmed());
    }

    let _ = (ctx, root);
    Ok(())
}

fn cmd_merge(
    ctx: &CommandContext,
    indices: &[usize],
    root: &Path,
    repo: &dyn voe_repo_api::Repository,
) -> CommandResult {
    if indices.len() < 2 {
        return Err(VoeError::Other(
            "merge requires at least two 1-based indices from the full mask list".to_string(),
        ));
    }

    let unstaged = compute_unstaged(root, repo)?;
    let staged = compute_staged(repo)?;
    let all = merged_masks(&unstaged, &staged);

    for &i in indices {
        if i == 0 || i > all.len() {
            return Err(VoeError::Other(format!(
                "Index out of range: {} (have {} masks total, 1-based)",
                i,
                all.len()
            )));
        }
    }

    let zero_based: Vec<usize> = indices.iter().map(|&i| i - 1).collect();

    let mut all_changes: Vec<MaskChange> = Vec::new();
    let mut old_oids: Vec<ObjectId> = Vec::new();
    let mut all_staged = true;
    let mut merge_paths: Vec<String> = Vec::new();
    for &idx in &zero_based {
        let (source, oid, mask) = &all[idx];
        all_changes.extend(mask.changes.clone());
        if let Some(o) = oid {
            old_oids.push(o.clone());
        } else {
            // Unstaged mask — no OID, mark the merge target as unstaged
            // so we don't touch the index.
            all_staged = false;
        }
        if *source != "staged" {
            all_staged = false;
        }
        for c in &mask.changes {
            let p = c.location.path().to_string_lossy().into_owned();
            if !merge_paths.contains(&p) {
                merge_paths.push(p);
            }
        }
    }

    // Remove old masks from the index only if every selected mask was
    // staged — mixing staged + unstaged masks is treated as a "fresh"
    // merge that operates purely on the commit store.
    if all_staged {
        for p in &merge_paths {
            for oid in &old_oids {
                repo.index_store().remove_mask(p, oid)?;
            }
        }
    }

    let merged = ChunkMask {
        id: ObjectId::new("merge-placeholder"),
        changes: all_changes,
        metadata: Default::default(),
        tag: None,
    };
    let merged_oid = repo.commit_store().store_mask(&merged)?;

    if all_staged {
        for p in &merge_paths {
            repo.index_store().add_mask(p, &merged_oid)?;
        }
    }

    println!(
        "  {} {} mask(s) into {} ({} changes)",
        "Merged".magenta().bold(),
        old_oids.len().max(indices.len()).bold(),
        merged_oid,
        merged.changes.len().bold()
    );
    if all_staged {
        println!("  {}", "(index updated automatically)".dimmed());
    } else {
        println!(
            "  {}",
            "(mixed staged/unstaged — merge result left unstaged)".dimmed()
        );
    }

    let _ = (ctx, root);
    Ok(())
}

fn cmd_reload(root: &Path, repo: &dyn voe_repo_api::Repository) -> CommandResult {
    let mut unstaged = Vec::new();
    let mut staged = Vec::new();
    refresh_both(root, repo, &mut unstaged, &mut staged);
    println!("{}", "Reloaded.".green().bold());
    Ok(())
}

fn cmd_help() {
    println!();
    println!("{}", "Maskmanager sub-commands:".bold());
    let rows: &[(&str, &str)] = &[
        (
            "list, ls, l",
            "Show all known masks (unstaged + current staging area)",
        ),
        (
            "split <n>, s <n>",
            "Split the n-th mask (from 'list' output) into single-change masks",
        ),
        (
            "merge <n1> <n2> ..., m",
            "Merge the listed masks into one multi-change mask",
        ),
        ("reload, refresh", "Re-scan the working tree and index"),
        ("help, h, ?", "Show this help"),
    ];
    let widest = rows.iter().map(|(c, _)| c.len()).max().unwrap_or(0);
    for (cmd, desc) in rows {
        println!("  {:<width$}  {}", cmd.bold().cyan(), desc, width = widest);
    }
    println!(
        "  {:<width$}  (or 'voe shell' → 'mm split 2')",
        "Usage: voe maskmanager <sub-command> ...",
        width = widest,
    );
    println!(
        "  {:<width$}  split/merge auto-sync the index when the target is staged.",
        "Note:",
        width = widest,
    );
    println!();
}

impl Command for MaskManagerCommand {
    fn info(&self) -> CommandInfo {
        CommandInfo {
            name: "maskmanager",
            description: "View and edit masks: list, split, merge (auto-syncs the index)",
            usage: "voe maskmanager <list|split|merge|reload>",
            examples: &[
                "voe maskmanager list",
                "voe maskmanager split 3",
                "voe maskmanager merge 1 2",
            ],
            aliases: &["maskmgr"],
        }
    }

    fn execute(&self, ctx: &mut CommandContext) -> CommandResult {
        let (subcmd, shell_tokens) = resolve_subcommand(ctx)?;
        let (root, repo) = resolve_repo(ctx)?;

        match subcmd {
            "list" => cmd_list(&root, repo.as_ref()),
            "help" => {
                cmd_help();
                Ok(())
            }
            "reload" => cmd_reload(&root, repo.as_ref()),
            "split" => {
                let n = resolve_usize(ctx, "n", &shell_tokens).ok_or_else(|| {
                    VoeError::Other(
                        "maskmanager split requires a 1-based index argument".to_string(),
                    )
                })?;
                cmd_split(ctx, n, &root, repo.as_ref())
            }
            "merge" => {
                let indices: Vec<usize> = if let Some(raw) = ctx.args.get("indices") {
                    let parts: Vec<&str> = raw.split('\x1f').collect();
                    parse_index_list(&parts)?
                } else {
                    let parts: Vec<&str> = shell_tokens.iter().map(|s| s.as_str()).collect();
                    parse_index_list(&parts)?
                };
                cmd_merge(ctx, &indices, &root, repo.as_ref())
            }
            other => Err(VoeError::Other(format!(
                "Unknown maskmanager subcommand: '{}'.  Use 'voe maskmanager help'.",
                other
            ))),
        }
    }
}
