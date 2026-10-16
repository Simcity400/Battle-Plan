//! `CGui` manager; retained registration knowledge, excluded from hourly state.

/// CGui primary vtable.
pub const VTABLE_RVA: usize = 0x02b2_8720;
/// CGui secondary vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x02b2_87f8;
/// CGui singleton pointer.
pub const SINGLETON_RVA: usize = 0x0343_a2d0;
/// CGui constructor.
pub const CONSTRUCTOR_RVA: usize = 0x0224_4bf0;
/// CGui registered-object array data pointer.
pub const REGISTERED_OBJECTS: usize = 0x108;
/// CGui registered-object array native i32 capacity.
pub const REGISTERED_OBJECT_CAPACITY: usize = 0x110;
/// CGui registered-object array native i32 count.
pub const REGISTERED_OBJECT_COUNT: usize = 0x114;
/// GUI update-permitted byte checked before registration.
pub const CAN_UPDATE: usize = 0x249;
/// CGui registration byte slot.
pub const REGISTER_OBJECT_VSLOT: usize = 0xa0;
/// GUI registration appends an object and may activate it without assigning a sprite effect.
pub const REGISTER_OBJECT_RVA: usize = 0x0224_5830;
/// GUI registration active-object continuation, reached only for an
/// active widget dispatch result.
pub const REGISTER_ACTIVE_OBJECT_RVA: usize = 0x0224_55c0;
/// CGui graphics pointer.
pub const GRAPHICS: usize = 0xb8;
/// CGui embedded type-name lookup tree.
pub const TYPE_LOOKUP_TREE: usize = 0xc0;
/// GUI container-window factory dispatch at +0x60, inherited by the game GUI.
pub const CONTAINER_FACTORY_VSLOT: usize = 0x60;
/// CGui container-window factory.
pub const CONTAINER_FACTORY_RVA: usize = 0x0224_6270;
