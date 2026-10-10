//! State-building history command. State assignments and province-indexed
//! assignments remain separate members.

/// Primary vtable at object displacement zero.
pub const VTABLE_RVA: usize = 0x029c_4bc0;
/// Command allocation size.
pub const SIZE: usize = 0x70;
/// Inherited state-template pointer.
pub const STATE_TEMPLATE: usize = 0x40;
/// State-building assignment array header.
pub const STATE_BUILDING_ASSIGNMENTS: usize = 0x48;
/// Assignment signed capacity, checked against live count before allocation.
pub const STATE_BUILDING_ASSIGNMENT_CAPACITY: usize = 0x50;
/// Assignment signed count, restored on growth and incremented on append.
pub const STATE_BUILDING_ASSIGNMENT_COUNT: usize = 0x54;
/// Assignment allocator pointer used for capacity growth.
pub const STATE_BUILDING_ASSIGNMENT_ALLOCATOR: usize = 0x58;
/// Province-ID-keyed assignment map header with sentinel.
pub const PROVINCE_BUILDING_ASSIGNMENTS: usize = 0x60;
/// Live map-node count, u64, checked against map limit before insertion.
pub const PROVINCE_BUILDING_ASSIGNMENT_COUNT: usize = 0x68;
/// Building-history parser receiving the constructed command.
pub const PARSE_RVA: usize = 0x0158_dfc0;
/// Building-history application validating state province membership before
/// applying the state array and province map assignments in order.
pub const APPLY_RVA: usize = 0x0158_e970;
