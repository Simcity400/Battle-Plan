//! Battle-plan save fields; spelling is pinned by token registration and serializers.
use super::{
    BTreeMap, BTreeSet, Item, block, boolean, bytes, field, integer, integer_list, invalid,
    object_id, required_bytes, scalar, typed_handle,
};
use std::io;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Order {
    pub front: Option<u64>,
    pub section: u32,
    pub segment_start: Option<i32>,
    pub segment_end: Option<i32>,
    pub blitz: bool,
    pub withdraw: bool,
    pub fallback: bool,
    pub virtual_order: bool,
    pub has_child: Option<bool>,
    pub owner_override: u64,
    pub assigned: Vec<u64>,
    /// `type`: the order kind.
    pub order_type: i32,
    /// Ordinary successors in native order (`order_children`, token 0x30B3,
    /// repeated scalars from `COrderInstance::serialize@1410362C0`).
    pub successors: Vec<u32>,
    /// The order-name strings in `ArmyOrderPlanRow` order: prefix, name,
    /// first, second, unique, postfix. An absent key is an empty string.
    pub names: [Vec<u8>; 6],
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Front {
    pub section_ids: Vec<u32>,
    pub section_ends: Vec<u32>,
    pub pairs: Vec<u32>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tables {
    pub orders: BTreeMap<(u64, u32), Order>,
    pub fronts: BTreeMap<u64, Front>,
    pub colors: BTreeMap<u64, Option<Vec<u32>>>,
    /// Each plan group's saved `name`.
    pub group_names: BTreeMap<u64, Vec<u8>>,
    /// Each group's saved `order_instance` then `fallback` instance ids, in
    /// serializer order.
    pub group_orders: BTreeMap<u64, [Vec<u32>; 2]>,
}
fn flag(items: &[Item], key: &[u8]) -> io::Result<bool> {
    Ok(field(items, key)?
        .map(boolean)
        .transpose()?
        .unwrap_or(false))
}
fn optional_int(items: &[Item], key: &[u8]) -> io::Result<Option<i32>> {
    field(items, key)?.map(|v| integer(scalar(v)?)).transpose()
}
fn repeated_ids(items: &[Item], key: &[u8]) -> io::Result<Vec<u32>> {
    items
        .iter()
        .filter(|item| item.key.as_deref() == Some(key))
        .map(|item| integer(scalar(&item.value)?))
        .collect()
}
/// writes each only when its string is nonempty: `COrderInstance+0x1A0` as
/// `prefix` (token 0x30F8), `+0x120` as `operation` (0x2F1B), `+0x140` as
/// `first` (0x28DE), `+0x160` as `second` (0x28DF), `+0x180` as `unique`
/// (0x30FA, not in checksum mode) and `+0x1C0` as `postfix` (0x4A69).
const NAME_KEYS: [&[u8]; 6] = [
    b"prefix",
    b"operation",
    b"first",
    b"second",
    b"unique",
    b"postfix",
];

impl Tables {
    pub(super) fn country(&mut self, country: &[Item]) -> io::Result<()> {
        let Some(theatres) = field(country, b"theatres")? else {
            return Ok(());
        };
        for theatre in block(theatres)?
            .iter()
            .filter(|i| i.key.as_deref() == Some(b"theatre"))
        {
            for item in block(&theatre.value)? {
                match item.key.as_deref() {
                    Some(b"front") => {
                        let items = block(&item.value)?;
                        let handle = object_id(items)?;
                        if handle as u32 != 66 {
                            return Err(invalid("saved front has wrong reference type"));
                        }
                        let mut front = Front::default();
                        let mut ids = BTreeSet::new();
                        for section in items
                            .iter()
                            .filter(|i| i.key.as_deref() == Some(b"section"))
                        {
                            let section = block(&section.value)?;
                            let id = integer::<i32>(&required_bytes(section, b"id")?)? as u32;
                            if !ids.insert(id) {
                                return Err(invalid("duplicate saved front section"));
                            }
                            let pairs = integer_list(section, b"sorted_pairs")?;
                            if !pairs.len().is_multiple_of(2) {
                                return Err(invalid("saved front has incomplete province pair"));
                            }
                            front.section_ids.push(id);
                            front.pairs.extend(pairs);
                            front.section_ends.push(
                                u32::try_from(front.pairs.len())
                                    .map_err(|_| invalid("saved front pair count overflow"))?,
                            );
                        }
                        if self.fronts.insert(handle, front).is_some() {
                            return Err(invalid("duplicate saved front reference"));
                        }
                    }
                    Some(b"orders_group" | b"field_marshal_group") => {
                        let group = block(&item.value)?;
                        let handle = object_id(group)?;
                        let color = field(group, b"color")?
                            .map(|_| integer_list(group, b"color"))
                            .transpose()?;
                        if color
                            .as_ref()
                            .is_some_and(|v| v.len() != 3 || v.iter().any(|v| *v > 255))
                        {
                            return Err(invalid("saved group RGB color invalid"));
                        }
                        if self.colors.insert(handle, color).is_some() {
                            return Err(invalid("duplicate saved plan group"));
                        }
                        self.group_names
                            .insert(handle, bytes(group, b"name")?.unwrap_or_default());
                        for item in group.iter().filter(|i| {
                            matches!(
                                i.key.as_deref(),
                                Some(b"order_instance" | b"fallback" | b"virtual_fallback")
                            )
                        }) {
                            let items = block(&item.value)?;
                            let instance = integer(&required_bytes(items, b"instance_id")?)?;
                            let list = match item.key.as_deref() {
                                Some(b"order_instance") => Some(0),
                                Some(b"fallback") => Some(1),
                                _ => None,
                            };
                            if let Some(list) = list {
                                self.group_orders.entry(handle).or_default()[list].push(instance);
                            }
                            let assigned = items
                                .iter()
                                .filter(|i| i.key.as_deref() == Some(b"scheduled_member"))
                                .map(|i| typed_handle(&i.value))
                                .collect::<io::Result<_>>()?;
                            let order = Order {
                                front: field(items, b"root_front")?
                                    .map(typed_handle)
                                    .transpose()?,
                                section: optional_int(items, b"root_section")?.unwrap_or(0) as u32,
                                segment_start: optional_int(items, b"sorted_pairs_from")?,
                                segment_end: optional_int(items, b"sorted_pairs_to")?,
                                blitz: flag(items, b"blitz")?,
                                withdraw: flag(items, b"withdraw")?,
                                fallback: flag(items, b"fallback")?,
                                virtual_order: flag(items, b"virtual_order")?,
                                has_child: field(items, b"manage_child_sections")?
                                    .map(boolean)
                                    .transpose()?,
                                owner_override: field(items, b"owner")?
                                    .map(typed_handle)
                                    .transpose()?
                                    .unwrap_or(0),
                                assigned,
                                order_type: optional_int(items, b"type")?.unwrap_or(0),
                                successors: repeated_ids(items, b"order_children")?,
                                names: {
                                    let mut names: [Vec<u8>; 6] = Default::default();
                                    for (name, key) in names.iter_mut().zip(NAME_KEYS) {
                                        *name = bytes(items, key)?.unwrap_or_default();
                                    }
                                    names
                                },
                            };
                            if self.orders.insert((handle, instance), order).is_some() {
                                return Err(invalid("duplicate saved plan order"));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(body: &str) -> io::Result<Tables> {
        let parsed = crate::parser::parse(body.as_bytes())?;
        let mut tables = Tables::default();
        tables.country(&parsed.items)?;
        Ok(tables)
    }
    #[test]
    fn plans_keep_scoped_orders_native_list_order_and_empty_front_sections() {
        let tables = parse("theatres={theatre={front={id={id=9 type=66} section={id=4 sorted_pairs={1 2 1 3}} section={id=2 sorted_pairs={}}} orders_group={id={id=8 type=53} color={127 0 255} order_instance={instance_id=3 root_front={id=9 type=66} root_section=4 sorted_pairs_from=-1 sorted_pairs_to=1 blitz=yes withdraw=no virtual_order=yes fallback=yes manage_child_sections=no scheduled_member={id=4294967295 type=51} scheduled_member={id=5 type=51}}}}}").unwrap();
        let row = &tables.orders[&((8 << 32) | 53, 3)];
        assert_eq!(
            row.assigned,
            vec![(u64::from(u32::MAX) << 32) | 51, (5 << 32) | 51]
        );
        assert!(row.blitz && row.virtual_order && row.fallback && !row.withdraw);
        assert_eq!((row.has_child, row.segment_start), (Some(false), Some(-1)));
        let front = &tables.fronts[&((9 << 32) | 66)];
        assert_eq!(front.section_ids, vec![4, 2]);
        assert_eq!(front.section_ends, vec![4, 4]);
        assert_eq!(front.pairs, vec![1, 2, 1, 3]);
    }
    #[test]
    fn order_names_read_the_tokens_the_serializer_writes() {
        // W2: `+0x120` is saved as `operation`, not `name`.
        let tables = parse("theatres={theatre={orders_group={id={id=8 type=53} order_instance={instance_id=3 operation=\"o_phalanx\" prefix=\"o_operation\"} order_instance={instance_id=4 first=\"o_red\" second=\"o_go\" unique=\"Riga\" postfix=\"o_offensive\"}}}}").unwrap();
        let named = &tables.orders[&((8 << 32) | 53, 3)];
        assert_eq!(named.names[0], b"o_operation");
        assert_eq!(named.names[1], b"o_phalanx");
        assert!(named.names[2..].iter().all(Vec::is_empty));
        let paired = &tables.orders[&((8 << 32) | 53, 4)];
        assert!(paired.names[0].is_empty() && paired.names[1].is_empty());
        assert_eq!(
            paired.names[2..],
            [
                b"o_red".to_vec(),
                b"o_go".to_vec(),
                b"Riga".to_vec(),
                b"o_offensive".to_vec()
            ]
        );
    }
    #[test]
    fn malformed_pairs_duplicate_sections_and_invalid_flags_are_refused() {
        for body in [
            "front={id={id=9 type=66} section={id=1 sorted_pairs={1}}}",
            "front={id={id=9 type=66} section={id=1} section={id=1}}",
            "orders_group={id={id=8 type=53} order_instance={instance_id=1 blitz=maybe}}",
            "orders_group={id={id=8 type=53} order_instance={instance_id=1 blitz=yes blitz=no}}",
        ] {
            assert!(parse(&format!("theatres={{theatre={{{body}}}}}")).is_err());
        }
    }
}
