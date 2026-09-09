//! CDesignerEquipmentVariantItem; pinned-executable RTTI at vtable RVA 0x29B2268 confirms the class and primary base.
//! Excluded presentation knowledge from `design_catalog_context_live/src/a729_hook.rs`.

/// Row vtable RVA; a729_hook.rs:55,571 checked the exact a729 vtable before dereferencing each row.
pub const VTABLE_RVA: usize = 0x029b_2268;
/// CDesignerEquipmentVariantItem allocation size; A28_DYNAMIC_COLLECTION_TRACE.json:140 records 0x570-byte row reuse/allocation.
pub const SIZE: usize = 0x570;
/// Row +0x560 CEquipmentVariant pointer; a729_hook.rs:54,574 read and validated the target vtable/type.
pub const BOUND_VARIANT: usize = 0x560;
pub const BIND_RVA: usize = 0x0149_2b70;
