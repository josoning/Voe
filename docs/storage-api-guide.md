# VOE Storage Backend — API Usage Guide

> Applies to version 0.1 · Code locations: `crates/voe-fs` and `crates/voe-core::storage`

---

## 1. Quick Start

### 1.1 Minimal working example

```rust
use std::path::PathBuf;
use voe_fs::{LocalFileBackend, StorageBackend};

fn main() -> voe_core::error::Result<()> {
    let backend = LocalFileBackend::new();

    let root = PathBuf::from("/tmp/voe-demo");
    backend.create_dir_all(&root)?;

    backend.write_file(
        &root.join("hello.txt"),
        b"Hello, StorageBackend!",
    )?;

    let content = backend.read_file_to_string(&root.join("hello.txt"))?;
    println!("{}", content);

    backend.copy_file(
        &root.join("hello.txt"),
        &root.join("backup/hello.copy"),
    )?;

    Ok(())
}
```

### 1.2 Enable logging and caching

```rust
use voe_fs::{
    LocalFileBackend, LoggingStorageBackend, CachedStorageBackend,
    StorageBackend,
};

// Bare-metal: direct file system
let raw: Box<dyn StorageBackend> = Box::new(LocalFileBackend::new());

// Stack cache + logging (decorators can nest in any order)
let production: Box<dyn StorageBackend> = Box::new(
    CachedStorageBackend::new(
        Box::new(LoggingStorageBackend::new(raw)),
    )
);

// Preferred order: logging on the outside, cache on the inside.
// Rationale: cache hits produce **no** log noise.
let recommended: Box<dyn StorageBackend> = Box::new(
    LoggingStorageBackend::new(
        Box::new(CachedStorageBackend::new(
            Box::new(LocalFileBackend::new())
        ))
    )
);
```

### 1.3 Configure `tracing` log output

```rust
fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // LoggingStorageBackend uses target = "storage"
    // Enable it independently via RUST_LOG=storage=debug
}
```

---

## 2. Type Overview

### 2.1 Re-export manifest

```rust
// From voe-fs
pub use voe_fs::{
    // Trait
    StorageBackend,

    // Concrete implementations
    LocalFileBackend,

    // Decorators
    LoggingStorageBackend,
    CachedStorageBackend,

    // Domain storage implementations
    FileSystemObjectStore,
    FsRefStore,
    FsIndexStore,
    FsRepository,
    FsRepoManager,

    // Helpers
    StorageEntryType,
    StorageMetadata,
};

// From voe-core::storage
use voe_core::storage::{ObjectStore, ChunkStore, RoutedObjectStore, ObjectStoreFactory};
```

### 2.2 Core data structures

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageEntryType {
    File,
    Directory,
    Symlink,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct StorageMetadata {
    pub entry_type: StorageEntryType,
    pub size: u64,
    pub created: Option<SystemTime>,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub is_hidden: bool,   // name starts with '.'
}
```

### 2.3 Error types

```rust
use voe_core::error::VoeError;

match backend.read_file(&path) {
    Ok(data) => { /* ... */ }
    Err(VoeError::FileNotFound { path }) => { /* file missing */ }
    Err(VoeError::DirNotFound { path }) => { /* directory missing */ }
    Err(VoeError::PermissionDenied { path }) => { /* no access */ }
    Err(VoeError::StorageFull { path }) => { /* disk full */ }
    Err(VoeError::FileExists { path }) => { /* already exists */ }
    Err(VoeError::InvalidPath { path, reason }) => { /* invalid path */ }
    Err(VoeError::Storage(msg)) => { /* generic low-level I/O error */ }
    Err(other) => { /* any other error */ }
}
```

---

## 3. `StorageBackend` Method Reference

### 3.1 Query methods (pure reads, no side effects)

| Signature | Semantics | Return |
|---|---|---|
| `exists(&self, path: &Path) -> Result<bool>` | `true` if path exists, `false` if missing (**never errors**) | `Ok(bool)` |
| `is_file(&self, path: &Path) -> Result<bool>` | `true` if it is a regular file; `false` when missing or not a file | `Ok(bool)` |
| `is_dir(&self, path: &Path) -> Result<bool>` | `true` if it is a directory; `false` when missing or not a directory | `Ok(bool)` |
| `metadata(&self, path: &Path) -> Result<StorageMetadata>` | Full metadata | `Err(FileNotFound)` when missing |

```rust
let meta = backend.metadata(&some_path)?;
assert!(meta.is_file());
assert_eq!(meta.size, 4096);
assert!(meta.is_hidden);           // name starts with '.'
println!("last modified: {:?}", meta.modified);
```

### 3.2 Directory operations

| Signature | Semantics | Possible error |
|---|---|---|
| `create_dir(&self, path: &Path) -> Result<()>` | Create a **single** directory | `FileExists` if present; `ParentNotFound` if parent is missing |
| `create_dir_all(&self, path: &Path) -> Result<()>` | Recursively create all ancestors | Idempotent — silently succeeds if already exists |
| `remove_dir(&self, path: &Path) -> Result<()>` | Remove an **empty** directory | `Storage` if the directory is not empty |
| `remove_dir_all(&self, path: &Path) -> Result<()>` | Recursively remove the directory and all its contents | `DirNotFound` if missing |
| `list_dir(&self, path: &Path) -> Result<Vec<PathBuf>>` | List immediate children (non-recursive) | `DirNotFound` if path is missing or not a directory |

### 3.3 File operations

| Signature | Semantics | Notes |
|---|---|---|
| `create_file(&self, path: &Path) -> Result<()>` | Create an empty file (with nested parents) | Parents auto-created |
| `read_file(&self, path: &Path) -> Result<Vec<u8>>` | Read entire file into memory | `FileNotFound` if missing |
| `read_file_to_string(&self, path: &Path) -> Result<String>` | Read and decode as UTF-8 (default impl) | Non-UTF-8 content → `Storage` error |
| `write_file(&self, path: &Path, data: &[u8]) -> Result<()>` | Overwrite (or create) a file | **Parents auto-created** |
| `append_to_file(&self, path: &Path, data: &[u8]) -> Result<()>` | Append data | **Parents auto-created**; file created if missing |
| `delete_file(&self, path: &Path) -> Result<()>` | Delete a file | **Idempotent**: returns `Ok` when missing |
| `rename_file(&self, from: &Path, to: &Path) -> Result<()>` | Atomic rename | **`to`'s parent auto-created** |
| `move_file(&self, from: &Path, to: &Path) -> Result<()>` | Move file (rename + copy/remove fallback on cross-device) | **`to`'s parent auto-created** |
| `copy_file(&self, from: &Path, to: &Path) -> Result<()>` | Copy file | **`to`'s parent auto-created** |

### 3.4 Default-implemented helpers

| Signature | Implementation |
|---|---|
| `read_file_to_string(...)` | `String::from_utf8(self.read_file(path)?)` |
| `copy_dir_all(from, to)` | recursive `create_dir_all(to)` → `list_dir(from)` → per-file `copy_file` / recursive call for subdirectories |
| `ensure_parent_dir(path)` | `self.exists(parent)?` → `create_dir_all(parent)` if missing |

### 3.5 Summary of automatic parent-directory creation

The following methods **recursively create missing parent directories** for the target path:

- `create_file(path)`
- `write_file(path, ...)`
- `append_to_file(path, ...)`
- `rename_file(from, to)` — creates `to.parent()`
- `move_file(from, to)` — creates `to.parent()`
- `copy_file(from, to)` — creates `to.parent()`

Call `backend.ensure_parent_dir(p)` explicitly whenever you need the guarantee ahead of time.

---

## 4. Decorator Usage Patterns

### 4.1 Logging only

```rust
let backend = LoggingStorageBackend::new(Box::new(LocalFileBackend::new()));
backend.write_file(&p, data)?;  // tracing::debug!(target:"storage", ...)
```

### 4.2 Caching only

```rust
let backend = CachedStorageBackend::new(Box::new(LocalFileBackend::new()));
let a = backend.read_file(&p)?;   // read from disk + populate cache
let b = backend.read_file(&p)?;   // cache hit — zero I/O

// Manual eviction (tests / special cases)
backend.clear_cache();
```

### 4.3 Composition (recommended order)

```rust
// Outside → Inside: Logging → Caching → Local
// Benefit: cache hits do not produce log noise
let backend: Box<dyn StorageBackend> = Box::new(
    LoggingStorageBackend::new(
        Box::new(CachedStorageBackend::new(
            Box::new(LocalFileBackend::new())
        ))
    )
);
```

### 4.4 Composition inside a repository

```rust
// Inside FsRepository::create_new, for example:
let backend: Box<dyn StorageBackend> = Box::new(
    CachedStorageBackend::new(
        Box::new(LoggingStorageBackend::new(
            Box::new(LocalFileBackend::new())
        ))
    )
);

// Each sub-module can still hold its own backend instance
let objects = FileSystemObjectStore::with_backend(
    voe_dir.join("objects"),
    Box::new(LocalFileBackend::new()),
);
```

---

## 5. Domain Storage Integration Guide

### 5.1 `FileSystemObjectStore`

Content-addressable object storage with auto-prefixed paths (`objects/ab/cdef...`):

```rust
use voe_core::object::{ObjectId, VoeObject, ObjectKind};
use voe_fs::FileSystemObjectStore;

// Uses LocalFileBackend by default
let store = FileSystemObjectStore::new("/repo/.voe/objects");

// Inject a custom backend
let store = FileSystemObjectStore::with_backend(
    "/repo/.voe/objects",
    Box::new(LoggingStorageBackend::new(Box::new(LocalFileBackend::new()))),
);

// Store and retrieve
let obj = VoeObject::new(ObjectKind::Blob, b"hello world".to_vec());
let id: ObjectId = store.store(&obj)?;

let retrieved = store.retrieve(&id)?;
assert_eq!(retrieved.content, b"hello world");
```

### 5.2 `FsRefStore`

HEAD and branch ref storage:

```rust
use voe_fs::FsRefStore;

let refs = FsRefStore::with_backend(
    "/repo/.voe/refs",        // refs_dir
    "/repo/.voe/HEAD",        // head_path
    Box::new(LocalFileBackend::new()),
);

refs.set_head(&some_object_id)?;
let head = refs.get_head()?;
assert!(head.is_some());

refs.set_ref("feature/test", &branch_id)?;
let all = refs.list_refs()?;  // Vec<(name, ObjectId)>
```

### 5.3 `FsIndexStore`

Staging area / index persistence:

```rust
use voe_core::commit::IndexState;
use voe_fs::FsIndexStore;

let index = FsIndexStore::with_backend(
    "/repo/.voe/index.json",
    Box::new(LocalFileBackend::new()),
);

let state: IndexState = index.load()?;
// mutate state ...
index.save(&state)?;
```

### 5.4 `TomlConfigSource` / `FsConfigManager`

TOML config file + lock hash:

```rust
use voe_fs::{TomlConfigSource, FsConfigManager};

let source = TomlConfigSource::new("/repo/voeconfig.toml");
let manager = FsConfigManager::new(source, "/repo/.voe/voeconfig.lock");

manager.init_defaults()?;      // first-time initialization
manager.set_user("alice")?;    // write TOML + update lock hash
let cfg = manager.config()?;   // read current config
```

### 5.5 `working_tree`

Scan workspace files:

```rust
use std::path::PathBuf;
use voe_fs::working_tree::{read_working_tree_with_backend, diff_to_masks};

let root = PathBuf::from("/repo");
let backend = LocalFileBackend::new();
let tree = read_working_tree_with_backend(&root, backend)?;
// HashMap<PathBuf /* relative path */, Vec<u8> /* content */>

let masks = diff_to_masks(&old_snapshot, &tree)?;
```

---

## 6. Implementing a Custom `StorageBackend`

### 6.1 Implementation template

```rust
use std::path::{Path, PathBuf};
use voe_core::error::{Result, VoeError};
use voe_core::storage::{StorageBackend, StorageEntryType, StorageMetadata};

pub struct MyCustomBackend { /* internal resources: AWS Client, DB connection, ... */ }

impl MyCustomBackend {
    pub fn new(/* ... */) -> Self { Self { /* ... */ } }

    // Recommended: a uniform I/O error mapper, cf. LocalFileBackend::map_io_err
    fn map_err(&self, path: &Path, e: /* underlying error type */) -> VoeError {
        // ...
    }
}

impl StorageBackend for MyCustomBackend {
    fn exists(&self, path: &Path) -> Result<bool> { /* ... */ }
    fn is_file(&self, path: &Path) -> Result<bool> { /* ... */ }
    fn is_dir(&self, path: &Path) -> Result<bool> { /* ... */ }
    fn metadata(&self, path: &Path) -> Result<StorageMetadata> { /* ... */ }

    fn create_dir(&self, path: &Path) -> Result<()> { /* ... */ }
    fn create_dir_all(&self, path: &Path) -> Result<()> { /* ... */ }
    fn remove_dir(&self, path: &Path) -> Result<()> { /* ... */ }
    fn remove_dir_all(&self, path: &Path) -> Result<()> { /* ... */ }
    fn list_dir(&self, path: &Path) -> Result<Vec<PathBuf>> { /* ... */ }

    fn create_file(&self, path: &Path) -> Result<()> { /* ... */ }
    fn read_file(&self, path: &Path) -> Result<Vec<u8>> { /* ... */ }
    fn write_file(&self, path: &Path, data: &[u8]) -> Result<()> { /* ... */ }
    fn append_to_file(&self, path: &Path, data: &[u8]) -> Result<()> { /* ... */ }
    fn delete_file(&self, path: &Path) -> Result<()> { /* ... */ }
    fn rename_file(&self, from: &Path, to: &Path) -> Result<()> { /* ... */ }
    fn move_file(&self, from: &Path, to: &Path) -> Result<()> { /* ... */ }
    fn copy_file(&self, from: &Path, to: &Path) -> Result<()> { /* ... */ }
    // read_file_to_string / copy_dir_all / ensure_parent_dir can use default impls
}
```

### 6.2 Semantic contracts you *must* honor

| Contract | Explanation |
|---|---|
| **`exists` returns `Ok(false)` for missing paths** | Not an `Err` |
| **`delete_file` is idempotent** | Returns `Ok(())` when missing |
| **`create_dir` does *not* auto-create parents** | Missing parent → `Err`; use `create_dir_all` for recursive |
| **`list_dir` is non-recursive** | Only immediate children |
| **`read_file` returns the full content in one shot** | For streaming, consider adding a future extension trait |
| **Returned `Path`s are...** | Absolute or relative depending on the backend's base semantics — document it clearly |
| **`Send + Sync`** | Required. Repository code may hold the backend across thread boundaries |

### 6.3 Make your backend visible to domain stores

Every domain store exposes a `with_backend` constructor:

```rust
// FileSystemObjectStore
let store = FileSystemObjectStore::with_backend(
    base_path,
    Box::new(MyCustomBackend::new()),
);

// FsRefStore
let refs = FsRefStore::with_backend(refs_dir, head_path, Box::new(MyCustomBackend::new()));

// FsIndexStore
let index = FsIndexStore::with_backend(index_path, Box::new(MyCustomBackend::new()));

// FsRepository (already supported — added in this session)
let repo = FsRepository::with_backend(
    repo_path,
    Box::new(MyCustomBackend::new()),
);
```

---

## 7. Migrating `std::fs` Direct Access

### 7.1 Drop-in replacement cheat sheet

| Before | After |
|---|---|
| `fs::read_to_string(p).unwrap()` | `backend.read_file_to_string(p)?` |
| `fs::write(p, data).unwrap()` | `backend.write_file(p, data)?` |
| `fs::create_dir_all(p).unwrap()` | `backend.create_dir_all(p)?` |
| `fs::remove_file(p).unwrap()` | `backend.delete_file(p)?` (automatically idempotent) |
| `fs::rename(a, b).unwrap()` | `backend.rename_file(a, b)?` |
| `fs::copy(a, b).unwrap()` | `backend.copy_file(a, b)?` |
| `fs::metadata(p)?.len()` | `backend.metadata(p)?.size` |
| `fs::read_dir(p)?.collect()` | `backend.list_dir(p)?` |
| `if p.exists() { ... }` | `if backend.exists(&p)? { ... }` |
| `fs::create_dir(p)` may fail | `backend.create_dir(p)?` or `create_dir_all` |

### 7.2 Migration steps

1. **Obtain a backend instance** — either from an existing repository field, or inject a `Box<dyn StorageBackend>` at construction.
2. **Replace calls** using the cheat sheet above.
3. **Adjust error handling** — switch from `.unwrap()` to `?` or `match` on `VoeError`.
4. **Remove** any leftover `use std::fs;`.
5. Run `cargo test --workspace` to verify.

### 7.3 Already migrated modules

| Crate | File | Migration |
|---|---|---|
| `voe-fs` | `filesystem.rs` | `FileSystemObjectStore` switched to `backend.write_file/read_file/exists/...` |
| `voe-fs` | `repo.rs` | `FsRefStore` / `FsIndexStore` / `FsRepository` all use the backend |
| `voe-fs` | `config_backend.rs` | `TomlConfigSource` / `FsConfigManager` use the backend |
| `voe-fs` | `working_tree.rs` | Added `_with_backend` variants; originals wrap `LocalFileBackend` |

---

## 8. Performance & Concurrency

### 8.1 `LocalFileBackend` performance

`LocalFileBackend` is a thin wrapper around `std::fs`. Performance is **identical** to direct
`std::fs` calls — zero additional overhead.

### 8.2 `CachedStorageBackend` characteristics

| Trait | Details |
|---|---|
| Read cache | `HashMap<PathBuf, Vec<u8>>`, returns a clone on hit |
| Metadata cache | `HashMap<PathBuf, StorageMetadata>` |
| Concurrency | Protected by `Mutex` — simple and correct. An extreme read-heavy workload could be switched to `RwLock` later. |
| Invalidation | Writes / deletes auto-invalidate; `rename` / `move` migrate keys; `copy` clones the source cache entry to the target |
| Memory | No upper bound — watch memory in large-file scenarios |

### 8.3 Thread safety

Every backend implementation satisfies `Send + Sync` and can be shared across threads.
`CachedStorageBackend` uses an internal `Mutex` to keep concurrent access safe.

---

## 9. FAQ

**Q: `read_file` loads the entire file into memory. What about large files?**

A: The current trait is intentionally a full-byte-stream interface — it matches VOE's workload
(objects are typically chunk-level, on the order of tens of KB). For streaming, consider a
future extension trait `StorageBackendStream` (e.g. `read_file_stream(path) -> impl AsyncRead`).

**Q: Why does `create_file` auto-create parents but `create_dir` does not?**

A: For symmetry — `create_dir` mirrors the semantics of `std::fs::create_dir` (no parent
creation), forcing the caller to explicitly call `create_dir_all` when that is the intent.
File operations (`write_file` / `create_file` / `append_to_file`) are almost always "write
to this exact target path", so auto-creating parents is the right default.

**Q: `delete_file` returns `Ok` for missing paths — won't that hide bugs?**

A: It is **intentionally idempotent**, mirroring the spirit of `fs::remove_file` (where the
`NotFound` error is often swallowed by callers). If you need strict verification, call
`exists()` before `delete_file()`.

**Q: How do I inject a custom backend into `FsRepository`?**

A: `FsRepository::with_backend(path, backend)` and `FsRepository::open_with_backend(path, backend)`
were added specifically for this — see §6.3.

**Q: Does decorator order matter?**

A: Yes. `Logging → Caching → Local` (outside → inside) means:
- Cache hits produce **no** log output (good)
- Cache misses log after the inner call completes
- Writes log before executing (if the order were reversed)

Recommended rule of thumb: **outer layers = cross-cutting concerns (logging / metrics)**,
**inner layers = performance concerns (caching / retry)**, **core = the actual backend**.

---

## 10. Reference

| File | Description |
|---|---|
| `crates/voe-core/src/storage.rs` | Trait definitions (inline docs generated automatically by `cargo doc`) |
| `crates/voe-core/src/error.rs` | Error type definitions |
| `crates/voe-fs/src/local_backend.rs` | `LocalFileBackend` implementation + 28 unit tests |
| `crates/voe-fs/src/decorators.rs` | Decorator implementations |
| `crates/voe-fs/src/filesystem.rs` | `FileSystemObjectStore` (ObjectStore + ChunkStore) |
| `crates/voe-fs/src/repo.rs` | `FsRefStore` / `FsIndexStore` / `FsRepository` |
| `crates/voe-fs/src/config_backend.rs` | `TomlConfigSource` / `FsConfigManager` |
| `docs/storage-architecture.md` | Architecture design document |
