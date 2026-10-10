//! Passive current-hour task-force counter ETA operands. Read stored stats,
//! actual-location common modifiers and fuel; playback applies the searched
//! departure's separate terrain adjustment. Speed/visibility modifier results
//! are recorded before content distance scaling and halving. The reader never
//! refreshes stored abilities or calls getters. Exile is serialized; the raw
//! strategic-redeployment request is retained with unresolved task-force save law.
//!
//! One receiver packet shares selectors and ETA/risk keys. Original ledger
//! contributors independently demand their limited keys and stat fields without
//! own mission gates. Full risk borrows original speed, visibility and context
//! attempts. All demanded keys use one common graph. Repeated ship runs and HQ
//! entries retain occurrences; failure ends that receiver graph without retry.
//! Later task forces are separate observations. Metadata/root failures last
//! only this capture; array bounds and fixed stack storage bound traversal.
//! The pointer-free packet remains separate from transient authenticated owner
//! controls, consumed and cleared by shared acquisition before return. Combined
//! visitors consume original gates/header attempts; risk borrowers consume the
//! original nullable ledger certificate. Neither retries failures. World/hour
//! admission and live validation belong to the caller.

use crate::{
    Memory, ccountry, cfleet,
    cpdx_array::Array,
    creference_registry::ScopedResolver,
    cship, csub_unit_definition, ctask_force, cunit, cweather_manager,
    modifier_accumulator::{self as accumulator, Value},
    modifier_definition::{self as metadata, Operands},
    modifier_keys as keys, naval_mission_inputs as mission, naval_ship_modifier_inputs as naval,
    read,
};

const P: i64 = 100_000;
const KEYS: [u32; 5] = [
    keys::NAVAL_SPEED_FACTOR,
    keys::NAVAL_RANGE,
    keys::NAVAL_RANGE_FACTOR,
    keys::NAVY_WEATHER_PENALTY,
    keys::UNDERWAY_REPLENISHMENT_RANGE_FACTOR,
];
// These common modifier queries use province zero and include local sources.
// Detection retains its own selection.
const RISK_KEYS: [u32; 5] = [
    keys::NAVAL_DETECTION,
    keys::NAVAL_COORDINATION,
    keys::SPOTTING_CHANCE,
    keys::NIGHT_SPOTTING_CHANCE,
    keys::NAVY_VISIBILITY,
];
const TERM_COUNT: usize = 3 + RISK_KEYS.len();
const VALUE_COUNT: usize = KEYS.len() + RISK_KEYS.len();
// Detection reads its separate current-relation accumulator.
const TARGETED_INDEX: usize = VALUE_COUNT;
const TARGETED_SPOTTING_KEY: u32 = keys::SPOTTING_CHANCE_AGAINST;
type Terms = [Result<i64, &'static str>; TERM_COUNT];

/// Immediate loan from the original ordered SHIPS walk. A numerical arena
/// belongs to the caller and is prepared before this native callback. Native
/// pointers may be consumed only during `ship`; they must not be retained.
/// The sink borrows original ship-model attempts, including their failures,
/// and may acquire only genuinely unattempted outside-census model leaves.
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

/// Independent native results; no absent value is a numerical zero.
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

/// Independent same-pass mission-risk observation. An unreached native query
/// is `Ok(None)`; a failed demand or reached source is never numerical zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissionRiskInputs {
    pub sample_hour: i32,
    /// Raw fixed5 spotting result before adding unity.
    pub combined_detection: Result<Option<i64>, &'static str>,
    /// Raw combined values without an added unity term.
    pub coordination: Result<Option<i64>, &'static str>,
    pub spotting_chance: Result<Option<i64>, &'static str>,
    pub night_spotting_chance: Result<Option<i64>, &'static str>,
    pub visibility: Result<Option<i64>, &'static str>,
    /// ETA speed uses actual source, ignoring destination. Reuse original speed
    /// attempts; playback adds source terrain and loaded content scale.
    pub speed_base: Result<Option<i64>, &'static str>,
    pub speed_modifier: Result<Option<i64>, &'static str>,
    pub terrain_class: Result<Option<i32>, &'static str>,
    /// Original authenticated stat selector shared with ETA. Each stat leaf
    /// retains its own failure, independently of the common modifier graph.
    pub stats: Result<Option<crate::transport_spotting_graph::TaskForceStats>, &'static str>,
    pub roster_owner: Result<Option<u32>, &'static str>,
    pub location: Result<Option<u32>, &'static str>,
    pub region: Result<Option<u32>, &'static str>,
    pub location_controller: Result<Option<u32>, &'static str>,
    pub detached_activity: Result<Option<i32>, &'static str>,
    /// Original SHIPS header's signed native count. Zero proves the native
    /// carrier loop is empty; a failed or unobserved header proves nothing.
    pub ship_count: Result<Option<i32>, &'static str>,
    /// Original roster owner's relation toward actual-location controller.
    pub targeted_spotting_modifier: Result<Option<i64>, &'static str>,
    /// Actual-source terrain speed. Negative stored class borrows the original
    /// ordered SHIPS class observations.
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

/// Capture-local roots and fixed-key metadata, prepared without allocation.
pub(super) struct Reader<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    state: usize,
    countries: Option<Result<ccountry::Reader<'a, M>, &'static str>>,
    weather: Option<Result<cweather_manager::Reader<'a, M>, &'static str>>,
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
}

fn field<T: crate::memory::Raw>(
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

fn auth(memory: &impl Memory, base: usize, pointer: usize, rva: usize) -> Result<(), &'static str> {
    let expected = base
        .checked_add(rva)
        .ok_or("fresh task-force module overflow")? as u64;
    if field::<u64>(memory, pointer, 0)? != expected {
        return Err("fresh task-force source identity mismatch");
    }
    Ok(())
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
        }
    }

    pub(super) fn with_modifier_guard(mut self, guard: Result<(), &'static str>) -> Self {
        self.modifier_guard = guard;
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
        pointer: usize,
        class: Result<u32, &'static str>,
    ) -> Result<(), &'static str> {
        if self.ship_observer.is_none() || self.receiver_handle.is_none() {
            return Ok(());
        }
        // The caller already admitted the exact SHIPS pointer and parent.
        let handle = field::<u64>(self.memory, pointer, 8).and_then(|handle| {
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

    fn country(&mut self, raw: u32) -> Result<ccountry::Country, &'static str> {
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

    /// The exact metadata::operands branch, with its shared global roots retained
    /// as success/failure observations rather than reacquired for another key.
    fn definition(&mut self, index: usize) -> Result<Operands, &'static str> {
        let root = *self.metadata_root.get_or_insert_with(|| {
            let address = self
                .base
                .checked_add(metadata::TABLE_RVA)
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
                let callback_at = self
                    .base
                    .checked_add(metadata::ELIGIBILITY_CALLBACK_RVA)
                    .ok_or("fresh task-force metadata callback overflow")?;
                let callback = read::<u64>(self.memory, callback_at)
                    .ok_or("fresh task-force metadata callback unreadable")?;
                if callback == 0 {
                    return Ok(0);
                }
                if self
                    .base
                    .checked_add(metadata::ELIGIBILITY_FLAGS_GETTER_RVA)
                    .map(|address| address as u64)
                    != Some(callback)
                {
                    return Err("fresh task-force metadata callback unsupported");
                }
                let flags_at = self
                    .base
                    .checked_add(metadata::TRADE_FEATURE_FLAGS_RVA)
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

    /// One sorted accumulator census supplies all demanded keys. Reorder only
    /// the fixed lookup requests, never the native ship/HQ occurrences.
    fn requested_values(
        &self,
        address: usize,
        eta: bool,
        risk: bool,
        country: bool,
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
        let mut sorted = [Value::default(); VALUE_COUNT];
        accumulator::lookup_many_at(
            self.memory,
            address,
            &sorted_keys[..len],
            &mut sorted[..len],
        )?;
        let mut output = [Value::default(); VALUE_COUNT];
        for (&(_, index), value) in requests[..len].iter().zip(sorted) {
            output[index] = value;
        }
        Ok(output)
    }

    fn country_values(
        &self,
        address: usize,
        eta: bool,
        risk: bool,
    ) -> Result<[Value; VALUE_COUNT], &'static str> {
        self.requested_values(address, eta, risk, true)
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

    fn terms(&mut self, address: usize, eta: bool, risk: bool) -> Result<Terms, &'static str> {
        let values = self.requested_values(address, eta, risk, false)?;
        Ok(self.effective_terms(values, eta, risk))
    }

    /// One common graph for requested native keys at the task's actual location.
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
                auth(
                    self.memory,
                    self.base,
                    location,
                    crate::cprovince::VTABLE_RVA,
                )?;
                if risk && self.full_risk {
                    self.context.location =
                        field::<u32>(self.memory, location, crate::cprovince::ID).map(Some);
                    self.context.controller =
                        field::<u32>(self.memory, location, crate::cprovince::CONTROLLER).map(Some);
                }
                let region =
                    field::<u64>(self.memory, location, crate::cprovince::STRATEGIC_REGION)?
                        as usize;
                let id = crate::cstrategic_region::id(self.memory, self.base, region)?
                    .ok_or("fresh task-force location region absent")?;
                if risk {
                    self.context.region = Ok(Some(id));
                }
                let template =
                    field::<u64>(self.memory, region, crate::cstrategic_region::TEMPLATE)? as usize;
                auth(
                    self.memory,
                    self.base,
                    template,
                    crate::cstrategic_region::TEMPLATE_VTABLE_RVA,
                )?;
                let terrain = naval::terrain_from_template(self.memory, self.base, template)?;
                if risk && self.full_risk {
                    self.context.terrain_speed = self.speed_class.and_then(|class| {
                        if class < 0 {
                            return Err("risk original mixed terrain speed pending");
                        }
                        let (_, source) = naval::terrain_class(self.memory, terrain, class as u32)?;
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
                        add(record, crate::sweather_per_region::MODIFIERS)?,
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
            if fleet != 0 {
                auth(self.memory, self.base, fleet, cfleet::VTABLE_RVA)?;
                let reference = field::<u64>(self.memory, fleet, cfleet::LEADER)?;
                if let Some(leader) = self.references.resolve(self.memory, reference)? {
                    auth(
                        self.memory,
                        self.base,
                        leader,
                        crate::cnavy_leader::VTABLE_RVA,
                    )?;
                    if field::<u64>(self.memory, leader, 8)? != reference {
                        return Err("fresh task-force fleet leader reference mismatch");
                    }
                    sum(
                        &mut total,
                        self.terms(add(leader, mission::LEADER_MODIFIERS)?, eta, risk)?,
                    );
                }
            }
            let headquarters = Array::read(
                self.memory,
                add(task, ctask_force::ATTACHED_HEADQUARTERS)?,
                8,
            )
            .ok_or("fresh task-force headquarters invalid")?;
            let mut strongest: Terms = [Ok(0); TERM_COUNT];
            let acquired = crate::cpdx_array::pointers(
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
            let (source, speed_source) = naval::terrain_class(reader.memory, terrain, class)?;
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
        let walked = crate::cpdx_array::pointers(
            self.memory,
            ships,
            "fresh task-force ship pointer unreadable",
            |ordinal, ship| {
                auth(self.memory, self.base, ship, cship::VTABLE_RVA)?;
                if field::<u64>(self.memory, ship, cship::TASK_FORCE)? != task as u64 {
                    return Err("fresh task-force ship parent mismatch");
                }
                let class = field::<u32>(self.memory, ship, cship::CLASS_DATABASE_ID);
                if risk {
                    let original = self.ship_loan(ordinal, ship, class);
                    sink_result = sink_result.and(original);
                }
                let class = class?;
                match run {
                    Some((previous, count)) if previous == class => run = Some((class, count + 1)),
                    Some((previous, count)) => {
                        flush(self, previous, count, total)?;
                        run = Some((class, 1));
                    }
                    None => run = Some((class, 1)),
                }
                Ok(())
            },
        );
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
                speed.and_then(|value| crate::fixed5::fraction(value, i64::from(ships.count) * P));
        }
        *total = (*total).map(|value| {
            value.and_then(|value| crate::fixed5::fraction(value, i64::from(ships.count) * P))
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
            crate::cnavy_leader::VTABLE_RVA,
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

    fn deficit(&self, fuel: Result<i64, &'static str>, rva: usize) -> Result<i64, &'static str> {
        let fuel = fuel?;
        if fuel == P {
            return Ok(0);
        }
        let address = self
            .base
            .checked_add(rva)
            .ok_or("fresh task-force define address overflow")?;
        let define =
            read::<i64>(self.memory, address).ok_or("fresh task-force define unreadable")?;
        Ok(P.wrapping_sub(fuel).wrapping_mul(define) / P)
    }

    fn underway(
        &mut self,
        task: usize,
        roster: ccountry::Country,
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
                    add(country.pointer, ccountry::MODIFIER_ACCUMULATOR)?,
                    &[KEYS[4]],
                )?;
                self.effective(4, values[0])?
            }
        } else {
            self.effective(4, country_values?[4])?
        };
        // Loaded underway-replenishment range bonus.
        let address = self
            .base
            .checked_add(0x0331_bfc8)
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
        Result<ccountry::Country, &'static str>,
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
                    add(country.pointer, ccountry::MODIFIER_ACCUMULATOR)?,
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

    /// Current diplomacy matrix selected by original roster country and actual
    /// location controller. Raw zero and readable unlinked references select
    /// physical slot zero.
    fn targeted_spotting(
        &mut self,
        roster: ccountry::Country,
        controller: Result<Option<u32>, &'static str>,
    ) -> Result<i64, &'static str> {
        let raw = controller?.ok_or("risk targeted spotting actual location absent")?;
        let slot = self.country_slot(raw)?;
        let diplomacy =
            field::<u64>(self.memory, roster.pointer, ccountry::DIPLOMACY_STATUS)? as usize;
        auth(
            self.memory,
            self.base,
            diplomacy,
            crate::cdiplomacy_status::VTABLE_RVA,
        )?;
        if field::<u64>(self.memory, diplomacy, crate::cdiplomacy_status::COUNTRY)?
            != roster.pointer as u64
        {
            return Err("risk targeted spotting diplomacy owner mismatch");
        }
        let matrix = Array::read(
            self.memory,
            add(diplomacy, crate::cdiplomacy_status::RELATIONS)?,
            8,
        )
        .ok_or("risk targeted spotting relation matrix invalid")?;
        let status = matrix
            .element(slot, 8)
            .and_then(|at| read::<u64>(self.memory, at))
            .filter(|pointer| *pointer != 0)
            .ok_or("risk targeted spotting relation slot absent or unreadable")?
            as usize;
        auth(
            self.memory,
            self.base,
            status,
            crate::crelation_status::VTABLE_RVA,
        )?;
        if field::<u64>(
            self.memory,
            status,
            crate::crelation_status::DIPLOMACY_STATUS,
        )? != diplomacy as u64
        {
            return Err("risk targeted spotting relation owner mismatch");
        }
        let target = field::<u32>(self.memory, status, crate::crelation_status::TARGET_COUNTRY)?;
        if self.country_slot(target)? != slot {
            return Err("risk targeted spotting relation target mismatch");
        }
        let address = add(status, crate::crelation_status::MODIFIER + 0x10)?;
        let values = self.values(address, &[TARGETED_SPOTTING_KEY])?;
        self.effective(TARGETED_INDEX, values[0])
    }

    fn country_slot(&self, raw: u32) -> Result<u32, &'static str> {
        if raw == 0 {
            return Ok(0);
        }
        match self
            .countries
            .as_ref()
            .ok_or("risk original country resolver unavailable")?
        {
            Ok(countries) => countries.link(raw).map(|slot| slot.unwrap_or(0)),
            Err(reason) => Err(*reason),
        }
    }

    fn stats(&self, task: usize) -> Result<usize, &'static str> {
        let stats = field::<u64>(self.memory, task, cunit::STAT_BLOCK)? as usize;
        auth(
            self.memory,
            self.base,
            stats,
            csub_unit_definition::VTABLE_RVA,
        )?;
        Ok(stats)
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
                    crate::cpdx_array::pointers(
                        self.memory,
                        ships,
                        "risk original ship pointer unreadable",
                        |ordinal, ship| {
                            auth(self.memory, self.base, ship, cship::VTABLE_RVA)?;
                            if field::<u64>(self.memory, ship, cship::TASK_FORCE)? != task as u64 {
                                return Err("risk original ship parent mismatch");
                            }
                            let original = self.ship_loan(
                                ordinal,
                                ship,
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
                        crate::transport_spotting_graph::TaskForceStats::capture(
                            self.memory,
                            source.stats,
                        )
                    } else {
                        crate::transport_spotting_graph::TaskForceStats::capture_detection(
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

    /// An authenticated original demand survives a separate motion refusal.
    /// It reads risk stats and speed once, without range or simulation flags.
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
            stats.and_then(|stats| field(self.memory, stats, 8))
        } else {
            Ok(0)
        };
        self.speed_class = terrain_class;
        let terms = self.common_inputs(task, false, true).2;
        self.observe_ship_count(task);
        let (base_speed, terrain_class, speed_modifier, detached_activity) = if self.full_risk {
            let base = stats.and_then(|stats| field(self.memory, stats, 0x1a8));
            let fuel = self.fuel(task);
            let speed = terms[0].and_then(|value| {
                self.deficit(fuel, 0x0331_bd80)
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
    ) -> (FreshEtaInputs, Result<ccountry::Country, &'static str>) {
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
        Result<ccountry::Country, &'static str>,
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
        Result<ccountry::Country, &'static str>,
        MissionRiskInputs,
    ) {
        let stats = self.stats(task);
        let base_speed = stats.and_then(|stats| field(self.memory, stats, 0x1a8));
        let terrain_class = stats.and_then(|stats| field(self.memory, stats, 8));
        let range_base = self.modifier_guard.and_then(|_| {
            stats.and_then(|stats| field::<i64>(self.memory, stats, mission::STAT_RANGE))
        });
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
        // OUT_OF_FUEL_SPEED_FACTOR / OUT_OF_FUEL_RANGE_FACTOR; read only on the deficit branch.
        let observed_speed_modifier = terms[0].and_then(|value| {
            self.deficit(fuel, 0x0331_bd80)
                .map(|deficit| value.wrapping_add(deficit))
        });
        let speed_modifier = self.modifier_guard.and(observed_speed_modifier);
        let modified_range = range_base.and_then(|base| {
            let additive = terms[1]?;
            let factor = terms[2]?.wrapping_add(self.deficit(fuel, 0x0331_bdf8)?);
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
