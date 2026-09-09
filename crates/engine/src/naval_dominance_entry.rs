//! Country-keyed entry containing a CDominanceValues object at +0x08.
//!
//! The outer entry's engine template name was not recovered by the old walk.

/// Naval dominance array entry stride; naval_missions_status_live::read_naval_supremacy_rows steps 0x50 bytes.
pub const STRIDE: usize = 0x50;
/// Naval dominance entry country key, u32; old DOMINANCE_VALUES_OFF_TAG is relative to this outer entry.
pub const COUNTRY_KEY: usize = 0x00;
/// Naval dominance entry CDominanceValues subobject; old vtable at entry+8 has RTTI displacement 0.
pub const VALUES: usize = 0x08;
