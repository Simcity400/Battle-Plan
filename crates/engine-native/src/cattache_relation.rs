//! CAttacheRelation; shared fields live in `crelation`.

use engine_sites::Rva;
/// Attache relation primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x29dcfc8);
/// Relation type word used by construction and factory dispatch.
pub const TYPE: u32 = 0x38d9;
