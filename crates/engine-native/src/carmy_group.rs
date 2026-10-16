//! CArmyGroup derives from COrdersGroup; shared fields are in corders_group.

/// CArmyGroup vtable authenticating group roots.
pub const VTABLE_RVA: usize = 0x0293_bcf8;
/// Child orders-group pointer-vector data.
pub const MEMBER_DATA: usize = 0x230;
/// Child-vector capacity, u32.
pub const MEMBER_CAPACITY: usize = 0x238;
/// Child-vector count, u32.
pub const MEMBER_COUNT: usize = 0x23c;
/// Group allocation size.
pub const SIZE: usize = 0x250;
/// Constructor installing the child vector and group discriminator.
pub const CONSTRUCTOR_RVA: usize = 0x00bd_a6b0;
