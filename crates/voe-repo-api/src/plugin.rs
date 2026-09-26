use crate::command::Command;
use voe_types::error::Result;

#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub description: &'static str,
    pub author: &'static str,
}

pub trait Plugin: Send + Sync {
    fn info(&self) -> PluginInfo;
    fn on_load(&self) -> Result<()> {
        Ok(())
    }
    fn on_unload(&self) -> Result<()> {
        Ok(())
    }
    fn commands(&self) -> Vec<Box<dyn Command>> {
        Vec::new()
    }
}
