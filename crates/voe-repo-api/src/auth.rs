use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use voe_types::author::current_timestamp;
use voe_types::error::Result;

pub trait Signer: Send + Sync {
    fn sign(&self, data: &[u8]) -> Result<Vec<u8>>;
    fn public_key(&self) -> Vec<u8>;
    fn key_fingerprint(&self) -> String;
    fn algorithm(&self) -> &str;
}

pub trait Verifier: Send + Sync {
    fn verify(&self, data: &[u8], signature: &[u8], public_key: &[u8]) -> Result<bool>;
}

pub trait AuthStore: Send + Sync {
    fn issue_key_pair(&self, repo_id: &str, validity_days: u64) -> Result<AuthGrant>;
    fn validate_grant(&self, grant: &AuthGrant) -> Result<bool>;
    fn revoke_grant(&self, key_fingerprint: &str) -> Result<()>;
    fn list_active_grants(&self, repo_id: &str) -> Result<Vec<AuthGrant>>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthGrant {
    pub repo_id: String,
    pub public_key: Vec<u8>,
    pub key_fingerprint: String,
    pub issued_at: voe_types::author::Timestamp,
    pub expires_at: voe_types::author::Timestamp,
    pub permissions: Permissions,
}

impl AuthGrant {
    pub fn is_valid(&self) -> bool {
        let now = current_timestamp();
        now >= self.issued_at && now <= self.expires_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Permissions {
    pub write_paths: Vec<String>,
    pub max_offline_commits: Option<u64>,
    pub metadata: HashMap<String, String>,
}

impl Permissions {
    pub fn read_only() -> Self {
        Self {
            write_paths: Vec::new(),
            max_offline_commits: Some(0),
            metadata: HashMap::new(),
        }
    }

    pub fn full_access() -> Self {
        Self {
            write_paths: vec!["*".to_string()],
            max_offline_commits: None,
            metadata: HashMap::new(),
        }
    }

    pub fn can_write(&self, path: &str) -> bool {
        if self.write_paths.iter().any(|p| p == "*") {
            return true;
        }
        self.write_paths.iter().any(|p| path.starts_with(p))
    }
}
