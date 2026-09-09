//! CExtendedScrollbar layout; presentation-only knowledge, with native i64 fixed values.
//! Sources are archived under archive/pre-rewrite-2026-09-09.
//! Matching installed-exe RTTI confirms CExtendedScrollbar at displacements 0, +0x80 and +0xb0.

/// CExtendedScrollbar primary vtable; old dynamic_collection_trace_live/src/standard_list.rs:33 gates four production readers.
pub const VTABLE_RVA: usize = 0x02b3_2ff0;
/// CExtendedScrollbar value-base vtable at +0x80; old standard_list.rs:34 verifies the secondary base.
pub const VALUE_VTABLE_RVA: usize = 0x02b3_3270;
/// CExtendedScrollbar third-base vtable at +0xb0; old standard_list.rs:35 verifies the third base.
pub const THIRD_VTABLE_RVA: usize = 0x02b3_3300;
/// CExtendedScrollbar value interface +0x80; old focus_research_decisions_depth_live/src/a729_focus_item_hook.rs:1713-1740 invokes this base.
pub const VALUE_INTERFACE: usize = 0x80;
/// CExtendedScrollbar third interface +0xb0; old dynamic_collection_trace_live list readers gate its vtable before reading.
pub const THIRD_INTERFACE: usize = 0xb0;
/// CExtendedScrollbar lower range i64 +0x1558; old standard_list.rs:37 and list readers preserve the native fixed value.
pub const MINIMUM: usize = 0x1558;
/// CExtendedScrollbar upper range i64 +0x1560; old standard_list.rs:38 and list readers preserve the native fixed value.
pub const MAXIMUM: usize = 0x1560;
/// CExtendedScrollbar target i64 +0x15f8; old standard_list.rs:39 and FUN1422d6cf0 write/read this native fixed value.
pub const TARGET: usize = 0x15f8;
/// CExtendedScrollbar displayed current i64 +0x1600; old standard_list.rs:40 and FUN1422d6fe0 write/read this native fixed value.
pub const CURRENT: usize = 0x1600;
/// CExtendedScrollbar allocation size; old facility-specialization hook dossier records destructor-derived 0x1620 bytes.
pub const SIZE: usize = 0x1620;
/// CExtendedScrollbar target mutation function; old facility dossier identifies FUN1422d6cf0 range clamp and target write.
pub const SET_TARGET_RVA: usize = 0x022d_6cf0;
/// CExtendedScrollbar animation function; old facility dossier identifies FUN1422d6fe0 moving current toward target.
pub const UPDATE_CURRENT_RVA: usize = 0x022d_6fe0;
/// CExtendedScrollbar geometry/range rescaling function; old facility dossier identifies FUN1422d64c0 and its current clamp.
pub const RESCALE_RVA: usize = 0x022d_64c0;
/// CExtendedScrollbar value-interface virtual slot +0x20 returns f32 with argument 1; old focus item reader invokes it, distinct from raw i64 fields.
pub const DISPLAY_VALUE_SLOT: usize = 0x20;
