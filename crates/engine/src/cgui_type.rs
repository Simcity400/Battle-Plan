//! Authored `CGuiType` definition shared by GUI widgets; presentation knowledge only.

/// CGuiType vtable.
pub const VTABLE_RVA: usize = 0x02b3_1868;
/// CGuiType authored name, MSVC string +0x28.
pub const NAME: usize = 0x28;
/// CGuiType authored-name getter vtable byte slot.
pub const NAME_VSLOT: usize = 0x48;
/// CGuiType authored source path, MSVC string +0x48.
pub const SOURCE_PATH: usize = 0x48;
/// Authored-type source line u32 +0xE0.
pub const SOURCE_LINE_CANDIDATE: usize = 0xe0;
