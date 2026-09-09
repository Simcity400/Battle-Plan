//! `CCombatTactic` identity and preferred-tactic modifier key.

/// CCombatTactic primary vtable; archived offsets.rs:809, confirmed by a729 RTTI.
pub const VTABLE_RVA: usize = 0x027d_0058;
/// CCombatTactic MSVC string token; archived offsets.rs:811 and `compact_country_preferred_tactic_row`.
pub const TOKEN: usize = 0x10;
/// CCombatTactic MSVC string name; archived offsets.rs:812 and side-detail tactic identity read.
pub const NAME: usize = 0x30;
/// CCombatTactic i32 database id; archived offsets.rs:813 and preferred-tactic identity read.
pub const ID: usize = 0x98;
/// CCombatTactic u32 modifier-metadata key; archived tactic_weight.rs:29 and preferred-tactic weight reader.
pub const MODIFIER_KEY: usize = 0x174;
