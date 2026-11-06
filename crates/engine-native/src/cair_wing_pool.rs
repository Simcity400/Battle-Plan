//! CAirWingPool; all offsets are primary-object relative.

use engine_sites::Rva;
pub(crate) mod fuel;

/// Wing-pool primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x029f_77d8);
/// Wing-pool allocation size.
pub const SIZE: usize = 0x48;
/// Full CAirBase reference containing u32 type and id.
pub const AIR_BASE_REFERENCE: usize = 0x18;
/// Subunit-definition pointer, initially the registered null-definition singleton.
pub const DEFINITION: usize = 0x20;
/// Primary wing-pointer vector.
pub const WINGS: usize = 0x28;
/// Wing-vector count, u32.
#[cfg(test)]
pub const WINGS_COUNT: usize = 0x34;
/// Owning strategic-air object supplying the country reference.
pub const OWNER: usize = 0x40;
/// Constructor installing the vtable and wing vector.
pub const CONSTRUCTOR_RVA: Rva = Rva(0x00fd_e990);
/// Destructor unregistering through the base reference.
pub const DESTRUCTOR_RVA: Rva = Rva(0x00fd_ef50);
