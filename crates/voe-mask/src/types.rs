use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use voe_types::object::ObjectId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaskLocation {
    File {
        path: PathBuf,
    },
    Chunk {
        path: PathBuf,
        offset: u64,
    },
    Range {
        path: PathBuf,
        offset: u64,
        length: u64,
    },
    Directory {
        path: PathBuf,
    },
}

impl MaskLocation {
    pub fn file(path: impl Into<PathBuf>) -> Self {
        MaskLocation::File { path: path.into() }
    }

    pub fn chunk(path: impl Into<PathBuf>, offset: u64) -> Self {
        MaskLocation::Chunk {
            path: path.into(),
            offset,
        }
    }

    pub fn range(path: impl Into<PathBuf>, offset: u64, length: u64) -> Self {
        MaskLocation::Range {
            path: path.into(),
            offset,
            length,
        }
    }

    pub fn directory(path: impl Into<PathBuf>) -> Self {
        MaskLocation::Directory { path: path.into() }
    }

    /// Returns the target path for file/chunk/range/directory masks.
    pub fn path(&self) -> &PathBuf {
        match self {
            MaskLocation::File { path } => path,
            MaskLocation::Chunk { path, .. } => path,
            MaskLocation::Range { path, .. } => path,
            MaskLocation::Directory { path } => path,
        }
    }
}

impl fmt::Display for MaskLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MaskLocation::File { path } => write!(f, "file:{}", path.display()),
            MaskLocation::Chunk { path, offset } => {
                write!(f, "chunk:{}+{}", path.display(), offset)
            }
            MaskLocation::Range {
                path,
                offset,
                length,
            } => {
                write!(
                    f,
                    "range:{}+{}..{}",
                    path.display(),
                    offset,
                    offset + length
                )
            }
            MaskLocation::Directory { path } => write!(f, "dir:{}", path.display()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaskContent {
    Full(Vec<u8>),
    Chunks(Vec<ChunkRef>),
    Deleted,
}

impl MaskContent {
    pub fn full(data: impl Into<Vec<u8>>) -> Self {
        MaskContent::Full(data.into())
    }

    pub fn chunks(refs: Vec<ChunkRef>) -> Self {
        MaskContent::Chunks(refs)
    }

    pub fn deleted() -> Self {
        MaskContent::Deleted
    }

    pub fn is_deleted(&self) -> bool {
        matches!(self, MaskContent::Deleted)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChunkRef {
    pub id: ObjectId,
    pub offset: u64,
    pub length: u64,
}

impl ChunkRef {
    pub fn new(id: ObjectId, offset: u64, length: u64) -> Self {
        Self { id, offset, length }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MaskMetadata {
    pub label: Option<String>,
    pub description: Option<String>,

    pub context_requirements: Vec<ContextRequirement>,

    pub dependencies: DependencyList,

    pub exclusions: Vec<ObjectId>,

    pub auto_migrate: bool,

    pub semantic_tags: Vec<String>,

    pub source_commit: Option<ObjectId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextRequirement {
    PathExists(PathBuf),
    PathContains(PathBuf, Vec<u8>),
    PriorMask(ObjectId),
    AbsentMask(ObjectId),
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DependencyList {
    pub ids: Vec<ObjectId>,
    pub semantic: Vec<String>,
}

impl DependencyList {
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty() && self.semantic.is_empty()
    }

    pub fn from_ids(ids: Vec<ObjectId>) -> Self {
        Self {
            ids,
            semantic: Vec::new(),
        }
    }

    pub fn from_semantic(tags: Vec<String>) -> Self {
        Self {
            ids: Vec::new(),
            semantic: tags,
        }
    }
}

/// A single change (location + content) within a mask.  A mask groups
/// one or more `MaskChange` entries under a shared identity and metadata
/// so that a single mask can describe edits to multiple distinct paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaskChange {
    pub location: MaskLocation,
    pub content: MaskContent,
}

impl MaskChange {
    pub fn new(location: MaskLocation, content: MaskContent) -> Self {
        Self { location, content }
    }
}

pub trait Mask: Send + Sync {
    fn id(&self) -> &ObjectId;

    /// All changes carried by this mask.  A simple single-file mask returns
    /// a slice of exactly one element; a multi-change mask returns many.
    fn changes(&self) -> &[MaskChange];

    fn metadata(&self) -> &MaskMetadata;
    fn mask_type(&self) -> MaskKind;

    /// Optional tag (e.g. a version boundary like "v1.0") attached to this
    /// mask.  Used to mark release-branch sub-versions and other named
    /// groupings.
    fn tag(&self) -> Option<&str>;

    fn apply(&self, state: &mut HashMap<PathBuf, Vec<u8>>) -> voe_types::error::Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaskKind {
    Chunk,
    File,
    Directory,
}
