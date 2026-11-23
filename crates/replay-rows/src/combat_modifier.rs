use super::*;
use crate::value::value_struct;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CombatModifierValue {
    pub id: u32,
    pub value_a: i64,
    pub value_b: i64,
}
value_struct!(CombatModifierValue {
    id: u32,
    value_a: i64,
    value_b: i64
});

impl OpaqueValue for CombatModifierValue {}
