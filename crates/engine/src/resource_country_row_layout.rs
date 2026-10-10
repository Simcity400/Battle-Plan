//! Sorted resource-to-country row used by the resource-scope receiver.

/// Row stride: two i32 words.
pub const STRIDE: usize = 8;
/// Resource slot, i32, compared with resource +0xd8.
pub const RESOURCE_SLOT: usize = 0;
/// Country link, i32, returned when the searched resource slot matches.
pub const COUNTRY: usize = 4;
