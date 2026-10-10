//! Numerical projection of the original AirMissions manager/header pass.
//! Allocating acquisition stays on the AirMissions/Late worker. Stored spans are numerical; native
//! memo keys retire before the original reader loan returns. Capacity pressure
//! survives terminal failure, including demand beyond admitted country rows.
use crate::table_capture::Arena;
use engine::{Memory, cair_regional_mission as native, cpdx_array::Array, cstrategic_air, read};
use replay::rows::{
    TransportAirCountry, TransportAirInputs, TransportAirMission, TransportAirRegion,
};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

type Span = Option<(usize, usize)>;

#[derive(Default)]
struct Country {
    country: Option<u32>,
    present: bool,
    regions: Span,
    factors: Span,
}

#[derive(Default)]
struct Region {
    attempted: bool,
    count: Option<i32>,
    missions: Span,
}

#[derive(Debug, Default, Clone, Copy)]
struct Mission {
    words: engine::transport_spotting_graph::AirMissionWords,
    normalization: Option<engine::transport_air_normalization::Source>,
}

#[derive(Clone, Copy)]
struct Memo {
    pointer: usize,
    region: u32,
    value: Result<Mission, &'static str>,
}
impl Default for Memo {
    fn default() -> Self {
        Self {
            pointer: 0,
            region: 0,
            value: Err("transport original air mission observation incomplete"),
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Refusals {
    pub domain: u64,
    pub headers: u64,
    pub identity: u64,
    pub leaves: u64,
    pub normalization: u64,
    pub capacity: u64,
    pub registration: u64,
    pub admission: u64,
}

const REFUSAL_NAMES: [&str; 8] = [
    "transport original strategic air root domain refused",
    "transport original air root or regional headers refused",
    "transport original air mission identity or backlink refused",
    "transport original air mission scalar or stat refused",
    "transport original normalized air producer graph refused",
    "transport original air graph capacity refused",
    "transport positive inactive air registration is unsupported",
    "transport original air native callback admission refused",
];
static REFUSALS: [AtomicU64; REFUSAL_NAMES.len()] =
    [const { AtomicU64::new(0) }; REFUSAL_NAMES.len()];
pub(crate) fn status() -> impl serde::Serialize {
    crate::status::map(
        REFUSAL_NAMES
            .into_iter()
            .zip(REFUSALS.iter().map(|count| count.load(Relaxed))),
    )
}
impl Refusals {
    fn values(self) -> [u64; REFUSAL_NAMES.len()] {
        [
            self.domain,
            self.headers,
            self.identity,
            self.leaves,
            self.normalization,
            self.capacity,
            self.registration,
            self.admission,
        ]
    }
}

pub(crate) struct Captured {
    countries: Arena<Country>,
    regions: Arena<Region>,
    missions: Arena<Option<Mission>>,
    factors: Arena<Option<i64>>,
    normalization: Option<engine::transport_air_normalization::Capture>,
    memo: Arena<Memo>,
    memo_index: Vec<usize>,
    hour: Option<i32>,
    complete: bool,
    pub refusals: Refusals,
    reported: [u64; REFUSAL_NAMES.len()],
}

fn reserve<T: Default>(arena: &mut Arena<T>, count: usize, growing: bool) -> Span {
    arena.required = arena.required.saturating_add(count);
    reserve_prepared(arena, count, growing)
}
fn reserve_prepared<T: Default>(arena: &mut Arena<T>, count: usize, growing: bool) -> Span {
    let end = arena.len.checked_add(count)?;
    if growing
        && engine::walk::Buffer::Growing(&mut arena.first)
            .ensure(end)
            .is_err()
    {
        return None;
    }
    if end > arena.first.len() {
        return None;
    }
    let first = arena.len;
    arena.len = end;
    for row in &mut arena.first[first..end] {
        *row = T::default();
    }
    Some((first, count))
}

fn normalization_faults(source: &engine::transport_air_normalization::Source) -> u64 {
    let mut faults = u64::from(source.attack.is_none())
        + u64::from(source.defence.is_none())
        + u64::from(source.speed.is_none())
        + u64::from(source.ace.is_none())
        + u64::from(source.environment.is_none());
    for modifiers in [source.wing, source.country] {
        faults += match modifiers {
            None => 1,
            Some(modifiers) => {
                modifiers
                    .direct
                    .iter()
                    .filter(|word| word.is_none())
                    .count() as u64
                    + modifiers
                        .positive
                        .iter()
                        .filter(|word| word.is_none())
                        .count() as u64
                    + modifiers
                        .effective
                        .iter()
                        .filter(|word| word.is_none())
                        .count() as u64
            }
        };
    }
    if let Some(Some(environment)) = source.environment {
        faults += environment
            .weather
            .iter()
            .chain(&environment.night)
            .filter(|word| word.is_none())
            .count() as u64;
        faults += u64::from(environment.night_factor.is_none())
            + u64::from(environment.target_region.is_none())
            + u64::from(source.night_stat.is_none())
            + u64::from(source.base_region.is_none())
            + u64::from(source.carrier_mission.is_none());
    }
    if let Some(Some(ace)) = source.ace {
        faults += u64::from(ace.assigned_aircraft.is_none())
            + u64::from(ace.owner_effectiveness.is_none());
        for modifiers in [ace.modifiers, ace.type_modifiers] {
            faults += match modifiers {
                None => 1,
                Some(modifiers) => {
                    modifiers
                        .direct
                        .iter()
                        .filter(|word| word.is_none())
                        .count() as u64
                        + modifiers
                            .positive
                            .iter()
                            .filter(|word| word.is_none())
                            .count() as u64
                        + modifiers
                            .effective
                            .iter()
                            .filter(|word| word.is_none())
                            .count() as u64
                }
            };
        }
        faults += u64::from(
            ace.modifiers
                .is_some_and(|modifiers| modifiers.combat.is_none()),
        );
    }
    faults
}

impl Captured {
    pub(crate) fn prepare_normalization(&mut self, entries: usize) {
        if let Some(normalization) = &mut self.normalization {
            let (nodes, current) = normalization.capacity();
            let _ = normalization.try_grow(nodes, current.max(entries));
        }
    }

    pub fn new(countries: usize, regions: usize, missions: usize, factors: usize) -> Self {
        let normalization = (|| {
            let nodes = engine::allocation::add(engine::allocation::mul(missions, 5)?, countries)?;
            engine::transport_air_normalization::Capture::try_new(nodes, missions)
        })()
        .ok();
        let memo_slots =
            memo_slots(missions).and_then(|slots| engine::allocation::filled(slots, 0));
        Self {
            countries: Arena::new(countries),
            regions: Arena::new(regions),
            missions: Arena::new(missions),
            factors: Arena::new(factors),
            normalization,
            memo: Arena::new(missions),
            memo_index: engine::allocation::owned_vec(memo_slots),
            hour: None,
            complete: false,
            refusals: Default::default(),
            reported: [0; REFUSAL_NAMES.len()],
        }
    }
    pub fn reset(&mut self) {
        for row in &mut self.countries.first[..self.countries.len] {
            *row = Default::default();
        }
        for row in &mut self.regions.first[..self.regions.len] {
            *row = Default::default();
        }
        self.countries.reset();
        self.regions.reset();
        self.missions.reset();
        self.factors.reset();
        self.retire_memos();
        self.memo.reset();
        if let Some(normalization) = &mut self.normalization {
            normalization.clear();
        }
        self.hour = None;
        self.complete = false;
        self.refusals = Default::default();
        self.reported.fill(0);
    }
    pub fn set_hour(&mut self, hour: i32) {
        self.hour = Some(hour);
    }
    pub fn denied(&mut self, wings: usize, original: engine::walk::Outcome) {
        let demand = match original {
            engine::walk::Outcome::Rows { len } => len,
            engine::walk::Outcome::Truncated { len, unread_rows } => {
                len.saturating_add(unread_rows as usize)
            }
            engine::walk::Outcome::Unavailable { .. } => 0,
        }
        .max(wings);
        self.missions.required = self.missions.required.max(demand);
        self.memo.required = self.memo.required.max(wings);
        self.refusals.admission = self.refusals.admission.saturating_add(1);
        self.fail();
    }
    pub fn finish(&mut self, original: engine::walk::Outcome) {
        self.retire_memos();
        if let Some(normalization) = &mut self.normalization {
            normalization.retire();
        }
        if matches!(original, engine::walk::Outcome::Unavailable { .. }) {
            self.complete = false;
            self.refusals.domain = self.refusals.domain.saturating_add(1);
        }
        self.report_refusals();
    }
    pub fn fail(&mut self) {
        self.retire_memos();
        if let Some(normalization) = &mut self.normalization {
            normalization.retire();
        }
        self.complete = false;
        self.refusals.domain = self.refusals.domain.saturating_add(1);
        self.report_refusals();
    }
    fn report_refusals(&mut self) {
        for (index, value) in self.refusals.values().into_iter().enumerate() {
            REFUSALS[index].fetch_add(value.saturating_sub(self.reported[index]), Relaxed);
            self.reported[index] = value;
        }
    }
    fn retire_memos(&mut self) {
        self.memo.first[..self.memo.len].fill(Memo::default());
        self.memo.len = 0;
        self.memo_index.fill(0);
    }
    /// Registry hints prepare the same floors as construction, even before
    /// this frame has captured an hour and supplied its own growth demand.
    pub fn prepare(&mut self, countries: usize, regions: usize, missions: usize, factors: usize) {
        crate::table_capture::grow_vec(&mut self.countries.first, countries);
        crate::table_capture::grow_vec(&mut self.regions.first, regions);
        crate::table_capture::grow_vec(&mut self.missions.first, missions);
        crate::table_capture::grow_vec(&mut self.factors.first, factors);
        crate::table_capture::grow_vec(&mut self.memo.first, missions);
        self.grow();
    }
    pub fn grow(&mut self) {
        let mut count = self.countries.first.len();
        self.countries.grow(&mut count);
        let mut count = self.regions.first.len();
        self.regions.grow(&mut count);
        let mut count = self.missions.first.len();
        self.missions.grow(&mut count);
        let mut count = self.factors.first.len();
        self.factors.grow(&mut count);
        let mut count = self.memo.first.len();
        self.memo.grow(&mut count);
        if let Ok(slots) = memo_slots(self.memo.first.len())
            && engine::allocation::resize(&mut self.memo_index, slots, 0).is_ok()
        {
            self.memo_index.fill(0);
        }
        if let Some(normalization) = &mut self.normalization {
            let (mut nodes, mut entries) = normalization.capacity();
            let (node_demand, entry_demand) = normalization.required();
            crate::table_capture::grow_capacity(&mut nodes, node_demand);
            crate::table_capture::grow_capacity(&mut entries, entry_demand);
            // Whole original occurrence demand also bounds the fixed node
            // graph when an earlier arena prevented its first observation.
            if let Ok(floor) = engine::allocation::mul(self.missions.first.len(), 5)
                .and_then(|nodes| engine::allocation::add(nodes, self.countries.first.len()))
            {
                crate::table_capture::grow_capacity(&mut nodes, floor);
            }
            crate::table_capture::grow_capacity(&mut entries, self.missions.first.len());
            let _ = normalization.try_grow(nodes, entries);
        }
    }
    pub fn allocated_bytes(&self) -> usize {
        self.countries.allocated_bytes()
            + self.regions.allocated_bytes()
            + self.missions.allocated_bytes()
            + self.factors.allocated_bytes()
            + self
                .normalization
                .as_ref()
                .map_or(0, |capture| capture.allocated_bytes())
            + self.memo.allocated_bytes()
            + self.memo_index.capacity() * std::mem::size_of::<usize>()
    }
    pub fn observer<'a, M: Memory>(
        &'a mut self,
        memory: &'a M,
        base: usize,
        game: usize,
        regions: Result<&'a [replay::rows::TransportSpottingRegion], &'static str>,
        guard: Result<(), &'static str>,
    ) -> Observer<'a, M> {
        Observer {
            captured: self,
            memory,
            base,
            game,
            regions,
            guard,
            growing: false,
        }
    }
    /// The worker consumes original domains and headers once, growing only
    /// its owned arenas. Existing mission memos survive every resize.
    pub fn observer_growing<'a, M: Memory>(
        &'a mut self,
        memory: &'a M,
        base: usize,
        game: usize,
        regions: Result<&'a [replay::rows::TransportSpottingRegion], &'static str>,
        guard: Result<(), &'static str>,
    ) -> Observer<'a, M> {
        if let Some(normalization) = &mut self.normalization {
            normalization.enable_worker_growth();
        }
        let mut observer = self.observer(memory, base, game, regions, guard);
        observer.growing = true;
        observer
    }
    /// Off-thread, after the original native-memory loan and all pointers retire.
    pub fn packet(&self, hour: i32) -> std::io::Result<Option<TransportAirInputs>> {
        if self.hour != Some(hour) {
            return Ok(None);
        }
        let countries = if !self.complete {
            None
        } else {
            let mut countries = engine::allocation::vec(self.countries.len)?;
            for (slot, country) in self.countries.first[..self.countries.len]
                .iter()
                .enumerate()
            {
                let regions = if let Some((first, count)) = country.regions {
                    let mut regions = engine::allocation::vec(count)?;
                    for (region, row) in self.regions.first[first..first + count].iter().enumerate()
                    {
                        let missions = if let Some((first, count)) = row.missions {
                            let mut missions = engine::allocation::vec(count)?;
                            for original in &self.missions.first[first..first + count] {
                                missions.push(original.map(|value| Ok::<_, std::io::Error>(TransportAirMission {
                                    wing: value.words.wing, current_type: value.words.current_type,
                                    aircraft: value.words.aircraft, disruption: value.words.disruption,
                                    detection_13: value.words.detection_13, detection_14: value.words.detection_14,
                                    normalization: match (value.normalization, self.normalization.as_ref()) {
                                        (Some(source), Some(capture)) => Some(replay::rows::TransportAirNormalization::from_native(source, capture)?),
                                        _ => None,
                                    },
                                })).transpose()?);
                            }
                            Some(missions)
                        } else {
                            None
                        };
                        regions.push(TransportAirRegion {
                            region: region as u32,
                            count: row.count,
                            missions,
                        });
                    }
                    Some(regions)
                } else {
                    None
                };
                let factors = if let Some((first, count)) = country.factors {
                    let mut values = engine::allocation::vec(count)?;
                    values.extend_from_slice(&self.factors.first[first..first + count]);
                    Some(values)
                } else {
                    None
                };
                countries.push(TransportAirCountry {
                    country: country.country,
                    slot: slot as u32,
                    present: country.present,
                    regions,
                    factors,
                });
            }
            Some(countries)
        };
        Ok(Some(TransportAirInputs {
            sample_hour: hour,
            countries,
        }))
    }
}

fn memo_slots(capacity: usize) -> std::io::Result<usize> {
    if capacity == 0 {
        return Ok(0);
    }
    engine::allocation::mul(capacity, 2)?
        .checked_next_power_of_two()
        .ok_or_else(engine::allocation::oom)
}

pub(crate) struct Observer<'a, M: Memory> {
    captured: &'a mut Captured,
    memory: &'a M,
    base: usize,
    game: usize,
    regions: Result<&'a [replay::rows::TransportSpottingRegion], &'static str>,
    guard: Result<(), &'static str>,
    growing: bool,
}

impl<M: Memory> native::TransportObserver for Observer<'_, M> {
    fn slot(&mut self, original: cstrategic_air::SlotObservation) {
        match original {
            cstrategic_air::SlotObservation::Domain(count) => {
                self.captured.complete =
                    reserve(&mut self.captured.countries, count as usize, self.growing).is_some();
                if !self.captured.complete {
                    self.captured.refusals.capacity += 1;
                }
            }
            cstrategic_air::SlotObservation::Slot { slot, root } => {
                if let Some(row) = self
                    .captured
                    .countries
                    .first
                    .get_mut(slot as usize)
                    .filter(|_| (slot as usize) < self.captured.countries.len)
                {
                    row.present = root.is_some();
                    row.country = root.map(|root| root.country);
                }
            }
        }
    }
    fn root(&mut self, slot: u32, pointer: usize, outer: Result<Array, &'static str>) {
        let regions = match outer {
            Ok(outer) => reserve(
                &mut self.captured.regions,
                outer.count as usize,
                self.growing,
            ),
            Err(_) => {
                self.captured.refusals.headers += 1;
                None
            }
        };
        if outer.is_ok() && regions.is_none() {
            self.captured.refusals.capacity += 1;
        }
        let factors = pointer
            .checked_add(0xe0)
            .and_then(|at| Array::read(self.memory, at, 0xa0));
        let factor_span = if let Some(factors) = factors {
            let span = reserve(
                &mut self.captured.factors,
                factors.count as usize,
                self.growing,
            );
            if let Some((first, count)) = span {
                for index in 0..count {
                    let value = factors
                        .element(index as u32, 0xa0)
                        .and_then(|at| at.checked_add(0x80))
                        .and_then(|at| read(self.memory, at));
                    if value.is_none() {
                        self.captured.refusals.leaves += 1;
                    }
                    self.captured.factors.first[first + index] = value;
                }
            } else {
                self.captured.refusals.capacity += 1;
            }
            span
        } else {
            self.captured.refusals.headers += 1;
            None
        };
        if let Some(row) = self
            .captured
            .countries
            .first
            .get_mut(slot as usize)
            .filter(|_| (slot as usize) < self.captured.countries.len)
        {
            row.regions = regions;
            row.factors = factor_span;
        }
    }
    fn region(&mut self, slot: u32, region: u32, original: Result<Array, &'static str>) {
        // Original headers still establish full payload demand when another
        // arena has refused row admission. Never size only the admitted prefix.
        if let Ok(original) = original {
            self.captured.missions.required = self
                .captured
                .missions
                .required
                .saturating_add(original.count as usize);
            self.captured.memo.required = self
                .captured
                .memo
                .required
                .saturating_add(original.count as usize);
        }
        let Some((first, count)) = self.captured.countries.first[..self.captured.countries.len]
            .get(slot as usize)
            .and_then(|row| row.regions)
        else {
            return;
        };
        if region as usize >= count {
            self.captured.refusals.headers += 1;
            return;
        }
        let row = &mut self.captured.regions.first[first + region as usize];
        if row.attempted {
            self.captured.refusals.headers += 1;
            return;
        }
        row.attempted = true;
        match original {
            Ok(original) => {
                row.count = Some(original.count as i32);
                row.missions = reserve_prepared(
                    &mut self.captured.missions,
                    original.count as usize,
                    self.growing,
                );
                if row.missions.is_none() {
                    self.captured.refusals.capacity += 1;
                }
            }
            Err(_) => {
                self.captured.refusals.headers += 1;
            }
        }
    }
    fn mission<N: Memory>(
        &mut self,
        memory: &N,
        countries: &engine::ccountry::Reader<'_, N>,
        wings: &[native::WingIdentity],
        mission: native::MissionObservation<'_>,
    ) {
        let native::MissionObservation {
            slot,
            region,
            ordinal,
            pointer,
            original,
        } = mission;
        let value = self.memo_mission(memory, countries, wings, region, pointer, original);
        if let Ok(value) = value {
            self.captured.refusals.leaves += u64::from(value.words.faults);
            if value
                .words
                .current_type
                .is_some_and(|kind| kind & 0x1f7fa != 0 && kind & 0x4400 == 0)
                && value.words.disruption.is_some_and(|raw| raw != 0)
                && value.words.aircraft.is_some_and(|count| count > 0)
            {
                self.captured.refusals.normalization +=
                    value.normalization.as_ref().map_or(1, normalization_faults);
            }
        } else {
            self.captured.refusals.identity += 1;
        }
        let Some((first, count)) = self.captured.countries.first[..self.captured.countries.len]
            .get(slot as usize)
            .and_then(|row| row.regions)
        else {
            return;
        };
        if region as usize >= count {
            return;
        }
        let Some((first, count)) = self.captured.regions.first[first + region as usize].missions
        else {
            return;
        };
        if (ordinal as usize) < count {
            self.captured.missions.first[first + ordinal as usize] = value.ok();
        }
    }
    fn inactive_region(&mut self, slot: u32, region: u32, original: Result<Array, &'static str>) {
        self.region(slot, region, original);
        if original.is_ok_and(|header| header.count > 0) {
            // Existing native registration writers place every positive list
            // in the active-region vector. Preserve this unsupported reached
            // form explicitly instead of inventing an empty registration.
            self.captured.refusals.registration += 1;
            if let Some((first, count)) = self.captured.countries.first
                [..self.captured.countries.len]
                .get(slot as usize)
                .and_then(|row| row.regions)
                && (region as usize) < count
            {
                self.captured.regions.first[first + region as usize].missions = None;
            }
        }
    }
    fn complete_regions(&self, slot: u32) -> bool {
        self.captured.countries.first[..self.captured.countries.len]
            .get(slot as usize)
            .and_then(|row| row.regions)
            .is_some()
    }
    fn region_attempted(&self, slot: u32, region: u32) -> bool {
        self.captured.countries.first[..self.captured.countries.len]
            .get(slot as usize)
            .and_then(|row| row.regions)
            .and_then(|(first, count)| {
                ((region as usize) < count)
                    .then(|| self.captured.regions.first[first + region as usize].attempted)
            })
            .unwrap_or(false)
    }
    fn retire(&mut self) {
        self.captured.retire_memos();
        if let Some(normalization) = &mut self.captured.normalization {
            normalization.retire();
        }
    }
}

impl<M: Memory> Observer<'_, M> {
    fn memo_mission<N: Memory>(
        &mut self,
        memory: &N,
        countries: &engine::ccountry::Reader<'_, N>,
        wings: &[native::WingIdentity],
        region: u32,
        pointer: usize,
        original: Result<&native::WingIdentity, &'static str>,
    ) -> Result<Mission, &'static str> {
        if self.growing
            && (self.captured.memo.len >= self.captured.memo.first.len()
                || self.captured.memo.len >= self.captured.memo_index.len() / 2)
        {
            let required = self
                .captured
                .memo
                .len
                .checked_add(1)
                .ok_or("transport original air mission memo demand overflow")?;
            engine::walk::Buffer::Growing(&mut self.captured.memo.first).ensure(required)?;
            let slots = memo_slots(self.captured.memo.first.len())
                .map_err(|_| "transport original air mission memo allocation failed")?;
            engine::allocation::resize(&mut self.captured.memo_index, slots, 0)
                .map_err(|_| "transport original air mission memo allocation failed")?;
            self.captured.memo_index.fill(0);
            for index in 0..self.captured.memo.len {
                let memo = &self.captured.memo.first[index];
                let mut slot = (memo.pointer >> 4)
                    .wrapping_mul(0x9e37_79b9)
                    .wrapping_add(memo.region as usize)
                    & (slots - 1);
                while self.captured.memo_index[slot] != 0 {
                    slot = (slot + 1) & (slots - 1);
                }
                self.captured.memo_index[slot] = index + 1;
            }
        }
        if self.captured.memo_index.is_empty() {
            self.captured.refusals.capacity += 1;
            return Err("transport original air mission memo capacity refused");
        }
        let mut slot = (pointer >> 4)
            .wrapping_mul(0x9e37_79b9)
            .wrapping_add(region as usize)
            & (self.captured.memo_index.len() - 1);
        loop {
            let index = self.captured.memo_index[slot];
            if index == 0 {
                break;
            }
            let memo = &self.captured.memo.first[index - 1];
            if memo.pointer == pointer && memo.region == region {
                return memo.value;
            }
            slot = (slot + 1) & (self.captured.memo_index.len() - 1);
        }
        if self.captured.memo.len >= self.captured.memo.first.len()
            || self.captured.memo.len >= self.captured.memo_index.len() / 2
        {
            self.captured.refusals.capacity += 1;
            return Err("transport original air mission memo capacity refused");
        }
        let index = self.captured.memo.len;
        self.captured.memo.first[index] = Memo {
            pointer,
            region,
            ..Default::default()
        };
        self.captured.memo_index[slot] = index + 1;
        self.captured.memo.len += 1;
        let region_context = self.regions.and_then(|regions| {
            regions
                .binary_search_by_key(&region, |row| row.region_id)
                .map(|index| &regions[index])
                .map_err(|_| "transport original sampled air region missing")
        });
        let sampled = region_context.and_then(|region| {
            region
                .sampled_province
                .ok_or("transport original sampled air province refused")
        });
        let environment = region_context.and_then(|region| {
            region
                .air_environment
                .map(Into::into)
                .ok_or("transport original air environment graph refused")
        });
        let mut normalization = None;
        let value = original.and_then(|original| {
            engine::transport_spotting_graph::AirMissionWords::capture_observed(
                memory,
                self.base,
                original,
                |wing, kind, stats, value| {
                    if kind & 0x1f7fa != 0
                        && kind & 0x4400 == 0
                        && value.disruption.is_some_and(|raw| raw != 0)
                        && value.aircraft.is_some_and(|count| count > 0)
                        && let Some(capture) = &mut self.captured.normalization
                    {
                        normalization = Some(capture.capture(
                            memory,
                            self.base,
                            self.game,
                            countries,
                            wings,
                            original,
                            wing,
                            kind,
                            stats,
                            sampled,
                            environment,
                            self.guard,
                        ));
                    }
                },
            )
        });
        let value = value.map(|words| Mission {
            words,
            normalization,
        });
        self.captured.memo.first[index].value = value;
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::cair_regional_mission::TransportObserver;
    use engine::walk::Outcome;

    struct Unavailable;
    impl Memory for Unavailable {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            false
        }
    }

    #[test]
    fn transport_air_original_null_empty_and_failed_headers_remain_distinct() {
        let mut capture = Captured::new(2, 2, 0, 0);
        capture.set_hour(7);
        {
            let mut observer = capture.observer(
                &Unavailable,
                0,
                0,
                Err("original sampled region unavailable"),
                Err("unreached normalization guard"),
            );
            observer.slot(cstrategic_air::SlotObservation::Domain(2));
            observer.slot(cstrategic_air::SlotObservation::Slot {
                slot: 0,
                root: None,
            });
            observer.slot(cstrategic_air::SlotObservation::Slot {
                slot: 1,
                root: Some(cstrategic_air::Root {
                    address: 0x1000,
                    country: 77,
                    canonical_slot: 1,
                }),
            });
            observer.root(
                1,
                0x1000,
                Ok(Array {
                    data: 0x2000,
                    count: 2,
                }),
            );
            observer.region(1, 0, Ok(Array::default()));
            observer.region(1, 1, Err("original region header refused"));
            // A later readable header cannot replace that original failure.
            observer.region(1, 1, Ok(Array::default()));
        }
        capture.finish(Outcome::Rows { len: 0 });
        let packet = capture.packet(7).unwrap().unwrap();
        let countries = packet.countries.unwrap();
        assert!(!countries[0].present);
        assert!(countries[0].regions.is_none());
        let regions = countries[1].regions.as_ref().unwrap();
        assert_eq!(regions[0].count, Some(0));
        assert_eq!(regions[0].missions, Some(vec![]));
        assert_eq!(regions[1].count, None);
        assert_eq!(regions[1].missions, None);
        assert_eq!(countries[1].factors, None);
        assert!(capture.packet(8).unwrap().is_none());
        capture.reset();
        capture.set_hour(8);
        capture.complete = true;
        assert_eq!(capture.packet(8).unwrap().unwrap().countries, Some(vec![]));
    }

    #[test]
    fn transport_air_full_original_capacity_demand_survives_terminal_failure() {
        let mut capture = Captured::new(1, 0, 0, 0);
        capture.set_hour(7);
        capture
            .observer(
                &Unavailable,
                0,
                0,
                Err("unreached region"),
                Err("unreached guard"),
            )
            .slot(cstrategic_air::SlotObservation::Domain(37));
        capture.finish(Outcome::Unavailable {
            reason: "original census terminal failure",
        });
        assert_eq!(capture.countries.required, 37);
        assert!(capture.packet(7).unwrap().unwrap().countries.is_none());
        capture.grow();
        capture.reset();
        capture.set_hour(8);
        capture
            .observer(
                &Unavailable,
                0,
                0,
                Err("unreached region"),
                Err("unreached guard"),
            )
            .slot(cstrategic_air::SlotObservation::Domain(37));
        capture.finish(Outcome::Rows { len: 0 });
        assert_eq!(
            capture.packet(8).unwrap().unwrap().countries.unwrap().len(),
            37
        );
        assert_eq!(capture.refusals.capacity, 0);
    }

    #[test]
    fn transport_air_original_payload_demand_survives_an_unadmitted_country_domain() {
        let mut capture = Captured::new(0, 0, 0, 0);
        capture.set_hour(7);
        {
            let mut observer = capture.observer(
                &Unavailable,
                0,
                0,
                Err("unreached region"),
                Err("unreached guard"),
            );
            observer.slot(cstrategic_air::SlotObservation::Domain(37));
            observer.root(
                1,
                0x1000,
                Ok(Array {
                    data: 0x2000,
                    count: 19,
                }),
            );
            observer.region(
                1,
                7,
                Ok(Array {
                    data: 0x3000,
                    count: 113,
                }),
            );
        }
        capture.finish(Outcome::Unavailable {
            reason: "original census terminal failure",
        });
        assert_eq!(
            (
                capture.countries.required,
                capture.regions.required,
                capture.missions.required,
                capture.memo.required
            ),
            (37, 19, 113, 113)
        );
        capture.grow();
        assert!(capture.countries.first.len() >= 37);
        assert!(capture.regions.first.len() >= 19);
        assert!(capture.missions.first.len() >= 113);
        assert!(capture.memo.first.len() >= 113);
        assert!(capture.memo_index.len() / 2 >= 113);
    }

    #[test]
    fn transport_air_denied_native_admission_keeps_only_original_numerical_demand() {
        let mut capture = Captured::new(0, 0, 0, 0);
        capture.set_hour(7);
        capture.denied(
            3,
            Outcome::Truncated {
                len: 2,
                unread_rows: 7,
            },
        );
        assert_eq!(capture.missions.required, 9);
        assert_eq!(capture.memo.required, 3);
        assert_eq!(capture.refusals.admission, 1);
        assert_eq!(capture.refusals.domain, 1);
        assert_eq!(capture.packet(7).unwrap().unwrap().countries, None);
        assert_eq!(capture.memo.len, 0);
        assert!(capture.memo_index.iter().all(|&index| index == 0));
    }

    #[test]
    fn transport_air_positive_inactive_registration_refuses_its_occurrences_by_name() {
        let mut capture = Captured::new(1, 1, 1, 0);
        capture.set_hour(7);
        {
            let mut observer = capture.observer(
                &Unavailable,
                0,
                0,
                Err("unreached sample"),
                Err("unreached guard"),
            );
            observer.slot(cstrategic_air::SlotObservation::Domain(1));
            observer.slot(cstrategic_air::SlotObservation::Slot {
                slot: 0,
                root: Some(cstrategic_air::Root {
                    address: 0x1000,
                    country: 0,
                    canonical_slot: 0,
                }),
            });
            observer.root(
                0,
                0x1000,
                Ok(Array {
                    data: 0x2000,
                    count: 1,
                }),
            );
            observer.inactive_region(
                0,
                0,
                Ok(Array {
                    data: 0x3000,
                    count: 1,
                }),
            );
        }
        capture.finish(Outcome::Rows { len: 0 });
        let packet = capture.packet(7).unwrap().unwrap();
        let region = &packet.countries.as_ref().unwrap()[0]
            .regions
            .as_ref()
            .unwrap()[0];
        assert_eq!(region.count, Some(1));
        assert_eq!(region.missions, None);
        assert_eq!(capture.refusals.registration, 1);
        assert_eq!(
            REFUSAL_NAMES[6],
            "transport positive inactive air registration is unsupported"
        );
    }

    struct AppearingMission {
        ready: std::cell::Cell<bool>,
        reads: std::cell::Cell<usize>,
        base: usize,
        state: usize,
        wing: usize,
    }
    impl Memory for AppearingMission {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            if address == self.state + engine::cgame_state::COUNTRIES
                || address == self.state + engine::cgame_state::COUNTRY_LINKS
            {
                let count = if address == self.state + engine::cgame_state::COUNTRIES {
                    2_u32
                } else {
                    1
                };
                destination[..8].copy_from_slice(&0x8000_u64.to_le_bytes());
                destination[8..12].copy_from_slice(&count.to_le_bytes());
                destination[12..].copy_from_slice(&count.to_le_bytes());
                return true;
            }
            let mission = self.wing + engine::cair_wing::MISSION;
            if address == mission {
                destination.copy_from_slice(
                    &((self.base + engine::air_mission::VTABLE_RVA) as u64).to_le_bytes(),
                );
                true
            } else if address == mission + engine::air_mission::WING {
                destination.copy_from_slice(&(self.wing as u64).to_le_bytes());
                true
            } else if address == mission + engine::air_mission::CURRENT_TYPE && self.ready.get() {
                destination.copy_from_slice(&0_u32.to_le_bytes());
                true
            } else {
                false
            }
        }
    }

    #[test]
    fn worker_transport_air_grows_all_original_arenas_at_5000_countries() {
        const ROOTS: usize = 0x100_0000;
        struct WorkerMemory {
            original: AppearingMission,
            factors: std::cell::Cell<usize>,
        }
        impl Memory for WorkerMemory {
            fn copy(&self, at: usize, output: &mut [u8]) -> bool {
                if (ROOTS..ROOTS + 5001 * 0x200).contains(&at) && (at - ROOTS) % 0x200 == 0xe0 {
                    self.factors.set(self.factors.get() + 1);
                    output[..8].copy_from_slice(&0x7000_0000_u64.to_le_bytes());
                    output[8..12].copy_from_slice(&2_u32.to_le_bytes());
                    output[12..].copy_from_slice(&2_u32.to_le_bytes());
                    return true;
                }
                if [0x7000_0080, 0x7000_0120].contains(&at) {
                    self.factors.set(self.factors.get() + 1);
                    output.copy_from_slice(&(-7_i64).to_le_bytes());
                    return true;
                }
                self.original.copy(at, output)
            }
        }
        let memory = WorkerMemory {
            original: AppearingMission {
                ready: std::cell::Cell::new(true),
                reads: std::cell::Cell::new(0),
                base: 0x100000000,
                state: 0x1000,
                wing: 0x10000,
            },
            factors: std::cell::Cell::new(0),
        };
        let countries =
            engine::ccountry::Reader::new(&memory, memory.original.base, memory.original.state)
                .unwrap();
        let original = native::WingIdentity {
            mission_pointer: memory.original.wing + engine::cair_wing::MISSION,
            wing: 99,
            ..Default::default()
        };
        let mut capture = Captured::new(0, 0, 0, 0);
        capture.set_hour(7);
        {
            let mut observer = capture.observer_growing(
                &memory,
                memory.original.base,
                memory.original.state,
                Err("unreached region"),
                Err("unreached normalization guard"),
            );
            observer.slot(cstrategic_air::SlotObservation::Domain(5001));
            for slot in 1..=5000 {
                let pointer = ROOTS + slot as usize * 0x200;
                observer.slot(cstrategic_air::SlotObservation::Slot {
                    slot,
                    root: Some(cstrategic_air::Root {
                        address: pointer,
                        country: slot,
                        canonical_slot: slot,
                    }),
                });
                observer.root(
                    slot,
                    pointer,
                    Ok(Array {
                        data: 0x8000,
                        count: 3,
                    }),
                );
                for region in 0..3 {
                    observer.region(
                        slot,
                        region,
                        Ok(Array {
                            data: 0x9000,
                            count: 2,
                        }),
                    );
                    for ordinal in 0..2 {
                        observer.mission(
                            &memory,
                            &countries,
                            std::slice::from_ref(&original),
                            native::MissionObservation {
                                slot,
                                region,
                                ordinal,
                                pointer: original.mission_pointer,
                                original: Ok(&original),
                            },
                        );
                    }
                }
            }
        }
        assert_eq!(memory.factors.get(), 15000);
        capture.finish(Outcome::Rows { len: 30000 });
        assert_eq!(capture.refusals.capacity, 0);
        assert_eq!(
            (
                capture.countries.len,
                capture.regions.len,
                capture.missions.len,
                capture.factors.len
            ),
            (5001, 15000, 30000, 10000)
        );
        let packet = capture.packet(7).unwrap().unwrap();
        let countries = packet.countries.unwrap();
        assert_eq!(countries.len(), 5001);
        for country in &countries[1..] {
            assert_eq!(country.factors, Some(vec![Some(-7); 2]));
            let regions = country.regions.as_ref().unwrap();
            assert_eq!(regions.len(), 3);
            for region in regions {
                let missions = region.missions.as_ref().unwrap();
                assert_eq!(missions.len(), 2);
                assert!(
                    missions
                        .iter()
                        .all(|mission| mission.as_ref().is_some_and(|mission| mission.wing == 99))
                );
            }
        }
    }

    #[test]
    fn worker_transport_air_memo_growth_preserves_original_values_and_identity_failures() {
        let memory = AppearingMission {
            ready: std::cell::Cell::new(false),
            reads: std::cell::Cell::new(0),
            base: 0x100000000,
            state: 0x1000,
            wing: 0x10000,
        };
        let countries = engine::ccountry::Reader::new(&memory, memory.base, memory.state).unwrap();
        let original = native::WingIdentity {
            mission_pointer: memory.wing + engine::cair_wing::MISSION,
            wing: 99,
            ..Default::default()
        };
        let mut capture = Captured::new(0, 0, 0, 0);
        let mut observer = capture.observer_growing(
            &memory,
            memory.base,
            memory.state,
            Err("unreached region"),
            Err("unreached guard"),
        );
        let first = observer
            .memo_mission(
                &memory,
                &countries,
                &[original],
                0,
                original.mission_pointer,
                Ok(&original),
            )
            .unwrap();
        for region in 1..=5000 {
            assert_eq!(
                observer
                    .memo_mission(
                        &memory,
                        &countries,
                        &[original],
                        region,
                        original.mission_pointer,
                        Err("original identity failed")
                    )
                    .unwrap_err(),
                "original identity failed"
            );
        }
        let reads = memory.reads.get();
        memory.ready.set(true);
        assert_eq!(
            observer
                .memo_mission(
                    &memory,
                    &countries,
                    &[original],
                    0,
                    original.mission_pointer,
                    Ok(&original)
                )
                .unwrap()
                .words,
            first.words
        );
        assert_eq!(
            observer
                .memo_mission(
                    &memory,
                    &countries,
                    &[original],
                    1,
                    original.mission_pointer,
                    Ok(&original)
                )
                .unwrap_err(),
            "original identity failed"
        );
        assert_eq!(memory.reads.get(), reads);
        assert_eq!(observer.captured.memo.len, 5001);
        assert_eq!(observer.captured.refusals.capacity, 0);
    }

    #[test]
    fn transport_air_repeated_mission_keeps_original_field_and_identity_failure_until_retirement() {
        let memory = AppearingMission {
            ready: std::cell::Cell::new(false),
            reads: std::cell::Cell::new(0),
            base: 0x100000000,
            state: 0x1000,
            wing: 0x10000,
        };
        let countries = engine::ccountry::Reader::new(&memory, memory.base, memory.state).unwrap();
        let original = native::WingIdentity {
            mission_pointer: memory.wing + engine::cair_wing::MISSION,
            wing: 99,
            ..Default::default()
        };
        let mut capture = Captured::new(0, 0, 3, 0);
        capture.set_hour(7);
        {
            let mut observer = capture.observer(
                &memory,
                memory.base,
                memory.state,
                Err("unreached sample"),
                Err("unreached guard"),
            );
            let first = observer
                .memo_mission(
                    &memory,
                    &countries,
                    std::slice::from_ref(&original),
                    3,
                    original.mission_pointer,
                    Ok(&original),
                )
                .unwrap();
            assert_eq!(first.words.current_type, None);
            let after_first = memory.reads.get();
            memory.ready.set(true);
            let repeated = observer
                .memo_mission(
                    &memory,
                    &countries,
                    std::slice::from_ref(&original),
                    3,
                    original.mission_pointer,
                    Ok(&original),
                )
                .unwrap();
            assert_eq!(repeated.words, first.words);
            assert_eq!(
                memory.reads.get(),
                after_first,
                "original failed word is never reread"
            );
            assert!(
                observer
                    .memo_mission(
                        &memory,
                        &countries,
                        std::slice::from_ref(&original),
                        4,
                        original.mission_pointer,
                        Err("original wing census identity failed")
                    )
                    .is_err()
            );
            let identity_reads = memory.reads.get();
            let repeated = observer.memo_mission(
                &memory,
                &countries,
                std::slice::from_ref(&original),
                4,
                original.mission_pointer,
                Ok(&original),
            );
            assert_eq!(
                repeated.unwrap_err(),
                "original wing census identity failed"
            );
            assert_eq!(
                memory.reads.get(),
                identity_reads,
                "original failed identity is never recertified"
            );
        }
        capture.finish(Outcome::Rows { len: 0 });
        assert_eq!(capture.memo.len, 0);
        assert!(capture.memo_index.iter().all(|&index| index == 0));
        capture.reset();
        capture.set_hour(8);
        let current = capture
            .observer(
                &memory,
                memory.base,
                memory.state,
                Err("unreached sample"),
                Err("unreached guard"),
            )
            .memo_mission(
                &memory,
                &countries,
                std::slice::from_ref(&original),
                3,
                original.mission_pointer,
                Ok(&original),
            )
            .unwrap();
        assert_eq!(
            current.words.current_type,
            Some(0),
            "the next original pass owns a new observation"
        );
        capture.finish(Outcome::Rows { len: 0 });
    }
}
