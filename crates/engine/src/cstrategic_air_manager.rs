//! Strategic-air manager indexing per-country air objects, separate from
//! air-base country containers.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x0294_2118;
/// Country-slot strategic-air pointer-vector data.
pub const COUNTRY_AIR: usize = 0x30;
/// Country-slot vector count, u32.
pub const COUNTRY_AIR_COUNT: usize = 0x3c;
/// State-indexed first-province lists for ordinary, kind-1 and kind-2 bases,
/// populated from map building sites and selected by base kind.
pub const DEFAULT_STATE_PROVINCE_LISTS: usize = 0x48;
pub const KIND1_STATE_PROVINCE_LISTS: usize = 0x60;
pub const KIND2_STATE_PROVINCE_LISTS: usize = 0x78;
/// State-indexed air-base pointer-array data.
pub const STATE_AIR_BASES: usize = 0x90;
/// State-base array capacity.
pub const STATE_AIR_BASES_CAPACITY: usize = 0x98;
/// State-base array count.
pub const STATE_AIR_BASES_COUNT: usize = 0x9c;
/// State-indexed rocket air-base array.
pub const STATE_ROCKET_BASES: usize = 0xa8;
/// State-indexed gun air-base array.
pub const STATE_GUN_BASES: usize = 0xc0;
/// Dense stateless air-base pointer vector.
pub const STATELESS_AIR_BASES: usize = 0xd8;
/// Stateless-base vector capacity, u32.
pub const STATELESS_AIR_BASES_CAPACITY: usize = 0xe0;
/// Stateless-base vector count, u32.
pub const STATELESS_AIR_BASES_COUNT: usize = 0xe4;
/// Stateless-base erasure nulls its slot before compaction.
pub const ERASE_STATELESS_AIR_BASE_RVA: usize = 0x00c4_1940;
/// Carrier-air-base registration storing the ship handle.
pub const ADD_CARRIER_AIR_BASE_RVA: usize = 0x00c3_be60;
/// Building-driven base creation receiving the manager secondary interface.
pub const BUILDING_AIR_BASE_RVA: usize = 0x00c5_2420;
/// Country-entry lookup resolving raw tag before indexing.
pub const COUNTRY_ENTRY_RVA: usize = 0x006e_ca90;
/// Manager constructor initializing independent vector headers.
pub const CONSTRUCTOR_RVA: usize = 0x00c3_8a40;
