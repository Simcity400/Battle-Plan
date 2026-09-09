//! CNameGroupMember as read by deployment-line naming.
//!
//! The full generated display name is not recovered by this walk. Zero name
//! order was omitted by the old wire representation, not proved absent.

/// CNameGroupMember primary vtable; offsets.rs:433 and pinned RTTI confirm the actual class name.
pub const VTABLE_RVA: usize = 0x291f518;
/// CNameGroupMember raw name order, i32; offsets.rs:434 and read_deployment_line read +0x80.
pub const ORDER: usize = 0x80;
