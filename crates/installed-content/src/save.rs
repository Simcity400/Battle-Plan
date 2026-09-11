//! Text-save values for the oracle. The shared Clausewitz parser owns syntax;
//! this module interprets only the state fields the recorder currently captures.

pub mod air;
pub mod characters;
pub mod construction;
pub mod economy;
pub mod governance;
pub mod intel;
pub mod naval_history;
pub mod production;
pub mod projects;
pub mod session;
pub mod weather;

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use crate::{
    ResolvedLayers,
    parser::{self, Item, Value},
};

const MONTH_DAYS: [u8; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
pub const FIXED_SCALE: i64 = 100_000;

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

    /// CGameDate parser 140527450 and formatter 142254180 use a fixed 365-day
    /// calendar, epoch 43,800,000, and zero-based month/day/hour internally.
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

impl std::fmt::Display for GameDate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}.{}.{}.{}",
            self.year, self.month, self.day, self.hour
        )
    }
}

/// The literal's exact value in the engine's /100000 representation. The
/// original spelling and visible precision remain available to the oracle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decimal {
    pub raw: i64,
    pub fractional_digits: u32,
    pub lexeme: Vec<u8>,
}

impl Decimal {
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        let (negative, unsigned) = match bytes.first() {
            Some(b'-') => (true, &bytes[1..]),
            Some(b'+') => (false, &bytes[1..]),
            _ => (false, bytes),
        };
        let mut parts = unsigned.split(|&byte| byte == b'.');
        let integer = parts.next().unwrap();
        let fraction = parts.next();
        if integer.is_empty()
            || parts.next().is_some()
            || !integer.iter().all(u8::is_ascii_digit)
            || fraction.is_some_and(|fraction| {
                fraction.is_empty() || !fraction.iter().all(u8::is_ascii_digit)
            })
        {
            return Err(invalid("save scalar is not a decimal number"));
        }
        let mut magnitude = 0_i128;
        for &digit in integer {
            magnitude = magnitude
                .checked_mul(10)
                .and_then(|n| n.checked_add(i128::from(digit - b'0')))
                .ok_or_else(|| invalid("save decimal exceeds integer representation"))?;
        }
        magnitude = magnitude
            .checked_mul(i128::from(FIXED_SCALE))
            .ok_or_else(|| invalid("save decimal exceeds integer representation"))?;
        let fraction = fraction.unwrap_or_default();
        if fraction
            .get(5..)
            .is_some_and(|tail| tail.iter().any(|&digit| digit != b'0'))
        {
            return Err(invalid(
                "save scalar has nonzero precision below one engine /100000 unit; refusing to round",
            ));
        }
        let mut place = 10_000_i128;
        for &digit in fraction.iter().take(5) {
            magnitude = magnitude
                .checked_add(i128::from(digit - b'0') * place)
                .ok_or_else(|| invalid("save decimal exceeds integer representation"))?;
            place /= 10;
        }
        let raw = i64::try_from(if negative { -magnitude } else { magnitude })
            .map_err(|_| invalid("save scalar exceeds the engine i64 representation"))?;
        Ok(Self {
            raw,
            fractional_digits: u32::try_from(fraction.len())
                .map_err(|_| invalid("decimal precision exceeds u32"))?,
            lexeme: bytes.to_vec(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EquipmentAmount {
    pub id: u32,
    pub id_type: u32,
    pub amount: Decimal,
}

/// One `country_intel` triple of a saved CUnit, in serialized order.
///
/// CUnit writer BF7010 emits three signed integers per stored viewer entry: the
/// viewer key word, the low 32 bits of the entry's decay accumulator, and the
/// sign-extended level byte. Every entry is written, including nonpositive
/// levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntelEntry {
    pub viewer: i32,
    pub timer_low: i32,
    pub level: i32,
}

/// One `received_air_volunteer_permission` block of a saved country.
///
/// CCountry writer 70C580 emits one block per position of the native host
/// vector, in vector order, with the resolved host tag and its paired count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolunteerPermission {
    pub tag: Vec<u8>,
    pub count: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Country {
    pub tag: Vec<u8>,
    /// A direct country-name field, if the save carries one. Usually absent;
    /// localisation and cosmetic tags are not substituted for captured names.
    pub name: Option<Vec<u8>>,
    pub political_power: Option<Decimal>,
    pub command_power: Option<Decimal>,
    pub stability: Option<Decimal>,
    pub war_support: Option<Decimal>,
    /// Serialized stockpile entries, not an assumed equivalent of a live cache.
    pub convoy_equipment: Option<Vec<EquipmentAmount>>,
    pub instances_counter: Option<u32>,
    /// Ordered host blocks; absence and an empty vector are indistinguishable.
    pub volunteer_permissions: Vec<VolunteerPermission>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub id: u32,
    pub owner: Option<Vec<u8>>,
    /// The literal field only; absence is retained until its native save rule
    /// is applied by state_control, independently of province control.
    pub controller: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvinceOverride {
    pub id: u32,
    pub owner: Option<Vec<u8>>,
    pub controller: Option<Vec<u8>>,
    /// E6CA90 writes the signed CProvince+0x38 integer iff it is nonzero.
    pub victory_points: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Division {
    /// Native reference qword: type in the low u32, id in the high u32.
    pub id: u64,
    /// CCountry's enclosing units roster, independently of logical_country.
    pub roster_owner: Vec<u8>,
    pub logical_owner: Vec<u8>,
    pub expeditionary_owner: Option<Vec<u8>>,
    /// CUnit writer BF7010 omits location only when its pointer is null.
    pub province: Option<u32>,
    /// CArmy writer C80E00 emits the standard template, not fake intel, iff nonnull.
    pub template: Option<u64>,
    /// Saved strength is absolute current hit points in native /100000 units.
    pub hit_points: Decimal,
    pub organisation: Decimal,
    /// Ordered viewer entries; the writer omits the field for an empty array.
    pub country_intel: Vec<IntelEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarRelation {
    /// Enclosing countries.<tag>, retained independently of the explicit first.
    pub roster_owner: Vec<u8>,
    /// Enclosing diplomacy.active_relations.<tag>, independently of second.
    pub target: Vec<u8>,
    /// Native ordered endpoints; neither country IDs nor instigator sort them.
    pub first: Vec<u8>,
    pub second: Vec<u8>,
    /// CRelation writer D19260 omits this exactly when cancel is set.
    pub start_date: Option<GameDate>,
    /// Omission proves end <= start, not a particular raw end-hour default.
    pub end_date: Option<GameDate>,
    pub cancel: bool,
    /// CWarRelation writer D19640 emits all four signed integers unscaled.
    pub first_casualties: i64,
    pub second_casualties: i64,
    pub first_unknown_casualties: i64,
    pub second_unknown_casualties: i64,
    pub first_was_instigator: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Faction {
    /// Full native reference: type low u32, id high u32; only both zero is null.
    pub id: u64,
    /// CFaction+18 literal bytes, empty when omitted; no display-name fallback.
    pub name: Vec<u8>,
    /// The native pointer-array order, with its leader first when nonempty.
    pub members: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Player {
    pub country: Vec<u8>,
    /// CHuman +0x20, distinct from the unsaved display name at +0x40.
    pub user: Vec<u8>,
    pub country_leader: bool,
    /// BAD720 omits precisely the signed -1 value.
    pub id: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LandCombat {
    pub id: u64,
    pub province: u32,
    pub day: i32,
    pub duration: i32,
    /// Direct side unit references, not the separate front subset.
    pub attackers: Vec<u64>,
    pub defenders: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fleet {
    pub id: u64,
    pub owner: Vec<u8>,
    pub home_base: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskForce {
    pub id: u64,
    pub fleet: u64,
    pub roster_owner: Vec<u8>,
    pub logical_owner: Vec<u8>,
    pub expeditionary_owner: Option<Vec<u8>>,
    pub province: Option<u32>,
    pub mission: i32,
    /// Ordered viewer entries; the writer omits the field for an empty array.
    pub country_intel: Vec<IntelEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ship {
    pub id: u64,
    pub task_force: u64,
    pub strength: Decimal,
    pub organisation: Decimal,
    pub experience: Decimal,
    pub sunk_convoys: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmyOrder {
    pub owner_group: u64,
    pub instance: u32,
    pub country: Vec<u8>,
    pub order_type: i32,
    /// Saved can_execute is a signed byte spelling; preserve all native bits.
    pub active: u8,
    pub paradrop_target: u32,
    pub path: Vec<u32>,
    pub target_states: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirWing {
    pub id: u64,
    pub owner: Vec<u8>,
    pub country_override: Vec<u8>,
    pub air_base: u64,
    pub planes: i32,
    pub effective_planes: i32,
    pub mission_mask: u32,
    pub mission_region: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavalCombat {
    pub id: u64,
    pub province: u32,
    pub attacker_positioning: Decimal,
    pub defender_positioning: Decimal,
    pub attacker_anti_air: Decimal,
    pub defender_anti_air: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavalCombatParticipant {
    pub combat: u64,
    pub side: u8,
    pub battle_slot: u32,
    pub ship: Option<u64>,
    pub convoy: Option<u64>,
    pub owner: Vec<u8>,
    pub state: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Save {
    pub date: GameDate,
    pub campaign_id: Vec<u8>,
    pub version: Vec<u8>,
    pub seed: i32,
    pub countries: BTreeMap<Vec<u8>, Country>,
    pub states: BTreeMap<u32, State>,
    /// Sparse: absent entries are not evidence of a missing engine province.
    pub provinces: BTreeMap<u32, ProvinceOverride>,
    /// Full typed handles uniquely identify divisions across every country roster.
    pub divisions: BTreeMap<u64, Division>,
    /// One first-owned entry per ordered country pair, not a grouped war ID.
    pub war_relations: Vec<WarRelation>,
    pub factions: BTreeMap<u64, Faction>,
    /// The saved header preserves repeated country keys and may retain users
    /// absent from the live roster; it is not a current-player census.
    pub players: Vec<Player>,
    pub land_combats: BTreeMap<u64, LandCombat>,
    pub fleets: BTreeMap<u64, Fleet>,
    pub task_forces: BTreeMap<u64, TaskForce>,
    pub ships: BTreeMap<u64, Ship>,
    pub army_orders: BTreeMap<(u64, u32), ArmyOrder>,
    pub air_wings: BTreeMap<u64, AirWing>,
    pub naval_combats: BTreeMap<u64, NavalCombat>,
    pub naval_combat_participants: BTreeMap<(u64, u8, u32), NavalCombatParticipant>,
    /// Saved (id, type) to equipment-definition key. Repeated definition names
    /// are normal; the typed engine ids uniquely identify each saved variant.
    pub equipment_definitions: BTreeMap<(u32, u32), Vec<u8>>,
    pub air: air::Pack,
    pub characters: characters::Pack,
    pub construction: construction::Pack,
    pub economy: economy::Pack,
    pub governance: governance::Pack,
    pub intel: intel::Pack,
    pub naval_history: naval_history::Pack,
    pub production: production::Pack,
    pub projects: projects::Pack,
    pub session: session::Pack,
    pub weather: weather::Pack,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvinceControl {
    pub id: u32,
    pub state: Option<u32>,
    /// None is the no-owner/no-controller sentinel for a province without a state.
    pub owner: Option<Vec<u8>>,
    pub controller: Option<Vec<u8>>,
    pub controller_explicit: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateControl {
    pub id: u32,
    pub owner: Vec<u8>,
    pub controller: Vec<u8>,
    pub controller_explicit: bool,
}

impl Save {
    /// CState writer 1409d4040 omits controllers equal to a positive owner by
    /// raw id or canonical link slot; post-load 1409cdf80 restores owner.
    /// The writer emits zero controllers, unlike CProvince. A missing owner
    /// is refused because that path omits both fields, losing controller data.
    pub fn state_control(&self, membership: &MapMembership) -> io::Result<Vec<StateControl>> {
        let state_ids = membership
            .provinces
            .values()
            .flatten()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        if let Some(id) = self.states.keys().find(|id| !state_ids.contains(id)) {
            return Err(invalid(format!(
                "save state {id} has no membership in the verified content"
            )));
        }
        state_ids
            .into_iter()
            .map(|id| {
                let state = self
                    .states
                    .get(&id)
                    .ok_or_else(|| invalid(format!("save lacks state {id}")))?;
                let owner = state
                    .owner
                    .as_ref()
                    .ok_or_else(|| invalid(format!("save state {id} lacks its owner")))?;
                let controller = state.controller.as_ref().unwrap_or(owner);
                for tag in [owner, controller] {
                    if !self.countries.contains_key(tag) {
                        return Err(invalid(format!(
                            "state {id} references unknown saved country {}",
                            String::from_utf8_lossy(tag)
                        )));
                    }
                }
                Ok(StateControl {
                    id,
                    owner: owner.clone(),
                    controller: controller.clone(),
                    controller_explicit: state.controller.is_some(),
                })
            })
            .collect()
    }

    /// CProvince writer 140e6ca90 omits a controller equivalent to its state's
    /// owner; post-load 1409cdf80 fills omitted/nonpositive controls from that
    /// owner. A missing state or state-owner field cannot be filled this way.
    pub fn province_control(&self, membership: &MapMembership) -> io::Result<Vec<ProvinceControl>> {
        let state_ids = membership
            .provinces
            .values()
            .flatten()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        if let Some(id) = self.states.keys().find(|id| !state_ids.contains(id)) {
            return Err(invalid(format!(
                "save state {id} has no membership in the verified content"
            )));
        }
        if let Some(id) = self
            .provinces
            .keys()
            .find(|id| !membership.provinces.contains_key(id) || **id == 0)
        {
            return Err(invalid(format!(
                "save province {id} is absent from the verified map or is its sentinel"
            )));
        }
        let mut rows = Vec::with_capacity(membership.provinces.len());
        for (&id, &state_id) in &membership.provinces {
            if id == 0 {
                continue;
            }
            let owner = state_id.map(|state_id| {
                self.states.get(&state_id).ok_or_else(|| invalid(format!("save lacks state {state_id}, required by province {id}")))?
                    .owner.clone().ok_or_else(|| invalid(format!("save state {state_id} lacks its owner; refusing to infer province {id}")))
            }).transpose()?;
            let override_row = self.provinces.get(&id);
            if override_row.is_some_and(|row| row.owner.is_some()) {
                return Err(invalid(format!(
                    "save province {id} has an unsupported per-province owner field"
                )));
            }
            let explicit = override_row.and_then(|row| row.controller.clone());
            let controller = explicit.clone().or_else(|| owner.clone());
            for tag in [&owner, &controller].into_iter().flatten() {
                if !self.countries.contains_key(tag) {
                    return Err(invalid(format!(
                        "province {id} references unknown saved country {}",
                        String::from_utf8_lossy(tag)
                    )));
                }
            }
            rows.push(ProvinceControl {
                id,
                state: state_id,
                owner,
                controller,
                controller_explicit: explicit.is_some(),
            });
        }
        Ok(rows)
    }
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
        tag: tag.to_vec(),
        name: bytes(items, b"name")?,
        political_power: politics
            .map(|items| decimal(items, b"political_power"))
            .transpose()?
            .flatten(),
        command_power: decimal(items, b"command_power")?,
        stability: decimal(items, b"stability")?,
        war_support: decimal(items, b"war_support")?,
        convoy_equipment: convoy_equipment(items)?,
        instances_counter: bytes(items, b"instances_counter")?
            .map(|bytes| integer(&bytes))
            .transpose()?,
        volunteer_permissions: volunteer_permissions(items)?,
    })
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

fn parse_division(roster_owner: &[u8], items: &[Item]) -> io::Result<Division> {
    // CUnit and CSupplyConsumer each serialize the same CReferenceObject.
    // Repeated equal id blocks are native output; conflicting ones are not.
    let mut id = None;
    for item in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"id"))
    {
        let handle = typed_handle(&item.value)?;
        if id.is_some_and(|id| id != handle) {
            return Err(invalid("division has conflicting repeated typed ids"));
        }
        id = Some(handle);
    }
    let id = id.ok_or_else(|| invalid("division lacks its typed id"))?;
    let province = bytes(items, b"location")?
        .map(|value| integer::<u32>(&value))
        .transpose()?;
    if province == Some(0) {
        return Err(invalid(
            "division explicitly saves location zero; native null location must be omitted",
        ));
    }
    Ok(Division {
        id,
        roster_owner: roster_owner.to_vec(),
        logical_owner: required_bytes(items, b"logical_country")?,
        expeditionary_owner: bytes(items, b"expeditionary_owner")?,
        province,
        template: field(items, b"division_template_id")?
            .map(typed_handle)
            .transpose()?,
        hit_points: Decimal::parse(&required_bytes(items, b"strength")?)?,
        organisation: Decimal::parse(&required_bytes(items, b"organisation")?)?,
        country_intel: country_intel(items)?,
    })
}

/// The writer emits one anonymous flat list of three signed integers per entry.
fn country_intel(items: &[Item]) -> io::Result<Vec<IntelEntry>> {
    let Some(value) = field(items, b"country_intel")? else {
        return Ok(Vec::new());
    };
    let entries = block(value)?;
    if entries.is_empty() || entries.len() % 3 != 0 {
        return Err(invalid(
            "saved country_intel is empty or not a whole number of viewer triples",
        ));
    }
    let mut words = Vec::with_capacity(entries.len());
    for entry in entries {
        if entry.key.is_some() {
            return Err(invalid("saved country_intel must be an anonymous list"));
        }
        words.push(integer::<i32>(scalar(&entry.value)?)?);
    }
    Ok(words
        .chunks_exact(3)
        .map(|triple| IntelEntry {
            viewer: triple[0],
            timer_low: triple[1],
            level: triple[2],
        })
        .collect())
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
    Ok(WarRelation {
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
    Ok(Faction {
        id,
        name: bytes(items, b"name")?.unwrap_or_default(),
        members,
    })
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

fn parse_land_combat(items: &[Item]) -> io::Result<LandCombat> {
    fn participants(items: &[Item], side: &[u8]) -> io::Result<Vec<u64>> {
        required_block(items, side)?
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"unit"))
            .map(|item| typed_handle(&item.value))
            .collect()
    }
    Ok(LandCombat {
        id: typed_handle(
            field(items, b"id")?.ok_or_else(|| invalid("land combat lacks typed id"))?,
        )?,
        province: integer::<i32>(&required_bytes(items, b"location")?)? as u32,
        day: integer(&required_bytes(items, b"day")?)?,
        duration: integer(&required_bytes(items, b"duration")?)?,
        attackers: participants(items, b"attacker")?,
        defenders: participants(items, b"defender")?,
    })
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

fn parse_fleet(owner: &[u8], items: &[Item]) -> io::Result<Fleet> {
    Ok(Fleet {
        id: object_id(items)?,
        owner: owner.to_vec(),
        home_base: field(items, b"home_base")?
            .map(|value| integer(scalar(value)?))
            .transpose()?,
    })
}

fn parse_task_force(owner: &[u8], fleet: u64, items: &[Item]) -> io::Result<TaskForce> {
    Ok(TaskForce {
        id: object_id(items)?,
        fleet,
        roster_owner: owner.to_vec(),
        logical_owner: required_bytes(items, b"logical_country")?,
        expeditionary_owner: bytes(items, b"expeditionary_owner")?,
        province: field(items, b"location")?
            .map(|value| integer(scalar(value)?))
            .transpose()?,
        mission: integer(&required_bytes(
            required_block(items, b"mission")?,
            b"mission",
        )?)?,
        country_intel: country_intel(items)?,
    })
}

fn parse_ship(task_force: u64, items: &[Item]) -> io::Result<Ship> {
    Ok(Ship {
        id: object_id(items)?,
        task_force,
        strength: decimal(items, b"strength")?.ok_or_else(|| invalid("ship lacks strength"))?,
        organisation: decimal(items, b"organisation")?
            .ok_or_else(|| invalid("ship lacks organisation"))?,
        experience: decimal(items, b"experience")?.unwrap_or(Decimal::parse(b"0")?),
        sunk_convoys: field(items, b"sunk_convoys")?
            .map(|value| integer(scalar(value)?))
            .transpose()?
            .unwrap_or(0),
    })
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

fn parse_army_order(country: &[u8], owner_group: u64, items: &[Item]) -> io::Result<ArmyOrder> {
    Ok(ArmyOrder {
        owner_group,
        country: country.to_vec(),
        instance: integer(&required_bytes(items, b"instance_id")?)?,
        order_type: integer(&required_bytes(items, b"type")?)?,
        active: field(items, b"can_execute")?
            .map(|value| integer::<i8>(scalar(value)?))
            .transpose()?
            .unwrap_or(0) as u8,
        paradrop_target: field(items, b"invasion_source")?
            .map(|value| integer::<i32>(scalar(value)?))
            .transpose()?
            .unwrap_or(0) as u32,
        path: integer_list(items, b"path")?,
        target_states: integer_list(items, b"states")?,
    })
}

fn parse_orders(
    country: &[u8],
    items: &[Item],
    orders: &mut BTreeMap<(u64, u32), ArmyOrder>,
) -> io::Result<()> {
    let Some(theatres) = field(items, b"theatres")? else {
        return Ok(());
    };
    for theatre in block(theatres)?
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"theatre"))
    {
        for group in block(&theatre.value)?.iter().filter(|item| {
            matches!(
                item.key.as_deref(),
                Some(b"orders_group" | b"field_marshal_group")
            )
        }) {
            let group = block(&group.value)?;
            let id = object_id(group)?;
            for item in group.iter().filter(|item| {
                matches!(
                    item.key.as_deref(),
                    Some(b"order_instance" | b"fallback" | b"virtual_fallback")
                )
            }) {
                let order = parse_army_order(country, id, block(&item.value)?)?;
                if orders.insert((id, order.instance), order).is_some() {
                    return Err(invalid("duplicate scoped army order instance in save"));
                }
            }
        }
    }
    Ok(())
}

fn parse_air_wings(items: &[Item]) -> io::Result<BTreeMap<u64, AirWing>> {
    let mut wings = BTreeMap::new();
    let Some(air) = field(items, b"strategic_air")? else {
        return Ok(wings);
    };
    for country in block(air)? {
        let owner = country
            .key
            .as_deref()
            .filter(|key| !key.is_empty())
            .ok_or_else(|| invalid("strategic air country lacks tag"))?;
        for pool in block(&country.value)?
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"air_wing_pool"))
        {
            let pool = block(&pool.value)?;
            for wing in pool
                .iter()
                .filter(|item| item.key.as_deref() == Some(b"air_wings"))
            {
                let fields = block(&wing.value)?;
                let mission = required_block(fields, b"mission")?;
                let wing = AirWing {
                    id: object_id(fields)?,
                    owner: owner.to_vec(),
                    country_override: required_bytes(fields, b"tag")?,
                    air_base: typed_handle(
                        field(pool, b"air_base")?
                            .ok_or_else(|| invalid("populated air pool lacks air_base"))?,
                    )?,
                    planes: integer(&required_bytes(fields, b"count")?)?,
                    effective_planes: field(mission, b"effective_planes_count")?
                        .map(|value| integer(scalar(value)?))
                        .transpose()?
                        .unwrap_or(0),
                    mission_mask: integer::<i32>(&required_bytes(mission, b"type")?)? as u32,
                    mission_region: field(mission, b"strategic_region")?
                        .map(|value| integer(scalar(value)?))
                        .transpose()?,
                };
                if wings.insert(wing.id, wing).is_some() {
                    return Err(invalid("duplicate full air wing handle in save"));
                }
            }
        }
    }
    Ok(wings)
}

fn parse_naval_combat(
    items: &[Item],
    participants: &mut BTreeMap<(u64, u8, u32), NavalCombatParticipant>,
) -> io::Result<NavalCombat> {
    let id = object_id(items)?;
    let attacker = required_block(items, b"attacker")?;
    let defender = required_block(items, b"defender")?;
    for (side, fields) in [(0, attacker), (1, defender)] {
        for group in fields
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"group"))
        {
            for item in block(&group.value)?
                .iter()
                .filter(|item| item.key.as_deref() == Some(b"member"))
            {
                let fields = block(&item.value)?;
                let row = NavalCombatParticipant {
                    combat: id,
                    side,
                    battle_slot: integer::<i32>(&required_bytes(fields, b"unique_id")?)? as u32,
                    ship: field(fields, b"ship")?.map(typed_handle).transpose()?,
                    convoy: field(fields, b"convoy")?.map(typed_handle).transpose()?,
                    owner: required_bytes(required_block(fields, b"cached_info")?, b"tag")?,
                    state: integer::<i32>(&required_bytes(fields, b"state")?)? as u32,
                };
                if participants
                    .insert((id, side, row.battle_slot), row)
                    .is_some()
                {
                    return Err(invalid("duplicate combat side battle slot in save"));
                }
            }
        }
    }
    Ok(NavalCombat {
        id,
        province: integer::<i32>(&required_bytes(items, b"location")?)? as u32,
        attacker_positioning: Decimal::parse(&required_bytes(attacker, b"positioning")?)?,
        defender_positioning: Decimal::parse(&required_bytes(defender, b"positioning")?)?,
        attacker_anti_air: Decimal::parse(&required_bytes(attacker, b"anti_air")?)?,
        defender_anti_air: Decimal::parse(&required_bytes(defender, b"anti_air")?)?,
    })
}

/// Binary saves are refused before parsing. This never rounds a save decimal or
/// fills a missing field with a historical or runtime default.
pub fn parse(input: &[u8]) -> io::Result<Save> {
    let input = input.strip_prefix(b"\xef\xbb\xbf").unwrap_or(input);
    if input.starts_with(b"HOI4bin") {
        return Err(invalid(
            "binary the game saves cannot be compared; set save_as_binary=no in settings.txt, then create a fresh save",
        ));
    }
    if !input.starts_with(b"HOI4txt") || !input.get(7).is_some_and(u8::is_ascii_whitespace) {
        return Err(invalid(
            "expected a HOI4txt save header; set save_as_binary=no in settings.txt and create a fresh save",
        ));
    }
    let document = parser::parse(input)?;
    let items = &document.items;
    let date = GameDate::parse(&required_bytes(items, b"date")?)?;
    let campaign_id = required_bytes(items, b"game_unique_id")?;
    let version = required_bytes(items, b"version")?;
    let seed = integer(&required_bytes(items, b"game_unique_seed")?)?;
    let mut countries = BTreeMap::new();
    let mut divisions = BTreeMap::new();
    let mut fleets = BTreeMap::new();
    let mut task_forces = BTreeMap::new();
    let mut ships = BTreeMap::new();
    let mut army_orders = BTreeMap::new();
    let mut war_relations = Vec::new();
    let mut war_pairs = BTreeSet::new();
    for item in required_block(items, b"countries")? {
        let tag = item
            .key
            .as_deref()
            .filter(|tag| !tag.is_empty())
            .ok_or_else(|| invalid("country has no tag"))?;
        let country_items = block(&item.value)?;
        parse_orders(tag, country_items, &mut army_orders)?;
        let country = parse_country(tag, country_items).map_err(|error| {
            invalid(format!("country {}: {error}", String::from_utf8_lossy(tag)))
        })?;
        if countries.insert(tag.to_vec(), country).is_some() {
            return Err(invalid("duplicate country tag in save"));
        }
        // CCountry writer 14070c580 omits units when all three unit rosters
        // are empty. Other unit kinds within a present block are separate tables.
        if let Some(units) = field(country_items, b"units")? {
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
                    for item in fields
                        .iter()
                        .filter(|item| item.key.as_deref() == Some(b"ship"))
                    {
                        let ship = parse_ship(task_force.id, block(&item.value)?)?;
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
                if let Some(war) = field(block(&item.value)?, b"war_relation")? {
                    let war = parse_war_relation(tag, target, block(war)?).map_err(|error| {
                        invalid(format!(
                            "country {} relation {}: {error}",
                            String::from_utf8_lossy(tag),
                            String::from_utf8_lossy(target)
                        ))
                    })?;
                    if !war_pairs.insert((war.first.clone(), war.second.clone())) {
                        return Err(invalid("duplicate ordered war relation pair in save"));
                    }
                    war_relations.push(war);
                }
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
            let combat = parse_land_combat(block(&item.value)?)?;
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
                    owner: bytes(province, b"owner")?,
                    controller: bytes(province, b"controller")?,
                    victory_points: bytes(province, b"victory_points")?
                        .map(|value| integer(&value))
                        .transpose()?,
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
        countries,
        states,
        provinces,
        divisions,
        war_relations,
        factions,
        players: parse_players(items)?,
        land_combats,
        fleets,
        task_forces,
        ships,
        army_orders,
        air_wings: parse_air_wings(items)?,
        naval_combats,
        naval_combat_participants,
        equipment_definitions,
        air: air::parse(items).map_err(|error| pack_error("air", error))?,
        characters: characters::parse(items).map_err(|error| pack_error("characters", error))?,
        construction: construction::parse(items)
            .map_err(|error| pack_error("construction", error))?,
        economy: economy::parse(items).map_err(|error| pack_error("economy", error))?,
        governance: governance::parse(items).map_err(|error| pack_error("governance", error))?,
        intel: intel::parse(items).map_err(|error| pack_error("intel", error))?,
        naval_history: naval_history::parse(items)
            .map_err(|error| pack_error("naval history", error))?,
        production: production::parse(items).map_err(|error| pack_error("production", error))?,
        projects: projects::parse(items).map_err(|error| pack_error("projects", error))?,
        session: session::parse(items).map_err(|error| pack_error("session", error))?,
        weather: weather::parse(items).map_err(|error| pack_error("weather", error))?,
    })
}

/// Static province membership comes from the same verified content layers the
/// viewer uses, not from the replay being checked. Id zero is the map sentinel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapMembership {
    pub provinces: BTreeMap<u32, Option<u32>>,
}

impl MapMembership {
    pub fn from_definition(bytes: &[u8]) -> io::Result<Self> {
        let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
        let text =
            std::str::from_utf8(bytes).map_err(|_| invalid("definition.csv is not UTF-8"))?;
        let mut provinces = BTreeMap::new();
        for line in text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            let id = integer(line.split(';').next().unwrap().as_bytes())?;
            if provinces.insert(id, None).is_some() {
                return Err(invalid("duplicate province id in definition.csv"));
            }
        }
        if provinces.is_empty() {
            return Err(invalid("definition.csv contains no provinces"));
        }
        Ok(Self { provinces })
    }

    pub fn add_state_history(&mut self, input: &[u8]) -> io::Result<()> {
        let document = parser::parse(input)?;
        let state = required_block(&document.items, b"state")?;
        let id: u32 = integer(&required_bytes(state, b"id")?)?;
        if id == 0 || self.provinces.values().any(|&state| state == Some(id)) {
            return Err(invalid("duplicate or zero state id in state history"));
        }
        let mut assignments = Vec::new();
        for item in required_block(state, b"provinces")? {
            if item.key.is_some() {
                return Err(invalid("state provinces must be an unkeyed id list"));
            }
            let province: u32 = integer(scalar(&item.value)?)?;
            if province == 0
                || self.provinces.get(&province) != Some(&None)
                || assignments.contains(&province)
            {
                return Err(invalid(
                    "state history uses a zero, unknown or already assigned province",
                ));
            }
            assignments.push(province);
        }
        if assignments.is_empty() {
            return Err(invalid("state history contains no provinces"));
        }
        for province in assignments {
            self.provinces.insert(province, Some(id));
        }
        Ok(())
    }
}

pub fn read_map_membership(layers: &ResolvedLayers) -> io::Result<MapMembership> {
    let mut membership = MapMembership::from_definition(&layers.read("map/definition.csv")?)?;
    let mut state_count = 0;
    for path in layers.paths().filter(|path| {
        path.to_ascii_lowercase().starts_with("history/states/")
            && path.to_ascii_lowercase().ends_with(".txt")
    }) {
        membership
            .add_state_history(&layers.read(path)?)
            .map_err(|error| invalid(format!("{path}: {error}")))?;
        state_count += 1;
    }
    if state_count == 0 {
        return Err(invalid("verified content contains no state history"));
    }
    Ok(membership)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EquipmentTypes {
    pub types: BTreeMap<Vec<u8>, Vec<Vec<u8>>>,
    pub limitations: BTreeMap<Vec<u8>, String>,
}

impl EquipmentTypes {
    pub fn is_convoy(&self, definition: &[u8]) -> Option<bool> {
        self.types
            .get(definition)
            .map(|types| types.iter().any(|kind| kind == b"convoy"))
    }
}

#[derive(Clone, Debug)]
struct EquipmentDefinition {
    types: Option<Vec<Vec<u8>>>,
    archetype: Option<Vec<u8>>,
    parent: Option<Vec<u8>>,
}

#[derive(Default)]
struct EquipmentCatalog {
    definitions: BTreeMap<Vec<u8>, Result<EquipmentDefinition, String>>,
}

impl EquipmentCatalog {
    fn add(&mut self, input: &[u8]) -> io::Result<()> {
        let document = parser::parse(input)?;
        for section in document.get_all(b"equipments") {
            for item in block(section)? {
                let key = item
                    .key
                    .clone()
                    .filter(|key| !key.is_empty())
                    .ok_or_else(|| invalid("equipment definition has no name"))?;
                let definition = (|| {
                    let items = block(&item.value)?;
                    let types = field(items, b"type")?
                        .map(|value| {
                            let mut kinds = match value {
                                Value::Scalar(kind) => vec![kind.clone()],
                                Value::Block(items) => items
                                    .iter()
                                    .map(|item| {
                                        if item.key.is_some() {
                                            return Err(invalid(
                                                "equipment type list has a keyed entry",
                                            ));
                                        }
                                        scalar(&item.value).map(<[u8]>::to_vec)
                                    })
                                    .collect::<io::Result<Vec<_>>>()?,
                                Value::Tagged { .. } => {
                                    return Err(invalid(
                                        "equipment type is not a scalar or plain list",
                                    ));
                                }
                            };
                            if kinds.is_empty()
                                || kinds
                                    .iter()
                                    .any(|kind| kind.is_empty() || kind.starts_with(b"@"))
                            {
                                return Err(invalid(
                                    "equipment type is empty or uses an unresolved variable",
                                ));
                            }
                            kinds.sort();
                            kinds.dedup();
                            Ok(kinds)
                        })
                        .transpose()?;
                    Ok(EquipmentDefinition {
                        types,
                        archetype: bytes(items, b"archetype")?,
                        parent: bytes(items, b"parent")?,
                    })
                })()
                .map_err(|error: io::Error| error.to_string());
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    self.definitions.entry(key.clone())
                {
                    entry.insert(definition);
                } else {
                    self.definitions.insert(key, Err("duplicate equipment definition across resolved files; precedence is ambiguous".into()));
                }
            }
        }
        Ok(())
    }

    fn resolve(self) -> EquipmentTypes {
        fn visit(
            name: &[u8],
            catalog: &EquipmentCatalog,
            visiting: &mut std::collections::BTreeSet<Vec<u8>>,
            memo: &mut BTreeMap<Vec<u8>, Result<Vec<Vec<u8>>, String>>,
        ) -> Result<Vec<Vec<u8>>, String> {
            if let Some(result) = memo.get(name) {
                return result.clone();
            }
            if visiting.len() >= parser::MAX_DEPTH {
                return Err("equipment inheritance exceeds the parser depth limit".into());
            }
            if !visiting.insert(name.to_vec()) {
                return Err("equipment inheritance cycle".into());
            }
            let result = (|| {
                let definition = catalog
                    .definitions
                    .get(name)
                    .ok_or_else(|| {
                        format!(
                            "missing equipment definition {}",
                            String::from_utf8_lossy(name)
                        )
                    })?
                    .as_ref()
                    .map_err(Clone::clone)?;
                let mut candidates = Vec::new();
                if let Some(types) = &definition.types {
                    candidates.push(types.clone());
                }
                for parent in [&definition.parent, &definition.archetype]
                    .into_iter()
                    .flatten()
                {
                    candidates.push(visit(parent, catalog, visiting, memo)?);
                }
                let Some(first) = candidates.first() else {
                    return Err("equipment definition has no type, parent or archetype".into());
                };
                if candidates.iter().any(|types| types != first) {
                    return Err("equipment type and inherited types disagree; precedence is not established".into());
                }
                Ok(first.clone())
            })();
            visiting.remove(name);
            memo.insert(name.to_vec(), result.clone());
            result
        }
        let mut result = EquipmentTypes::default();
        let mut memo = BTreeMap::new();
        for name in self.definitions.keys() {
            match visit(
                name,
                &self,
                &mut std::collections::BTreeSet::new(),
                &mut memo,
            ) {
                Ok(types) => {
                    result.types.insert(name.clone(), types);
                }
                Err(reason) => {
                    result.limitations.insert(name.clone(), reason);
                }
            }
        }
        result
    }
}

/// Resolves the equipment classifier from verified definitions, including the
/// archetype/parent chain. The oracle can limit an unresolved convoy sum without
/// treating every saved type-70 object as convoy equipment.
pub fn read_equipment_types(layers: &ResolvedLayers) -> io::Result<EquipmentTypes> {
    let mut catalog = EquipmentCatalog::default();
    for path in layers.paths().filter(|path| {
        path.to_ascii_lowercase()
            .starts_with("common/units/equipment/")
            && path.to_ascii_lowercase().ends_with(".txt")
    }) {
        catalog
            .add(&layers.read(path)?)
            .map_err(|error| invalid(format!("{path}: {error}")))?;
    }
    Ok(catalog.resolve())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_preserve_engine_units_and_literal_precision_without_rounding() {
        for (literal, raw, digits) in [
            ("78.1411", 7_814_110, 4),
            ("0.7", 70_000, 1),
            ("-0.00001", -1, 5),
            ("1.000000", 100_000, 6),
            ("92233720368547.75807", i64::MAX, 5),
            ("-92233720368547.75808", i64::MIN, 5),
        ] {
            let value = Decimal::parse(literal.as_bytes()).unwrap();
            assert_eq!(value.raw, raw);
            assert_eq!(value.fractional_digits, digits);
            assert_eq!(value.lexeme, literal.as_bytes());
        }
        for bad in [
            "",
            ".1",
            "1.",
            "--1",
            "NaN",
            "1e2",
            "1.000001",
            "92233720368547.75808",
        ] {
            assert!(Decimal::parse(bad.as_bytes()).is_err(), "{bad}");
        }
    }

    #[test]
    fn engine_calendar_has_no_leap_day_and_saves_use_hour_one_through_twenty_four() {
        assert_eq!(
            GameDate::parse(b"1936.1.2.17")
                .unwrap()
                .engine_hour()
                .unwrap(),
            60_759_400
        );
        assert_eq!(
            GameDate::parse(b"1936.9.29.19")
                .unwrap()
                .engine_hour()
                .unwrap(),
            60_765_882
        );
        assert_eq!(
            GameDate::parse(b"1936.1.1.24")
                .unwrap()
                .engine_hour()
                .unwrap()
                + 1,
            GameDate::parse(b"1936.1.2.1")
                .unwrap()
                .engine_hour()
                .unwrap()
        );
        for bad in [
            "1936.2.29.1",
            "1936.1.1.0",
            "1936.1.1.25",
            "1936.13.1.1",
            "1936.1.0.1",
            "1936.1.1",
        ] {
            assert!(GameDate::parse(bad.as_bytes()).is_err(), "{bad}");
        }
    }

    #[test]
    fn extraction_does_not_confuse_nested_similarly_named_fields_or_invent_missing_values() {
        let save = parse(br#"HOI4txt
date="1936.1.2.17" game_unique_id="campaign" version="build" game_unique_seed=-42
countries={ GER={ command_power=78.1411 politics={ political_power=582.128 } stability=0.7 war_support=0.3
flags={ stability={value=99 date=1936.1.2.17} } unrelated={stability=99}
convoys={ equipment={ equipment={ id={ id=1 type=70 } amount=200 } } } } D01={} }
states={ 1={owner="GER"} } provinces={ 2={controller="D01"} 3={buildings={}} }
"#).unwrap();
        assert_eq!(save.seed, -42);
        assert_eq!(
            save.countries[b"GER".as_slice()]
                .stability
                .as_ref()
                .unwrap()
                .raw,
            70_000
        );
        assert_eq!(
            save.countries[b"GER".as_slice()]
                .political_power
                .as_ref()
                .unwrap()
                .raw,
            58_212_800
        );
        assert_eq!(
            save.countries[b"GER".as_slice()]
                .convoy_equipment
                .as_ref()
                .unwrap()[0]
                .amount
                .raw,
            20_000_000
        );
        assert_eq!(save.countries[b"D01".as_slice()].command_power, None);
        assert_eq!(save.provinces[&3].controller, None);
        assert_eq!(save.countries[b"GER".as_slice()].name, None);
        assert_eq!(
            save.intel.countries[b"GER".as_slice()]
                .flags
                .as_ref()
                .unwrap()[0]
                .value,
            99
        );
    }

    #[test]
    fn province_victory_points_keep_signed_integers_and_literal_omission() {
        let text = |value: &str| {
            format!(
                "HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 countries={{}} states={{}} provinces={{1={{victory_points={value}}} 2={{}}}}"
            )
        };
        for value in [i32::MIN, -1, 0, i32::MAX] {
            let saved = parse(text(&value.to_string()).as_bytes()).unwrap();
            assert_eq!(saved.provinces[&1].victory_points, Some(value));
            assert_eq!(saved.provinces[&2].victory_points, None);
        }
        for invalid in ["2147483648", "-2147483649", "1.0", "1.5"] {
            assert!(parse(text(invalid).as_bytes()).is_err());
        }
    }

    #[test]
    fn binary_malformed_and_ambiguous_saves_are_refused() {
        assert!(
            parse(b"HOI4bin\0")
                .unwrap_err()
                .to_string()
                .contains("save_as_binary=no")
        );
        assert!(parse(b"HOI4txtbad").is_err());
        let header = b"HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 countries={} states={} provinces={} ";
        let mut duplicate = header.to_vec();
        duplicate.extend_from_slice(b"date=1936.1.2.1");
        assert!(parse(&duplicate).is_err());
        assert!(parse(header).is_ok());
    }

    #[test]
    fn static_membership_rejects_missing_and_duplicate_province_assignments() {
        let mut map =
            MapMembership::from_definition(b"0;0;0;0\n1;1;2;3\n2;3;2;1\n3;4;5;6\n").unwrap();
        map.add_state_history(b"state={id=5 provinces={1 2}}")
            .unwrap();
        assert_eq!(map.provinces[&0], None);
        assert_eq!(map.provinces[&1], Some(5));
        assert_eq!(map.provinces[&3], None);
        let original = map.clone();
        for bad in [
            "state={id=6 provinces={2}}",
            "state={id=6 provinces={4}}",
            "state={id=6 provinces={3 3}}",
            "state={id=5 provinces={3}}",
        ] {
            assert!(map.add_state_history(bad.as_bytes()).is_err());
            assert_eq!(map, original);
        }
    }

    #[test]
    fn full_province_control_uses_only_proven_defaults_and_refuses_missing_ownership() {
        let mut save = parse(
            br#"HOI4txt
date=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0
countries={ GER={} D01={} } states={1={owner=GER}} provinces={2={controller=D01}}
equipments={convoy_1={id={id=1 type=70}} convoy_1={id={id=2 type=70}}}
"#,
        )
        .unwrap();
        let mut map =
            MapMembership::from_definition(b"0;0;0;0\n1;1;2;3\n2;3;2;1\n3;4;5;6\n").unwrap();
        map.add_state_history(b"state={id=1 provinces={1 2}}")
            .unwrap();
        let rows = save.province_control(&map).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].controller.as_deref(), Some(b"GER".as_slice()));
        assert!(!rows[0].controller_explicit);
        assert_eq!(rows[1].controller.as_deref(), Some(b"D01".as_slice()));
        assert!(rows[1].controller_explicit);
        assert_eq!(rows[2].owner, None);
        assert_eq!(rows[2].controller, None);
        assert_eq!(save.equipment_definitions.len(), 2);
        save.states.get_mut(&1).unwrap().owner = None;
        assert!(
            save.province_control(&map)
                .unwrap_err()
                .to_string()
                .contains("lacks its owner")
        );
        save.states.remove(&1);
        assert!(
            save.province_control(&map)
                .unwrap_err()
                .to_string()
                .contains("lacks state")
        );
    }

    #[test]
    fn state_control_has_its_own_explicit_and_owner_equivalent_save_rules() {
        let save = parse(
            br#"HOI4txt
date=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0
countries={ GER={} D01={} }
states={1={owner=GER controller=D01} 2={owner=GER}}
provinces={3={controller=D01}}
"#,
        )
        .unwrap();
        let map = MapMembership {
            provinces: BTreeMap::from([(0, None), (1, Some(1)), (2, Some(2)), (3, Some(2))]),
        };
        let states = save.state_control(&map).unwrap();
        assert_eq!(states.len(), 2);
        assert_eq!(states[0].owner, b"GER");
        assert_eq!(states[0].controller, b"D01");
        assert!(states[0].controller_explicit);
        assert_eq!(states[1].controller, b"GER");
        assert!(!states[1].controller_explicit);
        let provinces = save.province_control(&map).unwrap();
        // A state's explicit controller does not replace province ownership
        // or its independent sparse controller fields, and vice versa.
        assert_eq!(provinces[0].controller.as_deref(), Some(b"GER".as_slice()));
        assert_eq!(provinces[2].controller.as_deref(), Some(b"D01".as_slice()));
    }

    #[test]
    fn state_comparison_refuses_missing_fields_unknown_tags_and_content_mismatch() {
        let save = parse(
            br#"HOI4txt
date=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0
countries={ GER={} } states={1={owner=GER}} provinces={}
"#,
        )
        .unwrap();
        let map = MapMembership {
            provinces: BTreeMap::from([(1, Some(1))]),
        };
        let mut missing = save.clone();
        missing.states.clear();
        assert!(
            missing
                .state_control(&map)
                .unwrap_err()
                .to_string()
                .contains("lacks state 1")
        );
        let mut missing = save.clone();
        missing.states.get_mut(&1).unwrap().owner = None;
        assert!(
            missing
                .state_control(&map)
                .unwrap_err()
                .to_string()
                .contains("lacks its owner")
        );
        for owner in [false, true] {
            let mut unknown = save.clone();
            let state = unknown.states.get_mut(&1).unwrap();
            if owner {
                state.owner = Some(b"UNKNOWN".to_vec());
            } else {
                state.controller = Some(b"UNKNOWN".to_vec());
            }
            assert!(
                unknown
                    .state_control(&map)
                    .unwrap_err()
                    .to_string()
                    .contains("unknown saved country")
            );
        }
        let mut unexpected = save;
        unexpected.states.insert(
            2,
            State {
                id: 2,
                owner: Some(b"GER".to_vec()),
                controller: None,
            },
        );
        assert!(
            unexpected
                .state_control(&map)
                .unwrap_err()
                .to_string()
                .contains("no membership")
        );
    }

    #[test]
    fn country_intel_triples_keep_native_order_signs_and_nonpositive_levels() {
        let save = division_save(
            r#"
division={
 id={id=1 type=4919} logical_country=SOV strength=0 organisation=0
 country_intel={ 2 -2147483648 4 -1 0 -1 }
}
division={ id={id=2 type=4919} logical_country=SOV strength=0 organisation=0 }
"#,
        )
        .unwrap();
        let intel = &save.divisions[&((1_u64 << 32) | 4919)].country_intel;
        assert_eq!(
            intel,
            &[
                IntelEntry {
                    viewer: 2,
                    timer_low: i32::MIN,
                    level: 4
                },
                IntelEntry {
                    viewer: -1,
                    timer_low: 0,
                    level: -1
                },
            ]
        );
        // An omitted field is a native empty array, not a refusal.
        assert!(
            save.divisions[&((2_u64 << 32) | 4919)]
                .country_intel
                .is_empty()
        );
    }

    #[test]
    fn a_partial_or_keyed_country_intel_list_is_refused() {
        for list in ["{ 1 2 }", "{ }", "{ viewer=1 2 3 }"] {
            let save = division_save(&format!(
                "division={{ id={{id=1 type=4919}} logical_country=SOV strength=0 organisation=0 country_intel={list} }}"
            ));
            assert!(save.is_err(), "{list}");
        }
    }

    #[test]
    fn volunteer_permission_blocks_keep_save_order_and_repeated_tags() {
        let save = parse(
            b"HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 countries={ SOV={ received_air_volunteer_permission={ tag=GER count=3 } received_air_volunteer_permission={ tag=GER count=-1 } } GER={} } states={} provinces={}",
        )
        .unwrap();
        let hosts = &save.countries[b"SOV".as_slice()].volunteer_permissions;
        assert_eq!(
            hosts,
            &[
                VolunteerPermission {
                    tag: b"GER".to_vec(),
                    count: 3
                },
                VolunteerPermission {
                    tag: b"GER".to_vec(),
                    count: -1
                },
            ]
        );
        assert!(
            save.countries[b"GER".as_slice()]
                .volunteer_permissions
                .is_empty()
        );
        assert!(
            parse(
                b"HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 countries={ SOV={ received_air_volunteer_permission={ tag=GER } } } states={} provinces={}",
            )
            .is_err()
        );
    }

    fn division_save(units: &str) -> io::Result<Save> {
        parse(format!(
            "HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 countries={{ SOV={{ units={{ {units} }} }} D01={{}} GER={{}} }} states={{}} provinces={{}}"
        ).as_bytes())
    }

    #[test]
    fn division_extract_preserves_full_handles_ownership_and_native_fixed_values() {
        let save = division_save(
            r#"
division={
 id={id=2147483649 type=4919} id={type=4919 id=2147483649}
 logical_country=D01 expeditionary_owner=GER location=6906
 division_template_id={id=4294967295 type=6000}
 strength=90071992547.40993 organisation=-0.00001
 equipment={ equipment={id={id=1 type=70} amount=100} }
}
division={
 id={id=2147483649 type=4920} logical_country=SOV
 strength=0 organisation=0
}
"#,
        )
        .unwrap();
        let id = (2_147_483_649_u64 << 32) | 4919;
        let division = &save.divisions[&id];
        assert_eq!(save.divisions.len(), 2);
        assert_eq!(division.id, id);
        assert_eq!(division.roster_owner, b"SOV");
        assert_eq!(division.logical_owner, b"D01");
        assert_eq!(
            division.expeditionary_owner.as_deref(),
            Some(b"GER".as_slice())
        );
        assert_eq!(division.province, Some(6906));
        assert_eq!(division.template, Some((u64::from(u32::MAX) << 32) | 6000));
        assert_eq!(division.hit_points.raw, 9_007_199_254_740_993);
        assert_eq!(division.organisation.raw, -1);
        let without_optional = &save.divisions[&((2_147_483_649_u64 << 32) | 4920)];
        assert_eq!(without_optional.province, None);
        assert_eq!(without_optional.template, None);
        assert_eq!(without_optional.expeditionary_owner, None);
    }

    #[test]
    fn division_missing_required_values_and_conflicting_duplicates_are_refused() {
        let valid = "division={id={id=1 type=51} logical_country=SOV strength=0 organisation=0}";
        for fragment in [
            valid.replace("id={id=1 type=51}", ""),
            valid.replace("type=51", ""),
            valid.replace("id=1", ""),
            valid.replace("logical_country=SOV", ""),
            valid.replace("strength=0", ""),
            valid.replace("organisation=0", ""),
            valid.replace("strength=0", "strength=0.000001"),
            valid.replace("type=51", "type=4294967296"),
            valid.replace(
                "logical_country=SOV",
                "logical_country=SOV logical_country=D01",
            ),
            valid.replace("organisation=0", "organisation=0 organisation=0"),
            valid.replace(
                "logical_country=SOV",
                "id={id=2 type=51} logical_country=SOV",
            ),
            format!("{valid} {valid}"),
        ] {
            assert!(division_save(&fragment).is_err(), "{fragment}");
        }
    }

    #[test]
    fn division_explicit_invalid_optionals_are_not_treated_as_absent() {
        let prefix = "division={id={id=1 type=51} logical_country=SOV strength=0 organisation=0 ";
        for optional in [
            "location=0",
            "location=-1",
            "location={}",
            "location=1 location=1",
            "division_template_id={}",
            "division_template_id={id=2}",
            "division_template_id={type=52}",
            "division_template_id={id=0 type=0}",
            "division_template_id=no",
            "expeditionary_owner={}",
        ] {
            assert!(
                division_save(&format!("{prefix}{optional}}}")).is_err(),
                "{optional}"
            );
        }
        assert!(
            division_save(
                "division={id={id=0 type=0} logical_country=SOV strength=0 organisation=0}"
            )
            .is_err()
        );
        for handle in ["id=0 type=51", "id=1 type=0"] {
            let body = format!(
                "division={{id={{{handle}}} logical_country=SOV strength=0 organisation=0 division_template_id={{{handle}}}}}"
            );
            assert!(division_save(&body).is_ok(), "{body}");
        }
    }

    #[test]
    fn duplicate_division_handle_in_another_country_roster_is_refused() {
        let body = "division={id={id=1 type=51} logical_country=D01 strength=1 organisation=1}";
        let text = format!(
            "HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 countries={{ SOV={{units={{{body}}}}} D01={{units={{{body}}}}} }} states={{}} provinces={{}}"
        );
        assert!(
            parse(text.as_bytes())
                .unwrap_err()
                .to_string()
                .contains("duplicate full division handle")
        );
    }

    const WAR_FIELDS: &str = "first=GER second=SOV start_date=1936.1.1.12 first_casualties=0 second_casualties=0 first_unknown_casualties=0 second_unknown_casualties=0 first_was_instigator=yes";

    fn diplomacy_save(relations: &str, factions: &str) -> io::Result<Save> {
        parse(format!(
            "HOI4txt\ndate=1936.1.2.1 game_unique_id=c version=b game_unique_seed=0 countries={{ GER={{diplomacy={{active_relations={{{relations}}}}}}} SOV={{}} D01={{}} }} states={{}} provinces={{}} {factions}"
        ).as_bytes())
    }

    #[test]
    fn war_relations_preserve_envelopes_order_and_unscaled_signed_integers() {
        let body = WAR_FIELDS
            .replace("first=GER", "first=D01")
            .replace("second=SOV", "second=GER")
            .replace(
                "first_casualties=0",
                "first_casualties=-9223372036854775808",
            )
            .replace(
                "second_casualties=0",
                "second_casualties=9223372036854775807",
            )
            .replace(
                "first_unknown_casualties=0",
                "first_unknown_casualties=9007199254740993",
            )
            .replace(
                "second_unknown_casualties=0",
                "second_unknown_casualties=-1",
            )
            .replace("first_was_instigator=yes", "first_was_instigator=no");
        let save = diplomacy_save(&format!("SOV={{war_relation={{{body}}}}}"), "").unwrap();
        let war = &save.war_relations[0];
        assert_eq!(war.roster_owner, b"GER");
        assert_eq!(war.target, b"SOV");
        // Canonical equivalence and envelope agreement belong to the oracle.
        assert_eq!(war.first, b"D01");
        assert_eq!(war.second, b"GER");
        assert_eq!(war.first_casualties, i64::MIN);
        assert_eq!(war.second_casualties, i64::MAX);
        assert_eq!(war.first_unknown_casualties, 9_007_199_254_740_993);
        assert_eq!(war.second_unknown_casualties, -1);
        assert!(!war.first_was_instigator);
        assert!(!war.cancel);
        assert_eq!(
            war.start_date.unwrap(),
            GameDate::parse(b"1936.1.1.12").unwrap()
        );
        assert_eq!(war.end_date, None);
    }

    #[test]
    fn war_dates_preserve_native_omissions_without_filling_raw_hours() {
        let cancelled = WAR_FIELDS.replace("start_date=1936.1.1.12", "cancel=yes");
        for suffix in ["", " end_date=1936.1.2.1"] {
            let save = diplomacy_save(&format!("SOV={{war_relation={{{cancelled}{suffix}}}}}"), "")
                .unwrap();
            let war = &save.war_relations[0];
            assert!(war.cancel);
            assert_eq!(war.start_date, None);
            assert_eq!(war.end_date.is_some(), !suffix.is_empty());
        }
        let save = diplomacy_save(
            &format!("SOV={{war_relation={{{WAR_FIELDS} end_date=1936.1.1.13}}}}"),
            "",
        )
        .unwrap();
        let war = &save.war_relations[0];
        assert_eq!(
            war.end_date.unwrap().engine_hour().unwrap(),
            war.start_date.unwrap().engine_hour().unwrap() + 1
        );
        let empty = diplomacy_save("SOV={opinion=10}", "").unwrap();
        assert!(empty.war_relations.is_empty());
        assert!(empty.factions.is_empty());
    }

    #[test]
    fn war_missing_required_fields_and_malformed_values_are_refused() {
        for required in WAR_FIELDS.split(' ') {
            let body = WAR_FIELDS.replace(required, "");
            assert!(
                diplomacy_save(&format!("SOV={{war_relation={{{body}}}}}"), "").is_err(),
                "missing {required}"
            );
        }
        for body in [
            WAR_FIELDS.replace("first=GER", "first={}"),
            WAR_FIELDS.replace("second=SOV", "second=\"\""),
            WAR_FIELDS.replace("first_casualties=0", "first_casualties=0.0"),
            WAR_FIELDS.replace(
                "second_casualties=0",
                "second_casualties=9223372036854775808",
            ),
            WAR_FIELDS.replace(
                "first_unknown_casualties=0",
                "first_unknown_casualties=-9223372036854775809",
            ),
            WAR_FIELDS.replace("first_was_instigator=yes", "first_was_instigator=1"),
            WAR_FIELDS.replace("start_date=1936.1.1.12", "start_date=1936.2.29.12"),
            WAR_FIELDS.replace("start_date=1936.1.1.12", "cancel=no"),
            format!("{WAR_FIELDS} cancel=no"),
            format!("{WAR_FIELDS} cancel=yes"),
            format!("{WAR_FIELDS} cancel={{}}"),
            format!("{WAR_FIELDS} end_date=1936.1.1.12"),
            format!("{WAR_FIELDS} end_date=1936.1.1.11"),
            format!("{WAR_FIELDS} end_date={{}}"),
            format!("{WAR_FIELDS} first_casualties=0"),
            format!("{WAR_FIELDS} first_was_instigator=yes"),
        ] {
            assert!(
                diplomacy_save(&format!("SOV={{war_relation={{{body}}}}}"), "").is_err(),
                "{body}"
            );
        }
    }

    #[test]
    fn war_duplicate_pairs_are_refused_across_target_envelopes() {
        let war = format!("war_relation={{{WAR_FIELDS}}}");
        for relations in [
            format!("SOV={{{war} {war}}}"),
            format!("SOV={{{war}}} D01={{{war}}}"),
        ] {
            assert!(diplomacy_save(&relations, "").is_err(), "{relations}");
        }
        let reversed = WAR_FIELDS
            .replace("first=GER", "first=SOV")
            .replace("second=SOV", "second=GER");
        let save = diplomacy_save(
            &format!("SOV={{{war}}} GER={{war_relation={{{reversed}}}}}"),
            "",
        )
        .unwrap();
        assert_eq!(save.war_relations.len(), 2);
        assert_eq!(save.war_relations[1].first, b"SOV");
    }

    #[test]
    fn factions_preserve_full_handles_literal_name_and_leader_order() {
        let save = diplomacy_save(
            "",
            r#"faction={id={type=88 id=4294967295} members={SOV GER D01} template=faction_template_allies}
faction={id={id=1 type=0} name="Literal name"}
faction={id={id=0 type=88} members={} name=""}"#,
        )
        .unwrap();
        let full = (u64::from(u32::MAX) << 32) | 88;
        assert_eq!(save.factions.len(), 3);
        let faction = &save.factions[&full];
        assert_eq!(faction.id, full);
        assert!(faction.name.is_empty());
        assert_eq!(
            faction.members,
            [b"SOV".to_vec(), b"GER".to_vec(), b"D01".to_vec()]
        );
        let no_members = &save.factions[&(1_u64 << 32)];
        assert_eq!(no_members.name, b"Literal name");
        assert!(no_members.members.is_empty());
        assert!(save.factions[&88].name.is_empty());
        assert!(save.factions[&88].members.is_empty());
    }

    #[test]
    fn factions_refuse_duplicate_handles_members_and_malformed_fields() {
        for body in [
            "",
            "id={}",
            "id={id=1}",
            "id={type=88}",
            "id={id=0 type=0}",
            "id={id=4294967296 type=88}",
            "id={id=1 type=-1}",
            "id={id=1 type=88 type=88}",
            "id={id=1 type=88} id={id=1 type=88}",
            "id=no",
            "id={id=1 type=88} name={}",
            "id={id=1 type=88} name=x name=x",
            "id={id=1 type=88} members=no",
            "id={id=1 type=88} members={GER GER}",
            "id={id=1 type=88} members={GER=yes}",
            "id={id=1 type=88} members={{GER}}",
            "id={id=1 type=88} members={\"\"}",
            "id={id=1 type=88} members={} members={}",
        ] {
            assert!(
                diplomacy_save("", &format!("faction={{{body}}}")).is_err(),
                "{body}"
            );
        }
        let duplicate = "faction={id={type=88 id=1}} faction={id={id=1 type=88}}";
        assert!(
            diplomacy_save("", duplicate)
                .unwrap_err()
                .to_string()
                .contains("duplicate full faction handle")
        );
    }

    #[test]
    fn land_combat_keeps_typed_participants_and_does_not_substitute_front_units() {
        let save = diplomacy_save(
            "",
            "combat={land_combat={
            id={type=62 id=3} location=42 day=10 duration=240
            attacker={unit={type=51 id=7} front={unit={type=51 id=99}} unit={type=52 id=7}}
            defender={unit={type=51 id=8}}
        }}",
        )
        .unwrap();
        let combat = &save.land_combats[&((3_u64 << 32) | 62)];
        assert_eq!(combat.province, 42);
        assert_eq!(combat.attackers, [(7_u64 << 32) | 51, (7_u64 << 32) | 52]);
        assert_eq!(combat.defenders, [(8_u64 << 32) | 51]);
        let duplicate =
            "land_combat={id={id=1 type=62} location=2 day=0 duration=0 attacker={} defender={}}";
        assert!(diplomacy_save("", &format!("combat={{{duplicate} {duplicate}}}")).is_err());
        assert!(
            diplomacy_save(
                "",
                "combat={land_combat={id={id=1 type=62} location=2 attacker={}}}"
            )
            .is_err()
        );
    }

    #[test]
    fn player_header_preserves_coop_entries_empty_users_and_signed_ids() {
        let save = diplomacy_save(
            "",
            r#"player_countries={
            GER={user="" country_leader=no}
            GER={user="same" country_leader=yes id=-2147483648}
            SOV={user="same" country_leader=no id=2147483647}
            ---={user="zero" country_leader=no id=0}
        }"#,
        )
        .unwrap();
        assert_eq!(save.players.len(), 4);
        assert_eq!(save.players[0].country, b"GER");
        assert!(save.players[0].user.is_empty());
        assert_eq!(save.players[0].id, -1);
        assert_eq!(save.players[1].id, i32::MIN);
        assert!(save.players[1].country_leader);
        assert_eq!(save.players[2].id, i32::MAX);
        assert_eq!(save.players[3].country, b"---");
        assert_eq!(save.players[3].id, 0);
        assert!(diplomacy_save("", "").unwrap().players.is_empty());
    }

    #[test]
    fn player_header_requires_serialized_fields_and_preserves_ambiguity() {
        for entry in [
            "user=x",
            "country_leader=yes",
            "user=x country_leader=maybe",
            "user=x country_leader=yes id=2147483648",
            "user=x country_leader=yes id=-2147483649",
            "user=x user=y country_leader=yes",
            "user={} country_leader=yes",
            "user=x country_leader=yes id=1 id=1",
        ] {
            assert!(
                diplomacy_save("", &format!("player_countries={{GER={{{entry}}}}}")).is_err(),
                "{entry}"
            );
        }
        let saved = diplomacy_save("", "player_countries={GER={user=x country_leader=yes id=1} GER={user=x country_leader=yes id=1}}").unwrap();
        assert_eq!(saved.players.len(), 2);
        assert_eq!(saved.players[0], saved.players[1]);
    }

    #[test]
    fn army_orders_preserve_all_lists_marshal_ownership_and_signed_native_bits() {
        let saved = parse(br#"HOI4txt
date=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0
states={} provinces={} countries={GER={theatres={theatre={id={id=1 type=67}
orders_group={id={type=53 id=1}
 order_instance={instance_id=4294967295 type=4 can_execute=-128 invasion_source=-1 path={2 1 2 -1} states={3 3}}
 fallback={instance_id=2 type=2}
 virtual_fallback={instance_id=3 type=-1}}
field_marshal_group={id={type=53 id=2} field_marshal_group=yes orders_group={id={id=1 type=53}}
 order_instance={instance_id=2 type=3 invasion_source=4}}
}}}}
"#).unwrap();
        assert_eq!(saved.army_orders.len(), 4);
        let group = (1_u64 << 32) | 53;
        let order = &saved.army_orders[&(group, u32::MAX)];
        assert_eq!(
            (order.order_type, order.active, order.paradrop_target),
            (4, 128, u32::MAX)
        );
        assert_eq!(order.path, [2, 1, 2, u32::MAX]);
        assert_eq!(order.target_states, [3, 3]);
        assert!(saved.army_orders[&(group, 2)].path.is_empty());
        assert_eq!(
            saved.army_orders[&((2_u64 << 32) | 53, 2)].paradrop_target,
            4
        );
    }

    #[test]
    fn air_wings_preserve_pool_and_override_countries_and_required_mission_fields() {
        let saved = diplomacy_save("", "strategic_air={GER={air_wing_pool={
            id={id=1 type=70} air_base={id=2 type=63}
            air_wings={id={id=3 type=54} count=-1 tag=--- experience=0 equipment={allow_zero_entries=no} mission={type=-2147483648}}
            air_wings={id={id=4 type=54} count=100 tag=SOV experience=0 equipment={allow_zero_entries=no} mission={type=2 effective_planes_count=57 strategic_region=5}}
        }}}").unwrap();
        assert_eq!(saved.air_wings.len(), 2);
        let wing = &saved.air_wings[&((3_u64 << 32) | 54)];
        assert_eq!(wing.owner, b"GER");
        assert_eq!(wing.country_override, b"---");
        assert_eq!(
            (wing.planes, wing.effective_planes, wing.mission_mask),
            (-1, 0, 1 << 31)
        );
        assert_eq!(wing.mission_region, None);
        let body = "strategic_air={GER={air_wing_pool={id={id=1 type=70} air_base={id=2 type=63} air_wings={id={id=3 type=54} count=1 tag=GER experience=0 equipment={allow_zero_entries=no} mission={type=0}}}}}";
        assert!(diplomacy_save("", body).is_ok());
        for fragment in [
            body.replace("count=1", ""),
            body.replace("type=0", ""),
            body.replace("tag=GER", ""),
            body.replace("air_base={id=2 type=63}", ""),
        ] {
            assert!(diplomacy_save("", &fragment).is_err(), "{fragment}");
        }
        let error = diplomacy_save("", &body.replace("allow_zero_entries=no", ""))
            .unwrap_err()
            .to_string();
        assert!(error.contains("air save tables: missing serialized air boolean"));
    }

    #[test]
    fn naval_combat_keeps_full_member_handles_signed_slots_and_required_side_scalars() {
        let body = "combat={naval_combat={id={type=61 id=4294967295} location=2
            attacker={positioning=-0.00001 anti_air=90071992547.40993 group={
                member={unique_id=-1 state=-2147483648 ship={id=1 type=51} convoy={id=2 type=69} cached_info={tag=GER}}
                member={unique_id=0 state=32 cached_info={tag=---}}}}
            defender={positioning=0 anti_air=0}}}";
        let saved = diplomacy_save("", body).unwrap();
        let id = (u64::from(u32::MAX) << 32) | 61;
        assert_eq!(saved.naval_combats[&id].attacker_positioning.raw, -1);
        let member = &saved.naval_combat_participants[&(id, 0, u32::MAX)];
        assert_eq!(member.state, 1 << 31);
        assert_eq!(member.ship, Some((1_u64 << 32) | 51));
        assert_eq!(member.convoy, Some((2_u64 << 32) | 69));
        assert!(saved.naval_combat_participants[&(id, 0, 0)].ship.is_none());
        for fragment in [
            body.replace("unique_id=-1", ""),
            body.replace("anti_air=0", ""),
            body.replace("tag=GER", ""),
            body.replace("unique_id=0", "unique_id=-1"),
        ] {
            assert!(diplomacy_save("", &fragment).is_err(), "{fragment}");
        }
    }

    #[test]
    fn equipment_classification_follows_inheritance_and_names_ambiguous_cases() {
        let mut catalog = EquipmentCatalog::default();
        catalog
            .add(
                br#"equipments={
convoy={type=convoy} convoy_1={archetype=convoy} new_convoy={parent=convoy_1 archetype=convoy}
tank={type={armor light}} light_tank={archetype=tank}
unknown={archetype=missing} loop_a={parent=loop_b} loop_b={parent=loop_a}
conflict={parent=tank archetype=convoy} variable={type=@type_from_script}
}"#,
            )
            .unwrap();
        let types = catalog.resolve();
        assert_eq!(types.is_convoy(b"new_convoy"), Some(true));
        assert_eq!(types.is_convoy(b"light_tank"), Some(false));
        for name in [
            b"unknown".as_slice(),
            b"loop_a",
            b"loop_b",
            b"conflict",
            b"variable",
        ] {
            assert_eq!(types.is_convoy(name), None);
            assert!(types.limitations.contains_key(name));
        }
        let mut catalog = EquipmentCatalog::default();
        catalog
            .add(b"equipments={duplicate={type=convoy}}")
            .unwrap();
        catalog.add(b"equipments={duplicate={type=armor}}").unwrap();
        assert_eq!(catalog.resolve().is_convoy(b"duplicate"), None);
    }
    // Suggested additions to installed-content/src/save.rs::tests.
    // Pure strings only. No build or repository changes were made by this helper.

    fn naval_save(units: &str) -> io::Result<Save> {
        parse(format!(
        "HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 countries={{ GER={{units={{{units}}}}} SOV={{}} D01={{}} }} states={{}} provinces={{}}"
    ).as_bytes())
    }

    #[test]
    fn naval_save_keeps_nested_ownership_full_handles_and_exact_fixed5() {
        let saved = naval_save(
            r#"
      fleet={id={id=4294967295 type=61} home_base=2
        task_force={id={id=2147483648 type=4713} id={type=4713 id=2147483648}
          location=1 logical_country=SOV expeditionary_owner=D01 mission={mission=-1}
          ship={id={id=4294967295 type=51} definition=battleship
            strength=90071992547.40993 organisation=78.72001 experience=34.32537 sunk_convoys=-1
            history={sunk_ship={id={id=9 type=51} strength=99}}}
          ship={id={id=0 type=51} definition=destroyer strength=0 organisation=0}
        }
      }
      fleet={id={id=1 type=61}}
    "#,
        )
        .unwrap();
        let fleet_id = (u64::from(u32::MAX) << 32) | 61;
        let force_id = (2_147_483_648_u64 << 32) | 4713;
        let ship_id = (u64::from(u32::MAX) << 32) | 51;
        assert_eq!(
            (
                saved.fleets.len(),
                saved.task_forces.len(),
                saved.ships.len()
            ),
            (2, 1, 2)
        );
        assert_eq!(saved.fleets[&fleet_id].owner, b"GER");
        assert_eq!(saved.fleets[&fleet_id].home_base, Some(2));
        assert_eq!(saved.fleets[&((1_u64 << 32) | 61)].home_base, None);
        let force = &saved.task_forces[&force_id];
        assert_eq!(
            (force.fleet, force.province, force.mission),
            (fleet_id, Some(1), -1)
        );
        assert_eq!(force.roster_owner, b"GER");
        assert_eq!(force.logical_owner, b"SOV");
        assert_eq!(
            force.expeditionary_owner.as_deref(),
            Some(b"D01".as_slice())
        );
        let ship = &saved.ships[&ship_id];
        assert_eq!(ship.task_force, force_id);
        assert_eq!(ship.strength.raw, 9_007_199_254_740_993);
        assert_eq!(ship.organisation.raw, 7_872_001);
        assert_eq!(ship.experience.raw, 3_432_537);
        assert_eq!(ship.sunk_convoys, -1);
        let omitted = &saved.ships[&51];
        assert_eq!((omitted.experience.raw, omitted.sunk_convoys), (0, 0));
    }

    #[test]
    fn naval_save_refuses_ambiguous_census_and_missing_required_values() {
        let ship = "ship={id={id=1 type=51} strength=1 organisation=1}";
        let force = format!(
            "task_force={{id={{id=2 type=51}} logical_country=GER mission={{mission=0}} {ship}}}"
        );
        let fleet = format!("fleet={{id={{id=3 type=61}} {force}}}");
        assert!(naval_save(&fleet).is_ok());
        for invalid in [
            fleet.replace("strength=1", ""),
            fleet.replace("organisation=1", ""),
            fleet.replace("strength=1", "strength=1.000001"),
            fleet.replace("logical_country=GER", ""),
            fleet.replace("mission={mission=0}", "mission={}"),
            fleet.replace("mission={mission=0}", ""),
            fleet.replace("id={id=1 type=51}", "id={id=1 type=51} id={id=2 type=51}"),
            fleet.replace(ship, &format!("{ship} {ship}")),
            fleet.replace(&force, &format!("{force} {force}")),
            format!("{fleet} {fleet}"),
            format!("{fleet} fleet={{id={{id=4 type=61}} {force}}}"),
        ] {
            assert!(naval_save(&invalid).is_err(), "{invalid}");
        }
    }
}
