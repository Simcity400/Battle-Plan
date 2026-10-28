//! CLoopHistoryContainer, the circular window reached through CLoopHistory's third window.

/// History-window primary vtable.
pub const VTABLE_RVA: usize = 0x02a4_bba8;
/// History-window record-pointer CPdxArray.
pub const RECORDS: usize = 8;

/// Signed history-window capacity used for modulo indexing.
pub const CAPACITY: usize = 0x20;
/// History-window current i32 circular slot, wrapping at capacity.
pub const CURRENT_SLOT: usize = 0x24;
/// History-window byte set on the first wrap.
pub const WRAPPED: usize = 0x28;
/// History-window record-fields pointer to the i64 field array.
pub const RECORD_FIELDS: usize = 0x30;
/// History-window owning history pointer.
pub const HISTORY: usize = 0x38;
