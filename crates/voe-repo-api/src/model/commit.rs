use std::fmt;

use serde::{Deserialize, Serialize};

use voe_mask::ChunkMask;
use voe_types::error::{Result, VoeError};

pub use super::index::{IndexEntry, IndexState, IndexStore};
pub use super::refs::RefStore;
pub use super::tag::Tag;
pub use super::tree::{Tree, TreeEntry, TreeEntryType};
use voe_types::author::Author;
use voe_types::object::{ObjectId, ObjectKind, VoeObject};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitSignature {
    pub algorithm: String,
    pub key_fingerprint: String,
    pub signature: Vec<u8>,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    pub tree: ObjectId,
    pub parents: Vec<ObjectId>,
    pub author: Author,
    pub committer: Author,
    pub message: String,
    pub masks: Vec<ObjectId>,
    pub signature: Option<CommitSignature>,
}

impl Commit {
    pub fn new(
        tree: ObjectId,
        parents: Vec<ObjectId>,
        author: Author,
        committer: Author,
        message: String,
        masks: Vec<ObjectId>,
    ) -> Self {
        Self {
            tree,
            parents,
            author,
            committer,
            message,
            masks,
            signature: None,
        }
    }

    pub fn initial(tree: ObjectId, author: Author, message: String, masks: Vec<ObjectId>) -> Self {
        Self {
            tree,
            parents: Vec::new(),
            author: author.clone(),
            committer: author,
            message,
            masks,
            signature: None,
        }
    }

    pub fn with_masks(mut self, masks: Vec<ObjectId>) -> Self {
        self.masks = masks;
        self
    }

    pub fn with_signature(mut self, signature: CommitSignature) -> Self {
        self.signature = Some(signature);
        self
    }

    pub fn add_parent(&mut self, parent: ObjectId) {
        self.parents.push(parent);
    }

    pub fn is_signed(&self) -> bool {
        self.signature.is_some()
    }

    pub fn has_parent(&self) -> bool {
        !self.parents.is_empty()
    }

    pub fn to_voe_object(&self) -> Result<VoeObject> {
        let content = serde_json::to_vec(self)
            .map_err(|e| VoeError::Storage(format!("Failed to serialize Commit: {}", e)))?;
        Ok(VoeObject::new(ObjectKind::Commit, content))
    }

    pub fn from_voe_object(object: &VoeObject) -> Result<Self> {
        if object.kind != ObjectKind::Commit {
            return Err(VoeError::Storage(format!(
                "Expected Commit object, got {:?}",
                object.kind
            )));
        }
        serde_json::from_slice(&object.content)
            .map_err(|e| VoeError::Storage(format!("Failed to deserialize Commit: {}", e)))
    }
}

impl fmt::Display for Commit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "tree {} parents {:?} author {} committer {} masks {} message \"{}\"",
            self.tree,
            self.parents,
            self.author,
            self.committer,
            self.masks.len(),
            self.message
        )
    }
}

/// A wrapper that holds a mask stored in a commit's `masks` list.
/// Previously distinguished `ChunkMask` from `Separator`, but separators
/// are now just tagged masks, so this enum collapses to a single variant.
/// Kept as an enum for future extensibility and backward-compatible match
/// sites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaskObject {
    Chunk(ChunkMask),
}

impl MaskObject {
    pub fn as_mask(&self) -> &dyn voe_mask::Mask {
        match self {
            MaskObject::Chunk(m) => m,
        }
    }
}

pub trait CommitStore: Send + Sync {
    fn store_commit(&self, commit: &Commit) -> Result<ObjectId>;
    fn retrieve_commit(&self, id: &ObjectId) -> Result<Commit>;
    fn store_mask(&self, mask: &ChunkMask) -> Result<ObjectId>;
    fn retrieve_mask(&self, id: &ObjectId) -> Result<MaskObject>;
}
