//! Vertex-buffer wrapper reached from a railway arrow; exact C++ class name is unconfirmed.

/// Arrow wrapper's four native vertex-buffer pointer fields, read by rail_supply_arrow.rs:26,562-563.
pub const VERTEX_BUFFERS: [usize; 4] = [0x48, 0xc8, 0x148, 0x1c8];
