//! Allocator ABI for runtime DLC enumeration.

/// DLC vector allocator pointer global.
pub const SINGLETON_RVA: usize = 0x0270_1980;
/// Allocator auxiliary qword following the pointer global.
pub const AUX_RVA: usize = 0x0270_1988;
/// Allocator vtable pointer.
pub const VTABLE: usize = 0;
/// Free-method pointer at vtable +0x10.
pub const FREE_VTABLE_SLOT: usize = 0x10;
