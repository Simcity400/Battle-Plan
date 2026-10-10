//! State operands shared by authenticated concrete peace-action classes.

/// Signed state ID, serialized and consumed by the settlement.
pub const STATE: usize = 0x48;
/// Four stored bytes in order: demilitarization, reparations, resource rights
/// and industry dismantling. Enabled bytes are serialized and grouped together.
pub const FLAGS: usize = 0x4c;
