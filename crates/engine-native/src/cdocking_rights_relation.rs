//! CDockingRightsRelation; shared fields live in `crelation`.

use engine_sites::Rva;
/// CDockingRightsRelation primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x2a9aac0);
/// CDockingRightsRelation native type word.
pub const TYPE: u32 = 0x3afa;
/// Docking-rights relation constructor; its allocation size is 0x50 bytes.
pub const CONSTRUCTOR_RVA: Rva = Rva(0x01a1_40d0);
