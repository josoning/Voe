use std::collections::HashMap;
use std::path::{Path, PathBuf};

use voe_mask::{ChunkMask, MaskContent};
use voe_storage_api::StorageBackend;
use voe_types::error::{Result, VoeError};
use voe_types::object::ObjectId;

use crate::backend::LocalFileBackend;

pub fn read_working_tree(root: &Path) -> Result<HashMap<PathBuf, Vec<u8>>> {
    let backend = LocalFileBackend::new();
    read_working_tree_with_backend(root, &backend)
}

pub fn read_working_tree_with_backend(
    root: &Path,
    backend: &dyn StorageBackend,
) -> Result<HashMap<PathBuf, Vec<u8>>> {
    let mut result: HashMap<PathBuf, Vec<u8>> = HashMap::new();
    collect_files(backend, root, root, &mut result)?;
    Ok(result)
}

fn collect_files(
    backend: &dyn StorageBackend,
    root: &Path,
    current: &Path,
    out: &mut HashMap<PathBuf, Vec<u8>>,
) -> Result<()> {
    let entries = backend.list_dir(current)?;
    for entry_path in entries {
        let meta = match backend.metadata(&entry_path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let name = entry_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        if meta.is_dir() {
            if name == ".voe" || name.starts_with('.') {
                continue;
            }
            collect_files(backend, root, &entry_path, out)?;
        } else if meta.is_file() {
            let rel = entry_path.strip_prefix(root).map_err(|e| {
                VoeError::Storage(format!(
                    "Failed to strip prefix {} from {}: {}",
                    root.display(),
                    entry_path.display(),
                    e
                ))
            })?;
            let data = backend.read_file(&entry_path)?;
            let key = PathBuf::from("/").join(rel);
            out.insert(key, data);
        }
    }
    Ok(())
}

pub fn diff_to_masks(
    parent: &HashMap<PathBuf, Vec<u8>>,
    working: &HashMap<PathBuf, Vec<u8>>,
) -> Vec<ChunkMask> {
    let mut masks: Vec<ChunkMask> = Vec::new();

    for (path, data) in working {
        match parent.get(path) {
            Some(parent_data) if parent_data == data => {}
            _ => {
                let id_source = format!("new:{}:{}", path.display(), ObjectId::from_bytes(data));
                let mask = ChunkMask::file(
                    ObjectId::new(id_source),
                    path.clone(),
                    MaskContent::full(data.clone()),
                );
                masks.push(mask);
            }
        }
    }

    for path in parent.keys() {
        if !working.contains_key(path) {
            let id_source = format!("del:{}", path.display());
            let mask = ChunkMask::deleted(ObjectId::new(id_source), path.clone());
            masks.push(mask);
        }
    }

    masks
}

pub fn write_snapshot_to_disk(root: &Path, snapshot: &HashMap<PathBuf, Vec<u8>>) -> Result<()> {
    let backend = LocalFileBackend::new();
    write_snapshot_to_disk_with_backend(root, snapshot, &backend)
}

pub fn write_snapshot_to_disk_with_backend(
    root: &Path,
    snapshot: &HashMap<PathBuf, Vec<u8>>,
    backend: &dyn StorageBackend,
) -> Result<()> {
    let current_files = collect_current_visible_files(backend, root)?;

    let mut to_delete: Vec<PathBuf> = Vec::new();
    for disk_path in &current_files {
        let disk_rel = to_snapshot_key(root, disk_path);
        if !snapshot.contains_key(&disk_rel) {
            to_delete.push(disk_path.clone());
        }
    }

    for disk_path in to_delete.iter() {
        backend.delete_file(disk_path).map_err(|e| {
            VoeError::Storage(format!("Failed to delete {}: {}", disk_path.display(), e))
        })?;
    }

    for disk_path in to_delete.iter() {
        let mut cursor = disk_path.parent();
        while let Some(dir) = cursor {
            if dir == root || dir == root.join(".voe") {
                break;
            }
            let entries = match backend.list_dir(dir) {
                Ok(e) => e,
                Err(_) => break,
            };
            let is_empty = entries.is_empty();
            if is_empty {
                let _ = backend.remove_dir(dir);
                cursor = dir.parent();
            } else {
                break;
            }
        }
    }

    for (key, data) in snapshot {
        let relative = key.to_string_lossy().trim_start_matches('/').to_string();
        let disk_path = root.join(&relative);

        backend.ensure_parent_dir(&disk_path)?;

        let needs_write = match backend.read_file(&disk_path) {
            Ok(existing) => existing != *data,
            Err(_) => true,
        };
        if needs_write {
            backend.write_file(&disk_path, data).map_err(|e| {
                VoeError::Storage(format!("Failed to write {}: {}", disk_path.display(), e))
            })?;
        }
    }

    Ok(())
}

fn collect_current_visible_files(
    backend: &dyn StorageBackend,
    root: &Path,
) -> Result<Vec<PathBuf>> {
    let mut result: Vec<PathBuf> = Vec::new();
    collect_visible(backend, root, root, &mut result)?;
    Ok(result)
}

fn collect_visible(
    backend: &dyn StorageBackend,
    root: &Path,
    current: &Path,
    out: &mut Vec<PathBuf>,
) -> Result<()> {
    let entries = match backend.list_dir(current) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry_path in entries {
        let meta = match backend.metadata(&entry_path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let name = entry_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        if entry_path == root.join(".voe") {
            continue;
        }
        if name.starts_with('.') {
            continue;
        }

        if meta.is_dir() {
            collect_visible(backend, root, &entry_path, out)?;
        } else if meta.is_file() {
            out.push(entry_path);
        }
    }
    Ok(())
}

fn to_snapshot_key(root: &Path, disk_path: &Path) -> PathBuf {
    let rel = disk_path.strip_prefix(root).unwrap_or(disk_path);
    PathBuf::from("/").join(rel)
}
