use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use voe_types::error::Result;

pub use voe_types::{ServerInfo, ServerRole};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ServerRegistry {
    pub authoritative: String,
    pub servers: HashMap<String, ServerInfo>,
}

impl ServerRegistry {
    pub fn new(authoritative_id: impl Into<String>, authoritative_url: impl Into<String>) -> Self {
        let id = authoritative_id.into();
        let url = authoritative_url.into();
        let info = ServerInfo {
            id: id.clone(),
            name: "authoritative".to_string(),
            url,
            role: ServerRole::Authoritative,
            served_prefixes: vec!["/".to_string()],
            read_only: false,
        };
        let mut servers = HashMap::new();
        servers.insert(id.clone(), info);
        Self {
            authoritative: id,
            servers,
        }
    }

    pub fn add_subsystem(&mut self, info: ServerInfo) {
        self.servers.insert(info.id.clone(), info);
    }

    pub fn remove(&mut self, id: &str) {
        if id != self.authoritative {
            self.servers.remove(id);
        }
    }

    pub fn authoritative_server(&self) -> &ServerInfo {
        &self.servers[&self.authoritative]
    }

    pub fn find_server_for_path(&self, path: &str) -> Option<&ServerInfo> {
        let mut best: Option<(&ServerInfo, usize)> = None;
        for info in self.servers.values() {
            for prefix in &info.served_prefixes {
                if path.starts_with(prefix.as_str()) {
                    let specificity = prefix.len();
                    match best {
                        None => best = Some((info, specificity)),
                        Some((_, best_spec)) if specificity > best_spec => {
                            best = Some((info, specificity))
                        }
                        _ => {}
                    }
                }
            }
        }
        best.map(|(info, _)| info)
    }

    pub fn writable_servers(&self) -> Vec<&ServerInfo> {
        self.servers.values().filter(|s| !s.read_only).collect()
    }

    pub fn subsystem_servers(&self) -> Vec<&ServerInfo> {
        self.servers
            .values()
            .filter(|s| matches!(s.role, ServerRole::Subsystem))
            .collect()
    }

    pub fn list_all(&self) -> Vec<&ServerInfo> {
        self.servers.values().collect()
    }
}

pub trait ServerRegistryStore: Send + Sync {
    fn load(&self) -> Result<ServerRegistry>;
    fn save(&self, registry: &ServerRegistry) -> Result<()>;
}
