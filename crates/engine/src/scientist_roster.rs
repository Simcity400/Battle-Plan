//! Scientist roster object reached through `CCountry +0xff0`; native class name unknown.
//!
//! Sources: `scientists_live_lane/src/lib.rs:9-12` and archived collector
//! `scientist_pins_from_country_object`. No vtable guard identifies this root.

/// Scientist roster +0xa0 wrapper-pointer array data; read by collector and standalone scientist lane.
pub const ENTRIES: usize = 0xa0;
/// Scientist roster +0xac wrapper count u32; old SCIENTIST_ROSTER_VECTOR_COUNT +0x0c is vector-relative.
pub const COUNT: usize = 0xac;
