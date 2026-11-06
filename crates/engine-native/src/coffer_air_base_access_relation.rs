//! COfferAirBaseAccessRelation; shared fields live in `crelation`.

use engine_sites::Rva;
/// COfferAirBaseAccessRelation primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x02a9_a990);
/// COfferAirBaseAccessRelation native type word.
pub const TYPE: u32 = 0x2855;
/// Air-base access-offer constructor; allocation size is 0x50 bytes.
pub const CONSTRUCTOR_RVA: Rva = Rva(0x01a1_3b40);
