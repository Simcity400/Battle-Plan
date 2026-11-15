//! Synthetic historical values shared with recording integration fixtures.
use crate::{rows::CombatModifierValue, value::value_struct};

/// One element of the side list LandCombatModifiers held before
/// LAND_MODIFIER_ROWS_FORMAT, as its writers encoded it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OldDivision {
    pub division: u64,
    pub ordinal: u32,
    pub entries: Vec<CombatModifierValue>,
}
value_struct!(OldDivision { division: u64, ordinal: u32, entries: Vec<CombatModifierValue> });
