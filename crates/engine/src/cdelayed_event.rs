//! CDelayedEvent layout.
//!
//! Scope and originator tags may both be zero. The containing vector supplies
//! the owning country identity. Due hours are a remaining duration, not an
//! absolute date. Event-hook acquisition is excluded.

/// CDelayedEvent primary vtable.
pub const VTABLE_RVA: usize = 0x027d_0fb8;
/// CDelayedEvent complete-object locator.
pub const COMPLETE_OBJECT_LOCATOR_RVA: usize = 0x02c1_5f18;
/// CDelayedEvent type descriptor.
pub const TYPE_DESCRIPTOR_RVA: usize = 0x0319_7af0;
/// CDelayedEvent class hierarchy descriptor.
pub const CLASS_HIERARCHY_RVA: usize = 0x02c1_5f40;
/// CDelayedEvent base-class descriptor array.
pub const BASE_CLASS_ARRAY_RVA: usize = 0x02c1_5f58;
/// CDelayedEvent self base descriptor.
pub const BASE_CLASS_DESCRIPTOR_RVA: usize = 0x02c1_5f70;
/// CDelayedEvent CEvent pointer.
pub const EVENT: usize = 0x08;
/// CDelayedEvent inline CEventScope.
pub const SCOPE: usize = 0x10;
/// CDelayedEvent scope country, u32.
pub const SCOPE_COUNTRY_TAG: usize = 0x18;
/// CDelayedEvent first scope random word, u32.
pub const SCOPE_RANDOM_A: usize = 0x1c;
/// CDelayedEvent second scope random word, u32.
pub const SCOPE_RANDOM_B: usize = 0x20;
/// CDelayedEvent originator country tag, u32.
pub const ORIGINATOR_TAG: usize = 0xc0;
/// CDelayedEvent remaining hours, signed i32.
pub const DUE_REMAINING_HOURS: usize = 0xc4;
/// Delayed-event serialization entry.
pub const SERIALIZE_RVA: usize = 0x0116_c5a0;
/// Delayed-event scope access used for dispatch.
pub const SCOPE_ACCESS_RVA: usize = 0x0022_2400;
