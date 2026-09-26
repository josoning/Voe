use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObjectId(pub String);

impl ObjectId {
    /// Sentinel value representing "no commit yet".  Used as the initial
    /// head of `main@mainline` right after `voe init`, before any commit
    /// has been created.
    pub const NULL: ObjectId = ObjectId(String::new());

    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn from_bytes(content: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(content);
        let hash = hasher.finalize();
        Self(hex::encode(hash))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_valid(&self) -> bool {
        !self.0.is_empty() && self.0.chars().all(|c| c.is_ascii_hexdigit())
    }

    /// Returns `true` when this is the `NULL` sentinel (no commit yet).
    pub fn is_null(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for ObjectId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for ObjectId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectKind {
    Blob,
    Tree,
    Commit,
    Tag,
    Merge,
}

impl fmt::Display for ObjectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObjectKind::Blob => write!(f, "blob"),
            ObjectKind::Tree => write!(f, "tree"),
            ObjectKind::Commit => write!(f, "commit"),
            ObjectKind::Tag => write!(f, "tag"),
            ObjectKind::Merge => write!(f, "merge"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoeObject {
    pub id: ObjectId,
    pub kind: ObjectKind,
    pub content: Vec<u8>,
}

impl VoeObject {
    pub fn new(kind: ObjectKind, content: Vec<u8>) -> Self {
        let id = ObjectId::from_bytes(&content);
        Self { id, kind, content }
    }

    pub fn with_id(id: ObjectId, kind: ObjectKind, content: Vec<u8>) -> Self {
        Self { id, kind, content }
    }

    pub fn size(&self) -> usize {
        self.content.len()
    }
}
