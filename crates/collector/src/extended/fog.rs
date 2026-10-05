//! Native fog-derivation input acquisition (registry domain D102). Only growth
//! and replay conversion allocate; every engine read finishes before the callback returns.

use crate::{
    read_accounting,
    sampling::qpc,
    table_capture::{Arena, CaptureRows, bounded, grow_capacity, grow_vec},
};
use engine::allocation::{copy_vec, owned_vec, reserve, resize};
use engine::{
    Memory, cair_base, ccontroller_area, ccountry, ctheatre,
    memory::{RecordLayout, RecordSources},
    radar_group, unit_viewer_entry,
    walk::Outcome,
};
use replay::{
    frame::TableInput,
    rows::{self, TableId, tables},
};

#[cfg(test)]
use replay::frame::TableView;

pub const N: usize = 7;
pub const TABLES: [TableId; N] = [
    TableId::RadarCoverage,
    TableId::VolunteerHosts,
    TableId::TheatreAreas,
    TableId::StationedUnits,
    TableId::UnitIntel,
    TableId::AirBaseRegistrations,
    TableId::SentVolunteers,
];

/// The stationed walk censuses areas and units together, so its cost is charged
/// once to TheatreAreas. The other families have independent roots.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    ("radar coverage groups", 0, &[0]),
    ("volunteer host vectors", 1, &[1]),
    ("volunteer theatre areas and stationed units", 2, &[2, 3]),
    ("unit viewer levels", 4, &[4]),
    ("air base registrations", 5, &[5]),
    ("sent volunteer references", 6, &[6]),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub radar_provinces: usize,
    pub area_provinces: usize,
    pub wings: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: [1024, 512, 512, 1024, 32_768, 2048, 512],
            radar_provinces: 65_536,
            area_provinces: 16_384,
            wings: 4096,
        }
    }
}

pub struct Captured {
    radar: CaptureRows<radar_group::Row>,
    radar_provinces: Arena<u32>,
    volunteers: CaptureRows<ccountry::VolunteerHostRow>,
    sent_volunteers: CaptureRows<ccountry::SentVolunteerRow>,
    areas: CaptureRows<ccontroller_area::Row>,
    area_provinces: Arena<u32>,
    units: CaptureRows<ctheatre::StationedUnitRow>,
    intel: CaptureRows<unit_viewer_entry::Row>,
    registrations: CaptureRows<cair_base::RegistrationRow>,
    wings: Arena<u64>,
    /// Dense per-walk country-link resolver for the registrations walk. It is
    /// sized from the live link count off the game thread (worker preparation
    /// or `grow`); a shorter cache only repeats resolves, never changes rows.
    resolved: Vec<cair_base::ResolvedLink>,
    /// Live link count observed by the last registrations walk.
    required_links: usize,
    active: [bool; N],
    /// Owned hourly payload staged for direct worker installation. Preserved
    /// across reset and cleared only after the current payload is installed.
    pub(crate) radar_raw_mode: bool,
    /// Owned UnitIntel payload uses the shared division census when available.
    pub(crate) intel_raw_mode: bool,
    /// Raw-copy split gate for the shared hourly bases payload's
    /// registrations half (AirBaseRegistrations). Set by the game thread
    /// every hour with a valid raw payload staged via the shared air copy;
    /// `capture` then skips the typed walk and the decode worker fills the
    /// rows before the ready handoff. Preserved across `reset`, cleared by
    /// the worker.
    pub(crate) registrations_raw_mode: bool,
}

const SKIPPED: Outcome = Outcome::Unavailable {
    reason: replay::cadence::SKIPPED_REASON,
};

impl Captured {
    /// Fail only the Gated job's tables, preserving independent raw verdicts.
    pub(crate) fn fail_gated_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.prepare_hour(hour, force_full);
        let outcomes = TABLES.map(|table| super::failed_capture(table, hour, force_full, reason));
        if !self.radar_raw_mode {
            self.radar.fail(outcomes[0]);
        }
        self.volunteers.fail(outcomes[1]);
        self.sent_volunteers.fail(outcomes[6]);
        self.areas.fail(outcomes[2]);
        self.units.fail(outcomes[3]);
        if !self.intel_raw_mode {
            self.intel.fail(outcomes[4]);
        }
        if !self.registrations_raw_mode {
            self.registrations.fail(outcomes[5]);
        }
    }

    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: [0; N],
            radar_provinces: 0,
            area_provinces: 0,
            wings: 0,
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            radar: CaptureRows::new(capacity.rows[0]),
            radar_provinces: Arena::new(capacity.radar_provinces),
            volunteers: CaptureRows::new(capacity.rows[1]),
            sent_volunteers: CaptureRows::new(capacity.rows[6]),
            areas: CaptureRows::new(capacity.rows[2]),
            area_provinces: Arena::new(capacity.area_provinces),
            units: CaptureRows::new(capacity.rows[3]),
            intel: CaptureRows::new(capacity.rows[4]),
            registrations: CaptureRows::new(capacity.rows[5]),
            wings: Arena::new(capacity.wings),
            resolved: Vec::new(),
            required_links: 0,
            active: [true; N],
            radar_raw_mode: false,
            intel_raw_mode: false,
            registrations_raw_mode: false,
        }
    }

    pub fn reset(&mut self) {
        self.radar.reset();
        self.radar_provinces.reset();
        self.volunteers.reset();
        self.sent_volunteers.reset();
        self.areas.reset();
        self.area_provinces.reset();
        self.units.reset();
        self.intel.reset();
        self.registrations.reset();
        self.wings.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.radar.allocated_bytes()
            + self.radar_provinces.allocated_bytes()
            + self.volunteers.allocated_bytes()
            + self.sent_volunteers.allocated_bytes()
            + self.areas.allocated_bytes()
            + self.area_provinces.allocated_bytes()
            + self.units.allocated_bytes()
            + self.intel.allocated_bytes()
            + self.registrations.allocated_bytes()
            + self.wings.allocated_bytes()
            + self.resolved.capacity() * size_of::<cair_base::ResolvedLink>()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.radar.grow(&mut capacity.rows[0]);
        self.radar_provinces.grow(&mut capacity.radar_provinces);
        self.volunteers.grow(&mut capacity.rows[1]);
        self.sent_volunteers.grow(&mut capacity.rows[6]);
        self.areas.grow(&mut capacity.rows[2]);
        self.area_provinces.grow(&mut capacity.area_provinces);
        self.units.grow(&mut capacity.rows[3]);
        self.intel.grow(&mut capacity.rows[4]);
        // The worker can satisfy this payload's complete demand before the
        // first decode. Propagate that storage to the other pooled frames.
        grow_capacity(&mut capacity.rows[5], self.registrations.rows.len());
        grow_capacity(&mut capacity.wings, self.wings.first.len());
        self.registrations.grow(&mut capacity.rows[5]);
        self.wings.grow(&mut capacity.wings);
        self.grow_resolver(self.required_links);
    }

    /// Size the dense resolver to a live link count. Off the game thread only.
    fn grow_resolver(&mut self, links: usize) {
        if links > self.resolved.len() {
            let _ = resize(&mut self.resolved, links, None).map_err(crate::allocation::io);
        }
    }

    pub(crate) fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        self.active = TABLES.map(|table| replay::cadence::should_walk(table, hour, force_full));
    }

    /// Every walk reads independently. Areas and stationed units share one
    /// theatre traversal.
    /// No allocation, encoding or I/O occurs here; times are raw QPC ticks.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.prepare_hour(hour, force_full);
        self.reset();
        let mut ticks = [0; N];

        // A staged owned payload already contains this hour's native rows.
        // Its acquisition costs are applied by the caller.
        if !self.active[0] {
            self.radar.outcome = SKIPPED;
        } else if self.radar_raw_mode {
            // Refresh payload staged: leave the reset outcome for the decode
            // worker, which installs identical rows and outcomes.
            ticks[0] = 0;
        } else {
            let _reads = read_accounting::table(TableId::RadarCoverage);
            let begin = qpc();
            let first = radar_group::walk(
                memory,
                base,
                game_state,
                &mut self.radar.rows,
                &mut self.radar_provinces.first,
            );
            arena_first(
                &mut self.radar,
                &mut self.radar_provinces,
                first.outcome,
                first.provinces_len,
                first.required_provinces,
            );
            ticks[0] = elapsed(begin);
        }

        let volunteer_reads = read_accounting::table(TableId::VolunteerHosts);
        let begin = qpc();
        self.volunteers.outcome = bounded(
            ccountry::walk_volunteer_hosts(memory, base, game_state, &mut self.volunteers.rows),
            self.volunteers.rows.len(),
        );
        ticks[1] = elapsed(begin);
        drop(volunteer_reads);
        let sent_reads = read_accounting::table(TableId::SentVolunteers);
        let begin = qpc();
        let rows = &mut self.sent_volunteers.rows;
        let len = rows.len();
        self.sent_volunteers.outcome = bounded(
            ccountry::walk_sent_volunteers(memory, base, game_state, rows),
            len,
        );
        ticks[6] = elapsed(begin);
        drop(sent_reads);

        let stationed_slot = if self.active[2] { 2 } else { 3 };
        let area_reads = read_accounting::table(TABLES[stationed_slot]);
        let begin = qpc();
        let first = ctheatre::walk_stationed(
            memory,
            base,
            game_state,
            ctheatre::StationedBuffers {
                areas_active: self.active[2],
                areas: &mut self.areas.rows,
                area_provinces: &mut self.area_provinces.first,
                units: &mut self.units.rows,
            },
        );
        if self.active[2] {
            arena_first(
                &mut self.areas,
                &mut self.area_provinces,
                first.areas,
                first.provinces_len,
                first.required_provinces,
            );
        } else {
            self.areas.outcome = SKIPPED;
        }
        self.units.outcome = bounded(first.units, self.units.rows.len());
        ticks[stationed_slot] = elapsed(begin);
        drop(area_reads);

        // Shared census acquisition already produced this hour's owned rows.
        if self.intel_raw_mode {
            // Refresh payload staged: leave the reset outcome for the decode
            // worker, which installs identical rows and outcomes.
            ticks[4] = 0;
        } else {
            let _reads = read_accounting::table(TableId::UnitIntel);
            let begin = qpc();
            self.intel.outcome = bounded(
                unit_viewer_entry::walk(memory, base, game_state, &mut self.intel.rows),
                self.intel.rows.len(),
            );
            ticks[4] = elapsed(begin);
        }

        // Shared bases payload's registrations half: on hours with a valid
        // raw payload staged via the shared air copy, the decode worker
        // fills the typed rows before the ready handoff, so the typed walk
        // is skipped here.
        if self.registrations_raw_mode {
            // Refresh payload staged: leave the reset outcome for the decode
            // worker, which installs identical rows and outcomes.
            ticks[5] = 0;
        } else {
            let _reads = read_accounting::table(TableId::AirBaseRegistrations);
            let begin = qpc();
            let first = cair_base::walk_registrations(
                memory,
                base,
                game_state,
                cair_base::RegistrationBuffers {
                    registrations: &mut self.registrations.rows,
                    wings: &mut self.wings.first,
                    resolved: &mut self.resolved,
                },
            );
            self.required_links = first.required_links;
            arena_first(
                &mut self.registrations,
                &mut self.wings,
                first.outcome,
                first.wings_len,
                first.required_wings,
            );
            ticks[5] = elapsed(begin);
        }
        ticks
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.radar.outcome,
            self.volunteers.outcome,
            self.areas.outcome,
            self.units.outcome,
            self.intel.outcome,
            self.registrations.outcome,
            self.sent_volunteers.outcome,
        ]
    }

    #[cfg(test)]
    pub fn tables(&self) -> [TableView; N] {
        self.table_inputs(None).map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("no projection supplied"),
        })
    }

    pub(crate) fn table_inputs(
        &self,
        mut projection: Option<&mut crate::projection::Projection<'_>>,
    ) -> [TableInput; N] {
        use crate::projection::scalar_table;
        let outcomes = self.outcomes();
        [
            self.radar_table(outcomes[0], projection.as_deref_mut()),
            scalar_table::<tables::VolunteerHosts, _>(
                projection.as_deref_mut(),
                outcomes[1],
                &self.volunteers.rows,
                |r| (u128::from(r.country) << 32) | u128::from(r.position),
                |r| rows::VolunteerHostRow {
                    country: r.country,
                    position: r.position,
                    host: r.host,
                    deployed_divisions: r.deployed_divisions,
                },
            ),
            self.area_table(outcomes[2], projection.as_deref_mut()),
            scalar_table::<tables::StationedUnits, _>(
                projection.as_deref_mut(),
                outcomes[3],
                &self.units.rows,
                |r| (u128::from(r.theatre) << 32) | u128::from(r.ordinal),
                |r| rows::StationedUnitRow {
                    theatre: r.theatre,
                    ordinal: r.ordinal,
                    unit: r.unit,
                },
            ),
            scalar_table::<tables::UnitIntel, _>(
                projection.as_deref_mut(),
                outcomes[4],
                &self.intel.rows,
                |r| (u128::from(r.unit) << 32) | u128::from(r.ordinal),
                |r| rows::UnitIntelRow {
                    unit: r.unit,
                    ordinal: r.ordinal,
                    viewer: r.viewer,
                    level: r.level,
                },
            ),
            self.registration_table(outcomes[5], projection.as_deref_mut()),
            scalar_table::<tables::SentVolunteers, _>(
                projection,
                outcomes[6],
                &self.sent_volunteers.rows,
                |r| (u128::from(r.country) << 32) | u128::from(r.ordinal),
                |r| rows::SentVolunteerRow {
                    country: r.country,
                    ordinal: r.ordinal,
                    division: r.division,
                },
            ),
        ]
    }

    fn radar_table(
        &self,
        outcome: Outcome,
        projection: Option<&mut crate::projection::Projection<'_>>,
    ) -> TableInput {
        if !valid_arena(outcome, &self.radar.rows, &self.radar_provinces) {
            return crate::spooling::unavailable(
                "radar coverage vector lies outside the captured arena",
            )
            .into();
        }
        let provinces = |r: &radar_group::Row| {
            &self.radar_provinces.first[r.provinces_start..r.provinces_start + r.provinces_len]
        };
        crate::projection::project_table::<tables::RadarCoverage, _>(
            projection,
            outcome,
            &self.radar.rows,
            ArenaRow::key,
            |r, old| {
                r.country == old.country
                    && r.ordinal == old.ordinal
                    && r.state == old.state
                    && provinces(r) == old.provinces
            },
            |r| rows::RadarCoverageRow {
                country: r.country,
                ordinal: r.ordinal,
                state: r.state,
                provinces: owned_vec(copy_vec(provinces(r))),
            },
        )
    }

    fn area_table(
        &self,
        outcome: Outcome,
        projection: Option<&mut crate::projection::Projection<'_>>,
    ) -> TableInput {
        if !valid_arena(outcome, &self.areas.rows, &self.area_provinces) {
            return crate::spooling::unavailable(
                "controller area province vector lies outside the captured arena",
            )
            .into();
        }
        let provinces = |r: &ccontroller_area::Row| {
            &self.area_provinces.first[r.provinces_start..r.provinces_start + r.provinces_len]
        };
        crate::projection::project_table::<tables::TheatreAreas, _>(
            projection,
            outcome,
            &self.areas.rows,
            ArenaRow::key,
            |r, old| {
                r.theatre == old.theatre
                    && r.ordinal == old.ordinal
                    && r.representative == old.representative
                    && provinces(r) == old.provinces
            },
            |r| rows::TheatreAreaRow {
                theatre: r.theatre,
                ordinal: r.ordinal,
                representative: r.representative,
                provinces: owned_vec(copy_vec(provinces(r))),
            },
        )
    }

    fn registration_table(
        &self,
        outcome: Outcome,
        projection: Option<&mut crate::projection::Projection<'_>>,
    ) -> TableInput {
        if !valid_arena(outcome, &self.registrations.rows, &self.wings) {
            return crate::spooling::unavailable(
                "air registration wing vector lies outside the captured arena",
            )
            .into();
        }
        let wings = |r: &cair_base::RegistrationRow| {
            &self.wings.first[r.wings_start..r.wings_start + r.wings_len]
        };
        crate::projection::project_table::<tables::AirBaseRegistrations, _>(
            projection,
            outcome,
            &self.registrations.rows,
            ArenaRow::key,
            |r, old| {
                r.base == old.base
                    && r.slot == old.slot
                    && r.capacity_raw == old.capacity_raw
                    && wings(r) == old.wings
            },
            |r| rows::AirBaseRegistrationRow {
                base: r.base,
                slot: r.slot,
                capacity_raw: r.capacity_raw,
                wings: owned_vec(copy_vec(wings(r))),
            },
        )
    }

    /// Complete an accepted shared-bases failure after the reserved generic
    /// capture has skipped this table. Other fog tables keep their own result.
    pub(crate) fn fail_registrations_capture(&mut self, reason: &'static str) {
        self.registrations.outcome = Outcome::Unavailable { reason };
        self.wings.reset();
        self.registrations_raw_mode = false;
    }

    /// Reserve complete storage from counts already observed by acquisition.
    /// This hint never replaces the owned walk's validation or outcome.
    pub(crate) fn prepare_registrations_payload(&mut self, job: &super::air::AirBasesJob) {
        if !self.registrations_raw_mode || job.log.overflowed || job.log.fault[1] {
            return;
        }
        let Some([rows, wings]) = job.registration_demand else {
            return;
        };
        let mut rows_capacity = self.registrations.rows.len();
        grow_capacity(&mut rows_capacity, rows);
        grow_vec(&mut self.registrations.rows, rows_capacity);
        let mut wings_capacity = self.wings.first.len();
        grow_capacity(&mut wings_capacity, wings);
        grow_vec(&mut self.wings.first, wings_capacity);
    }

    /// Decode-worker interpretation of the shared bases payload's
    /// registrations half: replays `cair_base::walk_registrations` against
    /// the same recorded spans the bases and aces decodes read (see
    /// `super::air::Captured::decode_air_bases_payload`), installing
    /// identical rows, arena bytes and failure reasons. Complete owned demand
    /// grows typed storage before this one walk, avoiding startup truncation.
    pub(crate) fn decode_registrations_payload(
        &mut self,
        base: usize,
        job: &super::air::AirBasesJob,
        decode_ticks: &mut [u64; super::air::AIR_BASES_N],
    ) {
        use crate::raw::Recorded;
        let recorded = Recorded::build(&job.log);
        let started = qpc();
        self.prepare_registrations_payload(job);
        let first = cair_base::walk_registrations(
            &recorded,
            base,
            job.game_state,
            cair_base::RegistrationBuffers {
                registrations: &mut self.registrations.rows,
                wings: &mut self.wings.first,
                resolved: &mut self.resolved,
            },
        );
        self.required_links = first.required_links;
        arena_first(
            &mut self.registrations,
            &mut self.wings,
            first.outcome,
            first.wings_len,
            first.required_wings,
        );
        self.registrations_raw_mode = false;
        decode_ticks[1] += elapsed(started);
    }
}

/// Native registrations owned by the decode worker's single AirBases memo.
/// It never owns a pooled frame or observes game memory.
#[derive(Debug, Default)]
pub(crate) struct RegistrationsMemo {
    rows: Vec<cair_base::RegistrationRow>,
    wings: Vec<u64>,
    required_wings: usize,
}

impl RegistrationsMemo {
    pub(crate) fn capacity_bytes(&self) -> u64 {
        crate::raw_memory::vector_bytes(&self.rows) + crate::raw_memory::vector_bytes(&self.wings)
    }
}

impl Captured {
    pub(crate) fn registrations_memo_context(&self) -> Option<[usize; 2]> {
        self.registrations_raw_mode
            .then_some([self.registrations.rows.len(), self.wings.first.len()])
    }

    pub(crate) fn registrations_memo_complete(&self) -> bool {
        matches!(self.registrations.outcome, Outcome::Rows { .. })
            && valid_arena(
                self.registrations.outcome,
                &self.registrations.rows,
                &self.wings,
            )
    }

    pub(crate) fn save_registrations_memo(&self, memo: &mut RegistrationsMemo) {
        debug_assert!(self.registrations_memo_complete());
        memo.rows.clear();
        memo.wings.clear();
        if reserve(&mut memo.rows, self.registrations.outcome.len()).is_err()
            || reserve(&mut memo.wings, self.wings.len).is_err()
        {
            return;
        }
        memo.rows
            .extend_from_slice(&self.registrations.rows[..self.registrations.outcome.len()]);
        memo.wings
            .extend_from_slice(&self.wings.first[..self.wings.len]);
        memo.required_wings = self.wings.required;
    }

    pub(crate) fn restore_registrations_memo(&mut self, memo: &RegistrationsMemo) {
        // The caller has compared the complete invocation capacity fingerprint
        // before restoring either half. No partial family restore is possible.
        self.registrations.rows[..memo.rows.len()].copy_from_slice(&memo.rows);
        self.registrations.outcome = Outcome::Rows {
            len: memo.rows.len(),
        };
        self.wings.first[..memo.wings.len()].copy_from_slice(&memo.wings);
        self.wings.len = memo.wings.len();
        self.wings.required = memo.required_wings;
        self.registrations_raw_mode = false;
    }
}

#[cfg(test)]
mod registrations_memo_tests;

#[cfg(test)]
pub(crate) struct RegistrationsSnapshot {
    pub outcome: Outcome,
    pub rows: Vec<cair_base::RegistrationRow>,
    pub wings: Vec<u64>,
}

#[cfg(test)]
impl Captured {
    /// Typed direct collection for the shared bases copy/decode harness:
    /// runs `walk_registrations` into caller-sized buffers with the same
    /// arena handling the game thread applies.
    pub(crate) fn collect_registrations_direct(
        rows_cap: usize,
        wings_cap: usize,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
    ) -> RegistrationsSnapshot {
        let capacities = Capacities {
            rows: [4, 4, 4, 4, 4, rows_cap, 4],
            radar_provinces: 8,
            area_provinces: 8,
            wings: wings_cap,
        };
        let mut captured = Captured::with_capacities(&capacities);
        let first = cair_base::walk_registrations(
            memory,
            base,
            game_state,
            cair_base::RegistrationBuffers {
                registrations: &mut captured.registrations.rows,
                wings: &mut captured.wings.first,
                resolved: &mut captured.resolved,
            },
        );
        arena_first(
            &mut captured.registrations,
            &mut captured.wings,
            first.outcome,
            first.wings_len,
            first.required_wings,
        );
        captured.registrations_snapshot()
    }

    /// Dense resolver length and the live link demand last observed.
    pub(crate) fn resolver_sizes(&self) -> (usize, usize) {
        (self.resolved.len(), self.required_links)
    }

    /// Snapshot of the decoded registrations half for harness comparison.
    pub(crate) fn registrations_snapshot(&self) -> RegistrationsSnapshot {
        RegistrationsSnapshot {
            outcome: self.registrations.outcome,
            rows: self.registrations.rows[..self.registrations.outcome.len()].to_vec(),
            wings: self.wings.first[..self.wings.len].to_vec(),
        }
    }
}

/// A row whose payload is a span of a shared caller-owned arena.
trait ArenaRow<T>: Default + Clone + Copy + PartialEq {
    fn key(&self) -> u128;
    fn span(&self) -> (usize, usize);
}

impl ArenaRow<u32> for radar_group::Row {
    fn key(&self) -> u128 {
        (u128::from(self.country) << 32) | u128::from(self.ordinal)
    }
    fn span(&self) -> (usize, usize) {
        (self.provinces_start, self.provinces_len)
    }
}

impl ArenaRow<u32> for ccontroller_area::Row {
    fn key(&self) -> u128 {
        (u128::from(self.theatre) << 32) | u128::from(self.ordinal)
    }
    fn span(&self) -> (usize, usize) {
        (self.provinces_start, self.provinces_len)
    }
}

impl ArenaRow<u64> for cair_base::RegistrationRow {
    fn key(&self) -> u128 {
        (u128::from(self.base) << 32) | u128::from(self.slot)
    }
    fn span(&self) -> (usize, usize) {
        (self.wings_start, self.wings_len)
    }
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

fn valid_arena<R: ArenaRow<T>, T>(outcome: Outcome, rows: &[R], arena: &Arena<T>) -> bool {
    arena.len <= arena.first.len()
        && arena.required >= arena.len
        && rows.get(..outcome.len()).is_some_and(|rows| {
            rows.iter().all(|row| {
                let (start, len) = row.span();
                start.checked_add(len).is_some_and(|end| end <= arena.len)
            })
        })
}

fn arena_outcome<R: ArenaRow<T>, T>(
    outcome: Outcome,
    rows: &[R],
    buffer: usize,
    len: usize,
    required: usize,
) -> Outcome {
    let outcome = bounded(outcome, rows.len());
    // This pass reported `len` used elements of a `buffer`-element arena; every
    // published span must lie inside them and the demand can never be smaller.
    if len <= buffer
        && required >= len
        && rows.get(..outcome.len()).is_some_and(|rows| {
            rows.iter().all(|row| {
                let (start, span) = row.span();
                start.checked_add(span).is_some_and(|end| end <= len)
            })
        })
    {
        outcome
    } else {
        Outcome::Unavailable {
            reason: "captured arena range invalid",
        }
    }
}

fn arena_first<R: ArenaRow<T>, T: Default + Clone + PartialEq>(
    captured: &mut CaptureRows<R>,
    arena: &mut Arena<T>,
    outcome: Outcome,
    len: usize,
    required: usize,
) {
    captured.outcome = arena_outcome(outcome, &captured.rows, arena.first.len(), len, required);
    arena.len = len;
    arena.required = required;
}

pub(crate) const RADAR_N: usize = 1;
pub(crate) const INTEL_N: usize = 1;

mod owned;

pub(crate) use owned::{
    IntelCopyContext, IntelCopyReport, OwnedFogJob, RadarCopyContext, RadarCopyReport,
    copy_intel_refresh_with_divisions, copy_radar_refresh,
};

pub(crate) type RadarJob = OwnedFogJob<radar_group::Row>;
pub(crate) type IntelJob = OwnedFogJob<unit_viewer_entry::Row>;

#[cfg(test)]
mod tests;

/// Shared synthetic address space for the radar copy/decode harness: one
/// country with one radar group covering two provinces. Mirrors the engine
/// radar fixture shapes.
#[cfg(test)]
pub(crate) mod radar_test_kit;

/// Shared synthetic address space for the intel copy/decode harness: one
/// country with one division (two viewer entries) and one task force (no
/// entries). Mirrors the engine viewer-entry fixture shapes.
#[cfg(test)]
pub(crate) mod intel_test_kit;
