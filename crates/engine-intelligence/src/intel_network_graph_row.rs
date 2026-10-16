//! Row stored in the intel-network graph's row range; its C++ type name is unknown.

/// Graph-row byte stride.
pub const STRIDE: usize = 0x50;
/// Signed subnetwork selector; negative values skip strength.
pub const SUBNETWORK_SELECTOR: usize = 0x18;
/// Stored state strength without a positivity filter; serialized in the
/// parallel strength list.
pub const STATE_STRENGTH: usize = 0x28;
