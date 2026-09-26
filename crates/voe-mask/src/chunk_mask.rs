use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

use voe_types::error::{Result, VoeError};
use voe_types::object::{ObjectId, ObjectKind, VoeObject};

use super::types::{
    ChunkRef, DependencyList, Mask, MaskChange, MaskContent, MaskKind, MaskLocation, MaskMetadata,
};

/// A mask that stores changes against file paths.  A single `ChunkMask`
/// now groups one or more [`MaskChange`] entries under a shared identity
/// and metadata so that one mask object can describe edits to multiple
/// distinct paths in an atomic group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkMask {
    pub id: ObjectId,
    pub changes: Vec<MaskChange>,
    pub metadata: MaskMetadata,
    pub tag: Option<String>,
}

impl Serialize for ChunkMask {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_struct("ChunkMask", 4)?;
        s.serialize_field("id", &self.id)?;
        s.serialize_field("changes", &self.changes)?;
        s.serialize_field("metadata", &self.metadata)?;
        s.serialize_field("tag", &self.tag)?;
        s.end()
    }
}

impl ChunkMask {
    /// Construct a multi-change mask from an id, a list of changes, and
    /// metadata.  Returns an error when `changes` is empty.
    pub fn new(id: ObjectId, changes: Vec<MaskChange>, metadata: MaskMetadata) -> Self {
        Self {
            id,
            changes,
            metadata,
            tag: None,
        }
    }

    /// Build a single-change mask that replaces the full file content of
    /// `path` with `content`.
    pub fn file(id: ObjectId, path: impl Into<PathBuf>, content: MaskContent) -> Self {
        Self {
            id,
            changes: vec![MaskChange::new(MaskLocation::file(path), content)],
            metadata: MaskMetadata::default(),
            tag: None,
        }
    }

    /// Build a single-change mask that describes a chunk-level patch.
    pub fn chunk(
        id: ObjectId,
        path: impl Into<PathBuf>,
        offset: u64,
        ref_id: ObjectId,
        length: u64,
    ) -> Self {
        Self {
            id,
            changes: vec![MaskChange::new(
                MaskLocation::chunk(path, offset),
                MaskContent::Chunks(vec![ChunkRef::new(ref_id, offset, length)]),
            )],
            metadata: MaskMetadata::default(),
            tag: None,
        }
    }

    /// Build a single-change mask that deletes a file.
    pub fn deleted(id: ObjectId, path: impl Into<PathBuf>) -> Self {
        Self {
            id,
            changes: vec![MaskChange::new(
                MaskLocation::file(path),
                MaskContent::Deleted,
            )],
            metadata: MaskMetadata::default(),
            tag: None,
        }
    }

    /// Build a tag-only mask: a mask with no file changes that only carries
    /// a tag.  Replaces the old `Separator` concept entirely — callers that
    /// previously created a separator now create a tagged mask via this
    /// constructor.
    pub fn tagged(id: ObjectId, tag: impl Into<String>) -> Self {
        Self {
            id,
            changes: Vec::new(),
            metadata: MaskMetadata::default(),
            tag: Some(tag.into()),
        }
    }

    /// Start building a multi-change mask.  Push additional changes with
    /// [`Self::with_change`] before finishing.
    pub fn builder(id: ObjectId) -> ChunkMaskBuilder {
        ChunkMaskBuilder {
            id,
            changes: Vec::new(),
            metadata: MaskMetadata::default(),
            tag: None,
        }
    }

    pub fn with_metadata(mut self, metadata: MaskMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.metadata.label = Some(label.into());
        self
    }

    pub fn with_dependencies(mut self, deps: DependencyList) -> Self {
        self.metadata.dependencies = deps;
        self
    }

    pub fn with_exclusions(mut self, exclusions: Vec<ObjectId>) -> Self {
        self.metadata.exclusions = exclusions;
        self
    }

    pub fn with_auto_migrate(mut self, enable: bool) -> Self {
        self.metadata.auto_migrate = enable;
        self
    }

    pub fn with_source_commit(mut self, commit_id: ObjectId) -> Self {
        self.metadata.source_commit = Some(commit_id);
        self
    }

    /// Convenience: the single-change `ChunkMask::file` / `deleted` / `chunk`
    /// constructors each produce a mask with exactly one change.  This helper
    /// returns the first change, or panics if the mask has zero changes
    /// (should never happen for well-formed masks).
    pub fn first(&self) -> &MaskChange {
        &self.changes[0]
    }

    pub fn to_voe_object(&self) -> Result<VoeObject> {
        let content = serde_json::to_vec(self)
            .map_err(|e| VoeError::Storage(format!("Failed to serialize ChunkMask: {}", e)))?;
        Ok(VoeObject::new(ObjectKind::Blob, content))
    }

    pub fn from_voe_object(object: &VoeObject) -> Result<Self> {
        // Try new format first — if it fails, the serializer is broken.
        // We do *not* rely on serde's untagged / adjacently-tagged tricks
        // because we want a clean error on genuinely malformed input.
        let value: serde_json::Value = serde_json::from_slice(&object.content)
            .map_err(|e| VoeError::Storage(format!("Failed to parse ChunkMask JSON: {}", e)))?;

        Self::from_json_value(&value)
            .map_err(|e| VoeError::Storage(format!("Failed to deserialize ChunkMask: {}", e)))
    }

    fn from_json_value(value: &serde_json::Value) -> std::result::Result<Self, String> {
        let obj = value
            .as_object()
            .ok_or_else(|| "ChunkMask JSON must be an object".to_string())?;

        let id: ObjectId = serde_json::from_value(
            obj.get("id")
                .ok_or_else(|| "missing `id`".to_string())?
                .clone(),
        )
        .map_err(|e| format!("bad `id`: {}", e))?;

        let metadata: MaskMetadata = obj
            .get("metadata")
            .cloned()
            .map(serde_json::from_value)
            .unwrap_or_else(|| Ok(MaskMetadata::default()))
            .map_err(|e| format!("bad `metadata`: {}", e))?;

        // New format: `changes` array
        let changes: Vec<MaskChange> = if let Some(c) = obj.get("changes") {
            serde_json::from_value(c.clone()).map_err(|e| format!("bad `changes`: {}", e))?
        } else {
            // Legacy format: flat `location` + `content`
            let location: MaskLocation = serde_json::from_value(
                obj.get("location")
                    .ok_or_else(|| "missing `changes` or legacy `location`".to_string())?
                    .clone(),
            )
            .map_err(|e| format!("bad `location`: {}", e))?;
            let content: MaskContent = serde_json::from_value(
                obj.get("content")
                    .ok_or_else(|| "missing legacy `content`".to_string())?
                    .clone(),
            )
            .map_err(|e| format!("bad `content`: {}", e))?;
            vec![MaskChange::new(location, content)]
        };

        let tag = obj
            .get("tag")
            .cloned()
            .map(serde_json::from_value::<Option<String>>)
            .unwrap_or_else(|| Ok(None))
            .map_err(|e| format!("bad `tag`: {}", e))?;

        if changes.is_empty() && tag.is_none() {
            return Err("ChunkMask must contain at least one change or a tag".to_string());
        }

        Ok(Self {
            id,
            changes,
            metadata,
            tag,
        })
    }
}

/// Builder that accumulates multiple changes before producing a
/// `ChunkMask`.  Constructed via [`ChunkMask::builder`].
pub struct ChunkMaskBuilder {
    id: ObjectId,
    changes: Vec<MaskChange>,
    metadata: MaskMetadata,
    tag: Option<String>,
}

impl ChunkMaskBuilder {
    pub fn change(mut self, location: MaskLocation, content: MaskContent) -> Self {
        self.changes.push(MaskChange::new(location, content));
        self
    }

    pub fn file(self, path: impl Into<PathBuf>, content: MaskContent) -> Self {
        self.change(MaskLocation::file(path), content)
    }

    pub fn deleted(self, path: impl Into<PathBuf>) -> Self {
        self.change(MaskLocation::file(path), MaskContent::Deleted)
    }

    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    pub fn with_metadata(mut self, metadata: MaskMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.metadata.label = Some(label.into());
        self
    }

    pub fn with_dependencies(mut self, deps: DependencyList) -> Self {
        self.metadata.dependencies = deps;
        self
    }

    pub fn build(self) -> ChunkMask {
        assert!(
            !self.changes.is_empty() || self.tag.is_some(),
            "ChunkMaskBuilder requires at least one change or a tag"
        );
        ChunkMask {
            id: self.id,
            changes: self.changes,
            metadata: self.metadata,
            tag: self.tag,
        }
    }
}

impl Mask for ChunkMask {
    fn id(&self) -> &ObjectId {
        &self.id
    }

    fn changes(&self) -> &[MaskChange] {
        &self.changes
    }

    fn metadata(&self) -> &MaskMetadata {
        &self.metadata
    }

    fn mask_type(&self) -> MaskKind {
        for c in &self.changes {
            match &c.location {
                MaskLocation::Chunk { .. } => return MaskKind::Chunk,
                MaskLocation::Range { .. } => return MaskKind::Chunk,
                MaskLocation::Directory { .. } => return MaskKind::Directory,
                _ => {}
            }
        }
        MaskKind::File
    }

    fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
    }

    fn apply(&self, state: &mut HashMap<PathBuf, Vec<u8>>) -> Result<()> {
        for change in &self.changes {
            let path = change.location.path().clone();
            match &change.content {
                MaskContent::Full(data) => {
                    state.insert(path, data.clone());
                }
                MaskContent::Deleted => {
                    state.remove(&path);
                }
                MaskContent::Chunks(_refs) => {
                    state.entry(path).or_default();
                }
            }
        }
        Ok(())
    }
}

impl fmt::Display for ChunkMask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.changes.len() == 1 {
            let change = &self.changes[0];
            let kind = match &change.content {
                MaskContent::Full(_) => "modify",
                MaskContent::Chunks(_) => "chunk",
                MaskContent::Deleted => "delete",
            };
            write!(f, "[{}:{}] {}", kind, self.id, change.location)
        } else {
            write!(f, "[multi:{}] {} changes", self.id, self.changes.len())
        }
    }
}
