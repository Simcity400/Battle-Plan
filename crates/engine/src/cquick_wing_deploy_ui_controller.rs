//! Quick-wing-deployment UI wrapper.

/// Wrapper constructor.
pub const CONSTRUCTOR_RVA: usize = 0x01cd_b350;
/// Primary vtable.
pub const VTABLE_RVA: usize = 0x02a4_f980;
/// Selected-options query used by the row materializer.
pub const GET_OPTIONS_RVA: usize = 0x01cd_b770;
/// Underlying quick-wing-deployment builder pointer.
pub const CONTROLLER: usize = 0x10;
/// Selected bucket, i32, from zero through three.
pub const BUCKET: usize = 0x38;
