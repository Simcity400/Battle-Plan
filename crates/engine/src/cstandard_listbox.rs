//! Standard-listbox presentation layout; no hourly walker is provided.
//! Candidate offsets with unresolved pointer basis remain separate rather
//! than aliases. Primary displacement is zero and secondary displacement +8.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x02b2_cb48;
/// CStandardlistbox secondary vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x02b2_cc78;
/// CStandardlistbox secondary interface +8.
pub const SECONDARY_INTERFACE: usize = 0x08;
/// CStandardlistbox child-list head pointer +0x90.
pub const CHILD_HEAD: usize = 0x90;
/// CStandardlistbox child count i32 +0xa0.
pub const CHILD_COUNT: usize = 0xa0;
/// CStandardlistbox CExtendedScrollbar pointer +0x190.
pub const SCROLLBAR: usize = 0x190;
/// Listbox allocation size.
pub const SIZE: usize = 0x258;
/// Candidate row spacing; pointer basis unresolved.
pub const ROW_SPACING_CANDIDATE: usize = 0x170;
/// Candidate axis selector; pointer basis unresolved.
pub const AXIS_SELECTOR_CANDIDATE: usize = 0x184;
/// Candidate head field with unresolved object identity, separate from the admitted head.
pub const DOSSIER_HEAD_CANDIDATE: usize = 0x88;
/// Candidate count field with unresolved object identity, separate from the admitted count.
pub const DOSSIER_COUNT_CANDIDATE: usize = 0x98;
/// Candidate scrollbar field with unresolved object identity, separate from the admitted pointer.
pub const DOSSIER_SCROLLBAR_CANDIDATE: usize = 0x188;
/// Position virtual-dispatch byte offset returning two i32 values.
pub const POSITION_SLOT: usize = 0xa0;
/// Viewport virtual-dispatch byte offset returning two i32 values.
pub const VIEWPORT_SLOT: usize = 0x30;
/// Content virtual-dispatch byte offset returning two i32 values.
pub const CONTENT_SLOT: usize = 0xe0;
/// Content-extent callable requiring the list identity gate.
pub const CONTENT_EXTENT_RVA: usize = 0x0229_5e90;
/// Primary-interface append dispatch byte offset. The caller passes the
/// unadjusted list pointer, row and flag one.
pub const APPEND_SLOT: usize = 0x20;
