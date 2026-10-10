//! Trigger-node base layout and virtual contract.
//! This module installs no trigger evaluator or hook.

/// CTrigger loaded-node interned keyword, u32.
pub const KEYWORD: usize = 0x20;
/// Condition-check virtual byte offset.
pub const CHECK_VTABLE_OFFSET: usize = 0x18;
/// CTrigger accepted-scope virtual byte offset.
pub const ACCEPTED_SCOPE_VTABLE_OFFSET: usize = 0x70;
/// Evaluation virtual byte offset; reached after scope compatibility succeeds.
pub const EVALUATE_VTABLE_OFFSET: usize = 0xb0;
/// CTrigger child-ignore virtual byte offset.
pub const IGNORE_VTABLE_OFFSET: usize = 0x10;
/// Condition check; incompatible scopes return false.
pub const CHECK_RVA: usize = 0x0054_0570;
/// Scope compatibility; any accepted carried scope suffices.
pub const SCOPE_COMPATIBLE_RVA: usize = 0x0054_2ca0;
/// Child loader; alternative branches attach to the preceding conditional node.
pub const LOAD_CHILD_RVA: usize = 0x0054_34f0;
/// State scope bit.
pub const SCOPE_STATE: u32 = 0x02;
/// Country scope bit.
pub const SCOPE_COUNTRY: u32 = 0x04;
/// Unit-leader scope bit.
pub const SCOPE_UNIT_LEADER: u32 = 0x08;
/// Combat scope bit.
pub const SCOPE_COMBAT: u32 = 0x10;
/// Formation scope bit.
pub const SCOPE_FORMATION: u32 = 0x40;

/// CTrigger child-pointer array.
pub const CHILDREN: usize = 0x08;
/// CTrigger child count i32.
pub const CHILD_COUNT: usize = 0x14;
/// CTrigger child-pointer stride.
pub const CHILD_STRIDE: usize = 8;
/// CTrigger body-loader virtual byte offset.
pub const LOAD_BODY_VTABLE_OFFSET: usize = 0x28;
/// Shared ignore-false implementation; returns zero.
pub const IGNORE_FALSE_RVA: usize = 0x0011_cd10;
