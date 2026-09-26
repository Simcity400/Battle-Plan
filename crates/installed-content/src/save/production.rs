//! Exact literal equipment state. Native serializer omissions are retained
//! independently of fields whose correspondence has not been established.

use std::{collections::BTreeMap, io};

use super::{
    Decimal, GameDate, block, boolean, bytes, decimal, field, integer, invalid, object_id,
    required_block, required_bytes, scalar, typed_handle,
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
    /// CDeploymentStatus writes the whole `deployment` wrapper, which also
    /// carries configuration fields. An absent wrapper is not an empty queue.
    pub deployment: Option<Vec<Conveyor>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Production {
    /// E5D3B0 traverses the entire equipment line list in native order.
    pub lines: Vec<Line>,
    /// E5D3B0 always emits its primary+200 pool under equipments.
    pub stockpile: Option<Pool>,
    /// E5D3B0 writes the +190 licence container under token 0x37B2 through
    /// 1424ABB40, so an absent block is malformed rather than an empty licence
    /// set. Container serializer 141429400 writes two arrays into it: this one
    /// as bare `available=` typed variant handles in container order (repeated
    /// handles stay distinct entries), and the licence relations below.
    pub licenses: Option<Vec<u64>>,
    /// `owned_license=` entries, in the same container order. Present exactly
    /// when `licenses` is: the two arrays share one wrapper.
    pub license_records: Option<Vec<LicenseRecord>>,
    /// E5D3B0 writes the +A0 roster under `available_equipments` as
    /// `equipment=` typed handles, repeats kept; absent is not empty.
    pub available_variants: Option<Vec<u64>>,
    /// The +E8 roster under `foreign_lease_equipments`, written only when
    /// nonempty: absence is an empty roster.
    pub foreign_lease_variants: Vec<u64>,
}

fn parse_equipment_roster(value: &crate::parser::Value) -> io::Result<Vec<u64>> {
    block(value)?
        .iter()
        .map(|item| {
            if item.key.as_deref() != Some(b"equipment") {
                return Err(invalid("saved equipment roster has an unmodelled field"));
            }
            typed_handle(&item.value)
        })
        .collect()
}

/// One CEquipmentProductionLicense as serializer 141429310 writes it. Every
/// field is unconditional, so a missing one is a malformed save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LicenseRecord {
    pub lended_cic: i32,
    pub required_cic: i32,
    /// Licensee tag, the country whose roster stores this record.
    pub owner: Vec<u8>,
    /// Licensor tag.
    pub giver: Vec<u8>,
    pub start_date: GameDate,
    pub parent: u64,
    /// Ordered produced-variant handles; repeated handles stay distinct.
    pub equipment: Vec<u64>,
}

/// One CMilitaryDeploymentConveyor as serializer 1414247D0 writes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conveyor {
    pub id: u64,
    /// Written by dereferencing +0x20, so a saved conveyor always has one.
    pub template: u64,
    pub name: Vec<u8>,
    pub amount: i32,
    pub location: u32,
    pub priority: u32,
    /// Literal role text from the engine lexer; token zero has its own spelling.
    pub role: Option<Vec<u8>>,
    pub closed: Option<bool>,
    /// 1414247D0 writes the +0x98 country reference under token 0x3ABA only
    /// when it is positive. The reference is identified but not recorded.
    pub government_in_exile_tag: Option<Vec<u8>>,
    pub lines: Vec<DeploymentLine>,
}

/// One CMilitaryDeploymentLine and the CMilitaryDeployment it owns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeploymentLine {
    pub id: u64,
    /// `division_name` block. Serializer 1409BCC70 writes the order word only
    /// when it is nonzero, so an omitted `name_order` proves a stored zero.
    pub name_order: Option<i32>,
    /// Name-object `type` word at +0x08, which 1409BCC70 writes unconditionally.
    pub name_type: Option<i32>,
    /// Custom division name held in the name object's +0x88 string. Identified
    /// but not recorded; its presence is retained so the oracle can say so.
    pub name_override: Option<Vec<u8>>,
    pub name_override_programmatic: Option<bool>,
    pub amount: Option<i32>,
    pub deployment: Option<Deployment>,
}

/// CMilitaryDeployment's own saved scalars and containers. Serializer
/// 1414246F0 omits training, max_training and max_manpower unless the stored
/// word is strictly positive, the equipment wrapper unless gate 140FFB8F0
/// finds a nonzero allocation row, the halting reason unless its optional flag
/// is set and the priority at its default of one, so every field is optional
/// here and the comparator applies those rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deployment {
    pub training: Option<Decimal>,
    pub max_training: Option<Decimal>,
    pub max_manpower: Option<i32>,
    pub manpower_have: Option<Vec<ManpowerValue>>,
    pub manpower_need: Option<Vec<ManpowerValue>>,
    pub equipment: Option<Pool>,
    /// `halting_reason` word; 1414246F0 writes it only when the optional's
    /// `_bIsSet` byte at CMilitaryDeployment+0xc4 is set.
    pub halting_reason: Option<i32>,
    /// CEquipmentDistributable priority; 141595070 omits the default of one.
    pub priority: Option<i32>,
}

/// One CArmyManpowerValues entry: a country tag and a signed count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManpowerValue {
    pub tag: Vec<u8>,
    pub value: i32,
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
    /// `override_model`, the variant's explicit model entity.
    pub override_model: Option<Vec<u8>>,
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
                let fields = block(&country.value)?;
                let production = field(fields, b"production")?
                    .map(|value| parse_production(block(value)?))
                    .transpose()?;
                let deployment = field(fields, b"deployment")?
                    .map(|value| parse_deployment(block(value)?))
                    .transpose()?;
                if result
                    .insert(
                        tag.clone(),
                        Country {
                            production,
                            deployment,
                        },
                    )
                    .is_some()
                {
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
    let container = field(items, b"production_licenses")?
        .map(block)
        .transpose()?;
    if let Some(container) = container {
        // Both published arrays share this wrapper; an unmodelled key would be
        // a third array this parser cannot account for.
        if let Some(key) = container.iter().find_map(|item| {
            item.key
                .as_deref()
                .filter(|key| !matches!(*key, b"available" | b"owned_license"))
        }) {
            return Err(invalid(format!(
                "saved licence container holds unmodelled key {}",
                String::from_utf8_lossy(key)
            )));
        }
    }
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
        licenses: container
            .map(|items| {
                items
                    .iter()
                    .filter(|item| item.key.as_deref() == Some(b"available"))
                    .map(|item| typed_handle(&item.value))
                    .collect::<io::Result<Vec<_>>>()
            })
            .transpose()?,
        license_records: container
            .map(|items| {
                items
                    .iter()
                    .filter(|item| item.key.as_deref() == Some(b"owned_license"))
                    .map(|item| parse_license_record(block(&item.value)?))
                    .collect::<io::Result<Vec<_>>>()
            })
            .transpose()?,
        available_variants: field(items, b"available_equipments")?
            .map(parse_equipment_roster)
            .transpose()?,
        foreign_lease_variants: field(items, b"foreign_lease_equipments")?
            .map(parse_equipment_roster)
            .transpose()?
            .unwrap_or_default(),
    })
}

fn parse_license_record(items: &[Item]) -> io::Result<LicenseRecord> {
    let mut equipment = Vec::new();
    for item in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"equipment"))
    {
        equipment.push(typed_handle(&item.value)?);
    }
    Ok(LicenseRecord {
        lended_cic: integer(&required_bytes(items, b"lended_cic")?)?,
        required_cic: integer(&required_bytes(items, b"required_cic")?)?,
        owner: required_bytes(items, b"owner")?,
        giver: required_bytes(items, b"giver")?,
        start_date: GameDate::parse(&required_bytes(items, b"start_date")?)?,
        parent: typed_handle(
            field(items, b"parent")?
                .ok_or_else(|| invalid("saved licence record lacks its parent variant"))?,
        )?,
        equipment,
    })
}

/// The `deployment` wrapper also carries configuration blocks; only the
/// conveyor entries are queue state, and their native order is preserved.
fn parse_deployment(items: &[Item]) -> io::Result<Vec<Conveyor>> {
    let mut conveyors = Vec::new();
    for item in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"military_deployment_conveyor"))
    {
        let fields = block(&item.value)?;
        let mut lines = Vec::new();
        for line in fields
            .iter()
            .filter(|line| line.key.as_deref() == Some(b"military_deployment_line"))
        {
            lines.push(parse_deployment_line(block(&line.value)?)?);
        }
        conveyors.push(Conveyor {
            id: object_id(fields)?,
            template: typed_handle(
                field(fields, b"division_template_id")?
                    .ok_or_else(|| invalid("saved conveyor lacks its division template"))?,
            )?,
            name: bytes(fields, b"name")?.unwrap_or_default(),
            amount: integer(&required_bytes(fields, b"amount")?)?,
            location: integer(&required_bytes(fields, b"location")?)?,
            priority: integer(&required_bytes(fields, b"priority")?)?,
            role: bytes(fields, b"role")?,
            closed: field(fields, b"closed")?.map(boolean).transpose()?,
            government_in_exile_tag: bytes(fields, b"government_in_exile_tag")?,
            lines,
        });
    }
    Ok(conveyors)
}

fn parse_deployment_line(items: &[Item]) -> io::Result<DeploymentLine> {
    let name = field(items, b"division_name")?.map(block).transpose()?;
    Ok(DeploymentLine {
        id: object_id(items)?,
        name_order: name
            .map(|name| field(name, b"name_order"))
            .transpose()?
            .flatten()
            .map(|value| integer(scalar(value)?))
            .transpose()?,
        name_type: name
            .map(|name| field(name, b"type"))
            .transpose()?
            .flatten()
            .map(|value| integer(scalar(value)?))
            .transpose()?,
        name_override: name
            .map(|name| bytes(name, b"override"))
            .transpose()?
            .flatten(),
        name_override_programmatic: name
            .map(|name| field(name, b"override_set_programmatically"))
            .transpose()?
            .flatten()
            .map(boolean)
            .transpose()?,
        amount: field(items, b"amount")?
            .map(|value| integer(scalar(value)?))
            .transpose()?,
        deployment: field(items, b"military_deployment")?
            .map(|value| parse_deployment_object(block(value)?))
            .transpose()?,
    })
}

fn parse_deployment_object(items: &[Item]) -> io::Result<Deployment> {
    Ok(Deployment {
        training: decimal(items, b"training")?,
        max_training: decimal(items, b"max_training")?,
        max_manpower: field(items, b"max_manpower")?
            .map(|value| integer(scalar(value)?))
            .transpose()?,
        manpower_have: parse_manpower(items, b"army_manpower_value")?,
        manpower_need: parse_manpower(items, b"army_manpower_need")?,
        equipment: field(items, b"equipment")?
            .map(|value| parse_pool(block(value)?))
            .transpose()?,
        halting_reason: field(items, b"halting_reason")?
            .map(|value| integer(scalar(value)?))
            .transpose()?,
        priority: field(items, b"priority")?
            .map(|value| integer(scalar(value)?))
            .transpose()?,
    })
}

fn parse_manpower(items: &[Item], key: &[u8]) -> io::Result<Option<Vec<ManpowerValue>>> {
    field(items, key)?
        .map(|value| {
            let mut values = Vec::new();
            for entry in block(value)? {
                if entry.key.as_deref() != Some(b"value") {
                    return Err(invalid("saved manpower container holds a foreign key"));
                }
                let entry = block(&entry.value)?;
                values.push(ManpowerValue {
                    tag: required_bytes(entry, b"tag")?,
                    value: integer(&required_bytes(entry, b"value")?)?,
                });
            }
            Ok(values)
        })
        .transpose()
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
        override_model: bytes(items, b"override_model")?,
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
    fn equipment_rosters_keep_saved_order_repeats_and_absence() {
        let production = |text: &[u8]| {
            let document = crate::parser::parse(text).unwrap();
            parse(&document.items).unwrap().countries.unwrap()[b"GER".as_slice()]
                .production
                .clone()
                .unwrap()
        };
        let saved = production(
            br#"countries={GER={production={available_equipments={
                equipment={id=4294967295 type=70}
                equipment={id=0 type=70}
                equipment={id=4294967295 type=70}
            } foreign_lease_equipments={ equipment={id=0 type=70} }}}}"#,
        );
        assert_eq!(
            saved.available_variants,
            Some(vec![0xffff_ffff_0000_0046, 70, 0xffff_ffff_0000_0046])
        );
        assert_eq!(saved.foreign_lease_variants, [70]);
        let absent = production(b"countries={GER={production={}}}");
        assert_eq!(absent.available_variants, None);
        assert!(absent.foreign_lease_variants.is_empty());
    }

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

    fn pack(text: &[u8]) -> io::Result<Pack> {
        parse(&crate::parser::parse(text)?.items)
    }

    const QUEUE: &[u8] = br#"countries={
GER={
production={
    production_licenses={
        available={ id=5061 type=70 }
        available={ id=5061 type=70 }
        owned_license={ lended_cic=-1 required_cic=2147483647 owner="LAT" giver="ENG"
                        start_date="1939.5.10.1" parent={ id=1441 type=70 }
                        equipment={ id=5061 type=70 } equipment={ id=5061 type=70 } }
    }
}
deployment={
    unlocked_subunits={ { infantry yes} }
    military_deployment_conveyor={
        id={ id=433 type=64 }
        division_template_id={ id=2526 type=52 }
        name="Medium Tank 1942 C"
        amount=-1
        location=6103
        priority=1
        role=armor
        closed=no
        military_deployment_line={
            id={ id=7067 type=63 }
            division_name={ type=0 override="1. Light Tank" override_set_programmatically=yes }
            amount=1
            military_deployment={
                army_manpower_value={ }
                army_manpower_need={ }
                halting_reason=1
                priority=3
            }
        }
        military_deployment_line={
            id={ id=7066 type=63 }
            division_name={ type=0 name_order=4 }
            amount=1
            military_deployment={
                training=0.00001
                max_training=0.98799
                army_manpower_value={ value={ tag="ENG" value=10620 } value={ tag="GER" value=1 } }
                army_manpower_need={ }
                equipment={
                    equipment={ id={ id=5553 type=70 } amount=16 }
                    allow_zero_entries=no
                }
                max_manpower=10620
            }
        }
    }
    military_deployment_conveyor={
        id={ id=434 type=64 }
        division_template_id={ id=1 type=52 }
        name=""
        amount=0
        location=0
        priority=0
        role=infantry
        closed=yes
        government_in_exile_tag="ETH"
    }
}
}
}"#;

    #[test]
    fn licences_and_the_deployment_queue_keep_order_duplicates_and_signed_words() {
        let saved = pack(QUEUE).unwrap();
        let country = &saved.countries.unwrap()[b"GER".as_slice()];
        let licenses = country
            .production
            .as_ref()
            .unwrap()
            .licenses
            .clone()
            .unwrap();
        assert_eq!(licenses.len(), 2);
        assert_eq!(licenses[0], (5061_u64 << 32) | 70);
        assert_eq!(licenses[0], licenses[1]);

        let records = country
            .production
            .as_ref()
            .unwrap()
            .license_records
            .clone()
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].lended_cic, -1);
        assert_eq!(records[0].required_cic, i32::MAX);
        assert_eq!(records[0].owner, b"LAT");
        assert_eq!(records[0].giver, b"ENG");
        assert_eq!(records[0].parent, (1441_u64 << 32) | 70);
        assert_eq!(records[0].equipment, vec![(5061_u64 << 32) | 70; 2]);
        assert_eq!(records[0].start_date.year, 1939);

        let queue = country.deployment.clone().unwrap();
        assert_eq!(queue.len(), 2);
        assert_eq!(
            queue[1].government_in_exile_tag.as_deref(),
            Some(b"ETH".as_slice())
        );
        // A custom-named line omits name_order and carries an override name;
        // an untrained deployment omits training, max_training and equipment.
        let custom = &queue[0].lines[0];
        assert_eq!(custom.name_order, None);
        assert_eq!(
            custom.name_override.as_deref(),
            Some(b"1. Light Tank".as_slice())
        );
        assert_eq!(custom.name_override_programmatic, Some(true));
        let untrained = custom.deployment.as_ref().unwrap();
        assert_eq!(untrained.training, None);
        assert_eq!(untrained.max_training, None);
        assert_eq!(untrained.equipment, None);
        assert_eq!(untrained.max_manpower, None);
        assert_eq!(untrained.halting_reason, Some(1));
        assert_eq!(untrained.priority, Some(3));
        assert_eq!(queue[0].id, (433_u64 << 32) | 64);
        assert_eq!(queue[0].template, (2526_u64 << 32) | 52);
        assert_eq!(queue[0].name, b"Medium Tank 1942 C");
        assert_eq!(queue[0].amount, -1);
        assert_eq!(queue[0].location, 6103);
        assert_eq!(queue[0].priority, 1);
        assert_eq!(queue[0].role.as_deref(), Some(b"armor".as_slice()));
        assert_eq!(queue[0].closed, Some(false));
        assert_eq!(queue[1].closed, Some(true));
        assert!(queue[1].lines.is_empty());
        assert_eq!(queue[1].name, b"");

        let line = &queue[0].lines[1];
        assert_eq!(line.id, (7066_u64 << 32) | 63);
        assert_eq!(line.name_order, Some(4));
        assert_eq!(line.name_type, Some(0));
        assert_eq!(line.amount, Some(1));
        let deployment = line.deployment.as_ref().unwrap();
        assert_eq!(deployment.training.as_ref().unwrap().raw, 1);
        assert_eq!(deployment.max_training.as_ref().unwrap().raw, 98_799);
        assert_eq!(deployment.max_manpower, Some(10_620));
        assert_eq!(
            deployment.manpower_have.as_ref().unwrap(),
            &[
                ManpowerValue {
                    tag: b"ENG".to_vec(),
                    value: 10_620
                },
                ManpowerValue {
                    tag: b"GER".to_vec(),
                    value: 1
                },
            ]
        );
        assert_eq!(deployment.manpower_need.as_ref().unwrap(), &Vec::new());
        assert_eq!(deployment.priority, None);
        let pool = deployment.equipment.as_ref().unwrap();
        assert_eq!(pool.allow_zero_entries, Some(false));
        assert_eq!(pool.equipment[0].variant, (5553_u64 << 32) | 70);
        assert_eq!(pool.equipment[0].amount.raw, 1_600_000);
    }

    #[test]
    fn absent_containers_stay_absent_and_foreign_keys_are_refused() {
        let saved = pack(b"countries={GER={production={}}}").unwrap();
        let country = &saved.countries.unwrap()[b"GER".as_slice()];
        assert_eq!(country.production.as_ref().unwrap().licenses, None);
        assert_eq!(country.production.as_ref().unwrap().license_records, None);
        assert_eq!(country.deployment, None);
        let empty = pack(b"countries={GER={production={production_licenses={}} deployment={}}}")
            .unwrap()
            .countries
            .unwrap();
        let country = &empty[b"GER".as_slice()];
        assert_eq!(
            country.production.as_ref().unwrap().licenses,
            Some(Vec::new())
        );
        assert_eq!(
            country.production.as_ref().unwrap().license_records,
            Some(Vec::new())
        );
        assert_eq!(country.deployment, Some(Vec::new()));
        for text in [
            "countries={GER={production={production_licenses={ owned={ id=1 type=70 } }}}}",
            r#"countries={GER={production={production_licenses={ owned_license={
                required_cic=0 owner="A" giver="B" start_date="1936.1.1.1"
                parent={ id=1 type=70 } }}}}}"#,
            r#"countries={GER={production={production_licenses={ owned_license={
                lended_cic=0 required_cic=0 owner="A" giver="B" start_date="1936.1.1.1" }}}}}"#,
            "countries={GER={deployment={military_deployment_conveyor={ name=x }}}}",
            r#"countries={GER={deployment={military_deployment_conveyor={
                id={ id=1 type=64 } name=x amount=0 location=0 priority=0 }}}}"#,
            r#"countries={GER={deployment={military_deployment_conveyor={
                id={ id=1 type=64 } division_template_id={ id=1 type=52 }
                name=x amount=0 location=0 priority=0
                military_deployment_line={ id={ id=2 type=63 } military_deployment={
                    army_manpower_value={ tag={ tag="ENG" value=1 } } } } }}}}"#,
        ] {
            assert!(pack(text.as_bytes()).is_err(), "{text}");
        }
    }
}
