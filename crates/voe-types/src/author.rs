use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub type Timestamp = u64;

pub fn current_timestamp() -> Timestamp {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    pub name: String,
    pub email: String,
    pub timestamp: Timestamp,
}

impl Author {
    pub fn new(name: impl Into<String>, email: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            email: email.into(),
            timestamp: current_timestamp(),
        }
    }

    pub fn with_timestamp(mut self, ts: Timestamp) -> Self {
        self.timestamp = ts;
        self
    }
}

impl fmt::Display for Author {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} <{}>", self.name, self.email)
    }
}
