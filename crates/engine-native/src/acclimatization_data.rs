//! Embedded division acclimatization state; hourly consistency is unverified.

use engine_sites::Rva;
/// Embedded acclimatization object vtable.
pub const VTABLE_RVA: Rva = Rva(0x0285_bba0);
/// Category/weight vector data.
pub const DATA: usize = 0x08;
/// Vector capacity, u32.
pub const CAPACITY: usize = 0x10;
/// Vector count, u32.
pub const COUNT: usize = 0x14;
/// Maximum, i64 raw value.
pub const MAXIMUM: usize = 0x38;
/// The category whose climate the division stands in now: the pointer the
/// movement availability calculation compares the winning entry's category with
/// (null: no climate, factor one).
pub const ACTIVE: usize = 0x20;
/// An acclimatization category's factor, i64 fixed5, that the movement
/// availability calculation scales the winning weight's share by.
pub const CATEGORY_FACTOR: usize = 0x30;
/// `NDefines::NGraphics::ACCLIMATIZATION_CAMO_SHOW_AT`, a signed fixed5 word
/// loaded from the content's defines and multiplied by the maximum for the
/// camouflage threshold.
pub const CAMO_SHOW_AT_RVA: Rva = Rva(0x033e_1658);

pub use game_model::acclimatization::*;
