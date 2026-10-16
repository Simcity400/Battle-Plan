//! The action body pointed to by CIncomingDiplomaticAction+0x18.
//! Its concrete class is not authenticated by this layout.

/// Raw recorded dword; serialization emits its low byte as initiator relevance.
/// The saved action type is a separate field. Keep all recorded bits.
pub const ACTION_TYPE: usize = 0x10;
/// Diplomatic action body actor CCountryTag.
pub const ACTOR: usize = 0x14;
/// Diplomatic action body original actor CCountryTag u32.
pub const ORIGINAL_ACTOR: usize = 0x18;
/// Recipient raw country reference, u32 bits; only a signed-positive raw tag
/// is resolved, as with other recorded country references.
pub const RECIPIENT: usize = 0x1c;
/// Diplomatic action body original recipient CCountryTag u32.
pub const ORIGINAL_RECIPIENT: usize = 0x20;
/// Raw engine date word; signedness is unspecified, so preserve all 32 bits.
pub const HOUR: usize = 0x30;
/// Last-command raw engine date word, with the same unspecified signedness.
pub const LAST_COMMAND_HOUR: usize = 0x48;
