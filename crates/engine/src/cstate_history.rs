//! Top-level state-history loader constructing separate dated entries.

/// Primary vtable at displacement zero.
pub const VTABLE_RVA: usize = 0x029c_4d10;
/// Dated-block loader initializes a date, attempts key parsing and constructs
/// a state-history entry even when date parsing rejects the key.
pub const LOAD_DATED_BLOCK_RVA: usize = 0x0158_f190;
