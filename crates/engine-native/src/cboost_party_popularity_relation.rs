//! CBoostPartyPopularityRelation; shared fields live in `crelation`.

use engine_sites::Rva;
/// Popularity relation primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x029d_d088);
/// Relation type word used by construction and factory dispatch.
pub const TYPE: u32 = 0x30ed;
