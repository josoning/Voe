use std::collections::HashMap;
use std::path::PathBuf;

use voe_mask::SimpleMaskResolver;
use voe_repo_api::command::CommandContext;
use voe_repo_api::snapshot::SnapshotEngine;
use voe_types::error::{Result, VoeError};
use voe_types::object::{ObjectId, ObjectKind};

/// Locate the repository root from the current working directory and open it.
/// Returns both the absolute root path and a boxed repository handle so
/// callers can consume them independently.
pub fn resolve_repo(ctx: &CommandContext) -> Result<(PathBuf, Box<dyn voe_repo_api::Repository>)> {
    let repo_manager = ctx
        .repo_manager
        .ok_or_else(|| VoeError::Other("RepoManager not available".to_string()))?;

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let root = repo_manager
        .find_root(&cwd)
        .ok_or_else(|| VoeError::RepoNotFound { path: cwd.clone() })?;

    let repo = repo_manager.open(root.clone())?;
    Ok((root, repo))
}

/// Resolve a user-supplied target string into a commit `ObjectId`.
///
/// Accepts:
///   - The literal `"HEAD"` (and descendants `HEAD~N`, `HEAD^`, `HEAD^^`)
///   - A full OID that already exists as a commit
///   - A short prefix uniquely identifying a commit in the object store
///   - Any commit OID followed by `~N` to walk back N generations (e.g. `abc123~2`)
///
/// Ambiguous prefixes and non-commit objects produce descriptive errors.
pub fn resolve_oid(repo: &dyn voe_repo_api::Repository, target: &str) -> Result<ObjectId> {
    // Split off any `~N` or `^N` ancestor suffix so we can resolve the
    // base ref first and then walk the parent chain.
    let (base, generations) = split_ancestor_suffix(target);

    let mut oid = resolve_base_oid(repo, base)?;

    if generations > 0 {
        oid = walk_ancestors(repo, &oid, generations)?;
    }

    Ok(oid)
}

/// Split `"HEAD~3"` → (`"HEAD"`, `3`) and `"abc123^"` → (`"abc123"`, `1`).
/// Returns `(target, 0)` when no ancestor suffix is present.
fn split_ancestor_suffix(target: &str) -> (&str, u32) {
    // Try `~N` first (explicit numeric).
    if let Some(idx) = target.rfind('~') {
        let n = target[idx + 1..].parse::<u32>().ok();
        if let Some(n) = n {
            return (&target[..idx], n);
        }
    }
    // Try `^N` or bare `^` (equivalent to `^1`).
    if let Some(idx) = target.rfind('^') {
        let suffix = &target[idx + 1..];
        let n = if suffix.is_empty() {
            1
        } else {
            suffix.parse::<u32>().unwrap_or(0)
        };
        if n > 0 {
            return (&target[..idx], n);
        }
    }
    (target, 0)
}

/// Resolve the base portion of a ref (`"HEAD"`, a full OID, or a short prefix).
fn resolve_base_oid(repo: &dyn voe_repo_api::Repository, base: &str) -> Result<ObjectId> {
    if base == "HEAD" {
        return repo
            .ref_store()
            .get_head()?
            .ok_or_else(|| VoeError::Other("No HEAD set — nothing to resolve".to_string()));
    }

    let candidate = ObjectId::from(base);
    if candidate.is_valid() && repo.object_store().exists(&candidate)? {
        let obj = repo.object_store().retrieve(&candidate)?;
        if obj.kind == ObjectKind::Commit {
            return Ok(candidate);
        }
        return Err(VoeError::Other(format!(
            "Object {} is not a commit (kind={:?})",
            candidate, obj.kind
        )));
    }

    let mut matches: Vec<ObjectId> = Vec::new();
    for oid in repo.object_store().list()? {
        if oid.as_str().starts_with(base) {
            let obj = repo.object_store().retrieve(&oid)?;
            if obj.kind == ObjectKind::Commit {
                matches.push(oid);
            }
        }
    }

    if matches.is_empty() {
        return Err(VoeError::Other(format!(
            "No commit found matching prefix '{}'",
            base
        )));
    }
    if matches.len() > 1 {
        let ambiguous: Vec<String> = matches.iter().map(|o| o.to_string()).collect();
        return Err(VoeError::Other(format!(
            "Ambiguous prefix '{}' — matches {} commits:\n  {}",
            base,
            matches.len(),
            ambiguous.join("\n  ")
        )));
    }

    Ok(matches.pop().unwrap())
}

/// Walk the first parent N times, returning the ancestor OID.
/// Returns an error if the chain runs out of parents before N steps.
fn walk_ancestors(
    repo: &dyn voe_repo_api::Repository,
    start: &ObjectId,
    generations: u32,
) -> Result<ObjectId> {
    let mut current = start.clone();
    for step in 0..generations {
        let commit = repo.commit_store().retrieve_commit(&current)?;
        let parent = commit.parents.first().ok_or_else(|| {
            VoeError::Other(format!(
                "Commit {} has no parent — cannot walk {} generation(s) back from {}",
                current, generations, start
            ))
        })?;
        current = parent.clone();
        // Silence unused loop-variable warning if we ever need it for
        // diagnostic messages in the future.
        let _ = step;
    }
    Ok(current)
}

/// Compute the working-tree snapshot that a given commit OID produces.
/// Returns a map from snapshot-keyed paths (e.g. `/src/main.rs`) to file bytes.
pub fn snapshot_for(
    repo: &dyn voe_repo_api::Repository,
    commit_oid: &ObjectId,
) -> Result<HashMap<PathBuf, Vec<u8>>> {
    let resolver = SimpleMaskResolver;
    let engine = SnapshotEngine::new(repo.commit_store(), repo.chunk_store(), &resolver);
    engine.snapshot_from(commit_oid)
}

// ---------------------------------------------------------------------------
// PathFilter — moved here from the former `add` builtin (now removed).
// Kept as a reusable utility for future `stagemanager add --path` style
// operations that filter unstaged masks by filesystem path.

use std::path::Path as StdPath;

/// Pure path matcher used to decide which mask paths should be staged.
/// The matcher is filesystem-free so it can be unit-tested in isolation;
/// the only filesystem-dependent step is translating a user-supplied
/// path segment (e.g. `"src"`) into a trailing-slash directory prefix,
/// which `PathFilter::build` takes care of before constructing this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathFilter {
    /// When `true`, every path is considered a match (user said `"."` or
    /// gave no paths at all).
    filter_all: bool,
    /// Normalized prefixes. Entries ending with `'/'` match anything underneath
    /// that directory; entries without a trailing slash match a single file
    /// exactly. All prefixes start with `'/'`, mirroring how `ChunkMask`
    /// stores paths.
    prefixes: Vec<String>,
}

impl PathFilter {
    /// Build a filter from an optional raw `paths` argument (paths joined
    /// by `\x1f`, as produced by the dispatcher).  The `root` is consulted
    /// to decide whether each user path should be treated as a directory
    /// prefix (trailing `/`) or a file exact-match.
    pub fn build(root: &StdPath, raw_paths: Option<&str>) -> Self {
        let explicit_paths: Vec<String> = raw_paths
            .map(|s| {
                s.split('\x1f')
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let filter_all =
            explicit_paths.is_empty() || explicit_paths.iter().any(|p| p == "." || p == "./");

        let normalized_prefixes: Vec<String> = explicit_paths
            .iter()
            .filter(|p| !filter_all || *p == "." || *p == "./")
            .map(|p| {
                let mut np = if p.starts_with('/') {
                    p.clone()
                } else {
                    format!("/{}", p.trim_start_matches("./"))
                };
                // Normalize directory paths so both "src" and "src/" match
                // anything under "/src/".
                if !np.ends_with('/') && np.len() > 1 {
                    let candidate = root.join(p);
                    if candidate.is_dir() {
                        np.push('/');
                    }
                }
                np
            })
            .filter(|p| p != "/")
            .collect();

        Self {
            filter_all,
            prefixes: normalized_prefixes,
        }
    }

    /// Returns `true` when `path` should be staged according to the user's
    /// path specification.
    pub fn matches(&self, path: &str) -> bool {
        if self.filter_all {
            return true;
        }
        self.prefixes.iter().any(|prefix| {
            if prefix.ends_with('/') {
                path == prefix.trim_end_matches('/') || path.starts_with(prefix.as_str())
            } else {
                path == *prefix
            }
        })
    }

    /// Returns the number of explicit prefixes (useful for tests / debug).
    pub fn prefix_count(&self) -> usize {
        self.prefixes.len()
    }

    /// `true` when the filter will match every path.
    pub fn is_filter_all(&self) -> bool {
        self.filter_all
    }
}

#[cfg(test)]
mod path_filter_tests {
    use super::*;

    #[test]
    fn filter_all_when_no_paths() {
        let f = PathFilter::build(StdPath::new("/repo"), None);
        assert!(f.is_filter_all());
        assert_eq!(f.prefix_count(), 0);
        assert!(f.matches("/any/file.rs"));
        assert!(f.matches("/deep/nested/file.txt"));
    }

    #[test]
    fn filter_all_when_path_is_dot() {
        let f = PathFilter::build(StdPath::new("/repo"), Some("."));
        assert!(f.is_filter_all());
        assert!(f.matches("/any/file.rs"));
    }

    #[test]
    fn exact_file_match() {
        let f = PathFilter {
            filter_all: false,
            prefixes: vec!["/src/main.rs".to_string()],
        };
        assert!(f.matches("/src/main.rs"));
        assert!(!f.matches("/src/lib.rs"));
    }

    #[test]
    fn directory_prefix_match() {
        let f = PathFilter {
            filter_all: false,
            prefixes: vec!["/src/".to_string()],
        };
        assert!(f.matches("/src/main.rs"));
        assert!(f.matches("/src/lib/utils.rs"));
        assert!(!f.matches("/src.rs"));
    }

    #[test]
    fn multiple_prefixes() {
        let f = PathFilter {
            filter_all: false,
            prefixes: vec!["/src/main.rs".to_string(), "/tests/".to_string()],
        };
        assert!(f.matches("/src/main.rs"));
        assert!(!f.matches("/src/lib.rs"));
        assert!(f.matches("/tests/test_a.rs"));
    }
}
