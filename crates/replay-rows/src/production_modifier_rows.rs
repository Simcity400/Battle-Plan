use super::*;
use crate::value::value_struct;

/// One native formatter tree. A failed root retains its key and refusal;
/// independent roots remain usable. Names are retained display bytes and need
/// the recorded language, as for NavalModifierBreakdowns.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProductionModifierTree {
    pub key: u32,
    pub nodes: Vec<ModifierBreakdownNode>,
    pub failure: Vec<u8>,
}
value_struct!(ProductionModifierTree { key: u32, nodes: Vec<ModifierBreakdownNode>, failure: Vec<u8> });
opaque_value!(Vec<ProductionModifierTree>);
opaque_value!([i64; 7]);
