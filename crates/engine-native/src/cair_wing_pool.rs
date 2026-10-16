//! CAirWingPool; all offsets are primary-object relative.

pub(crate) mod fuel;

/// Wing-pool primary vtable.
pub const VTABLE_RVA: usize = 0x0296_4208;
/// Wing-pool allocation size.
pub const SIZE: usize = 0x48;
/// Full CAirBase reference containing u32 type and id.
pub const AIR_BASE_REFERENCE: usize = 0x18;
/// Subunit-definition pointer, initially the registered null-definition singleton.
pub const DEFINITION: usize = 0x20;
/// Primary wing-pointer vector.
pub const WINGS: usize = 0x28;
/// Wing-vector capacity, u32.
pub const WINGS_CAPACITY: usize = 0x30;
/// Wing-vector count, u32.
pub const WINGS_COUNT: usize = 0x34;
/// Owning strategic-air object supplying the country reference.
pub const OWNER: usize = 0x40;
/// Constructor installing the vtable and wing vector.
pub const CONSTRUCTOR_RVA: usize = 0x00f4_5290;
/// Alternate constructor sharing this layout.
pub const ALTERNATE_CONSTRUCTOR_RVA: usize = 0x00f4_5300;
/// Destructor unregistering through the base reference.
pub const DESTRUCTOR_RVA: usize = 0x00f4_5860;
/// Current fuel sum visiting every stored wing pointer.
pub const CURRENT_FUEL_RVA: usize = 0x00f4_6340;
/// Maximum fuel sum visiting the same vector with training gates.
pub const MAXIMUM_FUEL_RVA: usize = 0x00f4_68a0;
