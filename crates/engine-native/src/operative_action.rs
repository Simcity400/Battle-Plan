
pub const IMPLEMENTATION: usize = 0x10;
pub const VARIANT: usize = 0x18;
pub const BUILD_NETWORK: u32 = 1;
pub const QUIET_NETWORK: u32 = 2;
pub const COUNTER_INTELLIGENCE: u32 = 3;
pub const ROOT_OUT_RESISTANCE: u32 = 4;
pub const BOOST_IDEOLOGY: u32 = 5;
pub const CONTROL_TRADE: u32 = 6;
pub const DIPLOMATIC_PRESSURE: u32 = 7;
pub const PROPAGANDA: u32 = 8;
pub const fn targets_state(variant: u32) -> bool {
    matches!(
        variant,
        BUILD_NETWORK | QUIET_NETWORK | ROOT_OUT_RESISTANCE | BOOST_IDEOLOGY | PROPAGANDA
    )
}
pub const PENDING_VARIANT: usize = 0x08;
pub const PENDING_TARGET_COUNTRY: usize = 0x0c;
pub const PENDING_TARGET_STATE: usize = 0x10;
pub const PENDING_IDEOLOGY: usize = 0x18;
pub const SAVE_TOKENS: [u32; 8] = [
    0x3d1a, 0x4b20, 0x3d1b, 0x4b21, 0x4b1c, 0x4b1d, 0x4b1e, 0x4b1f,
];
