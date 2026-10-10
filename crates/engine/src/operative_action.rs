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
/// Definition tokens selected by action variant; spellings come from recorded LexicalTokens.
pub const SAVE_TOKENS: [u32; 8] = [
    0x3d1a, 0x4b20, 0x3d1b, 0x4b21, 0x4b1c, 0x4b1d, 0x4b1e, 0x4b1f,
];
