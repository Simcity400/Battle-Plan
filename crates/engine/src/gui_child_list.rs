//! Engine child-list header returned by the overlap list interface; concrete template name is not retained.

/// GUI child-list first node pointer +0; old dynamic_collection_trace_live/src/a729_named_overlap_hook.rs:690 starts forward traversal here.
pub const HEAD: usize = 0;
/// GUI child-list last node pointer +8; old a729_named_overlap_hook.rs:690 starts reverse traversal here.
pub const TAIL: usize = 0x08;
/// GUI child-list count u32 +0x10; old a729_named_overlap_hook.rs:683 bounds the subsequent linked-list traversal.
pub const COUNT: usize = 0x10;
