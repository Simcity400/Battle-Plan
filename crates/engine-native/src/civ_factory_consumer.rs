
use engine_sites::Rva;
pub const SIZE: usize = 0x18;
pub const AVAILABLE: usize = 0x0c;
pub const ALLOCATED: usize = 0x10;

pub const CONSUMER_GOODS_VTABLE_RVA: Rva = Rva(0x029e_cf80);
pub const SPECIAL_PROJECTS_VTABLE_RVA: Rva = Rva(0x029e_cf98);
pub const LICENSED_PRODUCTION_VTABLE_RVA: Rva = Rva(0x029e_cfb0);
pub const TRADE_VTABLE_RVA: Rva = Rva(0x029e_cfc8);
pub const CONTRACT_PAYMENTS_VTABLE_RVA: Rva = Rva(0x029e_cfe0);
