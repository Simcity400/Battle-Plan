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
|key, input| Ok(Self { kind: (key >> 64) as u8, owner: key as u64, nodes: read_value(Kind::Opaque, input)? }),
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
|key, input| Ok(Self { record: key as u32, sources: read_value(Kind::Opaque, input)?, daily: read_value(Kind::Opaque, input)?, weekly: read_value(Kind::Opaque, input)? }),
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
