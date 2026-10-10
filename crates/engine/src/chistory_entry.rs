//! CHistoryEntry base layout.

/// CHistoryEntry primary vtable.
pub const VTABLE_RVA: usize = 0x0292_5ac8;
/// Embedded CGameDate object.
pub const DATE: usize = 0x08;
/// Date's signed i32 raw hour.
pub const DATE_RAW_HOUR: usize = 0x10;
/// Child-entry list head pointer.
pub const CHILDREN_HEAD: usize = 0x20;
/// Child-entry list tail pointer.
pub const CHILDREN_TAIL: usize = 0x28;
/// Child-entry list count, i32.
pub const CHILDREN_COUNT: usize = 0x30;
/// History-entry constructor initializes the embedded date and clears the list.
pub const CONSTRUCTOR_RVA: usize = 0x00a2_db40;
/// Dated-entry insertion sorts by signed raw hour, after existing equal dates.
pub const INSERT_BY_DATE_RVA: usize = 0x00a2_e010;
/// Date setter copies the raw hour and conditionally recurses through children according to their virtual predicate.
pub const SET_DATE_RVA: usize = 0x00a2_ebc0;
