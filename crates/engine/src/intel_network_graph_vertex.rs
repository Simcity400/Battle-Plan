//! Vertex stored in intel-network graph +0x38; its C++ type name is unknown.

/// Intel-network graph vertex byte stride, confirmed by old read_intel_network_graph_rows (extractor.rs:8332-8337).
pub const STRIDE: usize = 0x10;
/// Intel-network graph vertex +0x00, native i32 state id; confirmed by old extractor.rs:8338-8347.
pub const STATE_ID: usize = 0x00;
/// Intel-network graph vertex +0x08, native u64 graph-row index; confirmed by old extractor.rs:8351-8367.
pub const ROW_INDEX: usize = 0x08;
