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
    /// The +4A8 recalculation flag, which E5D3B0 writes unconditionally as
    /// `dirty` (token 0x3484). While it is set the hourly production update
    /// 140E54DE0 has a factory-total recalculation pending.
    pub dirty: Option<bool>,
    /// E5D3B0 writes the +0x130 organisations under `industrial_organisations`
    /// in array order, each keyed by its script key, only when there is one:
    /// absence is an empty array.
    pub organisations: Vec<Organisation>,
}

/// One COrganisation block (Serialize 140da84b0), in the fields the replay
/// records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Organisation {
    /// The block's key, the organisation's script-key token as written.
    pub key: Vec<u8>,
    /// Full CID (type 79 low word).
    pub id: u64,
    /// `name` (token 0x1b) and `icon` (0xb5): written even when empty, so an
    /// absent one is malformed rather than empty.
    pub name: Option<Vec<u8>>,
    pub icon: Option<Vec<u8>>,
    /// `history` blocks in table slot order.
    pub history: Vec<OrganisationHistory>,
}

/// One `history={ equipment=<variant> data={ date units } }` block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrganisationHistory {
    pub variant: u64,
    /// `units` (0x2faa), which the value writer 140da8930 always writes.
    pub units: i32,
}

fn parse_organisation(key: &[u8], items: &[Item]) -> io::Result<Organisation> {
    let mut history = Vec::new();
    for item in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"history"))
    {
        let fields = block(&item.value)?;
        history.push(OrganisationHistory {
            variant: typed_handle(
                field(fields, b"equipment")?
                    .ok_or_else(|| invalid("saved organisation history lacks its equipment"))?,
            )?,
            units: integer(&required_bytes(required_block(fields, b"data")?, b"units")?)?,
        });
    }
    Ok(Organisation {
        key: key.to_vec(),
        id: object_id(items)?,
        name: bytes(items, b"name")?,
        icon: bytes(items, b"icon")?,
        history,
    })
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
    /// Every counted per-factory efficiency (+0xC0), raw Q15: 19260B0 writes
    /// them under `factory_efficiencies` through the /32768 formatter, and
    /// only when the count (+0xCC) is positive, so absence is an empty array.
    pub factory_efficiencies: Vec<i64>,
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
    /// `override_sprite` (token 0x3c92): BD4770 writes the icon entry's
    /// string, without its kind, exactly when the entry's active byte is set.
    pub override_sprite: Option<Vec<u8>>,
    pub modules: Option<Vec<Module>>,
    pub upgrades: Option<Vec<Upgrade>>,
    /// Serialize 140BD4770 emits token 0xEE only for a nonzero +0x41C,
    /// zero-extended at 140BD47FE. Omission proves zero, not +0x41D.
    pub version: Option<u8>,
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
        lines.push(parse_line(kind, block(&item.value)?)?);
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
        dirty: field(items, b"dirty")?.map(boolean).transpose()?,
        organisations: field(items, b"industrial_organisations")?
            .map(|value| {
                block(value)?
                    .iter()
                    .map(|item| {
                        let key = item
                            .key
                            .as_deref()
                            .ok_or_else(|| invalid("saved organisation has no key"))?;
                        parse_organisation(key, block(&item.value)?)
                    })
                    .collect::<io::Result<Vec<_>>>()
            })
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
        override_sprite: bytes(items, b"override_sprite")?,
        modules,
        upgrades,
        version: field(items, b"version")?
            .map(|value| integer(scalar(value)?))
            .transpose()?,
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

/// One equipment line (19260B0) of the given save subtype; the caller
/// supplies the `military_lines`-style wrapper's fields.
pub fn parse_line(kind: u8, fields: &[Item]) -> io::Result<Line> {
    Ok(Line {
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
        factory_efficiencies: field(fields, b"factory_efficiencies")?
            .map(|value| {
                block(value)?
                    .iter()
                    .map(|item| {
                        if item.key.is_some() {
                            return Err(invalid("saved factory efficiency has a key"));
                        }
                        super::economy::FuelQuantity::parse(scalar(&item.value)?)?.raw()
                    })
                    .collect::<io::Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default(),
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
mod tests;
