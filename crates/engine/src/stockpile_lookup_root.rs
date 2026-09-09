//! Stockpile lookup indirection in the garrison path; native class unknown.

/// Stockpile lookup value-table pointer; old read_default_stockpile_amount dereferences lookup_ref+0.
pub const VALUES: usize = 0x00;
/// Stockpile lookup value width, signed i64; old read_default_stockpile_amount indexes by equipment type+0x538 times eight.
pub const VALUE_STRIDE: usize = 0x08;
