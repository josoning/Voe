use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use voe_storage_api::{StorageBackend, StorageEntryType, StorageMetadata};
use voe_types::error::{Result, VoeError};

pub struct LocalFileBackend;

impl LocalFileBackend {
    pub fn new() -> Self {
        Self
    }

    fn map_io_err(path: &Path, e: std::io::Error) -> VoeError {
        match e.kind() {
            std::io::ErrorKind::NotFound => {
                if path.is_dir() {
                    VoeError::DirNotFound {
                        path: path.to_path_buf(),
                    }
                } else {
                    VoeError::FileNotFound {
                        path: path.to_path_buf(),
                    }
                }
            }
            std::io::ErrorKind::PermissionDenied => VoeError::PermissionDenied {
                path: path.to_path_buf(),
            },
            std::io::ErrorKind::StorageFull => VoeError::StorageFull {
                path: path.to_path_buf(),
            },
            std::io::ErrorKind::AlreadyExists => VoeError::FileExists {
                path: path.to_path_buf(),
            },
            _ => VoeError::Storage(format!("IO error on {}: {}", path.display(), e)),
        }
    }

    fn map_dir_io_err(path: &Path, e: std::io::Error) -> VoeError {
        match e.kind() {
            std::io::ErrorKind::NotFound => VoeError::DirNotFound {
                path: path.to_path_buf(),
            },
            std::io::ErrorKind::PermissionDenied => VoeError::PermissionDenied {
                path: path.to_path_buf(),
            },
            std::io::ErrorKind::AlreadyExists => VoeError::FileExists {
                path: path.to_path_buf(),
            },
            _ => VoeError::Storage(format!("IO error on {}: {}", path.display(), e)),
        }
    }

    fn file_type_to_entry_type(ft: fs::FileType) -> StorageEntryType {
        if ft.is_file() {
            StorageEntryType::File
        } else if ft.is_dir() {
            StorageEntryType::Directory
        } else if ft.is_symlink() {
            StorageEntryType::Symlink
        } else {
            StorageEntryType::Unknown
        }
    }
}

impl Default for LocalFileBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl StorageBackend for LocalFileBackend {
    fn exists(&self, path: &Path) -> Result<bool> {
        Ok(path.exists())
    }

    fn is_file(&self, path: &Path) -> Result<bool> {
        Ok(path.is_file())
    }

    fn is_dir(&self, path: &Path) -> Result<bool> {
        Ok(path.is_dir())
    }

    fn metadata(&self, path: &Path) -> Result<StorageMetadata> {
        let meta = fs::metadata(path).map_err(|e| Self::map_io_err(path, e))?;
        let entry_type = Self::file_type_to_entry_type(meta.file_type());
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let is_hidden = file_name.starts_with('.');
        Ok(StorageMetadata {
            entry_type,
            size: meta.len(),
            created: meta.created().ok(),
            modified: meta.modified().ok(),
            accessed: meta.accessed().ok(),
            is_hidden,
        })
    }

    fn create_dir(&self, path: &Path) -> Result<()> {
        fs::create_dir(path).map_err(|e| Self::map_io_err(path, e))
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        fs::create_dir_all(path).map_err(|e| Self::map_io_err(path, e))
    }

    fn remove_dir(&self, path: &Path) -> Result<()> {
        fs::remove_dir(path).map_err(|e| Self::map_io_err(path, e))
    }

    fn remove_dir_all(&self, path: &Path) -> Result<()> {
        fs::remove_dir_all(path).map_err(|e| Self::map_io_err(path, e))
    }

    fn list_dir(&self, path: &Path) -> Result<Vec<PathBuf>> {
        let entries = fs::read_dir(path).map_err(|e| Self::map_dir_io_err(path, e))?;
        let mut result = Vec::new();
        for entry in entries {
            let entry =
                entry.map_err(|e| VoeError::Storage(format!("Failed to read dir entry: {}", e)))?;
            result.push(entry.path());
        }
        Ok(result)
    }

    fn create_file(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| Self::map_io_err(parent, e))?;
            }
        }
        fs::File::create(path)
            .map(|_| ())
            .map_err(|e| Self::map_io_err(path, e))
    }

    fn read_file(&self, path: &Path) -> Result<Vec<u8>> {
        fs::read(path).map_err(|e| Self::map_io_err(path, e))
    }

    fn write_file(&self, path: &Path, data: &[u8]) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| Self::map_io_err(parent, e))?;
            }
        }
        fs::write(path, data).map_err(|e| Self::map_io_err(path, e))
    }

    fn append_to_file(&self, path: &Path, data: &[u8]) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| Self::map_io_err(parent, e))?;
            }
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| Self::map_io_err(path, e))?;
        file.write_all(data).map_err(|e| {
            VoeError::Storage(format!("Failed to append to {}: {}", path.display(), e))
        })
    }

    fn delete_file(&self, path: &Path) -> Result<()> {
        if !path.exists() {
            return Ok(());
        }
        fs::remove_file(path).map_err(|e| Self::map_io_err(path, e))
    }

    fn rename_file(&self, from: &Path, to: &Path) -> Result<()> {
        if let Some(parent) = to.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| Self::map_io_err(parent, e))?;
            }
        }
        fs::rename(from, to).map_err(|e| Self::map_io_err(from, e))
    }

    fn move_file(&self, from: &Path, to: &Path) -> Result<()> {
        if let Some(parent) = to.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| Self::map_io_err(parent, e))?;
            }
        }
        match fs::rename(from, to) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
                fs::copy(from, to).map_err(|e| Self::map_io_err(from, e))?;
                fs::remove_file(from).map_err(|e| Self::map_io_err(from, e))
            }
            Err(e) => Err(Self::map_io_err(from, e)),
        }
    }

    fn copy_file(&self, from: &Path, to: &Path) -> Result<()> {
        if let Some(parent) = to.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| Self::map_io_err(parent, e))?;
            }
        }
        fs::copy(from, to)
            .map(|_| ())
            .map_err(|e| Self::map_io_err(from, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tempdir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        let nonce = format!("voe_fs_test_{}_{}", name, std::process::id());
        p.push(nonce);
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn cleanup(p: &Path) {
        let _ = fs::remove_dir_all(p);
    }

    #[test]
    fn test_exists_returns_false_for_nonexistent() {
        let dir = tempdir("exists_false");
        let backend = LocalFileBackend::new();
        let target = dir.join("nope.txt");
        assert!(!backend.exists(&target).unwrap());
        cleanup(&dir);
    }

    #[test]
    fn test_exists_returns_true_for_file() {
        let dir = tempdir("exists_file");
        fs::write(dir.join("a.txt"), b"hello").unwrap();
        let backend = LocalFileBackend::new();
        assert!(backend.exists(&dir.join("a.txt")).unwrap());
        cleanup(&dir);
    }

    #[test]
    fn test_exists_returns_true_for_dir() {
        let dir = tempdir("exists_dir");
        fs::create_dir(dir.join("subdir")).unwrap();
        let backend = LocalFileBackend::new();
        assert!(backend.exists(&dir.join("subdir")).unwrap());
        cleanup(&dir);
    }

    #[test]
    fn test_is_file() {
        let dir = tempdir("is_file");
        fs::write(dir.join("f.txt"), b"x").unwrap();
        fs::create_dir(dir.join("d")).unwrap();
        let backend = LocalFileBackend::new();
        assert!(backend.is_file(&dir.join("f.txt")).unwrap());
        assert!(!backend.is_file(&dir.join("d")).unwrap());
        assert!(!backend.is_file(&dir.join("missing")).unwrap());
        cleanup(&dir);
    }

    #[test]
    fn test_is_dir() {
        let dir = tempdir("is_dir");
        fs::write(dir.join("f.txt"), b"x").unwrap();
        fs::create_dir(dir.join("d")).unwrap();
        let backend = LocalFileBackend::new();
        assert!(backend.is_dir(&dir.join("d")).unwrap());
        assert!(!backend.is_dir(&dir.join("f.txt")).unwrap());
        assert!(!backend.is_dir(&dir.join("missing")).unwrap());
        cleanup(&dir);
    }

    #[test]
    fn test_metadata_file() {
        let dir = tempdir("meta_file");
        let p = dir.join("file.txt");
        fs::write(&p, b"12345").unwrap();
        let backend = LocalFileBackend::new();
        let meta = backend.metadata(&p).unwrap();
        assert!(meta.is_file());
        assert_eq!(meta.size, 5);
        assert!(!meta.is_hidden);
        cleanup(&dir);
    }

    #[test]
    fn test_metadata_hidden() {
        let dir = tempdir("meta_hidden");
        let p = dir.join(".hidden");
        fs::write(&p, b"x").unwrap();
        let backend = LocalFileBackend::new();
        let meta = backend.metadata(&p).unwrap();
        assert!(meta.is_hidden);
        cleanup(&dir);
    }

    #[test]
    fn test_metadata_not_found() {
        let dir = tempdir("meta_nf");
        let backend = LocalFileBackend::new();
        let err = backend.metadata(&dir.join("nope")).unwrap_err();
        assert!(matches!(err, VoeError::FileNotFound { .. }));
        cleanup(&dir);
    }

    #[test]
    fn test_create_dir() {
        let dir = tempdir("create_dir");
        let backend = LocalFileBackend::new();
        backend.create_dir(&dir.join("sub")).unwrap();
        assert!(backend.is_dir(&dir.join("sub")).unwrap());
        cleanup(&dir);
    }

    #[test]
    fn test_create_dir_already_exists_is_error() {
        let dir = tempdir("create_dir_exists");
        fs::create_dir(dir.join("sub")).unwrap();
        let backend = LocalFileBackend::new();
        let err = backend.create_dir(&dir.join("sub")).unwrap_err();
        assert!(matches!(err, VoeError::FileExists { .. }));
        cleanup(&dir);
    }

    #[test]
    fn test_create_dir_all() {
        let dir = tempdir("create_dir_all");
        let backend = LocalFileBackend::new();
        backend.create_dir_all(&dir.join("a/b/c")).unwrap();
        assert!(backend.is_dir(&dir.join("a/b/c")).unwrap());
        cleanup(&dir);
    }

    #[test]
    fn test_remove_dir() {
        let dir = tempdir("remove_dir");
        fs::create_dir(dir.join("sub")).unwrap();
        let backend = LocalFileBackend::new();
        backend.remove_dir(&dir.join("sub")).unwrap();
        assert!(!backend.exists(&dir.join("sub")).unwrap());
        cleanup(&dir);
    }

    #[test]
    fn test_remove_dir_non_empty_fails() {
        let dir = tempdir("remove_dir_ne");
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("sub/x.txt"), b"y").unwrap();
        let backend = LocalFileBackend::new();
        let err = backend.remove_dir(&dir.join("sub")).unwrap_err();
        assert!(matches!(err, VoeError::Storage(_)));
        cleanup(&dir);
    }

    #[test]
    fn test_remove_dir_all() {
        let dir = tempdir("remove_dir_all");
        fs::create_dir_all(dir.join("a/b")).unwrap();
        fs::write(dir.join("a/b/x.txt"), b"y").unwrap();
        let backend = LocalFileBackend::new();
        backend.remove_dir_all(&dir.join("a")).unwrap();
        assert!(!backend.exists(&dir.join("a")).unwrap());
        cleanup(&dir);
    }

    #[test]
    fn test_list_dir() {
        let dir = tempdir("list_dir");
        fs::write(dir.join("a.txt"), b"1").unwrap();
        fs::write(dir.join("b.txt"), b"2").unwrap();
        fs::create_dir(dir.join("sub")).unwrap();
        let backend = LocalFileBackend::new();
        let mut entries = backend.list_dir(&dir).unwrap();
        entries.sort();
        assert_eq!(entries.len(), 3);
        cleanup(&dir);
    }

    #[test]
    fn test_list_dir_not_found() {
        let dir = tempdir("list_dir_nf");
        let backend = LocalFileBackend::new();
        let err = backend.list_dir(&dir.join("missing")).unwrap_err();
        assert!(matches!(err, VoeError::DirNotFound { .. }));
        cleanup(&dir);
    }

    #[test]
    fn test_create_file() {
        let dir = tempdir("create_file");
        let backend = LocalFileBackend::new();
        let p = dir.join("nested/deep/file.txt");
        backend.create_file(&p).unwrap();
        assert!(backend.is_file(&p).unwrap());
        cleanup(&dir);
    }

    #[test]
    fn test_write_and_read_file() {
        let dir = tempdir("write_read");
        let backend = LocalFileBackend::new();
        let p = dir.join("data.bin");
        let data = vec![0u8, 1, 2, 3, 255];
        backend.write_file(&p, &data).unwrap();
        let read = backend.read_file(&p).unwrap();
        assert_eq!(read, data);
        cleanup(&dir);
    }

    #[test]
    fn test_read_file_not_found() {
        let dir = tempdir("read_nf");
        let backend = LocalFileBackend::new();
        let err = backend.read_file(&dir.join("missing")).unwrap_err();
        assert!(matches!(err, VoeError::FileNotFound { .. }));
        cleanup(&dir);
    }

    #[test]
    fn test_read_file_to_string() {
        let dir = tempdir("read_str");
        let backend = LocalFileBackend::new();
        let p = dir.join("s.txt");
        backend.write_file(&p, b"hello world").unwrap();
        let s = backend.read_file_to_string(&p).unwrap();
        assert_eq!(s, "hello world");
        cleanup(&dir);
    }

    #[test]
    fn test_append_to_file() {
        let dir = tempdir("append");
        let backend = LocalFileBackend::new();
        let p = dir.join("a.txt");
        backend.write_file(&p, b"hello ").unwrap();
        backend.append_to_file(&p, b"world").unwrap();
        let content = backend.read_file_to_string(&p).unwrap();
        assert_eq!(content, "hello world");
        cleanup(&dir);
    }

    #[test]
    fn test_delete_file() {
        let dir = tempdir("delete");
        let backend = LocalFileBackend::new();
        let p = dir.join("del.txt");
        backend.write_file(&p, b"bye").unwrap();
        backend.delete_file(&p).unwrap();
        assert!(!backend.exists(&p).unwrap());
    }

    #[test]
    fn test_delete_nonexistent_is_ok() {
        let dir = tempdir("delete_none");
        let backend = LocalFileBackend::new();
        backend.delete_file(&dir.join("nope")).unwrap();
        cleanup(&dir);
    }

    #[test]
    fn test_rename_file() {
        let dir = tempdir("rename");
        let backend = LocalFileBackend::new();
        let from = dir.join("old.txt");
        let to = dir.join("new.txt");
        backend.write_file(&from, b"content").unwrap();
        backend.rename_file(&from, &to).unwrap();
        assert!(!backend.exists(&from).unwrap());
        assert!(backend.exists(&to).unwrap());
        assert_eq!(backend.read_file_to_string(&to).unwrap(), "content");
        cleanup(&dir);
    }

    #[test]
    fn test_rename_creates_parent() {
        let dir = tempdir("rename_parent");
        let backend = LocalFileBackend::new();
        let from = dir.join("a.txt");
        let to = dir.join("nested/deep/a.txt");
        backend.write_file(&from, b"content").unwrap();
        backend.rename_file(&from, &to).unwrap();
        assert!(backend.exists(&to).unwrap());
        cleanup(&dir);
    }

    #[test]
    fn test_move_file() {
        let dir = tempdir("move");
        let backend = LocalFileBackend::new();
        let from = dir.join("src.txt");
        let to = dir.join("dst.txt");
        backend.write_file(&from, b"move me").unwrap();
        backend.move_file(&from, &to).unwrap();
        assert!(!backend.exists(&from).unwrap());
        assert_eq!(backend.read_file_to_string(&to).unwrap(), "move me");
        cleanup(&dir);
    }

    #[test]
    fn test_copy_file() {
        let dir = tempdir("copy");
        let backend = LocalFileBackend::new();
        let from = dir.join("orig.txt");
        let to = dir.join("copy.txt");
        backend.write_file(&from, b"copy me").unwrap();
        backend.copy_file(&from, &to).unwrap();
        assert!(backend.exists(&from).unwrap());
        assert!(backend.exists(&to).unwrap());
        assert_eq!(backend.read_file_to_string(&to).unwrap(), "copy me");
        cleanup(&dir);
    }

    #[test]
    fn test_copy_dir_all() {
        let dir = tempdir("copy_dir");
        let backend = LocalFileBackend::new();
        let src = dir.join("src");
        let dst = dir.join("dst");
        backend.create_dir_all(&src).unwrap();
        backend.write_file(&src.join("a.txt"), b"aaa").unwrap();
        backend.create_dir_all(&src.join("sub")).unwrap();
        backend.write_file(&src.join("sub/b.txt"), b"bbb").unwrap();
        backend.copy_dir_all(&src, &dst).unwrap();
        assert_eq!(
            backend.read_file_to_string(&dst.join("a.txt")).unwrap(),
            "aaa"
        );
        assert_eq!(
            backend.read_file_to_string(&dst.join("sub/b.txt")).unwrap(),
            "bbb"
        );
        cleanup(&dir);
    }

    #[test]
    fn test_ensure_parent_dir() {
        let dir = tempdir("ensure_parent");
        let backend = LocalFileBackend::new();
        let p = dir.join("deep/nested/file.txt");
        backend.ensure_parent_dir(&p).unwrap();
        assert!(backend.is_dir(&dir.join("deep/nested")).unwrap());
        cleanup(&dir);
    }

    #[test]
    fn test_map_io_err_not_found_file() {
        let dir = tempdir("map_nf");
        let p = dir.join("missing.txt");
        let err = fs::read(&p).unwrap_err();
        let mapped = LocalFileBackend::map_io_err(&p, err);
        assert!(matches!(mapped, VoeError::FileNotFound { .. }));
        cleanup(&dir);
    }

    #[test]
    fn test_map_dir_io_err_not_found() {
        let dir = tempdir("map_nf_dir");
        let p = dir.join("missing_dir");
        let err = fs::read_dir(&p).unwrap_err();
        let mapped = LocalFileBackend::map_dir_io_err(&p, err);
        assert!(matches!(mapped, VoeError::DirNotFound { .. }));
        cleanup(&dir);
    }

    #[test]
    fn test_map_io_err_already_exists() {
        let dir = tempdir("map_exists");
        fs::create_dir(dir.join("exists")).unwrap();
        let p = dir.join("exists");
        let err = fs::create_dir(&p).unwrap_err();
        let mapped = LocalFileBackend::map_io_err(&p, err);
        assert!(matches!(mapped, VoeError::FileExists { .. }));
        cleanup(&dir);
    }

    // ----------- Performance benchmarks (zero dependencies, std::time only) -----------

    const BENCH_ITER: usize = 500;

    fn bench_read_throughput<F: Fn(&Path) -> std::result::Result<Vec<u8>, VoeError>>(
        label: &str,
        path: &Path,
        reader: F,
    ) -> u128 {
        let start = std::time::Instant::now();
        let mut total = 0usize;
        for _ in 0..BENCH_ITER {
            let data = reader(path).unwrap();
            total += data.len();
        }
        let elapsed = start.elapsed();
        let avg_us = elapsed.as_micros() / BENCH_ITER as u128;
        println!(
            "[bench] {}: {} iters, {} bytes, avg {} µs/op, total {} ms",
            label,
            BENCH_ITER,
            total,
            avg_us,
            elapsed.as_millis(),
        );
        avg_us
    }

    #[test]
    fn bench_local_vs_cached_repeated_reads() {
        use crate::backend::CachedStorageBackend;

        let dir = tempdir("bench_read");
        let p = dir.join("bench.bin");
        let data: Vec<u8> = (0..4096u32).map(|i| i as u8).collect();

        let raw = LocalFileBackend::new();
        raw.write_file(&p, &data).unwrap();

        let cached = CachedStorageBackend::new(Box::new(LocalFileBackend::new()));
        cached.write_file(&p, &data).unwrap();

        let raw_avg = bench_read_throughput("LocalFileBackend  read_file", &p, |pp| {
            LocalFileBackend::new().read_file(pp)
        });
        let cached_avg = bench_read_throughput("CachedStorageBackend read_file (warm)", &p, |pp| {
            cached.read_file(pp)
        });

        let speedup = raw_avg as f64 / cached_avg.max(1) as f64;
        println!(
            "[bench] speedup: CachedStorageBackend is {:.1}x faster than LocalFileBackend",
            speedup
        );
        cleanup(&dir);
    }

    #[test]
    fn bench_metadata_throughput() {
        use crate::backend::CachedStorageBackend;

        let dir = tempdir("bench_meta");
        let p = dir.join("meta.bin");
        fs::write(&p, b"xyz").unwrap();

        let raw_avg = {
            let start = std::time::Instant::now();
            for _ in 0..BENCH_ITER {
                let _ = LocalFileBackend::new().metadata(&p).unwrap();
            }
            start.elapsed().as_micros() / BENCH_ITER as u128
        };

        let cached = CachedStorageBackend::new(Box::new(LocalFileBackend::new()));
        cached.metadata(&p).unwrap();
        let cached_avg = {
            let start = std::time::Instant::now();
            for _ in 0..BENCH_ITER {
                let _ = cached.metadata(&p).unwrap();
            }
            start.elapsed().as_micros() / BENCH_ITER as u128
        };

        println!(
            "[bench] metadata: Local {}µs vs Cached(warm) {}µs",
            raw_avg, cached_avg
        );
        cleanup(&dir);
    }

    #[test]
    fn bench_write_throughput() {
        let dir = tempdir("bench_write");
        let data: Vec<u8> = (0..8192u32).map(|i| i as u8).collect();
        let p = dir.join("w.bin");

        let start = std::time::Instant::now();
        for _ in 0..200 {
            LocalFileBackend::new().write_file(&p, &data).unwrap();
        }
        let elapsed_ms = start.elapsed().as_millis();
        println!(
            "[bench] LocalFileBackend write_file x200: {} ms, {} µs/op",
            elapsed_ms,
            elapsed_ms * 1000 / 200,
        );
        cleanup(&dir);
    }
}
