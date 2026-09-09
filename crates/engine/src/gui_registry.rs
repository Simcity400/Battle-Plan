//! Runtime GUI registry interface reached from the idler; exact C++ class unproven.

/// GUI registry window lookup byte slot; dynamic_collection_trace_live/src/a729_headless_inactive.rs invokes +0x60 with (this, MSVC name string, u8) and receives a pointer.
pub const WINDOW_LOOKUP_VSLOT: usize = 0x60;
