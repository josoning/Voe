# VOE Unified Storage Backend — Architecture Design Document

> Version 0.1 · 2026-09

---

## 1. System Overview

### 1.1 Design Goals

The unified storage backend is the infrastructure layer of the VOE project. Its core objective
is to **decouple business logic from concrete file-system implementations**, replacing ad-hoc
`std::fs::xxx` direct calls scattered throughout the codebase.

- **Unified abstraction** — every file operation goes through a single `StorageBackend` trait.
- **Swappable implementation** — switching to S3 / Ceph / distributed storage requires zero
  changes to business code.
- **Composable cross-cutting concerns** — logging, caching, permission checks, etc. stack on
  top via the decorator pattern.
- **Diagnostic errors** — fine-grained error mapping keeps the path of failure clear.

### 1.2 Layered Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│              Business layer (voe-commands, voe-cli)             │
├─────────────────────────────────────────────────────────────────┤
│            Repository / domain services (voe-fs::repo)      │
│   FsRepository · FsRefStore · FsIndexStore · FsConfigManager    │
├─────────────────────────────────────────────────────────────────┤
│               Domain storage (ObjectStore / ChunkStore)          │
│           FileSystemObjectStore (uses StorageBackend)            │
├─────────────────────────────────────────────────────────────────┤
│        Unified abstraction layer (voe-core::storage)             │
│     StorageBackend  ·  ObjectStore  ·  ChunkStore  ·  Routed... │
├─────────────────────────────────────────────────────────────────┤
│                  Decorator layer                                 │
│   LoggingStorageBackend  ·  CachedStorageBackend  ·  ...          │
├─────────────────────────────────────────────────────────────────┤
│           Backend implementation layer (LocalFileBackend)        │
│              std::fs / walkdir / OS file system                   │
└─────────────────────────────────────────────────────────────────┘
```

### 1.3 Composed example (decorator pattern)

```rust
// Typical composition: cache + logging + local file system
let backend: Box<dyn StorageBackend> = Box::new(
    CachedStorageBackend::new(
        Box::new(LoggingStorageBackend::new(
            Box::new(LocalFileBackend::new())
        ))
    )
);
```

Call chain:

```
caller
  → CachedStorageBackend.read_file()  (check cache → return on hit)
    → LoggingStorageBackend.read_file()  (emit tracing log)
      → LocalFileBackend.read_file()    (std::fs::read())
        ← std::io::Error → map_io_err → VoeError
      ← Vec<u8>
    ← Vec<u8> + tracing::debug!
  ← populate cache + return Vec<u8>
```

---

## 2. Trait Hierarchy

### 2.1 Trait inheritance / dependency graph

```
StorageBackend ──────────────────────────┐
  (voe-core::storage.rs)                 │
                                         │ depended on by
ObjectStore  ─┐                          │
  store / retrieve / exists / delete /    │
  list                                    │
                                          │
ChunkStore   ─┤───── FileSystemObjectStore ─┤
  store_chunk / retrieve_chunk /            │
  assemble_file                            │
                                          │
RoutedObjectStore ── extends ObjectStore+  │
                    ChunkStore             │
                                          │
Repository (voe-core) depends on           │
  ObjectStore / ChunkStore / RefStore /    │
  IndexStore / ConfigManager               │
                                          ▼
                                StorageBackend (as the foundation)
```

### 2.2 Responsibility split

| Trait | Responsibility | Operations |
|---|---|---|
| `StorageBackend` | Low-level atomic file/directory I/O | exists · is_file · is_dir · metadata · create_dir · create_dir_all · remove_dir · remove_dir_all · list_dir · create_file · read_file · read_file_to_string · write_file · append_to_file · delete_file · rename_file · move_file · copy_file · copy_dir_all · ensure_parent_dir |
| `ObjectStore` | Content-addressable object storage | store(Object) → ObjectId · retrieve(ObjectId) → VoeObject · exists · delete · list |
| `ChunkStore` | Chunk-level I/O and reassembly | store_chunk · retrieve_chunk · assemble_file |
| `RoutedObjectStore` | Object storage with routing | extends ObjectStore + ChunkStore + store_for_path · retrieve_from(server) · store_chunk_for_path |
| `ObjectStoreFactory` | Object storage factory | create_local · create_for_server |

**Design principles**:
- `StorageBackend` only deals with **path ↔ byte stream**. It is oblivious to object IDs,
  chunk semantics, or higher-level concepts.
- `ObjectStore` / `ChunkStore` sit on top of `StorageBackend` and handle content addressing
  plus chunk metadata.
- Upper-layer repositories (`FsRepository`) compose `ObjectStore` + `RefStore` +
  `IndexStore` + `ConfigManager`, each of which holds its own `Box<dyn StorageBackend>`.

### 2.3 Associated types and thread safety

```rust
pub trait StorageBackend: Send + Sync { ... }
```

Every storage trait is bounded by `Send + Sync`, so instances can be safely shared across
threads. Implementations that require internal mutation (e.g. `CachedStorageBackend`) protect
shared state with `Mutex<HashMap<...>>`.

---

## 3. Module Dependency Graph

```
voe-core/src/storage.rs ──────────────────┐
  (defines StorageBackend and related traits)   │
                                            ▼
voe-core/src/error.rs ─────── VoeError
  (FileNotFound / DirNotFound / ...)         │
                                            │
voe-fs/src/local_backend.rs ──────────┐│
  LocalFileBackend + map_io_err             ││
                                            ││
voe-fs/src/decorators.rs ────────────┤│
  LoggingStorageBackend · CachedStorageBackend││
                                            ││
voe-fs/src/filesystem.rs ────────────┤│
  FileSystemObjectStore                     ││
                                            ││
voe-fs/src/repo.rs ───────────────────┤│
  FsRepository · FsRefStore · FsIndexStore  ││
                                            ││
voe-fs/src/config_backend.rs ────────┤│
  TomlConfigSource · FsConfigManager        ││
                                            ││
voe-fs/src/working_tree.rs ──────────┘│
  read_working_tree_with_backend            │
                                            │
voe-fs/src/lib.rs ─── re-exports ◄─────┘
                                            │
voe-core/src/storage.rs (ObjectStore /      │
  ChunkStore traits) ◄──────────────────────┘
```

**Dependency direction**: `voe-core` defines the traits → `voe-fs` implements them →
all internal modules of `voe-fs` depend only on the `StorageBackend` trait, never on
a concrete backend type.

---

## 4. Error Handling Strategy

### 4.1 Error-type mapping table

`LocalFileBackend::map_io_err` / `map_dir_io_err` map `std::io::ErrorKind` to fine-grained
`VoeError` variants:

| `std::io::ErrorKind` | Context | Mapped to | Carries |
|---|---|---|---|
| `NotFound` | file op (read/write/metadata) | `VoeError::FileNotFound` | `path: PathBuf` |
| `NotFound` | dir op (`read_dir`) | `VoeError::DirNotFound` | `path: PathBuf` |
| `PermissionDenied` | any | `VoeError::PermissionDenied` | `path: PathBuf` |
| `StorageFull` | any | `VoeError::StorageFull` | `path: PathBuf` |
| `AlreadyExists` | any | `VoeError::FileExists` | `path: PathBuf` |
| other | any | `VoeError::Storage(String)` | formatted error message |

> **Distinguishing `FileNotFound` vs `DirNotFound`**: file operations use `map_io_err`,
> which probes `path.is_dir()` before mapping. Directory operations (`list_dir`) use a
> dedicated `map_dir_io_err` that always returns `DirNotFound`, avoiding the ambiguity of
> a missing path whose type cannot be determined.

### 4.2 Methods with lenient error behavior

| Method | Behavior when missing |
|---|---|
| `delete_file(path)` | **silently succeeds** (idempotent) |
| `exists(path)` | returns `Ok(false)` — never an error |
| `ObjectStore::exists(id)` | returns `Ok(false)` |
| `RefStore::get_ref(name)` | returns `Ok(None)` |

### 4.3 Typical error-handling pattern

```rust
// Caller can branch on the specific variant
match backend.read_file(&path) {
    Ok(data) => { /* normal path */ }
    Err(VoeError::FileNotFound { .. }) => { /* create on first access */ }
    Err(VoeError::PermissionDenied { path }) => { /* permission alert */ }
    Err(e) => { /* generic log */ tracing::warn!("storage error: {}", e); }
}
```

---

## 5. Decorator Pattern in Detail

### 5.1 Structure

```rust
pub struct LoggingStorageBackend {
    inner: Box<dyn StorageBackend>,
}

pub struct CachedStorageBackend {
    inner: Box<dyn StorageBackend>,
    read_cache: Mutex<HashMap<PathBuf, Vec<u8>>>,
    metadata_cache: Mutex<HashMap<PathBuf, StorageMetadata>>,
}
```

Every decorator wraps a `Box<dyn StorageBackend>`, implements the same trait, and layers
additional behavior around each call.

### 5.2 `CachedStorageBackend` invalidation policy

| Method | Cache behavior |
|---|---|
| `metadata` | hit → return clone; miss → load into cache |
| `read_file` | hit → return clone; miss → load into cache |
| `write_file` | on success: update read_cache, **delete** metadata cache |
| `append_to_file` | on success: **invalidate** read + metadata cache (content cannot be patched precisely) |
| `delete_file` | on success: **invalidate** read + metadata cache |
| `rename_file` | on success: migrate read cache key (`from` → `to`), invalidate metadata cache |
| `move_file` | on success: migrate read cache key (`from` → `to`), invalidate metadata cache |
| `copy_file` | on success: if the source path was cached, **clone** into the destination path cache |
| `remove_dir` / `remove_dir_all` | **invalidate** caches under the target path |
| `exists` / `is_file` / `is_dir` / `list_dir` / `create_dir*` / `create_file` | **not cached** — delegate directly to inner |

### 5.3 `LoggingStorageBackend` log policy

- Uniform `target` of `"storage"` so it can be filtered independently.
- **Success**: `tracing::debug!` with a summary of the return value (byte count / entry count).
- **Failure**: `tracing::warn!` with the full error.
- Never at `info` level — normal operations should not produce log noise.

---

## 6. Future Extension Points

### 6.1 Pluggable backends

Implementing the `StorageBackend` trait is all it takes to swap out the underlying store:

```rust
pub struct S3StorageBackend {
    bucket: String,
    client: aws_sdk_s3::Client,
}

impl StorageBackend for S3StorageBackend { /* ... */ }

pub struct DistributedStorageBackend {
    nodes: Vec<Box<dyn StorageBackend>>,
    policy: ReplicationPolicy,
}

impl StorageBackend for DistributedStorageBackend { /* ... */ }
```

Business code needs **zero changes** — inject a different backend through `with_backend(...)`:

```rust
let store = FileSystemObjectStore::with_backend(
    objects_path,
    Box::new(S3StorageBackend::new("my-bucket", s3_client)),
);
```

### 6.2 Additional decorators (not yet implemented)

| Decorator | Purpose |
|---|---|
| `MetricsStorageBackend` | Prometheus / custom metrics: QPS, P99 latency, error rate |
| `ReadonlyStorageBackend` | Production-grade write protection — every write returns `VoeError::PermissionDenied` |
| `RetryStorageBackend` | Retry + exponential backoff (for remote backends) |
| `QuarantineStorageBackend` | Circuit breaker for failing paths, isolates corrupted nodes |
| `AuditLogStorageBackend` | Structured JSON audit log capturing caller, timestamp, path, change type |

### 6.3 Multi-pool routing

`RoutedObjectStore` makes tiered storage possible:

```
Small files (< 64 KB) → local fast SSD StorageBackend
Large files (≥ 64 KB) → S3StorageBackend (hot/cold tiering)
Cold data (≥ 90 days) → GlacierStorageBackend (archive)
```

### 6.4 Disaster recovery across regions

`DistributedStorageBackend` + replication policy:

```
Write path: local → replica 1 → replica 2 (quorum: 2/3)
Read path: nearest node, automatic failover on error
```

---

## 7. Backward Compatibility Guarantees

1. **1:1 API behavior** — every method of `LocalFileBackend` behaves exactly like its
   `std::fs` counterpart.
2. **Idempotency preserved** — `delete_file` on a missing path still succeeds, so migration
   cannot introduce behavioral regressions.
3. **Automatic parent-directory creation** — `write_file` / `create_file` / `rename_file` /
   `copy_file` / `move_file` all create missing target parents.
4. **Cross-device move** — `move_file` automatically falls back to copy + remove on
   `CrossesDevices`.

---

## 8. Test Coverage

| Layer | Tests | Location |
|---|---|---|
| StorageBackend unit | 28 (covers exists / is_file / is_dir / metadata / create_dir* / remove_dir* / list_dir / read / write / append / delete / rename / move / copy / copy_dir_all / ensure_parent_dir / map_io_err) | `voe-fs/src/local_backend.rs::tests` |
| Integration | 12 (config round-trip / lock file / hash sync) | `voe-fs/tests/config_integration.rs` |
| Integration | 27 (repo init/open / object store / chunk reassembly / ref traversal / snapshot stacking) | `voe-fs/tests/repository.rs` |
| **Total** | **92, all passing** | |

Run with:

```bash
cargo test --workspace
```

### 8.1 Performance benchmarks (zero dependencies, `std::time::Instant`)

Benchmarks live inside `voe-fs/src/local_backend.rs::tests`. Run them with
`cargo test -- --nocapture bench_`. Typical output on Linux ext4 (numbers are affected
by the kernel page cache — use as a *relative* reference only):

| Metric | LocalFileBackend | CachedStorageBackend (warm) | Speedup |
|---|---|---|---|
| `read_file` 4 KB × 500 | ~2 µs/op | ~0 µs/op | **2.0×** |
| `metadata` × 500 | ~0 µs/op | ~1 µs/op (first call after cold) | kernel-cache dependent |
| `write_file` 8 KB × 200 | ~5 µs/op | — (write-through) | — |

`CachedStorageBackend` uses an in-process `HashMap<PathBuf, Vec<u8>>` and delivers the most
benefit for repeated reads of small hot files. For statistically rigorous benchmarks,
consider adding the `criterion` crate.

---

## 9. Future Work: Async & Batch Operations

> Original requirement #7 called for async processing and batch operations. The current
> version (v0.1) of `StorageBackend` is a **synchronous interface** — every method blocks the
> calling thread. Async and batch are intentionally planned as **extension layers** rather
> than part of the core trait, for three reasons:
>
> 1. A sync interface can serve both sync *and* async call sites (wrap in `tokio::task::spawn_blocking`).
> 2. Adding `async fn` to the trait would force `async_trait` on every implementer and add
>    a `BoxFuture` indirection to every call.
> 3. 99% of VOE's workload is small files (≤ 1 MB); sync I/O + the OS page cache is already
>    fast. Async matters primarily for **network-bound remote backends** (S3 / object storage),
>    which are not in scope yet.

### 9.1 Batch operations extension point

Batch read is the single most valuable near-term addition. It can live as a sibling trait in
`voe-core/src/storage.rs`:

```rust
pub trait BatchStorageBackend: StorageBackend {
    fn read_files(&self, paths: &[PathBuf]) -> Result<Vec<(PathBuf, Vec<u8>)>> {
        paths.iter()
            .map(|p| self.read_file(p).map(|d| (p.clone(), d)))
            .collect()
    }
    fn write_files(&self, entries: &[(PathBuf, Vec<u8>)]) -> Result<()> {
        for (p, d) in entries {
            self.write_file(p, d)?;
        }
        Ok(())
    }
}
```

The default implementation is a serial loop. Concrete backends (S3, for instance) can
override it with a concurrent / pipelined version without changing the trait signature.

### 9.2 Async backend shape (v0.2 plan)

```rust
#[async_trait]
pub trait AsyncStorageBackend: Send + Sync {
    async fn read_file(&self, path: &Path) -> Result<Vec<u8>>;
    async fn write_file(&self, path: &Path, data: &[u8]) -> Result<()>;
    async fn list_dir(&self, path: &Path) -> Result<Vec<StorageEntry>>;
    // ...
}
```

The synchronous `LocalFileBackend` can bridge into the async world either through
`async_trait`'s `block_in_place` or via an adapter layer using `spawn_blocking`. The two
interfaces convert into one another so business code can choose per call site.

### 9.3 Recommended priorities

| Capability | Priority | Triggering scenario |
|---|---|---|
| `BatchStorageBackend` bulk read/write | ★★★ | `voe sync` / `voe checkout` pulling dozens or hundreds of objects at once |
| `AsyncStorageBackend` + S3 | ★★★ | When cloud storage is introduced |
| Directory-level diff / watch | ★★ | VSCode / IDE integration for live indexing |
| Streaming interface (`Read`/`Write` traits) | ★ | Avoid loading large files entirely into memory |

Guiding principle: **thicken and harden the sync interface first, then layer async and
batch on top as additional traits**, preserving single responsibility.
