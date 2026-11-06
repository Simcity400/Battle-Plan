//! CAirBaseAccessRelation; shared fields live in `crelation`.

use engine_sites::Rva;
/// Air-base access relation vtable.
pub const VTABLE_RVA: Rva = Rva(0x2a9a8d0);
/// Relation type word used by construction and factory dispatch.
pub const TYPE: u32 = 0x283a;
/// Constructor receiving the allocated relation object.
pub const CONSTRUCTOR_RVA: Rva = Rva(0x01a1_3b10);
