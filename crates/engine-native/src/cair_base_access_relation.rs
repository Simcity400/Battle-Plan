//! CAirBaseAccessRelation; shared fields live in `crelation`.

/// Air-base access relation vtable.
pub const VTABLE_RVA: usize = 0x2a21738;
/// Relation type word used by construction and factory dispatch.
pub const TYPE: u32 = 0x283a;
/// Constructor receiving the allocated relation object.
pub const CONSTRUCTOR_RVA: usize = 0x0198_1100;
