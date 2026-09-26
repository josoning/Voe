use serde::{Deserialize, Serialize};

use voe_types::error::{Result, VoeError};

use voe_types::object::{ObjectId, ObjectKind, VoeObject};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tree {
    pub entries: Vec<TreeEntry>,
}

impl Tree {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn with_entries(entries: Vec<TreeEntry>) -> Self {
        Self { entries }
    }

    pub fn add_entry(&mut self, entry: TreeEntry) {
        self.entries.push(entry);
    }

    pub fn to_voe_object(&self) -> Result<VoeObject> {
        let content = serde_json::to_vec(self)
            .map_err(|e| VoeError::Storage(format!("Failed to serialize Tree: {}", e)))?;
        Ok(VoeObject::new(ObjectKind::Tree, content))
    }
}

impl Default for Tree {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeEntry {
    pub name: String,
    pub oid: ObjectId,
    pub entry_type: TreeEntryType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TreeEntryType {
    Blob,
    Tree,
    Mask,
}
