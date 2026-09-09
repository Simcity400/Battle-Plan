//! Contained template storage returned by CArmy's template getter.
//!
//! CReferencedDivisionTemplate contains this at +0x18. Its RTTI does not name
//! the contained type; the old UZ dossier's CDivision label is unconfirmed.

/// Contained template subunit-entry vector data; old UZ func_140ff5c30.decompile.c:64-67 reads the getter result +A8.
pub const SUBUNIT_DATA: usize = 0xa8;
/// Contained template subunit-entry i32 count; old UZ func_140ff5c30.decompile.c:65-66 reads +B4.
pub const SUBUNIT_COUNT: usize = 0xb4;
/// Contained template subunit-entry stride; old UZ func_140ff5c30.decompile.c:65-67 advances two qwords, first points to CSubUnitDefinition.
pub const SUBUNIT_ENTRY_STRIDE: usize = 0x10;
