//! Copied slot record within CEquipmentType; native nominal record class is unidentified.

pub const STRIDE: usize = 0x50;
/// Copied-slot LexerToken u32; archived slot_identity.rs reads row+8 and resolves CPdxLexer strings.
pub const TOKEN: usize = 0x08;
/// Unassigned-slot token retained in the complete census; archived slot_identity.rs SENTINEL_TOKEN and native slot law.
pub const UNASSIGNED_TOKEN: u32 = 0x165;
