//! Bucket specialization of CCharacterManager's robin-hood name index; offsets use the bucket base.

/// Name-index bucket stride.
pub const STRIDE: usize = 0x18;
/// Name-index bucket probe-control byte.
pub const CONTROL: usize = 0x04;
/// Name-index bucket script-name key i32 bits.
pub const KEY: usize = 0x08;
/// Name-index bucket CCharacter* value.
pub const CHARACTER: usize = 0x10;
