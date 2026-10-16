//! Local intelligence network embedded at country-intel +0x10.
//! Its graph root pointer at +8 uses a separate graph layout.

/// Local-intelligence network primary vtable.
pub const VTABLE_RVA: usize = 0x298_94f8;
/// Local-intelligence network constructor initializes its graph.
pub const CONSTRUCTOR_RVA: usize = 0x11a_7810;
/// CLocalIntelNetwork +0x08, pointer to the unnamed 0x50-byte graph.
pub const GRAPH: usize = 0x08;
/// Local-intelligence node lookup searches by state id, then reads its graph row.
pub const READ_NODE_DESCRIPTOR_RVA: usize = 0x11a_cdd0;
