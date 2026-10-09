//! CTakeNavyPeaceAction layout for the hash-pinned executable.

/// Primary vtable, confirmed by a729 RTTI and constructor 141506E90.
pub const VTABLE_RVA: usize = 0x029b_c770;

/// 141506ED3 loads this image literal into RDX before localizing at 141506EDF.
/// Read the bytes, not a duplicated localisation-key spelling.
pub const LABEL_LITERAL_RVA: usize = 0x0295_8c18;
/// Literal storage at 142958C18 ends with NUL at 142958C2D.
pub const LABEL_LITERAL_BYTES: usize = 22;
/// Optional CShip pointer; 14150DE40 serializes its full reference, when set.
pub const SHIP: usize = 0x48;
/// Signed fixed5 screening ratio; 14150DE40 writes it, 141442330 aggregates it.
pub const RATIO: usize = 0x50;
