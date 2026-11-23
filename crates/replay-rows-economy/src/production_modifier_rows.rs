use super::*;
use replay_rows::value::value_struct;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProductionModifierTree {
    pub key: u32,
    pub nodes: Vec<ModifierBreakdownNode>,
    pub failure: Vec<u8>,
}
value_struct!(ProductionModifierTree { key: u32, nodes: Vec<ModifierBreakdownNode>, failure: Vec<u8> });
impl OpaqueValue for ProductionModifierTree {}
