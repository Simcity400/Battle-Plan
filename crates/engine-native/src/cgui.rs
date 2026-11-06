//! `CGui` manager; retained registration knowledge, excluded from hourly state.

use engine_sites::Rva;
/// CGui primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x02bb_4e08);
/// CGui secondary vtable.
pub const SECONDARY_VTABLE_RVA: Rva = Rva(0x02bb_4ee0);
/// CGui singleton pointer.
pub const SINGLETON_RVA: Rva = Rva(0x0350_0fc0);
/// CGui constructor.
pub const CONSTRUCTOR_RVA: Rva = Rva(0x022b_8b30);
/// CGui graphics pointer.
pub const GRAPHICS: usize = 0xb8;
