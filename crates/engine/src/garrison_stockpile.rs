//! Stockpile root used by the old garrison-alert reader at CCountry +0xF98.
//!
//! Native class identity and validity at the hourly boundary are unproved.

/// Garrison stockpile category-pointer array; old a729_garrison_alert.rs:124 and read_default_stockpile_amount read +0x08.
pub const CATEGORIES: usize = 0x08;
/// Garrison stockpile context pointer; old read_stockpile_amount reads +0x20 before special-stockpile lookup.
pub const CONTEXT: usize = 0x20;
/// Garrison stockpile default category index; old read_default_stockpile_amount selects element 0x0F.
pub const DEFAULT_CATEGORY: usize = 0x0f;
/// Garrison stockpile category-pointer stride; old read_default_stockpile_amount multiplies category index by eight.
pub const CATEGORY_STRIDE: usize = 0x08;
