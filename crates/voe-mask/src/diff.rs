//! Compute the delta between two snapshots as a list of [`ChunkMask`]es.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::{ChunkMask, MaskContent};
use voe_types::object::ObjectId;

/// Build a list of [`ChunkMask`]es describing every path whose content
/// differs between `parent` and `working`, plus a deletion mask for
/// every path that existed in `parent` but not in `working`.
///
/// Both maps key snapshots by absolute-style paths (typically `/foo/bar`).
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
