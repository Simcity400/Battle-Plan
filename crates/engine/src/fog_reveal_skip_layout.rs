//! Pointee layout of the division fog-skip predicate.
//! The receiving pointer does not authenticate a template or recorded division field.

/// Fog-skip byte; flagged divisions are skipped. Writer and pointee class are unspecified.
pub const FLAG: usize = 0x234;
