//! Research UI view. Its callback lifetime does not establish an hourly
//! walk or a consistent whole-world observation.

/// Selected-research object pointer at +0x520, with technology pointer at +0x18.
pub const SELECTED_RESEARCH_OBJECT: usize = 0x520;
/// Year-action routine taking view, action field and output.
pub const YEAR_PRODUCER_RVA: usize = 0x01bc_ea60;
/// Exact year-action entry sentinel.
pub const YEAR_PRODUCER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
];
/// Year-action name materialization instruction site.
pub const YEAR_ACTION_NAME_MATERIALIZATION_RVA: usize = 0x01bc_f2d3;
/// Year-action resolver call site.
pub const YEAR_ACTION_RESOLVER_CALL_RVA: usize = 0x01bc_f2e5;
/// Year-action identity comparison instruction site.
pub const YEAR_ACTION_IDENTITY_COMPARE_RVA: usize = 0x01bc_f326;
/// Selected year-action helper call site.
pub const YEAR_HELPER_CALL_RVA: usize = 0x01bc_f37f;
/// Exact selected-helper return site required by the caller frame.
pub const YEAR_HELPER_RETURN_RVA: usize = 0x01bc_f384;
