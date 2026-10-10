//! Event-option command actor and copied event scope. Pending-event
//! matching selects the scope country independently of the constructor actor.

/// Actor country reference, u32.
pub const ACTOR_TAG: usize = 0x34;
/// Inline copied event scope.
pub const SCOPE: usize = 0x38;
/// Event-option command constructor.
pub const CONSTRUCT_RVA: usize = 0x0152_7b30;
/// Constructor entry sentinel bytes.
pub const CONSTRUCT_SENTINEL: [u8; 17] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x48, 0x83, 0xec,
    0x30,
];
