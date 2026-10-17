//! Operative mission embedded in its leader.

/// Polymorphic implementation pointer.
pub const IMPLEMENTATION: usize = 0x10;
/// Mission u32 variant used by serialization and dispatch.
pub const VARIANT: usize = 0x18;
/// Build-network action discriminator.
pub const BUILD_NETWORK: u32 = 1;
/// Quiet-network action discriminator.
pub const QUIET_NETWORK: u32 = 2;
/// Counter-intelligence action discriminator.
pub const COUNTER_INTELLIGENCE: u32 = 3;
/// Root-out-resistance action discriminator.
pub const ROOT_OUT_RESISTANCE: u32 = 4;
/// Boost-ideology action discriminator.
pub const BOOST_IDEOLOGY: u32 = 5;
/// Control-trade action discriminator.
pub const CONTROL_TRADE: u32 = 6;
/// Diplomatic-pressure action discriminator.
pub const DIPLOMATIC_PRESSURE: u32 = 7;
/// Propaganda action discriminator.
pub const PROPAGANDA: u32 = 8;
/// Whether the variant's implementation targets a state as well as a country.
pub const fn targets_state(variant: u32) -> bool {
    matches!(
        variant,
        BUILD_NETWORK | QUIET_NETWORK | ROOT_OUT_RESISTANCE | BOOST_IDEOLOGY | PROPAGANDA
    )
}
/// Pending mission's u32 variant, relative to the leader's pending mission.
pub const PENDING_VARIANT: usize = 0x08;
/// Pending mission's raw target country reference, u32.
pub const PENDING_TARGET_COUNTRY: usize = 0x0c;
/// Pending mission's target state pointer; null for none.
pub const PENDING_TARGET_STATE: usize = 0x10;
/// Pending mission's ideology pointer; null for none.
pub const PENDING_IDEOLOGY: usize = 0x18;
/// Definition tokens selected by action variant; spellings come from recorded LexicalTokens.
pub const SAVE_TOKENS: [u32; 8] = [
    0x3d1a, 0x4b20, 0x3d1b, 0x4b21, 0x4b1c, 0x4b1d, 0x4b1e, 0x4b1f,
];
