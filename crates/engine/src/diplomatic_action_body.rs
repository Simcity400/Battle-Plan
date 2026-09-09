//! The action body pointed to by CIncomingDiplomaticAction+0x18.
//! Source calls it pDiplomaticAction; concrete RTTI identity is not established.

/// Diplomatic action body type i32; build_profile.rs:566 and D53 read_incoming_action_row.
pub const ACTION_TYPE: usize = 0x10;
/// Diplomatic action body actor CCountryTag u32; build_profile.rs:567 and D53 incoming action reader.
pub const ACTOR: usize = 0x14;
/// Diplomatic action body original actor CCountryTag u32; build_profile.rs:568 and D53 incoming action reader.
pub const ORIGINAL_ACTOR: usize = 0x18;
/// Diplomatic action body recipient CCountryTag u32; build_profile.rs:569 and D53 incoming action reader.
pub const RECIPIENT: usize = 0x1c;
/// Diplomatic action body original recipient CCountryTag u32; build_profile.rs:570 and D53 incoming action reader.
pub const ORIGINAL_RECIPIENT: usize = 0x20;
/// Diplomatic action body raw date word read u32; build_profile.rs:571 and D53 incoming action reader, native signedness unproven.
pub const HOUR: usize = 0x30;
/// Diplomatic action body last-command date word read u32; build_profile.rs:572 and D53 incoming action reader.
pub const LAST_COMMAND_HOUR: usize = 0x48;
