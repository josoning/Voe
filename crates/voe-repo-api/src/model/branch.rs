//! Branch model for VOE.
//!
//! A branch identifier follows the format `name@tag1@tag2`. The leading part
//! before the first `@` is the branch *name* (may contain `/` for sub-branches).
//! Every segment after an `@` is a *label* (at least one label is always present;
//! when none is given, VOE injects the implicit `default` label).
//!
//! Reserved labels:
//! * `mainline`  - exactly one branch in the entire repo may carry this label.
//! * `release`   - append-only release lines (e.g. `3.1@release`).
//! * `temp`      - temporary branches VOE creates automatically.
//!
//! Branch names must not contain `#`, `@`, or `/`. Sub-branches use `/` in the
//! name itself (e.g. `main/debug@dbg`), which is distinct from the forbidden
//! `@`/`#` characters — `@` separates name from labels, `#` marks aliases.
//!
//! Aliases start with `#` and are unique across the repository.

use std::fmt;

use serde::{Deserialize, Serialize};

use voe_types::error::{Result, VoeError};

use voe_types::author::Timestamp;
use voe_types::object::{ObjectId, ObjectKind, VoeObject};

/// Labels that carry special semantics and may only be attached by designated
/// VOE commands or by VOE itself during automatic operations.
pub const RESERVED_LABELS: &[&str] = &["mainline", "release", "temp"];

/// The label implicitly added when the user writes a bare branch name without
/// any `@` separator.
pub const DEFAULT_LABEL: &str = "default";

/// Characters forbidden anywhere in the branch *name* portion (rule 10).
pub const FORBIDDEN_NAME_CHARS: &[char] = &['#', '@'];

/// A fully-parsed branch identifier consisting of a name and one or more labels.
///
/// Equality / hashing treats two `BranchId` values as identical **only** when
/// the names match **and** the label multisets match — ordering of labels is
/// irrelevant because labels are unordered attributes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchId {
    pub name: String,
    pub labels: Vec<String>,
}

// Manually implement PartialEq/Eq/Hash to ignore label order.
impl PartialEq for BranchId {
    fn eq(&self, other: &Self) -> bool {
        if self.name != other.name {
            return false;
        }
        let mut a = self.labels.clone();
        let mut b = other.labels.clone();
        a.sort();
        b.sort();
        a == b
    }
}

impl Eq for BranchId {}

impl std::hash::Hash for BranchId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        let mut labels = self.labels.clone();
        labels.sort();
        labels.hash(state);
    }
}

impl BranchId {
    /// Parse a string into a `BranchId`.
    ///
    /// Rules:
    /// * If no `@` is present → treated as a bare name, `default` label is
    ///   injected automatically.
    /// * If at least one `@` is present → the first segment becomes `name`,
    ///   every following segment becomes a label. Empty labels are rejected.
    /// * Labels are lower-cased at parse time for equality comparison.
    ///
    /// Example: `parse("main/debug@dbg")` → name=`main/debug`, labels=`["dbg"]`.
    /// Example: `parse("main@mainline")` → name=`main`, labels=`["mainline"]`.
    pub fn parse(input: &str) -> Result<Self> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(VoeError::InvalidBranchId(
                "branch identifier must not be empty".to_string(),
            ));
        }

        let parts: Vec<&str> = trimmed.split('@').collect();
        if parts.is_empty() {
            return Err(VoeError::InvalidBranchId("empty split result".to_string()));
        }

        let name = parts[0];
        if name.is_empty() {
            return Err(VoeError::InvalidBranchId(
                "branch name before '@' must not be empty".to_string(),
            ));
        }

        let labels: Vec<String> = if parts.len() == 1 {
            vec![DEFAULT_LABEL.to_string()]
        } else {
            let mut out = Vec::with_capacity(parts.len() - 1);
            for raw in &parts[1..] {
                if raw.is_empty() {
                    return Err(VoeError::InvalidBranchId(format!(
                        "empty label in '{}'",
                        trimmed
                    )));
                }
                // Labels are case-insensitive for equality but we preserve the
                // original casing.
                out.push((*raw).to_string());
            }
            out
        };

        let id = Self {
            name: name.to_string(),
            labels,
        };
        id.validate_name()?;
        id.validate_labels()?;
        Ok(id)
    }

    /// Validate that the branch `name` conforms to rule 10.
    ///
    /// The check is applied to the full name including any `/` path separators
    /// (sub-branch syntax). `/` is **allowed** here; only `#` and `@`
    /// are forbidden in the name portion.
    pub fn validate_name(&self) -> Result<()> {
        if self.name.is_empty() {
            return Err(VoeError::InvalidBranchName {
                name: self.name.clone(),
                reason: "name must not be empty".to_string(),
            });
        }
        for c in self.name.chars() {
            if FORBIDDEN_NAME_CHARS.contains(&c) {
                return Err(VoeError::InvalidBranchName {
                    name: self.name.clone(),
                    reason: format!("character '{}' is forbidden in branch name", c),
                });
            }
        }
        Ok(())
    }

    /// Validate every label — none may be empty, and each must be non-empty
    /// after trim.
    pub fn validate_labels(&self) -> Result<()> {
        if self.labels.is_empty() {
            return Err(VoeError::InvalidBranchId(
                "at least one label is required".to_string(),
            ));
        }
        for lbl in &self.labels {
            if lbl.is_empty() || lbl.trim().is_empty() {
                return Err(VoeError::InvalidBranchId(
                    "labels must not be empty".to_string(),
                ));
            }
            if lbl.chars().any(|c| c == '@' || c == '#') {
                return Err(VoeError::InvalidBranchId(format!(
                    "label '{}' contains forbidden character",
                    lbl
                )));
            }
        }
        Ok(())
    }

    /// Returns `true` when this branch carries the reserved `mainline` label.
    pub fn has_mainline(&self) -> bool {
        self.labels
            .iter()
            .any(|l| l.eq_ignore_ascii_case("mainline"))
    }

    /// Returns `true` when this branch carries the reserved `release` label.
    pub fn is_release(&self) -> bool {
        self.labels
            .iter()
            .any(|l| l.eq_ignore_ascii_case("release"))
    }

    /// Returns `true` when this branch carries the reserved `temp` label.
    pub fn has_temp(&self) -> bool {
        self.labels.iter().any(|l| l.eq_ignore_ascii_case("temp"))
    }

    /// Returns `true` when at least one of the reserved labels is present.
    pub fn has_reserved_label(&self) -> bool {
        self.labels
            .iter()
            .any(|l| RESERVED_LABELS.iter().any(|r| l.eq_ignore_ascii_case(r)))
    }

    /// Returns `true` when every label is reserved (e.g. bare `main@mainline`).
    pub fn labels_are_all_reserved(&self) -> bool {
        self.labels
            .iter()
            .all(|l| RESERVED_LABELS.iter().any(|r| l.eq_ignore_ascii_case(r)))
    }

    /// Render back to the canonical `name@label1@label2` form.
    /// Labels are sorted so the output is stable regardless of construction
    /// order — this matters for round-tripping and storage-key determinism.
    pub fn canonical(&self) -> String {
        let mut labels_sorted = self.labels.clone();
        labels_sorted.sort_by_key(|s| s.to_lowercase());
        format!("{}@{}", self.name, labels_sorted.join("@"))
    }

    /// Key used for filesystem/storage lookups.  Equivalent to `canonical()`.
    pub fn storage_key(&self) -> String {
        self.canonical()
    }

    /// Build the default mainline branch identifier (`main@mainline`).
    pub fn mainline_default() -> Self {
        Self {
            name: "main".to_string(),
            labels: vec!["mainline".to_string()],
        }
    }

    /// Build a temp sub-branch identifier given a parent `name`.  The temp
    /// label is added alongside any non-reserved labels on the parent so the
    /// caller can decide whether to inherit them.
    pub fn temp_sub_branch(parent_name: &str, non_reserved_parent_labels: &[String]) -> Self {
        let mut labels = non_reserved_parent_labels.to_vec();
        if !labels.iter().any(|l| l.eq_ignore_ascii_case("temp")) {
            labels.push("temp".to_string());
        }
        Self {
            name: parent_name.to_string(),
            labels,
        }
    }
}

impl fmt::Display for BranchId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.canonical())
    }
}

/// A VOE branch is a named, labelled pointer to a commit, along with metadata
/// such as its creator, optional parent branch (for sub-branches),
/// and aliases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Branch {
    pub id: BranchId,
    pub head: ObjectId,
    pub creator_device_id: String,
    pub parent_storage_key: Option<String>,
    pub aliases: Vec<Alias>,
    pub created_at: Timestamp,
    pub last_updated: Timestamp,
}

impl Branch {
    pub fn new(
        id: BranchId,
        head: ObjectId,
        creator_device_id: String,
        created_at: Timestamp,
    ) -> Self {
        Self {
            id,
            head,
            creator_device_id,
            parent_storage_key: None,
            aliases: Vec::new(),
            created_at,
            last_updated: created_at,
        }
    }

    /// Short-cut to check whether this branch is a release line.
    pub fn is_release(&self) -> bool {
        self.id.is_release()
    }
}

/// An alias for a branch.  Aliases start with `#` when displayed but the
/// stored `name` **must not** include the `#` — the prefix is implicit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alias {
    /// The alias text **without** the leading `#`.
    pub name: String,
}

impl Alias {
    /// Build an alias, stripping any leading `#` the caller may have included.
    pub fn new(name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        let stripped = name.strip_prefix('#').unwrap_or(&name).to_string();
        if stripped.is_empty() {
            return Err(VoeError::Branch(
                "alias must not be empty after stripping '#'".to_string(),
            ));
        }
        if stripped.contains('@') || stripped.contains('/') {
            return Err(VoeError::Branch(format!(
                "alias '{}' must not contain '@' or '/'",
                stripped
            )));
        }
        Ok(Self { name: stripped })
    }

    /// The display form (with leading `#`).
    pub fn display(&self) -> String {
        format!("#{}", self.name)
    }
}

impl fmt::Display for Alias {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.name)
    }
}

/// A **merge node** captures how two or more branches were combined.
///
/// Merge nodes are stored as `ObjectKind::Merge` VOE objects.  They sit in a
/// commit's `masks` list just like `ChunkMask` objects — this is how VOE
/// expresses mask-level merge provenance alongside the actual mask data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeNode {
    /// The primary branch (keeps its name/labels post-merge, rule 13).
    pub primary_branch_storage_key: String,
    /// Secondary branches that were merged into the primary.
    pub secondary_branch_storage_keys: Vec<String>,
    /// Commit ids that acted as the merge base for each secondary (one per
    /// `secondary_branch_storage_keys` entry, matching index).
    pub merge_bases: Vec<ObjectId>,
    /// Extra masks introduced to resolve conflicts during this merge — the
    /// "merge patch" masks referenced by rule 14.
    pub patch_mask_ids: Vec<ObjectId>,
    /// Human-readable merge message.
    pub message: String,
}

impl MergeNode {
    pub fn new(
        primary_branch_storage_key: String,
        secondary_branch_storage_keys: Vec<String>,
        merge_bases: Vec<ObjectId>,
    ) -> Self {
        Self {
            primary_branch_storage_key,
            secondary_branch_storage_keys,
            merge_bases,
            patch_mask_ids: Vec::new(),
            message: String::new(),
        }
    }

    /// Validate that `secondary_branch_storage_keys.len() == merge_bases.len()`.
    pub fn validate(&self) -> Result<()> {
        if self.secondary_branch_storage_keys.len() != self.merge_bases.len() {
            return Err(VoeError::MergeConflict(format!(
                "secondary branches ({}) and merge bases ({}) length mismatch",
                self.secondary_branch_storage_keys.len(),
                self.merge_bases.len()
            )));
        }
        Ok(())
    }

    pub fn with_patch_masks(mut self, ids: Vec<ObjectId>) -> Self {
        self.patch_mask_ids = ids;
        self
    }

    pub fn with_message(mut self, msg: impl Into<String>) -> Self {
        self.message = msg.into();
        self
    }

    pub fn to_voe_object(&self) -> Result<VoeObject> {
        let content = serde_json::to_vec(self)
            .map_err(|e| VoeError::Storage(format!("Failed to serialize MergeNode: {}", e)))?;
        Ok(VoeObject::new(ObjectKind::Merge, content))
    }

    pub fn from_voe_object(object: &VoeObject) -> Result<Self> {
        if object.kind != ObjectKind::Merge {
            return Err(VoeError::Storage(format!(
                "Expected Merge object, got {:?}",
                object.kind
            )));
        }
        serde_json::from_slice(&object.content)
            .map_err(|e| VoeError::Storage(format!("Failed to deserialize MergeNode: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // BranchId parsing
    // ------------------------------------------------------------------

    #[test]
    fn parse_bare_name_gets_default_label() {
        let id = BranchId::parse("main").unwrap();
        assert_eq!(id.name, "main");
        assert_eq!(id.labels, vec!["default".to_string()]);
    }

    #[test]
    fn parse_mainline_default() {
        let id = BranchId::parse("main@mainline").unwrap();
        assert_eq!(id.name, "main");
        assert_eq!(id.labels, vec!["mainline".to_string()]);
        assert!(id.has_mainline());
    }

    #[test]
    fn parse_multiple_labels() {
        let id = BranchId::parse("dev@fix@add").unwrap();
        assert_eq!(id.name, "dev");
        assert_eq!(id.labels.len(), 2);
    }

    #[test]
    fn parse_release_version_style() {
        let id = BranchId::parse("3.1@release").unwrap();
        assert_eq!(id.name, "3.1");
        assert!(id.is_release());
    }

    #[test]
    fn parse_sub_branch_keeps_slash_in_name() {
        let id = BranchId::parse("main/debug@dbg").unwrap();
        assert_eq!(id.name, "main/debug");
        assert_eq!(id.labels, vec!["dbg".to_string()]);
    }

    #[test]
    fn parse_rejects_empty() {
        assert!(BranchId::parse("").is_err());
    }

    #[test]
    fn parse_rejects_empty_label() {
        assert!(BranchId::parse("dev@").is_err());
        assert!(BranchId::parse("dev@@add").is_err());
    }

    #[test]
    fn parse_rejects_empty_name() {
        assert!(BranchId::parse("@mainline").is_err());
    }

    #[test]
    fn validate_name_rejects_hash() {
        // A bare name containing '#' is invalid (rule 10).
        assert!(BranchId::parse("dev#weird").is_err());
        // A sub-branch path may use '/' freely.
        assert!(BranchId::parse("main/debug@dbg").is_ok());
    }

    #[test]
    fn validate_at_is_separator_not_forbidden() {
        // '@' is the name/label separator, so `dev@weird` is perfectly valid:
        // name="dev", label="weird".
        assert!(BranchId::parse("dev@weird").is_ok());
    }

    // ------------------------------------------------------------------
    // BranchId equality is order-independent on labels
    // ------------------------------------------------------------------

    #[test]
    fn branch_id_equality_ignores_label_order() {
        let a = BranchId::parse("dev@fix@add").unwrap();
        let b = BranchId::parse("dev@add@fix").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.storage_key(), b.storage_key());
    }

    #[test]
    fn branch_id_canonical_is_stable() {
        let a = BranchId::parse("dev@zebra@alpha").unwrap();
        let b = BranchId::parse("dev@alpha@zebra").unwrap();
        assert_eq!(a.canonical(), b.canonical());
    }

    // ------------------------------------------------------------------
    // Alias
    // ------------------------------------------------------------------

    #[test]
    fn alias_strips_leading_hash() {
        let a = Alias::new("#feature").unwrap();
        assert_eq!(a.name, "feature");
        assert_eq!(a.display(), "#feature");
    }

    #[test]
    fn alias_rejects_at_and_slash() {
        assert!(Alias::new("#feat@weird").is_err());
        assert!(Alias::new("#feat/weird").is_err());
    }

    #[test]
    fn alias_rejects_empty() {
        assert!(Alias::new("").is_err());
        assert!(Alias::new("#").is_err());
    }
}
