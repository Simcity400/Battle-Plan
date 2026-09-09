//! Allocator ABI used for runtime DLC enumeration; concrete RTTI class name unproven.

/// DLC vector allocator pointer global; old sample_runtime_dlc_names reads module+0x2701980.
pub const SINGLETON_RVA: usize = 0x0270_1980;
/// DLC allocator auxiliary global qword; old sample_runtime_dlc_names reads the qword following the pointer.
pub const AUX_RVA: usize = 0x0270_1988;
/// Allocator vtable pointer; release_runtime_dlc_string_vector dereferences the allocator object first.
pub const VTABLE: usize = 0;
/// Allocator free-method vtable slot; release_runtime_dlc_string_vector calls the pointer at vtable+0x10.
pub const FREE_VTABLE_SLOT: usize = 0x10;
