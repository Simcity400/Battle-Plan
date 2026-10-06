use super::*;
use crate::value::value_struct;
pub use engine::modifier_breakdown::Node as ModifierBreakdownNode;
value_struct!(ModifierBreakdownNode { children: u32, value: i64, name: Vec<u8> });
opaque_value!(Vec<ModifierBreakdownNode>);
opaque_value!(Vec<Vec<u8>>);

/// Country (kind 0) or selected commander (kind 1/2) reserve-rate tree.
/// Country owners are raw links; commander owners are full handles.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReserveModifierBreakdownRow {
    pub kind: u8,
    pub owner: u64,
    pub nodes: Vec<ModifierBreakdownNode>,
}
typed_codec!(ReserveModifierBreakdownRow, 72, |row| (row.kind as u128) << 64 | row.owner as u128,
|key| Self { kind: (key >> 64) as u8, owner: key as u64, ..Default::default() },
{ 0 => (nodes): Opaque }, |row| {
   if row.kind > 2 || (row.kind == 0 && row.owner > u32::MAX as u64) {
       return Err(invalid("invalid reserve modifier source identity"));
   }
   check_tree(&row.nodes)
});

/// Cached balance description values and the ordered static-modifier keys.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PowerBalanceModifierBreakdownRow {
    pub record: u32,
    pub sources: Vec<Vec<u8>>,
    pub daily: Vec<ModifierBreakdownNode>,
    pub weekly: Vec<ModifierBreakdownNode>,
}
typed_codec!(PowerBalanceModifierBreakdownRow, 32, |row| row.record as u128,
|key| Self { record: key as u32, ..Default::default() },
{ 0 => (sources): Opaque, 1 => (daily): Opaque, 2 => (weekly): Opaque }, |row| {
   check_tree(&row.daily)?;
   check_tree(&row.weekly)
});

/// A single complete preorder tree, without a chosen size/depth limit.
pub fn check_tree(nodes: &[ModifierBreakdownNode]) -> io::Result<()> {
    let mut pending = 1_usize;
    for node in nodes {
        if pending == 0 {
            return Err(invalid("modifier breakdown has extra roots"));
        }
        pending = (pending - 1)
            .checked_add(node.children as usize)
            .ok_or_else(|| invalid("modifier breakdown child count overflow"))?;
    }
    if pending != 0 {
        return Err(invalid("modifier breakdown is incomplete"));
    }
    Ok(())
}

/// A native source occurrence. `battle_slot == u32::MAX` is a side source;
/// otherwise it joins NavalCombatShips by combat/side/slot. Source scope/index
/// identify the ordered adjuster occurrence selected by 140C230A0/1415B4BB0.
/// Order restores their traversal order after accumulator/key identity sorting.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalModifierUse {
    pub combat: u64,
    pub side: u8,
    pub battle_slot: u32,
    pub source_scope: u8,
    pub source_index: u32,
    pub order: u32,
}
value_struct!(NavalModifierUse {
    combat: u64,
    side: u8,
    battle_slot: u32,
    source_scope: u8,
    source_index: u32,
    order: u32
});
opaque_value!(Vec<NavalModifierUse>);

/// One tree per stable native accumulator identity and key. A repeated
/// source or combat adds a small use, not another tree. The node field has
/// its own delta bit, so unchanged trees carry across hours when uses change.
/// Nodes preserve 14054D0F0's depth -1 preorder after zero-subtree pruning;
/// the root value survives because convoy escape reads it (1415B4BB0).
/// Names are native display bytes and require the recording language when
/// shown; the viewer formats contributions through its modifier definitions.
/// The runtime accumulator identity and tree have no text-save counterpart.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalModifierBreakdownRow {
    pub accumulator: u64,
    pub key: u32,
    pub uses: Vec<NavalModifierUse>,
    pub nodes: Vec<ModifierBreakdownNode>,
}
typed_codec!(NavalModifierBreakdownRow, 96,
|row| (row.accumulator as u128) << 32 | row.key as u128,
|key| Self { accumulator: (key >> 32) as u64, key: key as u32, ..Default::default() },
{ 0 => (uses): Opaque, 1 => (nodes): Opaque },
|row| {
    if row.uses.is_empty()
        || row.uses.iter().any(|usage| usage.side > 1)
    { return Err(invalid("invalid naval modifier source identity")); }
    check_tree(&row.nodes)
});

#[cfg(test)]
mod naval_tests {
    use super::*;

    #[test]
    fn stable_accumulator_key_carries_unchanged_tree_when_uses_change() {
        let first = NavalModifierBreakdownRow {
            accumulator: 0x1234,
            key: 0x1b2,
            uses: vec![NavalModifierUse {
                combat: 1,
                side: 0,
                battle_slot: u32::MAX,
                source_scope: 0,
                order: 0,
                ..Default::default()
            }],
            nodes: vec![ModifierBreakdownNode {
                children: 0,
                value: 1000,
                name: b"doctrine".to_vec(),
            }],
        };
        let mut next = first.clone();
        next.uses[0].combat = 2;
        assert_eq!(next.key(), first.key());
        let delta = next.delta(Some(&first)).unwrap().unwrap();
        assert_eq!(delta.mask, 1);
        assert_eq!(
            delta
                .apply::<NavalModifierBreakdownRow>(Some(first))
                .unwrap(),
            next
        );
    }
}
