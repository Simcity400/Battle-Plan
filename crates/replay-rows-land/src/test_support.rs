use crate::CombatModifierValue;
use replay_rows::value::value_struct;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OldDivision {
    pub division: u64,
    pub ordinal: u32,
    pub entries: Vec<CombatModifierValue>,
}
value_struct!(OldDivision { division: u64, ordinal: u32, entries: Vec<CombatModifierValue> });
