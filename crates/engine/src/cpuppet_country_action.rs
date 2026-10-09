//! CPuppetCountryAction layout for the hash-pinned executable.

/// Primary vtable, confirmed by a729 RTTI and constructor 141506DC0.
pub const VTABLE_RVA: usize = 0x029b_c530;
/// Raw country reference; 14150DC80 serializes it and 14150D230 reads it.
pub const PUPPET: usize = 0x50;
/// Opaque eight bytes: 1415093E0 clones them, 141506DC0 initializes them.
/// Their meaning is unproved; do not decode them or follow them as a pointer.
pub const AUXILIARY: usize = 0x58;
