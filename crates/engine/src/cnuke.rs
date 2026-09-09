//! CNuke entries are inline in CCountry's two-type array, not pointers to rows.
//! The raw amount is a signed i64 progress quantity; it is not the ready count.

/// CNuke primary vtable, confirmed by pinned-exe RTTI and old nuke_stockpile_exact.rs:9.
pub const VTABLE_RVA: usize = 0x027d_0da0;
/// CNuke inline array stride; old nuke_stockpile_exact.rs:4 walks and authenticates each row.
pub const STRIDE: usize = 0x48;
/// CNuke owning CCountry pointer; old read_country_nukes checks +0x10 against the array owner.
pub const COUNTRY: usize = 0x10;
/// CNuke signed i64 raw progress/amount; old read_country_nukes copies both qword observations unchanged.
pub const AMOUNT: usize = 0x18;
/// CNuke type u32; old nuke_stockpile_exact.rs:7 checks type equals the array ordinal.
pub const TYPE: usize = 0x38;
/// CNuke ready count u32; old nuke_stockpile_exact.rs:8 reads it separately from amount.
pub const READY: usize = 0x3c;
/// CNuke two engine types; old constructor-invariant read requires the country array count to equal two.
pub const TYPE_COUNT: u32 = 2;
