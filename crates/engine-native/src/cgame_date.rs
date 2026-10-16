//! CGameDate includes its secondary persistent subobject.
//!
//! The vtable pointers at `GREGORIAN_BASE` and `PERSISTENT_BASE` belong to one date object. Textual
//! save hours range from 1 through 24.

/// CGameDate primary vtable.
pub const VTABLE_RVA: usize = 0x0270_2388;
/// CGameDate CPersistent vtable.
pub const PERSISTENT_VTABLE_RVA: usize = 0x0270_23b8;
/// CGameDate CGregorianDate base.
pub const GREGORIAN_BASE: usize = 0;
/// Embedded date base containing signed i32 raw hour.
pub const DATE_BASE: usize = 0x08;
/// CGameDate CPersistent base.
pub const PERSISTENT_BASE: usize = 0x10;
/// CGameDate null raw hour i32.
pub const NULL_RAW_HOUR: i32 = 43_808_760;
/// CGameDate hour-index getter.
pub const HOUR_INDEX_RVA: usize = 0x0052_70c0;
/// CGameDate deserializer.
pub const DESERIALIZE_RVA: usize = 0x0052_70d0;
/// CGameDate text parser.
pub const PARSE_RVA: usize = 0x0052_7450;
/// Date year setter.
pub const SET_YEAR_RVA: usize = 0x001a_8580;
/// Date text formatter.
pub const FORMAT_RVA: usize = 0x0225_4180;
/// Month-name formatter reads a signed raw hour; its receiver type is
/// not distinguished between game and Gregorian dates.
pub const FORMAT_WITH_MONTH_NAME_RVA: usize = 0x0052_6b50;
/// Unsupported game-date vtable candidate.
pub const REJECTED_LEGACY_VTABLE_RVA: usize = 0x026f_2398;
