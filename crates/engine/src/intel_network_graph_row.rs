//! Row stored between intel-network graph +0x10 and +0x18; its C++ type name is unknown.

/// Intel-network graph-row byte stride, confirmed by old read_intel_network_graph_rows (extractor.rs:8264-8273).
pub const STRIDE: usize = 0x50;
/// Intel-network graph row +0x18, native i32 subnetwork selector; confirmed by old extractor.rs:8369-8378.
pub const SUBNETWORK_SELECTOR: usize = 0x18;
