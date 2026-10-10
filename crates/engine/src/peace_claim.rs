//! Peace claim presentation operand. Its persistent ownership and hourly
//! source are not established. Presentation hooks supplying it are excluded from
//! the collector; the observed span does not establish a full allocation size.

/// Process-local action pointer, required to be nonnull.
pub const ACTION: usize = 0x08;
