//! Unnamed source-owner argument of the technology badge controller.
//!
//! The old hook reads raw specialization tokens from this object. Its C++
//! identity is unproved; do not alias it to a research-state struct by offset.

/// Technology badge source u32-token array pointer; archived dynamic_collection_trace_live/src/a729_technology_badge_grid_hook.rs:35,547-576.
pub const SPECIALIZATION_DATA: usize = 0x140;
/// Technology badge source u32-token count; archived dynamic_collection_trace_live/src/a729_technology_badge_grid_hook.rs:36,554.
pub const SPECIALIZATION_COUNT: usize = 0x14c;
/// Technology badge source token width in bytes; archived dynamic_collection_trace_live/src/a729_technology_badge_grid_hook.rs:568-576 reads LE u32 values.
pub const SPECIALIZATION_STRIDE: usize = 4;
