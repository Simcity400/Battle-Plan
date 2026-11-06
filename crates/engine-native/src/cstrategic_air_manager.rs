//! Strategic-air manager indexing per-country air objects, separate from
//! air-base country containers.

use engine_sites::Rva;
/// Primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x029d_5218);
/// Country-slot strategic-air pointer-vector data.
pub const COUNTRY_AIR: usize = 0x30;
/// State-indexed first-province lists for ordinary, kind-1 and kind-2 bases,
/// populated from map building sites and selected by base kind.
pub const DEFAULT_STATE_PROVINCE_LISTS: usize = 0x48;
pub const KIND1_STATE_PROVINCE_LISTS: usize = 0x60;
pub const KIND2_STATE_PROVINCE_LISTS: usize = 0x78;
/// State-indexed air-base pointer-array data.
pub const STATE_AIR_BASES: usize = 0x90;
/// State-indexed rocket air-base array.
pub const STATE_ROCKET_BASES: usize = 0xa8;
/// State-indexed gun air-base array.
pub const STATE_GUN_BASES: usize = 0xc0;
/// Dense stateless air-base pointer vector.
pub const STATELESS_AIR_BASES: usize = 0xd8;
/// Carrier-air-base registration storing the ship handle.
pub const ADD_CARRIER_AIR_BASE_RVA: Rva = Rva(0x00ca_8850);
/// Manager constructor initializing independent vector headers.
pub const CONSTRUCTOR_RVA: Rva = Rva(0x00ca_4960);
