//! Source object passed to the CFixedWindow construction wrapper; original class unproven.
//! This source owns a linked list at +0x3b0 and is distinct from CMapApplication.

/// CFixedWindow creation wrapper RVA.
pub const CREATE_WINDOW_RVA: usize = 0x022e_3280;
/// Source linked list.
pub const COPIED_STRING_LIST: usize = 0x3b0;
/// Preceding constructor-source member; semantics unspecified.
pub const MEMBER_398: usize = 0x398;
/// Constructor-source scalar, u32; semantics unspecified.
pub const SCALAR_414: usize = 0x414;
