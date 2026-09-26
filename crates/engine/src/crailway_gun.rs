//! CRailwayGun primary identity, verified in the pinned a729 constructor.
/// LEA at140E72D00 targets14295B600; secondary reference body is primary+10.
pub const VTABLE_RVA: usize = 0x0295_b600;
/// CRailwayGun CReferenceObject secondary vtable at primary+0x10;
/// constructor 140E72D0A/11 installs VA 14295B7D8 there.
pub const SECONDARY_VTABLE_RVA: usize = 0x0295_b7d8;
/// Count of the gun's own combat reference array at +0x420, which
/// `CRailwayGun::Serialize@140E78CF0` writes as `combat` (token 0x2916) and
/// which is distinct from the CUnit combat vector.
/// `CRailwayGun::HasReferenceEntries@140E76080` tests this signed count;
/// `Unit_GetActionCode@140BEF850` distinguishes idle (1) from code 12 with it.
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
