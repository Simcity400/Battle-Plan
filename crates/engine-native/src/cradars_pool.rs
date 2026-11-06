//! Country-embedded radar intel source. It owns the coverage groups
//! described in `radar_group`.

use engine_sites::Rva;
/// Primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x02a0_5068);
/// Static-intel-source secondary vtable.
pub const SECONDARY_VTABLE_RVA: Rva = Rva(0x02a0_5088);
/// Coverage-group pointer vector.
pub const GROUPS: usize = 0x28;
/// Coverage-group vector count, u32.
pub const GROUPS_COUNT: usize = 0x34;
/// Raw owner country reference copied from the constructor country identity.
pub const COUNTRY: usize = 0x40;
/// Radar-pool constructor.
pub const CONSTRUCTOR_RVA: Rva = Rva(0x0112_a510);
