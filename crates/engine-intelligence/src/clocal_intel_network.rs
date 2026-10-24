//! Local intelligence network embedded in the country intel. Its graph root pointer uses a separate
//! graph layout.

/// Local-intelligence network primary vtable.
pub const VTABLE_RVA: usize = 0x29a_0408;
/// Local-intelligence network constructor initializes its graph.
pub const CONSTRUCTOR_RVA: usize = 0x11b_c870;
/// CLocalIntelNetwork's pointer to the unnamed 0x50-byte graph.
pub const GRAPH: usize = 0x08;
/// Array header of one quiet byte per subnetwork, indexed by subnetwork
/// selector (saved as the local `sub_network_is_quiet` list). A selector
/// outside the list is not quiet.
pub const QUIET_DATA: usize = 0x18;
