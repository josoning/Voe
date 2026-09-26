use std::path::{Path, PathBuf};
use std::sync::Arc;

use voe_mask::{ChunkMask, ChunkRef};
use voe_repo_api::model::commit::{Commit, CommitStore, MaskObject};
use voe_storage_api::{ChunkStore, ObjectStore, StorageBackend};
use voe_types::error::{Result, VoeError};
use voe_types::object::{ObjectId, ObjectKind, VoeObject};

use crate::backend::LocalFileBackend;

pub struct FileSystemObjectStore {
    base_path: PathBuf,
    backend: Arc<dyn StorageBackend>,
}

impl FileSystemObjectStore {
    pub fn new<P: Into<PathBuf>>(path: P) -> Self {
        Self {
            base_path: path.into(),
            backend: Arc::new(LocalFileBackend::new()),
        }
    }

    pub fn with_backend<P: Into<PathBuf>>(path: P, backend: Arc<dyn StorageBackend>) -> Self {
        Self {
            base_path: path.into(),
            backend,
        }
    }

    pub fn base_path(&self) -> &Path {
        &self.base_path
    }

    pub fn backend(&self) -> &dyn StorageBackend {
        &*self.backend
    }

    pub fn ensure_dir(&self) -> Result<()> {
        if !self.backend.exists(&self.base_path)? {
            self.backend.create_dir_all(&self.base_path)?;
        }
        Ok(())
    }

    fn object_path(&self, id: &ObjectId) -> PathBuf {
        let hex = id.as_str();
        let prefix = &hex[..2.min(hex.len())];
        self.base_path.join(prefix).join(hex)
    }
}

impl ObjectStore for FileSystemObjectStore {
    fn store(&self, object: &VoeObject) -> Result<ObjectId> {
        self.ensure_dir()?;
        let path = self.object_path(&object.id);
        self.backend.ensure_parent_dir(&path)?;
        let mut buf = format!("{}\n", object.kind).into_bytes();
        buf.extend_from_slice(&object.content);
        self.backend.write_file(&path, &buf)?;
        Ok(object.id.clone())
    }

    fn retrieve(&self, id: &ObjectId) -> Result<VoeObject> {
        let path = self.object_path(id);
        if !self.backend.exists(&path)? {
            return Err(VoeError::ObjectNotFound { id: id.to_string() });
        }
        let raw = self.backend.read_file(&path)?;
        let (kind_str, content) = match raw.iter().position(|&b| b == b'\n') {
            Some(pos) => {
                let kind_str = String::from_utf8_lossy(&raw[..pos]).to_string();
                let content = raw[pos + 1..].to_vec();
                (kind_str, content)
            }
            None => (String::new(), raw.clone()),
        };
        let kind = parse_kind(&kind_str);
        Ok(VoeObject::with_id(id.clone(), kind, content))
    }

    fn exists(&self, id: &ObjectId) -> Result<bool> {
        self.backend.exists(&self.object_path(id))
    }

    fn delete(&self, id: &ObjectId) -> Result<()> {
        let path = self.object_path(id);
        self.backend.delete_file(&path)
    }

    fn list(&self) -> Result<Vec<ObjectId>> {
        let mut ids = Vec::new();
        if !self.backend.exists(&self.base_path)? {
            return Ok(ids);
        }
        for entry in walkdir::WalkDir::new(&self.base_path) {
            let entry = entry.map_err(|e| VoeError::Storage(e.to_string()))?;
            if entry.file_type().is_file() {
                if let Some(name) = entry.file_name().to_str() {
                    ids.push(ObjectId::from(name.to_string()));
                }
            }
        }
        Ok(ids)
    }
}

impl ChunkStore for FileSystemObjectStore {
    fn store_chunk(&self, _path: &Path, offset: u64, data: &[u8]) -> Result<ChunkRef> {
        let object = VoeObject::new(ObjectKind::Blob, data.to_vec());
        let id = self.store(&object)?;
        Ok(ChunkRef::new(id, offset, data.len() as u64))
    }

    fn retrieve_chunk(&self, chunk: &ChunkRef) -> Result<Vec<u8>> {
        let object = self.retrieve(&chunk.id)?;
        if object.content.len() as u64 != chunk.length {
            return Err(VoeError::Storage(format!(
                "Chunk length mismatch: stored {} vs expected {}",
                object.content.len(),
                chunk.length
            )));
        }
        Ok(object.content)
    }

    fn assemble_file(&self, chunks: &[ChunkRef]) -> Result<Vec<u8>> {
        let mut sorted: Vec<&ChunkRef> = chunks.iter().collect();
        sorted.sort_by_key(|c| c.offset);

        let total: usize = sorted.iter().map(|c| c.length as usize).sum();
        let mut result: Vec<u8> = Vec::with_capacity(total);

        for chunk in sorted {
            let data = self.retrieve_chunk(chunk)?;
            result.extend_from_slice(&data);
        }

        Ok(result)
    }
}

impl CommitStore for FileSystemObjectStore {
    fn store_commit(&self, commit: &Commit) -> Result<ObjectId> {
        let object = commit.to_voe_object()?;
        self.store(&object)
    }

    fn retrieve_commit(&self, id: &ObjectId) -> Result<Commit> {
        let object = self.retrieve(id)?;
        Commit::from_voe_object(&object)
    }

    fn store_mask(&self, mask: &ChunkMask) -> Result<ObjectId> {
        let object = mask.to_voe_object()?;
        self.store(&object)
    }

    fn retrieve_mask(&self, id: &ObjectId) -> Result<MaskObject> {
        let object = self.retrieve(id)?;
        // First try the modern ChunkMask format.
        match ChunkMask::from_voe_object(&object) {
            Ok(mask) => Ok(MaskObject::Chunk(mask)),
            // Backward compatibility: the object might be stored in the
            // legacy `Separator` format (label + id, no changes).  Convert
            // it into a tagged `ChunkMask`.
            Err(_) => {
                let value: serde_json::Value =
                    serde_json::from_slice(&object.content).map_err(|e| {
                        VoeError::Storage(format!("Failed to parse legacy Separator JSON: {}", e))
                    })?;
                let obj = value.as_object().ok_or_else(|| {
                    VoeError::Storage("Legacy Separator JSON must be an object".to_string())
                })?;
                let id_str: String = serde_json::from_value(
                    obj.get("id")
                        .ok_or_else(|| VoeError::Storage("legacy sep missing `id`".to_string()))?
                        .clone(),
                )
                .map_err(|e| VoeError::Storage(format!("bad id: {}", e)))?;
                let label: Option<String> = obj
                    .get("label")
                    .cloned()
                    .map(serde_json::from_value)
                    .unwrap_or_else(|| Ok(None))
                    .map_err(|e| VoeError::Storage(format!("bad label: {}", e)))?;
                let tag = label.clone().unwrap_or_else(|| id_str.clone());
                let mask = ChunkMask::tagged(ObjectId::new(&id_str), tag);
                Ok(MaskObject::Chunk(mask))
            }
        }
    }
}

fn parse_kind(s: &str) -> voe_types::object::ObjectKind {
    match s {
        "tree" => voe_types::object::ObjectKind::Tree,
        "commit" => voe_types::object::ObjectKind::Commit,
        "tag" => voe_types::object::ObjectKind::Tag,
        "merge" => voe_types::object::ObjectKind::Merge,
        _ => voe_types::object::ObjectKind::Blob,
    }
}
