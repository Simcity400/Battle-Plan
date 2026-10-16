use super::*;
use crate::value::value_struct;
pub use engine::modifier_breakdown::Node as ModifierBreakdownNode;
value_struct!(ModifierBreakdownNode { children: u32, value: i64, name: Vec<u8> });
opaque_value!(Vec<ModifierBreakdownNode>);
opaque_value!(Option<Vec<ModifierBreakdownNode>>);
opaque_value!(Vec<Vec<u8>>);

/// Retired: the reserve reinforcement hover's country (kind 0) or selected
/// commander (kind 1/2) reserve-rate tree. Kept for older recordings' framing.
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

/// The reinforcement-rate sources a reserve's chance arm prints: a country's
/// (`kind` 0, `owner` its raw reference) or a contextual commander's selected
/// unit modifier (`kind` the division's `leader_modifier_kind`, 1 or 2;
/// `owner` the division's commander handle). `nodes` is the printed tree of
/// key 0xFB ([`printed_tree`]); `name` is the commander's display name, read
/// only when its root is nonzero, the one case the game prints the commander
/// line. A commander whose root is zero has no row. `nodes` is `None` when
/// that source's read was refused at capture (counted in collector status
/// `land_reserve_speed`); the other rows keep theirs. Display bytes are native
/// stored text in the recording language. Rows exist only for the owners of
/// the hour's reserves.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReserveModifierTreeRow {
    pub kind: u8,
    pub owner: u64,
    pub name: Vec<u8>,
    pub nodes: Option<Vec<ModifierBreakdownNode>>,
}
typed_codec!(ReserveModifierTreeRow, 72, |row| (row.kind as u128) << 64 | row.owner as u128,
|key| Self { kind: (key >> 64) as u8, owner: key as u64, ..Default::default() },
{ 0 => (name): Bytes, 1 => (nodes): Opaque }, |row| {
   if row.kind > 2 || (row.kind == 0 && row.owner > u32::MAX as u64) {
       return Err(invalid("invalid reserve modifier source identity"));
   }
   if (row.kind == 0 || row.nodes.is_none()) && !row.name.is_empty() {
       return Err(invalid("only a read commander reserve source has a name"));
   }
   match &row.nodes {
       Some(nodes) => check_tree(nodes),
       None => Ok(()),
   }
});

/// The tree a depth -1 modifier breakdown prints from `nodes` (a complete
/// preorder): the root, every nonzero leaf with its display bytes and the
/// interior nodes above them, interior nodes without bytes. A root whose
/// children all pruned keeps one zero child, so it stays interior and prints
/// nothing, as the native walk does; a childless root keeps its own value
/// and bytes.
pub fn printed_tree(nodes: &[ModifierBreakdownNode]) -> io::Result<Vec<ModifierBreakdownNode>> {
    check_tree(nodes)?;
    let mut output = Vec::new();
    let mut cursor = 0;
    copy_printed(nodes, &mut cursor, true, &mut output)?;
    let root = &nodes[0];
    if root.children != 0 && output[0].children == 0 {
        output[0].children = 1;
        engine::allocation::reserve(&mut output, 1)?;
        output.push(ModifierBreakdownNode::default());
    }
    Ok(output)
}

fn copy_printed(
    nodes: &[ModifierBreakdownNode],
    cursor: &mut usize,
    root: bool,
    output: &mut Vec<ModifierBreakdownNode>,
) -> io::Result<bool> {
    let node = &nodes[*cursor];
    *cursor += 1;
    let start = output.len();
    engine::allocation::reserve(output, 1)?;
    output.push(ModifierBreakdownNode::default());
    let mut kept = 0_u32;
    for _ in 0..node.children {
        if copy_printed(nodes, cursor, false, output)? {
            kept += 1;
        }
    }
    if !root && (node.children != 0 || node.value == 0) && kept == 0 {
        output.truncate(start);
        return Ok(false);
    }
    // Only an original leaf prints; interior nodes keep no display bytes.
    let name = if node.children == 0 && node.value != 0 {
        engine::allocation::copy_vec(&node.name)?
    } else {
        Vec::new()
    };
    output[start] = ModifierBreakdownNode {
        children: kept,
        value: node.value,
        name,
    };
    Ok(true)
}

/// One power-balance record's (`PowerBalances.ordinal`) static-modifier sources: the complete
/// source roster in array order, every occurrence and empty sources included, by `static modifier`
/// script name. The cached description tree is built from these alone by rebuilds and additions, so
/// playback derives it with the content's static modifiers ([`crate::power_balance_derive`]).
///
/// `daily` and `weekly` are the native cached trees for keys 0x98/0x99 read
/// as witnesses: every row of recordings made before playback derived them,
/// and only the rows a diagnostic `breakdown_witness` collector build sampled
/// since. Empty means not read; current production writers leave both
/// empty. `recorder derived` compares each witness with the derivation;
/// consumers never display them.
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
   // A witness is a complete tree; both are read together or not at all.
   if row.daily.is_empty() != row.weekly.is_empty() {
       return Err(invalid("power balance witness trees are read together"));
   }
   if row.daily.is_empty() {
       return Ok(());
   }
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

/// A source occurrence. `battle_slot == u32::MAX` is a side source; otherwise
/// it joins NavalCombatShips by combat/side/slot. Source scope/index identify
/// the selected adjuster occurrence. Order restores traversal order after
/// accumulator/key identity sorting.
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

/// Retired: the naval combat hovers' modifier trees. Kept for older
/// recordings' framing.
///
/// One tree per stable native accumulator identity and key. A repeated
/// source or combat adds a small use, not another tree. The node field has
/// its own delta bit, so unchanged trees carry across hours when uses change.
/// Nodes preserve depth -1 preorder after zero-subtree pruning; the root
/// value survives because convoy escape reads it.
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

/// One key's tree of a naval hover source ([`NavalModifierTreeRow`]): the preorder the
/// modifier-breakdown formatter walks at depth -1 for that key, pruned to the root, the nonzero
/// leaves and the interior nodes above them. The root keeps its value (convoy escape and the side
/// sums read it); a root whose native children all pruned keeps one zero child, so it stays
/// interior and prints nothing, as the native walk does. Only leaves keep their display bytes; a
/// key whose root is zero and prints nothing is absent.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalModifierKeyTree {
    pub key: u32,
    pub nodes: Vec<ModifierBreakdownNode>,
}
value_struct!(NavalModifierKeyTree {
    key: u32,
    nodes: Vec<ModifierBreakdownNode>
});
opaque_value!(Vec<NavalModifierKeyTree>);

/// The hover trees of one naval modifier source accumulator: the side sources used for
/// positioning, screening and convoy escape, and the ship sources used for ship escape
/// and carrier sortie, each for every key in [`crate::naval_trees::KEYS`] that prints or
/// has a nonzero root. `kind` and `owner`/`sub` name the source by recorded identities
/// ([`crate::naval_trees`]), so a combat, side or ship reaches its trees by joining what
/// it already records. Rows exist only for sources a live combat reaches this hour; a
/// reached source without a row prints nothing and has a zero root for every key. Display
/// bytes are native stored text in the recording language.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalModifierTreeRow {
    pub kind: u8,
    pub owner: u64,
    pub sub: u32,
    pub trees: Vec<NavalModifierKeyTree>,
}
typed_codec!(NavalModifierTreeRow, 104,
|row| (row.kind as u128) << 96 | (row.owner as u128) << 32 | row.sub as u128,
|key| Self { kind: (key >> 96) as u8, owner: (key >> 32) as u64, sub: key as u32, ..Default::default() },
{ 0 => (trees): Opaque },
|row| check_naval_trees(row.kind, &row.trees));

/// Diagnostic oracle rows (collector feature `naval_tree_oracle`): the fresh
/// native trees of one source occurrence of a live combat, read at the hour
/// through the source's own accumulator. `battle_slot` is `u32::MAX` for a
/// side source, whose `ordinal` is its [`crate::naval_trees::SideSource`];
/// a ship source's `ordinal` is its position in
/// `NavalCombatShips.weapon_context.sources`. `kind`/`owner`/`sub` are the
/// identity the capture filed the source's [`NavalModifierTreeRow`] under.
/// Production builds take no sample and record the table empty.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalModifierTreeSampleRow {
    pub combat: u64,
    pub side: u8,
    pub battle_slot: u32,
    pub ordinal: u16,
    pub scope: u8,
    pub kind: u8,
    pub owner: u64,
    pub sub: u32,
    pub trees: Vec<NavalModifierKeyTree>,
}
typed_codec!(NavalModifierTreeSampleRow, 120,
|row| (row.combat as u128) << 56 | (row.side as u128) << 48 | (row.battle_slot as u128) << 16 | row.ordinal as u128,
|key| Self { combat: (key >> 56) as u64, side: (key >> 48) as u8, battle_slot: (key >> 16) as u32, ordinal: key as u16, ..Default::default() },
{ 0 => (scope): U8, 1 => (kind): U8, 2 => (owner): U64, 3 => (sub): U32, 4 => (trees): Opaque },
|row| {
    if row.side > 1 { return Err(invalid("invalid naval tree sample side")); }
    check_naval_trees(row.kind, &row.trees)
});

/// Known kind, strictly ascending keys, and one complete nonempty preorder
/// tree per key.
fn check_naval_trees(kind: u8, trees: &[NavalModifierKeyTree]) -> io::Result<()> {
    if crate::naval_trees::Kind::from_u8(kind).is_none() {
        return Err(invalid("unknown naval modifier source kind"));
    }
    let mut previous = None;
    for tree in trees {
        if previous.is_some_and(|key| key >= tree.key) {
            return Err(invalid("naval modifier tree keys are not ascending"));
        }
        previous = Some(tree.key);
        if tree.nodes.is_empty() {
            return Err(invalid("naval modifier key tree is empty"));
        }
        check_tree(&tree.nodes)?;
    }
    Ok(())
}

/// One fresh attrition formatter tree per native accumulator/key. The ordered
/// per-division uses, depth and scaling are in ArmyAttritionInputs.sources.
/// None is a counted root-local capture failure; it leaves other trees and all
/// independent scalar terms available. Native display names require the
/// recording language; runtime identities have no text-save counterpart.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AttritionModifierBreakdownRow {
    pub accumulator: u64,
    pub key: u32,
    pub nodes: Option<Vec<ModifierBreakdownNode>>,
}
typed_codec!(AttritionModifierBreakdownRow, 96,
|row| (row.accumulator as u128) << 32 | row.key as u128,
|key| Self { accumulator: (key >> 32) as u64, key: key as u32, ..Default::default() },
{ 0 => (nodes): Opaque }, |row| {
    if let Some(nodes) = &row.nodes { check_tree(nodes)?; }
    Ok(())
});

#[cfg(test)]
mod balance_tests {
    use super::*;

    /// Production rows carry sources alone; a witness carries both complete
    /// trees, as every row of older recordings does.
    #[test]
    fn witness_trees_are_both_complete_or_both_absent() {
        let row = |daily: Vec<ModifierBreakdownNode>, weekly: Vec<ModifierBreakdownNode>| {
            PowerBalanceModifierBreakdownRow {
                record: 1,
                sources: vec![b"source".to_vec()],
                daily,
                weekly,
            }
        };
        let root = vec![ModifierBreakdownNode::default()];
        let incomplete = vec![ModifierBreakdownNode {
            children: 1,
            ..Default::default()
        }];
        assert!(row(Vec::new(), Vec::new()).check_key_fields().is_ok());
        assert!(row(root.clone(), root.clone()).check_key_fields().is_ok());
        assert!(row(root.clone(), Vec::new()).check_key_fields().is_err());
        assert!(row(Vec::new(), root.clone()).check_key_fields().is_err());
        assert!(row(incomplete, root).check_key_fields().is_err());
    }
}

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

#[cfg(test)]
mod printed_tests {
    use super::*;

    fn node(children: u32, value: i64, name: &[u8]) -> ModifierBreakdownNode {
        ModifierBreakdownNode {
            children,
            value,
            name: name.to_vec(),
        }
    }

    /// The printed tree keeps the root, the nonzero leaves with their bytes
    /// and the interiors above them, and prints exactly what the full tree
    /// prints at depth -1.
    #[test]
    fn printed_trees_keep_what_a_depth_minus_one_breakdown_prints() {
        let full = [
            node(4, 99_000, b"aggregate"),
            node(0, 5_000, b"First"),
            node(0, 0, b"Zero"),
            node(2, 25_000, b"Nested aggregate"),
            node(0, 0, b"Nested zero"),
            node(0, -1_000, b"Nested"),
            node(1, 7, b"Empty aggregate"),
            node(0, 0, b"Inner zero"),
        ];
        assert_eq!(
            printed_tree(&full).unwrap(),
            [
                node(2, 99_000, b""),
                node(0, 5_000, b"First"),
                node(1, 25_000, b""),
                node(0, -1_000, b"Nested"),
            ]
        );
        // A root whose children all prune keeps one zero child; a childless
        // root keeps its value and bytes.
        assert_eq!(
            printed_tree(&[node(1, 3, b"root"), node(0, 0, b"zero")]).unwrap(),
            [node(1, 3, b""), node(0, 0, b"")]
        );
        assert_eq!(
            printed_tree(&[node(0, 3, b"root")]).unwrap(),
            [node(0, 3, b"root")]
        );
        assert!(printed_tree(&[node(1, 3, b"root")]).is_err());
    }

    #[test]
    fn reserve_sources_name_only_commanders() {
        let row = |kind: u8, owner: u64, name: &[u8]| ReserveModifierTreeRow {
            kind,
            owner,
            name: name.to_vec(),
            nodes: Some(vec![node(0, 1, b"x")]),
        };
        assert!(row(0, 7, b"").check_key_fields().is_ok());
        let refused = ReserveModifierTreeRow {
            nodes: None,
            ..row(1, 7, b"")
        };
        assert!(refused.check_key_fields().is_ok());
        assert!(
            ReserveModifierTreeRow {
                name: b"Leader".to_vec(),
                ..refused
            }
            .check_key_fields()
            .is_err()
        );
        assert!(row(1, u64::MAX, b"Leader").check_key_fields().is_ok());
        assert!(row(0, 7, b"Leader").check_key_fields().is_err());
        assert!(row(0, u64::MAX, b"").check_key_fields().is_err());
        assert!(row(3, 7, b"").check_key_fields().is_err());
    }
}
