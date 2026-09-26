use voe_types::error::Result;
use voe_types::object::ObjectId;

pub trait RefStore: Send + Sync {
    fn get_head(&self) -> Result<Option<ObjectId>>;
    fn set_head(&self, id: &ObjectId) -> Result<()>;
    fn get_ref(&self, name: &str) -> Result<Option<ObjectId>>;
    fn set_ref(&self, name: &str, id: &ObjectId) -> Result<()>;
    fn delete_ref(&self, name: &str) -> Result<()>;
    fn list_refs(&self) -> Result<Vec<(String, ObjectId)>>;
}
