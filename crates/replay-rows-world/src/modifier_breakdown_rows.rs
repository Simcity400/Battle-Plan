use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PowerBalanceModifierBreakdownRow {
    pub record: u32,
    pub sources: Vec<Vec<u8>>,
    pub daily: Vec<ModifierBreakdownNode>,
    pub weekly: Vec<ModifierBreakdownNode>,
}
typed_codec!(PowerBalanceModifierBreakdownRow, 32, |row| row.record as u128,
|key| Self { record: key as u32, ..Default::default() },
{ 0 => (sources): Opaque, 1 => (daily): Opaque, 2 => (weekly): Opaque }, |row| {
   if row.daily.is_empty() != row.weekly.is_empty() {
       return Err(invalid("power balance witness trees are read together"));
   }
   if row.daily.is_empty() {
       return Ok(());
   }
   check_tree(&row.daily)?;
   check_tree(&row.weekly)
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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

fn check_naval_trees(kind: u8, trees: &[NavalModifierKeyTree]) -> io::Result<()> {
    if game_model::naval_modifier_trees::Kind::from_u8(kind).is_none() {
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
