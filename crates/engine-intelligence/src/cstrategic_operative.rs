//! Country strategic-operative object owning intel networks. Its owner
//! field is a raw country reference.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x298_b448;
/// Strategic-operative allocation size.
pub const SIZE: usize = 0x1c8;
/// Constructor initializing vtable and owner.
pub const CONSTRUCTOR_RVA: usize = 0x11d_ca80;
/// Raw owner country reference, u32.
pub const OWNER: usize = 0x08;
/// Intel-network pointer-vector data.
pub const NETWORKS_DATA: usize = 0x10;
/// Pointer-vector signed capacity.
pub const NETWORKS_CAPACITY: usize = 0x18;
/// Pointer-vector signed length.
pub const NETWORKS_COUNT: usize = 0x1c;
/// Network-vector allocator pointer.
pub const NETWORKS_ALLOCATOR: usize = 0x20;
/// Network-pointer width.
pub const NETWORK_STRIDE: usize = 8;
