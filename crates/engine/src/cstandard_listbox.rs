//! CStandardlistbox layout retained from the excluded dynamic-collection presentation lane.
//! Sources are under archive/pre-rewrite-2026-09-09; no hourly walker is implied.
//! The earlier facility dossier used offsets eight bytes lower than the production walker.
//! Those candidate offsets retain their unresolved pointer basis instead of becoming aliases.
//! Matching installed-exe RTTI confirms CStandardlistbox with primary displacement 0 and secondary +8.

/// CStandardlistbox primary vtable; old dynamic_collection_trace_live/src/standard_list.rs:12 gates four production list readers.
pub const VTABLE_RVA: usize = 0x02b2_cb48;
/// CStandardlistbox secondary vtable; old standard_list.rs:14 verifies the base at +8 before each list walk.
pub const SECONDARY_VTABLE_RVA: usize = 0x02b2_cc78;
/// CStandardlistbox secondary interface +8; old a729_mio_history_hook.rs authenticates this base before virtual geometry reads.
pub const SECONDARY_INTERFACE: usize = 0x08;
/// CStandardlistbox child-list head pointer +0x90; old standard_list.rs:16 and four production readers agree.
pub const CHILD_HEAD: usize = 0x90;
/// CStandardlistbox child count i32 +0xa0; old standard_list.rs:17 and four production readers agree.
pub const CHILD_COUNT: usize = 0xa0;
/// CStandardlistbox CExtendedScrollbar pointer +0x190; old standard_list.rs:18 authenticates all three scrollbar vtables.
pub const SCROLLBAR: usize = 0x190;
/// CStandardlistbox allocation size; old A28/FACILITY_SPECIALIZATION_SMOOTH_LIST_HOOK_CONTRACT_20260719.json records destructor-derived size.
pub const SIZE: usize = 0x258;
/// CStandardlistbox row spacing candidate +0x170; old facility dossier lists this field, but its pointer basis is unconfirmed.
pub const ROW_SPACING_CANDIDATE: usize = 0x170;
/// CStandardlistbox axis selector candidate +0x184; old facility dossier lists this field, but its pointer basis is unconfirmed.
pub const AXIS_SELECTOR_CANDIDATE: usize = 0x184;
/// CStandardlistbox old head candidate +0x88; facility dossier conflicts with production +0x90 and did not prove the object join.
pub const DOSSIER_HEAD_CANDIDATE: usize = 0x88;
/// CStandardlistbox old count candidate +0x98; facility dossier conflicts with production +0xa0 and did not prove the object join.
pub const DOSSIER_COUNT_CANDIDATE: usize = 0x98;
/// CStandardlistbox old scrollbar candidate +0x188; facility dossier conflicts with production +0x190 and did not prove the object join.
pub const DOSSIER_SCROLLBAR_CANDIDATE: usize = 0x188;
/// CStandardlistbox secondary-interface position virtual slot; old a729_facility_smooth_list_hook.rs calls +0xa0 for two i32 values.
pub const POSITION_SLOT: usize = 0xa0;
/// CStandardlistbox secondary-interface viewport virtual slot; old a729_facility_smooth_list_hook.rs calls +0x30 for two i32 values.
pub const VIEWPORT_SLOT: usize = 0x30;
/// CStandardlistbox secondary-interface content virtual slot; old a729_facility_smooth_list_hook.rs calls +0xe0 for two i32 values.
pub const CONTENT_SLOT: usize = 0xe0;
/// CStandardlistbox content-extent function; old facility dossier identifies FUN142295e90, conditional on the list identity gate.
pub const CONTENT_EXTENT_RVA: usize = 0x0229_5e90;
/// CStandardlistbox primary-interface append virtual slot; retained FUN141c9d840 invokes +0x20 on the unadjusted list pointer with row and flag 1.
pub const APPEND_SLOT: usize = 0x20;
