use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub id: String,
    pub name: String,
    pub url: String,
    pub role: ServerRole,
    pub served_prefixes: Vec<String>,
    pub read_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerRole {
    Authoritative,
    Subsystem,
    ReadOnlyReplica,
}
