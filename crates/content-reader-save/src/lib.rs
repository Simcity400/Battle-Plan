
pub mod air;
pub mod characters;
pub mod combat_details;
pub mod combat_log;
pub mod construction;
mod control;
pub mod counter_inputs;
mod countries;
pub mod country_rosters;
pub mod economy;
pub mod faction_accounts;
pub mod faction_companions;
pub mod faction_goals;
pub mod governance;
pub mod intel;
pub mod naval_accidents;
pub mod naval_history;
pub mod naval_launch;
pub mod naval_management;
pub mod naval_missions;
pub mod plans;
pub mod production;
pub mod projects;
pub mod saved_event_targets;
pub mod script_events;
pub mod session;
mod units;
pub mod war_goals;
pub mod war_participation;
pub mod weather;

pub use control::{ProvinceControl, StateControl, StateControlEmission};
use countries::CountriesView;
pub use countries::{
    ActualCountry, CountryEmissionIdentity, CountryProjection, SaveDocument, SavedCountryEmission,
};

pub use units::{
    Acclimatization, AirWing, ArmyOrder, Division, DivisionHistory, DivisionHistoryEntry,
    DivisionName, DivisionPresentation, DivisionTemplate, DivisionTemplateSettings, Fleet,
    IntelEntry, LandCombat, NavalCombat, NavalCombatParticipant, RailwayGun, Ship, ShipName,
    TaskForce,
};
use units::{
    parse_air_wings, parse_division, parse_division_templates, parse_fleet, parse_land_combat,
    parse_naval_combat, parse_orders, parse_railway_gun, parse_ship, parse_task_force,
};

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use content_reader_layers::ResolvedLayers;
use content_reader_parser::parser::{Item, Value};

#[cfg(test)]
use content_reader_parser::parser;

pub(crate) const MONTH_DAYS: [u8; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
pub use content_reader_parser::parser::decimal::{Decimal, FIXED_SCALE};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn pack_error(pack: &str, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{pack} save tables: {error}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameDate {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
}

impl GameDate {
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        let text = std::str::from_utf8(bytes).map_err(|_| invalid("save date is not ASCII"))?;
        let components = text.split('.').collect::<Vec<_>>();
        if components.len() != 4 {
            return Err(invalid("save date must contain year.month.day.hour"));
        }
        let date = Self {
            year: components[0]
                .parse()
                .map_err(|_| invalid("invalid save year"))?,
            month: components[1]
                .parse()
                .map_err(|_| invalid("invalid save month"))?,
            day: components[2]
                .parse()
                .map_err(|_| invalid("invalid save day"))?,
            hour: components[3]
                .parse()
                .map_err(|_| invalid("invalid save hour"))?,
        };
        date.engine_hour()?;
        Ok(date)
    }

    pub fn engine_hour(self) -> io::Result<i32> {
        if !(0..99_998).contains(&self.year) {
            return Err(invalid(
                "oracle supports save years 0 through 99997; BCE dates are unsupported",
            ));
        }
        let month = usize::from(self.month)
            .checked_sub(1)
            .filter(|&month| month < 12)
            .ok_or_else(|| invalid("save month must be 1 through 12"))?;
        if self.day == 0 || self.day > MONTH_DAYS[month] {
            return Err(invalid(
                "save day is outside the engine's fixed non-leap calendar",
            ));
        }
        if !(1..=24).contains(&self.hour) {
            return Err(invalid("save hour must be 1 through 24"));
        }
        let prior_days: i64 = MONTH_DAYS[..month]
            .iter()
            .map(|&days| i64::from(days))
            .sum();
        let days = i64::from(self.year) * 365 + prior_days + i64::from(self.day) - 1;
        i32::try_from(43_800_000 + days * 24 + i64::from(self.hour) - 1)
            .map_err(|_| invalid("save date exceeds the engine hour representation"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavedDate {
    Calendar(GameDate),
    Invalid,
}

impl SavedDate {
    pub const INVALID_HOUR: i32 = 43_791_240;

    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        if bytes == b"-1.1.1.1" {
            Ok(Self::Invalid)
        } else {
            GameDate::parse(bytes).map(Self::Calendar)
        }
    }

    pub fn engine_hour(self) -> io::Result<i32> {
        match self {
            Self::Calendar(date) => date.engine_hour(),
            Self::Invalid => Ok(Self::INVALID_HOUR),
        }
    }
}

impl std::fmt::Display for GameDate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}.{}.{}.{}",
            self.year, self.month, self.day, self.hour
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EquipmentAmount {
    pub id: u32,
    pub id_type: u32,
    pub amount: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolunteerPermission {
    pub tag: Vec<u8>,
    pub count: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Country {
    pub tag: Vec<u8>,
    pub major: bool,
    pub cosmetic_tag: Option<Vec<u8>>,
    pub original_tag: Option<Vec<u8>>,
    pub political_power: Option<Decimal>,
    pub command_power: Option<Decimal>,
    pub stability: Option<Decimal>,
    pub war_support: Option<Decimal>,
    pub manpower_residual: i32,
    pub recruitment_ratio: Option<i64>,
    pub exile_manpower: Option<i32>,
    pub exile_manpower_cap: Option<i32>,
    pub convoy_equipment: Option<Vec<EquipmentAmount>>,
    pub instances_counter: Option<u32>,
    pub volunteer_permissions: Vec<VolunteerPermission>,
    pub naval_mines: Option<BTreeMap<u32, Decimal>>,
    pub naval_region_access: Option<Vec<(u32, u8)>>,
    pub regional_convoy_efficiency: Option<BTreeMap<u32, Decimal>>,
    pub stability_terms: StabilityTerms,
    pub refresh: Option<u32>,
    pub cores: Vec<u32>,
    pub claims: Vec<u32>,
    pub civil_war_targets: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StabilityTerms {
    pub propaganda_stability_penalty: Option<Decimal>,
    pub coastal_protection_ratio: Option<Decimal>,
    pub being_bombed_support_penalty: Option<Decimal>,
    pub heroes_dying_war_support_penalty: Option<Decimal>,
    pub convoy_raiding_war_support_penalty: Option<Decimal>,
    pub propaganda_war_support_penalty: Option<Decimal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub id: u32,
    pub owner: Option<Vec<u8>>,
    pub controller: Option<Vec<u8>>,
    pub demilitarized: Option<bool>,
    pub is_border_conflict: Option<bool>,
    pub manpower_available: Option<i32>,
    pub manpower_locked: Option<i32>,
    pub population: Option<Option<i32>>,
    pub category: Option<Vec<u8>>,
    pub name: Option<Vec<u8>>,
    pub strategic_locations: Vec<(Vec<u8>, u32)>,
    pub contested_owners: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvinceOverride {
    pub id: u32,
    pub name: Option<Vec<u8>>,
    pub owner: Option<Vec<u8>>,
    pub controller: Option<Vec<u8>>,
    pub victory_points: Option<i32>,
    pub strategic_locations: Vec<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AllocationCounters {
    pub debug_current_ref_id: u32,
    pub unit: i32,
    pub order_index: i32,
    pub front_index: i32,
    pub theatre_index: i32,
    pub theater_group_index: i32,
    pub military_deployment_line_index: i32,
    pub military_deployment_conveyor_index: i32,
    pub equipment_variant_index: i32,
    pub country_leader_index: i32,
    pub navy_id: i32,
    pub land_combat_id: i32,
    pub multiplayer_random_count: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationOpinion {
    pub owner: Vec<u8>,
    pub target: Vec<u8>,
    pub cached_sum: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationTuple {
    pub owner: Vec<u8>,
    pub target: Vec<u8>,
    pub kind: Vec<u8>,
    pub ordinal: u32,
    pub first: Vec<u8>,
    pub second: Vec<u8>,
    pub start_date: Option<GameDate>,
    pub end_date: Option<GameDate>,
    pub cancel: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CountryIntelRecord {
    pub civilian: i64,
    pub army: i64,
    pub navy: i64,
    pub air: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarRelation {
    pub participation: [Option<war_participation::Ledger>; 2],
    pub first_goals: Vec<u64>,
    pub second_goals: Vec<u64>,
    pub roster_owner: Vec<u8>,
    pub target: Vec<u8>,
    pub first: Vec<u8>,
    pub second: Vec<u8>,
    pub start_date: Option<GameDate>,
    pub end_date: Option<GameDate>,
    pub cancel: bool,
    pub first_casualties: i64,
    pub second_casualties: i64,
    pub first_unknown_casualties: i64,
    pub second_unknown_casualties: i64,
    pub first_was_instigator: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Faction {
    pub goals: faction_goals::Status,
    pub companions: faction_companions::Snapshot,
    pub id: u64,
    pub name: Option<Vec<u8>>,
    pub members: Vec<Vec<u8>>,
    pub icon: Option<Vec<u8>>,
    pub template: Option<Vec<u8>>,
    pub color: Option<Vec<i64>>,
    pub color_overridden: Option<bool>,
    pub theatres: Vec<FactionTheatre>,
    pub advisor_slots: Vec<Option<Vec<u8>>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FactionTheatre {
    pub name: Option<Vec<u8>>,
    pub tag: Option<Vec<u8>>,
    pub commander: Option<u64>,
    pub regions: Vec<u32>,
    pub countries: Vec<Vec<u8>>,
    pub hidden: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Player {
    pub country: Vec<u8>,
    pub user: Vec<u8>,
    pub country_leader: bool,
    pub id: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Save {
    pub date: GameDate,
    pub campaign_id: Vec<u8>,
    pub version: Vec<u8>,
    pub seed: i32,
    pub counters: AllocationCounters,
    pub countries: BTreeMap<Vec<u8>, Country>,
    pub country_emissions: Option<Vec<SavedCountryEmission>>,
    pub states: BTreeMap<u32, State>,
    pub provinces: BTreeMap<u32, ProvinceOverride>,
    pub divisions: BTreeMap<u64, Division>,
    pub railway_guns: BTreeMap<u64, RailwayGun>,
    pub division_templates: BTreeMap<u64, DivisionTemplate>,
    pub division_template_owners: BTreeMap<u64, Vec<u8>>,
    pub war_relations: Vec<WarRelation>,
    pub war_goals: BTreeMap<u64, war_goals::WarGoal>,
    pub wargoal_countries: BTreeMap<Vec<u8>, war_goals::CountryInventory>,
    pub relation_opinions: Vec<RelationOpinion>,
    pub war_delivery_histories: Vec<war_participation::DeliveryHistory>,
    pub relation_tuples: Vec<RelationTuple>,
    pub country_intel: BTreeMap<Vec<u8>, Vec<CountryIntelRecord>>,
    pub country_naval_launch: Vec<naval_launch::CountryNavalLaunchOccurrence>,
    pub factions: BTreeMap<u64, Faction>,
    pub faction_accounts: Option<faction_accounts::Accounts>,
    pub player: Option<Vec<u8>>,
    pub players: Vec<Player>,
    pub land_combats: BTreeMap<u64, LandCombat>,
    pub fleets: BTreeMap<u64, Fleet>,
    pub naval_management: naval_management::Pack,
    pub task_forces: BTreeMap<u64, TaskForce>,
    pub ships: BTreeMap<u64, Ship>,
    pub army_orders: BTreeMap<(u64, u32), ArmyOrder>,
    pub plans: plans::Tables,
    pub air_wings: BTreeMap<u64, AirWing>,
    pub naval_combats: BTreeMap<u64, NavalCombat>,
    pub naval_combat_participants: BTreeMap<(u64, u8, u32), NavalCombatParticipant>,
    pub equipment_definitions: BTreeMap<(u32, u32), Vec<u8>>,
    pub saved_event_targets: Vec<saved_event_targets::Target>,
    pub air: air::Pack,
    pub characters: characters::Pack,
    pub combat_log: combat_log::Pack,
    pub construction: construction::Pack,
    pub economy: economy::Pack,
    pub governance: governance::Pack,
    pub intel: intel::Pack,
    pub naval_history: naval_history::Pack,
    pub production: production::Pack,
    pub projects: projects::Pack,
    pub session: session::Pack,
    pub weather: weather::Pack,
    pub counter_inputs: counter_inputs::Pack,
    pub country_rosters: country_rosters::Pack,
    pub naval_missions: naval_missions::Pack,
}

fn field<'a>(items: &'a [Item], key: &[u8]) -> io::Result<Option<&'a Value>> {
    let mut found = items
        .iter()
        .filter(|item| item.key.as_deref() == Some(key))
        .map(|item| &item.value);
    let value = found.next();
    if found.next().is_some() {
        return Err(invalid(format!(
            "duplicate save field {}",
            String::from_utf8_lossy(key)
        )));
    }
    Ok(value)
}

fn block(value: &Value) -> io::Result<&[Item]> {
    match value {
        Value::Block(items) => Ok(items),
        _ => Err(invalid("expected an ordinary save block")),
    }
}

fn scalar(value: &Value) -> io::Result<&[u8]> {
    value
        .scalar()
        .ok_or_else(|| invalid("expected a save scalar"))
}

fn bytes(items: &[Item], key: &[u8]) -> io::Result<Option<Vec<u8>>> {
    field(items, key)?
        .map(|value| scalar(value).map(<[u8]>::to_vec))
        .transpose()
}

fn contested_owners(state: &[Item]) -> io::Result<Vec<Vec<u8>>> {
    let Some(value) = field(state, b"contested_owners")? else {
        return Ok(Vec::new());
    };
    block(value)?
        .iter()
        .map(|item| match item.key {
            None => scalar(&item.value).map(<[u8]>::to_vec),
            Some(_) => Err(invalid("contested owner entry has a key")),
        })
        .collect()
}

fn required_bytes(items: &[Item], key: &[u8]) -> io::Result<Vec<u8>> {
    bytes(items, key)?
        .filter(|bytes| !bytes.is_empty())
        .ok_or_else(|| invalid(format!("save lacks {}", String::from_utf8_lossy(key))))
}

fn required_block<'a>(items: &'a [Item], key: &[u8]) -> io::Result<&'a [Item]> {
    block(
        field(items, key)?
            .ok_or_else(|| invalid(format!("save lacks {}", String::from_utf8_lossy(key))))?,
    )
}

fn integer<T: std::str::FromStr>(bytes: &[u8]) -> io::Result<T> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("save integer is not ASCII"))?;
    text.parse().map_err(|_| invalid(format!("save integer {text:?} is out of range or malformed")))
}

fn decimal(items: &[Item], key: &[u8]) -> io::Result<Option<Decimal>> {
    field(items, key)?
        .map(|value| Decimal::parse(scalar(value)?))
        .transpose()
}

fn convoy_equipment(country: &[Item]) -> io::Result<Option<Vec<EquipmentAmount>>> {
    let Some(value) = field(country, b"convoys")? else {
        return Ok(None);
    };
    let equipment = required_block(block(value)?, b"equipment")?;
    let mut result = Vec::new();
    for entry in equipment
        .iter()
        .filter(|entry| entry.key.as_deref() == Some(b"equipment"))
    {
        let entry = block(&entry.value)?;
        let id = required_block(entry, b"id")?;
        result.push(EquipmentAmount {
            id: integer(&required_bytes(id, b"id")?)?,
            id_type: integer(&required_bytes(id, b"type")?)?,
            amount: Decimal::parse(&required_bytes(entry, b"amount")?)?,
        });
    }
    Ok(Some(result))
}

fn parse_country(tag: &[u8], items: &[Item]) -> io::Result<Country> {
    let politics = field(items, b"politics")?.map(block).transpose()?;
    Ok(Country {
        major: field(items, b"major")?
            .map(boolean)
            .transpose()?
            .unwrap_or(false),
        tag: tag.to_vec(),
        cosmetic_tag: bytes(items, b"cosmetic_tag")?,
        original_tag: bytes(items, b"original_tag")?,
        political_power: politics
            .map(|items| decimal(items, b"political_power"))
            .transpose()?
            .flatten(),
        command_power: decimal(items, b"command_power")?,
        stability: decimal(items, b"stability")?,
        war_support: decimal(items, b"war_support")?,
        manpower_residual: field(items, b"manpower")?
            .map(block)
            .transpose()?
            .map(|fields| bytes(fields, b"current"))
            .transpose()?
            .flatten()
            .map(|value| integer(&value))
            .transpose()?
            .unwrap_or(0),
        recruitment_ratio: field(items, b"manpower")?
            .map(block)
            .transpose()?
            .map(|fields| bytes(fields, b"ratio"))
            .transpose()?
            .flatten()
            .map(|value| integer(&value))
            .transpose()?,
        exile_manpower: field(items, b"manpower")?
            .map(block)
            .transpose()?
            .map(|fields| bytes(fields, b"exile_manpower"))
            .transpose()?
            .flatten()
            .map(|value| integer(&value))
            .transpose()?,
        exile_manpower_cap: field(items, b"manpower")?
            .map(block)
            .transpose()?
            .map(|fields| bytes(fields, b"max_exile_manpower"))
            .transpose()?
            .flatten()
            .map(|value| integer(&value))
            .transpose()?,
        convoy_equipment: convoy_equipment(items)?,
        instances_counter: bytes(items, b"instances_counter")?
            .map(|bytes| integer(&bytes))
            .transpose()?,
        volunteer_permissions: volunteer_permissions(items)?,
        naval_mines: naval_mines(items)?,
        naval_region_access: naval_region_access(items)?,
        regional_convoy_efficiency: regional_convoy_efficiency(items)?,
        cores: integer_list(items, b"cores")?,
        claims: integer_list(items, b"claims")?,
        civil_war_targets: items
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"civil_war_target"))
            .map(|item| Ok(scalar(&item.value)?.to_vec()))
            .collect::<io::Result<_>>()?,
        stability_terms: StabilityTerms {
            propaganda_stability_penalty: decimal(items, b"propaganda_stability_penalty")?,
            coastal_protection_ratio: decimal(items, b"coastal_protection_ratio")?,
            being_bombed_support_penalty: decimal(items, b"being_bombed_support_penalty")?,
            heroes_dying_war_support_penalty: decimal(items, b"heroes_dying_war_support_penalty")?,
            convoy_raiding_war_support_penalty: decimal(
                items,
                b"convoy_raiding_war_support_penalty",
            )?,
            propaganda_war_support_penalty: decimal(items, b"propaganda_war_support_penalty")?,
        },
        refresh: bytes(items, b"refresh")?
            .map(|bytes| integer(&bytes))
            .transpose()?,
    })
}

fn naval_region_access(items: &[Item]) -> io::Result<Option<Vec<(u32, u8)>>> {
    let Some(navy) = field(items, b"strategic_navy")? else {
        return Ok(None);
    };
    let Some(access) = field(block(navy)?, b"per_region_access")? else {
        return Ok(Some(Vec::new()));
    };
    let values = block(access)?;
    if values.len() % 2 != 0 || values.iter().any(|item| item.key.is_some()) {
        return Err(invalid(
            "per_region_access is not a flat list of region and mode pairs",
        ));
    }
    values
        .chunks_exact(2)
        .map(|pair| {
            Ok((
                integer(scalar(&pair[0].value)?)?,
                integer(scalar(&pair[1].value)?)?,
            ))
        })
        .collect::<io::Result<Vec<_>>>()
        .map(Some)
}

fn regional_convoy_efficiency(items: &[Item]) -> io::Result<Option<BTreeMap<u32, Decimal>>> {
    let Some(navy) = field(items, b"strategic_navy")? else {
        return Ok(None);
    };
    let Some(records) = field(block(navy)?, b"regional_convoys")? else {
        return Ok(Some(BTreeMap::new()));
    };
    let mut out = BTreeMap::new();
    let mut records = block(records)?.iter();
    let count = records
        .next()
        .ok_or_else(|| invalid("regional_convoys lacks its record count"))?;
    if count.key.is_some() {
        return Err(invalid(
            "regional_convoys does not lead with its record count",
        ));
    }
    let count: usize = integer(scalar(&count.value)?)?;
    let mut seen = 0;
    for record in records {
        if record.key.is_some() {
            return Err(invalid("a regional convoy record is keyed"));
        }
        let fields = block(&record.value)?;
        let region = integer::<u32>(&required_bytes(fields, b"index")?)?;
        seen += 1;
        if let Some(efficiency) = decimal(required_block(fields, b"data")?, b"efficiency")?
            && out.insert(region, efficiency).is_some()
        {
            return Err(invalid("regional_convoys repeats a region"));
        }
    }
    if seen > count {
        return Err(invalid(
            "regional_convoys holds more records than its count",
        ));
    }
    Ok(Some(out))
}

fn naval_mines(items: &[Item]) -> io::Result<Option<BTreeMap<u32, Decimal>>> {
    let Some(navy) = field(items, b"strategic_navy")? else {
        return Ok(None);
    };
    let Some(values) = field(block(navy)?, b"per_region_mines")? else {
        return Ok(None);
    };
    let mut mines = BTreeMap::new();
    let mut pairs = block(values)?.chunks_exact(2);
    for pair in &mut pairs {
        if pair[0].key.is_some() || pair[1].key.is_some() {
            return Err(invalid("per_region_mines holds keyed items"));
        }
        let region = integer::<u32>(scalar(&pair[0].value)?)?;
        let amount = Decimal::parse(scalar(&pair[1].value)?)?;
        if region == 0 || amount.raw <= 0 || mines.insert(region, amount).is_some() {
            return Err(invalid("per_region_mines region or amount invalid"));
        }
    }
    if !pairs.remainder().is_empty() {
        return Err(invalid("per_region_mines has an unmatched region"));
    }
    Ok(Some(mines))
}

fn typed_handle(value: &Value) -> io::Result<u64> {
    let fields = block(value)?;
    let id: u32 = integer(&required_bytes(fields, b"id")?)?;
    let kind: u32 = integer(&required_bytes(fields, b"type")?)?;
    let handle = (u64::from(id) << 32) | u64::from(kind);
    if handle == 0 {
        return Err(invalid(
            "serialized object has a null typed handle; refusing to confuse an unregistered object with an omitted pointer",
        ));
    }
    Ok(handle)
}

fn null_permitted_handle(value: &Value) -> io::Result<u64> {
    let fields = block(value)?;
    let id: u32 = integer(&required_bytes(fields, b"id")?)?;
    let kind: u32 = integer(&required_bytes(fields, b"type")?)?;
    Ok((u64::from(id) << 32) | u64::from(kind))
}

fn volunteer_permissions(items: &[Item]) -> io::Result<Vec<VolunteerPermission>> {
    items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"received_air_volunteer_permission"))
        .map(|item| {
            let fields = block(&item.value)?;
            Ok(VolunteerPermission {
                tag: required_bytes(fields, b"tag")?,
                count: integer(&required_bytes(fields, b"count")?)?,
            })
        })
        .collect()
}

fn boolean(value: &Value) -> io::Result<bool> {
    match scalar(value)? {
        b"yes" => Ok(true),
        b"no" => Ok(false),
        _ => Err(invalid("save boolean must be yes or no")),
    }
}

fn parse_war_relation(
    roster_owner: &[u8],
    target: &[u8],
    items: &[Item],
) -> io::Result<WarRelation> {
    let cancel = field(items, b"cancel")?.map(boolean).transpose()?;
    if cancel == Some(false) {
        return Err(invalid(
            "war relation explicitly saves cancel=no; native writer omits an unset cancel flag",
        ));
    }
    let cancel = cancel.unwrap_or(false);
    let start_date = bytes(items, b"start_date")?
        .map(|bytes| GameDate::parse(&bytes))
        .transpose()?;
    if cancel == start_date.is_some() {
        return Err(invalid(
            "war relation must save start_date exactly when cancel is unset",
        ));
    }
    let end_date = bytes(items, b"end_date")?
        .map(|bytes| GameDate::parse(&bytes))
        .transpose()?;
    if let (Some(start), Some(end)) = (start_date, end_date)
        && end.engine_hour()? <= start.engine_hour()?
    {
        return Err(invalid(
            "war relation saves end_date only when it is after start_date",
        ));
    }
    let participation = |key: &[u8]| -> io::Result<Option<war_participation::Ledger>> {
        field(items, key)?
            .map(|value| war_participation::parse(block(value)?))
            .transpose()
    };
    Ok(WarRelation {
        participation: [
            participation(b"war_score_first_vs_second")?,
            participation(b"war_score_second_vs_first")?,
        ],
        first_goals: war_goals::references(items, b"first_wargoals")?,
        second_goals: war_goals::references(items, b"second_wargoals")?,
        roster_owner: roster_owner.to_vec(),
        target: target.to_vec(),
        first: required_bytes(items, b"first")?,
        second: required_bytes(items, b"second")?,
        start_date,
        end_date,
        cancel,
        first_casualties: integer(&required_bytes(items, b"first_casualties")?)?,
        second_casualties: integer(&required_bytes(items, b"second_casualties")?)?,
        first_unknown_casualties: integer(&required_bytes(items, b"first_unknown_casualties")?)?,
        second_unknown_casualties: integer(&required_bytes(items, b"second_unknown_casualties")?)?,
        first_was_instigator: boolean(
            field(items, b"first_was_instigator")?
                .ok_or_else(|| invalid("save lacks first_was_instigator"))?,
        )?,
    })
}

pub const OWN_TABLE_RELATION_KINDS: [&[u8]; 2] = [b"war_relation", b"puppet"];

fn parse_allocation_counters(items: &[Item]) -> io::Result<AllocationCounters> {
    let value = |key: &[u8]| -> io::Result<i32> { integer(&required_bytes(items, key)?) };
    Ok(AllocationCounters {
        debug_current_ref_id: integer(&required_bytes(items, b"debug_current_ref_id")?)?,
        unit: value(b"unit")?,
        order_index: value(b"order_index")?,
        front_index: value(b"front_index")?,
        theatre_index: value(b"theatre_index")?,
        theater_group_index: value(b"theater_group_index")?,
        military_deployment_line_index: value(b"military_deployment_line_index")?,
        military_deployment_conveyor_index: value(b"military_deployment_conveyor_index")?,
        equipment_variant_index: value(b"equipment_variant_index")?,
        country_leader_index: value(b"country_leader_index")?,
        navy_id: value(b"navy_id")?,
        land_combat_id: value(b"land_combat_id")?,
        multiplayer_random_count: value(b"multiplayer_random_count")?,
    })
}

fn parse_relation_tuple(
    owner: &[u8],
    target: &[u8],
    kind: &[u8],
    ordinal: u32,
    items: &[Item],
) -> io::Result<RelationTuple> {
    let cancel = field(items, b"cancel")?.map(boolean).transpose()?;
    if cancel == Some(false) {
        return Err(invalid(
            "relation explicitly saves cancel=no; native writer omits an unset cancel flag",
        ));
    }
    let cancel = cancel.unwrap_or(false);
    let start_date = bytes(items, b"start_date")?
        .map(|bytes| GameDate::parse(&bytes))
        .transpose()?;
    if cancel == start_date.is_some() {
        return Err(invalid(
            "relation must save start_date exactly when cancel is unset",
        ));
    }
    let end_date = bytes(items, b"end_date")?
        .map(|bytes| GameDate::parse(&bytes))
        .transpose()?;
    if let (Some(start), Some(end)) = (start_date, end_date)
        && end.engine_hour()? <= start.engine_hour()?
    {
        return Err(invalid(
            "relation saves end_date only when it is after start_date",
        ));
    }
    Ok(RelationTuple {
        owner: owner.to_vec(),
        target: target.to_vec(),
        kind: kind.to_vec(),
        ordinal,
        first: required_bytes(items, b"first")?,
        second: required_bytes(items, b"second")?,
        start_date,
        end_date,
        cancel,
    })
}

fn parse_country_intel(items: &[Item]) -> io::Result<Vec<CountryIntelRecord>> {
    let mut records = Vec::with_capacity(items.len());
    for item in items {
        if item.key.is_some() {
            return Err(invalid("country intel list must be positional"));
        }
        let mut record = CountryIntelRecord::default();
        let mut seen = [false; 4];
        for entry in block(&item.value)? {
            let lane = match entry.key.as_deref() {
                Some(b"civilian") => 0,
                Some(b"army") => 1,
                Some(b"navy") => 2,
                Some(b"airforce") => 3,
                _ => return Err(invalid("country intel record has an unexpected field")),
            };
            if std::mem::replace(&mut seen[lane], true) {
                return Err(invalid("country intel record repeats a lane"));
            }
            let raw = Decimal::parse(scalar(&entry.value)?)?.raw;
            if raw == 0 {
                return Err(invalid(
                    "country intel record saves a zero lane; the native writer omits it",
                ));
            }
            *[
                &mut record.civilian,
                &mut record.army,
                &mut record.navy,
                &mut record.air,
            ][lane] = raw;
        }
        records.push(record);
    }
    Ok(records)
}

fn parse_faction(items: &[Item]) -> io::Result<Faction> {
    let id =
        typed_handle(field(items, b"id")?.ok_or_else(|| invalid("faction lacks its typed id"))?)?;
    let mut members = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(value) = field(items, b"members")? {
        for item in block(value)? {
            if item.key.is_some() {
                return Err(invalid("faction members must be an ordinary list of tags"));
            }
            let tag = scalar(&item.value)?;
            if tag.is_empty() {
                return Err(invalid("faction member has an empty country tag"));
            }
            if !seen.insert(tag) {
                return Err(invalid("duplicate country tag in saved faction members"));
            }
            members.push(tag.to_vec());
        }
    }
    let color = field(items, b"color")?
        .map(|value| {
            block(value)?
                .iter()
                .map(|item| {
                    if item.key.is_some() {
                        return Err(invalid("saved faction colour has a keyed entry"));
                    }
                    integer::<i64>(scalar(&item.value)?)
                })
                .collect::<io::Result<Vec<_>>>()
        })
        .transpose()?;
    Ok(Faction {
        goals: faction_goals::parse(items)?,
        companions: faction_companions::parse(items)?,
        id,
        name: bytes(items, b"name")?,
        members,
        icon: bytes(items, b"icon")?,
        template: bytes(items, b"template")?,
        color,
        color_overridden: field(items, b"is_color_overridden")?
            .map(boolean)
            .transpose()?,
        theatres: parse_faction_theatres(items)?,
        advisor_slots: parse_faction_advisor_slots(items)?,
    })
}

fn parse_faction_advisor_slots(items: &[Item]) -> io::Result<Vec<Option<Vec<u8>>>> {
    let Some(upgrades) = field(items, b"upgrades")? else {
        return Ok(Vec::new());
    };
    let Some(slots) = field(block(upgrades)?, b"slots")? else {
        return Ok(Vec::new());
    };
    let slots = block(slots)?;
    if slots.is_empty() {
        return Err(invalid(
            "faction upgrade writer emits advisor slots only for a nonzero count",
        ));
    }
    slots
        .iter()
        .map(|slot| {
            if slot.key.is_some() {
                return Err(invalid("faction advisor slots must be anonymous blocks"));
            }
            bytes(block(&slot.value)?, b"advisor")
        })
        .collect()
}

fn parse_faction_theatres(items: &[Item]) -> io::Result<Vec<FactionTheatre>> {
    let Some(pings) = field(items, b"pings")? else {
        return Ok(Vec::new());
    };
    let pings = block(pings)?;
    if pings
        .iter()
        .any(|item| item.key.as_deref() != Some(b"regions"))
    {
        return Err(invalid("faction pings has a field other than regions"));
    }
    let Some(regions) = field(pings, b"regions")? else {
        return Ok(Vec::new());
    };
    block(regions)?
        .iter()
        .map(|item| {
            if item.key.is_some() {
                return Err(invalid("faction theatres must be an ordinary list"));
            }
            let theatre = block(&item.value)?;
            Ok(FactionTheatre {
                name: bytes(theatre, b"name")?,
                tag: bytes(theatre, b"tag")?,
                commander: field(theatre, b"commander")?
                    .map(typed_handle)
                    .transpose()?,
                regions: integer_list(theatre, b"ids")?,
                countries: tag_list(theatre, b"countries")?,
                hidden: tag_list(theatre, b"hidden")?,
            })
        })
        .collect()
}

fn tag_list(items: &[Item], key: &[u8]) -> io::Result<Vec<Vec<u8>>> {
    let Some(value) = field(items, key)? else {
        return Ok(Vec::new());
    };
    block(value)?
        .iter()
        .map(|item| {
            let tag = scalar(&item.value)?;
            if item.key.is_some() || tag.is_empty() {
                return Err(invalid("saved country tag list has a keyed or empty entry"));
            }
            Ok(tag.to_vec())
        })
        .collect()
}

fn parse_players(items: &[Item]) -> io::Result<Vec<Player>> {
    let Some(players) = field(items, b"player_countries")? else {
        return Ok(Vec::new());
    };
    block(players)?
        .iter()
        .map(|item| {
            let country = item
                .key
                .clone()
                .filter(|key| !key.is_empty())
                .ok_or_else(|| invalid("saved player lacks country key"))?;
            let entry = block(&item.value)?;
            Ok(Player {
                country,
                user: bytes(entry, b"user")?.ok_or_else(|| invalid("saved player lacks user"))?,
                country_leader: boolean(
                    field(entry, b"country_leader")?
                        .ok_or_else(|| invalid("saved player lacks country_leader"))?,
                )?,
                id: field(entry, b"id")?
                    .map(|value| integer(scalar(value)?))
                    .transpose()?
                    .unwrap_or(-1),
            })
        })
        .collect()
}

fn object_id(items: &[Item]) -> io::Result<u64> {
    let mut id = None;
    for item in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"id"))
    {
        let handle = typed_handle(&item.value)?;
        if id.is_some_and(|previous| previous != handle) {
            return Err(invalid("object has conflicting repeated typed ids"));
        }
        id = Some(handle);
    }
    id.ok_or_else(|| invalid("saved object lacks typed id"))
}

fn token_list(items: &[Item], key: &[u8]) -> io::Result<Vec<Vec<u8>>> {
    let Some(value) = field(items, key)? else {
        return Ok(Vec::new());
    };
    block(value)?
        .iter()
        .map(|item| {
            let token = scalar(&item.value)?;
            if item.key.is_some() || token.is_empty() {
                return Err(invalid("saved token list has a keyed or empty entry"));
            }
            Ok(token.to_vec())
        })
        .collect()
}

fn state_strategic_locations(state: &[Item]) -> io::Result<Vec<(Vec<u8>, u32)>> {
    let Some(value) = field(state, b"strategic_region_data")? else {
        return Ok(Vec::new());
    };
    block(value)?
        .iter()
        .map(|item| {
            let [location, province] = block(&item.value)? else {
                return Err(invalid(
                    "saved strategic location is not one location and province",
                ));
            };
            if item.key.is_some() || location.key.is_some() || province.key.is_some() {
                return Err(invalid("saved strategic location has a keyed entry"));
            }
            let location = scalar(&location.value)?;
            if location.is_empty() {
                return Err(invalid("saved strategic location lacks its location"));
            }
            Ok((
                location.to_vec(),
                integer::<i32>(scalar(&province.value)?)? as u32,
            ))
        })
        .collect()
}

fn integer_list(items: &[Item], key: &[u8]) -> io::Result<Vec<u32>> {
    let Some(value) = field(items, key)? else {
        return Ok(Vec::new());
    };
    block(value)?
        .iter()
        .map(|item| {
            if item.key.is_some() {
                return Err(invalid("saved integer list has keyed entry"));
            }
            Ok(integer::<i32>(scalar(&item.value)?)? as u32)
        })
        .collect()
}

pub fn parse(input: &[u8]) -> io::Result<Save> {
    SaveDocument::read(input)?.parse()
}

fn parse_items(items: &[Item], country_view: CountriesView<'_>) -> io::Result<Save> {
    let date = GameDate::parse(&required_bytes(items, b"date")?)?;
    let campaign_id = required_bytes(items, b"game_unique_id")?;
    let version = required_bytes(items, b"version")?;
    let seed = integer(&required_bytes(items, b"game_unique_seed")?)?;
    let counters = parse_allocation_counters(items)?;
    let mut countries = BTreeMap::new();
    let mut divisions = BTreeMap::new();
    let mut railway_guns = BTreeMap::new();
    let mut division_template_owners = BTreeMap::new();
    let mut fleets = BTreeMap::new();
    let mut task_forces = BTreeMap::new();
    let mut ships = BTreeMap::new();
    let mut army_orders = BTreeMap::new();
    let mut plans = plans::Tables::default();
    let mut war_relations = Vec::new();
    let mut war_goals = BTreeMap::new();
    let mut wargoal_countries = BTreeMap::new();
    let mut war_pairs = BTreeSet::new();
    let mut relation_opinions = Vec::new();
    let mut war_delivery_histories = Vec::new();
    let mut relation_tuples = Vec::new();
    let mut country_intel: BTreeMap<Vec<u8>, Vec<CountryIntelRecord>> = BTreeMap::new();
    for item in country_view.required()? {
        let tag = item
            .key
            .as_deref()
            .filter(|tag| !tag.is_empty())
            .ok_or_else(|| invalid("country has no tag"))?;
        let country_items = block(&item.value)?;
        war_goals::objects(country_items, &mut war_goals)?;
        wargoal_countries.insert(tag.to_vec(), war_goals::inventory(country_items)?);
        for item in country_items
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"division_template_id"))
        {
            let template = typed_handle(&item.value)?;
            if division_template_owners
                .insert(template, tag.to_vec())
                .is_some()
            {
                return Err(invalid("duplicate template handle across country rosters"));
            }
        }
        parse_orders(tag, country_items, &mut army_orders)?;
        plans.country(country_items)?;
        let country = parse_country(tag, country_items).map_err(|error| {
            invalid(format!("country {}: {error}", String::from_utf8_lossy(tag)))
        })?;
        if countries.insert(tag.to_vec(), country).is_some() {
            return Err(invalid("duplicate country tag in save"));
        }
        if let Some(units) = field(country_items, b"units")? {
            for item in block(units)?
                .iter()
                .filter(|item| item.key.as_deref() == Some(b"railway_gun"))
            {
                let gun = parse_railway_gun(tag, block(&item.value)?).map_err(|error| {
                    invalid(format!(
                        "country {} railway gun: {error}",
                        String::from_utf8_lossy(tag)
                    ))
                })?;
                if railway_guns.insert(gun.id, gun).is_some() {
                    return Err(invalid(
                        "duplicate full railway gun handle across saved rosters",
                    ));
                }
            }
            for item in block(units)?
                .iter()
                .filter(|item| item.key.as_deref() == Some(b"division"))
            {
                let division = parse_division(tag, block(&item.value)?).map_err(|error| {
                    invalid(format!(
                        "country {} division: {error}",
                        String::from_utf8_lossy(tag)
                    ))
                })?;
                if divisions.insert(division.id, division).is_some() {
                    return Err(invalid(
                        "duplicate full division handle across saved rosters",
                    ));
                }
            }
            for item in block(units)?
                .iter()
                .filter(|item| item.key.as_deref() == Some(b"fleet"))
            {
                let fields = block(&item.value)?;
                let fleet = parse_fleet(tag, fields)?;
                for item in fields
                    .iter()
                    .filter(|item| item.key.as_deref() == Some(b"task_force"))
                {
                    let fields = block(&item.value)?;
                    let task_force = parse_task_force(tag, fleet.id, fields)?;
                    for (ordinal, item) in fields
                        .iter()
                        .filter(|item| item.key.as_deref() == Some(b"ship"))
                        .enumerate()
                    {
                        let mut ship = parse_ship(task_force.id, block(&item.value)?)?;
                        ship.roster_ordinal = u32::try_from(ordinal)
                            .map_err(|_| invalid("ship roster exceeds u32"))?;
                        if ships.insert(ship.id, ship).is_some() {
                            return Err(invalid("duplicate full ship handle in save"));
                        }
                    }
                    if task_forces.insert(task_force.id, task_force).is_some() {
                        return Err(invalid("duplicate full task force handle in save"));
                    }
                }
                if fleets.insert(fleet.id, fleet).is_some() {
                    return Err(invalid("duplicate full fleet handle in save"));
                }
            }
        }
        if let Some(diplomacy) = field(country_items, b"diplomacy")?
            && let Some(relations) = field(block(diplomacy)?, b"active_relations")?
        {
            for item in block(relations)? {
                let target = item
                    .key
                    .as_deref()
                    .filter(|tag| !tag.is_empty())
                    .ok_or_else(|| invalid("active relation has no target country tag"))?;
                let record = block(&item.value)?;
                let context = |error: io::Error| {
                    invalid(format!(
                        "country {} relation {}: {error}",
                        String::from_utf8_lossy(tag),
                        String::from_utf8_lossy(target)
                    ))
                };
                if let Some(war) = field(record, b"war_relation")? {
                    let war = parse_war_relation(tag, target, block(war)?).map_err(context)?;
                    if !war_pairs.insert((war.first.clone(), war.second.clone())) {
                        return Err(invalid("duplicate ordered war relation pair in save"));
                    }
                    war_relations.push(war);
                }
                if let Some(history) = field(record, b"lend_lease_to_allies_history")? {
                    war_delivery_histories.push(
                        war_participation::parse_delivery_history(tag, target, block(history)?)
                            .map_err(context)?,
                    );
                }
                if let Some(sum) = field(record, b"cached_sum")? {
                    relation_opinions.push(RelationOpinion {
                        owner: tag.to_vec(),
                        target: target.to_vec(),
                        cached_sum: integer(scalar(sum)?).map_err(context)?,
                    });
                }
                let mut ordinal = 0;
                for entry in record {
                    let Some(kind) = entry.key.as_deref() else {
                        continue;
                    };
                    let Value::Block(fields) = &entry.value else {
                        continue;
                    };
                    if field(fields, b"first")?.is_none() || field(fields, b"second")?.is_none() {
                        continue;
                    }
                    let this = ordinal;
                    ordinal += 1;
                    if kind == b"war_relation" || kind == OWN_TABLE_RELATION_KINDS[1] {
                        continue;
                    }
                    relation_tuples.push(
                        parse_relation_tuple(tag, target, kind, this, fields).map_err(context)?,
                    );
                }
            }
        }
        if let Some(intel) = field(country_items, b"intel")?
            && let Some(list) = field(block(intel)?, b"intel")?
        {
            let records = parse_country_intel(block(list)?).map_err(|error| {
                invalid(format!(
                    "country {} intel: {error}",
                    String::from_utf8_lossy(tag)
                ))
            })?;
            if records.is_empty() {
                return Err(invalid(
                    "save writes an intel list only for a positive count",
                ));
            }
            if country_intel.insert(tag.to_vec(), records).is_some() {
                return Err(invalid("duplicate country intel list in save"));
            }
        }
    }
    let mut factions = BTreeMap::new();
    for item in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"faction"))
    {
        let faction = parse_faction(block(&item.value)?)?;
        if factions.insert(faction.id, faction).is_some() {
            return Err(invalid("duplicate full faction handle in save"));
        }
    }
    let mut land_combats = BTreeMap::new();
    let mut naval_combats = BTreeMap::new();
    let mut naval_combat_participants = BTreeMap::new();
    if let Some(combat) = field(items, b"combat")? {
        for item in block(combat)?.iter().filter(|item| {
            matches!(
                item.key.as_deref(),
                Some(b"land_combat" | b"border_war_combat")
            )
        }) {
            let combat = parse_land_combat(
                block(&item.value)?,
                item.key.as_deref() == Some(b"border_war_combat"),
            )?;
            if land_combats.insert(combat.id, combat).is_some() {
                return Err(invalid("duplicate full land combat handle in save"));
            }
        }
        for item in block(combat)?
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"naval_combat"))
        {
            let combat = parse_naval_combat(block(&item.value)?, &mut naval_combat_participants)?;
            if naval_combats.insert(combat.id, combat).is_some() {
                return Err(invalid("duplicate full naval combat handle in save"));
            }
        }
    }
    let mut states = BTreeMap::new();
    for item in required_block(items, b"states")? {
        let id = integer(
            item.key
                .as_deref()
                .ok_or_else(|| invalid("state has no id"))?,
        )?;
        let state = block(&item.value)?;
        if states
            .insert(
                id,
                State {
                    id,
                    owner: bytes(state, b"owner")?,
                    controller: bytes(state, b"controller")?,
                    demilitarized: field(state, b"demilitarized")?.map(boolean).transpose()?,
                    is_border_conflict: field(state, b"is_border_conflict")?
                        .map(boolean)
                        .transpose()?,
                    name: bytes(state, b"name")?,
                    category: bytes(state, b"state_category")?,
                    manpower_available: field(state, b"manpower_pool")?
                        .map(|value| integer(&required_bytes(block(value)?, b"available")?))
                        .transpose()?,
                    manpower_locked: field(state, b"manpower_pool")?
                        .map(|value| {
                            bytes(block(value)?, b"locked")?
                                .map(|value| integer(&value))
                                .transpose()
                        })
                        .transpose()?
                        .flatten(),
                    population: field(state, b"manpower_pool")?
                        .map(|value| {
                            bytes(block(value)?, b"total")?
                                .map(|total| integer(&total))
                                .transpose()
                        })
                        .transpose()?,
                    strategic_locations: state_strategic_locations(state)?,
                    contested_owners: contested_owners(state)?,
                },
            )
            .is_some()
        {
            return Err(invalid("duplicate state id in save"));
        }
    }
    let mut provinces = BTreeMap::new();
    for item in required_block(items, b"provinces")? {
        let id = integer(
            item.key
                .as_deref()
                .ok_or_else(|| invalid("province has no id"))?,
        )?;
        let province = block(&item.value)?;
        if provinces
            .insert(
                id,
                ProvinceOverride {
                    id,
                    name: bytes(province, b"name")?,
                    owner: bytes(province, b"owner")?,
                    controller: bytes(province, b"controller")?,
                    victory_points: bytes(province, b"victory_points")?
                        .map(|value| integer(&value))
                        .transpose()?,
                    strategic_locations: token_list(province, b"strategic_province_location")?,
                },
            )
            .is_some()
        {
            return Err(invalid("duplicate province id in save"));
        }
    }
    let mut equipment_definitions = BTreeMap::new();
    if let Some(equipments) = field(items, b"equipments")? {
        for item in block(equipments)? {
            let definition = item
                .key
                .clone()
                .filter(|key| !key.is_empty())
                .ok_or_else(|| invalid("saved equipment has no definition key"))?;
            let id = required_block(block(&item.value)?, b"id")?;
            let key = (
                integer(&required_bytes(id, b"id")?)?,
                integer(&required_bytes(id, b"type")?)?,
            );
            if equipment_definitions.insert(key, definition).is_some() {
                return Err(invalid("duplicate typed equipment id in save"));
            }
        }
    }
    Ok(Save {
        date,
        campaign_id,
        version,
        seed,
        counters,
        countries,
        country_emissions: None,
        states,
        provinces,
        divisions,
        railway_guns,
        division_templates: parse_division_templates(items)?,
        division_template_owners,
        war_relations,
        war_goals,
        wargoal_countries,
        relation_opinions,
        war_delivery_histories,
        relation_tuples,
        country_intel,
        country_naval_launch: naval_launch::parse_country_occurrences(required_block(
            items,
            b"countries",
        )?)?,
        factions,
        faction_accounts: faction_accounts::parse(items)?,
        player: bytes(items, b"player")?,
        players: parse_players(items)?,
        land_combats,
        fleets,
        naval_management: naval_management::parse_with_countries(items, country_view)
            .map_err(|error| pack_error("naval management", error))?,
        task_forces,
        ships,
        army_orders,
        plans,
        air_wings: parse_air_wings(items)?,
        naval_combats,
        naval_combat_participants,
        equipment_definitions,
        saved_event_targets: saved_event_targets::parse(items)?,
        air: air::parse_with_countries(items, country_view)
            .map_err(|error| pack_error("air", error))?,
        characters: characters::parse_with_countries(items, country_view)
            .map_err(|error| pack_error("characters", error))?,
        combat_log: combat_log::parse(items).map_err(|error| pack_error("combat log", error))?,
        construction: construction::parse_with_countries(items, country_view)
            .map_err(|error| pack_error("construction", error))?,
        economy: economy::parse_with_countries(items, country_view)
            .map_err(|error| pack_error("economy", error))?,
        governance: governance::parse_with_countries(items, country_view)
            .map_err(|error| pack_error("governance", error))?,
        intel: intel::parse_with_countries(items, country_view)
            .map_err(|error| pack_error("intel", error))?,
        naval_history: naval_history::parse_with_countries(items, country_view)
            .map_err(|error| pack_error("naval history", error))?,
        production: production::parse_with_countries(items, country_view)
            .map_err(|error| pack_error("production", error))?,
        projects: projects::parse_with_countries(items, country_view)
            .map_err(|error| pack_error("projects", error))?,
        session: session::parse(items).map_err(|error| pack_error("session", error))?,
        weather: weather::parse(items).map_err(|error| pack_error("weather", error))?,
        counter_inputs: counter_inputs::parse_with_countries(items, country_view)
            .map_err(|error| pack_error("counter inputs", error))?,
        country_rosters: country_rosters::parse_with_countries(items, country_view)
            .map_err(|error| pack_error("country rosters", error))?,
        naval_missions: naval_missions::parse_with_countries(items, country_view)
            .map_err(|error| pack_error("naval missions", error))?,
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapMembership {
    pub provinces: BTreeMap<u32, Option<u32>>,
}

impl MapMembership {
    pub fn from_definition(bytes: &[u8]) -> io::Result<Self> {
        let mut provinces = BTreeMap::new();
        for row in content_reader_map::map::definition_rows(bytes)? {
            if provinces.insert(u32::from(row.id), None).is_some() {
                return Err(invalid("duplicate province id in definition.csv"));
            }
        }
        if provinces.is_empty() {
            return Err(invalid("definition.csv contains no provinces"));
        }
        Ok(Self { provinces })
    }

    pub fn from_definitions(
        definitions: &BTreeMap<u32, content_reader_map::map::ProvinceDefinition>,
    ) -> io::Result<Self> {
        if definitions.is_empty() {
            return Err(invalid("definition.csv contains no provinces"));
        }
        Ok(Self {
            provinces: definitions.keys().map(|&id| (id, None)).collect(),
        })
    }

    pub fn add_state_history(
        &mut self,
        state: &content_reader_states::states::StateDefinition,
    ) -> io::Result<()> {
        let id = state.id;
        if id == 0
            || id > i32::MAX as u32
            || self.provinces.values().any(|&state| state == Some(id))
        {
            return Err(invalid("duplicate or invalid state id in state history"));
        }
        let mut assignments = std::collections::BTreeSet::new();
        for &province in &state.provinces {
            if province == 0 || self.provinces.get(&province) != Some(&None) {
                return Err(invalid(format!(
                    "state {id} uses unknown or already assigned province {province}"
                )));
            }
            assignments.insert(province);
        }
        if assignments.is_empty() {
            return Err(invalid(format!("state {id} history contains no provinces")));
        }
        for province in assignments {
            self.provinces.insert(province, Some(id));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProvinceMembership {
    Known(Option<u32>),
    Unknown(BTreeSet<u32>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExcludedState {
    pub file: String,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PartialMapMembership {
    pub provinces: BTreeMap<u32, ProvinceMembership>,
    pub known_states: BTreeSet<u32>,
    pub excluded_states: BTreeMap<u32, ExcludedState>,
    pub notes: Vec<String>,
}

impl From<MapMembership> for PartialMapMembership {
    fn from(membership: MapMembership) -> Self {
        let known_states = membership.provinces.values().flatten().copied().collect();
        Self {
            provinces: membership
                .provinces
                .into_iter()
                .map(|(id, state)| (id, ProvinceMembership::Known(state)))
                .collect(),
            known_states,
            ..Self::default()
        }
    }
}

impl PartialMapMembership {
    pub fn province_is_unknown(&self, id: u32) -> bool {
        matches!(
            self.provinces.get(&id),
            Some(ProvinceMembership::Unknown(_))
        )
    }

    pub fn state_is_excluded(&self, id: u32) -> bool {
        self.excluded_states.contains_key(&id)
    }

    pub fn contains_province(&self, id: u32) -> bool {
        self.provinces.contains_key(&id)
    }
}

pub fn read_partial_map_membership(layers: &ResolvedLayers) -> io::Result<PartialMapMembership> {
    let (provinces, states, notes) = content_state_history(layers)?;
    partial_map_membership(&provinces, &states, notes)
}

pub fn read_partial_state_history(
    layers: &ResolvedLayers,
) -> io::Result<(
    PartialMapMembership,
    io::Result<construction::PartialBaseResources>,
)> {
    let (provinces, states, notes) = content_state_history(layers)?;
    let membership = partial_map_membership(&provinces, &states, notes)?;
    let resources = construction::PartialBaseResources::from_states(&states, &membership);
    Ok((membership, resources))
}

fn partial_map_membership(
    provinces: &BTreeMap<u32, content_reader_map::map::ProvinceDefinition>,
    states: &content_reader_states::states::States,
    notes: Vec<content_reader_parser::Diagnostic>,
) -> io::Result<PartialMapMembership> {
    let mut known = MapMembership::from_definitions(provinces)?;
    let mut excluded_states = BTreeMap::new();
    let mut unknown_provinces: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for (&id, state) in states.definitions() {
        let reasons = states.diagnostics(id);
        if !reasons.is_empty() {
            excluded_states.insert(
                id,
                ExcludedState {
                    file: states
                        .source(id)
                        .expect("kept template has a source")
                        .to_string(),
                    reasons: reasons.to_vec(),
                },
            );
            for &province in &state.provinces {
                if !known.provinces.contains_key(&province) {
                    return Err(invalid(format!(
                        "excluded state {id} uses unknown province {province}"
                    )));
                }
                unknown_provinces.entry(province).or_default().insert(id);
            }
        }
    }
    let mut known_states = BTreeSet::new();
    for (&id, state) in states.definitions() {
        if excluded_states.contains_key(&id) {
            continue;
        }
        known.add_state_history(state)?;
        known_states.insert(id);
    }
    let mut membership = PartialMapMembership {
        known_states,
        excluded_states,
        notes: notes.into_iter().map(|(_, note)| note).collect(),
        ..known.into()
    };
    for (province, excluded) in unknown_provinces {
        membership
            .provinces
            .insert(province, ProvinceMembership::Unknown(excluded));
    }
    Ok(membership)
}

fn content_state_history(
    layers: &ResolvedLayers,
) -> io::Result<(
    BTreeMap<u32, content_reader_map::map::ProvinceDefinition>,
    content_reader_states::states::States,
    Vec<content_reader_parser::Diagnostic>,
)> {
    let provinces = content_reader_map::map::load_definitions(layers)?;
    let defines = content_reader_defines::defines::read(layers)?;
    let (states, notes) = content_reader_states::states::read(layers, &defines, &provinces)?;
    if states.definitions().is_empty() {
        return Err(invalid("verified content contains no state history"));
    }
    Ok((provinces, states, notes))
}

#[cfg(test)]
fn history_fixture(input: &[u8]) -> content_reader_states::states::StateDefinition {
    content_reader_script::test_game::enter();
    let context = content_reader_states::states::Context {
        resource: &|_| content_reader_states::states::Resource::Found,
        max_building_levels: 20,
        province_count: 1000,
        names: &content_reader_script::key_names::KeyNames::default(),
    };
    content_reader_states::states::parse_states(input, &context)
        .unwrap()
        .0
        .remove(0)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod state_history_tests;
