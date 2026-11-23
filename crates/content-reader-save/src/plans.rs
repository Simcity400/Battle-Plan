use super::{
    BTreeMap, BTreeSet, Decimal, Item, block, boolean, bytes, field, integer, integer_list,
    invalid, object_id, required_block, required_bytes, scalar, typed_handle,
};
use std::io;

const ORDERS_GROUP_REFERENCE_TYPE: u32 = 53;

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
    pub route_is_ok: bool,
    pub has_child: Option<bool>,
    pub child_fronts: Vec<ChildFront>,
    pub owner_override: u64,
    pub assigned: Vec<u64>,
    pub order_type: i32,
    pub successors: Vec<u32>,
    pub names: [Vec<u8>; 6],
    pub preparation: Option<Decimal>,
    pub invasion_transports: Option<i32>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChildFront {
    pub group: u64,
    pub segment_start: i32,
    pub segment_end: i32,
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
    pub group_names: BTreeMap<u64, Option<Vec<u8>>>,
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

fn child_fronts(items: &[Item]) -> io::Result<Vec<ChildFront>> {
    let Some(value) = field(items, b"child_front_ratios")? else {
        return Ok(Vec::new());
    };
    block(value)?
        .iter()
        .map(|entry| {
            if entry.key.is_some() {
                return Err(invalid("saved child_front_ratios entry is not anonymous"));
            }
            let fields = block(&entry.value)?;
            let group = typed_handle(
                field(fields, b"group")?.ok_or_else(|| invalid("saved child front lacks group"))?,
            )?;
            if group as u32 != ORDERS_GROUP_REFERENCE_TYPE {
                return Err(invalid("saved child front group has wrong reference type"));
            }
            Ok(ChildFront {
                group,
                segment_start: integer(&required_bytes(fields, b"pair_section_start")?)?,
                segment_end: integer(&required_bytes(fields, b"pair_section_end")?)?,
            })
        })
        .collect()
}

fn invasion_transports(items: &[Item], order_type: i32) -> io::Result<Option<i32>> {
    if order_type != 3 {
        return Ok(None);
    }
    let subscriber = required_block(items, b"convoys")?;
    integer(&required_bytes(subscriber, b"convoys")?).map(Some)
}
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
                        if handle as u32 != ORDERS_GROUP_REFERENCE_TYPE {
                            return Err(invalid("saved plan group has wrong reference type"));
                        }
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
                        self.group_names.insert(handle, bytes(group, b"name")?);
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
                            let order_type = optional_int(items, b"type")?.unwrap_or(0);
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
                                route_is_ok: flag(items, b"route_is_ok")?,
                                has_child: field(items, b"manage_child_sections")?
                                    .map(boolean)
                                    .transpose()?,
                                child_fronts: child_fronts(items)?,
                                owner_override: field(items, b"owner")?
                                    .map(typed_handle)
                                    .transpose()?
                                    .unwrap_or(0),
                                assigned,
                                order_type,
                                successors: repeated_ids(items, b"order_children")?,
                                names: {
                                    let mut names: [Vec<u8>; 6] = Default::default();
                                    for (name, key) in names.iter_mut().zip(NAME_KEYS) {
                                        *name = bytes(items, key)?.unwrap_or_default();
                                    }
                                    names
                                },
                                preparation: bytes(items, b"time")?
                                    .map(|text| Decimal::parse(&text))
                                    .transpose()?,
                                invasion_transports: invasion_transports(items, order_type)?,
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
        let parsed = content_reader_parser::parser::parse(body.as_bytes())?;
        let mut tables = Tables::default();
        tables.country(&parsed.items)?;
        Ok(tables)
    }
    #[test]
    fn group_names_preserve_omission_and_emitted_empty_text() {
        for (name, expected) in [
            ("", None),
            ("name=\"\"", Some(Vec::new())),
            (
                "name=\"Synthetic group\"",
                Some(b"Synthetic group".to_vec()),
            ),
        ] {
            let tables = parse(&format!(
                "theatres={{theatre={{orders_group={{id={{ id=19 type=53 }} {name}}}}}}}"
            ))
            .unwrap();
            assert_eq!(tables.group_names[&((19_u64 << 32) | 53)], expected);
        }
        for name in ["name={}", "name=\"A\" name=\"B\""] {
            assert!(
                parse(&format!(
                    "theatres={{theatre={{orders_group={{id={{ id=19 type=53 }} {name}}}}}}}"
                ))
                .is_err()
            );
        }
    }

    #[test]
    fn plans_keep_scoped_orders_saved_list_order_and_empty_front_sections() {
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
        let tables = parse("theatres={theatre={orders_group={id={id=8 type=53} order_instance={instance_id=3 operation=\"o_phalanx\" prefix=\"o_operation\"} order_instance={instance_id=4 first=\"o_red\" second=\"o_go\" unique=\"Riga\" postfix=\"o_offensive\"}}}}").unwrap();
        let named = &tables.orders[&((8 << 32) | 53, 3)];
        assert_eq!(named.preparation, None);
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
    fn an_invasion_keeps_its_saved_preparation_exactly() {
        let tables = parse("theatres={theatre={orders_group={id={id=8 type=53} order_instance={instance_id=3 type=3 convoys={convoys=0 total=0} time=12.34567}}}}").unwrap();
        let order = &tables.orders[&((8 << 32) | 53, 3)];
        assert_eq!(order.order_type, 3);
        assert_eq!(
            order.preparation.as_ref().map(|time| time.raw),
            Some(1_234_567)
        );
        assert!(parse("theatres={theatre={orders_group={id={id=8 type=53} order_instance={instance_id=3 time=1.000001}}}}").is_err());
    }
    #[test]
    fn execution_route_truth_defaults_false_without_using_can_execute() {
        for (fields, expected) in [
            ("", false),
            ("can_execute=yes", false),
            ("route_is_ok=no can_execute=yes", false),
            ("route_is_ok=yes can_execute=no", true),
        ] {
            let tables = parse(&format!("theatres={{theatre={{orders_group={{id={{id=8 type=53}} order_instance={{instance_id=3 type=4 {fields}}}}}}}}}")).unwrap();
            let order = &tables.orders[&((8 << 32) | 53, 3)];
            assert_eq!(order.route_is_ok, expected);
            assert_eq!(order.preparation, None);
        }
    }
    #[test]
    fn paradrop_time_retains_signed_fixed5_extremes_and_full_order_identity() {
        for (text, expected) in [
            ("0", 0),
            ("-0.00001", -1),
            ("12.34567", 1_234_567),
            ("-92233720368547.75808", i64::MIN),
            ("92233720368547.75807", i64::MAX),
        ] {
            let tables = parse(&format!("theatres={{theatre={{orders_group={{id={{id=4294967295 type=53}} order_instance={{instance_id=4294967295 type=4 route_is_ok=yes time={text}}}}}}}}}")).unwrap();
            let order = &tables.orders[&((u64::from(u32::MAX) << 32) | 53, u32::MAX)];
            assert!(order.route_is_ok);
            assert_eq!(
                order.preparation.as_ref().map(|time| time.raw),
                Some(expected)
            );
        }
        let tables = parse("theatres={theatre={orders_group={id={id=8 type=53} order_instance={instance_id=3 type=4 time=1}} orders_group={id={id=9 type=53} order_instance={instance_id=3 type=4 time=2}}}}").unwrap();
        assert_eq!(tables.orders.len(), 2);
        assert_eq!(
            tables.orders[&((8 << 32) | 53, 3)]
                .preparation
                .as_ref()
                .unwrap()
                .raw,
            100_000
        );
        assert_eq!(
            tables.orders[&((9 << 32) | 53, 3)]
                .preparation
                .as_ref()
                .unwrap()
                .raw,
            200_000
        );
    }
    #[test]
    fn execution_save_refuses_ambiguous_fields_and_malformed_identities() {
        for fields in [
            "route_is_ok=maybe",
            "route_is_ok=1",
            "route_is_ok={yes}",
            "route_is_ok=yes route_is_ok=no",
            "time=1 time=2",
            "time={1}",
            "time=1.000001",
            "time=92233720368547.75808",
            "time=-92233720368547.75809",
        ] {
            assert!(parse(&format!("theatres={{theatre={{orders_group={{id={{id=8 type=53}} order_instance={{instance_id=3 type=4 {fields}}}}}}}}}")).is_err(), "{fields}");
        }
        for body in [
            "orders_group={id={id=8 type=51} order_instance={instance_id=3 type=4}}",
            "orders_group={id={id=8 type=53} order_instance={instance_id=-1 type=4}}",
            "orders_group={id={id=8 type=53} order_instance={instance_id=4294967296 type=4}}",
            "orders_group={id={id=8 type=53} order_instance={instance_id=3 type=4} order_instance={instance_id=3 type=4}}",
            "orders_group={id={id=8 type=53} order_instance={instance_id=3 type=4} fallback={instance_id=3 type=4}}",
        ] {
            assert!(
                parse(&format!("theatres={{theatre={{{body}}}}}")).is_err(),
                "{body}"
            );
        }
    }
    #[test]
    fn invasion_convoys_require_the_nested_signed_key_even_for_zero() {
        for count in [0, -3, i32::MIN, i32::MAX] {
            let tables = parse(&format!("theatres={{theatre={{orders_group={{id={{id=8 type=53}} order_instance={{instance_id=3 type=3 convoys={{convoys={count} total=7}}}}}}}}}}")).unwrap();
            assert_eq!(
                tables.orders[&((8 << 32) | 53, 3)].invasion_transports,
                Some(count)
            );
        }
        for subscriber in [
            "",
            "convoys=0",
            "convoys={total=7}",
            "convoys={convoys=2147483648}",
            "convoys={convoys=0 convoys=1}",
            "convoys={convoys=0} convoys={convoys=0}",
        ] {
            assert!(parse(&format!("theatres={{theatre={{orders_group={{id={{id=8 type=53}} order_instance={{instance_id=3 type=3 {subscriber}}}}}}}}}")).is_err());
        }
        let ordinary = parse("theatres={theatre={orders_group={id={id=8 type=53} order_instance={instance_id=3 type=1}}}}").unwrap();
        assert_eq!(
            ordinary.orders[&((8 << 32) | 53, 3)].invasion_transports,
            None
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

    #[test]
    fn child_fronts_preserve_saved_order_duplicate_groups_and_signed_endpoints() {
        let tables = parse("theatres={theatre={orders_group={id={id=8 type=53} order_instance={instance_id=3 child_front_ratios={{group={id=4 type=53} pair_section_start=-1 pair_section_end=-1} {group={id=2 type=53} pair_section_start=-2147483648 pair_section_end=2147483647} {group={id=4 type=53} pair_section_start=7 pair_section_end=9}}}}}}").unwrap();
        assert_eq!(
            tables.orders[&((8 << 32) | 53, 3)].child_fronts,
            [
                ChildFront {
                    group: (4 << 32) | 53,
                    segment_start: -1,
                    segment_end: -1
                },
                ChildFront {
                    group: (2 << 32) | 53,
                    segment_start: i32::MIN,
                    segment_end: i32::MAX
                },
                ChildFront {
                    group: (4 << 32) | 53,
                    segment_start: 7,
                    segment_end: 9
                },
            ]
        );
        let empty = parse("theatres={theatre={orders_group={id={id=8 type=53} order_instance={instance_id=3 child_front_ratios={}} order_instance={instance_id=4}}}}").unwrap();
        assert!(
            empty
                .orders
                .values()
                .all(|order| order.child_fronts.is_empty())
        );
    }

    #[test]
    fn child_fronts_require_anonymous_typed_records_and_both_saved_spans() {
        for body in [
            "{pair_section_start=0 pair_section_end=1}",
            "{group={id=4 type=51} pair_section_start=0 pair_section_end=1}",
            "{group={id=4 type=53} pair_section_end=1}",
            "{group={id=4 type=53} pair_section_start=0}",
            "{group={id=4 type=53} pair_section_start=0 pair_section_start=1 pair_section_end=1}",
            "child={group={id=4 type=53} pair_section_start=0 pair_section_end=1}",
        ] {
            assert!(parse(&format!("theatres={{theatre={{orders_group={{id={{id=8 type=53}} order_instance={{instance_id=3 child_front_ratios={{{body}}}}}}}}}}}")).is_err(), "{body}");
        }
    }
}
