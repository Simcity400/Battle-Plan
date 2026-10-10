//! Runtime GUI registry interface reached from the idler; exact C++ class unproven.

/// Window-lookup virtual byte offset; receives a name descriptor and byte argument, and returns a pointer.
pub const WINDOW_LOOKUP_VSLOT: usize = 0x60;
