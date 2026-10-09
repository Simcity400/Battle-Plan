//! COperativeMission, inline at COperativeLeader+0x1088 (140C18FF0).

/// Polymorphic implementation pointer; GetCountryTag@140FAE550 reads +10.
pub const IMPLEMENTATION: usize = 0x10;
/// Variant u32; Write@140FB0F90 and dispatcher 140FAFAA0 switch on +18.
pub const VARIANT: usize = 0x18;
/// Operative action build-network discriminator; old build_profile.rs:504 and A30 mission_spec agree on 1.
pub const BUILD_NETWORK: u32 = 1;
/// Operative action quiet-network discriminator; old build_profile.rs:505 and A30 mission_spec agree on 2.
pub const QUIET_NETWORK: u32 = 2;
/// Operative action counter-intelligence discriminator; old a30_non_operation_action_identity.rs:96-101 mission_spec.
pub const COUNTER_INTELLIGENCE: u32 = 3;
/// Operative action root-out-resistance discriminator; old a30_non_operation_action_identity.rs mission_spec maps 4.
pub const ROOT_OUT_RESISTANCE: u32 = 4;
/// Operative action boost-ideology discriminator; old a30_non_operation_action_identity.rs mission_spec maps 5.
pub const BOOST_IDEOLOGY: u32 = 5;
/// Operative action control-trade discriminator; old a30_non_operation_action_identity.rs mission_spec maps 6.
pub const CONTROL_TRADE: u32 = 6;
/// Operative action diplomatic-pressure discriminator; old a30_non_operation_action_identity.rs mission_spec maps 7.
pub const DIPLOMATIC_PRESSURE: u32 = 7;
/// Operative action propaganda discriminator; old a30_non_operation_action_identity.rs mission_spec maps 8.
pub const PROPAGANDA: u32 = 8;
/// COperativeMission::Write@140FB0F90 selects these concrete definition
/// tokens by variant; spelling comes from the recorded LexicalTokens.
pub const SAVE_TOKENS: [u32; 8] = [
    0x3d1a, 0x4b20, 0x3d1b, 0x4b21, 0x4b1c, 0x4b1d, 0x4b1e, 0x4b1f,
];
