//! Text-save values for the oracle. The shared Clausewitz parser owns syntax;
//! this crate projects the state fields used by recording comparisons.

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

use installed_content_core::{ResolvedLayers, parser::Item, parser::Value};

#[cfg(test)]
use installed_content_core::parser;

pub(crate) const MONTH_DAYS: [u8; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
pub use installed_content_core::{parser::decimal::Decimal, parser::decimal::FIXED_SCALE};

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
    /// Text saves use 1 through 24, not zero through 23.
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

    /// The game's date parser and formatter use a fixed 365-day calendar,
    /// epoch 43,800,000, and zero-based month/day/hour internally.
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

/// A saved date field that may hold the engine's invalid date.
/// Objects initialize unset dates to raw hour 43,791,240, which the serializers
/// test as unset and the formatter writes as `-1.1.1.1`. This is not a
/// supported BCE calendar date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavedDate {
    Calendar(GameDate),
    Invalid,
}

impl SavedDate {
    /// Raw engine hour of the invalid-date sentinel.
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
            // The formatter uses signed year /8760 and hour %24. Month
            // and day use (delta /24) %365, so delta=-8760
            // uniquely writes -1.1.1.1 with epoch 43,800,000.
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

/// One `received_air_volunteer_permission` block of a saved country.
///
/// The country writer emits one block per position of the host
/// vector, in vector order, with the resolved host tag and its paired count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolunteerPermission {
    pub tag: Vec<u8>,
    pub count: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Country {
    pub tag: Vec<u8>,
    /// The country writer emits major only when its stored byte is nonzero.
    pub major: bool,
    /// The country writer emits the cosmetic tag unconditionally.
    /// Empty means none; omission stays distinct for the oracle to reject.
    pub cosmetic_tag: Option<Vec<u8>>,
    /// The country writer emits `original_tag` only when its stored signed
    /// country reference is positive, using the tag's bytes.
    /// Omission certifies nonpositive original; it never means the own tag.
    pub original_tag: Option<Vec<u8>>,
    pub political_power: Option<Decimal>,
    pub command_power: Option<Decimal>,
    pub stability: Option<Decimal>,
    pub war_support: Option<Decimal>,
    /// The manpower writer emits manpower.current only when the signed residual is nonzero.
    pub manpower_residual: i32,
    pub recruitment_ratio: Option<i64>,
    pub exile_manpower: Option<i32>,
    pub exile_manpower_cap: Option<i32>,
    /// Serialized stockpile entries, not an assumed equivalent of a live cache.
    pub convoy_equipment: Option<Vec<EquipmentAmount>>,
    pub instances_counter: Option<u32>,
    /// Ordered host blocks; absence and an empty vector are indistinguishable.
    pub volunteer_permissions: Vec<VolunteerPermission>,
    /// `strategic_navy.per_region_mines` by region. The writer lists only
    /// positive amounts, so a region missing from a present block holds zero.
    pub naval_mines: Option<BTreeMap<u32, Decimal>>,
    /// `strategic_navy.per_region_access` as (strategic region id, mode) pairs in
    /// saved order; `None` when the country saved no strategic navy. An
    /// omitted access child is a known empty list. The writer omits an empty
    /// array and emits only signed-positive byte modes.
    pub naval_region_access: Option<Vec<(u32, u8)>>,
    /// `strategic_navy.regional_convoys`: each saved record's delivery efficiency by
    /// strategic region id; `None` when the country saved no strategic navy. The writer
    /// omits `efficiency` at 1.0 and a whole record whose required convoys are zero,
    /// efficiency is 1.0 and last sinking is unset, so a missing region is full.
    pub regional_convoy_efficiency: Option<BTreeMap<u32, Decimal>>,
    /// Stored stability and war-support terms. The country writer
    /// writes each only when its word is nonzero, so absence is zero. Each
    /// is a five-decimal number: the Q15 penalties truncated toward zero, the
    /// coastal term through the fixed5 writer.
    pub stability_terms: StabilityTerms,
    /// Refresh flags are written unconditionally as `refresh`.
    /// The pre-hourly update consumes them each hour:
    /// bit 1 rebuilds modifiers and recalculates the production status's
    /// factory totals, bit 9 recalculates the totals alone.
    pub refresh: Option<u32>,
    /// `cores`: the IDs of the states the country is a core of, omitted
    /// when empty. The state holds the same membership reciprocally;
    /// loading a save updates both sides.
    pub cores: Vec<u32>,
    /// `claims`: the IDs of the states the country claims, omitted when
    /// empty. The state holds the same membership reciprocally, as it does
    /// for cores.
    pub claims: Vec<u32>,
    /// `civil_war_target`: one tag per entry of the civil-war array, in
    /// array order, written only for a nonzero count.
    pub civil_war_targets: Vec<Vec<u8>>,
}

/// The country's Q15 penalties and coastal term
/// as saved: each a five-decimal number (the writer converts a Q15 word to
/// fixed5 first); `None` is an omitted (zero) word.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StabilityTerms {
    /// The saved `propaganda_stability_penalty` term.
    pub propaganda_stability_penalty: Option<Decimal>,
    /// The saved `coastal_protection_ratio` term.
    pub coastal_protection_ratio: Option<Decimal>,
    /// The saved `being_bombed_support_penalty` term.
    pub being_bombed_support_penalty: Option<Decimal>,
    /// The saved `heroes_dying_war_support_penalty` term.
    pub heroes_dying_war_support_penalty: Option<Decimal>,
    /// The saved `convoy_raiding_war_support_penalty` term.
    pub convoy_raiding_war_support_penalty: Option<Decimal>,
    /// The saved `propaganda_war_support_penalty` term.
    pub propaganda_war_support_penalty: Option<Decimal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub id: u32,
    pub owner: Option<Vec<u8>>,
    /// The literal field only; absence is retained until its native save rule
    /// is applied by known_state_control, independently of province control.
    pub controller: Option<Vec<u8>>,
    /// The state writer emits this boolean unconditionally. Missing is
    /// retained so the oracle can reject it, never silently assumed false.
    pub demilitarized: Option<bool>,
    /// The state writer always emits the border-conflict flag.
    pub is_border_conflict: Option<bool>,
    /// The pool writer emits manpower_pool.available. The state writer omits
    /// the whole pool when total <= 0, so omission cannot prove available is zero.
    pub manpower_available: Option<i32>,
    pub manpower_locked: Option<i32>,
    /// The pool writer's `total`, the state's population: `None` when the
    /// pool is omitted (which certifies only a nonpositive population),
    /// `Some(None)` for a pool written without it.
    pub population: Option<Option<i32>>,
    /// The `state_category` name the state writer emits for the state's
    /// category definition.
    pub category: Option<Vec<u8>>,
    /// The set_state_name rename, emitted as `name` only when the string
    /// is nonempty.
    pub name: Option<Vec<u8>>,
    /// `strategic_region_data`: the state's strategic locations in stored
    /// order, each `{ <location> <anchor province> }`, written only when the
    /// list is nonempty.
    pub strategic_locations: Vec<(Vec<u8>, u32)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvinceOverride {
    pub id: u32,
    /// The province writer emits `name` only when its override is nonempty.
    pub name: Option<Vec<u8>>,
    pub owner: Option<Vec<u8>>,
    pub controller: Option<Vec<u8>>,
    /// The province writer emits the signed victory-point integer only when nonzero.
    pub victory_points: Option<i32>,
    /// `strategic_province_location`: the province's own strategic locations
    /// in stored order, written only when the list is nonempty.
    pub strategic_locations: Vec<Vec<u8>>,
}

/// The engine's monotonic allocation counters as the save header serializes
/// them. The header serializer emits every key unconditionally, so an
/// absent key is a header change rather than a default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AllocationCounters {
    /// Written through the unsigned formatter, unlike every other counter.
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

/// One `countries.<tag>.diplomacy.active_relations.<target>` record's cached
/// opinion. The relation-status writer emits `cached_sum` only when the
/// native dword is nonzero, so an absent key proves an exact zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationOpinion {
    pub owner: Vec<u8>,
    pub target: Vec<u8>,
    pub cached_sum: i32,
}

/// One typed relation block under an `active_relations` target. The key spelling
/// is the relation class's own token, so the comparator joins on it rather than
/// on a reconstructed name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationTuple {
    pub owner: Vec<u8>,
    pub target: Vec<u8>,
    /// The literal block key, which is the relation type token's spelling.
    pub kind: Vec<u8>,
    /// Native order within this target's relation blocks.
    pub ordinal: u32,
    pub first: Vec<u8>,
    pub second: Vec<u8>,
    /// The relation writer omits this exactly when cancel is set.
    pub start_date: Option<GameDate>,
    /// Omission proves end <= start, not a particular raw end-hour default.
    pub end_date: Option<GameDate>,
    pub cancel: bool,
}

/// One record of a country's positional four-lane intel list. The record
/// writer omits a lane exactly when its raw qword is zero and writes every
/// present lane through the fixed5 path, so each lane's raw value is exact.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CountryIntelRecord {
    pub civilian: i64,
    pub army: i64,
    pub navy: i64,
    pub air: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarRelation {
    /// Stored directional ledgers; presence stays explicit for oracle coverage.
    pub participation: [Option<war_participation::Ledger>; 2],
    pub first_goals: Vec<u64>,
    pub second_goals: Vec<u64>,
    /// Enclosing `countries.<tag>`, retained independently of the explicit first.
    pub roster_owner: Vec<u8>,
    /// Enclosing `diplomacy.active_relations.<tag>`, independently of second.
    pub target: Vec<u8>,
    /// Native ordered endpoints; neither country IDs nor instigator sort them.
    pub first: Vec<u8>,
    pub second: Vec<u8>,
    /// The relation writer omits this exactly when cancel is set.
    pub start_date: Option<GameDate>,
    /// Omission proves end <= start, not a particular raw end-hour default.
    pub end_date: Option<GameDate>,
    pub cancel: bool,
    /// The war-relation writer emits all four signed integers unscaled.
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
    /// Full native reference: type low u32, id high u32; only both zero is null.
    pub id: u64,
    /// The faction writer emits the name only for a nonzero stored length;
    /// an emitted empty first-NUL prefix differs from an omitted name.
    pub name: Option<Vec<u8>>,
    /// The native pointer-array order, with its leader first when nonempty.
    pub members: Vec<Vec<u8>>,
    /// The faction writer always emits `icon`.
    pub icon: Option<Vec<u8>>,
    /// `template`: the spelling of the template's lexer token,
    /// written only when the faction has a template.
    pub template: Option<Vec<u8>>,
    /// `color`: each component multiplied by 255.0f and truncated to an integer.
    /// Red, green and blue are included; alpha is included only when it is
    /// not 1.0. None when the save omits it.
    pub color: Option<Vec<i64>>,
    /// `is_color_overridden`, the stored override byte as yes or no.
    pub color_overridden: Option<bool>,
    /// `pings.regions` in saved theatre order.
    pub theatres: Vec<FactionTheatre>,
    /// `upgrades.slots` in native slot order: each slot's `advisor`, the
    /// character token of the slot's advisor, `None` for an empty slot.
    /// The faction writer saves its upgrade status as `upgrades`, whose
    /// writer emits `slots` only for a nonzero slot count, one block per
    /// slot, with `advisor` only for a non-null advisor; omission is no slots.
    pub advisor_slots: Vec<Option<Vec<u8>>>,
}

/// One saved faction theatre. Name, tag, commander, execution type, template
/// and the saved integer list are not drawn by the pin and not kept.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FactionTheatre {
    /// `ids`: strategic region ids as raw i32 bits, in native order.
    pub regions: Vec<u32>,
    /// `countries` tags, in native order.
    pub countries: Vec<Vec<u8>>,
    /// Hidden-pin tags, written under the same `hidden` key the event,
    /// idea-category and achievement readers use.
    pub hidden: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Player {
    pub country: Vec<u8>,
    /// Stored user name, distinct from the unsaved display name.
    pub user: Vec<u8>,
    pub country_leader: bool,
    /// The player writer omits precisely the signed -1 value.
    pub id: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Save {
    pub date: GameDate,
    pub campaign_id: Vec<u8>,
    pub version: Vec<u8>,
    pub seed: i32,
    /// The save header's own allocation counters at the instant it was written.
    pub counters: AllocationCounters,
    pub countries: BTreeMap<Vec<u8>, Country>,
    /// Full physical emission ledger after the witnessed whole-Item gate.
    /// None for an ordinary strict parse without that identity witness.
    pub country_emissions: Option<Vec<SavedCountryEmission>>,
    pub states: BTreeMap<u32, State>,
    /// Sparse: absent entries are not evidence of a missing engine province.
    pub provinces: BTreeMap<u32, ProvinceOverride>,
    /// Full typed handles uniquely identify divisions across every country roster.
    pub divisions: BTreeMap<u64, Division>,
    /// Full typed handles uniquely identify railway guns across country rosters.
    pub railway_guns: BTreeMap<u64, RailwayGun>,
    pub division_templates: BTreeMap<u64, DivisionTemplate>,
    /// The country's explicit division_template_id roster, including unused ones.
    pub division_template_owners: BTreeMap<u64, Vec<u8>>,
    /// One first-owned entry per ordered country pair, not a grouped war ID.
    pub war_relations: Vec<WarRelation>,
    pub war_goals: BTreeMap<u64, war_goals::WarGoal>,
    pub wargoal_countries: BTreeMap<Vec<u8>, war_goals::CountryInventory>,
    /// Every saved `cached_sum`; a pair without one held an exact zero opinion.
    pub relation_opinions: Vec<RelationOpinion>,
    pub war_delivery_histories: Vec<war_participation::DeliveryHistory>,
    /// Every typed relation block except `war_relation`, which has its own table.
    pub relation_tuples: Vec<RelationTuple>,
    /// Per-country positional intel list, indexed by the target's canonical
    /// country-link slot; a country without a list held no records at all,
    /// which is a different fact from a list of all-zero records.
    pub country_intel: BTreeMap<Vec<u8>, Vec<CountryIntelRecord>>,
    /// Physical country-emission order and each navy's ordered signed base keys.
    /// This projection does not relax the full parser's duplicate-country rule.
    pub country_naval_launch: Vec<naval_launch::CountryNavalLaunchOccurrence>,
    pub factions: BTreeMap<u64, Faction>,
    pub faction_accounts: Option<faction_accounts::Accounts>,
    /// The saving client's viewing country tag (`player`). The writer uses
    /// the viewed country when signed-positive, else the fallback country.
    /// Every game save has it; only synthetic fixtures omit it.
    pub player: Option<Vec<u8>>,
    /// The saved header preserves repeated country keys and may retain users
    /// absent from the live roster; it is not a current-player census.
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
    /// Saved (id, type) to equipment-definition key. Repeated definition names
    /// are normal; the typed engine ids uniquely identify each saved variant.
    pub equipment_definitions: BTreeMap<(u32, u32), Vec<u8>>,
    /// Repeated top-level blocks, in native registry order.
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
    std::str::from_utf8(bytes)
        .map_err(|_| invalid("save integer is not ASCII"))?
        .parse()
        .map_err(|_| invalid("save integer is out of range or malformed"))
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

/// The strategic navy's `per_region_access`, written as flat region id and
/// mode pairs, the same layout consumed when loading.
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

/// The strategic navy's `regional_convoys`: a leading record count, then anonymous records
/// of `index` (the strategic region id) and `data`. Only records writing `efficiency` are
/// kept; the others are full.
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

/// Raw reference words where the native writer serializes a stored null
/// reference as `{ id=0 type=0 }`.
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

/// The two relation classes whose rows belong to other recorded tables:
/// War relations (WarRelations) and subject relations (Subjects). They still
/// occupy an ordinal because the native serializable vector holds them too.
/// Every other relation-shaped block becomes a RelationTuple, so a class this
/// build spells differently is reported as a difference rather than dropped.
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

/// The list is positional: every element is an anonymous block whose lanes the
/// writer emits in a fixed order and only when their raw qword is nonzero, so
/// an omitted lane is an exact zero and an empty block is four exact zeroes.
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

/// `upgrades.slots`: one anonymous block per slot, its optional `advisor`.
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

/// The faction theatre writer emits `regions` only for a
/// nonempty theatre array, so `pings={}` is an exact empty list.
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

/// A saved list of bare tokens, in order; empty when the key is absent.
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

/// A state's `strategic_region_data`: one anonymous block per location,
/// each holding exactly the location and its anchor province as two bare
/// values, as the text save writes them.
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

/// Binary saves are refused before parsing. This never rounds a save decimal or
/// fills a missing field with a historical or runtime default.
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
        // The country writer omits units when all three unit rosters
        // are empty. Other unit kinds within a present block are separate tables.
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
                // The relation-status writer emits every serializable relation
                // contiguously from one vector, so the ordinal counts every
                // relation-shaped block, including the two families that have
                // their own recorded tables. A block is recognised as a
                // relation by the `first`/`second` pair that the base relation
                // writer emits unconditionally for every class. This identifies
                // the saved shape without reconstructing runtime membership;
                // no other sub-block written here carries both keys.
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

/// Static province membership comes from the same verified content layers the
/// viewer uses, not from the replay being checked. Id zero is the map sentinel.
/// The state-history reader keeps `provinces` apart from history; province
/// initialization assigns each province's state from that array. No owner,
/// controller, core, building or victory-point
/// history value is a membership input; comparisons take those from the save.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapMembership {
    pub provinces: BTreeMap<u32, Option<u32>>,
}

impl MapMembership {
    /// Every record's id, as `definition.csv` is read for the map
    /// (`installed_content_core::map::definition_rows`).
    pub fn from_definition(bytes: &[u8]) -> io::Result<Self> {
        let mut provinces = BTreeMap::new();
        for row in installed_content_core::map::definition_rows(bytes)? {
            if provinces.insert(u32::from(row.id), None).is_some() {
                return Err(invalid("duplicate province id in definition.csv"));
            }
        }
        if provinces.is_empty() {
            return Err(invalid("definition.csv contains no provinces"));
        }
        Ok(Self { provinces })
    }

    /// Consume the shared StateDefinition used by every content consumer.
    pub fn add_state_history(
        &mut self,
        state: &installed_content_states::states::StateDefinition,
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
            // Province initialization ignores repeats within one state.
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

/// A province's state assignment is either verified (including no state), or
/// unknown because it is a candidate of an excluded state. Unknown never means
/// unassigned; its state ids identify the exclusions a consumer must report.
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

/// Verified assignments for the readable states, with explicit exclusions for
/// every state whose reader produced a diagnostic. The province census remains
/// exact; the excluded states' candidate provinces have unknown membership.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PartialMapMembership {
    pub provinces: BTreeMap<u32, ProvinceMembership>,
    /// Clean kept templates, even when another template makes a province's
    /// assignment unknown.
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
    let (states, notes) = content_state_history(layers)?;
    partial_map_membership(layers, &states, notes)
}

/// Read state history once for the oracle's membership and authored resources.
/// A resource representation failure does not discard verified membership.
pub fn read_partial_state_history(
    layers: &ResolvedLayers,
) -> io::Result<(
    PartialMapMembership,
    io::Result<construction::PartialBaseResources>,
)> {
    let (states, notes) = content_state_history(layers)?;
    let membership = partial_map_membership(layers, &states, notes)?;
    let resources = construction::PartialBaseResources::from_states(&states, &membership);
    Ok((membership, resources))
}

fn partial_map_membership(
    layers: &ResolvedLayers,
    states: &installed_content_states::states::States,
    notes: Vec<installed_content_core::Diagnostic>,
) -> io::Result<PartialMapMembership> {
    let definitions = installed_content_core::terrain::read_default_map(layers)?.file("definitions")?;
    let mut known = MapMembership::from_definition(&layers.read(&definitions)?)?;
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
                        .to_owned(),
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
        // Validate every clean assignment before marking excluded candidates
        // unknown, so an exclusion cannot hide a clean/clean overlap.
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

/// The oracle uses the same native reader and recorded registry/defines/map
/// as the viewer, retaining explicit partial coverage.
/// A `Deferred effect` warns of an unported token reader, not an unapplied
/// history result. Reading history calls each effect's key reader and queues
/// state-history commands by date. Game setup runs the history commands in
/// their state scopes. We read those scripts exactly, but never substitute
/// their authored operands for
/// the saved post-setup control, buildings or province victory points.
fn content_state_history(
    layers: &ResolvedLayers,
) -> io::Result<(
    installed_content_states::states::States,
    Vec<installed_content_core::Diagnostic>,
)> {
    let map = installed_content_core::terrain::read_default_map(layers)?;
    let provinces =
        installed_content_core::map::parse_definitions(&layers.read(&map.file("definitions")?)?)?;
    let defines = installed_content_core::defines::read(layers)?;
    let (states, notes) = installed_content_states::states::read(layers, &defines, &provinces)?;
    if states.definitions().is_empty() {
        return Err(invalid("verified content contains no state history"));
    }
    Ok((states, notes))
}

/// The oracle's state history over the shipped game in `BATTLE_PLAN_GAME_DIR`: every
/// state file reads with no diagnostic (a deferred history effect once refused
/// it whole).
#[test]
#[ignore = "reads the game's files; set BATTLE_PLAN_GAME_DIR"]
fn vanilla_state_history_reads_without_diagnostics() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let Some(root) = std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")) else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let (states, notes) = content_state_history(&layers).unwrap();
    assert!(!states.definitions().is_empty());
    assert!(notes.is_empty(), "{notes:?}");
    let membership = read_partial_map_membership(&layers).unwrap();
    assert!(membership.excluded_states.is_empty());
}

#[cfg(test)]
fn history_fixture(input: &[u8]) -> installed_content_states::states::StateDefinition {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let context = installed_content_states::states::Context {
        resource: &|_| installed_content_states::states::Resource::Found,
        max_building_levels: 20,
        province_count: 1000,
        names: &installed_content_core::key_names::KeyNames::default(),
    };
    installed_content_states::states::parse_states(input, &context)
        .unwrap()
        .0
        .remove(0)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod state_history_tests;
