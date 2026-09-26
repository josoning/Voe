//! Shared mask data / listing utilities consumed by both the `stagemanager`
//! and `maskmanager` builtins.  These functions own no command dispatch
//! logic — they only compute, summarize, and print mask collections.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use owo_colors::OwoColorize;

use voe_mask::{ChunkMask, MaskContent, SimpleMaskResolver};
use voe_repo_api::model::commit::MaskObject;
use voe_repo_api::snapshot::SnapshotEngine;
use voe_types::error::Result;
use voe_types::object::ObjectId;

/// A visual style hint for a mask summary row — drives coloring in the
/// terminal output.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ActionStyle {
    Modified,
    ModifiedMulti,
    Deleted,
    Unknown,
}

/// Single-row summary of a [`ChunkMask`] for `list` command output.
pub struct MaskSummary {
    pub action: String,
    pub action_style: ActionStyle,
    pub path: String,
    pub extra_changes: usize,
    pub change_count: usize,
}

/// Classify the change content of a [`ChunkMask`] into a visual summary.
pub fn summarize_mask(mask: &ChunkMask) -> MaskSummary {
    let change_count = mask.changes.len();
    let path = mask
        .changes
        .first()
        .map(|c| c.location.path().clone())
        .unwrap_or_else(|| PathBuf::from("/"));
    let rel = path.to_string_lossy().trim_start_matches('/').to_string();

    let (action, action_style) = match mask.changes.first() {
        Some(c) => match &c.content {
            MaskContent::Full(_) | MaskContent::Chunks(_) => {
                if change_count == 1 {
                    ("M".to_string(), ActionStyle::Modified)
                } else {
                    ("M+".to_string(), ActionStyle::ModifiedMulti)
                }
            }
            MaskContent::Deleted => ("D".to_string(), ActionStyle::Deleted),
        },
        None => ("?".to_string(), ActionStyle::Unknown),
    };

    MaskSummary {
        action,
        action_style,
        path: rel,
        extra_changes: change_count.saturating_sub(1),
        change_count,
    }
}

/// Apply ANSI color to an action string based on its style.
pub fn format_action(action: &str, style: ActionStyle) -> String {
    match style {
        ActionStyle::Modified => action.green().bold().to_string(),
        ActionStyle::ModifiedMulti => action.green().bold().to_string(),
        ActionStyle::Deleted => action.red().bold().to_string(),
        ActionStyle::Unknown => action.yellow().bold().to_string(),
    }
}

/// Compute every mask that differs from HEAD but is not fully staged.
pub fn compute_unstaged(
    _root: &Path,
    repo: &dyn voe_repo_api::Repository,
) -> Result<Vec<ChunkMask>> {
    repo.config_manager().refresh()?;
    let cm = repo.config_manager();
    let cs = repo.commit_store();
    let refs = repo.ref_store();

    let parent_snapshot = match refs.get_head()? {
        Some(head) if !head.is_null() => {
            let resolver = SimpleMaskResolver;
            let engine = SnapshotEngine::new(cs, repo.chunk_store(), &resolver);
            engine.snapshot_from(&head).unwrap_or_default()
        }
        _ => HashMap::new(),
    };

    let working = repo.read_working_tree()?;
    let all_masks = voe_mask::diff_to_masks(&parent_snapshot, &working);

    let index_state = repo.index_store().load()?;

    let mut staged_contents: HashMap<String, Vec<MaskContent>> = HashMap::new();
    for entry in &index_state.entries {
        for mask_id in &entry.mask_ids {
            if let Ok(MaskObject::Chunk(staged_mask)) = cs.retrieve_mask(mask_id) {
                for change in &staged_mask.changes {
                    let path = change.location.path().to_string_lossy().into_owned();
                    staged_contents
                        .entry(path)
                        .or_default()
                        .push(change.content.clone());
                }
            }
        }
    }

    let mut unstaged: Vec<ChunkMask> = Vec::new();
    for mask in all_masks {
        let rel = mask
            .changes
            .first()
            .map(|c| {
                c.location
                    .path()
                    .to_string_lossy()
                    .trim_start_matches('/')
                    .to_string()
            })
            .unwrap_or_default();

        if rel == "voeconfig.toml" {
            continue;
        }
        if cm.is_ignored(&rel) {
            continue;
        }

        let all_staged = mask.changes.iter().all(|change| {
            let path = change.location.path().to_string_lossy().into_owned();
            staged_contents
                .get(&path)
                .map(|contents| contents.contains(&change.content))
                .unwrap_or(false)
        });
        if all_staged {
            continue;
        }
        unstaged.push(mask);
    }

    Ok(unstaged)
}

/// Return every mask currently tracked by the index (the "staged set").
pub fn compute_staged(repo: &dyn voe_repo_api::Repository) -> Result<Vec<(ObjectId, ChunkMask)>> {
    let cs = repo.commit_store();
    let index_state = repo.index_store().load()?;
    let mut result: Vec<(ObjectId, ChunkMask)> = Vec::new();

    for entry in &index_state.entries {
        for mask_id in &entry.mask_ids {
            if let Ok(MaskObject::Chunk(mask)) = cs.retrieve_mask(mask_id) {
                result.push((mask_id.clone(), mask));
            }
        }
    }

    Ok(result)
}

pub fn print_unstaged(unstaged: &[ChunkMask]) {
    let header = format!("=== Unstaged ({}) ===", unstaged.len().bold());
    println!("{}", header.bold().yellow());
    if unstaged.is_empty() {
        println!("  {}", "(none)".dimmed());
    } else {
        for (i, mask) in unstaged.iter().enumerate() {
            let s = summarize_mask(mask);
            let action = format_action(&s.action, s.action_style);
            let path = s.path.bold().to_string();
            let extra = if s.extra_changes > 0 {
                format!(" {} more changes", format!("(+{})", s.extra_changes).blue())
            } else {
                String::new()
            };
            let idx = format!("[{:>3}]", i + 1).dimmed().to_string();
            let oid = mask.id.to_string().dimmed().to_string();
            println!("  {} {}  {}{}  ({})", idx, action, path, extra, oid);
        }
    }
}

pub fn print_staged(staged: &[(ObjectId, ChunkMask)]) {
    println!();
    let header = format!("=== Staged ({}) ===", staged.len().bold());
    println!("{}", header.bold().green());
    if staged.is_empty() {
        println!("  {}", "(none)".dimmed());
    } else {
        for (i, (oid, mask)) in staged.iter().enumerate() {
            let s = summarize_mask(mask);
            let action = format_action(&s.action, s.action_style);
            let path = s.path.bold().to_string();
            let extra = if s.extra_changes > 0 {
                format!(" {} more changes", format!("(+{})", s.extra_changes).blue())
            } else {
                String::new()
            };
            let idx = format!("[{:>3}]", i + 1).dimmed().to_string();
            let oid_str = oid.to_string().dimmed().to_string();
            let changes_label = format!(
                "{} change{}",
                s.change_count,
                if s.change_count == 1 { "" } else { "s" }
            );
            println!(
                "  {} {}  {}{}  {} ({})",
                idx,
                action,
                path,
                extra,
                changes_label.bold(),
                oid_str
            );
        }
    }
}

/// Print unstaged and staged in a single grouped view — used by
/// `maskmanager list`.  Prepend each row with a source marker so the
/// user can tell which list an index came from.
pub fn print_all_masks(unstaged: &[ChunkMask], staged: &[(ObjectId, ChunkMask)]) {
    let header = format!(
        "=== All masks ({}) ===",
        (unstaged.len() + staged.len()).bold()
    );
    println!("{}", header.bold().cyan());

    // Merge into a single ordered list so the printed [n] index is the
    // same one `maskmanager split` / `merge` will use.  Unstaged come
    // first, then staged.
    let mut all: Vec<(&str, Option<&ObjectId>, &ChunkMask)> = Vec::new();
    for m in unstaged {
        all.push(("unstaged", None, m));
    }
    for (oid, m) in staged {
        all.push(("staged", Some(oid), m));
    }

    if all.is_empty() {
        println!("  {}", "(none)".dimmed());
        return;
    }

    for (i, (source, oid, mask)) in all.iter().enumerate() {
        let s = summarize_mask(mask);
        let action = format_action(&s.action, s.action_style);
        let path = s.path.bold().to_string();
        let extra = if s.extra_changes > 0 {
            format!(" {} more changes", format!("(+{})", s.extra_changes).blue())
        } else {
            String::new()
        };
        let idx = format!("[{:>3}]", i + 1).dimmed().to_string();
        let src_mark = match *source {
            "staged" => "staged".green().to_string(),
            _ => "unstaged".yellow().to_string(),
        };
        let oid_display = oid
            .map(|o| o.to_string())
            .unwrap_or_else(|| mask.id.to_string());
        println!(
            "  {} {}  {}{}  {}  ({})",
            idx,
            action,
            path,
            extra,
            src_mark.dimmed(),
            oid_display.dimmed()
        );
    }
}

pub fn refresh_both(
    root: &Path,
    repo: &dyn voe_repo_api::Repository,
    unstaged: &mut Vec<ChunkMask>,
    staged: &mut Vec<(ObjectId, ChunkMask)>,
) {
    if let Ok(m) = compute_unstaged(root, repo) {
        *unstaged = m;
    } else {
        eprintln!("{}", "Warning: failed to refresh unstaged changes".yellow());
    }
    if let Ok(m) = compute_staged(repo) {
        *staged = m;
    } else {
        eprintln!("{}", "Warning: failed to refresh staged masks".yellow());
    }
}
