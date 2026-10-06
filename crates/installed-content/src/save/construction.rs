//! Exact construction, building and occupation values serialized in text saves.

use std::{collections::BTreeMap, io};

use super::{
    Decimal, GameDate, Item, block, boolean, bytes, decimal, field, integer, invalid,
    required_block, required_bytes, scalar, typed_handle,
};

pub type BuildingKey = (u32, Vec<u8>);
/// Owning country tag and building type of an offsite building.
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
    /// Native fixed5 health toward the next repaired level; 100 means one level.
    pub partial_health: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Production {
    pub id: u64,
    pub country: Vec<u8>,
    /// Position in the shared building/railway queue, independent of priority.
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
    /// CBuildingReference location, -1 for a country target.
    pub target_location: i32,
    /// BA6770 emits only a signed-positive reference; location targets omit it.
    pub target_country: Option<Vec<u8>>,
    /// F594E0 emits only positive repair counts; omission proves <= 0.
    pub repair_levels: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RailwayConstruction {
    pub production: Production,
    pub base_level: i32,
    pub target_level: i32,
    pub cancelled: bool,
    pub current_segment: i32,
    /// Save spelling is remaining_hours; native 19F0B70 computes remaining work.
    pub remaining_cost: Decimal,
    pub route: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccupationSettings {
    pub country: Vec<u8>,
    pub garrison_template: Option<u64>,
    pub default_law: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateOccupationLaw {
    pub controller: Vec<u8>,
    pub occupied: Vec<u8>,
    /// Literal integer key, including zero; not a guessed CState association.
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
    /// C5B610 serializes positive rows only, retaining their relative order.
    pub manpower: Vec<Manpower>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manpower {
    pub source_country: Vec<u8>,
    pub amount: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateResistance {
    pub state: u32,
    /// F87B70 emits only a signed-positive country link.
    pub country: Option<Vec<u8>>,
    pub level: Decimal,
    pub speed: Decimal,
    pub compliance: Decimal,
    pub compliance_speed: Decimal,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryCollaboration {
    pub source: Vec<u8>,
    pub target: Vec<u8>,
    pub progress: Decimal,
}

/// One `modify_building_resources` entry: the building's name and its
/// (one-based resource database index, amount) elements in native order.
pub type BuildingOverride = (Vec<u8>, Vec<(i32, Decimal)>);

/// A country's `resources` block as `CCountryResources::Write@140CAEB80`
/// writes it, reduced to what the raid state inputs read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountryResources {
    /// Each `origin`'s `state` (`CResourceOrigin + 0x80`, written by
    /// `Serialize@140CAF880` only when not null), in native order.
    pub origin_states: Vec<u32>,
    /// `modify_building_resources` (`+0x7a0`, read back by `140CAA0C0`) in
    /// native order. An entry can be empty.
    pub building_overrides: Vec<BuildingOverride>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub state_buildings: BTreeMap<BuildingKey, Building>,
    pub province_buildings: BTreeMap<BuildingKey, Building>,
    /// A country's offsite buildings (CCountry +0x1350), which 70C580 writes
    /// as the country's `buildings` block (token 0x2f59) and omits when empty,
    /// so a country without one has none. They have no location: `location`
    /// is zero.
    pub country_buildings: BTreeMap<OffsiteKey, Building>,
    pub building_construction: BTreeMap<u64, BuildingConstruction>,
    pub railway_construction: BTreeMap<u64, RailwayConstruction>,
    pub occupation_settings: BTreeMap<Vec<u8>, OccupationSettings>,
    pub state_occupation_laws: BTreeMap<LawKey, StateOccupationLaw>,
    pub state_garrisons: BTreeMap<GarrisonKey, StateGarrison>,
    pub country_reserve_manpower: BTreeMap<Vec<u8>, Vec<Manpower>>,
    pub state_resistance: BTreeMap<u32, StateResistance>,
    pub country_collaboration: BTreeMap<(Vec<u8>, Vec<u8>), CountryCollaboration>,
    /// A state's `resources` block: its own `CState + 0x1b8` pool by name.
    pub state_resources: BTreeMap<(u32, Vec<u8>), Decimal>,
    /// A state's `dynamic_modifier` block: its `CState + 0x788` container in order.
    pub state_dynamic_modifiers: BTreeMap<u32, Vec<super::governance::DynamicModifier>>,
    /// Each country's `resources` block, when it has one.
    pub country_resources: BTreeMap<Vec<u8>, CountryResources>,
}

fn country_resources(fields: &[Item]) -> io::Result<CountryResources> {
    let mut out = CountryResources::default();
    for item in fields {
        match item.key.as_deref() {
            Some(b"origin") => {
                if let Some(state) = field(block(&item.value)?, b"state")? {
                    out.origin_states.push(integer(scalar(state)?)?);
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
    // 14A36A0 omits these three values exactly when their native integer is zero.
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
        // Embedded CGameDate primary +50/raw +58/CPersistent +60.
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
    // 19F0F20 emits all four numeric values unconditionally; cancel is bool.
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

fn manpower(items: &[Item], key: &[u8]) -> io::Result<Vec<Manpower>> {
    let Some(value) = field(items, key)? else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for item in block(value)? {
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
    Ok(rows)
}

fn occupation(country: &[u8], items: &[Item], pack: &mut Pack) -> io::Result<()> {
    let settings = OccupationSettings {
        country: country.to_vec(),
        // FEAE70 emits token 0x3142 only for a nonnull template pointer.
        garrison_template: field(items, b"division_template_id")?
            .map(typed_handle)
            .transpose()?,
        default_law: bytes(items, b"default_law")?,
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
    let mut pack = Pack::default();
    for item in required_block(items, b"countries")? {
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
                    // CRailwayGunRepairLine: no construction table records
                    // it; its position still counts toward later ordinals.
                    Some(b"railway_gun") => {}
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
        if let Some(value) = field(fields, b"collaboration")? {
            // Country 70C580 and status13E08A0 use the same literal at two levels.
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
                compliance: zero_omitted_fixed(fields, b"compliance")?,
                compliance_speed: zero_omitted_fixed(fields, b"compliance_speed")?,
                active: false_omitted(fields, b"operational_status")?,
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

/// Initial CStateTemplate resource declarations from the independently verified
/// content. They are not the runtime CState resource pool serialized by a save.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BaseResources {
    pub rows: BTreeMap<(u32, Vec<u8>), (u32, i64)>,
    states: std::collections::BTreeSet<u32>,
}

impl BaseResources {
    /// Initial resources already read by CStateTemplate's native replay
    /// (140AAF580): ordered, repeated blocks appended, decimal i32 amounts.
    pub fn add_state_history(&mut self, state: &crate::states::StateDefinition) -> io::Result<()> {
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

/// Initial resource rows for clean state templates. Consumers report exclusions
/// through the membership returned by `super::read_partial_state_history`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PartialBaseResources {
    pub known: BaseResources,
}

impl PartialBaseResources {
    pub(super) fn from_states(
        states: &crate::states::States,
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
