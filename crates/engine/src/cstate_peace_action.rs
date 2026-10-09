//! State operands shared by authenticated concrete peace-action classes.

/// Signed state ID; 14150DDB0 serializes it and 140E3D170 consumes it.
pub const STATE: usize = 0x48;
/// Four stored bytes in order: demilitarization, reparations, resource rights,
/// dismantle industry. 14150DDB0 writes each enabled byte; 140E240A0 groups them.
pub const FLAGS: usize = 0x4c;
