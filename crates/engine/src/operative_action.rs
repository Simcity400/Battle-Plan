//! Inline action value at COperativeLeader+0x1088; concrete C++ class name is unproved.

/// Operative action implementation/wrapper pointer; old build_profile.rs:501 and a30_non_operation_action_identity.rs:19 read action+10.
pub const IMPLEMENTATION: usize = 0x10;
/// Operative action variant u32; old build_profile.rs:502 and a30_non_operation_action_identity.rs:20 read action+18.
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
