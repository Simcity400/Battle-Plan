
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{
        FieldBatch,
        Raw,
        RecordField,
        RecordLayout,
        gather_records_once,
        scalar_from_bytes,
    },
    modifier_cache::Value,
    read,
    recent::Recent,
};
use crate::{
    ccountry,
    cfleet,
    cship::{self, modifier_sources as naval},
    ctask_force::{self, mission_layout as mission, targeted_spotting},
    cunit,
    cweather_manager,
};
use engine_native_parts::{
    creference_registry::ScopedResolver,
    csub_unit_definition,
    modifier_accumulator as accumulator,
    modifier_definition::{self as metadata, Operands},
};
use engine_executable::modifier_keys as keys;
use engine_sites::Rva;

const P: i64 = 100_000;
const KEYS: [u32; 5] = [
    keys::NAVAL_SPEED_FACTOR,
    keys::NAVAL_RANGE,
    keys::NAVAL_RANGE_FACTOR,
    keys::NAVY_WEATHER_PENALTY,
    keys::UNDERWAY_REPLENISHMENT_RANGE_FACTOR,
];
const RISK_KEYS: [u32; 5] = [
    keys::NAVAL_DETECTION,
    keys::NAVAL_COORDINATION,
    keys::SPOTTING_CHANCE,
    keys::NIGHT_SPOTTING_CHANCE,
    keys::NAVY_VISIBILITY,
];
const TERM_COUNT: usize = 3 + RISK_KEYS.len();
const VALUE_COUNT: usize = KEYS.len() + RISK_KEYS.len();
pub const CENSUS_KEYS: usize = VALUE_COUNT;

pub type Certificate = (u64, u64);

pub type CensusIdentity = (usize, usize, [u32; VALUE_COUNT]);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeldCensus {
    pub receiver: usize,
    pub address: usize,
    pub keys: [u32; VALUE_COUNT],
    pub values: [Value; VALUE_COUNT],
    pub certificate: Certificate,
}

impl HeldCensus {
    pub fn identity(&self) -> CensusIdentity {
        (self.address, self.receiver, self.keys)
    }
}

pub struct Censuses<'a> {
    pub held: &'a [HeldCensus],
    pub certify: &'a dyn Fn(usize) -> Option<Certificate>,
    pub log: &'a mut Vec<HeldCensus>,
}
const STAT_FIELDS: [(usize, usize); 3] = [(0, 8), (8, 4), (0x1a8, 16)];
const PROVINCE_FIELDS: [(usize, usize); 2] = [(0, 8), (crate::cprovince::STRATEGIC_REGION, 8)];
const REGION_FIELDS: [(usize, usize); 3] = [
    (0, 8),
    (engine_native_parts::cstrategic_region::TEMPLATE, 8),
    (engine_native_parts::cstrategic_region::ID, 4),
];
const TEMPLATE_FIELDS: [(usize, usize); 2] = [(0, 8), (0x130, 8)];
const FLEET_FIELDS: [(usize, usize); 2] = [(0, 8), (cfleet::LEADER, 8)];
const LEADER_FIELDS: [(usize, usize); 1] = [(0, 16)];

fn gather<const N: usize>(
    memory: &impl Memory,
    pointer: usize,
    fields: [(usize, usize); N],
) -> cship::GatheredObject<N> {
    let gathered = cship::GatheredObject::gather(memory, pointer, fields);
    if pointer != 0 && !gathered.complete() {
        engine_native_memory::batch_fallback::record(engine_native_memory::batch_fallback::Batch::FreshEtaGathers);
    }
    gathered
}

const TARGETED_INDEX: usize = VALUE_COUNT;
const TARGETED_SPOTTING_KEY: u32 = keys::SPOTTING_CHANCE_AGAINST;
type Terms = [Result<i64, &'static str>; TERM_COUNT];

pub trait OriginalRiskShips {
    fn begin(&mut self, receiver: u64, count: Result<i32, &'static str>);
    fn ship(
        &mut self,
        receiver: u64,
        ordinal: u32,
        pointer: usize,
        handle: Result<u64, &'static str>,
        class: Result<u32, &'static str>,
    ) -> Result<(), &'static str>;
    fn finish(&mut self, receiver: u64, original: Result<(), &'static str>);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreshEtaInputs {
    pub sample_hour: i32,
    pub base_speed: Result<i64, &'static str>,
    pub speed_modifier: Result<i64, &'static str>,
    pub terrain_class: Result<i32, &'static str>,
    pub modified_range: Result<i64, &'static str>,
    pub exiled: Result<u8, &'static str>,
    pub strategic_redeployment: Result<u8, &'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissionRiskInputs {
    pub sample_hour: i32,
    pub combined_detection: Result<Option<i64>, &'static str>,
    pub coordination: Result<Option<i64>, &'static str>,
    pub spotting_chance: Result<Option<i64>, &'static str>,
    pub night_spotting_chance: Result<Option<i64>, &'static str>,
    pub visibility: Result<Option<i64>, &'static str>,
    pub speed_base: Result<Option<i64>, &'static str>,
    pub speed_modifier: Result<Option<i64>, &'static str>,
    pub terrain_class: Result<Option<i32>, &'static str>,
    pub stats: Result<Option<TaskForceStats>, &'static str>,
    pub roster_owner: Result<Option<u32>, &'static str>,
    pub location: Result<Option<u32>, &'static str>,
    pub region: Result<Option<u32>, &'static str>,
    pub location_controller: Result<Option<u32>, &'static str>,
    pub detached_activity: Result<Option<i32>, &'static str>,
    pub ship_count: Result<Option<i32>, &'static str>,
    pub targeted_spotting_modifier: Result<Option<i64>, &'static str>,
    pub actual_terrain_speed: Result<Option<i64>, &'static str>,
}

#[derive(Clone, Copy)]
struct RiskContext {
    roster_owner: Result<u32, &'static str>,
    location: Result<Option<u32>, &'static str>,
    region: Result<Option<u32>, &'static str>,
    controller: Result<Option<u32>, &'static str>,
    ships: Option<Result<Array, &'static str>>,
    ships_walked: bool,
    targeted_spotting: Result<i64, &'static str>,
    terrain_speed: Result<i64, &'static str>,
}

impl Default for RiskContext {
    fn default() -> Self {
        Self {
            roster_owner: Err("risk original roster owner unavailable"),
            location: Err("risk original actual location unavailable"),
            region: Err("risk original actual region unavailable"),
            controller: Err("risk original location controller unavailable"),
            ships: None,
            ships_walked: false,
            targeted_spotting: Err("risk original targeted spotting unavailable"),
            terrain_speed: Err("risk original actual terrain speed unavailable"),
        }
    }
}

struct RiskSources {
    terms: Terms,
    stats: Result<usize, &'static str>,
    base_speed: Result<i64, &'static str>,
    speed_modifier: Result<i64, &'static str>,
    terrain_class: Result<i32, &'static str>,
    detached_activity: Result<i32, &'static str>,
}

pub(super) struct Reader<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    state: usize,
    countries: Option<Result<ccountry::Reader<'a, M>, &'static str>>,
    weather: Option<Result<cweather_manager::Reader<'a, M>, &'static str>>,
    ocean: Option<usize>,
    censuses: Recent<(usize, [u32; VALUE_COUNT]), [Value; VALUE_COUNT], 32>,
    terrain_classes: Recent<(usize, u32), (usize, usize), 16>,
    locations: Recent<usize, (u32, usize), 16>,
    world_locations: Option<&'a mut ctask_force::WorldHold<(u32, usize)>>,
    fleets: Recent<usize, Option<usize>, 16>,
    fleet_leaders: Option<(
        &'a mut ctask_force::FleetLeaders,
        ctask_force::Certify,
        ctask_force::Certify,
    )>,
    definitions: [Option<Result<Operands, &'static str>>; VALUE_COUNT + 1],
    metadata_root: Option<Result<usize, &'static str>>,
    eligibility: Option<Result<u32, &'static str>>,
    modifier_guard: Result<(), &'static str>,
    references: ScopedResolver,
    context: RiskContext,
    full_risk: bool,
    speed_class: Result<i32, &'static str>,
    ship_observer: Option<&'a mut dyn OriginalRiskShips>,
    receiver_handle: Option<u64>,
    certified: Option<Censuses<'a>>,
    stat_view: cship::GatheredObject<3>,
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    let address = pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .ok_or("fresh task-force address overflow or null")?;
    read(memory, address).ok_or("fresh task-force operand unreadable")
}

fn auth(memory: &impl Memory, base: usize, pointer: usize, rva: Rva) -> Result<(), &'static str> {
    let expected = engine_sites::at(base, rva).ok_or("fresh task-force module overflow")? as u64;
    if field::<u64>(memory, pointer, 0)? != expected {
        return Err("fresh task-force source identity mismatch");
    }
    Ok(())
}

const ROSTER_SHIP: [(usize, usize); 4] = [
    (0, 8),
    (cship::TASK_FORCE, 8),
    (cship::CLASS_DATABASE_ID, 4),
    (cship::HANDLE, 8),
];

const ROSTER_STRIDE: usize = 28;
const ROSTER_RECORD_FIELDS: [RecordField; 4] = engine_native_memory::memory::packed_fields(ROSTER_SHIP);
const ROSTER_RECORD: RecordLayout<'static> =
    RecordLayout::new(&ROSTER_RECORD_FIELDS, ROSTER_STRIDE).expect("roster ship record");
const ROSTER_OFFSETS: [usize; 4] = [0, 8, 16, 20];

enum RosterFields<'a> {
    Gathered(Option<FieldBatch<4>>),
    Record(&'a [u8]),
}

struct RosterShip<'a> {
    ship: usize,
    fields: RosterFields<'a>,
}

impl<'a> RosterShip<'a> {
    fn gather(memory: &impl Memory, ship: usize) -> Self {
        let addresses =
            ROSTER_SHIP.map(|(offset, _)| ship.checked_add(offset).filter(|_| ship != 0));
        let fields = addresses.iter().all(Option::is_some).then(|| {
            FieldBatch::gather(
                memory,
                std::array::from_fn(|index| (addresses[index].unwrap(), ROSTER_SHIP[index].1)),
            )
        });
        Self {
            ship,
            fields: RosterFields::Gathered(fields),
        }
    }

    fn record(ship: usize, bytes: &'a [u8]) -> Self {
        Self {
            ship,
            fields: RosterFields::Record(bytes),
        }
    }

    fn get<T: Raw>(&self, memory: &impl Memory, index: usize) -> Result<T, &'static str> {
        match &self.fields {
            RosterFields::Gathered(Some(fields)) => fields
                .get(memory, index)
                .ok_or("fresh task-force operand unreadable"),
            RosterFields::Gathered(None) => field(memory, self.ship, ROSTER_SHIP[index].0),
            RosterFields::Record(bytes) => {
                let first = ROSTER_OFFSETS[index];
                scalar_from_bytes(&bytes[first..first + std::mem::size_of::<T>()])
                    .ok_or("fresh task-force operand unreadable")
            }
        }
    }

    fn admit(
        &self,
        memory: &impl Memory,
        base: usize,
        task: usize,
        parent_mismatch: &'static str,
    ) -> Result<(), &'static str> {
        let expected = engine_sites::at(base, cship::VTABLE_RVA)
            .ok_or("fresh task-force module overflow")? as u64;
        if self.get::<u64>(memory, 0)? != expected {
            return Err("fresh task-force source identity mismatch");
        }
        if self.get::<u64>(memory, 1)? != task as u64 {
            return Err(parent_mismatch);
        }
        Ok(())
    }
}

fn add(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .ok_or("fresh task-force source address overflow or null")
}

fn sum(total: &mut Terms, terms: Terms) {
    for (total, term) in total.iter_mut().zip(terms) {
        *total = total.and_then(|value| term.map(|term| value.wrapping_add(term)));
    }
}

impl<'a, M: Memory> Reader<'a, M> {
    pub(super) fn new(memory: &'a M, base: usize, state: usize) -> Self {
        Self {
            memory,
            base,
            state,
            countries: None,
            weather: None,
            ocean: None,
            censuses: Recent::default(),
            terrain_classes: Recent::default(),
            locations: Recent::default(),
            world_locations: None,
            fleets: Recent::default(),
            fleet_leaders: None,
            definitions: [None; VALUE_COUNT + 1],
            metadata_root: None,
            eligibility: None,
            modifier_guard: Err("fresh task-force modifier layout unverified"),
            references: ScopedResolver::new(base),
            context: RiskContext::default(),
            full_risk: false,
            speed_class: Err("risk original speed class unreached"),
            ship_observer: None,
            receiver_handle: None,
            certified: None,
            stat_view: cship::GatheredObject::gather(memory, 0, STAT_FIELDS),
        }
    }

    pub(super) fn with_censuses(mut self, censuses: Censuses<'a>) -> Self {
        self.certified = Some(censuses);
        self
    }

    pub(super) fn with_modifier_guard(mut self, guard: Result<(), &'static str>) -> Self {
        self.modifier_guard = guard;
        self
    }

    pub(super) fn with_world_locations(
        mut self,
        held: &'a mut ctask_force::WorldHold<(u32, usize)>,
    ) -> Self {
        self.world_locations = Some(held);
        self
    }

    pub(super) fn with_fleet_leaders(
        mut self,
        held: &'a mut ctask_force::FleetLeaders,
        fleets: ctask_force::Certify,
        leaders: ctask_force::Certify,
    ) -> Self {
        self.fleet_leaders = Some((held, fleets, leaders));
        self
    }

    pub(super) fn with_risk_ship_observer(
        mut self,
        observer: &'a mut dyn OriginalRiskShips,
    ) -> Self {
        self.ship_observer = Some(observer);
        self
    }

    pub(super) fn set_receiver(&mut self, handle: u64) {
        self.receiver_handle = Some(handle);
    }

    fn ship_header(&mut self, original: Result<Array, &'static str>) {
        self.context.ships = Some(original);
        if let (Some(observer), Some(receiver)) =
            (self.ship_observer.as_deref_mut(), self.receiver_handle)
        {
            let count = original.and_then(|array| {
                i32::try_from(array.count).map_err(|_| "risk native ship count is negative")
            });
            observer.begin(receiver, count);
        }
    }

    fn ship_loan(
        &mut self,
        ordinal: u32,
        ship: &RosterShip<'_>,
        class: Result<u32, &'static str>,
    ) -> Result<(), &'static str> {
        if self.ship_observer.is_none() || self.receiver_handle.is_none() {
            return Ok(());
        }
        let pointer = ship.ship;
        let handle = ship.get::<u64>(self.memory, 3).and_then(|handle| {
            if handle >> 32 == 0 || handle as u32 != cship::HANDLE_TYPE {
                Err("risk original ship full handle unsupported")
            } else {
                Ok(handle)
            }
        });
        if let (Some(observer), Some(receiver)) =
            (self.ship_observer.as_deref_mut(), self.receiver_handle)
        {
            observer.ship(receiver, ordinal, pointer, handle, class)
        } else {
            Ok(())
        }
    }

    fn finish_ship_loan(&mut self, result: Result<(), &'static str>) {
        if let (Some(observer), Some(receiver)) =
            (self.ship_observer.as_deref_mut(), self.receiver_handle)
        {
            observer.finish(receiver, result);
        }
    }

    fn country(&mut self, raw: u32) -> Result<engine_native_memory::country_identity::Country, &'static str> {
        let reader = self
            .countries
            .get_or_insert_with(|| ccountry::Reader::new(self.memory, self.base, self.state));
        match reader {
            Ok(reader) => reader.resolve(raw),
            Err(reason) => Err(*reason),
        }
    }

    fn effective(&mut self, index: usize, value: Value) -> Result<i64, &'static str> {
        if !value.present {
            return Ok(0);
        }
        if self.definitions[index].is_none() {
            self.definitions[index] = Some(self.definition(index));
        }
        let definition = self.definitions[index].expect("definition observation installed");
        definition.map(|definition| definition.effective(value.raw))
    }

    fn definition(&mut self, index: usize) -> Result<Operands, &'static str> {
        let root = *self.metadata_root.get_or_insert_with(|| {
            let address = engine_sites::at(self.base, metadata::TABLE_RVA)
                .ok_or("fresh task-force metadata root overflow")?;
            read::<u64>(self.memory, address)
                .filter(|pointer| *pointer != 0)
                .map(|pointer| pointer as usize)
                .ok_or("fresh task-force metadata root unreadable or null")
        });
        let key = if index == TARGETED_INDEX {
            TARGETED_SPOTTING_KEY
        } else if index >= KEYS.len() {
            RISK_KEYS[index - KEYS.len()]
        } else {
            KEYS[index]
        };
        let row = root?
            .checked_add(key as usize * metadata::STRIDE)
            .ok_or("fresh task-force metadata row overflow")?;
        let selector = field::<u32>(self.memory, row, metadata::FEATURE_SELECTOR)?;
        let eligibility_flags = if selector == 0 {
            0
        } else {
            (*self.eligibility.get_or_insert_with(|| {
                let callback_at = engine_sites::at(self.base, metadata::ELIGIBILITY_CALLBACK_RVA)
                    .ok_or("fresh task-force metadata callback overflow")?;
                let callback = read::<u64>(self.memory, callback_at)
                    .ok_or("fresh task-force metadata callback unreadable")?;
                if callback == 0 {
                    return Ok(0);
                }
                if engine_sites::at(self.base, metadata::ELIGIBILITY_FLAGS_GETTER_RVA)
                    .map(|address| address as u64)
                    != Some(callback)
                {
                    return Err("fresh task-force metadata callback unsupported");
                }
                let flags_at = engine_sites::at(self.base, metadata::TRADE_FEATURE_FLAGS_RVA)
                    .ok_or("fresh task-force metadata feature flags overflow")?;
                read::<u32>(self.memory, flags_at)
                    .ok_or("fresh task-force metadata feature flags unreadable")
            }))?
        };
        let flags = if selector == 0 || selector & eligibility_flags != 0 {
            field(self.memory, row, metadata::FLAGS)?
        } else {
            0
        };
        Ok(Operands {
            flags,
            selector,
            eligibility_flags,
        })
    }

    fn values<const N: usize>(
        &self,
        address: usize,
        keys: &[u32; N],
    ) -> Result<[Value; N], &'static str> {
        let mut values = [Value::default(); N];
        accumulator::lookup_many_at(self.memory, address, keys, &mut values)?;
        Ok(values)
    }

    fn requested_values(
        &mut self,
        address: usize,
        eta: bool,
        risk: bool,
        country: bool,
        receiver: Option<usize>,
    ) -> Result<[Value; VALUE_COUNT], &'static str> {
        let mut requests = [(0_u32, 0_usize); VALUE_COUNT];
        let mut len = 0;
        for index in 0..VALUE_COUNT {
            let requested = if index >= KEYS.len() {
                risk && (index == KEYS.len() || self.full_risk)
            } else if index == 0 {
                eta || (risk && self.full_risk)
            } else if index < 3 {
                eta
            } else if index == 3 {
                country && (eta || risk)
            } else {
                country && eta
            };
            if requested {
                let key = if index < KEYS.len() {
                    KEYS[index]
                } else {
                    RISK_KEYS[index - KEYS.len()]
                };
                requests[len] = (key, index);
                len += 1;
            }
        }
        requests[..len].sort_unstable_by_key(|&(key, _)| key);
        let mut sorted_keys = [0; VALUE_COUNT];
        for (key, &(value, _)) in sorted_keys.iter_mut().zip(&requests[..len]) {
            *key = value;
        }
        let sorted = match self.censuses.get(&(address, sorted_keys)) {
            Some(sorted) => sorted,
            None => {
                let certificate = match (receiver, self.certified.as_ref()) {
                    (Some(receiver), Some(certified)) => (certified.certify)(receiver),
                    _ => None,
                };
                let held = receiver
                    .zip(certificate)
                    .and_then(|(receiver, certificate)| {
                        let held = self.certified.as_ref()?.held;
                        let index = held
                            .binary_search_by_key(
                                &(address, receiver, sorted_keys),
                                HeldCensus::identity,
                            )
                            .ok()?;
                        (held[index].certificate == certificate).then_some(held[index].values)
                    });
                let sorted = match held {
                    Some(values) => values,
                    None => {
                        let mut sorted = [Value::default(); VALUE_COUNT];
                        accumulator::lookup_many_at(
                            self.memory,
                            address,
                            &sorted_keys[..len],
                            &mut sorted[..len],
                        )?;
                        sorted
                    }
                };
                if let Some((receiver, certificate)) = receiver.zip(certificate)
                    && let Some(certified) = self.certified.as_mut()
                    && engine_allocation::reserve(certified.log, 1).is_ok()
                {
                    certified.log.push(HeldCensus {
                        receiver,
                        address,
                        keys: sorted_keys,
                        values: sorted,
                        certificate,
                    });
                }
                self.censuses.insert((address, sorted_keys), sorted);
                sorted
            }
        };
        let mut output = [Value::default(); VALUE_COUNT];
        for (&(_, index), value) in requests[..len].iter().zip(sorted) {
            output[index] = value;
        }
        Ok(output)
    }

    fn country_values(
        &mut self,
        address: usize,
        country: usize,
        eta: bool,
        risk: bool,
    ) -> Result<[Value; VALUE_COUNT], &'static str> {
        self.requested_values(address, eta, risk, true, Some(country))
    }

    fn effective_terms(&mut self, values: [Value; VALUE_COUNT], eta: bool, risk: bool) -> Terms {
        std::array::from_fn(|index| {
            if index >= 3 {
                let slot = KEYS.len() + index - 3;
                if risk && (index == 3 || self.full_risk) {
                    self.effective(slot, values[slot])
                } else {
                    Ok(0)
                }
            } else if eta || (risk && self.full_risk && index == 0) {
                self.effective(index, values[index])
            } else {
                Ok(0)
            }
        })
    }

    fn certified_terms(
        &mut self,
        address: usize,
        receiver: usize,
        eta: bool,
        risk: bool,
    ) -> Result<Terms, &'static str> {
        let values = self.requested_values(address, eta, risk, false, Some(receiver))?;
        Ok(self.effective_terms(values, eta, risk))
    }

    fn terms(&mut self, address: usize, eta: bool, risk: bool) -> Result<Terms, &'static str> {
        let values = self.requested_values(address, eta, risk, false, None)?;
        Ok(self.effective_terms(values, eta, risk))
    }

    fn terrain_class(
        &mut self,
        terrain: usize,
        class: u32,
    ) -> Result<(usize, usize), &'static str> {
        let key = (terrain, class);
        if let Some(found) = self.terrain_classes.get(&key) {
            return Ok(found);
        }
        let found = naval::terrain_class(self.memory, terrain, class)?;
        self.terrain_classes.insert(key, found);
        Ok(found)
    }

    fn location(&mut self, location: usize, risk: bool) -> Result<(u32, usize), &'static str> {
        let found = match self.locations.get(&location) {
            Some(found) => Some(found),
            None => {
                let held = self
                    .world_locations
                    .as_deref_mut()
                    .and_then(|held| held.get(location));
                if let Some(found) = held {
                    self.locations.insert(location, found);
                }
                held
            }
        };
        if let Some(found) = found {
            if risk && self.full_risk {
                self.context.location =
                    field::<u32>(self.memory, location, crate::cprovince::ID).map(Some);
                self.context.controller =
                    field::<u32>(self.memory, location, crate::cprovince::CONTROLLER).map(Some);
            }
            if risk {
                self.context.region = Ok(Some(found.0));
            }
            return Ok(found);
        }
        let province = gather(self.memory, location, PROVINCE_FIELDS);
        let province = province.memory(self.memory);
        auth(&province, self.base, location, crate::cprovince::VTABLE_RVA)?;
        if risk && self.full_risk {
            self.context.location =
                field::<u32>(&province, location, crate::cprovince::ID).map(Some);
            self.context.controller =
                field::<u32>(&province, location, crate::cprovince::CONTROLLER).map(Some);
        }
        let region =
            field::<u64>(&province, location, crate::cprovince::STRATEGIC_REGION)? as usize;
        let region_fields = gather(self.memory, region, REGION_FIELDS);
        let region_memory = region_fields.memory(self.memory);
        let id = engine_native_parts::cstrategic_region::id(&region_memory, self.base, region)?
            .ok_or("fresh task-force location region absent")?;
        if risk {
            self.context.region = Ok(Some(id));
        }
        let template =
            field::<u64>(&region_memory, region, engine_native_parts::cstrategic_region::TEMPLATE)? as usize;
        let template_fields = gather(self.memory, template, TEMPLATE_FIELDS);
        let template_memory = template_fields.memory(self.memory);
        auth(
            &template_memory,
            self.base,
            template,
            engine_native_parts::cstrategic_region::TEMPLATE_VTABLE_RVA,
        )?;
        let terrain = naval::terrain_from_template_kept(
            &template_memory,
            self.base,
            template,
            &mut self.ocean,
        )?;
        let found = (id, terrain);
        self.locations.insert(location, found);
        if let Some(held) = self.world_locations.as_deref_mut() {
            held.hold(location, found);
        }
        Ok(found)
    }

    fn fleet_leader(&mut self, fleet: usize) -> Result<Option<usize>, &'static str> {
        if let Some(found) = self.fleets.get(&fleet) {
            return Ok(found);
        }
        if let Some((held, fleets, leaders)) = self.fleet_leaders.as_mut()
            && let Some(found) = held.get(fleet, *fleets, *leaders)
        {
            self.fleets.insert(fleet, found);
            return Ok(found);
        }
        let fleet_certificate = self
            .fleet_leaders
            .as_ref()
            .and_then(|(_, fleets, _)| fleets(fleet));
        let mut leader_certificate = Some((0, 0));
        let fleet_fields = gather(self.memory, fleet, FLEET_FIELDS);
        let fleet_memory = fleet_fields.memory(self.memory);
        auth(&fleet_memory, self.base, fleet, cfleet::VTABLE_RVA)?;
        let reference = field::<u64>(&fleet_memory, fleet, cfleet::LEADER)?;
        let leader = self.references.resolve(self.memory, reference)?;
        if let Some(leader) = leader {
            leader_certificate = self
                .fleet_leaders
                .as_ref()
                .and_then(|(_, _, leaders)| leaders(leader));
            let leader_fields = gather(self.memory, leader, LEADER_FIELDS);
            let leader_memory = leader_fields.memory(self.memory);
            auth(
                &leader_memory,
                self.base,
                leader,
                engine_native_parts::cnavy_leader::VTABLE_RVA,
            )?;
            if field::<u64>(&leader_memory, leader, 8)? != reference {
                return Err("fresh task-force fleet leader reference mismatch");
            }
        }
        self.fleets.insert(fleet, leader);
        if let Some((held, _, _)) = self.fleet_leaders.as_mut() {
            held.hold(fleet, leader, fleet_certificate.zip(leader_certificate));
        }
        Ok(leader)
    }

    fn common(
        &mut self,
        task: usize,
        country_values: [Value; VALUE_COUNT],
        eta: bool,
        risk: bool,
    ) -> Result<Terms, &'static str> {
        let mut total = self.effective_terms(country_values, eta, risk);
        let acquired = (|| -> Result<(), &'static str> {
            let location = field::<u64>(self.memory, task, cunit::LOCATION)? as usize;
            if location == 0 && risk {
                self.context.location = Ok(None);
                self.context.region = Ok(None);
                self.context.controller = Ok(None);
            }
            if location != 0 {
                let (id, terrain) = self.location(location, risk)?;
                if risk && self.full_risk {
                    self.context.terrain_speed = self.speed_class.and_then(|class| {
                        if class < 0 {
                            return Err("risk original mixed terrain speed pending");
                        }
                        let (_, source) = self.terrain_class(terrain, class as u32)?;
                        field(self.memory, source, 0x20)
                    });
                }
                let mut terrain_terms = [Ok(0); TERM_COUNT];
                let acquired = self.terrain_terms(task, terrain, &mut terrain_terms, eta, risk);
                sum(&mut total, terrain_terms);
                acquired?;
                let weather = self.weather.get_or_insert_with(|| {
                    cweather_manager::Reader::new(self.memory, self.base, self.state)
                });
                let record = match weather {
                    Ok(weather) => weather.movement_record(id as i32, true)?,
                    Err(reason) => return Err(*reason),
                };
                if record != 0 {
                    let mut weather = self.terms(
                        add(record, crate::cweather_manager::region_weather::MODIFIERS)?,
                        eta,
                        risk,
                    )?;
                    let reduction = self
                        .effective(3, country_values[3])
                        .map(|value| P.wrapping_add(value).clamp(0, P));
                    for value in &mut weather {
                        *value = value.and_then(|value| {
                            reduction.map(|factor| value.wrapping_mul(factor) / P)
                        });
                    }
                    sum(&mut total, weather);
                }
                sum(
                    &mut total,
                    self.terms(add(task, mission::LOCAL_MODIFIERS)?, eta, risk)?,
                );
            }
            let fleet = field::<u64>(self.memory, task, ctask_force::FLEET)? as usize;
            if fleet != 0
                && let Some(leader) = self.fleet_leader(fleet)?
            {
                sum(
                    &mut total,
                    self.certified_terms(
                        add(leader, mission::LEADER_MODIFIERS)?,
                        leader,
                        eta,
                        risk,
                    )?,
                );
            }
            let headquarters = Array::read(
                self.memory,
                add(task, ctask_force::ATTACHED_HEADQUARTERS)?,
                8,
            )
            .ok_or("fresh task-force headquarters invalid")?;
            let mut strongest: Terms = [Ok(0); TERM_COUNT];
            let acquired = engine_native_memory::cpdx_array::pointers(
                self.memory,
                headquarters,
                "fresh task-force headquarters unreadable",
                |_, entry| {
                    let commander =
                        field::<u64>(self.memory, entry, mission::HEADQUARTERS_COMMANDER)? as usize;
                    if commander == 0 {
                        return Ok(());
                    }
                    let reference =
                        field::<u64>(self.memory, commander, mission::COMMANDER_CHARACTER)?;
                    let mut terms = [Ok(0); TERM_COUNT];
                    let acquired = self.headquarters_terms(reference, &mut terms, eta, risk);
                    for (strongest, term) in strongest.iter_mut().zip(terms) {
                        *strongest = strongest.and_then(|value| term.map(|term| value.max(term)));
                    }
                    acquired
                },
            );
            sum(&mut total, strongest);
            acquired?;
            Ok(())
        })();
        if let Err(reason) = acquired {
            sum(&mut total, [Err(reason); TERM_COUNT]);
        }
        Ok(total)
    }

    fn terrain_terms(
        &mut self,
        task: usize,
        terrain: usize,
        total: &mut Terms,
        eta: bool,
        risk: bool,
    ) -> Result<(), &'static str> {
        let ships = Array::read(self.memory, add(task, ctask_force::SHIPS)?, 8)
            .ok_or("fresh task-force ship roster invalid");
        if risk {
            self.ship_header(ships);
            self.context.ships_walked = true;
            if let Err(reason) = ships {
                self.finish_ship_loan(Err(reason));
            }
        }
        let ships = ships?;
        let mut run: Option<(u32, i32)> = None;
        let mixed = risk && self.full_risk && matches!(self.speed_class, Ok(class) if class < 0);
        let mut speed = Ok(0_i64);
        let mut flush = |reader: &mut Self, class, count: i32, total: &mut Terms| {
            let (source, speed_source) = reader.terrain_class(terrain, class)?;
            if mixed {
                speed = speed.and_then(|sum| {
                    field::<i64>(reader.memory, speed_source, 0x20)
                        .map(|value| sum.wrapping_add(i64::from(count).wrapping_mul(value)))
                });
            }
            let terms = reader.terms(add(source, 0x10)?, eta, risk)?;
            sum(
                total,
                terms.map(|value| value.map(|value| i64::from(count).wrapping_mul(value))),
            );
            Ok::<(), &'static str>(())
        };
        let mut sink_result = Ok(());
        let mut visit = |reader: &mut Self, ordinal, ship: &RosterShip<'_>| {
            ship.admit(
                reader.memory,
                reader.base,
                task,
                "fresh task-force ship parent mismatch",
            )?;
            let class = ship.get::<u32>(reader.memory, 2);
            if risk {
                let original = reader.ship_loan(ordinal, ship, class);
                sink_result = sink_result.and(original);
            }
            let class = class?;
            match run {
                Some((previous, count)) if previous == class => run = Some((class, count + 1)),
                Some((previous, count)) => {
                    flush(reader, previous, count, total)?;
                    run = Some((class, 1));
                }
                None => run = Some((class, 1)),
            }
            Ok(())
        };
        let memory = self.memory;
        let walked = {
            let mut bytes = [0; ROSTER_STRIDE * engine_native_memory::cpdx_array::POINTER_PAGE];
            let mut complete = [false; engine_native_memory::cpdx_array::POINTER_PAGE];
            engine_native_memory::cpdx_array::pointer_pages(
                memory,
                ships,
                "fresh task-force ship pointer unreadable",
                |first, page| {
                    gather_records_once(
                        memory,
                        page,
                        &ROSTER_RECORD,
                        &mut bytes,
                        &mut complete,
                        engine_native_memory::batch_fallback::Batch::FreshEtaGathers,
                    );
                    for (index, &pointer) in page.iter().enumerate() {
                        let ship = if complete[index] {
                            RosterShip::record(
                                pointer,
                                &bytes[index * ROSTER_STRIDE..(index + 1) * ROSTER_STRIDE],
                            )
                        } else {
                            RosterShip::gather(memory, pointer)
                        };
                        visit(self, first + index as u32, &ship)?;
                    }
                    Ok(())
                },
            )
        };
        if risk {
            self.finish_ship_loan(walked.and(sink_result));
        }
        walked?;
        if let Some((class, count)) = run {
            flush(self, class, count, total)?;
        }
        if ships.count == 0 {
            if mixed {
                self.context.terrain_speed = Ok(0);
            }
            return Ok(());
        }
        if mixed {
            self.context.terrain_speed =
                speed.and_then(|value| engine_native_memory::fixed5::fraction(value, i64::from(ships.count) * P));
        }
        *total = (*total).map(|value| {
            value.and_then(|value| engine_native_memory::fixed5::fraction(value, i64::from(ships.count) * P))
        });
        Ok(())
    }

    fn headquarters_terms(
        &mut self,
        reference: u64,
        total: &mut Terms,
        eta: bool,
        risk: bool,
    ) -> Result<(), &'static str> {
        let Some(character) = self.references.resolve(self.memory, reference)? else {
            return Ok(());
        };
        auth(
            self.memory,
            self.base,
            character,
            crate::ccharacter::VTABLE_RVA,
        )?;
        if field::<u64>(self.memory, character, 8)? != reference {
            return Err("fresh task-force commander character mismatch");
        }
        let cached = field::<u64>(self.memory, character, mission::CHARACTER_UNIT_LEADER)? as usize;
        let leader = if cached != 0 {
            Some(cached)
        } else {
            let reference = field::<u64>(
                self.memory,
                character,
                mission::CHARACTER_UNIT_LEADER_REFERENCE,
            )?;
            self.references.resolve(self.memory, reference)?
        };
        let Some(leader) = leader else {
            return Ok(());
        };
        if field::<i32>(self.memory, leader, mission::UNIT_LEADER_KIND)? != 2 {
            return Ok(());
        }
        auth(
            self.memory,
            self.base,
            leader,
            engine_native_parts::cnavy_leader::VTABLE_RVA,
        )?;
        for offset in mission::UNIT_LEADER_SOURCES {
            let source = field::<u64>(self.memory, leader, offset)? as usize;
            sum(
                total,
                self.terms(add(source, mission::SOURCE_MODIFIERS)?, eta, risk)?,
            );
        }
        Ok(())
    }

    fn fuel(&self, task: usize) -> Result<i64, &'static str> {
        let requested = field::<i64>(self.memory, task, ctask_force::CURRENT_FUEL_USAGE)?;
        if requested < 1 {
            return Ok(P);
        }
        let fuel = field::<i64>(self.memory, task, mission::FUEL)?;
        if (fuel as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
            return Ok(P);
        }
        Ok((fuel.wrapping_mul(P) / requested).clamp(0, P))
    }

    fn deficit(&self, fuel: Result<i64, &'static str>, rva: Rva) -> Result<i64, &'static str> {
        let fuel = fuel?;
        if fuel == P {
            return Ok(0);
        }
        let address =
            engine_sites::at(self.base, rva).ok_or("fresh task-force define address overflow")?;
        let define =
            read::<i64>(self.memory, address).ok_or("fresh task-force define unreadable")?;
        Ok(P.wrapping_sub(fuel).wrapping_mul(define) / P)
    }

    fn underway(
        &mut self,
        task: usize,
        roster: engine_native_memory::country_identity::Country,
        country_values: Result<[Value; VALUE_COUNT], &'static str>,
    ) -> Result<i64, &'static str> {
        if field::<u8>(self.memory, task, mission::UNDERWAY)? == 0 {
            return Ok(0);
        }
        let numerator = field::<i32>(self.memory, task, mission::REPLENISHMENT)?;
        let denominator = field::<i32>(self.memory, task, mission::REPLENISHMENT + 4)?;
        let ratio = if denominator == 0 {
            P
        } else {
            (i64::from(numerator).wrapping_mul(P * P) / (i64::from(denominator) * P)).clamp(-P, P)
        };
        let expeditionary = field::<i32>(self.memory, task, cunit::EXPEDITIONARY_OWNER)?;
        let modifier = if expeditionary > 0 {
            let country = self.country(expeditionary as u32)?;
            if country.pointer == roster.pointer {
                self.effective(4, country_values?[4])?
            } else {
                let values = self.values(
                    add(country.pointer, engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)?,
                    &[KEYS[4]],
                )?;
                self.effective(4, values[0])?
            }
        } else {
            self.effective(4, country_values?[4])?
        };
        let address = engine_sites::at(self.base, Rva(0x033d_fbc0))
            .ok_or("fresh task-force define address overflow")?;
        let define =
            read::<i64>(self.memory, address).ok_or("fresh task-force define unreadable")?;
        Ok(modifier
            .wrapping_add(P)
            .wrapping_mul(define)
            .wrapping_div(P)
            .wrapping_mul(ratio)
            .wrapping_div(P))
    }

    fn common_inputs(
        &mut self,
        task: usize,
        eta: bool,
        risk: bool,
    ) -> (
        Result<engine_native_memory::country_identity::Country, &'static str>,
        Result<[Value; VALUE_COUNT], &'static str>,
        Terms,
    ) {
        self.context = RiskContext::default();
        let raw_owner = field::<u32>(self.memory, task, cunit::ROSTER_OWNER);
        self.context.roster_owner = raw_owner;
        let roster = raw_owner.and_then(|raw| self.country(raw));
        let country_values = if eta || risk {
            roster.and_then(|country| {
                self.country_values(
                    add(country.pointer, engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)?,
                    country.pointer,
                    eta,
                    risk,
                )
            })
        } else {
            self.modifier_guard
                .and(Err("fresh task-force common query unreached"))
        };
        let common = country_values.and_then(|values| self.common(task, values, eta, risk));
        if risk && self.full_risk {
            self.context.targeted_spotting =
                roster.and_then(|roster| self.targeted_spotting(roster, self.context.controller));
        }
        (
            roster,
            country_values,
            common.unwrap_or_else(|reason| [Err(reason); TERM_COUNT]),
        )
    }

    fn targeted_spotting(
        &mut self,
        roster: engine_native_memory::country_identity::Country,
        controller: Result<Option<u32>, &'static str>,
    ) -> Result<i64, &'static str> {
        let raw = controller?.ok_or("risk targeted spotting actual location absent")?;
        let countries = self
            .countries
            .as_ref()
            .ok_or("risk original country resolver unavailable")?
            .as_ref()
            .map_err(|reason| *reason)?;
        let value =
            targeted_spotting::raw(self.memory, self.base, countries, roster, raw)?;
        self.effective(TARGETED_INDEX, value)
    }
    fn stats(&mut self, task: usize) -> Result<usize, &'static str> {
        let stats = field::<u64>(self.memory, task, cunit::STAT_BLOCK)? as usize;
        self.stat_view = gather(self.memory, stats, STAT_FIELDS);
        auth(
            &self.stat_view.memory(self.memory),
            self.base,
            stats,
            csub_unit_definition::VTABLE_RVA,
        )?;
        Ok(stats)
    }

    fn stat<T: Raw>(&self, stats: usize, offset: usize) -> Result<T, &'static str> {
        field(&self.stat_view.memory(self.memory), stats, offset)
    }

    fn observe_ship_count(&mut self, task: usize) {
        if self.context.ships.is_none() {
            let original = add(task, ctask_force::SHIPS).and_then(|at| {
                Array::read(self.memory, at, 8).ok_or("fresh task-force ship roster invalid")
            });
            self.ship_header(original);
        }
        if self.ship_observer.is_some() && !self.context.ships_walked {
            self.context.ships_walked = true;
            let mut sink_result = Ok(());
            let walked = self
                .context
                .ships
                .expect("original ship header installed")
                .and_then(|ships| {
                    engine_native_memory::cpdx_array::pointers(
                        self.memory,
                        ships,
                        "risk original ship pointer unreadable",
                        |ordinal, ship| {
                            let ship = RosterShip::gather(self.memory, ship);
                            ship.admit(
                                self.memory,
                                self.base,
                                task,
                                "risk original ship parent mismatch",
                            )?;
                            let original = self.ship_loan(
                                ordinal,
                                &ship,
                                Err("risk ship class query unreached without source terrain"),
                            );
                            sink_result = sink_result.and(original);
                            Ok(())
                        },
                    )
                });
            self.finish_ship_loan(walked.and(sink_result));
        }
    }

    fn risk_packet(
        &self,
        sample_hour: i32,
        demand: Result<bool, &'static str>,
        guard: Result<(), &'static str>,
        contributor: bool,
        source: RiskSources,
    ) -> MissionRiskInputs {
        let reached = demand.and_then(|reached| {
            if reached {
                guard.map(|()| true)
            } else {
                Ok(false)
            }
        });
        let field = |value: Result<i64, &'static str>| {
            reached.and_then(|reached| if reached { value.map(Some) } else { Ok(None) })
        };
        let contribution = if contributor {
            guard.map(|()| true)
        } else {
            reached
        };
        MissionRiskInputs {
            sample_hour,
            combined_detection: contribution.and_then(|reached| {
                if reached {
                    source.terms[3].map(Some)
                } else {
                    Ok(None)
                }
            }),
            coordination: field(source.terms[4]),
            spotting_chance: field(source.terms[5]),
            night_spotting_chance: field(source.terms[6]),
            visibility: field(source.terms[7]),
            speed_base: field(source.base_speed),
            speed_modifier: field(source.speed_modifier),
            terrain_class: reached.and_then(|reached| {
                if reached {
                    source.terrain_class.map(Some)
                } else {
                    Ok(None)
                }
            }),
            stats: contribution.map(|contribution| {
                if contribution {
                    Some(if reached == Ok(true) {
                        TaskForceStats::capture(
                            self.memory,
                            source.stats,
                        )
                    } else {
                        TaskForceStats::capture_detection(
                            self.memory,
                            source.stats,
                        )
                    })
                } else {
                    None
                }
            }),
            roster_owner: reached.and_then(|reached| {
                if reached {
                    self.context.roster_owner.map(Some)
                } else {
                    Ok(None)
                }
            }),
            location: reached.and_then(|reached| {
                if reached {
                    self.context.location
                } else {
                    Ok(None)
                }
            }),
            region: reached.and_then(|reached| {
                if reached {
                    self.context.region
                } else {
                    Ok(None)
                }
            }),
            location_controller: reached.and_then(|reached| {
                if reached {
                    self.context.controller
                } else {
                    Ok(None)
                }
            }),
            detached_activity: reached.and_then(|reached| {
                if reached {
                    source.detached_activity.map(Some)
                } else {
                    Ok(None)
                }
            }),
            ship_count: contribution.and_then(|reached| {
                if !reached {
                    return Ok(None);
                }
                self.context
                    .ships
                    .ok_or("risk original ship header unavailable")?
                    .and_then(|ships| {
                        i32::try_from(ships.count)
                            .map(Some)
                            .map_err(|_| "risk native ship count is negative")
                    })
            }),
            targeted_spotting_modifier: field(self.context.targeted_spotting),
            actual_terrain_speed: field(self.context.terrain_speed),
        }
    }

    #[cfg(test)]
    pub(super) fn capture_mission_risk(
        &mut self,
        task: usize,
        sample_hour: i32,
        demand: Result<bool, &'static str>,
        guard: Result<(), &'static str>,
    ) -> MissionRiskInputs {
        self.capture_mission_risk_union(task, sample_hour, demand, guard, false)
    }

    pub(super) fn capture_mission_risk_union(
        &mut self,
        task: usize,
        sample_hour: i32,
        demand: Result<bool, &'static str>,
        guard: Result<(), &'static str>,
        contributor: bool,
    ) -> MissionRiskInputs {
        self.full_risk = demand == Ok(true) && guard.is_ok();
        if (!self.full_risk && !contributor) || guard.is_err() {
            return self.risk_packet(
                sample_hour,
                demand,
                guard,
                contributor,
                RiskSources {
                    terms: [Ok(0); TERM_COUNT],
                    stats: Err("risk stat query unreached"),
                    base_speed: Ok(0),
                    speed_modifier: Ok(0),
                    terrain_class: Ok(0),
                    detached_activity: Ok(0),
                },
            );
        }
        let stats = self.stats(task);
        let terrain_class = if self.full_risk {
            stats.and_then(|stats| self.stat(stats, 8))
        } else {
            Ok(0)
        };
        self.speed_class = terrain_class;
        let terms = self.common_inputs(task, false, true).2;
        self.observe_ship_count(task);
        let (base_speed, terrain_class, speed_modifier, detached_activity) = if self.full_risk {
            let base = stats.and_then(|stats| self.stat(stats, 0x1a8));
            let fuel = self.fuel(task);
            let speed = terms[0].and_then(|value| {
                self.deficit(fuel, Rva(0x033d_f978))
                    .map(|deficit| value.wrapping_add(deficit))
            });
            (
                base,
                terrain_class,
                speed,
                field(self.memory, task, ctask_force::DETACHED_ACTIVITY),
            )
        } else {
            (Ok(0), Ok(0), Ok(0), Ok(0))
        };
        self.risk_packet(
            sample_hour,
            demand,
            guard,
            contributor,
            RiskSources {
                terms,
                stats,
                base_speed,
                terrain_class,
                speed_modifier,
                detached_activity,
            },
        )
    }

    pub(super) fn capture(
        &mut self,
        task: usize,
        sample_hour: i32,
    ) -> (FreshEtaInputs, Result<engine_native_memory::country_identity::Country, &'static str>) {
        let (eta, owner, _) = self.capture_with_mission_risk(task, sample_hour, Ok(false), Ok(()));
        (eta, owner)
    }

    pub(super) fn capture_with_mission_risk(
        &mut self,
        task: usize,
        sample_hour: i32,
        demand: Result<bool, &'static str>,
        guard: Result<(), &'static str>,
    ) -> (
        FreshEtaInputs,
        Result<engine_native_memory::country_identity::Country, &'static str>,
        MissionRiskInputs,
    ) {
        self.capture_with_mission_risk_union(task, sample_hour, demand, guard, false)
    }

    pub(super) fn capture_with_mission_risk_union(
        &mut self,
        task: usize,
        sample_hour: i32,
        demand: Result<bool, &'static str>,
        guard: Result<(), &'static str>,
        contributor: bool,
    ) -> (
        FreshEtaInputs,
        Result<engine_native_memory::country_identity::Country, &'static str>,
        MissionRiskInputs,
    ) {
        let stats = self.stats(task);
        let base_speed = stats.and_then(|stats| self.stat(stats, 0x1a8));
        let terrain_class = stats.and_then(|stats| self.stat(stats, 8));
        let range_base = self
            .modifier_guard
            .and_then(|_| stats.and_then(|stats| self.stat::<i64>(stats, mission::STAT_RANGE)));
        let risk_reached = demand == Ok(true) && guard.is_ok();
        self.full_risk = risk_reached;
        self.speed_class = terrain_class;
        let contribution_reached = guard.is_ok() && (risk_reached || contributor);
        let (roster, country_values, terms) =
            self.common_inputs(task, self.modifier_guard.is_ok(), contribution_reached);
        if contribution_reached {
            self.observe_ship_count(task);
        }
        let fuel = if self.modifier_guard.is_ok() || risk_reached {
            self.fuel(task)
        } else {
            self.modifier_guard
                .and(Err("fresh task-force fuel query unreached"))
        };
        let observed_speed_modifier = terms[0].and_then(|value| {
            self.deficit(fuel, Rva(0x033d_f978))
                .map(|deficit| value.wrapping_add(deficit))
        });
        let speed_modifier = self.modifier_guard.and(observed_speed_modifier);
        let modified_range = range_base.and_then(|base| {
            let additive = terms[1]?;
            let factor = terms[2]?.wrapping_add(self.deficit(fuel, Rva(0x033d_fa08))?);
            let underway = self.underway(task, roster?, country_values)?;
            Ok(base
                .wrapping_add(additive)
                .wrapping_mul(P.wrapping_add(factor).wrapping_add(underway).max(P / 10))
                / P)
        });
        (
            FreshEtaInputs {
                sample_hour,
                base_speed,
                speed_modifier,
                terrain_class,
                modified_range,
                exiled: field(self.memory, task, cunit::EXILED),
                strategic_redeployment: field(self.memory, task, cunit::STRATEGIC_REDEPLOYMENT),
            },
            roster,
            self.risk_packet(
                sample_hour,
                demand,
                guard,
                contributor,
                RiskSources {
                    terms,
                    stats,
                    base_speed,
                    speed_modifier: observed_speed_modifier,
                    terrain_class,
                    detached_activity: if risk_reached {
                        field(self.memory, task, ctask_force::DETACHED_ACTIVITY)
                    } else {
                        Ok(0)
                    },
                },
            ),
        )
    }
}

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskForceStats {
    pub detection_138: ResultWord<i64>,
    pub visibility_148: ResultWord<i64>,
    pub visibility_150: ResultWord<i64>,
    pub carrier_detection_160: ResultWord<i64>,
}

impl TaskForceStats {
    pub fn capture(memory: &impl Memory, original: ResultWord<usize>) -> Self {
        Self::capture_with_visibility(memory, original, true)
    }

    pub fn capture_detection(memory: &impl Memory, original: ResultWord<usize>) -> Self {
        Self::capture_with_visibility(memory, original, false)
    }

    fn capture_with_visibility(
        memory: &impl Memory,
        original: ResultWord<usize>,
        visibility: bool,
    ) -> Self {
        let word = |offset| {
            original.and_then(|stats| {
                stats
                    .checked_add(offset)
                    .filter(|_| stats != 0)
                    .and_then(|at| read::<i64>(memory, at))
                    .ok_or("selected transport task-force stat word unreadable")
            })
        };
        Self {
            detection_138: word(0x138),
            visibility_148: if visibility {
                word(0x148)
            } else {
                Err("selected transport visibility stat query unreached")
            },
            visibility_150: if visibility {
                word(0x150)
            } else {
                Err("selected transport visibility stat query unreached")
            },
            carrier_detection_160: word(0x160),
        }
    }
}

type ResultWord<T> = Result<T, &'static str>;
