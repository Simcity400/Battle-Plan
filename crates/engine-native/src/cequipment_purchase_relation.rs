//! CEquipmentPurchaseRelation; shared fields live in `crelation`.

use engine_sites::Rva;
/// CEquipmentPurchaseRelation primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x2a9ad50);
/// CEquipmentPurchaseRelation native type word.
pub const TYPE: u32 = 0x33e8;
/// Equipment-purchase relation constructor; allocation size is 0x50 bytes.
pub const CONSTRUCTOR_RVA: Rva = Rva(0x01a1_49f0);
