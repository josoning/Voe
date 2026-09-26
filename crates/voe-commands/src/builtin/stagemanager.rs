//! Stagemanager — manage the current index (staging area).
//!
//! Responsibilities:
//! - `list`   — print every mask currently staged
//! - `add`    — move an unstaged mask into the staging area
//! - `remove` — pull a staged mask out of the staging area
//! - `reload` — refresh the view
//!
//! Mask editing (split/merge/rename) lives in the sibling `maskmanager`
//! command — `stagemanager` is intentionally dumb about what a mask *is* and
//! only tracks whether it is staged or not.

use std::path::Path;

use owo_colors::OwoColorize;

use voe_repo_api::command::{Command, CommandContext, CommandInfo, CommandResult};
use voe_types::error::{Result, VoeError};

use crate::builtin::shared::mask_listing::{
    compute_staged, compute_unstaged, format_action, print_staged, print_unstaged, summarize_mask,
};
use crate::utils::resolve_repo;

pub struct StagemanagerCommand;

/// Known stagemanager subcommand aliases.  `normalize_subcommand` maps every
/// entry here to its canonical name used in the dispatch match below.
const SUBCOMMAND_ALIASES: &[(&str, &str)] = &[
    ("list", "list"),
    ("ls", "list"),
    ("l", "list"),
    ("stage", "add"),
    ("add", "add"),
    ("a", "add"),
    ("unstage", "remove"),
    ("rm", "remove"),
    ("remove", "remove"),
    ("r", "remove"),
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

/// Resolve the stagemanager subcommand name from either the CLI-shaped
/// `args["subcommand"]` key or the shell-shaped `__pos` token list.
/// Falls back to `"list"` when nothing was specified.
fn resolve_subcommand(ctx: &CommandContext) -> Result<(&'static str, Vec<String>)> {
    if let Some(raw) = ctx.args.get("subcommand") {
        let canon = normalize_subcommand(raw);
        if canon.is_empty() {
            return Err(VoeError::Other(format!(
                "Unknown stagemanager subcommand: '{}'.  Use 'voe stagemanager help'.",
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
                    "Unknown stagemanager subcommand: '{}'.  Type 'help'.",
                    first
                )));
            }
            return Ok((canon, tokens.into_iter().skip(1).collect()));
        }
    }

    Ok(("list", Vec::new()))
}

/// Read an optional `usize` from either the named CLI key or the first
/// shell positional token.
fn resolve_usize(ctx: &CommandContext, key: &str, shell_tokens: &[String]) -> Option<usize> {
    if let Some(raw) = ctx.args.get(key) {
        return raw.parse().ok();
    }
    shell_tokens.first().and_then(|s| s.parse().ok())
}

fn cmd_list(repo: &dyn voe_repo_api::Repository) -> CommandResult {
    let staged = compute_staged(repo)?;
    print_staged(&staged);
    Ok(())
}

fn cmd_add(
    ctx: &CommandContext,
    n: usize,
    root: &Path,
    repo: &dyn voe_repo_api::Repository,
) -> CommandResult {
    let unstaged = compute_unstaged(root, repo)?;
    if n == 0 || n > unstaged.len() {
        return Err(VoeError::Other(format!(
            "Index out of range (have {} unstaged, 1-based)",
            unstaged.len()
        )));
    }
    let mask = &unstaged[n - 1];
    let new_oid = repo.commit_store().store_mask(mask)?;
    let mut add_failed = false;
    for change in &mask.changes {
        let path_str = change.location.path().to_string_lossy().into_owned();
        if let Err(e) = repo.index_store().add_mask(&path_str, &new_oid) {
            eprintln!("{}", format!("Failed to add mask to index: {}", e).red());
            add_failed = true;
        }
    }
    if !add_failed {
        println!("  {} {}", "Staged".green().bold(), new_oid);
    }
    let _ = ctx;
    Ok(())
}

fn cmd_remove(
    ctx: &CommandContext,
    n: usize,
    repo: &dyn voe_repo_api::Repository,
) -> CommandResult {
    let staged = compute_staged(repo)?;
    if n == 0 || n > staged.len() {
        return Err(VoeError::Other(format!(
            "Index out of range (have {} staged, 1-based)",
            staged.len()
        )));
    }
    let old_oid = staged[n - 1].0.clone();
    let index_state = repo.index_store().load()?;
    let mut paths_to_clean: Vec<String> = Vec::new();
    for entry in &index_state.entries {
        if entry.mask_ids.contains(&old_oid) {
            paths_to_clean.push(entry.path.clone());
        }
    }
    let mut remove_failed = false;
    for p in paths_to_clean {
        if let Err(e) = repo.index_store().remove_mask(&p, &old_oid) {
            eprintln!(
                "{}",
                format!("Failed to remove mask from index: {}", e).red()
            );
            remove_failed = true;
        }
    }
    if !remove_failed {
        println!("  {} {}", "Unstaged".yellow().bold(), old_oid);
    }
    let _ = ctx;
    Ok(())
}

fn cmd_reload(root: &Path, repo: &dyn voe_repo_api::Repository) -> CommandResult {
    let _ = compute_staged(repo)?;
    let _ = compute_unstaged(root, repo)?;
    println!("{}", "Reloaded.".green().bold());
    Ok(())
}

fn cmd_help() {
    println!();
    println!("{}", "Stagemanager sub-commands:".bold());
    let rows: &[(&str, &str)] = &[
        ("list, ls, l", "Show every mask currently staged (index)"),
        (
            "add <n>, stage <n>, a",
            "Stage the n-th unstaged mask (1-based)",
        ),
        (
            "remove <n>, unstage <n>, rm <n>, r",
            "Remove the n-th staged mask from the index",
        ),
        ("reload, refresh", "Re-scan and reload the index view"),
        ("help, h, ?", "Show this help"),
    ];
    let widest = rows.iter().map(|(c, _)| c.len()).max().unwrap_or(0);
    for (cmd, desc) in rows {
        println!("  {:<width$}  {}", cmd.bold().cyan(), desc, width = widest);
    }
    println!(
        "  {:<width$}  (or 'voe shell' → 'stagemanager add 3')",
        "Usage: voe stagemanager <sub-command> ...",
        width = widest,
    );
    println!();
}

// Silence unused-warning for shared helpers that are re-exported above
// for future stagemanager subcommands (e.g. list-all with unstaged preview).
#[allow(dead_code)]
fn _shared_not_used_here() {
    let _ = format_action;
    let _ = summarize_mask;
    let _ = print_unstaged;
}

impl Command for StagemanagerCommand {
    fn info(&self) -> CommandInfo {
        CommandInfo {
            name: "stagemanager",
            description: "Manage the staging area (index): stage and unstage masks",
            usage: "voe stagemanager <list|add|remove|reload>",
            examples: &["voe stagemanager list", "voe stagemanager add 1", "voe stagemanager remove 2"],
            aliases: &["stagemgr"],
        }
    }

    fn execute(&self, ctx: &mut CommandContext) -> CommandResult {
        let (subcmd, shell_tokens) = resolve_subcommand(ctx)?;
        let (root, repo) = resolve_repo(ctx)?;

        match subcmd {
            "list" => cmd_list(repo.as_ref()),
            "help" => {
                cmd_help();
                Ok(())
            }
            "reload" => cmd_reload(&root, repo.as_ref()),
            "add" => {
                let n = resolve_usize(ctx, "n", &shell_tokens).ok_or_else(|| {
                    VoeError::Other("stagemanager add requires a 1-based index argument".to_string())
                })?;
                cmd_add(ctx, n, &root, repo.as_ref())
            }
            "remove" => {
                let n = resolve_usize(ctx, "n", &shell_tokens).ok_or_else(|| {
                    VoeError::Other("stagemanager remove requires a 1-based index argument".to_string())
                })?;
                cmd_remove(ctx, n, repo.as_ref())
            }
            other => Err(VoeError::Other(format!(
                "Unknown stagemanager subcommand: '{}'.  Use 'voe stagemanager help'.",
                other
            ))),
        }
    }
}
