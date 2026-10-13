//! Railway-gun primary identity with unit class 13.
/// Primary vtable; secondary reference subobject is at primary+0x10.
pub const VTABLE_RVA: usize = 0x0295_b600;
/// Reference-object secondary vtable at primary+0x10.
pub const SECONDARY_VTABLE_RVA: usize = 0x0295_b7d8;
/// Signed count of the gun's own combat reference array, serialized as
/// `combat` and separate from the unit combat vector. This count distinguishes
/// idle action 1 from action 12.
pub const REFERENCE_ENTRY_COUNT: usize = 0x42c;
/// Runtime COrdersGroup `CRailwayGunAssignee` subobject pointer. The assign
/// command stores group+[`ASSIGNEE_BASE`] here; the counter join compares it.
pub const ASSIGNEE: usize = 0x440;
/// Offset of the `CRailwayGunAssignee` subobject inside COrdersGroup.
pub const ASSIGNEE_BASE: usize = 0x18;
/// COrdersGroup's `CRailwayGunAssignee` secondary vtable at primary+0x18.
pub const ORDERS_GROUP_ASSIGNEE_VTABLE_RVA: usize = 0x0293_bcd8;
/// CRailwayGun current strength, fixed5; the native strength getter divides
/// it by the per-gun rebuilt CSubUnitDefinition maximum.
pub const CURRENT_STRENGTH: usize = 0x3f0;
/// Railway gun current manpower, the signed low dword of primary +0x3F8:
/// immediate damage caps casualties at this value and subtracts them; refill
/// withdraws country manpower into it.
pub const MANPOWER: usize = 0x3f8;
