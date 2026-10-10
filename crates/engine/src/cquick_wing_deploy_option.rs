//! Quick-wing-deployment option. Subunit and resolved equipment pointers
//! are distinct fields; comparing candidate identities does not establish
//! that both address the same object kind.

/// Bucket element stride.
pub const SIZE: usize = 0x20;
/// Option constructor storing separate subunit and equipment pointers.
pub const CONSTRUCTOR_RVA: usize = 0x01e7_b5e0;
/// Subunit-definition pointer supplied by the candidate argument.
pub const SUBUNIT_DEFINITION: usize = 0x08;
/// Equipment-object pointer, resolved through its full type-and-ID pair.
pub const EQUIPMENT: usize = 0x10;
