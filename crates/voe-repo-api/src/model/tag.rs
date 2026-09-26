use serde::{Deserialize, Serialize};

use voe_types::error::{Result, VoeError};

use voe_types::author::Author;
use voe_types::object::{ObjectId, ObjectKind, VoeObject};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub target: ObjectId,
    pub target_kind: ObjectKind,
    pub name: String,
    pub tagger: Author,
    pub message: String,
}

impl Tag {
    pub fn new(target: ObjectId, name: String, tagger: Author, message: String) -> Self {
        Self {
            target,
            target_kind: ObjectKind::Commit,
            name,
            tagger,
            message,
        }
    }

    pub fn to_voe_object(&self) -> Result<VoeObject> {
        let content = serde_json::to_vec(self)
            .map_err(|e| VoeError::Storage(format!("Failed to serialize Tag: {}", e)))?;
        Ok(VoeObject::new(ObjectKind::Tag, content))
    }
}
