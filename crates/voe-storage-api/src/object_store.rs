use std::path::{Path, PathBuf};

use voe_mask::types::ChunkRef;
use voe_types::error::Result;
use voe_types::object::{ObjectId, VoeObject};
use voe_types::server_info::ServerInfo;

pub trait ObjectStore: Send + Sync {
    fn store(&self, object: &VoeObject) -> Result<ObjectId>;
    fn retrieve(&self, id: &ObjectId) -> Result<VoeObject>;
    fn exists(&self, id: &ObjectId) -> Result<bool>;
    fn delete(&self, id: &ObjectId) -> Result<()>;
    fn list(&self) -> Result<Vec<ObjectId>>;
}

pub trait ChunkStore: Send + Sync {
    fn store_chunk(&self, path: &Path, offset: u64, data: &[u8]) -> Result<ChunkRef>;
    fn retrieve_chunk(&self, chunk: &ChunkRef) -> Result<Vec<u8>>;
    fn assemble_file(&self, chunks: &[ChunkRef]) -> Result<Vec<u8>>;
}

pub trait RoutedObjectStore: ObjectStore + ChunkStore {
    fn store_for_path(&self, path: &Path, object: &VoeObject) -> Result<ObjectId>;
    fn retrieve_from(&self, server: &ServerInfo, id: &ObjectId) -> Result<VoeObject>;
    fn store_chunk_for_path(&self, path: &Path, offset: u64, data: &[u8]) -> Result<ChunkRef>;
}

pub trait ObjectStoreFactory: Send + Sync {
    fn create_local(&self, base_path: PathBuf) -> Result<Box<dyn ObjectStore>>;
    fn create_for_server(&self, server: &ServerInfo) -> Result<Box<dyn ObjectStore>>;
}
