pub mod chunk_mask;
pub mod diff;
pub mod resolver;
pub mod types;

pub use chunk_mask::ChunkMask;
pub use diff::diff_to_masks;
pub use resolver::{MaskConflict, MaskResolver, SimpleMaskResolver};
pub use types::{
    ChunkRef, ContextRequirement, DependencyList, Mask, MaskChange, MaskContent, MaskKind,
    MaskLocation, MaskMetadata,
};
