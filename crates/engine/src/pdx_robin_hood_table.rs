//! pdx_robin_hood_table specialization used by occupation status, laws and garrisons.
//! The +0xc field was called capacity by D66 and mask by the alert reader.
//! Both enumerate value + maximum_distance + 1 buckets; its name alone is not a proof.

/// Occupation robin-hood table bucket-array pointer; compact_contract/occupation_robin_hood.rs:75 reads the first qword.
pub const DATA: usize = 0x00;
/// Occupation robin-hood table live size, u32; old build_profile.rs:261 and read_robin_hood_table agree.
pub const SIZE: usize = 0x08;
/// Occupation robin-hood table capacity/mask span input, u32; old build_profile.rs:262 and alert reader +0xe4 agree on the offset.
pub const CAPACITY_OR_MASK: usize = 0x0c;
pub const MAX_DISTANCE: usize = 0x10;
