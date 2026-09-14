//! CStaticModifier, the derived definition consumed by modifier composition.

/// CStaticModifier vtable; pinned constructor 140549860 calls CModifier's
/// 1401496A0 constructor, then instruction 140549887 installs 142702620.
pub const VTABLE_RVA: usize = 0x0270_2620;
/// Inherited accumulator at +10; constructor 1401496A0 initializes it through
/// 140549470, and static-source composition 14054AD00 iterates source+10/+1C.
pub const ACCUMULATOR: usize = 0x10;
