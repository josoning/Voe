# Voe

A semi-centralized command-line version control system written in Rust.

## What Makes Voe Different

Most version control systems fall into one of two camps: centralized (SVN) or fully distributed (Git). Voe takes a middle position. It maintains a single authoritative server that coordinates subsystem servers, but every client holds a complete local repository copy so it can browse history and commit offline when a valid key-pair grant is held. This hybrid model is designed for organisations that want the governance of a central authority without sacrificing offline productivity.

| Dimension | Centralized (SVN) | Distributed (Git) | **Voe (semi-centralized)** |
|-----------|-------------------|-------------------|---------------------------|
| Authority | Single server | No authority, every clone is full | Single authoritative center |
| Offline browsing | Requires server | Full local history | Local repository copy |
| Offline commit | Impossible | Fully free | Conditional — valid key-pair grant required |
| Scalability | Single point of bottleneck | P2P | Authority center plus subsystem servers |
| Commit payload | Path-level change list | Full tree snapshot | Mask abstraction, storage-optimized |

## Project Status

As of this version, the local VCS workflow is working end-to-end: you can `init` a repository, `add` paths to stage changes, `commit` with a mask-based payload, `log` to browse history, `checkout` to restore a working tree, and `status` to see staged, unstaged, and untracked files. The local filesystem backend has unit and integration tests covering object storage, branch management, references, configuration, and mask resolution.

What remains unfinished is the semi-centralized networking layer: cryptographic signer/verifier backends, the `AuthStore` that issues offline grants, subsystem-server synchronization, and the routed remote object store. The trait contracts for these are all in place and ready to be implemented.

## Core Technical Ideas

### Semi-centralized architecture with subsystem servers

An authoritative server maintains the aggregated reference graph and a routing table that maps path prefixes to subsystem servers. Each subsystem server manages its own path prefix independently, and read-only replicas can be used as CDN-style caches. Authoritative duties include accepting subsystem syncs (eventually consistent) and issuing key-pair authorization grants.

### Offline commit via key-pair authorization

A server-side `AuthStore` issues a time-limited `AuthGrant` containing a public key, expiration timestamp, and a `Permissions` object describing which path prefixes the bearer can write. The client keeps the private key. When committing offline, the client attaches a `CommitSignature` (algorithm, key fingerprint, signature bytes, expiration) to the commit object. When the batch is later pushed to the authoritative server, a `Verifier` validates the signature and the `AuthStore` checks that the grant has not expired and that the commit paths fall within the permission scope.

### Mask — the central abstraction

Every commit carries a list of masks that describe what changed since the previous commit. Masks are the interface presented to upper layers; the storage backend is free to optimise internally — for instance by computing masks from a full tree snapshot, or by storing chunk references. The only contract is that, given a commit and its mask list, the complete file content can always be reconstructed.

```rust
pub enum MaskLocation {
    File { path: PathBuf },
    Chunk { path: PathBuf, offset: u64 },
    Range { path: PathBuf, offset: u64, length: u64 },
    Directory { path: PathBuf },
}

pub enum MaskContent {
    Full(Vec<u8>),
    Chunks(Vec<ChunkRef>),
    Deleted,
}

pub trait Mask: Send + Sync {
    fn id(&self) -> &ObjectId;
    fn location(&self) -> &MaskLocation;
    fn content(&self) -> &MaskContent;
    fn metadata(&self) -> &MaskMetadata;
    fn mask_type(&self) -> MaskKind;
    fn apply(&self, state: &mut HashMap<PathBuf, Vec<u8>>) -> Result<()>;
}
```

## Architecture

### Design philosophy

Traits are the boundaries. Modules are the implementations. The dependency graph is strictly layered and acyclic. `voe-core` is the only crate depended upon by every other crate; it contains only trait definitions and pure data structures, no business logic.

### Crate dependency flow

`voe-cli` depends on `voe-commands`, `voe-plugin`, and `voe-fs`. All four depend on `voe-core`. `voe-plugin` is also available as a compile-time or optionally dynamic (feature-gated `libloading`) plugin loader. The bottom layer (`voe-core`) knows nothing about the top layers.

| Crate | Responsibility | Downstream deps |
|-------|---------------|----------------|
| `voe-core` | Trait contract layer, zero implementation | `thiserror`, `serde`, `sha2`, `hex`, `uuid`, `toml`, `regex` |
| `voe-fs` | Local filesystem backend implementations | `voe-core`, `walkdir` |
| `voe-commands` | Command registry and builtin commands | `voe-core`, `clap` |
| `voe-plugin` | Plugin system (compile-time + optional dynamic) | `voe-core`, `libloading` (optional) |
| `voe-cli` | Entry point, wires all modules together, CLI parsing | All others, `anyhow`, `clap`, `tracing`, `tracing-subscriber` |

### voe-core module overview

`voe-core` is organized around four sub-modules plus a handful of top-level traits.

| Module | Key types | Purpose |
|--------|-----------|---------|
| `mask/` | `ChunkMask`, `SimpleMaskResolver` | Mask abstraction, resolution, conflict detection |
| `model/` | `ObjectId`, `VoeObject`, `Commit`, `Tag`, `Tree`, `Branch`, `Alias`, `RefStore`, `IndexStore`, `CommitStore`, `BranchStore` | All version-controlled data structures and the SHA-256 object model |
| `storage/` | `ObjectStore`, `ChunkStore`, `StorageBackend` | Persistent storage traits |
| `repository.rs` | `Repository`, `RepoManager` | Repository abstraction and factory |
| `auth.rs` | `Signer`, `Verifier`, `AuthStore`, `AuthGrant`, `Permissions` | Key-pair authorization system |
| `server.rs` | `ServerRegistry`, `ServerInfo`, `ServerRole` | Authority and subsystem routing |
| `command.rs` | `Command`, `CommandContext`, `CommandInfo`, `CommandResult` | Command abstraction and execution context |
| `plugin.rs` | `Plugin`, `PluginInfo` | Plugin abstraction |
| `config.rs` | `VoeConfig`, `ConfigManager`, `IgnoreRule`, `LockState` | Configuration read/write interface |
| `snapshot.rs` | `SnapshotEngine` | Reconstructs working tree from a commit's mask history |
| `error.rs` | `VoeError`, `Result<T>` | Unified error type via `thiserror` |

### Key trait boundaries

#### Repository and RepoManager

`RepoManager` is a factory that creates or opens a repository on disk. `Repository` exposes every backend-facing trait behind a single facade, so commands never need to know which concrete storage backend is in use.

```rust
pub trait RepoManager: Send + Sync {
    fn init(&self, path: PathBuf) -> Result<Box<dyn Repository>>;
    fn open(&self, path: PathBuf) -> Result<Box<dyn Repository>>;
    fn try_open_or_init(&self, path: PathBuf) -> Result<Box<dyn Repository>>;
    fn find_root(&self, start: &Path) -> Option<PathBuf>;
}

pub trait Repository: Send + Sync {
    fn path(&self) -> &PathBuf;
    fn object_store(&self) -> &dyn ObjectStore;
    fn chunk_store(&self) -> &dyn ChunkStore;
    fn commit_store(&self) -> &dyn CommitStore;
    fn ref_store(&self) -> &dyn RefStore;
    fn branch_store(&self) -> &dyn BranchStore;
    fn index_store(&self) -> &dyn IndexStore;
    fn config_manager(&self) -> &dyn ConfigManager;
    fn config_manager_mut(&mut self) -> &mut dyn ConfigManager;
}
```

#### ObjectStore, ChunkStore, and RoutedObjectStore

`ObjectStore` persists content-addressed blobs and trees. `ChunkStore` stores partial file chunks so the backend can optimise large files. `RoutedObjectStore` extends both with path-aware routing — given a path prefix it dispatches to the correct subsystem server.

```rust
pub trait ObjectStore: Send + Sync {
    fn store(&self, object: &VoeObject) -> Result<ObjectId>;
    fn retrieve(&self, id: &ObjectId) -> Result<VoeObject>;
    fn exists(&self, id: &ObjectId) -> Result<bool>;
    fn delete(&self, id: &ObjectId) -> Result<()>;
    fn list(&self) -> Result<Vec<ObjectId>>;
}

pub trait ChunkStore: Send + Sync {
    fn store_chunk(&self, path: &PathBuf, offset: u64, data: &[u8]) -> Result<ChunkRef>;
    fn retrieve_chunk(&self, chunk: &ChunkRef) -> Result<Vec<u8>>;
    fn assemble_file(&self, chunks: &[ChunkRef]) -> Result<Vec<u8>>;
}
```

#### Auth — Signer, Verifier, AuthStore

Three traits cover the full offline-authorization flow. A `Signer` produces signatures; a `Verifier` checks them; an `AuthStore` on the server issues, validates, and revokes grants.

```rust
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
```

#### ServerRegistry

`ServerRegistry` is a plain data structure plus routing helpers that map path prefixes to the subsystem server with the most specific match. `find_server_for_path` walks all registered servers and returns the one whose `served_prefixes` list contains the longest matching prefix.

### Commit data structure

A commit object carries the tree root, parent list, author/committer pair, message, mask list, and an optional cryptographic signature used for offline validation.

```rust
pub struct Commit {
    pub tree: ObjectId,
    pub parents: Vec<ObjectId>,
    pub author: Author,
    pub committer: Author,
    pub message: String,
    pub masks: Vec<ObjectId>,
    pub signature: Option<CommitSignature>,
}

pub struct CommitSignature {
    pub algorithm: String,
    pub key_fingerprint: String,
    pub signature: Vec<u8>,
    pub expires_at: i64,
}
```

### End-to-end command example

The dispatcher (`voe-cli`) wires a `FsRepoManager` into a `CommandContext`. Each builtin command takes the context, resolves the repository root via `find_root`, opens the repository, and uses the trait facades behind `Repository` to perform its work. Concrete storage types never leak past the crate boundary.

## Plugin system

### Compile-time plugins (recommended)

Implement the `Plugin` trait and register your plugin at assembly time in `voe-cli`. Every `Command` returned by `Plugin::commands()` is added to the global `CommandRegistry`.

```rust
pub struct HelloPlugin;

impl Plugin for HelloPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            name: "hello",
            version: "0.1.0",
            description: "Example plugin",
            author: "you@example.com",
        }
    }

    fn commands(&self) -> Vec<Box<dyn Command>> {
        vec![Box::new(HelloCommand)]
    }
}
```

### Dynamic-loading plugins (feature-gated)

Enable the `dynamic-plugins` feature on `voe-plugin` to load third-party `.so` files at runtime using `libloading`. The loader uses a C-compatible vtable to bridge the FFI boundary safely. See `voe-plugin/src/loader.rs` for details. This path is disabled by default because it involves `unsafe` FFI.

## Error handling strategy

| Crate | Error type | Tool | Style |
|-------|-----------|------|-------|
| `voe-core` | `VoeError` (enum) | `thiserror` | Strongly typed, for library-level matching |
| `voe-fs`, `voe-commands`, `voe-plugin` | `Result<T, VoeError>` | — | Pass through core errors |
| `voe-cli` | `anyhow::Result<T>` | `anyhow` | Flexible wrapping with user-friendly messages |

## Building and running

```bash
cargo build
cargo test
cargo run -- init my-repo
cd my-repo && cargo run -- status
```

## Project layout

The workspace root `Cargo.toml` defines five member crates. Source files live under `crates/`.

- `crates/voe-core/` — trait contract layer. Modules: `mask/`, `model/`, `storage/`, plus `auth.rs`, `command.rs`, `config.rs`, `error.rs`, `plugin.rs`, `repository.rs`, `server.rs`, `snapshot.rs`.
- `crates/voe-fs/` — local filesystem backend. Subdirectories: `backend/` (`LocalFileBackend`, `CachedStorageBackend`, `LoggingStorageBackend`) and `repo/` (`FsRepository`, `FsRepoManager`, `FsRefStore`, `FsIndexStore`, `FsBranchStore` internally). Also `filesystem.rs`, `working_tree.rs`, `config_backend.rs`.
- `crates/voe-commands/` — command system. `registry.rs` plus `builtin/` containing `init`, `add`, `commit`, `checkout`, `status`, `log`, `help`, `plugin`.
- `crates/voe-plugin/` — plugin system. `registry.rs` (always available) and `loader.rs` (feature-gated `dynamic-plugins`).
- `crates/voe-cli/` — binary entry point. `main.rs`, `app.rs` (clap definitions), `dispatcher.rs` (assembly and dispatch).

## Extension points

| Goal | Approach | Impact |
|------|---------|--------|
| Swap storage backend | Implement `ObjectStore`, `ChunkStore`, optionally `RoutedObjectStore` | New crate |
| Add a command | Implement `Command`, register in `CommandRegistry` | New file in `voe-commands/src/builtin/` |
| Add compile-time plugin | Implement `Plugin`, register at CLI assembly | New crate |
| Add dynamic plugin | Export `voe_plugin_create` from a dylib | External project |
| Swap config format | Implement a new `ConfigManager` | `voe-fs` or new crate |
| Add cryptographic backend | Implement `Signer` + `Verifier` (for example ed25519) | New crate (e.g. `voe-auth`) |
| Add remote object store | Implement `RoutedObjectStore` (for example HTTP or gRPC) | New crate |
| Sub-server sync protocol | Add a new `Sync` trait (not yet designed) | `voe-core` + new crate |

## Roadmap

- [x] Basic architecture framework
- [x] Architecture documentation (this file)
- [x] Mask data structures, Commit (with optional signature), ServerRegistry trait contracts
- [x] `IndexStore` and `RefStore` traits with local filesystem implementations
- [x] `add` / `commit` / `log` / `checkout` / `status` workflow
- [x] `SnapshotEngine` for reconstructing working tree from commit history
- [x] `BranchStore` with release-branch append-only enforcement
- [ ] Signer / Verifier backend implementation (ed25519 or similar)
- [ ] `AuthStore` implementation and offline authorization workflow
- [ ] `reset`, `branch`, `merge` commands
- [ ] Sub-server synchronization protocol
- [ ] Remote `RoutedObjectStore` implementation
