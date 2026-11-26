
use std::{collections::BTreeMap, io};

use super::{
    Decimal, GameDate, Item, block, boolean, bytes, decimal, field, integer, invalid,
    null_permitted_handle, required_block, required_bytes, scalar, typed_handle,
};

pub type BuildingKey = (u32, Vec<u8>);
pub type OffsiteKey = (Vec<u8>, Vec<u8>);
pub type LawKey = (Vec<u8>, Vec<u8>, u32);
pub type GarrisonKey = (Vec<u8>, u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Building {
    pub location: u32,
    pub building_type: Vec<u8>,
    pub ordinal: u32,
    pub level: i16,
    pub healthy_level: i16,
    pub partial_health: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Production {
    pub id: u64,
    pub country: Vec<u8>,
    pub queue_ordinal: u32,
    pub active_factories: i32,
    pub cost: Decimal,
    pub speed: Decimal,
    pub produced: Decimal,
    pub priority: i32,
    pub amount: i32,
    pub created_date: GameDate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildingConstruction {
    pub production: Production,
    pub building_type: Vec<u8>,
    pub target_location: i32,
    pub target_country: Option<Vec<u8>>,
    pub repair_levels: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RailwayConstruction {
    pub production: Production,
    pub base_level: i32,
    pub target_level: i32,
    pub cancelled: bool,
    pub current_segment: i32,
    pub remaining_cost: Decimal,
    pub route: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RailwayGunRepair {
    pub id: Option<u64>,
    pub country: Vec<u8>,
    pub queue_ordinal: u32,
    pub railway_gun: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccupationSettings {
    pub country: Vec<u8>,
    pub garrison_template: Option<u64>,
    pub default_law: Option<Vec<u8>>,
    pub occupied: Vec<Vec<u8>>,
    pub group_laws: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateOccupationLaw {
    pub controller: Vec<u8>,
    pub occupied: Vec<u8>,
    pub state: u32,
    pub law: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateGarrison {
    pub controller: Vec<u8>,
    pub occupied: Vec<u8>,
    pub state: u32,
    pub required_manpower: i32,
    pub strength_ratio: Decimal,
    pub garrison_required: Decimal,
    pub serialized_flag: bool,
    pub manpower: Vec<Manpower>,
    pub equipment: Option<super::production::Pool>,
    pub equipment_need: Option<Vec<(Vec<u8>, Decimal)>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manpower {
    pub source_country: Vec<u8>,
    pub amount: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateResistance {
    pub state: u32,
    pub country: Option<Vec<u8>>,
    pub level: Decimal,
    pub speed: Decimal,
    pub target: Decimal,
    pub compliance: Decimal,
    pub compliance_speed: Decimal,
    pub active: bool,
    pub added: Vec<AddedResistanceTarget>,
    pub definitions: ActiveOccupationDefinitions,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActiveOccupationDefinitions {
    pub resistance: Vec<Vec<u8>>,
    pub compliance: Vec<Vec<u8>>,
}

fn active_occupation_definitions(items: &[Item]) -> io::Result<ActiveOccupationDefinitions> {
    let names = |key: &[u8]| -> io::Result<Vec<Vec<u8>>> {
        field(items, key)?
            .map(|value| {
                block(value)?
                    .iter()
                    .map(|item| {
                        if item.key.is_some() {
                            return Err(invalid(
                                "active occupation definition list has a keyed entry",
                            ));
                        }
                        Ok(scalar(&item.value)?.to_vec())
                    })
                    .collect()
            })
            .transpose()
            .map(Option::unwrap_or_default)
    };
    Ok(ActiveOccupationDefinitions {
        resistance: names(b"resistance_modifiers")?,
        compliance: names(b"compliance_modifiers")?,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddedResistanceTarget {
    pub amount: Decimal,
    pub controller: Option<Vec<u8>>,
    pub occupied: Option<Vec<u8>>,
    pub tooltip: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryCollaboration {
    pub source: Vec<u8>,
    pub target: Vec<u8>,
    pub progress: Decimal,
}

pub type BuildingOverride = (Vec<u8>, Vec<(i32, Decimal)>);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavedRoute {
    pub kind: u8,
    pub from_state: Option<u32>,
    pub to_state: Option<u32>,
    pub from_port: Option<u32>,
    pub to_port: Option<u32>,
    pub land_path: Vec<u32>,
    pub naval_path: Vec<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavedDeliveryRoute {
    pub from_state: Option<u32>,
    pub to_state: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavedOrigin {
    pub state: Option<u32>,
    pub client_efficiency: [Option<Decimal>; 2],
    pub convoys: super::economy::ConvoyCounts,
    pub request: Option<i32>,
    pub efficiency: Option<Decimal>,
    pub route: Option<SavedRoute>,
    pub rights: Vec<(Vec<u8>, Vec<u8>)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountryResources {
    pub origin_states: Vec<u32>,
    pub origins: Vec<SavedOrigin>,
    pub rights_origins: Vec<(SavedOrigin, Vec<u8>)>,
    pub delivery_routes: BTreeMap<Vec<u8>, SavedDeliveryRoute>,
    pub building_overrides: Vec<BuildingOverride>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub state_buildings: BTreeMap<BuildingKey, Building>,
    pub province_buildings: BTreeMap<BuildingKey, Building>,
    pub country_buildings: BTreeMap<OffsiteKey, Building>,
    pub building_construction: BTreeMap<u64, BuildingConstruction>,
    pub railway_construction: BTreeMap<u64, RailwayConstruction>,
    pub railway_gun_repairs: Vec<RailwayGunRepair>,
    pub occupation_settings: BTreeMap<Vec<u8>, OccupationSettings>,
    pub state_occupation_laws: BTreeMap<LawKey, StateOccupationLaw>,
    pub state_garrisons: BTreeMap<GarrisonKey, StateGarrison>,
    pub country_reserve_manpower: BTreeMap<Vec<u8>, Vec<Manpower>>,
    pub state_resistance: BTreeMap<u32, StateResistance>,
    pub occupation_definitions: BTreeMap<(Vec<u8>, Vec<u8>), ActiveOccupationDefinitions>,
    pub country_collaboration: BTreeMap<(Vec<u8>, Vec<u8>), CountryCollaboration>,
    pub surrender_recipients: BTreeMap<Vec<u8>, Vec<u8>>,
    pub state_resources: BTreeMap<(u32, Vec<u8>), Decimal>,
    pub state_dynamic_modifiers: BTreeMap<u32, Vec<super::governance::DynamicModifier>>,
    pub country_resources: BTreeMap<Vec<u8>, CountryResources>,
}

fn country_resources(fields: &[Item]) -> io::Result<CountryResources> {
    let mut out = CountryResources::default();
    for item in fields {
        match item.key.as_deref() {
            Some(b"origin") => {
                let origin = saved_origin(block(&item.value)?)?;
                if let Some(state) = origin.state {
                    out.origin_states.push(state);
                }
                out.origins.push(origin);
            }
            Some(b"extra_resource_origin") => {
                let fields = block(&item.value)?;
                let origin = saved_origin(required_block(fields, b"origin")?)?;
                out.rights_origins
                    .push((origin, super::required_bytes(fields, b"giver")?));
            }
            Some(b"delivery_routes") => {
                for entry in block(&item.value)? {
                    let receiver = entry
                        .key
                        .clone()
                        .filter(|key| !key.is_empty())
                        .ok_or_else(|| invalid("shared delivery route lacks a receiver tag"))?;
                    let fields = block(&entry.value)?;
                    let endpoint = |key: &[u8]| -> io::Result<Option<u32>> {
                        field(fields, key)?
                            .map(|value| integer(scalar(value)?))
                            .transpose()
                    };
                    let route = SavedDeliveryRoute {
                        from_state: endpoint(b"from_state")?,
                        to_state: endpoint(b"to_state")?,
                    };
                    if out.delivery_routes.insert(receiver, route).is_some() {
                        return Err(invalid("duplicate shared delivery route receiver"));
                    }
                }
            }
            Some(b"modify_building_resources") => {
                for entry in block(&item.value)? {
                    let building = entry
                        .key
                        .clone()
                        .filter(|key| !key.is_empty())
                        .ok_or_else(|| invalid("building resource override lacks a building"))?;
                    let mut elements = Vec::new();
                    for element in block(&entry.value)? {
                        let index = integer(element.key.as_deref().ok_or_else(|| {
                            invalid("building resource override lacks a resource index")
                        })?)?;
                        elements.push((index, Decimal::parse(scalar(&element.value)?)?));
                    }
                    out.building_overrides.push((building, elements));
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

fn saved_origin(fields: &[Item]) -> io::Result<SavedOrigin> {
    let mut out = SavedOrigin {
        convoys: super::economy::convoy_counts(fields)?,
        ..SavedOrigin::default()
    };
    let mut efficiencies = 0;
    for item in fields {
        match item.key.as_deref() {
            Some(b"state") => out.state = Some(integer(scalar(&item.value)?)?),
            Some(b"efficiency") => {
                let value = Decimal::parse(scalar(&item.value)?)?;
                match efficiencies {
                    0 => out.client_efficiency[0] = Some(value),
                    1 => out.efficiency = Some(value),
                    _ => return Err(invalid("resource origin writes three efficiencies")),
                }
                efficiencies += 1;
            }
            Some(b"efficiency_due_to_lost_convoys") => {
                out.client_efficiency[1] = Some(Decimal::parse(scalar(&item.value)?)?);
            }
            Some(b"request") => {
                if out
                    .request
                    .replace(integer(scalar(&item.value)?)?)
                    .is_some()
                {
                    return Err(invalid("resource origin writes two requests"));
                }
            }
            Some(b"delivery_route") => out.route = Some(saved_route(block(&item.value)?)?),
            Some(b"given_resource_rights") => {
                for right in block(&item.value)? {
                    if right.key.is_some() {
                        return Err(invalid("a resource right is keyed"));
                    }
                    let [resource, holder] = block(&right.value)? else {
                        return Err(invalid("a resource right is not a resource and a holder"));
                    };
                    if resource.key.is_some() || holder.key.is_some() {
                        return Err(invalid("a resource right's parts are keyed"));
                    }
                    out.rights.push((
                        scalar(&resource.value)?.to_vec(),
                        scalar(&holder.value)?.to_vec(),
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

fn saved_route(fields: &[Item]) -> io::Result<SavedRoute> {
    let optional = |key: &[u8]| -> io::Result<Option<u32>> {
        field(fields, key)?
            .map(|value| integer(scalar(value)?))
            .transpose()
    };
    Ok(SavedRoute {
        kind: optional(b"type")?.unwrap_or(0) as u8,
        from_state: optional(b"from_state")?,
        to_state: optional(b"to_state")?,
        from_port: optional(b"from_port")?,
        to_port: optional(b"to_port")?,
        land_path: super::integer_list(fields, b"land_path")?,
        naval_path: super::integer_list(fields, b"naval_path")?,
    })
}

fn number<T: std::str::FromStr>(items: &[Item], key: &[u8]) -> io::Result<T> {
    integer(&required_bytes(items, key)?)
}

fn fixed(items: &[Item], key: &[u8]) -> io::Result<Decimal> {
    decimal(items, key)?
        .ok_or_else(|| invalid(format!("save lacks {}", String::from_utf8_lossy(key))))
}

fn zero_omitted_fixed(items: &[Item], key: &[u8]) -> io::Result<Decimal> {
    decimal(items, key)?
        .map(Ok)
        .unwrap_or_else(|| Decimal::parse(b"0"))
}

fn zero_omitted_integer(items: &[Item], key: &[u8]) -> io::Result<i32> {
    field(items, key)?
        .map(|value| integer(scalar(value)?))
        .transpose()
        .map(|n| n.unwrap_or(0))
}

fn false_omitted(items: &[Item], key: &[u8]) -> io::Result<bool> {
    field(items, key)?
        .map(boolean)
        .transpose()
        .map(|v| v.unwrap_or(false))
}

fn production(country: &[u8], queue_ordinal: u32, items: &[Item]) -> io::Result<Production> {
    Ok(Production {
        id: typed_handle(field(items, b"id")?.ok_or_else(|| invalid("production line lacks id"))?)?,
        country: country.to_vec(),
        queue_ordinal,
        active_factories: zero_omitted_integer(items, b"active_factories")?,
        cost: fixed(items, b"cost")?,
        speed: zero_omitted_fixed(items, b"speed")?,
        produced: zero_omitted_fixed(items, b"produced")?,
        priority: number(items, b"priority")?,
        amount: number(items, b"amount")?,
        created_date: GameDate::parse(&required_bytes(items, b"created_date")?)?,
    })
}

fn railway(country: &[u8], ordinal: u32, items: &[Item]) -> io::Result<RailwayConstruction> {
    let mut route = Vec::new();
    if let Some(value) = field(items, b"rail_way")? {
        for item in block(value)? {
            if item.key.is_some() {
                return Err(invalid("railway route contains a keyed entry"));
            }
            route.push(integer(scalar(&item.value)?)?);
        }
    }
    Ok(RailwayConstruction {
        production: production(country, ordinal, items)?,
        base_level: number(items, b"base")?,
        target_level: number(items, b"target")?,
        cancelled: false_omitted(items, b"cancel")?,
        current_segment: number(items, b"current_province")?,
        remaining_cost: fixed(items, b"remaining_hours")?,
        route,
    })
}

pub(crate) fn manpower(items: &[Item], key: &[u8]) -> io::Result<Vec<Manpower>> {
    let Some(value) = field(items, key)? else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    append_manpower(block(value)?, &mut rows)?;
    Ok(rows)
}

pub(crate) fn append_manpower(items: &[Item], rows: &mut Vec<Manpower>) -> io::Result<()> {
    for item in items {
        if item.key.as_deref() != Some(b"value") {
            return Err(invalid("unexpected serialized manpower entry"));
        }
        let fields = block(&item.value)?;
        let amount = number(fields, b"value")?;
        if amount <= 0 {
            return Err(invalid(
                "native manpower serializer emits only positive amounts",
            ));
        }
        rows.push(Manpower {
            source_country: required_bytes(fields, b"tag")?,
            amount,
        });
    }
    Ok(())
}

fn occupation(country: &[u8], items: &[Item], pack: &mut Pack) -> io::Result<()> {
    let settings = OccupationSettings {
        country: country.to_vec(),
        garrison_template: field(items, b"division_template_id")?
            .map(typed_handle)
            .transpose()?,
        default_law: bytes(items, b"default_law")?,
        occupied: field(items, b"occupation")?
            .map(|value| {
                block(value)?
                    .iter()
                    .map(|entry| {
                        entry
                            .key
                            .clone()
                            .filter(|tag| !tag.is_empty())
                            .ok_or_else(|| invalid("occupation lacks target tag"))
                    })
                    .collect::<io::Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default(),
        group_laws: BTreeMap::new(),
    };
    if pack
        .occupation_settings
        .insert(country.to_vec(), settings)
        .is_some()
    {
        return Err(invalid("duplicate occupation settings country"));
    }
    pack.country_reserve_manpower
        .insert(country.to_vec(), manpower(items, b"foreign_manpower")?);
    if let Some(value) = field(items, b"occupation")? {
        let mut seen = std::collections::BTreeSet::new();
        for item in block(value)? {
            let occupied = item
                .key
                .as_deref()
                .filter(|key| !key.is_empty())
                .ok_or_else(|| invalid("occupation lacks target tag"))?;
            if !seen.insert(occupied) {
                return Err(invalid("duplicate occupied country scope"));
            }
            let fields = block(&item.value)?;
            pack.occupation_definitions.insert(
                (country.to_vec(), occupied.to_vec()),
                active_occupation_definitions(fields)?,
            );
            pack.occupation_settings
                .get_mut(country)
                .expect("settings inserted above")
                .group_laws
                .insert(occupied.to_vec(), bytes(fields, b"occupation_law")?);
            if let Some(value) = field(fields, b"occupation_law_list")? {
                for item in block(value)? {
                    let state = integer(
                        item.key
                            .as_deref()
                            .ok_or_else(|| invalid("occupation law lacks raw state key"))?,
                    )?;
                    let row = StateOccupationLaw {
                        controller: country.to_vec(),
                        occupied: occupied.to_vec(),
                        state,
                        law: scalar(&item.value)?.to_vec(),
                    };
                    if pack
                        .state_occupation_laws
                        .insert((country.to_vec(), occupied.to_vec(), state), row)
                        .is_some()
                    {
                        return Err(invalid(
                            "duplicate controller/occupied/state occupation law",
                        ));
                    }
                }
            }
            if let Some(value) = field(fields, b"state_garrison_data")? {
                for item in block(value)? {
                    let state = integer(
                        item.key
                            .as_deref()
                            .ok_or_else(|| invalid("garrison lacks state key"))?,
                    )?;
                    if state == 0 {
                        return Err(invalid("saved garrison state must be positive"));
                    }
                    let fields = block(&item.value)?;
                    let row = StateGarrison {
                        controller: country.to_vec(),
                        occupied: occupied.to_vec(),
                        state,
                        required_manpower: zero_omitted_integer(fields, b"army_manpower_need")?,
                        strength_ratio: zero_omitted_fixed(fields, b"strength_ratio")?,
                        garrison_required: zero_omitted_fixed(fields, b"garrison_required")?,
                        serialized_flag: false_omitted(fields, b"suppression")?,
                        manpower: manpower(fields, b"manpower_pool")?,
                        equipment: field(fields, b"equipment")?
                            .map(|value| super::production::parse_pool(block(value)?))
                            .transpose()?,
                        equipment_need: field(fields, b"equipment_need")?
                            .map(|value| {
                                block(value)?
                                    .iter()
                                    .map(|item| {
                                        Ok((
                                            item.key
                                                .clone()
                                                .filter(|key| !key.is_empty())
                                                .ok_or_else(|| {
                                                    invalid(
                                                        "garrison requirement lacks equipment name",
                                                    )
                                                })?,
                                            Decimal::parse(scalar(&item.value)?)?,
                                        ))
                                    })
                                    .collect::<io::Result<Vec<_>>>()
                            })
                            .transpose()?,
                    };
                    if pack
                        .state_garrisons
                        .insert((country.to_vec(), state), row)
                        .is_some()
                    {
                        return Err(invalid(
                            "duplicate controller/state garrison across occupied scopes",
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn buildings<K: Ord>(
    location: u32,
    items: &[Item],
    output: &mut BTreeMap<K, Building>,
    key: impl Fn(&[u8]) -> K,
) -> io::Result<()> {
    for (ordinal, item) in items.iter().enumerate() {
        let name = item
            .key
            .as_ref()
            .filter(|key| !key.is_empty())
            .ok_or_else(|| invalid("saved building lacks type"))?;
        let fields = block(&item.value)?;
        let row = Building {
            location,
            building_type: name.clone(),
            ordinal: u32::try_from(ordinal).map_err(|_| invalid("building ordinal exceeds u32"))?,
            level: number(fields, b"level")?,
            healthy_level: number(fields, b"healthy_levels")?,
            partial_health: fixed(fields, b"partial_health")?,
        };
        if output.insert(key(name), row).is_some() {
            return Err(invalid("duplicate scoped building type"));
        }
    }
    Ok(())
}

pub fn parse(items: &[Item]) -> io::Result<Pack> {
    parse_with_countries(items, super::CountriesView::from_root(items)?)
}

pub(super) fn parse_with_countries(
    items: &[Item],
    countries: super::CountriesView<'_>,
) -> io::Result<Pack> {
    let mut pack = Pack::default();
    for item in countries.required()? {
        let country = item
            .key
            .as_deref()
            .filter(|key| !key.is_empty())
            .ok_or_else(|| invalid("construction country has no tag"))?;
        let fields = block(&item.value)?;
        if let Some(value) = field(fields, b"production")?
            && let Some(value) = field(block(value)?, b"general_lines")?
        {
            for (ordinal, item) in block(value)?.iter().enumerate() {
                let ordinal = u32::try_from(ordinal)
                    .map_err(|_| invalid("construction ordinal exceeds u32"))?;
                let fields = block(&item.value)?;
                match item.key.as_deref() {
                    Some(b"building") => {
                        let production = production(country, ordinal, fields)?;
                        let target = required_block(fields, b"building")?;
                        let repair_levels: Option<i32> = field(fields, b"to_repair")?
                            .map(|value| integer(scalar(value)?))
                            .transpose()?;
                        if repair_levels.is_some_and(|value| value <= 0) {
                            return Err(invalid(
                                "native construction serializer emits only positive to_repair",
                            ));
                        }
                        let row = BuildingConstruction {
                            production,
                            building_type: required_bytes(target, b"template")?,
                            target_location: number(target, b"location")?,
                            target_country: bytes(target, b"country")?,
                            repair_levels,
                        };
                        if pack
                            .building_construction
                            .insert(row.production.id, row)
                            .is_some()
                        {
                            return Err(invalid("duplicate full building construction handle"));
                        }
                    }
                    Some(b"rail_way") => {
                        let row = railway(country, ordinal, fields)?;
                        if pack
                            .railway_construction
                            .insert(row.production.id, row)
                            .is_some()
                        {
                            return Err(invalid("duplicate full railway construction handle"));
                        }
                    }
                    Some(b"railway_gun") => {
                        pack.railway_gun_repairs.push(RailwayGunRepair {
                            id: field(fields, b"id")?
                                .map(null_permitted_handle)
                                .transpose()?,
                            country: country.to_vec(),
                            queue_ordinal: ordinal,
                            railway_gun: field(fields, b"railway_gun")?
                                .map(null_permitted_handle)
                                .transpose()?,
                        });
                    }
                    _ => {
                        return Err(invalid(
                            "unsupported saved construction line kind; queue order cannot be compared",
                        ));
                    }
                }
            }
        }
        if let Some(value) = field(fields, b"occupation_status")? {
            occupation(country, block(value)?, &mut pack)?;
        }
        if let Some(value) = field(fields, b"buildings")? {
            buildings(0, block(value)?, &mut pack.country_buildings, |name| {
                (country.to_vec(), name.to_vec())
            })?;
        }
        if let Some(value) = field(fields, b"resources")? {
            pack.country_resources
                .insert(country.to_vec(), country_resources(block(value)?)?);
        }
        if let Some(recipient) = bytes(fields, b"last_collaborated_surrender_recipient")? {
            pack.surrender_recipients.insert(country.to_vec(), recipient);
        }
        if let Some(value) = field(fields, b"collaboration")? {
            if let Some(value) = field(block(value)?, b"collaboration")? {
                for item in block(value)? {
                    let target = item
                        .key
                        .clone()
                        .filter(|key| !key.is_empty())
                        .ok_or_else(|| invalid("collaboration lacks target country"))?;
                    let row = CountryCollaboration {
                        source: country.to_vec(),
                        target: target.clone(),
                        progress: fixed(block(&item.value)?, b"value")?,
                    };
                    if pack
                        .country_collaboration
                        .insert((country.to_vec(), target), row)
                        .is_some()
                    {
                        return Err(invalid("duplicate country collaboration pair"));
                    }
                }
            }
        }
    }
    for item in required_block(items, b"states")? {
        let state: u32 = integer(
            item.key
                .as_deref()
                .ok_or_else(|| invalid("construction state lacks id"))?,
        )?;
        let fields = block(&item.value)?;
        if let Some(value) = field(fields, b"buildings")? {
            buildings(state, block(value)?, &mut pack.state_buildings, |name| {
                (state, name.to_vec())
            })?;
        }
        if let Some(value) = field(fields, b"resources")? {
            for entry in block(value)? {
                let name = entry
                    .key
                    .clone()
                    .ok_or_else(|| invalid("state resource lacks a name"))?;
                let amount = Decimal::parse(scalar(&entry.value)?)?;
                if pack.state_resources.insert((state, name), amount).is_some() {
                    return Err(invalid("duplicate saved state resource"));
                }
            }
        }
        let modifiers = super::governance::parse_dynamic_modifiers(fields)?;
        if !modifiers.is_empty() {
            pack.state_dynamic_modifiers.insert(state, modifiers);
        }
        if let Some(value) = field(fields, b"resistance")? {
            let fields = block(value)?;
            let row = StateResistance {
                state,
                country: bytes(fields, b"occupied_country_tag")?,
                level: zero_omitted_fixed(fields, b"resistance")?,
                speed: zero_omitted_fixed(fields, b"resistance_speed")?,
                target: zero_omitted_fixed(fields, b"resistance_target")?,
                compliance: zero_omitted_fixed(fields, b"compliance")?,
                compliance_speed: zero_omitted_fixed(fields, b"compliance_speed")?,
                active: false_omitted(fields, b"operational_status")?,
                added: field(fields, b"added_resistance_targets")?
                    .map(|value| {
                        block(value)?
                            .iter()
                            .map(|entry| {
                                let fields = block(&entry.value)?;
                                Ok(AddedResistanceTarget {
                                    amount: zero_omitted_fixed(fields, b"amount")?,
                                    controller: bytes(fields, b"controller")?,
                                    occupied: bytes(fields, b"occupied")?,
                                    tooltip: bytes(fields, b"tooltip")?,
                                })
                            })
                            .collect::<io::Result<Vec<_>>>()
                    })
                    .transpose()?
                    .unwrap_or_default(),
                definitions: active_occupation_definitions(fields)?,
            };
            if pack.state_resistance.insert(state, row).is_some() {
                return Err(invalid("duplicate saved state resistance"));
            }
        }
    }
    for item in required_block(items, b"provinces")? {
        let province: u32 = integer(
            item.key
                .as_deref()
                .ok_or_else(|| invalid("construction province lacks id"))?,
        )?;
        if let Some(value) = field(block(&item.value)?, b"buildings")? {
            buildings(
                province,
                block(value)?,
                &mut pack.province_buildings,
                |name| (province, name.to_vec()),
            )?;
        }
    }
    if pack
        .building_construction
        .keys()
        .any(|id| pack.railway_construction.contains_key(id))
    {
        return Err(invalid("construction families share a full native handle"));
    }
    Ok(pack)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BaseResources {
    pub rows: BTreeMap<(u32, Vec<u8>), (u32, i64)>,
    states: std::collections::BTreeSet<u32>,
}

impl BaseResources {
    pub fn add_state_history(
        &mut self,
        state: &content_reader_states::states::StateDefinition,
    ) -> io::Result<()> {
        let state_id = state.id;
        if !self.states.insert(state_id) {
            return Err(invalid("duplicate base resource state history"));
        }
        let resources = state
            .resources
            .as_ref()
            .map_err(|why| invalid(why.clone()))?;
        for (ordinal, (name, amount)) in resources.iter().enumerate() {
            let ordinal =
                u32::try_from(ordinal).map_err(|_| invalid("base resource ordinal exceeds u32"))?;
            if let std::collections::btree_map::Entry::Vacant(entry) =
                self.rows.entry((state_id, name.as_bytes().to_vec()))
            {
                entry.insert((ordinal, i64::from(*amount) * super::FIXED_SCALE));
            } else {
                return Err(invalid(format!(
                    "duplicate state/base resource declaration {state_id}/{name} cannot be represented by the recorded (state, resource) key"
                )));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PartialBaseResources {
    pub known: BaseResources,
}

impl PartialBaseResources {
    pub(super) fn from_states(
        states: &content_reader_states::states::States,
        membership: &super::PartialMapMembership,
    ) -> io::Result<Self> {
        let mut known = BaseResources::default();
        for &id in &membership.known_states {
            known.add_state_history(states.get(id).expect("known template exists"))?;
        }
        Ok(Self { known })
    }
}

#[cfg(test)]
mod tests;
