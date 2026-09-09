//! CThreatSource objects reached through CWorldThreat's pointer array.

/// CThreatSource signed raw country link id; old build_profile.rs:72 and D57 walk resolve through country equivalence.
pub const COUNTRY: usize = 0x08;
pub const CURRENT: usize = 0x10;
/// CThreatSource original contribution i64; old build_profile.rs:76 and CThreatSource serializer width note.
pub const ORIGINAL: usize = 0x18;
/// CThreatSource raw date/hour i32; old build_profile.rs:77 and D57 ledger reader.
pub const DATE: usize = 0x30;
/// CThreatSource reason std::string; old build_profile.rs:78 and read_domain57_world_tension_reason.
pub const REASON: usize = 0x40;
