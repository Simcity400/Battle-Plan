//! The engine's standard list widget: its two vtables and its layout.
//!
//! Four hooks in this crate walk this one widget -- the air-selection list, the
//! facility smooth list, the MIO bonus list and the MIO history list. Each
//! authenticates the object it was handed against `PRIMARY_VTABLE_RVA` and
//! `SECONDARY_VTABLE_RVA` before reading it, which is the proof that all four
//! are looking at the SAME engine type and not at four types that happen to
//! agree. Each then carried its own copy of the same seven numbers, so a
//! re-measurement had four places to land and three ways to be forgotten.

/// The list object's own vtable, checked at `list + 0`.
pub const PRIMARY_VTABLE_RVA: usize = 0x02B2_CB48;
/// The second base's vtable, checked at `list + 8`.
pub const SECONDARY_VTABLE_RVA: usize = 0x02B2_CC78;

pub const HEAD_OFFSET: usize = 0x90;
pub const COUNT_OFFSET: usize = 0xA0;
pub const SCROLLBAR_OFFSET: usize = 0x190;

pub const NODE_ITEM_OFFSET: usize = 0;
pub const NODE_NEXT_OFFSET: usize = 0x10;

/// The extended scrollbar the same four hooks reach through
/// [`SCROLLBAR_OFFSET`], on the same terms: each authenticates it against all
/// three vtables below before reading the same four fields, which is the proof
/// that this is ONE widget and not four that agree.
///
/// These sat beside the list layout in all four files and were left duplicated
/// when the list layout was shared, because the sweep that found the list
/// family could not see a value like `0x15F8` at all -- it admitted only
/// `0..=255` and powers of two.
pub mod scrollbar {
    pub const PRIMARY_VTABLE_RVA: usize = 0x02B3_2FF0;
    pub const PLUS_80_VTABLE_RVA: usize = 0x02B3_3270;
    pub const PLUS_B0_VTABLE_RVA: usize = 0x02B3_3300;

    pub const MIN_OFFSET: usize = 0x1558;
    pub const MAX_OFFSET: usize = 0x1560;
    pub const TARGET_OFFSET: usize = 0x15F8;
    pub const CURRENT_OFFSET: usize = 0x1600;
}
