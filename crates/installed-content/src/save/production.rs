//! Exact literal equipment state. Native serializer omissions are retained
//! independently of fields whose correspondence has not been established.

use std::{collections::BTreeMap, io};

use super::{
    Decimal, block, boolean, bytes, decimal, field, integer, invalid, object_id, required_block,
    required_bytes, scalar, typed_handle,
};
use crate::parser::Item;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    /// The complete top-level equipment registry. An omitted registry is not
    /// an empty census.
    pub variants: Option<BTreeMap<u64, Variant>>,
    /// CArmy C80E00 serializes its primary+348 pool under equipment. The
    /// enclosing countries roster can be absent independently of empty banks.
    pub divisions: Option<BTreeMap<u64, Division>>,
    pub countries: Option<BTreeMap<Vec<u8>, Country>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Country {
    /// CCountry70C580 writes this wrapper unconditionally.
    pub production: Option<Production>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Production {
    /// E5D3B0 traverses the entire equipment line list in native order.
    pub lines: Vec<Line>,
    /// E5D3B0 always emits its primary+200 pool under equipments.
    pub stockpile: Option<Pool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub base: LineBase,
    /// Corresponding save subtype: military/naval/refit/railway gun.
    pub kind: u8,
    pub variant: u64,
    /// 19260B0 omits only a completely zero MIO CID.
    pub mio: Option<u64>,
    pub requested_factories: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Division {
    pub id: u64,
    pub roster_owner: Vec<u8>,
    /// Omission is retained as malformed/missing save coverage, not an empty
    /// bank: C80E00 writes this wrapper unconditionally.
    pub equipment: Option<Pool>,
    /// CArmy.requests.upgrades writes the group's nonempty request/delivery
    /// vectors. Empty runtime groups have no saved priority.
    pub upgrades: Option<ArmyUpgrades>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    pub id: u64,
    /// Literal enclosing definition key; never an inferred runtime token ID.
    pub definition: Vec<u8>,
    pub display_name: Option<Vec<u8>>,
    pub variant_name: Option<Vec<u8>>,
    pub append_variant_name: Option<bool>,
    pub obsolete: Option<bool>,
    pub creator: Option<Vec<u8>>,
    pub origin: Option<Vec<u8>>,
    pub modules: Option<Vec<Module>>,
    pub upgrades: Option<Vec<Upgrade>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Module {
    pub slot: Vec<u8>,
    pub definition: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upgrade {
    pub definition: Vec<u8>,
    /// F7A763 zero-extends the native byte before integer serialization.
    pub level: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmyUpgrades {
    /// Base1595070 omits priority precisely at1; it is not a version word.
    pub priority: Option<u32>,
    pub requests: Vec<UpgradeRequest>,
    pub deliveries: Vec<Delivery>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpgradeRequest {
    pub equipment: Option<u64>,
    pub request: Decimal,
    pub total: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineBase {
    pub id: u64,
    /// 1414A36A0 writes cost unconditionally, retaining signed i64 fixed5.
    pub cost: Decimal,
    /// The same writer omits precisely zero for these three fields.
    pub speed: Option<Decimal>,
    pub produced: Option<Decimal>,
    pub active_factories: Option<i32>,
    /// Literal priority token8D and amount token1A1 are always written.
    pub priority: i32,
    pub amount: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delivery {
    /// 1414FE370 omits status precisely when its native raw word is zero.
    pub status: Option<u32>,
    /// Only status1 carries a serialized progress value.
    pub progress: Option<Decimal>,
    /// Omitted precisely at the native fixed5 default100000.
    pub total_progress: Option<Decimal>,
    /// The produced equipment pool wrapper is always emitted.
    pub produced: Pool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Equipment {
    /// Full low-type/high-id native reference, including high bits and id zero.
    pub variant: u64,
    pub amount: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pool {
    /// FFDB00 writes entries in native order. Repeated variant handles remain
    /// distinct. The writer may omit zero entries, shifting saved positions.
    pub equipment: Vec<Equipment>,
    /// FFDB00 and unconditional1424AE850 always emit this field. Absence is
    /// retained so comparison reports missing data rather than assuming false.
    pub allow_zero_entries: Option<bool>,
}

pub fn parse(items: &[Item]) -> io::Result<Pack> {
    let variants = field(items, b"equipments")?
        .map(|value| {
            let mut result = BTreeMap::new();
            for item in block(value)? {
                let definition = item
                    .key
                    .as_ref()
                    .filter(|key| !key.is_empty())
                    .ok_or_else(|| invalid("saved equipment has no definition key"))?;
                let fields = block(&item.value)?;
                let id = typed_handle(
                    field(fields, b"id")?
                        .ok_or_else(|| invalid("saved equipment lacks its typed id"))?,
                )?;
                if result
                    .insert(id, parse_variant(id, definition, fields)?)
                    .is_some()
                {
                    return Err(invalid("duplicate full saved equipment handle"));
                }
            }
            Ok(result)
        })
        .transpose()?;
    let divisions = field(items, b"countries")?
        .map(|value| {
            let mut result = BTreeMap::new();
            for country in block(value)? {
                let tag = country
                    .key
                    .as_ref()
                    .filter(|key| !key.is_empty())
                    .ok_or_else(|| invalid("saved country has no tag"))?;
                let Some(units) = field(block(&country.value)?, b"units")? else {
                    continue;
                };
                for division in block(units)?
                    .iter()
                    .filter(|entry| entry.key.as_deref() == Some(b"division"))
                {
                    let fields = block(&division.value)?;
                    let id = object_id(fields)?;
                    let equipment = field(fields, b"equipment")?
                        .map(|value| parse_pool(block(value)?))
                        .transpose()?;
                    let upgrades = field(fields, b"requests")?
                        .map(block)
                        .transpose()?
                        .map(|requests| field(requests, b"upgrades"))
                        .transpose()?
                        .flatten()
                        .map(|value| parse_army_upgrades(block(value)?))
                        .transpose()?;
                    if result
                        .insert(
                            id,
                            Division {
                                id,
                                roster_owner: tag.clone(),
                                equipment,
                                upgrades,
                            },
                        )
                        .is_some()
                    {
                        return Err(invalid("duplicate full division equipment parent handle"));
                    }
                }
            }
            Ok(result)
        })
        .transpose()?;
    let countries = field(items, b"countries")?
        .map(|value| {
            let mut result = BTreeMap::new();
            for country in block(value)? {
                let tag = country
                    .key
                    .as_ref()
                    .ok_or_else(|| invalid("country has no tag"))?;
                let production = field(block(&country.value)?, b"production")?
                    .map(|value| parse_production(block(value)?))
                    .transpose()?;
                if result.insert(tag.clone(), Country { production }).is_some() {
                    return Err(invalid("duplicate production country tag"));
                }
            }
            Ok(result)
        })
        .transpose()?;
    Ok(Pack {
        variants,
        divisions,
        countries,
    })
}

fn parse_production(items: &[Item]) -> io::Result<Production> {
    let mut lines = Vec::new();
    for item in items {
        let kind = match item.key.as_deref() {
            Some(b"military_lines") => 0,
            Some(b"naval_lines") => 1,
            Some(b"ship_refit_lines") => 2,
            Some(b"railway_gun_lines") => 3,
            _ => continue,
        };
        let fields = block(&item.value)?;
        lines.push(Line {
            base: parse_line_base(fields)?,
            kind,
            variant: typed_handle(
                field(fields, b"equipment_variant_index")?
                    .ok_or_else(|| invalid("saved production line lacks its variant handle"))?,
            )?,
            mio: field(fields, b"industrial_manufacturer")?
                .map(typed_handle)
                .transpose()?,
            requested_factories: integer(&required_bytes(fields, b"requested_factories")?)?,
        });
    }
    Ok(Production {
        lines,
        stockpile: field(items, b"equipments")?
            .map(|value| parse_pool(block(value)?))
            .transpose()?,
    })
}

fn parse_variant(id: u64, definition: &[u8], items: &[Item]) -> io::Result<Variant> {
    let modules = field(items, b"modules")?
        .map(|value| {
            block(value)?
                .iter()
                .map(|item| {
                    Ok(Module {
                        slot: item
                            .key
                            .clone()
                            .ok_or_else(|| invalid("saved variant module has no slot"))?,
                        definition: scalar(&item.value)?.to_vec(),
                    })
                })
                .collect::<io::Result<Vec<_>>>()
        })
        .transpose()?;
    let upgrades = field(items, b"upgrades")?
        .map(|value| {
            let mut result = Vec::new();
            for item in required_block(block(value)?, b"upgrades")? {
                if item.key.is_some() {
                    return Err(invalid("saved variant upgrade is not an anonymous pair"));
                }
                let fields = block(&item.value)?;
                if fields.len() != 2 || fields.iter().any(|field| field.key.is_some()) {
                    return Err(invalid(
                        "saved variant upgrade must contain definition and level",
                    ));
                }
                result.push(Upgrade {
                    definition: scalar(&fields[0].value)?.to_vec(),
                    level: integer(scalar(&fields[1].value)?)?,
                });
            }
            Ok(result)
        })
        .transpose()?;
    Ok(Variant {
        id,
        definition: definition.to_vec(),
        display_name: bytes(items, b"name")?,
        variant_name: bytes(items, b"position")?,
        append_variant_name: field(items, b"show_position")?.map(boolean).transpose()?,
        obsolete: field(items, b"obsolete")?.map(boolean).transpose()?,
        creator: bytes(items, b"creator")?,
        origin: bytes(items, b"origin")?,
        modules,
        upgrades,
    })
}

fn parse_army_upgrades(items: &[Item]) -> io::Result<ArmyUpgrades> {
    let priority = field(items, b"priority")?
        .map(|value| integer::<i32>(scalar(value)?).map(|value| value as u32))
        .transpose()?;
    let mut requests = Vec::new();
    let mut deliveries = Vec::new();
    for item in items {
        match item.key.as_deref() {
            Some(b"request") => {
                let fields = block(&item.value)?;
                requests.push(UpgradeRequest {
                    equipment: field(fields, b"equipment_variant_index")?
                        .map(typed_handle)
                        .transpose()?,
                    request: Decimal::parse(&required_bytes(fields, b"request")?)?,
                    total: Decimal::parse(&required_bytes(fields, b"total")?)?,
                });
            }
            Some(b"delivery") => deliveries.push(parse_delivery(block(&item.value)?)?),
            _ => {}
        }
    }
    Ok(ArmyUpgrades {
        priority,
        requests,
        deliveries,
    })
}

/// The supplied fields belong to a genuine CEquipmentVariantPool serializer
/// wrapper, established by its caller. No recursive search substitutes a pool
/// from another subsystem. FFDB00 uses equipment/id/amount tokens2F4E/B/1A1.
pub fn parse_pool(items: &[Item]) -> io::Result<Pool> {
    let mut equipment = Vec::new();
    for item in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"equipment"))
    {
        let fields = block(&item.value)?;
        let variant = typed_handle(
            field(fields, b"id")?
                .ok_or_else(|| invalid("saved equipment pool entry lacks its typed id"))?,
        )?;
        equipment.push(Equipment {
            variant,
            amount: Decimal::parse(&required_bytes(fields, b"amount")?)?,
        });
    }
    Ok(Pool {
        equipment,
        allow_zero_entries: field(items, b"allow_zero_entries")?
            .map(boolean)
            .transpose()?,
    })
}

/// Common CProductionLine fields; subtype callers supply the proven wrapper.
/// Base1414A36A0 does not itself establish a production manager census.
pub fn parse_line_base(items: &[Item]) -> io::Result<LineBase> {
    Ok(LineBase {
        id: object_id(items)?,
        cost: Decimal::parse(&required_bytes(items, b"cost")?)?,
        speed: decimal(items, b"speed")?,
        produced: decimal(items, b"produced")?,
        active_factories: field(items, b"active_factories")?
            .map(|value| integer(scalar(value)?))
            .transpose()?,
        priority: integer(&required_bytes(items, b"priority")?)?,
        amount: integer(&required_bytes(items, b"amount")?)?,
    })
}

pub fn parse_delivery(items: &[Item]) -> io::Result<Delivery> {
    let status = field(items, b"status")?
        .map(|value| integer::<i32>(scalar(value)?).map(|value| value as u32))
        .transpose()?;
    let progress = decimal(items, b"progress")?;
    if (status == Some(1)) != progress.is_some() {
        return Err(invalid(
            "saved upgrade delivery must carry progress exactly when status is 1",
        ));
    }
    Ok(Delivery {
        status,
        progress,
        total_progress: decimal(items, b"total_progress")?,
        produced: parse_pool(required_block(items, b"produced")?)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_preserves_order_duplicates_signed_extrema_and_zero() {
        let doc = crate::parser::parse(
            br#"
            equipment={id={id=4294967295 type=70} amount=-92233720368547.75808}
            equipment={id={id=0 type=70} amount=0.00000}
            equipment={id={id=4294967295 type=70} amount=92233720368547.75807}
            allow_zero_entries=yes
            "#,
        )
        .unwrap();
        let pool = parse_pool(&doc.items).unwrap();
        assert_eq!(pool.allow_zero_entries, Some(true));
        assert_eq!(pool.equipment.len(), 3);
        assert_eq!(pool.equipment[0].variant, 0xffff_ffff_0000_0046);
        assert_eq!(pool.equipment[1].variant, 70);
        assert_eq!(pool.equipment[2].variant, pool.equipment[0].variant);
        assert_eq!(pool.equipment[0].amount.raw, i64::MIN);
        assert_eq!(pool.equipment[1].amount.raw, 0);
        assert_eq!(pool.equipment[2].amount.raw, i64::MAX);
        assert_eq!(parse_pool(&[]).unwrap().allow_zero_entries, None);
    }

    #[test]
    fn pool_refuses_conflicting_fields_null_handles_and_precision_loss() {
        for text in [
            "equipment={id={id=1 type=70} amount=1 amount=2}",
            "equipment={id={id=0 type=0} amount=1}",
            "equipment={id={id=1 type=70} amount=0.000001}",
            "equipment={id={id=1 type=70} amount=92233720368547.75808}",
            "allow_zero_entries=yes allow_zero_entries=no",
        ] {
            let doc = crate::parser::parse(text.as_bytes()).unwrap();
            assert!(parse_pool(&doc.items).is_err(), "{text}");
        }
    }

    #[test]
    fn registry_keeps_full_handles_and_distinguishes_absence_from_empty() {
        assert!(parse(&[]).unwrap().variants.is_none());
        let doc = crate::parser::parse(b"equipments={}").unwrap();
        assert!(parse(&doc.items).unwrap().variants.unwrap().is_empty());
        let doc =
            crate::parser::parse(b"equipments={same={id={id=1 type=70}} same={id={id=1 type=71}}}")
                .unwrap();
        let rows = parse(&doc.items).unwrap().variants.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[&0x1_0000_0046].definition, b"same");
        let doc = crate::parser::parse(
            b"equipments={first={id={id=1 type=70}} second={id={id=1 type=70}}}",
        )
        .unwrap();
        assert!(parse(&doc.items).is_err());
    }

    #[test]
    fn division_bank_wrapper_is_distinct_from_other_country_equipment() {
        let doc = crate::parser::parse(
            b"countries={GER={equipment={invalid=yes} units={
              division={id={id=7 type=51} id={id=7 type=51} equipment={
                equipment={id={id=3 type=70} amount=0.00001}}}
              division={id={id=8 type=51}}
            }}}",
        )
        .unwrap();
        let divisions = parse(&doc.items).unwrap().divisions.unwrap();
        assert_eq!(divisions.len(), 2);
        assert_eq!(
            divisions[&0x7_0000_0033]
                .equipment
                .as_ref()
                .unwrap()
                .equipment[0]
                .amount
                .raw,
            1
        );
        assert!(divisions[&0x8_0000_0033].equipment.is_none());
    }

    #[test]
    fn production_base_preserves_native_widths_and_writer_omissions() {
        let doc = crate::parser::parse(
            b"id={id=4294967295 type=56} cost=92233720368547.75807
              speed=-92233720368547.75808 produced=-0.00001
              active_factories=-2147483648 priority=2147483647 amount=-1",
        )
        .unwrap();
        let line = parse_line_base(&doc.items).unwrap();
        assert_eq!(line.id, 0xffff_ffff_0000_0038);
        assert_eq!(line.cost.raw, i64::MAX);
        assert_eq!(line.speed.unwrap().raw, i64::MIN);
        assert_eq!(line.produced.unwrap().raw, -1);
        assert_eq!(line.active_factories, Some(i32::MIN));
        assert_eq!(line.priority, i32::MAX);
        assert_eq!(line.amount, -1);
        let doc = crate::parser::parse(b"id={id=0 type=56} cost=0 priority=0 amount=1").unwrap();
        let line = parse_line_base(&doc.items).unwrap();
        assert_eq!(line.speed, None);
        assert_eq!(line.produced, None);
        assert_eq!(line.active_factories, None);
    }

    #[test]
    fn delivery_progress_exists_only_for_active_status_and_keeps_defaults_absent() {
        let doc = crate::parser::parse(b"produced={}").unwrap();
        let delivery = parse_delivery(&doc.items).unwrap();
        assert_eq!(delivery.status, None);
        assert_eq!(delivery.progress, None);
        assert_eq!(delivery.total_progress, None);
        let doc = crate::parser::parse(
            b"status=1 progress=-0.00001 total_progress=92233720368547.75807 produced={}",
        )
        .unwrap();
        let delivery = parse_delivery(&doc.items).unwrap();
        assert_eq!(delivery.progress.unwrap().raw, -1);
        assert_eq!(delivery.total_progress.unwrap().raw, i64::MAX);
        for text in [
            "status=1 produced={}",
            "status=2 progress=0 produced={}",
            "status=1 progress=0",
        ] {
            let doc = crate::parser::parse(text.as_bytes()).unwrap();
            assert!(parse_delivery(&doc.items).is_err(), "{text}");
        }
    }

    #[test]
    fn production_country_wrapper_and_variant_children_preserve_native_keys_and_order() {
        let doc = crate::parser::parse(
            br#"
            countries={GER={production={
                naval_lines={id={id=4294967295 type=57} cost=0 priority=1 amount=-1
                    requested_factories=-2 equipment_variant_index={id=3 type=70}
                    industrial_manufacturer={id=0 type=79}}
                military_lines={id={id=0 type=56} cost=0 priority=0 amount=1
                    requested_factories=2 equipment_variant_index={id=3 type=70}}
                equipments={allow_zero_entries=no}
            }}}
            equipments={plane={id={id=3 type=70} name="name" position="II"
                show_position=no obsolete=yes creator="GER" origin="---"
                modules={slot=first slot=second}
                upgrades={upgrades={{"engine" 255} {"engine" 0}}}
            }}
        "#,
        )
        .unwrap();
        let pack = parse(&doc.items).unwrap();
        let country = &pack.countries.as_ref().unwrap()[b"GER".as_slice()];
        let production = country.production.as_ref().unwrap();
        assert_eq!(production.lines[0].kind, 1);
        assert_eq!(production.lines[1].kind, 0);
        assert_eq!(production.lines[0].base.id, 0xffff_ffff_0000_0039);
        assert_eq!(production.lines[0].mio, Some(79));
        assert_eq!(production.lines[0].requested_factories, -2);
        assert_eq!(production.lines[1].mio, None);
        let variant = &pack.variants.as_ref().unwrap()[&0x3_0000_0046];
        assert_eq!(variant.variant_name.as_deref(), Some(b"II".as_slice()));
        assert_eq!(variant.append_variant_name, Some(false));
        let modules = variant.modules.as_ref().unwrap();
        assert_eq!(modules[0].slot, modules[1].slot);
        assert_eq!(modules[0].definition, b"first");
        assert_eq!(modules[1].definition, b"second");
        let upgrades = variant.upgrades.as_ref().unwrap();
        assert_eq!(upgrades[0].level, 255);
        assert_eq!(upgrades[1].level, 0);
        for level in ["-1", "256"] {
            let text =
                "equipments={plane={id={id=3 type=70} upgrades={upgrades={{engine LEVEL}}}}}"
                    .replace("LEVEL", level);
            let doc = crate::parser::parse(text.as_bytes()).unwrap();
            assert!(parse(&doc.items).is_err(), "{level}");
        }
    }

    #[test]
    fn army_upgrade_parent_priority_and_nested_requests_are_not_conflated() {
        let doc = crate::parser::parse(
            b"countries={GER={units={division={
            id={id=1 type=51} requests={upgrades={priority=-1
                request={request=-0.00001 total=92233720368547.75807
                    equipment_variant_index={id=4294967295 type=70}}
                request={request=0 total=1}
                delivery={status=-2147483648 produced={allow_zero_entries=yes
                    equipment={id={id=0 type=70} amount=-92233720368547.75808}}}
            }}
        }}}}",
        )
        .unwrap();
        let pack = parse(&doc.items).unwrap();
        let group = pack.divisions.as_ref().unwrap()[&0x1_0000_0033]
            .upgrades
            .as_ref()
            .unwrap();
        assert_eq!(group.priority, Some(u32::MAX));
        assert_eq!(group.requests.len(), 2);
        assert_eq!(group.requests[0].equipment, Some(0xffff_ffff_0000_0046));
        assert_eq!(group.requests[1].equipment, None);
        assert_eq!(group.deliveries[0].status, Some(0x8000_0000));
        assert_eq!(group.deliveries[0].progress, None);
        assert_eq!(
            group.deliveries[0].produced.equipment[0].amount.raw,
            i64::MIN
        );
    }
}
