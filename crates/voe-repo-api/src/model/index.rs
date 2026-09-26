use serde::{Deserialize, Serialize};

use voe_types::error::Result;
use voe_types::object::ObjectId;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub path: String,
    pub mask_ids: Vec<ObjectId>,
}

impl IndexEntry {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            mask_ids: Vec::new(),
        }
    }

    pub fn with_mask(path: impl Into<String>, mask_id: ObjectId) -> Self {
        Self {
            path: path.into(),
            mask_ids: vec![mask_id],
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexState {
    pub entries: Vec<IndexEntry>,
}

impl IndexState {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn add_entry(&mut self, entry: IndexEntry) {
        if let Some(existing) = self.entries.iter_mut().find(|e| e.path == entry.path) {
            existing.mask_ids.extend(entry.mask_ids);
        } else {
            self.entries.push(entry);
        }
    }

    pub fn all_mask_ids(&self) -> Vec<ObjectId> {
        let mut ids: Vec<ObjectId> = Vec::new();
        for e in &self.entries {
            ids.extend(e.mask_ids.clone());
        }
        ids
    }

    /// Remove a specific mask id from the entry for `path`.  If the entry
    /// ends up with no masks left, it is removed entirely.  Returns `true`
    /// if any change was made, `false` if the path or mask id was absent.
    pub fn remove_mask(&mut self, path: &str, mask_id: &ObjectId) -> bool {
        let idx = match self.entries.iter().position(|e| e.path == path) {
            Some(i) => i,
            None => return false,
        };
        let entry = &mut self.entries[idx];
        let before = entry.mask_ids.len();
        entry.mask_ids.retain(|id| id != mask_id);
        if entry.mask_ids.is_empty() {
            self.entries.remove(idx);
            true
        } else {
            before != entry.mask_ids.len()
        }
    }
}

pub trait IndexStore: Send + Sync {
    fn load(&self) -> Result<IndexState>;
    fn save(&self, state: &IndexState) -> Result<()>;
    fn add_mask(&self, path: &str, mask_id: &ObjectId) -> Result<()>;
    fn remove_mask(&self, path: &str, mask_id: &ObjectId) -> Result<()>;
    fn clear(&self) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(s: &str) -> ObjectId {
        ObjectId::new(s)
    }

    #[test]
    fn remove_mask_returns_false_when_path_absent() {
        let mut state = IndexState::new();
        assert!(!state.remove_mask("a.txt", &oid("m1")));
        assert!(state.is_empty());
    }

    #[test]
    fn remove_mask_returns_false_when_mask_absent() {
        let mut state = IndexState::with_masks("a.txt", vec![oid("m1")]);
        assert!(!state.remove_mask("a.txt", &oid("m2")));
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0].mask_ids.len(), 1);
    }

    #[test]
    fn remove_mask_removes_entry_when_last_mask_goes() {
        let mut state = IndexState::with_masks("a.txt", vec![oid("m1")]);
        assert!(state.remove_mask("a.txt", &oid("m1")));
        assert!(state.is_empty());
    }

    #[test]
    fn remove_mask_keeps_entry_when_more_masks_remain() {
        let mut state = IndexState::with_masks("a.txt", vec![oid("m1"), oid("m2"), oid("m3")]);
        assert!(state.remove_mask("a.txt", &oid("m2")));
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0].mask_ids, vec![oid("m1"), oid("m3")]);
    }

    #[test]
    fn remove_mask_only_affects_target_path() {
        let mut state = IndexState::new();
        state.add_entry(IndexEntry::with_mask("a.txt", oid("shared")));
        state.add_entry(IndexEntry::with_mask("b.txt", oid("shared")));
        state.add_entry(IndexEntry::with_mask("b.txt", oid("other")));

        assert!(state.remove_mask("a.txt", &oid("shared")));
        // a.txt entry should be gone (it had only the removed mask).
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0].path, "b.txt");
        assert_eq!(state.entries[0].mask_ids, vec![oid("shared"), oid("other")]);
    }
}

impl IndexState {
    /// Convenience constructor used only in tests — creates an IndexState
    /// with a single entry for `path` holding all of the given `mask_ids`.
    #[cfg(test)]
    fn with_masks(path: impl Into<String>, mask_ids: Vec<ObjectId>) -> Self {
        Self {
            entries: vec![IndexEntry {
                path: path.into(),
                mask_ids,
            }],
        }
    }
}
