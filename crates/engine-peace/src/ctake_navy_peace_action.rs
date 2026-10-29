//! Take-navy peace-action layout for the supported executable.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x02a4_c510;

/// Installed image literal passed for localisation. Read its bytes from
/// the installation rather than duplicating a key spelling.
pub const LABEL_LITERAL_RVA: usize = 0x029e_c078;
/// Image literal storage length excluding its terminating NUL.
pub const LABEL_LITERAL_BYTES: usize = 22;
/// Optional ship pointer serialized as a full handle when present.
pub const SHIP: usize = 0x48;
/// Signed fixed5 screening ratio, accumulated during settlement.
pub const RATIO: usize = 0x50;
