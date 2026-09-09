//! Engine linked-list node used by list and overlapping-element widgets; concrete template name is not retained.

/// GUI child-list node item pointer +0; old dynamic_collection_trace_live/src/standard_list.rs:20 and named-overlap capture read it.
pub const ITEM: usize = 0;
/// GUI child-list previous node pointer +8; old a729_named_overlap_hook.rs:714 walks it when the overlap is reversed.
pub const PREVIOUS: usize = 0x08;
/// GUI child-list next node pointer +0x10; old standard_list.rs:21 and named-overlap capture read it.
pub const NEXT: usize = 0x10;
