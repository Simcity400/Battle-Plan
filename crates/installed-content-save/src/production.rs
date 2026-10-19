//! Exact literal equipment state. Serializer omissions are retained
//! independently of fields whose correspondence has not been established.

use std::{collections::BTreeMap, io};

use super::{
    Decimal, GameDate, block, boolean, bytes, decimal, field, integer, invalid, object_id,
    required_block, required_bytes, scalar, typed_handle,
};
use installed_content_core::parser::Item;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    /// The complete top-level equipment registry. An omitted registry is not
    /// an empty census.
    pub variants: Option<BTreeMap<u64, Variant>>,
    /// Each army serializes its pool under `equipment`. The
    /// enclosing countries roster can be absent independently of empty banks.
    pub divisions: Option<BTreeMap<u64, Division>>,
    pub countries: Option<BTreeMap<Vec<u8>, Country>>,
    pub template_states: BTreeMap<Vec<u8>, TemplateState>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TemplateState {
    /// None is a missing occupation wrapper; its default priority is one.
    pub garrison_priority: Option<i32>,
    /// None is an absent deployment wrapper; Some(None) is an unset selection.
    pub selected_staff: Option<Option<u64>>,
    /// Present only when the deployment order counter is positive.
    pub next_hq_order: Option<i32>,
    pub lock_all: bool,
    /// None is a missing occupation wrapper. Defaults, groups and explicit state
    /// overrides retain their separate scopes, including unset group selections.
    #[allow(
        clippy::type_complexity,
        reason = "Country and state scope presence and an unset template are independent saved facts."
    )]
    pub garrisons: Option<BTreeMap<(Option<Vec<u8>>, Option<u32>), Option<u64>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Country {
    /// The country writer emits this wrapper unconditionally.
    pub production: Option<Production>,
    /// The whole `deployment` wrapper also
    /// carries configuration fields. An absent wrapper is not an empty queue.
    pub deployment: Option<Vec<Conveyor>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Production {
    /// The entire equipment line list is traversed in stored order.
    pub lines: Vec<Line>,
    /// The stockpile pool is always emitted under `equipments`.
    pub stockpile: Option<Pool>,
    /// The licence container is always written, so an absent block is malformed
    /// rather than an empty licence set. It contains two arrays: this one as
    /// bare `available=` typed variant handles in container order (repeated
    /// handles stay distinct entries), and the licence relations below.
    pub licenses: Option<Vec<u64>>,
    /// `owned_license=` entries, in the same container order. Present exactly
    /// when `licenses` is: the two arrays share one wrapper.
    pub license_records: Option<Vec<LicenseRecord>>,
    /// The roster under `available_equipments` contains
    /// `equipment=` typed handles, repeats kept; absent is not empty.
    pub available_variants: Option<Vec<u64>>,
    /// The roster under `foreign_lease_equipments`, written only when
    /// nonempty: absence is an empty roster.
    pub foreign_lease_variants: Vec<u64>,
    /// The recalculation flag, written unconditionally as `dirty`. While it
    /// is set the hourly production update has a factory-total recalculation
    /// pending.
    pub dirty: Option<bool>,
    /// Organisations are written under `industrial_organisations`
    /// in array order, each keyed by its script key, only when there is one:
    /// absence is an empty array.
    pub organisations: Vec<Organisation>,
    pub policies: Vec<Policy>,
    /// Ordered consumable design discounts; the writer omits an empty roster.
    pub design_templates: Vec<DesignTemplate>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesignTemplate {
    pub uses: i32,
    pub factor: Decimal,
    pub equipment: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    pub key: Vec<u8>,
    pub cost: i32,
    pub cooldown_days: i32,
}

/// One organisation block, in the fields the replay records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Organisation {
    /// The block's key, the organisation's script-key token as written.
    pub key: Vec<u8>,
    /// Full CID (type 79 low word).
    pub id: u64,
    /// `name` and `icon`: written even when empty, so an
    /// absent one is malformed rather than empty.
    pub name: Option<Vec<u8>>,
    pub icon: Option<Vec<u8>>,
    /// `history` blocks in table slot order.
    pub history: Vec<OrganisationHistory>,
    pub details: OrganisationDetails,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrganisationDetails {
    /// Exact unconditional numeric emissions, with integer fields kept integer.
    pub fixed: BTreeMap<Vec<u8>, Decimal>,
    pub integers: BTreeMap<Vec<u8>, i32>,
    pub upgrades: Option<bool>,
    pub unlocked: Vec<Vec<u8>>,
    pub queued: Vec<Vec<u8>>,
    pub allowed_policies: Vec<Vec<u8>>,
    pub policy: Option<Vec<u8>>,
    pub cooldown: Option<GameDate>,
    pub variables: Option<super::intel::Variables>,
    pub flags: Option<Vec<super::intel::Flag>>,
}

/// One `history={ equipment=<variant> data={ date units } }` block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrganisationHistory {
    pub variant: u64,
    /// `units`, which the value writer always emits.
    pub units: i32,
    pub design_date: Option<GameDate>,
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
            design_date: bytes(required_block(fields, b"data")?, b"date")?
                .map(|date| GameDate::parse(&date))
                .transpose()?,
        });
    }
    Ok(Organisation {
        key: key.to_vec(),
        id: object_id(items)?,
        name: bytes(items, b"name")?,
        icon: bytes(items, b"icon")?,
        history,
        details: parse_organisation_details(items)?,
    })
}

fn parse_organisation_details(items: &[Item]) -> io::Result<OrganisationDetails> {
    let mut details = OrganisationDetails::default();
    for key in [
        b"research_bonus".as_slice(),
        b"funds",
        b"research_assign_cost",
        b"production_assign_cost",
        b"add_mio_funds_gain_factor",
    ] {
        if let Some(value) = field(items, key)? {
            details
                .fixed
                .insert(key.to_vec(), Decimal::parse(scalar(value)?)?);
        }
    }
    for key in [
        b"task_capacity".as_slice(),
        b"size",
        b"points",
        b"design_team_change_cost",
    ] {
        if let Some(value) = field(items, key)? {
            details
                .integers
                .insert(key.to_vec(), integer(scalar(value)?)?);
        }
    }
    details.upgrades = field(items, b"upgrades")?.map(boolean).transpose()?;
    for (key, output) in [
        (b"unlocked".as_slice(), &mut details.unlocked),
        (b"queued_trait", &mut details.queued),
    ] {
        for item in items.iter().filter(|item| item.key.as_deref() == Some(key)) {
            output.push(required_bytes(block(&item.value)?, b"trait")?);
        }
    }
    if let Some(value) = field(items, b"allowed_policies")? {
        for item in block(value)? {
            details.allowed_policies.push(scalar(&item.value)?.to_vec());
        }
    }
    details.policy = bytes(items, b"policy")?;
    details.cooldown = bytes(items, b"cooldown")?
        .map(|date| GameDate::parse(&date))
        .transpose()?;
    details.variables = field(items, b"variables")?
        .map(|value| {
            let items = block(value)?;
            let mut variables = super::intel::parse_variables(items)?;
            // A literal scalar named random is a stored variable as well as a
            // loader-reserved name. Keep every save emission for exact comparison.
            for item in items.iter().filter(|item| {
                item.key.as_deref() == Some(b"random") && item.value.items().is_none()
            }) {
                variables.entries.push(super::intel::Variable {
                    name: b"random".to_vec(),
                    value: Decimal::parse(scalar(&item.value)?)?,
                });
            }
            variables.reserved_entries = 0;
            Ok::<_, io::Error>(variables)
        })
        .transpose()?;
    details.flags = field(items, b"flags")?
        .map(|value| super::intel::parse_flags(block(value)?))
        .transpose()?;
    Ok(details)
}

fn parse_equipment_roster(value: &installed_content_core::parser::Value) -> io::Result<Vec<u64>> {
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

/// One saved equipment-production licence. Every
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

/// One saved military deployment conveyor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conveyor {
    pub id: u64,
    /// The template is dereferenced by the writer, so a saved conveyor always has one.
    pub template: u64,
    pub name: Vec<u8>,
    pub amount: i32,
    pub location: u32,
    pub priority: u32,
    /// Literal role text from the engine lexer; token zero has its own spelling.
    pub role: Option<Vec<u8>>,
    pub closed: Option<bool>,
    /// The government-in-exile country reference is written only
    /// when it is positive. The reference is identified but not recorded.
    pub government_in_exile_tag: Option<Vec<u8>>,
    pub lines: Vec<DeploymentLine>,
}

/// One military deployment line and the deployment it owns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeploymentLine {
    pub id: u64,
    /// `division_name` block. The order word is written only
    /// when it is nonzero, so an omitted `name_order` proves a stored zero.
    pub name_order: Option<i32>,
    /// Name-object `type` word, written unconditionally.
    pub name_type: Option<i32>,
    /// `is_name_ordered=no` is emitted only when the ordered flag is clear.
    pub name_ordered: Option<bool>,
    /// Custom division name held in the name object. Identified
    /// but not recorded; its presence is retained so the oracle can say so.
    pub name_override: Option<Vec<u8>>,
    pub name_override_programmatic: Option<bool>,
    pub amount: Option<i32>,
    /// Lines without a deployment are erased, so a live line has one.
    pub deployment: Option<Deployment>,
}

/// A military deployment's own saved scalars and containers. The writer
/// omits training, max_training and max_manpower unless the stored word is
/// strictly positive, the equipment wrapper unless there is a nonzero
/// allocation row, the halting reason unless its optional flag
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
    /// `halting_reason` word, written only when the optional's presence flag is set.
    pub halting_reason: Option<i32>,
    /// Equipment-distribution priority; the default of one is omitted.
    pub priority: Option<i32>,
}

/// One army manpower entry: a country tag and a signed count.
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
    /// Only a completely zero MIO CID is omitted.
    pub mio: Option<u64>,
    pub requested_factories: i32,
    /// Every counted per-factory efficiency, raw Q15: written under
    /// `factory_efficiencies` through the /32768 formatter, and only when
    /// the count is positive, so absence is an empty array.
    pub factory_efficiencies: Vec<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Division {
    pub id: u64,
    pub roster_owner: Vec<u8>,
    /// Omission is retained as malformed/missing save coverage, not an empty
    /// bank: this wrapper is written unconditionally.
    pub equipment: Option<Pool>,
    /// Army upgrade requests write the group's nonempty request/delivery
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
    /// `override_sprite`: the writer emits the icon entry's
    /// string, without its kind, exactly when the entry's active byte is set.
    pub override_sprite: Option<Vec<u8>>,
    pub modules: Option<Vec<Module>>,
    pub upgrades: Option<Vec<Upgrade>>,
    /// The version byte is zero-extended and emitted only when nonzero.
    /// Omission proves zero only for that byte, not the adjacent stored byte.
    pub version: Option<u8>,
    /// `number_of_design_team_traits`, written as a signed integer only when nonzero.
    pub organisation_marker: Option<i32>,
    pub organisation: Option<u64>,
    pub organisation_bonuses: Vec<(Vec<u8>, Decimal)>,
    pub design: DesignState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesignState {
    pub parent: Option<u64>,
    pub max_version: Option<u8>,
    pub is_frame: Option<bool>,
    pub auto_upgraded: Option<bool>,
    pub highlight: Option<bool>,
    pub can_upgrade_type: Option<bool>,
    pub can_upgrade_variant: Option<bool>,
    pub can_upgrade_modules: Option<bool>,
    pub manpower: Option<i32>,
    pub role_icon_index: Option<i32>,
    pub division_names_group: Option<Vec<u8>>,
    pub named_bonuses: Option<Vec<u32>>,
    pub ideas: Option<Vec<Vec<u8>>>,
    pub design_team: Option<u64>,
    pub design_team_bonus: Option<Vec<(Vec<u8>, Decimal)>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Module {
    pub slot: Vec<u8>,
    pub definition: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upgrade {
    pub definition: Vec<u8>,
    /// The stored byte is zero-extended before integer serialization.
    pub level: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmyUpgrades {
    /// Priority is omitted precisely at 1; it is not a version word.
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
    /// Cost is written unconditionally, retaining signed i64 fixed5.
    pub cost: Decimal,
    /// The same writer omits precisely zero for these three fields.
    pub speed: Option<Decimal>,
    pub produced: Option<Decimal>,
    pub active_factories: Option<i32>,
    /// Literal `priority` and `amount` are always written.
    pub priority: i32,
    pub amount: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delivery {
    /// Status is omitted precisely when its stored raw word is zero.
    pub status: Option<u32>,
    /// Only status 1 carries a serialized progress value.
    pub progress: Option<Decimal>,
    /// Omitted precisely at the fixed5 default 100000.
    pub total_progress: Option<Decimal>,
    /// The produced equipment pool wrapper is always emitted.
    pub produced: Pool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Equipment {
    /// Full low-type/high-id reference, including high bits and id zero.
    pub variant: u64,
    pub amount: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pool {
    /// Entries are written in stored order. Repeated variant handles remain
    /// distinct. The writer may omit zero entries, shifting saved positions.
    pub equipment: Vec<Equipment>,
    /// This field is always emitted. Absence is
    /// retained so comparison reports missing data rather than assuming false.
    pub allow_zero_entries: Option<bool>,
}

pub fn parse(items: &[Item]) -> io::Result<Pack> {
    parse_with_countries(items, super::CountriesView::from_root(items)?)
}

pub(super) fn parse_with_countries(
    items: &[Item],
    countries: super::CountriesView<'_>,
) -> io::Result<Pack> {
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
    let divisions = countries
        .is_present()
        .then(|| {
            let mut result = BTreeMap::new();
            for country in countries.iter() {
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
    let mut template_states = BTreeMap::new();
    let countries = countries
        .is_present()
        .then(|| {
            let mut result = BTreeMap::new();
            for country in countries.iter() {
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
                let selection = field(fields, b"deployment")?
                    .map(|value| {
                        field(block(value)?, b"default_hq_template")?
                            .map(typed_handle)
                            .transpose()
                    })
                    .transpose()?;
                template_states.insert(
                    tag.clone(),
                    TemplateState {
                        garrison_priority: field(fields, b"occupation_status")?
                            .map(|value| -> io::Result<i32> {
                                Ok(bytes(block(value)?, b"priority")?
                                    .map(|raw| integer::<i32>(&raw))
                                    .transpose()?
                                    .unwrap_or(1))
                            })
                            .transpose()?,
                        selected_staff: selection,
                        next_hq_order: field(fields, b"deployment")?
                            .map(|value| bytes(block(value)?, b"hq_next_deploy_order"))
                            .transpose()?
                            .flatten()
                            .map(|value| integer::<i32>(&value))
                            .transpose()?,
                        garrisons: field(fields, b"occupation_status")?
                            .map(|value| parse_garrison_templates(block(value)?))
                            .transpose()?,
                        lock_all: field(fields, b"templates_locked")?
                            .map(boolean)
                            .transpose()?
                            .unwrap_or(false),
                    },
                );
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
        template_states,
    })
}

#[allow(
    clippy::type_complexity,
    reason = "Preserve country/state scope presence and unset selections in the shared saved map representation."
)]
fn parse_garrison_templates(
    items: &[Item],
) -> io::Result<BTreeMap<(Option<Vec<u8>>, Option<u32>), Option<u64>>> {
    let mut result = BTreeMap::new();
    result.insert(
        (None, None),
        field(items, b"division_template_id")?
            .map(typed_handle)
            .transpose()?,
    );
    if let Some(value) = field(items, b"occupation")? {
        for group in block(value)? {
            let tag = group
                .key
                .as_ref()
                .filter(|tag| !tag.is_empty())
                .ok_or_else(|| invalid("garrison group has no country"))?;
            let fields = block(&group.value)?;
            if result
                .insert(
                    (Some(tag.clone()), None),
                    field(fields, b"division_template_id")?
                        .map(typed_handle)
                        .transpose()?,
                )
                .is_some()
            {
                return Err(invalid("duplicate garrison country selection"));
            }
            if let Some(value) = field(fields, b"garrison_template_by_state")? {
                for state in block(value)? {
                    let id = state
                        .key
                        .as_deref()
                        .ok_or_else(|| invalid("garrison state selection has no state"))?;
                    let id = integer::<u32>(id)?;
                    if result
                        .insert(
                            (Some(tag.clone()), Some(id)),
                            Some(typed_handle(&state.value)?),
                        )
                        .is_some()
                    {
                        return Err(invalid("duplicate garrison state selection"));
                    }
                }
            }
        }
    }
    Ok(result)
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
        design_templates: field(items, b"discount")?
            .map(|value| {
                block(value)?
                    .iter()
                    .map(|item| {
                        let fields = block(&item.value)?;
                        Ok(DesignTemplate {
                            uses: integer(&required_bytes(fields, b"uses")?)?,
                            factor: Decimal::parse(&required_bytes(fields, b"discount")?)?,
                            equipment: required_block(fields, b"types")?
                                .iter()
                                .map(|item| scalar(&item.value).map(<[u8]>::to_vec))
                                .collect::<io::Result<Vec<_>>>()?,
                        })
                    })
                    .collect::<io::Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default(),
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
        policies: field(items, b"policies")?
            .map(|value| {
                block(value)?
                    .iter()
                    .map(|item| {
                        let fields = block(&item.value)?;
                        Ok(Policy {
                            key: item
                                .key
                                .clone()
                                .ok_or_else(|| invalid("saved MIO policy lacks key"))?,
                            cost: integer(&required_bytes(fields, b"cost")?)?,
                            cooldown_days: integer(&required_bytes(fields, b"cooldown")?)?,
                        })
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
/// conveyor entries are queue state, and their stored order is preserved.
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
            // The name is always emitted, including empty first-NUL text.
            name: bytes(fields, b"name")?
                .ok_or_else(|| invalid("deployment conveyor lacks its unconditional name field"))?,
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
        name_ordered: name
            .map(|name| field(name, b"is_name_ordered"))
            .transpose()?
            .flatten()
            .map(boolean)
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
        design: DesignState {
            parent: field(items, b"parent_id")?.map(typed_handle).transpose()?,
            max_version: field(items, b"max_version")?
                .map(|v| integer(scalar(v)?))
                .transpose()?,
            is_frame: field(items, b"is_frame")?.map(boolean).transpose()?,
            auto_upgraded: field(items, b"auto_upgraded")?.map(boolean).transpose()?,
            highlight: field(items, b"highlight")?.map(boolean).transpose()?,
            can_upgrade_type: field(items, b"can_upgrade_type")?
                .map(boolean)
                .transpose()?,
            can_upgrade_variant: field(items, b"can_upgrade_variant")?
                .map(boolean)
                .transpose()?,
            can_upgrade_modules: field(items, b"can_upgrade_modules")?
                .map(boolean)
                .transpose()?,
            manpower: field(items, b"manpower")?
                .map(|v| integer(scalar(v)?))
                .transpose()?,
            role_icon_index: field(items, b"role_icon_index")?
                .map(|v| integer(scalar(v)?))
                .transpose()?,
            division_names_group: bytes(items, b"division_names_group")?,
            named_bonuses: field(items, b"named_equipment_bonuses")?
                .map(|v| {
                    block(v)?
                        .iter()
                        .map(|i| integer(scalar(&i.value)?))
                        .collect::<io::Result<Vec<_>>>()
                })
                .transpose()?,
            ideas: field(items, b"ideas")?
                .map(|v| {
                    block(v)?
                        .iter()
                        .map(|i| Ok(scalar(&i.value)?.to_vec()))
                        .collect::<io::Result<Vec<_>>>()
                })
                .transpose()?,
            design_team: field(items, b"design_team")?
                .map(typed_handle)
                .transpose()?,
            design_team_bonus: field(items, b"design_team_bonus")?
                .map(|v| {
                    block(v)?
                        .iter()
                        .map(|i| {
                            Ok((
                                i.key.clone().ok_or_else(|| {
                                    invalid("variant design team bonus lacks a name")
                                })?,
                                Decimal::parse(scalar(&i.value)?)?,
                            ))
                        })
                        .collect::<io::Result<Vec<_>>>()
                })
                .transpose()?,
        },
        organisation_marker: field(items, b"number_of_design_team_traits")?
            .map(|value| integer(scalar(value)?))
            .transpose()?,
        organisation: field(items, b"design_team")?
            .map(typed_handle)
            .transpose()?,
        organisation_bonuses: field(items, b"design_team_bonus")?
            .map(|value| {
                block(value)?
                    .iter()
                    .map(|item| {
                        Ok((
                            item.key
                                .clone()
                                .ok_or_else(|| invalid("design bonus lacks token"))?,
                            Decimal::parse(scalar(&item.value)?)?,
                        ))
                    })
                    .collect::<io::Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default(),
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

/// The supplied fields belong to an equipment-variant pool serializer
/// wrapper, established by its caller. No recursive search substitutes a pool
/// from another subsystem. The pool uses the `equipment`, `id` and `amount` keys.
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

/// One equipment line of the given save subtype; the caller
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

/// Common production-line fields; subtype callers supply the proven wrapper.
/// These fields do not themselves establish a production manager census.
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
