use super::*;
use engine_allocation as allocation;

impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let mut failure = None;
        let mut raise = |index: usize, result: Result<usize, &'static str>| match result {
            Ok(count) => self.rows[index] = self.rows[index].max(count),
            Err(reason) => {
                failure.get_or_insert(reason);
            }
        };
        raise(
            1,
            collector_extended_support::demand::rows(cair_base::count(memory, base, state)),
        );
        let aces = cace::count(memory, base, state);
        raise(3, collector_extended_support::demand::rows(aces.outcome));
        self.ace_names = self.ace_names.max(aces.required_bytes);
        raise(
            4,
            collector_extended_support::demand::rows(cloop_history::count(memory, base, state)),
        );
        if let Err(reason) = self.presize_missions(memory, base, state) {
            failure.get_or_insert(reason);
        }
        if let Err(reason) = self.sorties.presize(memory, base, state) {
            failure.get_or_insert(reason);
        }
        failure.map_or(Ok(()), Err)
    }
}

fn reserve_to<T>(rows: &mut Vec<T>, len: usize) -> std::io::Result<()> {
    let additional = len.saturating_sub(rows.len());
    allocation::reserve(rows, additional)
}

pub const AIR_BASES_N: usize = 3;
const BASES_MASK: u32 = 0x01;
pub(super) const AIR_REGISTRATIONS_MASK: u32 = 0x02;
const AIR_ACES_MASK: u32 = 0x04;
const AIR_BASES_CENSUS_MASK: u32 = 0x07;
const BASE_VECTORS_MASK: u32 = 0x03;

const REG_MIRROR_BATCH: usize = 64;

#[derive(Debug, Default)]
pub struct AirBasesJob {
    pub log: RawLog<AIR_BASES_N>,
    pub carrier: carrier::Job,
    pub state_fuel: super::state_fuel_inputs::Packet,
    pub game_state: usize,
    pub registration_demand: Option<[usize; 2]>,
    pub(super) aces_active: bool,
    pub(super) census: Vec<(usize, u32)>,
    pub roster: ccountry::RosterSnapshot,
    pub(super) seen_links: Vec<(u32, u32)>,
    pub(super) seen_link_index: Vec<usize>,
    pub(super) required_census: usize,
    pub(super) required_links: usize,
    pub(super) capacity_charge: collector_primitives::raw_memory::Charge,
    pub slot_run: crate::air_base_changes::SlotRun,
    pub slot_report: crate::air_base_changes::SlotReport,
    pub carry: CarryRun,
    pub registration_store: Option<Box<super::RegistrationStore>>,
    pub(super) reached: Reached,
    pub segments: Vec<HeldSegment>,
    pub registration_work: collector_observers::air_base_carry_changes::Work,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeldSegment {
    pub handle: u64,
    pub pointer: usize,
    pub id: u64,
}

#[derive(Clone, Copy)]
pub struct CarryRun {
    pub changes: &'static dyn collector_observers::air_base_carry_changes::Changes,
}

impl Default for CarryRun {
    fn default() -> Self {
        Self {
            changes: &collector_observers::air_base_carry_changes::Observer,
        }
    }
}

impl std::fmt::Debug for CarryRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CarryRun").finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
pub(super) struct Reached {
    links: Vec<(u32, u32)>,
    containers: Vec<usize>,
    pools: Vec<usize>,
    wings: Vec<usize>,
    lost: bool,
}

impl Reached {
    fn clear(&mut self) {
        self.links.clear();
        self.containers.clear();
        self.pools.clear();
        self.wings.clear();
        self.lost = false;
    }

    fn link(&mut self, country: u32, slot: u32) {
        if allocation::advisory(|| allocation::reserve(&mut self.links, 1)).is_ok() {
            self.links.push((country, slot));
        } else {
            self.lost = true;
        }
    }

    fn pool(&mut self, pool: usize) {
        if allocation::advisory(|| allocation::reserve(&mut self.pools, 1)).is_ok() {
            self.pools.push(pool);
        } else {
            self.lost = true;
        }
    }

    fn container(&mut self, container: usize) {
        if allocation::advisory(|| allocation::reserve(&mut self.containers, 1)).is_ok() {
            self.containers.push(container);
        } else {
            self.lost = true;
        }
    }

    fn wing(&mut self, wing: usize) {
        if allocation::advisory(|| allocation::reserve(&mut self.wings, 1)).is_ok() {
            self.wings.push(wing);
        } else {
            self.lost = true;
        }
    }
}

impl AirBasesJob {
    pub fn new() -> Self {
        let mut job = Self {
            carrier: carrier::Job::new(),
            ..Self::default()
        };
        job.presize_countries();
        job
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.carrier.clear();
        self.state_fuel.clear();
        self.game_state = 0;
        self.registration_demand = None;
        self.aces_active = false;
        self.census.clear();
        self.roster.clear();
        self.seen_links.clear();
        self.seen_link_index.fill(usize::MAX);
        self.required_census = 0;
        self.required_links = 0;
        self.slot_report = Default::default();
        self.segments.clear();
        self.registration_work = Default::default();
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
            || self.required_census > self.census.capacity()
            || self.required_links > self.seen_links.capacity()
            || self.roster.required() > self.roster.capacity()
            || self.carrier.grow_for_next()
    }

    pub fn grow(&mut self) {
        engine_allocation::advisory(|| self.grow_hints());
    }

    fn grow_hints(&mut self) {
        fn grow_to<T>(rows: &mut Vec<T>, required: usize) {
            if required > rows.capacity() {
                let _ = allocation::reserve_exact(rows, required - rows.len());
            }
        }
        if self.log.grow_for_next()
            || self.required_census > self.census.capacity()
            || self.required_links > self.seen_links.capacity()
        {
            self.log.grow();
            grow_to(&mut self.census, self.required_census);
            grow_to(&mut self.seen_links, self.required_links);
        }
        let _ = self.roster.reserve(self.roster.required());
        self.carrier.grow();
        let capacity = self
            .required_links
            .max(collector_primitives::country_slots::census_rows(0));
        if reserve_to(&mut self.seen_link_index, capacity).is_err() {
            return;
        }
        self.seen_link_index.resize(capacity, usize::MAX);
        self.seen_link_index.fill(usize::MAX);
        self.update_capacity_charge();
    }

    pub fn presize_countries(&mut self) {
        collector_primitives::country_slots::reserve_census(&mut self.census);
        if collector_primitives::country_slots::reserve_census(&mut self.seen_links) {
            let capacity = self
                .required_links
                .max(collector_primitives::country_slots::census_rows(0));
            if reserve_to(&mut self.seen_link_index, capacity).is_err() {
                return;
            }
            self.seen_link_index.resize(capacity, usize::MAX);
            self.seen_link_index.fill(usize::MAX);
        }
        self.carrier.presize(crate::sizing::carrier_demand());
        self.update_capacity_charge();
    }

    pub(super) fn update_capacity_charge(&mut self) {
        use collector_primitives::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            auxiliary_vectors: vector_bytes(&self.census)
                + self.roster.allocated_bytes() as u64
                + vector_bytes(&self.seen_links)
                + vector_bytes(&self.seen_link_index)
                + self.carrier.bytes()
                + self.state_fuel.bytes(),
            ..Default::default()
        });
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AirBasesMemoContext {
    pub owner: usize,
    pub base: usize,
    pub seq: u64,
    pub hour: i32,
    pub force_full: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AirBasesMemoKey {
    pub(super) owner: usize,
    pub(super) base: usize,
    pub(super) game_state: usize,
    pub(super) capacities: [usize; 5],
    pub(super) carrier_capacities: [usize; 10],
    pub(super) carrier_seed: Result<(), &'static str>,
    pub(super) aces_active: bool,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AirBasesMemoReport {
    pub hit: bool,
    pub eligible: bool,
    pub compared: bool,
    pub installed: bool,
    pub compare_ticks: u64,
    pub restore_ticks: u64,
    pub install_ticks: u64,
}

static AIR_BASES_MEMO_MEMORY: collector_primitives::raw_memory::Tracker =
    collector_primitives::raw_memory::Tracker::new();

#[derive(Debug)]
pub struct AirBasesMemo {
    pub(super) key: Option<AirBasesMemoKey>,
    pub(super) previous: Option<(u64, i32)>,
    pub(super) spans: Vec<collector_raw::raw::Span>,
    pub(super) bytes: Vec<u8>,
    pub(super) has_bulk: bool,
    pub(super) bases: Vec<cair_base::Row>,
    pub(super) aces: Vec<cace::Row>,
    pub(super) names: Vec<u8>,
    pub(super) required_names: usize,
    pub(super) registrations: super::fog::RegistrationsMemo,
    pub(super) carrier: carrier::Memo,
    pub(super) decoded: super::DecodedRegistrations,
    pub(super) roster: Vec<Option<ccountry::Country>>,
    pub(super) roster_context: Option<([Array; 2], Option<ccountry::ReservedCountryObservation>)>,
    pub(super) capacity_charge: collector_primitives::raw_memory::Charge,
}

impl Default for AirBasesMemo {
    fn default() -> Self {
        Self::on_tracker(&AIR_BASES_MEMO_MEMORY)
    }
}

impl AirBasesMemo {
    pub(super) fn on_tracker(tracker: &'static collector_primitives::raw_memory::Tracker) -> Self {
        Self {
            key: None,
            previous: None,
            spans: Vec::new(),
            bytes: Vec::new(),
            has_bulk: false,
            bases: Vec::new(),
            aces: Vec::new(),
            names: Vec::new(),
            required_names: 0,
            registrations: super::fog::RegistrationsMemo::default(),
            carrier: carrier::Memo::default(),
            decoded: super::DecodedRegistrations::default(),
            roster: Vec::new(),
            roster_context: None,
            capacity_charge: collector_primitives::raw_memory::Charge::empty_on(tracker),
        }
    }

    pub fn invalidate(&mut self) {
        self.key = None;
        self.previous = None;
    }

    pub(super) fn capacity(&self) -> collector_primitives::raw_memory::Capacity {
        use collector_primitives::raw_memory::{Capacity, vector_bytes};
        Capacity {
            payload_vectors: vector_bytes(&self.spans)
                + vector_bytes(&self.bytes)
                + self.carrier.bytes().0,
            auxiliary_vectors: vector_bytes(&self.bases)
                + vector_bytes(&self.aces)
                + vector_bytes(&self.names)
                + self.registrations.capacity_bytes()
                + self.carrier.bytes().1
                + self.decoded.capacity_bytes()
                + vector_bytes(&self.roster),
            ..Capacity::default()
        }
    }

    fn roster_equal(&self, job: &AirBasesJob, base: usize) -> bool {
        let reserved = job.roster.reserved_observation(base, job.game_state);
        match (job.roster.roster(base, job.game_state), self.roster_context) {
            (None, None) => true,
            (Some(roster), Some((arrays, held_reserved))) => {
                arrays == roster.arrays()
                    && held_reserved == reserved
                    && self.roster.as_slice() == roster.slots()
            }
            _ => false,
        }
    }

    fn save_roster(&mut self, job: &AirBasesJob, base: usize) -> bool {
        self.roster.clear();
        self.roster_context = None;
        let Some(roster) = job.roster.roster(base, job.game_state) else {
            return true;
        };
        if reserve_to(&mut self.roster, roster.slots().len()).is_err() {
            return false;
        }
        self.roster.extend_from_slice(roster.slots());
        self.roster_context = Some((
            roster.arrays(),
            job.roster.reserved_observation(base, job.game_state),
        ));
        true
    }

    pub(super) fn input_equal(&self, log: &RawLog<AIR_BASES_N>) -> bool {
        let aces_active = self.key.is_none_or(|key| key.aces_active);
        !log.overflowed
            && !log
                .fault
                .iter()
                .enumerate()
                .any(|(slot, fault)| *fault && (slot < 2 || aces_active))
            && self.has_bulk == log.has_bulk
            && self.spans == log.spans
            && self.bytes.as_slice() == log.bytes.as_ref()
    }

    pub fn decode(
        &mut self,
        air: &mut Captured,
        fog: &mut super::fog::Captured,
        job: &AirBasesJob,
        ctx: AirBasesMemoContext,
        decode_ticks: &mut [u64; AIR_BASES_N],
    ) -> AirBasesMemoReport {
        let prepared = fog.prepare_decode(job, &mut air.carrier);
        self.decode_prepared(air, fog, job, ctx, decode_ticks, prepared)
    }

    #[allow(clippy::too_many_arguments, reason = "one decode's borrowed parts")]
    pub(super) fn decode_prepared(
        &mut self,
        air: &mut Captured,
        fog: &mut super::fog::Captured,
        job: &AirBasesJob,
        ctx: AirBasesMemoContext,
        decode_ticks: &mut [u64; AIR_BASES_N],
        prepared: Result<(), &'static str>,
    ) -> AirBasesMemoReport {
        let mut report = AirBasesMemoReport::default();
        air.active[3] = job.aces_active;
        let compared_started = qpc();
        let adjacent = self.previous.is_some_and(|(seq, hour)| {
            seq.checked_add(1) == Some(ctx.seq) && hour.checked_add(1) == Some(ctx.hour)
        });
        if ctx.force_full || !adjacent {
            self.key = None;
        }
        let capacity = air
            .bases_memo_context()
            .zip(fog.registrations_memo_context());
        let carrier_capacities =
            carrier_capacities(&air.carrier, &fog.carrier, &fog.carrier_scratch);
        let key = capacity.map(|(air, fog)| AirBasesMemoKey {
            owner: ctx.owner,
            base: ctx.base,
            game_state: job.game_state,
            capacities: [air[0], air[1], air[2], fog[0], fog[1]],
            carrier_capacities,
            carrier_seed: job.carrier.seed,
            aces_active: job.aces_active,
        });
        report.eligible = prepared.is_ok()
            && key.is_some()
            && job.carrier.seed.is_ok()
            && !job.carrier.log.borrow().overflowed
            && !job.log.overflowed
            && !job
                .log
                .fault
                .iter()
                .enumerate()
                .any(|(slot, fault)| *fault && (slot < 2 || job.aces_active))
            && job.log.spans.iter().all(|span| span.ok);
        if report.eligible && self.key == key {
            report.compared = true;
            report.hit = self.input_equal(&job.log)
                && self.carrier.input_equal(&job.carrier.log.borrow())
                && self.roster_equal(job, ctx.base);
        }
        report.compare_ticks = elapsed(compared_started);
        decode_ticks[1] += report.compare_ticks;
        if report.hit {
            let restore_before = decode_ticks.iter().sum::<u64>();
            let started = qpc();
            air.bases.rows[..self.bases.len()].copy_from_slice(&self.bases);
            air.bases.outcome = Outcome::Rows {
                len: self.bases.len(),
            };
            decode_ticks[0] += elapsed(started);
            if job.aces_active {
                let started = qpc();
                air.aces.rows[..self.aces.len()].copy_from_slice(&self.aces);
                air.aces.outcome = Outcome::Rows {
                    len: self.aces.len(),
                };
                air.names.first[..self.names.len()].copy_from_slice(&self.names);
                air.names.len = self.names.len();
                air.names.required = self.required_names;
                decode_ticks[2] += elapsed(started);
            } else {
                air.aces.outcome = SKIPPED;
                air.names.reset();
            }
            air.bases_raw_mode = false;
            let started = qpc();
            fog.restore_registrations_memo(&self.registrations);
            self.carrier.restore(&mut air.carrier, &mut fog.carrier);
            if let Err(reason) = fog.carrier.install_states(&job.state_fuel) {
                fog.fail_registrations_capture(reason);
            }
            decode_ticks[1] += elapsed(started);
            report.restore_ticks = decode_ticks
                .iter()
                .sum::<u64>()
                .saturating_sub(restore_before);
            self.previous = Some((ctx.seq, ctx.hour));
            return report;
        }

        self.key = None;
        air.decode_air_bases_payload(ctx.base, job, decode_ticks);
        let before = self.decoded.counts();
        let reuse = self.decoded.begin(&job.segments);
        let carried = fog.decode_registrations_prepared(
            ctx.base,
            job,
            &air.bases.rows[..air.bases.outcome.len()],
            &mut air.carrier,
            decode_ticks,
            reuse.then_some(&mut self.decoded as &mut dyn cair_base::RegistrationReuse),
            prepared,
        );
        if let Err(reason) = carried {
            air.bases.outcome = Outcome::Unavailable { reason };
        }
        self.decoded.finish();
        let after = self.decoded.counts();
        collector_observers::air_base_carry_changes::record_decoded(
            after.reused - before.reused,
            after.walked - before.walked,
        );
        let install_started = qpc();
        if report.eligible && air.bases_memo_complete() && fog.registrations_memo_complete() {
            if reserve_to(&mut self.spans, job.log.spans.len()).is_err()
                || reserve_to(&mut self.bytes, job.log.bytes.len()).is_err()
                || reserve_to(&mut self.bases, air.bases.outcome.len()).is_err()
                || (job.aces_active
                    && (reserve_to(&mut self.aces, air.aces.outcome.len()).is_err()
                        || reserve_to(&mut self.names, air.names.len).is_err()))
            {
                self.invalidate();
                self.capacity_charge.set(self.capacity());
                return report;
            }
            self.spans.clear();
            self.spans.extend_from_slice(&job.log.spans);
            self.bytes.clear();
            self.bytes.extend_from_slice(job.log.bytes.as_ref());
            self.has_bulk = job.log.has_bulk;
            self.bases.clear();
            self.bases
                .extend_from_slice(&air.bases.rows[..air.bases.outcome.len()]);
            self.aces.clear();
            self.names.clear();
            self.required_names = 0;
            if job.aces_active {
                self.aces
                    .extend_from_slice(&air.aces.rows[..air.aces.outcome.len()]);
                self.names
                    .extend_from_slice(&air.names.first[..air.names.len]);
                self.required_names = air.names.required;
            }
            fog.save_registrations_memo(&mut self.registrations);
            if !self.save_roster(job, ctx.base) {
                self.invalidate();
                self.capacity_charge.set(self.capacity());
                return report;
            }
            if !self.carrier.save(
                &air.carrier,
                &fog.carrier,
                &job.carrier.log.borrow(),
                job.carrier.demand,
            ) {
                self.invalidate();
                self.capacity_charge.set(self.capacity());
                return report;
            }
            self.capacity_charge.set(self.capacity());
            self.key = key;
            self.previous = Some((ctx.seq, ctx.hour));
            report.installed = true;
        } else {
            self.invalidate();
        }
        report.install_ticks = elapsed(install_started);
        decode_ticks[1] += report.install_ticks;
        report
    }
}

impl Captured {
    fn bases_memo_context(&self) -> Option<[usize; 3]> {
        self.bases_raw_mode.then_some([
            self.bases.rows.len(),
            if self.active[3] {
                self.aces.rows.len()
            } else {
                0
            },
            if self.active[3] {
                self.names.first.len()
            } else {
                0
            },
        ])
    }

    fn bases_memo_complete(&self) -> bool {
        matches!(self.bases.outcome, Outcome::Rows { len } if len <= self.bases.rows.len())
            && (!self.active[3]
                || (matches!(self.aces.outcome, Outcome::Rows { .. })
                    && self.names.required >= self.names.len
                    && valid_names(
                        self.aces.outcome,
                        &self.aces.rows,
                        &self.names.first,
                        self.names.len,
                    )))
    }
}

fn carrier_capacities(
    bases: &carrier::Bases,
    registrations: &carrier::Registrations,
    scratch: &carrier::Scratch,
) -> [usize; 10] {
    let bases = bases.capacity();
    let registrations = registrations.capacity();
    let scratch = scratch.capacity();
    [
        bases[0],
        bases[1],
        bases[2],
        registrations[0],
        registrations[1],
        scratch[0],
        scratch[1],
        scratch[2],
        scratch[3],
        scratch[4],
    ]
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AirBasesCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; AIR_BASES_N],
}

#[derive(Debug, Clone, Copy)]
pub struct AirBasesCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
}

fn bases_pool_u64(log: &RawLog<AIR_BASES_N>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

pub(super) fn bases_pool_u32(log: &RawLog<AIR_BASES_N>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

fn base_identity(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine_native::Memory,
    base: usize,
    pointer: usize,
) -> (bool, u64) {
    let identity = bspan(log, memory, Some(pointer), 16, BASE_VECTORS_MASK);
    match (identity, engine_sites::at(base, cair_base::VTABLE_RVA)) {
        (Some(start), Some(expected)) => {
            let handle = bases_pool_u64(log, start + 8);
            (
                bases_pool_u64(log, start) == expected as u64 && handle != 0,
                handle,
            )
        }
        _ => (false, 0),
    }
}

pub(super) fn bspan(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine_native::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

fn bases_link(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine_native::Memory,
    links: &Array,
    countries: u32,
    raw: u32,
    mask: u32,
) -> Option<u32> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = bspan(log, memory, links.data.checked_add(offset), 4, mask)?;
    let slot = bases_pool_u32(log, start);
    if slot == 0 || slot >= countries {
        return None;
    }
    Some(slot)
}

#[allow(clippy::too_many_arguments)]
fn copy_air_bases_census(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine_native::Memory,
    base: usize,
    game_state: usize,
    census: &mut Vec<(usize, u32)>,
    roster: &mut ccountry::RosterSnapshot,
    required: [&mut usize; 2],
    mask: u32,
) -> Option<(Array, u32)> {
    let [required_census, required_links] = required;
    roster.clear();
    if let Some(authenticated) = memory.authenticated_countries(base, game_state)
        && authenticated.slots().len() == authenticated.arrays()[0].count as usize
    {
        let [countries, links] = authenticated.arrays();
        if countries.count < 2 || links.count == 0 {
            return None;
        }
        *required_census = countries.count as usize - 1;
        *required_links = links.count as usize;
        if reserve_to(census, *required_census).is_err()
            || roster.reserve(authenticated.slots().len()).is_err()
        {
            return None;
        }
        if roster.capture(authenticated, base, game_state) {
            if let Some(root) = memory.authenticated_reserved_country_observation(base, game_state)
            {
                roster.capture_reserved_observation(base, game_state, root);
            }
            for (slot, country) in authenticated.slots().iter().copied().enumerate().skip(1) {
                let Some(country) =
                    country.filter(|country| country.canonical_slot as usize == slot)
                else {
                    continue;
                };
                if census.len() == census.capacity() {
                    log.overflowed = true;
                    return None;
                }
                census.push((country.pointer, slot as u32));
            }
            return Some((links, countries.count));
        }
    }
    let countries_start = bspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRIES),
        16,
        mask,
    )?;
    let links_start = bspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRY_LINKS),
        16,
        mask,
    )?;
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[countries_start..countries_start + 16]);
    let countries = Array::from_header(raw, 8)?;
    raw.copy_from_slice(&log.bytes[links_start..links_start + 16]);
    let links = Array::from_header(raw, 4)?;
    if countries.count < 2 || links.count == 0 {
        return None;
    }
    *required_census = countries.count as usize - 1;
    *required_links = links.count as usize;
    if reserve_to(census, *required_census).is_err() {
        return None;
    }
    let vtable = engine_sites::at(base, ccountry::VTABLE_RVA)? as u64;
    let mut reserved = false;
    for slot in 1..countries.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|offset| countries.data.checked_add(offset));
        let Some(pointer_start) = bspan(log, memory, pointer_addr, 8, mask) else {
            continue;
        };
        let pointer = match usize::try_from(bases_pool_u64(log, pointer_start))
            .ok()
            .filter(|&pointer| pointer != 0)
        {
            Some(pointer) => pointer,
            None => continue,
        };
        let vtable_start = bspan(log, memory, Some(pointer), 8, mask);
        let id_start = bspan(log, memory, pointer.checked_add(ccountry::ID), 4, mask);
        let (Some(vtable_start), Some(id_start)) = (vtable_start, id_start) else {
            continue;
        };
        if bases_pool_u64(log, vtable_start) != vtable {
            continue;
        }
        let id = bases_pool_u32(log, id_start);
        if id == 0 {
            bases_link(log, memory, &links, countries.count, slot, mask);
            if !reserved {
                reserved = true;
                stage_reserved_country(log, memory, &countries, vtable, mask);
            }
            continue;
        }
        let canonical = bases_link(log, memory, &links, countries.count, id, mask);
        let slot_link = bases_link(log, memory, &links, countries.count, slot, mask);
        let (Some(canonical), Some(slot_link)) = (canonical, slot_link) else {
            continue;
        };
        if slot_link != canonical {
            continue;
        }
        let check = bases_link(log, memory, &links, countries.count, canonical, mask);
        if check != Some(canonical) {
            continue;
        }
        let canonical_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|offset| countries.data.checked_add(offset));
        let canonical_pointer = bspan(log, memory, canonical_addr, 8, mask);
        let Some(canonical_pointer) = canonical_pointer else {
            continue;
        };
        if bases_pool_u64(log, canonical_pointer) != pointer as u64 {
            continue;
        }
        if slot != canonical {
            continue;
        }
        if census.len() == census.capacity() {
            log.overflowed = true;
            return None;
        }
        census.push((pointer, slot));
    }
    Some((links, countries.count))
}

fn stage_reserved_country(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine_native::Memory,
    countries: &Array,
    vtable: u64,
    mask: u32,
) {
    let Some(start) = bspan(log, memory, countries.element(0, 8), 8, mask) else {
        return;
    };
    let Some(root) = usize::try_from(bases_pool_u64(log, start))
        .ok()
        .filter(|&root| root != 0)
    else {
        return;
    };
    let Some(start) = bspan(log, memory, Some(root), 8, mask) else {
        return;
    };
    if bases_pool_u64(log, start) == vtable {
        bspan(log, memory, root.checked_add(ccountry::ID), 4, mask);
    }
}

pub(super) fn resolve_link(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine_native::Memory,
    links: &Array,
    seen: &mut Vec<(u32, u32)>,
    index: &mut [usize],
    countries: u32,
    raw: u32,
) -> Option<u32> {
    let bucket = (!index.is_empty()).then(|| raw as usize % index.len());
    if let Some(bucket) = bucket
        && let Some(&(key, slot)) = seen.get(index[bucket])
        && key == raw
    {
        return Some(slot);
    }
    if bucket.is_none_or(|bucket| index[bucket] != usize::MAX)
        && let Some(position) = seen.iter().position(|(key, _)| *key == raw)
    {
        if let Some(bucket) = bucket {
            index[bucket] = position;
        }
        return Some(seen[position].1);
    }
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = bspan(
        log,
        memory,
        links.data.checked_add(offset),
        4,
        AIR_REGISTRATIONS_MASK,
    )?;
    let slot = bases_pool_u32(log, start);
    if slot >= countries {
        return None;
    }
    if seen.len() == seen.capacity() && allocation::reserve(seen, 1).is_err() {
        return None;
    }
    let position = seen.len();
    seen.push((raw, slot));
    if let Some(bucket) = bucket {
        index[bucket] = position;
    }
    Some(slot)
}

fn block_word(log: &RawLog<AIR_BASES_N>, start: usize, index: usize) -> Option<usize> {
    let at = start.checked_add(index.checked_mul(8)?)?;
    let bytes = log.bytes.get(at..at.checked_add(8)?)?;
    usize::try_from(u64::from_le_bytes(bytes.try_into().ok()?)).ok()
}

fn copy_air_base_row(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine_native::Memory,
    base: usize,
    pointer: usize,
    state_slot: Option<u32>,
    handle: u64,
) -> Option<cair_base::Row> {
    let host = bspan(
        log,
        memory,
        pointer.checked_add(cair_base::HOST_REFERENCE),
        32,
        BASES_MASK,
    )?;
    let state_pointer = usize::try_from(bases_pool_u64(log, host + 8)).ok()?;
    let province_pointer = usize::try_from(bases_pool_u64(log, host + 16)).ok()?;
    let state = if state_pointer == 0 {
        None
    } else {
        let vtable = bspan(log, memory, Some(state_pointer), 8, BASES_MASK)?;
        let expected_state = match engine_sites::at(base, cstate::VTABLE_RVA) {
            Some(expected) => expected as u64,
            None => return None,
        };
        if bases_pool_u64(log, vtable) != expected_state {
            return None;
        }
        let id = match bspan(
            log,
            memory,
            state_pointer.checked_add(cstate::ID),
            4,
            BASES_MASK,
        ) {
            Some(start) => bases_pool_u32(log, start),
            None => return None,
        };
        Some(id)
    };
    if state != state_slot {
        return None;
    }
    let host_ref = bases_pool_u64(log, host);
    if state.is_none() && host_ref == 0 {
        return None;
    }
    let province = if province_pointer != 0 {
        let vtable = bspan(log, memory, Some(province_pointer), 8, BASES_MASK)?;
        let expected_province = match engine_sites::at(base, cprovince::VTABLE_RVA) {
            Some(expected) => expected as u64,
            None => return None,
        };
        if bases_pool_u64(log, vtable) != expected_province {
            return None;
        }
        let owner = bspan(
            log,
            memory,
            province_pointer.checked_add(cprovince::STATE),
            8,
            BASES_MASK,
        )?;
        if state_pointer != 0 && bases_pool_u64(log, owner) != state_pointer as u64 {
            return None;
        }
        let id = bspan(
            log,
            memory,
            province_pointer.checked_add(cprovince::ID),
            4,
            BASES_MASK,
        )?;
        Some(bases_pool_u32(log, id))
    } else {
        None
    };
    Some(cair_base::Row {
        id: handle,
        state,
        province,
        host: host_ref,
        capacity: i32::from_le_bytes(log.bytes[host + 24..host + 28].try_into().unwrap()),
        kind: i32::from_le_bytes(log.bytes[host + 28..host + 32].try_into().unwrap()),
    })
}

fn copy_air_aces_country(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine_native::Memory,
    base: usize,
    pointer: usize,
    slot: u32,
) -> bool {
    let header = match bspan(
        log,
        memory,
        pointer.checked_add(ccountry::ACES),
        16,
        AIR_ACES_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header..header + 16]);
    let roster = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return false,
    };
    for ordinal in 0..roster.count {
        if !copy_air_ace(log, memory, base, pointer, slot, &roster, ordinal) {
            return false;
        }
        if log.overflowed {
            return false;
        }
    }
    true
}

fn copy_air_ace(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine_native::Memory,
    base: usize,
    country_pointer: usize,
    slot: u32,
    roster: &Array,
    ordinal: u32,
) -> bool {
    let addr = roster.element(ordinal, 8);
    let entry = match bspan(log, memory, addr, 8, AIR_ACES_MASK) {
        Some(start) => start,
        None => return false,
    };
    let pointer = match usize::try_from(bases_pool_u64(log, entry))
        .ok()
        .filter(|&pointer| pointer != 0)
    {
        Some(pointer) => pointer,
        None => return false,
    };
    let identity = bspan(log, memory, Some(pointer), 16, AIR_ACES_MASK);
    let mask = bspan(
        log,
        memory,
        pointer.checked_add(cace::AIRCRAFT_TYPE_MASK),
        4,
        AIR_ACES_MASK,
    );
    let owner = bspan(
        log,
        memory,
        pointer.checked_add(cace::OWNER_COUNTRY),
        16,
        AIR_ACES_MASK,
    );
    let strings = bspan(
        log,
        memory,
        pointer.checked_add(cace::NAME),
        96,
        AIR_ACES_MASK,
    );
    let alive = bspan(
        log,
        memory,
        pointer.checked_add(cace::ALIVE),
        1,
        AIR_ACES_MASK,
    );
    let kill_type = bspan(
        log,
        memory,
        pointer.checked_add(cace::KILL_TYPE),
        4,
        AIR_ACES_MASK,
    );
    let handled = bspan(
        log,
        memory,
        pointer.checked_add(cace::HANDLED),
        1,
        AIR_ACES_MASK,
    );
    let (Some(identity), Some(_), Some(owner), Some(strings), Some(alive), Some(_), Some(handled)) =
        (identity, mask, owner, strings, alive, kill_type, handled)
    else {
        return false;
    };
    let expected = match engine_sites::at(base, cace::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    if bases_pool_u64(log, identity) != expected
        || bases_pool_u64(log, identity + 8) == 0
        || bases_pool_u64(log, owner) != country_pointer as u64
        || log.bytes[alive] > 1
        || log.bytes[handled] > 1
    {
        return false;
    }
    if log.bytes[alive] == 0
        && bspan(
            log,
            memory,
            pointer.checked_add(cace::KILLER_ACE_REFERENCE),
            12,
            AIR_ACES_MASK,
        )
        .is_none()
    {
        return false;
    }
    for index in 0..3 {
        let head = strings + index * 32;
        if !copy_ace_name(log, memory, head) {
            return false;
        }
        if log.overflowed {
            return false;
        }
    }
    let portrait = bspan(
        log,
        memory,
        pointer.checked_add(cace::PORTRAIT_GENDER),
        8,
        AIR_ACES_MASK,
    );
    if let Some(selected) = bspan(
        log,
        memory,
        pointer.checked_add(cace::TYPE_DEFINITION),
        8,
        AIR_ACES_MASK,
    ) {
        let definition = bases_pool_u64(log, selected) as usize;
        if definition != 0
            && let Some(head) = bspan(
                log,
                memory,
                definition.checked_add(cace::TYPE_NAME),
                32,
                AIR_ACES_MASK,
            )
        {
            copy_ace_name(log, memory, head);
        }
    }
    if let Some(portrait) = portrait {
        let gender = log.bytes[portrait];
        let index = i32::from_le_bytes(log.bytes[portrait + 4..portrait + 8].try_into().unwrap());
        let staged = AcePresentationMemory {
            memory,
            log: std::cell::RefCell::new(log),
        };
        if let Ok(names) = cace::portrait_sources(
            &staged,
            base,
            ccountry::Country {
                pointer: country_pointer,
                canonical_slot: slot,
                id: slot,
            },
            gender,
            index,
        ) {
            for name in names.into_iter().flatten() {
                if let Some((address, len)) = name.heap_payload() {
                    let mut log = staged.log.borrow_mut();
                    bspan(&mut log, memory, Some(address), len, AIR_ACES_MASK);
                }
            }
        }
    }
    !log.overflowed
}

struct AcePresentationMemory<'a, M> {
    memory: &'a M,
    log: std::cell::RefCell<&'a mut RawLog<AIR_BASES_N>>,
}
impl<M: Memory> Memory for AcePresentationMemory<'_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let mut log = self.log.borrow_mut();
        let Some(start) = bspan(
            &mut log,
            self.memory,
            Some(address),
            destination.len(),
            AIR_ACES_MASK,
        ) else {
            return false;
        };
        destination.copy_from_slice(&log.bytes[start..start + destination.len()]);
        true
    }
}

fn copy_ace_name(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine_native::Memory,
    head: usize,
) -> bool {
    let header = &log.bytes[head..head + 32];
    let len = u64::from_le_bytes(
        header[msvc_string::LENGTH..msvc_string::LENGTH + 8]
            .try_into()
            .unwrap(),
    );
    let capacity = u64::from_le_bytes(
        header[msvc_string::CAPACITY..msvc_string::CAPACITY + 8]
            .try_into()
            .unwrap(),
    );
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return false;
    }
    let len = match usize::try_from(len) {
        Ok(len) => len,
        Err(_) => return false,
    };
    if capacity == msvc_string::INLINE_CAPACITY {
        if header.get(len) != Some(&0) {
            return false;
        }
        return true;
    }
    let source = match usize::try_from(u64::from_le_bytes(
        header[msvc_string::DATA..msvc_string::DATA + 8]
            .try_into()
            .unwrap(),
    ))
    .ok()
    .filter(|&source| source != 0)
    {
        Some(source) => source,
        None => return false,
    };
    if source.checked_add(len).is_none() {
        return false;
    }
    if bspan(log, memory, source.checked_add(len), 1, AIR_ACES_MASK).is_none() {
        return false;
    }
    if len == 0 {
        return true;
    }
    if bspan(log, memory, Some(source), len, AIR_ACES_MASK).is_none() {
        return false;
    }
    true
}

#[derive(Clone, Copy, Default)]
struct Planned {
    slot: u32,
    container: usize,
    vtable: usize,
    span: usize,
}

enum ChunkWords {
    Planned(std::ops::Range<usize>),
    Taken { start: usize, copied: usize },
}

struct SlotPass<'a> {
    run: crate::air_base_changes::SlotRun,
    report: &'a mut crate::air_base_changes::SlotReport,
}

#[derive(Default)]
struct DenseSnapshot {
    header: Option<Array>,
    block: Option<usize>,
}

#[allow(clippy::too_many_arguments)]
fn copy_dense_plan(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine_native::Memory,
    image: usize,
    pointer: usize,
    containers: &Array,
    links: &Array,
    countries: u32,
    seen: &mut Vec<(u32, u32)>,
    seen_index: &mut [usize],
    plan: &mut [Planned; cair_base::DENSE_BOUND],
    snapshot: &mut DenseSnapshot,
) -> Result<usize, cair_base::DenseRefusal> {
    use cair_base::DenseRefusal as Refusal;
    let header = bspan(
        log,
        memory,
        pointer.checked_add(cair_base::REGISTERED_CONTAINERS),
        16,
        AIR_REGISTRATIONS_MASK,
    )
    .ok_or(Refusal::Header)?;
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header..header + 16]);
    let dense = Array::from_header(raw, 8).ok_or(Refusal::Header)?;
    snapshot.header = Some(dense);
    if dense.count > containers.count {
        return Err(Refusal::LongerThanSlots);
    }
    let len = dense.count as usize;
    if len > cair_base::DENSE_BOUND {
        return Err(Refusal::OverBound);
    }
    let block = if len == 0 {
        0
    } else {
        bspan(
            log,
            memory,
            Some(dense.data),
            len * 8,
            AIR_REGISTRATIONS_MASK,
        )
        .ok_or(Refusal::Header)?
    };
    snapshot.block = Some(block);
    let expected =
        engine_sites::at(image, ccountry_air_container::VTABLE_RVA).ok_or(Refusal::Container)? as u64;
    for index in 0..len {
        let container = block_word(log, block, index)
            .filter(|&pointer| pointer != 0)
            .ok_or(Refusal::Entry)?;
        if plan[..index]
            .iter()
            .any(|entry| entry.container == container)
        {
            return Err(Refusal::Entry);
        }
        let vtable = bspan(log, memory, Some(container), 8, AIR_REGISTRATIONS_MASK)
            .ok_or(Refusal::Container)?;
        if bases_pool_u64(log, vtable) != expected {
            return Err(Refusal::Container);
        }
        let span = bspan(
            log,
            memory,
            container.checked_add(ccountry_air_container::COUNTRY),
            cair_base::CONTAINER_SPAN,
            AIR_REGISTRATIONS_MASK,
        )
        .ok_or(Refusal::Container)?;
        let country = bases_pool_u32(log, span);
        let slot = if country == 0 {
            0
        } else {
            resolve_link(log, memory, links, seen, seen_index, countries, country)
                .ok_or(Refusal::Country)?
        };
        if log.overflowed {
            return Err(Refusal::Country);
        }
        if slot >= containers.count {
            return Err(Refusal::SlotRange);
        }
        if plan[..index].iter().any(|entry| entry.slot == slot) {
            return Err(Refusal::SharedSlot);
        }
        let word = bspan(
            log,
            memory,
            containers.element(slot, 8),
            8,
            AIR_REGISTRATIONS_MASK,
        )
        .ok_or(Refusal::Stale)?;
        if bases_pool_u64(log, word) != container as u64 {
            return Err(Refusal::Stale);
        }
        plan[index] = Planned {
            slot,
            container,
            vtable,
            span,
        };
    }
    plan[..len].sort_unstable_by_key(|entry| entry.slot);
    Ok(len)
}

fn snapshot_state_countries(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl Memory,
    pointer: usize,
    snapshot: &mut DenseSnapshot,
) {
    if snapshot.header.is_none() {
        let Some(start) = bspan(
            log,
            memory,
            pointer.checked_add(cair_base::REGISTERED_CONTAINERS),
            16,
            0,
        ) else {
            return;
        };
        let mut raw = [0; 16];
        raw.copy_from_slice(&log.bytes[start..start + 16]);
        snapshot.header = Array::from_header(raw, 8);
    }
    let Some(dense) = snapshot.header else {
        return;
    };
    if snapshot.block.is_none() {
        snapshot.block = if dense.count == 0 {
            Some(0)
        } else {
            (dense.count as usize)
                .checked_mul(8)
                .and_then(|len| bspan(log, memory, Some(dense.data), len, 0))
        };
    }
}

struct CarryPass<'a> {
    run: CarryRun,
    store: Option<&'a mut super::RegistrationStore>,
    reached: &'a mut Reached,
    segments: &'a mut Vec<HeldSegment>,
    work: &'a mut collector_observers::air_base_carry_changes::Work,
}

impl CarryPass<'_> {
    fn segment(&mut self, handle: u64, pointer: usize, id: u64) {
        if allocation::advisory(|| allocation::reserve(self.segments, 1)).is_ok() {
            self.segments.push(HeldSegment {
                handle,
                pointer,
                id,
            });
        }
    }

    fn carry_row(
        &mut self,
        log: &mut RawLog<AIR_BASES_N>,
        pointer: usize,
        slot: (usize, u32),
    ) -> Option<(u64, (u64, u64), cair_base::Row)> {
        let changes = self.run.changes;
        let held = self
            .store
            .as_deref_mut()?
            .census
            .row(pointer, slot, |handle| changes.sequence(handle))?;
        let carried = (held.handle, held.certificate, held.row);
        log.splice(held.spans, held.bytes).then_some(carried)
    }

    #[allow(clippy::too_many_arguments)]
    fn hold_row(
        &mut self,
        log: &RawLog<AIR_BASES_N>,
        from: (usize, usize),
        pointer: usize,
        slot: (usize, u32),
        handle: u64,
        certificate: Option<(u64, u64)>,
        row: cair_base::Row,
    ) {
        if let Some(store) = self.store.as_deref_mut() {
            let fresh = super::census_store::Fresh {
                spans: &log.spans[from.0..],
                bytes: &log.bytes[from.1..],
                start: from.1,
            };
            store
                .census
                .hold_row(pointer, slot, handle, certificate, row, &fresh);
        }
    }
}

const SLOT_BATCH: usize = 64;

fn take_slots(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine_native::Memory,
    slots: Array,
) -> usize {
    let mut copied = 0;
    while copied < slots.count as usize {
        let chunk = (slots.count as usize - copied).min(SLOT_BATCH);
        let Some(address) = slots.element(copied as u32, 8) else {
            break;
        };
        match log.take_array_entries(memory, address, 8, chunk, BASE_VECTORS_MASK) {
            Some((_, done)) => {
                copied += done;
                if done != chunk {
                    break;
                }
            }
            None => break,
        }
    }
    copied
}

#[allow(clippy::too_many_arguments)]
fn registrations(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine_native::Memory,
    base: usize,
    handle: u64,
    certificate: Option<(u64, u64)>,
    pointer: usize,
    host: bool,
    links: &Array,
    countries: u32,
    seen: &mut Vec<(u32, u32)>,
    seen_index: &mut [usize],
    demand: &mut Option<[usize; 2]>,
    carrier: &mut Option<cair_base::CarrierCapture<'_, impl Memory>>,
    slots: &mut SlotPass<'_>,
    carry: &mut CarryPass<'_>,
) -> bool {
    use collector_observers::air_base_carry_changes::Work;
    let started = qpc();
    carry.reached.clear();
    let Some(store) = carry.store.as_deref_mut().filter(|_| !host) else {
        let read = copy_air_base_registrations(
            log,
            memory,
            base,
            pointer,
            &mut DenseSnapshot::default(),
            links,
            countries,
            seen,
            seen_index,
            demand,
            carrier,
            slots,
            carry.reached,
        );
        carry.work.note(Work::UNCARRIED, started);
        return read;
    };
    store.visit();
    let planned = slots.run.dense;
    if let Some(held) = store.carryable(handle, pointer, certificate, planned) {
        let (rows, segment) = (held.demand, held.segment);
        let spliced = (held.spans.len(), held.bytes.len());
        let resolved = held.links.iter().all(|&(country, slot)| {
            resolve_link(log, memory, links, seen, seen_index, countries, country) == Some(slot)
                && !log.overflowed
        });
        if resolved && log.splice(held.spans, held.bytes) {
            *demand = demand.and_then(|[read, wings]| {
                Some([read.checked_add(rows[0])?, wings.checked_add(rows[1])?])
            });
            store.carried();
            carry.work.splice(spliced.0, spliced.1);
            carry.segment(handle, pointer, segment);
            carry.work.note(Work::CARRIED, started);
            return true;
        }
        if log.overflowed {
            carry.work.note(Work::CARRIED, started);
            return false;
        }
    }
    let (spans, bytes, before) = (log.spans.len(), log.bytes.len(), *demand);
    let mut dense_snapshot = DenseSnapshot::default();
    if !copy_air_base_registrations(
        log,
        memory,
        base,
        pointer,
        &mut dense_snapshot,
        links,
        countries,
        seen,
        seen_index,
        demand,
        carrier,
        slots,
        carry.reached,
    ) {
        carry.work.note(Work::FRESH, started);
        return false;
    }
    let pools_name_base = carry.reached.pools.iter().all(|&pool| {
        pool.checked_add(engine_native::cair_wing_pool::AIR_BASE_REFERENCE)
            .and_then(|at| engine_native::read::<u64>(memory, at))
            == Some(handle)
    });
    let certificate = certificate.filter(|_| pools_name_base);
    if certificate.is_some() {
        snapshot_state_countries(log, memory, pointer, &mut dense_snapshot);
    }
    let read = before
        .zip(*demand)
        .map(|([rows, wings], [after_rows, after_wings])| [after_rows - rows, after_wings - wings]);
    if let Some(read) = read
        && !carry.reached.lost
    {
        let fresh = super::registration_store::Fresh {
            spans: &log.spans[spans..],
            bytes: &log.bytes[bytes..],
            start: bytes,
            links: &carry.reached.links,
            demand: read,
            link_table: links.data..links.data + links.count as usize * 4,
            containers: &carry.reached.containers,
            wings: &carry.reached.wings,
        };
        store.hold(handle, pointer, certificate, planned, &fresh);
        if let Some(segment) = store.segment(handle) {
            carry.segment(handle, pointer, segment);
        }
    }
    carry.work.note(Work::FRESH, started);
    true
}

#[allow(clippy::too_many_arguments)]
fn copy_air_base_registrations(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine_native::Memory,
    base: usize,
    pointer: usize,
    dense_snapshot: &mut DenseSnapshot,
    links: &Array,
    countries: u32,
    seen: &mut Vec<(u32, u32)>,
    seen_index: &mut [usize],
    demand: &mut Option<[usize; 2]>,
    carrier: &mut Option<cair_base::CarrierCapture<'_, impl Memory>>,
    slots: &mut SlotPass<'_>,
    reached: &mut Reached,
) -> bool {
    let header = match bspan(
        log,
        memory,
        pointer.checked_add(cair_base::COUNTRY_CONTAINERS),
        16,
        AIR_REGISTRATIONS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header..header + 16]);
    let containers = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return false,
    };
    if let Some(carrier) = carrier.as_mut() {
        carrier.indexed_header(containers.count);
    }
    let expected_pool = match engine_sites::at(base, cair_wing_pool::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    let expected_primary = match engine_sites::at(base, engine_native_air::cair_wing::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    let expected_secondary = match engine_sites::at(base, engine_native_air::cair_wing::SECONDARY_VTABLE_RVA)
    {
        Some(expected) => expected as u64,
        None => return false,
    };
    let expected_container = match engine_sites::at(base, ccountry_air_container::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    let mut plan = [Planned::default(); cair_base::DENSE_BOUND];
    let planned = if slots.run.dense {
        match copy_dense_plan(
            log,
            memory,
            base,
            pointer,
            &containers,
            links,
            countries,
            seen,
            seen_index,
            &mut plan,
            dense_snapshot,
        ) {
            Ok(len) => Some(len),
            Err(refusal) => {
                slots.report.refusals[refusal as usize] += 1;
                None
            }
        }
    } else {
        None
    };
    if log.overflowed {
        return false;
    }
    match planned {
        Some(_) => slots.report.dense += 1,
        None => slots.report.full += 1,
    }
    let mut next_planned = 0;
    for chunk_first in (0..containers.count).step_by(REG_MIRROR_BATCH) {
        let count = ((containers.count - chunk_first) as usize).min(REG_MIRROR_BATCH);
        let container_chunk = match planned {
            Some(len) => {
                let end = chunk_first + count as u32;
                let first = next_planned;
                while plan[..len]
                    .get(next_planned)
                    .is_some_and(|entry| entry.slot < end)
                {
                    next_planned += 1;
                }
                if first == next_planned {
                    continue;
                }
                ChunkWords::Planned(first..next_planned)
            }
            None => {
                let Some(address) = containers.element(chunk_first, 8) else {
                    return false;
                };
                match log.take_array_chunk(memory, address, 8, count, AIR_REGISTRATIONS_MASK) {
                    Some((start, copied)) => ChunkWords::Taken { start, copied },
                    None => return false,
                }
            }
        };
        for offset in 0..count {
            let slot = chunk_first + offset as u32;
            let (container, listed) = match &container_chunk {
                ChunkWords::Planned(listed) => {
                    match plan[listed.clone()].iter().find(|entry| entry.slot == slot) {
                        Some(entry) => (entry.container, Some(*entry)),
                        None => continue,
                    }
                }
                ChunkWords::Taken { start, copied } => {
                    if offset >= *copied {
                        return false;
                    }
                    match usize::try_from(bases_pool_u64(log, start + offset * 8))
                        .ok()
                        .filter(|&pointer| pointer != 0)
                    {
                        Some(container) => (container, None),
                        None => continue,
                    }
                }
            };
            let vtable = match listed {
                Some(entry) => entry.vtable,
                None => match bspan(log, memory, Some(container), 8, AIR_REGISTRATIONS_MASK) {
                    Some(start) => start,
                    None => return false,
                },
            };
            if bases_pool_u64(log, vtable) != expected_container {
                return false;
            }
            let span = match listed {
                Some(entry) => entry.span,
                None => match bspan(
                    log,
                    memory,
                    container.checked_add(ccountry_air_container::COUNTRY),
                    cair_base::CONTAINER_SPAN,
                    AIR_REGISTRATIONS_MASK,
                ) {
                    Some(start) => start,
                    None => return false,
                },
            };
            let country = bases_pool_u32(log, span);
            reached.container(container);
            let mut words = [0u8; cair_base::CONTAINER_SPAN];
            words.copy_from_slice(&log.bytes[span..span + cair_base::CONTAINER_SPAN]);
            let (_, fuel) = cair_base::container_span_words(&words);
            if slot == 0 {
                if country != 0 {
                    match resolve_link(log, memory, links, seen, seen_index, countries, country) {
                        Some(0) => {}
                        _ => return false,
                    }
                    if log.overflowed {
                        return false;
                    }
                }
            } else {
                match resolve_link(log, memory, links, seen, seen_index, countries, country) {
                    Some(canonical) if canonical == slot => {}
                    _ => return false,
                }
                if log.overflowed {
                    return false;
                }
            }
            if country != 0 {
                reached.link(country, slot);
            }
            if let Some(carrier) = carrier.as_mut() {
                carrier.indexed_container(country, slot, container);
                carrier.begin_registration(fuel);
            }
            let pools_header = match bspan(
                log,
                memory,
                container.checked_add(ccountry_air_container::POOLS),
                16,
                AIR_REGISTRATIONS_MASK,
            ) {
                Some(start) => start,
                None => return false,
            };
            raw.copy_from_slice(&log.bytes[pools_header..pools_header + 16]);
            let pools = match Array::from_header(raw, 8) {
                Some(array) => array,
                None => return false,
            };
            if let Some(carrier) = carrier.as_mut() {
                carrier.registration_header(pools.count);
            }
            let mut registration_wings = Some(0usize);
            *demand = demand.and_then(|[rows, wings]| Some([rows.checked_add(1)?, wings]));
            let mut pool_first = 0u32;
            while pool_first < pools.count {
                let chunk = ((pools.count - pool_first) as usize).min(REG_MIRROR_BATCH);
                let first_address = pools.element(pool_first, 8);
                let block = match bspan(
                    log,
                    memory,
                    first_address,
                    chunk * 8,
                    AIR_REGISTRATIONS_MASK,
                ) {
                    Some(start) => start,
                    None => return false,
                };
                for index in 0..chunk {
                    let pointer = match block_word(log, block, index) {
                        Some(pointer) => pointer,
                        None => return false,
                    };
                    if pointer == 0 {
                        return false;
                    }
                    let pool_vtable =
                        match bspan(log, memory, Some(pointer), 8, AIR_REGISTRATIONS_MASK) {
                            Some(start) => start,
                            None => return false,
                        };
                    let wings = match bspan(
                        log,
                        memory,
                        pointer.checked_add(engine_native::cair_wing_pool::WINGS),
                        16,
                        AIR_REGISTRATIONS_MASK,
                    ) {
                        Some(start) => start,
                        None => return false,
                    };
                    if bases_pool_u64(log, pool_vtable) != expected_pool {
                        return false;
                    }
                    reached.pool(pointer);
                    let mut header = [0u8; 16];
                    header.copy_from_slice(&log.bytes[wings..wings + 16]);
                    let members = match Array::from_header(header, 8) {
                        Some(array) => array,
                        None => return false,
                    };
                    registration_wings = registration_wings
                        .and_then(|count| count.checked_add(members.count as usize));
                    if let Some(carrier) = carrier.as_mut() {
                        carrier.begin_pool(pointer, members);
                    }
                    *demand = demand.and_then(|[rows, wings]| {
                        Some([rows, wings.checked_add(members.count as usize)?])
                    });
                    let mut wing_first = 0u32;
                    while wing_first < members.count {
                        let wing_chunk =
                            ((members.count - wing_first) as usize).min(REG_MIRROR_BATCH);
                        let wing_address = members.element(wing_first, 8);
                        let wing_block = match bspan(
                            log,
                            memory,
                            wing_address,
                            wing_chunk * 8,
                            AIR_REGISTRATIONS_MASK,
                        ) {
                            Some(start) => start,
                            None => return false,
                        };
                        let mut wing_pointers = [0usize; REG_MIRROR_BATCH];
                        for (wing_index, destination) in
                            wing_pointers[..wing_chunk].iter_mut().enumerate()
                        {
                            let wing = match block_word(log, wing_block, wing_index) {
                                Some(wing) => wing,
                                None => return false,
                            };
                            if wing == 0 {
                                return false;
                            }
                            *destination = wing;
                        }
                        let mut fields = [(0usize, 0usize); REG_MIRROR_BATCH];
                        let mut complete_wings = 0;
                        for (wing_index, &wing) in wing_pointers[..wing_chunk].iter().enumerate() {
                            let identity =
                                match bspan(log, memory, Some(wing), 32, AIR_REGISTRATIONS_MASK) {
                                    Some(start) => start,
                                    None => break,
                                };
                            reached.wing(wing);
                            let backlink = match bspan(
                                log,
                                memory,
                                wing.checked_add(engine_native_air::cair_wing::POOL),
                                8,
                                AIR_REGISTRATIONS_MASK,
                            ) {
                                Some(start) => start,
                                None => break,
                            };
                            fields[wing_index] = (identity, backlink);
                            complete_wings += 1;
                        }
                        if log.overflowed {
                            return false;
                        }
                        for (wing_index, &(identity, backlink)) in
                            fields[..complete_wings].iter().enumerate()
                        {
                            if bases_pool_u64(log, identity) != expected_primary
                                || bases_pool_u64(log, identity + 16) != expected_secondary
                            {
                                return false;
                            }
                            if bases_pool_u64(log, backlink) != pointer as u64
                                || bases_pool_u64(log, identity + 24) == 0
                            {
                                return false;
                            }
                            if let Some(carrier) = carrier.as_mut() {
                                carrier.wing(wing_pointers[wing_index]);
                            }
                        }
                        if complete_wings != wing_chunk {
                            return false;
                        }
                        wing_first += wing_chunk as u32;
                    }
                    if let Some(carrier) = carrier.as_mut() {
                        carrier.finish_pool();
                    }
                }
                if log.overflowed {
                    return false;
                }
                pool_first += chunk as u32;
            }
            if let Some(carrier) = carrier.as_mut() {
                carrier.finish_registration(registration_wings);
            }
            if log.overflowed {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
pub(super) fn copy_air_bases_pass(
    job: &mut AirBasesJob,
    memory: &impl engine_native::Memory,
    base: usize,
    game_state: usize,
    aces_active: bool,
    ticks: &mut [u64; AIR_BASES_N],
) {
    copy_air_bases_pass_with_context(job, memory, base, game_state, aces_active, ticks);
}

fn copy_air_bases_pass_with_context(
    job: &mut AirBasesJob,
    memory: &impl engine_native::Memory,
    base: usize,
    game_state: usize,
    aces_active: bool,
    ticks: &mut [u64; AIR_BASES_N],
) {
    let started = qpc();
    job.clear();
    job.game_state = game_state;
    job.aces_active = aces_active;
    let census_mask = if aces_active {
        AIR_BASES_CENSUS_MASK
    } else {
        BASE_VECTORS_MASK
    };
    let census = read_accounting::with_table(TableId::AirBases, || {
        copy_air_bases_census(
            &mut job.log,
            memory,
            base,
            game_state,
            &mut job.census,
            &mut job.roster,
            [&mut job.required_census, &mut job.required_links],
            census_mask,
        )
    });
    ticks[0] += elapsed(started);
    if job.log.overflowed {
        return;
    }
    let (links, countries) = match census {
        Some(census) => census,
        None => {
            copy_air_bases_vectors(job, memory, base, None, ticks);
            return;
        }
    };
    copy_air_bases_vectors(job, memory, base, Some((&links, countries)), ticks);
    if job.log.overflowed || !aces_active {
        return;
    }
    let _reads = read_accounting::table(TableId::AirAces);
    let aces_started = qpc();
    let mut ace_index = 0;
    while ace_index < job.census.len() {
        let (pointer, slot) = job.census[ace_index];
        ace_index += 1;
        if !copy_air_aces_country(&mut job.log, memory, base, pointer, slot) {
            break;
        }
        if job.log.overflowed {
            break;
        }
    }
    ticks[2] += elapsed(aces_started);
}

#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn copy_air_bases_vectors(
    job: &mut AirBasesJob,
    memory: &impl engine_native::Memory,
    base: usize,
    census: Option<(&Array, u32)>,
    ticks: &mut [u64; AIR_BASES_N],
) {
    let _reads = read_accounting::table(TableId::AirBases);
    let vectors_started = qpc();
    let AirBasesJob {
        log,
        carrier: staged,
        game_state,
        seen_links,
        seen_link_index,
        registration_demand: job_registration_demand,
        slot_run,
        slot_report,
        carry: carry_run,
        registration_store,
        reached,
        segments,
        registration_work,
        ..
    } = job;
    let mut slots_pass = SlotPass {
        run: *slot_run,
        report: slot_report,
    };
    let mut carry = CarryPass {
        run: *carry_run,
        store: registration_store.as_deref_mut(),
        reached,
        segments,
        work: registration_work,
    };
    let mut complete = false;
    if let Some(store) = carry.store.as_deref_mut() {
        store.begin();
    }
    let carrier::Job {
        log: carrier_log,
        index,
        required_index,
        bases: carrier_bases,
        registrations: carrier_registrations,
        scratch,
        demand: staged_demand,
        seed,
    } = staged;
    let index = std::cell::RefCell::new(super::carrier_memory::ExactIndex::new(index));
    *seed = super::carrier_memory::seed_census(carrier_log.get_mut(), log, &mut index.borrow_mut());
    *required_index = index.borrow().required_slots();
    let (lengths, demand) = {
        let optional = super::carrier_memory::CaptureMemory::new(carrier_log, memory, &index);
        let countries = seed.and_then(|()| ccountry::Reader::new(&optional, base, *game_state));
        let refusal = countries.as_ref().err().copied();
        let mut observer = countries.as_ref().ok().map(|countries| {
            cair_base::CarrierCapture::new(
                &optional,
                base,
                *game_state,
                countries,
                carrier::buffers(carrier_bases, carrier_registrations, scratch),
            )
        });
        (|| {
            let manager_spans = log.spans.len();
            let manager = match bspan(
                &mut *log,
                memory,
                (*game_state).checked_add(cgame_state::STRATEGIC_AIR_MANAGER),
                8,
                BASE_VECTORS_MASK,
            ) {
                Some(start) => match usize::try_from(bases_pool_u64(&*log, start))
                    .ok()
                    .filter(|&pointer| pointer != 0)
                {
                    Some(manager) => manager,
                    None => {
                        ticks[0] += elapsed(vectors_started);
                        return;
                    }
                },
                None => {
                    ticks[0] += elapsed(vectors_started);
                    return;
                }
            };
            let vtable = bspan(&mut *log, memory, Some(manager), 8, BASE_VECTORS_MASK);
            let expected_manager = engine_sites::at(base, cstrategic_air_manager::VTABLE_RVA);
            let manager_ok = match (vtable, expected_manager) {
                (Some(start), Some(expected)) => bases_pool_u64(&*log, start) == expected as u64,
                _ => false,
            };
            let mut headers = [(0usize, 0u32, true); 4];
            let mut headers_ok = manager_ok;
            for (slot, (offset, indexed)) in [
                (cstrategic_air_manager::STATE_AIR_BASES, true),
                (cstrategic_air_manager::STATE_ROCKET_BASES, true),
                (cstrategic_air_manager::STATE_GUN_BASES, true),
                (cstrategic_air_manager::STATELESS_AIR_BASES, false),
            ]
            .into_iter()
            .enumerate()
            {
                let header = bspan(
                    &mut *log,
                    memory,
                    manager.checked_add(offset),
                    16,
                    BASE_VECTORS_MASK,
                );
                match header {
                    Some(start) => {
                        let mut raw = [0u8; 16];
                        raw.copy_from_slice(&log.bytes[start..start + 16]);
                        match Array::from_header(raw, 8) {
                            Some(array) => {
                                headers[slot] = (start, array.count, indexed);
                                let _ = array;
                            }
                            None => headers_ok = false,
                        }
                    }
                    None => headers_ok = false,
                }
                if log.overflowed {
                    ticks[0] += elapsed(vectors_started);
                    return;
                }
            }
            let mut bases_alive = headers_ok;
            let mut reg_alive = headers_ok && census.is_some();
            let mut registration_demand = Some([0usize; 2]);
            let mut bases_ticks = 0u64;
            let mut reg_ticks = 0u64;
            let changes = carry.run.changes;
            let membership = carry
                .store
                .is_some()
                .then(|| changes.membership())
                .flatten();
            if let Some(store) = carry.store.as_deref_mut() {
                store.census.read(log.spans.len() - manager_spans);
            }
            for (array, (header_start, count, indexed)) in headers.into_iter().enumerate() {
                if !bases_alive && !reg_alive {
                    break;
                }
                let mut raw = [0u8; 16];
                raw.copy_from_slice(&log.bytes[header_start..header_start + 16]);
                let Some(slots) = Array::from_header(raw, 8) else {
                    break;
                };
                let _ = count;
                let (first_span, start) = (log.spans.len(), log.bytes.len());
                let held = match carry.store.as_deref_mut() {
                    Some(store) if indexed => store.census.slots(array, slots, membership),
                    _ => None,
                };
                let copied = match held {
                    Some((spans, bytes)) => {
                        if !log.splice(spans, bytes) {
                            break;
                        }
                        slots.count as usize
                    }
                    None => {
                        let copied = take_slots(&mut *log, memory, slots);
                        if let Some(store) = carry.store.as_deref_mut() {
                            if indexed {
                                let fresh = super::census_store::Fresh {
                                    spans: &log.spans[first_span..],
                                    bytes: &log.bytes[start..],
                                    start,
                                };
                                let complete = copied == slots.count as usize && !log.overflowed;
                                store
                                    .census
                                    .hold_slots(array, slots, membership, complete, &fresh);
                            } else {
                                store.census.read(log.spans.len() - first_span);
                            }
                        }
                        copied
                    }
                };
                for index in 0..copied as u32 {
                    if !bases_alive && !reg_alive {
                        break;
                    }
                    let pointer =
                        match usize::try_from(bases_pool_u64(&*log, start + index as usize * 8))
                            .ok()
                            .filter(|&pointer| pointer != 0)
                        {
                            Some(pointer) => pointer,
                            None => {
                                if indexed {
                                    continue;
                                }
                                bases_alive = false;
                                reg_alive = false;
                                break;
                            }
                        };
                    let slot = (array, index);
                    if bases_alive {
                        let begin = qpc();
                        let from = (log.spans.len(), log.bytes.len());
                        let carried = indexed
                            .then(|| carry.carry_row(&mut *log, pointer, slot))
                            .flatten();
                        let (handle, certificate) = match carried {
                            Some((handle, certificate, source)) => {
                                if let Some(carrier) = observer.as_mut() {
                                    carrier.begin_base(pointer, handle, false);
                                }
                                optional.set_mask(1);
                                if let Some(carrier) = observer.as_mut() {
                                    carrier.authenticated_base(source);
                                }
                                (handle, Some(certificate))
                            }
                            None => {
                                let (identity_ok, handle) =
                                    base_identity(&mut *log, memory, base, pointer);
                                if !identity_ok {
                                    bases_alive = false;
                                    reg_alive = false;
                                    bases_ticks += elapsed(begin);
                                    break;
                                }
                                let certificate = (indexed && carry.store.is_some())
                                    .then(|| changes.sequence(handle))
                                    .flatten();
                                if let Some(carrier) = observer.as_mut() {
                                    carrier.begin_base(pointer, handle, !indexed);
                                }
                                optional.set_mask(1);
                                match copy_air_base_row(
                                    &mut *log,
                                    memory,
                                    base,
                                    pointer,
                                    indexed.then_some(index),
                                    handle,
                                ) {
                                    Some(source) => {
                                        if let Some(carrier) = observer.as_mut() {
                                            carrier.authenticated_base(source);
                                        }
                                        if indexed {
                                            carry.hold_row(
                                                &*log,
                                                from,
                                                pointer,
                                                slot,
                                                handle,
                                                certificate,
                                                source,
                                            );
                                        }
                                    }
                                    None => bases_alive = false,
                                }
                                if !indexed && let Some(store) = carry.store.as_deref_mut() {
                                    store.census.read(log.spans.len() - from.0);
                                }
                                (handle, certificate)
                            }
                        };
                        bases_ticks += elapsed(begin);
                        if reg_alive {
                            let _reads = read_accounting::table(TableId::AirBaseRegistrations);
                            let begin = qpc();
                            let (links, countries) = census.expect("reg takes need census links");
                            optional.set_mask(2);
                            if !registrations(
                                &mut *log,
                                memory,
                                base,
                                handle,
                                certificate,
                                pointer,
                                !indexed,
                                links,
                                countries,
                                &mut *seen_links,
                                &mut *seen_link_index,
                                &mut registration_demand,
                                &mut observer,
                                &mut slots_pass,
                                &mut carry,
                            ) {
                                reg_alive = false;
                            }
                            reg_ticks += elapsed(begin);
                        }
                    } else if reg_alive {
                        let _reads = read_accounting::table(TableId::AirBaseRegistrations);
                        let begin = qpc();
                        let (identity_ok, handle) = base_identity(&mut *log, memory, base, pointer);
                        if !identity_ok {
                            reg_alive = false;
                            reg_ticks += elapsed(begin);
                            break;
                        }
                        let certificate = (indexed && carry.store.is_some())
                            .then(|| changes.sequence(handle))
                            .flatten();
                        if let Some(carrier) = observer.as_mut() {
                            carrier.begin_base(pointer, handle, !indexed);
                        }
                        optional.set_mask(2);
                        let (links, countries) = census.expect("reg takes need census links");
                        if !registrations(
                            &mut *log,
                            memory,
                            base,
                            handle,
                            certificate,
                            pointer,
                            !indexed,
                            links,
                            countries,
                            &mut *seen_links,
                            &mut *seen_link_index,
                            &mut registration_demand,
                            &mut observer,
                            &mut slots_pass,
                            &mut carry,
                        ) {
                            reg_alive = false;
                        }
                        reg_ticks += elapsed(begin);
                    }
                    optional.set_mask(1);
                    let dense_started = qpc();
                    if let Some(carrier) = observer.as_mut() {
                        carrier.finish_base(reg_alive);
                    }
                    bases_ticks += elapsed(dense_started);
                    if log.overflowed {
                        break;
                    }
                }
                if copied != slots.count as usize {
                    bases_alive = false;
                    reg_alive = false;
                }
                if log.overflowed {
                    break;
                }
            }
            if reg_alive && !log.overflowed {
                *job_registration_demand = registration_demand;
            }
            complete = reg_alive && !log.overflowed;
            ticks[0] += elapsed(vectors_started)
                .saturating_sub(bases_ticks)
                .saturating_sub(reg_ticks);
            ticks[0] += bases_ticks;
            ticks[1] += reg_ticks;
        })();
        if let Some(store) = carry.store.as_deref_mut() {
            store.finish(complete);
        }
        if let Some(carrier) = observer.as_mut() {
            carrier.finish_walk();
        }
        observer.as_ref().map_or(
            (
                cair_base::CarrierLengths::default(),
                cair_base::CarrierDemand {
                    failure: refusal,
                    ..Default::default()
                },
            ),
            |carrier| (carrier.lengths(), carrier.demand()),
        )
    };
    *required_index = (*required_index).max(index.borrow().required_slots());
    carrier_bases.install(lengths, demand);
    carrier_registrations.install(lengths, demand);
    *staged_demand = demand;
}

pub fn copy_air_bases_refresh(
    memory: &impl Memory,
    ctx: AirBasesCopyContext,
    job: &mut AirBasesJob,
) -> AirBasesCopyReport {
    job.slot_run = crate::air_base_changes::SlotRun::current();
    job.carry = CarryRun::default();
    let report = copy_air_bases_with_settings(memory, ctx, job);
    crate::air_base_changes::record(&job.slot_report);
    collector_observers::air_base_carry_changes::record_work(&job.registration_work);
    if let Some(store) = job.registration_store.as_deref() {
        collector_observers::air_base_carry_changes::record(
            &store.counts(),
            (!job.log.overflowed).then(|| store.bases()),
        );
        collector_observers::air_base_carry_changes::record_census(&store.census_counts());
    }
    report
}

#[cfg(any(test, feature = "test-support"))]
pub fn copy_air_bases_fixture(
    memory: &impl Memory,
    ctx: AirBasesCopyContext,
    job: &mut AirBasesJob,
) -> AirBasesCopyReport {
    copy_air_bases_with_settings(memory, ctx, job)
}

fn copy_air_bases_with_settings(
    memory: &impl Memory,
    ctx: AirBasesCopyContext,
    job: &mut AirBasesJob,
) -> AirBasesCopyReport {
    let mut ticks = [0u64; AIR_BASES_N];
    copy_air_bases_pass_with_context(
        job,
        memory,
        ctx.base,
        ctx.game_state,
        replay_data::cadence::should_walk(TableId::AirAces, ctx.hour, ctx.force_full),
        &mut ticks,
    );
    let state_inputs_started = qpc();
    super::state_fuel_inputs::capture(job, memory, ctx.base);
    ticks[1] += elapsed(state_inputs_started);
    AirBasesCopyReport {
        raw_ok: !job.log.overflowed,
        copy_ticks: ticks,
    }
}

impl Captured {
    pub fn fail_air_bases_capture(&mut self, reason: &'static str) {
        self.bases.outcome = Outcome::Unavailable { reason };
        self.carrier.reset();
        self.aces.outcome = failed(self.active[3], reason);
        self.names.reset();
        self.bases_raw_mode = false;
    }

    pub fn decode_air_bases_payload(
        &mut self,
        base: usize,
        job: &AirBasesJob,
        decode_ticks: &mut [u64; AIR_BASES_N],
    ) {
        self.active[3] = job.aces_active;
        let recorded = match collector_raw::raw::Recorded::build(&job.log) {
            Ok(recorded) => recorded,
            Err(reason) => {
                self.fail_air_bases_capture(reason);
                return;
            }
        };
        let started = qpc();
        let walked = cair_base::walk(&recorded, base, job.game_state, &mut self.bases.rows);
        self.bases.outcome = bounded(walked, self.bases.rows.len());
        decode_ticks[0] += elapsed(started);
        if !job.aces_active {
            self.aces.outcome = SKIPPED;
            self.names.reset();
            self.bases_raw_mode = false;
            return;
        }
        let started = qpc();
        let recorded = crate::extended::roster_replay::RosterReplay::new(
            &recorded,
            &job.roster,
            base,
            job.game_state,
        );
        let walked = cace::walk(
                &recorded,
                base,
                job.game_state,
                &mut self.aces.rows,
                &mut self.names.first,
            );
        self.aces.outcome = names_outcome(walked, &self.aces.rows, &self.names.first);
        self.names.len = walked.bytes_len;
        self.names.required = walked.required_bytes;
        decode_ticks[2] += elapsed(started);
        self.bases_raw_mode = false;
    }
}
