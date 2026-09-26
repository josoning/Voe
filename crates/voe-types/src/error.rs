use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum VoeError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Object store error: {0}")]
    Storage(String),

    #[error("File not found: {path}")]
    FileNotFound { path: PathBuf },

    #[error("Directory not found: {path}")]
    DirNotFound { path: PathBuf },

    #[error("Permission denied: {path}")]
    PermissionDenied { path: PathBuf },

    #[error("Storage full or quota exceeded: {path}")]
    StorageFull { path: PathBuf },

    #[error("File already exists: {path}")]
    FileExists { path: PathBuf },

    #[error("Invalid path: {path} - {reason}")]
    InvalidPath { path: PathBuf, reason: String },

    #[error("Repository not found at {path}")]
    RepoNotFound { path: PathBuf },

    #[error("Repository already initialized at {path}")]
    RepoAlreadyExists { path: PathBuf },

    #[error("Object not found: {id}")]
    ObjectNotFound { id: String },

    #[error("Invalid object id: {id}")]
    InvalidObjectId { id: String },

    #[error("Command error: {0}")]
    Command(String),

    #[error("Plugin error: {0}")]
    Plugin(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Serialization error: {0}")]
    Serde(String),

    #[error("Authentication/authorization error: {0}")]
    Auth(String),

    #[error("Server registry error: {0}")]
    Server(String),

    #[error("Branch error: {0}")]
    Branch(String),

    #[error("Invalid branch name: {name} - {reason}")]
    InvalidBranchName { name: String, reason: String },

    #[error("Invalid branch identifier: {0}")]
    InvalidBranchId(String),

    #[error("Reserved keyword '{keyword}' may not be used in {context}")]
    ReservedKeyword { keyword: String, context: String },

    #[error("Branch already exists: {0}")]
    BranchAlreadyExists(String),

    #[error("Branch not found: {0}")]
    BranchNotFound(String),

    #[error("'mainline' label must be globally unique, but found multiple")]
    MainlineNotUnique,

    #[error("No branch carries the 'mainline' label")]
    MainlineNotFound,

    #[error(
        "Operation not allowed in detached HEAD state; commit or create a temp sub-branch first"
    )]
    DetachedHead,

    #[error("Release branches are append-only; cannot rewrite or delete existing masks")]
    ReleaseAppendViolation,

    #[error("Alias '{alias}' is already taken")]
    AliasAlreadyExists { alias: String },

    #[error("Alias '{alias}' not found")]
    AliasNotFound { alias: String },

    #[error("Merge conflict: {0}")]
    MergeConflict(String),

    #[error("Parent branch not found: {0}")]
    ParentBranchNotFound(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, VoeError>;
