#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod content;
pub mod kept;
pub mod parallel;
pub mod script_files;
pub mod textures;
#[cfg(any(test, feature = "layer-fixtures"))]
pub mod test_support;

pub use content_reader_parser::fields::{invalid as content_error, is_content_error};
pub use content::{
    ChangedLayer, GatherOptions, GatheredContent, HashStats, LayerRole, ResolveOptions,
    ResolvedContent, ResolvedLayer, ResolvedLayers, gather, gather_cancellable, playback, resolve,
    resolve_cancellable,
};
