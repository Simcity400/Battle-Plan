//! Volunteer-host list readers. Country arguments and resolved references
//! do not establish concrete callable ownership.

/// Volunteer-host list reader with a country argument selecting +0xf88.
pub const VOLUNTEER_HOST_LIST_READER_RVA: usize = 0x00c4_3b30;
/// Volunteer-host pair reader: resolve both country-reference arguments
/// before each query. Callable ownership is unconfirmed.
pub const VOLUNTEER_HOST_PAIR_READER_RVA: usize = 0x00c4_9a80;
