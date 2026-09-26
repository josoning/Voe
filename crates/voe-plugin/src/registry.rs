use std::collections::HashMap;

use voe_repo_api::plugin::{Plugin, PluginInfo};
use voe_types::error::{Result, VoeError};

pub struct PluginRegistry {
    plugins: HashMap<String, Box<dyn Plugin>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    pub fn register<P: Plugin + 'static>(&mut self, plugin: P) -> Result<()> {
        let info = plugin.info();
        let name = info.name.to_string();
        if self.plugins.contains_key(&name) {
            return Err(VoeError::Plugin(format!(
                "Plugin '{}' is already registered",
                name
            )));
        }
        plugin.on_load()?;
        self.plugins.insert(name, Box::new(plugin));
        Ok(())
    }

    pub fn unregister(&mut self, name: &str) -> Result<()> {
        if let Some(plugin) = self.plugins.remove(name) {
            plugin.on_unload()?;
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&(dyn Plugin + 'static)> {
        self.plugins.get(name).map(|p| p.as_ref())
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut (dyn Plugin + 'static)> {
        self.plugins.get_mut(name).map(|p| p.as_mut())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.plugins.contains_key(name)
    }

    pub fn names(&self) -> Vec<&str> {
        self.plugins.keys().map(|k| k.as_str()).collect()
    }

    pub fn infos(&self) -> Vec<PluginInfo> {
        self.plugins.values().map(|p| p.info()).collect()
    }

    pub fn iter(&self) -> PluginIter<'_> {
        PluginIter {
            inner: self.plugins.iter(),
        }
    }

    pub fn plugin_count(&self) -> usize {
        self.plugins.len()
    }

    pub fn unload_all(&mut self) -> Result<()> {
        let names: Vec<String> = self.plugins.keys().cloned().collect();
        for name in names {
            if let Some(plugin) = self.plugins.remove(&name) {
                plugin.on_unload()?;
            }
        }
        Ok(())
    }
}

pub struct PluginIter<'a> {
    inner: std::collections::hash_map::Iter<'a, String, Box<dyn Plugin>>,
}

impl<'a> Iterator for PluginIter<'a> {
    type Item = (&'a String, &'a dyn Plugin);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|(k, v)| (k, v.as_ref() as &dyn Plugin))
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}
