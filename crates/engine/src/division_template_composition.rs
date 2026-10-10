//! Contained division-template storage, separate from its reference wrapper.

/// Subunit-entry vector data.
pub const SUBUNIT_DATA: usize = 0xa8;
/// Signed i32 subunit-entry count.
pub const SUBUNIT_COUNT: usize = 0xb4;
/// Subunit-entry stride; the first word points to a subunit definition.
pub const SUBUNIT_ENTRY_STRIDE: usize = 0x10;
