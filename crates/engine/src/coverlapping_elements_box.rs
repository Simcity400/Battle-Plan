//! COverlappingElementsBox presentation layout; archive/pre-rewrite-2026-09-09 dynamic-collection lane.

/// COverlappingElementsBox layout function FUN142372190; old a729_named_overlap_hook.rs:24 identifies and sentinel-gates it.
pub const LAYOUT_RVA: usize = 0x0237_2190;
/// COverlappingElementsBox layout entry bytes; old a729_named_overlap_hook.rs:76-78 checks this exact-build sentinel.
pub const LAYOUT_SENTINEL: [u8; 16] = [
    0x41, 0x56, 0x48, 0x81, 0xec, 0x80, 0x00, 0x00, 0x00, 0x48, 0x8b, 0x01, 0x4c, 0x8b, 0xf1, 0xff,
];
/// COverlappingElementsBox list interface subobject +0xa8; old a729_named_overlap_hook.rs:670 invokes its child-list accessor.
pub const LIST_INTERFACE: usize = 0xa8;
/// COverlappingElementsBox list-interface accessor virtual slot +0x58; old a729_named_overlap_hook.rs:674 obtains its child list here.
pub const CHILD_LIST_SLOT: usize = 0x58;
/// COverlappingElementsBox reverse-order byte +0x13c; old a729_named_overlap_hook.rs:688 selects head/next or tail/previous traversal.
pub const REVERSED: usize = 0x13c;
