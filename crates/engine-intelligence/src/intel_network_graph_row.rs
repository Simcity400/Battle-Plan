//! Row stored in the intel-network graph's row range; its C++ type name is unknown.

/// Graph-row byte stride.
pub const STRIDE: usize = 0x50;
/// Signed subnetwork selector; negative values skip strength.
pub const SUBNETWORK_SELECTOR: usize = 0x18;
/// Stored state strength without a positivity filter; serialized in the
/// parallel strength list.
pub const STATE_STRENGTH: usize = 0x28;
/// The state's daily gain from the network's operatives, i64; saved as the
/// row's `gain_breakdown` `operatives`.
pub const GAIN_FROM_OPERATIVES: usize = 0x30;
/// The state's daily gain from adjacent states, i64; saved as the row's
/// `gain_breakdown` `adjacencies`.
pub const GAIN_FROM_ADJACENCY: usize = 0x38;
/// The state's daily gain, i64; saved in the parallel `gain` list.
pub const GAIN: usize = 0x40;
/// The state id, i32. The map mode's network layer reads the state from the
/// row, so the reader checks it against the vertex that indexes the row.
pub const STATE_ID: usize = 0x48;
