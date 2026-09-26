use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use voe_types::error::{Result, VoeError};
use voe_types::object::ObjectId;

use super::types::{ContextRequirement, Mask, MaskContent};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaskConflict {
    pub mask_a: ObjectId,
    pub mask_b: ObjectId,
    pub reason: String,
}

impl fmt::Display for MaskConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Conflict between {} and {}: {}",
            self.mask_a, self.mask_b, self.reason
        )
    }
}

pub trait MaskResolver: Send + Sync {
    fn resolve_order<'a>(&self, masks: &[&'a dyn Mask]) -> Result<Vec<&'a dyn Mask>>;
    fn validate_conflicts(&self, masks: &[&dyn Mask]) -> Result<Vec<MaskConflict>>;
    fn restack_after_removal<'a>(
        &self,
        masks: &[&'a dyn Mask],
        removed_id: &ObjectId,
    ) -> Result<Vec<&'a dyn Mask>>;
    fn restack_after_replace<'a>(
        &self,
        masks: &[&'a dyn Mask],
        old_id: &ObjectId,
        replacement: &'a dyn Mask,
    ) -> Result<Vec<&'a dyn Mask>>;
    fn apply_all(&self, ordered: &[&dyn Mask], state: &mut HashMap<PathBuf, Vec<u8>>)
        -> Result<()>;
    fn check_context(
        &self,
        mask: &dyn Mask,
        base: &HashMap<PathBuf, Vec<u8>>,
        applied_ids: &[ObjectId],
    ) -> Result<bool>;
}

pub struct SimpleMaskResolver;

impl SimpleMaskResolver {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SimpleMaskResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl MaskResolver for SimpleMaskResolver {
    fn resolve_order<'a>(&self, masks: &[&'a dyn Mask]) -> Result<Vec<&'a dyn Mask>> {
        let mut ordered: Vec<&'a dyn Mask> = Vec::new();
        let mut remaining: Vec<&'a dyn Mask> = masks.to_vec();
        let mut resolved_ids: Vec<ObjectId> = Vec::new();

        while !remaining.is_empty() {
            let mut progress = false;
            let mut selected: Vec<usize> = Vec::new();
            for (i, mask) in remaining.iter().enumerate() {
                let deps = &mask.metadata().dependencies;
                let all_deps_met = deps.ids.iter().all(|d| resolved_ids.contains(d));
                let no_conflicts = !mask
                    .metadata()
                    .exclusions
                    .iter()
                    .any(|e| resolved_ids.contains(e));

                if all_deps_met && no_conflicts {
                    resolved_ids.push(mask.id().clone());
                    ordered.push(remaining[i]);
                    selected.push(i);
                    progress = true;
                }
            }
            for idx in selected.into_iter().rev() {
                remaining.remove(idx);
            }
            if !progress {
                return Err(VoeError::Other(
                    "Unresolvable mask dependency cycle".to_string(),
                ));
            }
        }

        Ok(ordered)
    }

    fn validate_conflicts(&self, masks: &[&dyn Mask]) -> Result<Vec<MaskConflict>> {
        let mut conflicts: Vec<MaskConflict> = Vec::new();
        let ids: Vec<ObjectId> = masks.iter().map(|m| m.id().clone()).collect();

        let mut seen_exclusions: Vec<(ObjectId, ObjectId)> = Vec::new();
        for a in masks {
            for exc in &a.metadata().exclusions {
                if ids.contains(exc) {
                    let key = (a.id().clone(), exc.clone());
                    let rev = (exc.clone(), a.id().clone());
                    if !seen_exclusions.contains(&key) && !seen_exclusions.contains(&rev) {
                        seen_exclusions.push(key);
                        conflicts.push(MaskConflict {
                            mask_a: a.id().clone(),
                            mask_b: exc.clone(),
                            reason: "declared mutual exclusion".to_string(),
                        });
                    }
                }
            }
        }

        // For every pair of masks, check all location pairs across their
        // changes for overlapping Full-content targets — two masks writing
        // a full file to the same path is a hard conflict.
        let mut seen_paths: Vec<(ObjectId, ObjectId)> = Vec::new();
        for i in 0..masks.len() {
            for j in (i + 1)..masks.len() {
                let a = masks[i];
                let b = masks[j];
                for ca in a.changes() {
                    let a_path = ca.location.path();
                    for cb in b.changes() {
                        let b_path = cb.location.path();
                        if a_path == b_path {
                            if let (MaskContent::Full(_), MaskContent::Full(_)) =
                                (&ca.content, &cb.content)
                            {
                                let key = (a.id().clone(), b.id().clone());
                                if !seen_paths.contains(&key) {
                                    seen_paths.push(key);
                                    conflicts.push(MaskConflict {
                                        mask_a: a.id().clone(),
                                        mask_b: b.id().clone(),
                                        reason: "two full-content changes targeting same path"
                                            .to_string(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(conflicts)
    }

    fn restack_after_removal<'a>(
        &self,
        masks: &[&'a dyn Mask],
        removed_id: &ObjectId,
    ) -> Result<Vec<&'a dyn Mask>> {
        let mut remaining: Vec<&'a dyn Mask> = masks
            .iter()
            .filter(|m| m.id() != removed_id)
            .copied()
            .collect();

        let mut remaining_ids: Vec<ObjectId> = remaining.iter().map(|m| m.id().clone()).collect();

        let mut changed = true;
        while changed {
            changed = false;
            let mut to_remove: Vec<usize> = Vec::new();

            for (i, mask) in remaining.iter().enumerate() {
                let deps = &mask.metadata().dependencies.ids;
                if deps.iter().any(|d| !remaining_ids.contains(d)) {
                    to_remove.push(i);
                }
            }

            if !to_remove.is_empty() {
                for idx in to_remove.iter().rev() {
                    remaining.remove(*idx);
                }
                let new_ids: Vec<ObjectId> = remaining.iter().map(|m| m.id().clone()).collect();
                remaining_ids.clear();
                remaining_ids.extend(new_ids);
                changed = true;
            }
        }

        self.resolve_order(&remaining)
    }

    fn restack_after_replace<'a>(
        &self,
        masks: &[&'a dyn Mask],
        old_id: &ObjectId,
        replacement: &'a dyn Mask,
    ) -> Result<Vec<&'a dyn Mask>> {
        let mut result: Vec<&'a dyn Mask> =
            masks.iter().filter(|m| m.id() != old_id).copied().collect();
        result.push(replacement);
        self.resolve_order(&result)
    }

    fn apply_all(
        &self,
        ordered: &[&dyn Mask],
        state: &mut HashMap<PathBuf, Vec<u8>>,
    ) -> Result<()> {
        for mask in ordered {
            mask.apply(state)?;
        }
        Ok(())
    }

    fn check_context(
        &self,
        mask: &dyn Mask,
        base: &HashMap<PathBuf, Vec<u8>>,
        applied_ids: &[ObjectId],
    ) -> Result<bool> {
        for req in &mask.metadata().context_requirements {
            match req {
                ContextRequirement::PathExists(path) => {
                    if !base.contains_key(path) {
                        return Ok(false);
                    }
                }
                ContextRequirement::PriorMask(id) => {
                    if !applied_ids.contains(id) {
                        return Ok(false);
                    }
                }
                ContextRequirement::AbsentMask(id) => {
                    if applied_ids.contains(id) {
                        return Ok(false);
                    }
                }
                ContextRequirement::PathContains(path, data) => match base.get(path) {
                    Some(content) if content.windows(data.len()).any(|w| w == data) => {}
                    _ => return Ok(false),
                },
                ContextRequirement::Custom(_) => {}
            }
        }
        Ok(true)
    }
}
