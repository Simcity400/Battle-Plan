use super::label_reads::VICTORY_POINT;
use super::unit_rules::DEFENDING;
use std::collections::BTreeMap;
pub const STACK_KINDS: u32 = 0x27;

pub fn stack_anchor(
    anchors: &BTreeMap<(u32, u32), [f32; 3]>,
    defaults: &BTreeMap<u32, [f32; 3]>,
    province: u32,
    kind: u32,
) -> Option<[f32; 3]> {
    if kind >= STACK_KINDS {
        return None;
    }
    anchors
        .get(&(province, kind))
        .or_else(|| {
            (kind == VICTORY_POINT)
                .then(|| anchors.get(&(province, DEFENDING)))
                .flatten()
        })
        .or_else(|| defaults.get(&province))
        .copied()
}
