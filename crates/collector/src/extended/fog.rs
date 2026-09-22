//! Native fog-derivation input acquisition (registry domain D102). Only growth
//! and replay conversion allocate; every engine read finishes before the callback returns.

use crate::{
    read_accounting,
    sampling::qpc,
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
};
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

pub const N: usize = 6;
pub const TABLES: [TableId; N] = [
    TableId::RadarCoverage,
    TableId::VolunteerHosts,
    TableId::TheatreAreas,
    TableId::StationedUnits,
    TableId::UnitIntel,
    TableId::AirBaseRegistrations,
];

/// The stationed walk censuses areas and units together, so its cost is charged
/// once to TheatreAreas. The other four families have independent roots.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    ("radar coverage groups", 0, &[0]),
    ("volunteer host vectors", 1, &[1]),
    ("volunteer theatre areas and stationed units", 2, &[2, 3]),
    ("unit viewer levels", 4, &[4]),
    ("air base registrations", 5, &[5]),
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
            rows: [1024, 512, 512, 1024, 32_768, 2048],
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
    areas: CaptureRows<ccontroller_area::Row>,
    area_provinces: Arena<u32>,
    units: CaptureRows<ctheatre::StationedUnitRow>,
    intel: CaptureRows<unit_viewer_entry::Row>,
    registrations: CaptureRows<cair_base::RegistrationRow>,
    wings: Arena<u64>,
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
            areas: CaptureRows::new(capacity.rows[2]),
            area_provinces: Arena::new(capacity.area_provinces),
            units: CaptureRows::new(capacity.rows[3]),
            intel: CaptureRows::new(capacity.rows[4]),
            registrations: CaptureRows::new(capacity.rows[5]),
            wings: Arena::new(capacity.wings),
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
            + self.areas.allocated_bytes()
            + self.area_provinces.allocated_bytes()
            + self.units.allocated_bytes()
            + self.intel.allocated_bytes()
            + self.registrations.allocated_bytes()
            + self.wings.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.radar.grow(&mut capacity.rows[0]);
        self.radar_provinces.grow(&mut capacity.radar_provinces);
        self.volunteers.grow(&mut capacity.rows[1]);
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
                },
            );
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
            self.registration_table(outcomes[5], projection),
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
                provinces: provinces(r).to_vec(),
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
                provinces: provinces(r).to_vec(),
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
                    && r.country == old.country
                    && wings(r) == old.wings
            },
            |r| rows::AirBaseRegistrationRow {
                base: r.base,
                slot: r.slot,
                country: r.country,
                wings: wings(r).to_vec(),
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
            },
        );
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
        memo.rows
            .extend_from_slice(&self.registrations.rows[..self.registrations.outcome.len()]);
        memo.wings.clear();
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
mod registrations_memo_tests {
    use super::*;

    fn captured() -> Captured {
        Captured::with_capacities(&Capacities {
            rows: [0, 0, 0, 0, 0, 4],
            radar_provinces: 0,
            area_provinces: 0,
            wings: 8,
        })
    }

    #[test]
    fn registrations_memo_validates_ranges_and_preserves_required_capacity() {
        let mut source = captured();
        source.registrations_raw_mode = true;
        source.registrations.outcome = Outcome::Rows { len: 1 };
        source.registrations.rows[0] = cair_base::RegistrationRow {
            base: 12,
            slot: 3,
            country: 7,
            wings_start: 0,
            wings_len: 2,
        };
        source.wings.first[..2].copy_from_slice(&[100, 200]);
        source.wings.len = 2;
        source.wings.required = 5;
        assert!(source.registrations_memo_complete());
        let mut memo = RegistrationsMemo::default();
        source.save_registrations_memo(&mut memo);
        let mut target = captured();
        target.registrations_raw_mode = true;
        assert_eq!(target.registrations_memo_context(), Some([4, 8]));
        target.restore_registrations_memo(&memo);
        assert_eq!(target.registrations.outcome, Outcome::Rows { len: 1 });
        assert_eq!(target.registrations.rows[0], source.registrations.rows[0]);
        assert_eq!(target.wings.len, 2);
        assert_eq!(target.wings.required, 5);
        assert_eq!(&target.wings.first[..2], &[100, 200]);
        assert!(!target.registrations_raw_mode);
        source.registrations.rows[0].wings_start = usize::MAX;
        assert!(!source.registrations_memo_complete());
        source.registrations.rows[0].wings_start = 0;
        source.wings.required = 1;
        assert!(!source.registrations_memo_complete());
        source.wings.required = 5;
        source.wings.len = 9;
        assert!(!source.registrations_memo_complete());
    }

    #[test]
    fn registration_demand_growth_is_accounted_and_propagated_to_pooled_capacities() {
        use crate::extended::air::air_bases_test_kit as kit;
        let bytes = kit::fixture();
        let (_, _, job) = kit::split(&kit::memory(&bytes));
        for initial in [0, 1] {
            let mut capacities = Capacities {
                rows: [0, 0, 0, 0, 0, initial],
                radar_provinces: 0,
                area_provinces: 0,
                wings: initial,
            };
            let mut captured = Captured::with_capacities(&capacities);
            captured.registrations_raw_mode = true;
            let before = captured.allocated_bytes();
            captured.decode_registrations_payload(kit::BASE, &job, &mut [0; 3]);
            assert_eq!(captured.registrations.outcome, Outcome::Rows { len: 2 });
            assert_eq!((captured.wings.len, captured.wings.required), (2, 2));
            assert!(captured.allocated_bytes() > before);
            assert_eq!(
                captured.allocated_bytes(),
                captured.registrations.rows.capacity() * size_of::<cair_base::RegistrationRow>()
                    + captured.wings.first.capacity() * size_of::<u64>()
            );
            captured.grow(&mut capacities);
            assert_eq!(capacities.rows[5], captured.registrations.rows.len());
            assert_eq!(capacities.wings, captured.wings.first.len());
            assert!(capacities.rows[5] >= 2 && capacities.wings >= 2);
            let next = Captured::with_capacities(&capacities);
            assert_eq!(
                next.registrations.rows.len(),
                captured.registrations.rows.len()
            );
            assert_eq!(next.wings.first.len(), captured.wings.first.len());
        }
    }
}

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
            rows: [4, 4, 4, 4, 4, rows_cap],
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

fn bounded(outcome: Outcome, capacity: usize) -> Outcome {
    if outcome.len() > capacity {
        Outcome::Unavailable {
            reason: "walker row count exceeds allocated buffer",
        }
    } else if matches!(outcome, Outcome::Truncated { unread_rows: 0, .. }) {
        Outcome::Unavailable {
            reason: "walker marked truncation without missing rows",
        }
    } else {
        outcome
    }
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

#[derive(Debug)]
pub(crate) struct OwnedFogJob<R> {
    rows: Vec<R>,
    provinces: Vec<u32>,
    outcome: Outcome,
    provinces_len: usize,
    required_provinces: usize,
    row_limit: usize,
    province_limit: usize,
    acquired: bool,
    active: bool,
    raw_bytes: u64,
    capacity_charge: crate::raw_memory::Charge,
}

pub(crate) type RadarJob = OwnedFogJob<radar_group::Row>;
pub(crate) type IntelJob = OwnedFogJob<unit_viewer_entry::Row>;

impl<R: Default + Copy> Default for OwnedFogJob<R> {
    fn default() -> Self {
        Self::with_storage(0, 0)
    }
}

impl<R: Default + Copy> OwnedFogJob<R> {
    fn with_storage(rows: usize, provinces: usize) -> Self {
        let mut job = Self {
            rows: vec![R::default(); rows],
            provinces: vec![0; provinces],
            outcome: Outcome::Rows { len: 0 },
            provinces_len: 0,
            required_provinces: 0,
            row_limit: 0,
            province_limit: 0,
            acquired: false,
            active: false,
            raw_bytes: 0,
            capacity_charge: Default::default(),
        };
        job.update_capacity_charge();
        job
    }
    fn update_capacity_charge(&mut self) {
        self.capacity_charge.set(crate::raw_memory::Capacity {
            payload_vectors: crate::raw_memory::vector_bytes(&self.rows)
                + crate::raw_memory::vector_bytes(&self.provinces),
            ..Default::default()
        });
    }
    pub fn clear(&mut self) {
        self.outcome = Outcome::Rows { len: 0 };
        self.provinces_len = 0;
        self.required_provinces = 0;
        self.row_limit = 0;
        self.province_limit = 0;
        self.acquired = false;
        self.active = false;
        self.raw_bytes = 0;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * size_of::<R>() + self.provinces.capacity() * size_of::<u32>()
    }
    pub fn grow_for_next(&self) -> bool {
        self.row_limit > self.rows.len() || self.province_limit > self.provinces.len()
    }
    /// Called only during worker-side recycling.
    pub fn grow(&mut self) {
        if self.row_limit > self.rows.len() {
            self.rows.resize(self.row_limit, R::default());
        }
        if self.province_limit > self.provinces.len() {
            self.provinces.resize(self.province_limit, 0);
        }
        self.update_capacity_charge();
    }
    pub fn note_growth(&self) {
        if self.grow_for_next() {
            crate::raw::publish_growth(
                std::any::type_name::<R>(),
                serde_json::json!({
                    "row_capacity": self.rows.len(), "required_rows": self.row_limit,
                    "province_capacity": self.provinces.len(), "required_provinces": self.province_limit,
                }),
            );
        }
    }
    fn prepare(&mut self, rows: usize, provinces: usize) {
        self.clear();
        self.active = true;
        self.row_limit = rows;
        self.province_limit = provinces;
    }
    fn fits(&self, rows: usize, provinces: usize) -> bool {
        rows <= self.rows.len() && provinces <= self.provinces.len()
    }
}
impl RadarJob {
    pub fn new() -> Self {
        Self::with_storage(1024, 65_536)
    }
}
impl IntelJob {
    pub fn new() -> Self {
        Self::with_storage(32_768, 0)
    }
}

/// Count successful bytes while preserving the native guarded batch operations.
struct FogMemory<'a, M> {
    inner: &'a M,
    bytes: std::cell::Cell<u64>,
}
impl<M: Memory> FogMemory<'_, M> {
    fn count(&self, bytes: usize) {
        self.bytes
            .set(self.bytes.get().saturating_add(bytes as u64));
    }
}
impl<M: Memory> Memory for FogMemory<'_, M> {
    fn read_scalar<T: engine::memory::Raw>(&self, address: usize) -> Option<T> {
        let value = self.inner.read_scalar::<T>(address);
        if value.is_some() {
            self.count(size_of::<T>());
        }
        value
    }
    fn authenticated_countries(
        &self,
        base: usize,
        state: usize,
    ) -> Option<ccountry::AuthenticatedRoster<'_>> {
        self.inner.authenticated_countries(base, state)
    }
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let succeeded = self.inner.copy(address, destination);
        if succeeded {
            self.count(destination.len());
        }
        succeeded
    }
    fn copy_scattered<const K: usize>(
        &self,
        addresses: [usize; K],
        destinations: &mut [&mut [u8]; K],
    ) -> usize {
        let completed = self.inner.copy_scattered(addresses, destinations);
        for field in destinations.iter().take(completed) {
            self.count(field.len());
        }
        completed
    }
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let completed = self.inner.copy_records(sources, layout, destination);
        self.count(layout.completed_bytes(completed));
        completed
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct RadarCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; RADAR_N],
    pub raw_bytes: [u64; RADAR_N],
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct RadarCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    pub row_limit: usize,
    pub province_limit: usize,
}
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct IntelCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; INTEL_N],
    pub raw_bytes: [u64; INTEL_N],
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct IntelCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub row_limit: usize,
}
fn acquire_radar(job: &mut RadarJob, memory: &impl Memory, ctx: RadarCopyContext) {
    let _reads = read_accounting::table(TableId::RadarCoverage);
    let source = FogMemory {
        inner: memory,
        bytes: std::cell::Cell::new(0),
    };
    let result = radar_group::walk(
        &source,
        ctx.base,
        ctx.game_state,
        &mut job.rows[..job.row_limit],
        &mut job.provinces[..job.province_limit],
    );
    job.outcome = result.outcome;
    job.provinces_len = result.provinces_len;
    job.required_provinces = result.required_provinces;
    job.raw_bytes = source.bytes.get();
    job.acquired = true;
}
fn acquire_intel(
    job: &mut IntelJob,
    memory: &impl Memory,
    ctx: IntelCopyContext,
    divisions: Option<unit_viewer_entry::DivisionSources<'_>>,
) {
    let _reads = read_accounting::table(TableId::UnitIntel);
    let source = FogMemory {
        inner: memory,
        bytes: std::cell::Cell::new(0),
    };
    job.outcome = unit_viewer_entry::walk_with_divisions(
        &source,
        ctx.base,
        ctx.game_state,
        &mut job.rows[..job.row_limit],
        divisions,
    );
    job.raw_bytes = source.bytes.get();
    job.acquired = true;
}
/// Check destination capacities before acquiring one owned radar snapshot.
pub(crate) fn copy_radar_refresh(
    memory: &impl Memory,
    ctx: RadarCopyContext,
    job: &mut RadarJob,
) -> RadarCopyReport {
    if !replay::cadence::should_walk(TableId::RadarCoverage, ctx.hour, ctx.force_full) {
        job.clear();
        return RadarCopyReport::default();
    }
    let started = qpc();
    job.prepare(ctx.row_limit, ctx.province_limit);
    if job.fits(ctx.row_limit, ctx.province_limit) {
        acquire_radar(job, memory, ctx);
    }
    RadarCopyReport {
        raw_ok: job.acquired,
        copy_ticks: [elapsed(started)],
        raw_bytes: [job.raw_bytes],
    }
}
/// The caller retains the primary division census until acquisition returns.
pub(crate) fn copy_intel_refresh_with_divisions(
    memory: &impl Memory,
    ctx: IntelCopyContext,
    job: &mut IntelJob,
    divisions: Option<unit_viewer_entry::DivisionSources<'_>>,
) -> IntelCopyReport {
    let started = qpc();
    job.prepare(ctx.row_limit, 0);
    if job.fits(ctx.row_limit, 0) {
        acquire_intel(job, memory, ctx, divisions);
    }
    IntelCopyReport {
        raw_ok: job.acquired,
        copy_ticks: [elapsed(started)],
        raw_bytes: [job.raw_bytes],
    }
}

impl Captured {
    pub(crate) fn radar_copy_context(
        &self,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> RadarCopyContext {
        RadarCopyContext {
            base,
            game_state,
            hour,
            force_full,
            row_limit: self.radar.rows.len(),
            province_limit: self.radar_provinces.first.len(),
        }
    }

    pub(crate) fn intel_copy_context(&self, base: usize, game_state: usize) -> IntelCopyContext {
        IntelCopyContext {
            base,
            game_state,
            row_limit: self.intel.rows.len(),
        }
    }

    /// Complete an accepted worker failure after the sibling capture has
    /// finished. The reservation gate prevented any synchronous retry; no
    /// owned payload from this failed acquisition may subsequently be decoded.
    pub(crate) fn fail_radar_capture(&mut self, reason: &'static str) {
        self.radar.outcome = if self.active[0] {
            Outcome::Unavailable { reason }
        } else {
            SKIPPED
        };
        self.radar_provinces.reset();
        self.radar_raw_mode = false;
    }

    pub(crate) fn fail_intel_capture(&mut self, reason: &'static str) {
        self.intel.outcome = Outcome::Unavailable { reason };
        self.intel_raw_mode = false;
    }

    pub(crate) fn decode_radar_payload(
        &mut self,
        job: &RadarJob,
        decode_ticks: &mut [u64; RADAR_N],
    ) {
        self.active[0] = job.active;
        if !job.active {
            self.fail_radar_capture(replay::cadence::SKIPPED_REASON);
            return;
        }
        let started = qpc();
        if !job.acquired
            || job.row_limit != self.radar.rows.len()
            || job.province_limit != self.radar_provinces.first.len()
        {
            self.radar.outcome = Outcome::Unavailable {
                reason: "radar owned payload capacity mismatch",
            };
        } else {
            let len = job.outcome.len();
            self.radar.rows[..len].copy_from_slice(&job.rows[..len]);
            self.radar_provinces.first[..job.provinces_len]
                .copy_from_slice(&job.provinces[..job.provinces_len]);
            arena_first(
                &mut self.radar,
                &mut self.radar_provinces,
                job.outcome,
                job.provinces_len,
                job.required_provinces,
            );
        }
        decode_ticks[0] += elapsed(started);
        self.radar_raw_mode = false;
    }

    pub(crate) fn decode_intel_payload(
        &mut self,
        job: &IntelJob,
        decode_ticks: &mut [u64; INTEL_N],
    ) {
        let started = qpc();
        if !job.acquired || job.row_limit != self.intel.rows.len() {
            self.intel.outcome = Outcome::Unavailable {
                reason: "intel owned payload capacity mismatch",
            };
        } else {
            let len = job.outcome.len();
            self.intel.rows[..len].copy_from_slice(&job.rows[..len]);
            self.intel.outcome = job.outcome;
        }
        decode_ticks[0] += elapsed(started);
        self.intel_raw_mode = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::{
        carmy, cfleet, cgame_state, cprovince, cstate, ctask_force, cunit, memory::SliceMemory,
    };
    use replay::rows::TableSchema;

    const BASE: usize = 0x10000;
    const COUNTRY: usize = 0x11000;
    const RADAR_GROUP: usize = 0x12000;
    const RADAR_PROVINCES: usize = 0x12100;
    const STATE_OBJECT: usize = 0x12200;
    const HOSTS: usize = 0x12300;
    const TALLIES: usize = 0x12340;
    const THEATRE: usize = 0x13000;
    const AREA: usize = 0x13400;
    const PROVINCE_A: usize = 0x13600;
    const PROVINCE_B: usize = 0x13700;
    const DIVISION: usize = 0x14000;
    const FLEET: usize = 0x14400;
    const TASK_FORCE: usize = 0x14800;
    const VIEWERS: usize = 0x14c00;
    const MANAGER: usize = 0x15000;
    const AIR_BASE: usize = 0x15400;
    const CONTAINER: usize = 0x15800;
    const POOL: usize = 0x15c00;
    const WING: usize = 0x16000;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address - BASE..address - BASE + value.len()].copy_from_slice(value);
    }

    fn array(bytes: &mut [u8], address: usize, data: usize, count: u32) {
        put(bytes, address, &(data as u64).to_le_bytes());
        put(bytes, address + 8, &count.to_le_bytes());
        put(bytes, address + 12, &count.to_le_bytes());
    }

    /// One country with one radar group, one volunteer host, one volunteer
    /// theatre holding one area and one stationed unit, one division with two
    /// viewer entries, one task force with none, and one registered air base.
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x8000];
        array(&mut bytes, BASE + cgame_state::COUNTRIES, 0x10800, 2);
        put(&mut bytes, 0x10808, &(COUNTRY as u64).to_le_bytes());
        array(&mut bytes, BASE + cgame_state::COUNTRY_LINKS, 0x10900, 3);
        put(&mut bytes, 0x10904, &1_u32.to_le_bytes());
        put(&mut bytes, 0x10908, &1_u32.to_le_bytes());
        put(
            &mut bytes,
            COUNTRY,
            &(ccountry::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, COUNTRY + ccountry::ID, &2_u32.to_le_bytes());
        put(
            &mut bytes,
            COUNTRY + ccountry::RADARS_POOL,
            &(engine::cradars_pool::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            COUNTRY + ccountry::RADARS_POOL_COUNTRY,
            &2_u32.to_le_bytes(),
        );
        array(&mut bytes, COUNTRY + ccountry::RADAR_GROUPS, 0x10a00, 1);
        put(&mut bytes, 0x10a00, &(RADAR_GROUP as u64).to_le_bytes());
        put(
            &mut bytes,
            RADAR_GROUP + radar_group::PARENT,
            &((COUNTRY + ccountry::RADARS_POOL) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            RADAR_GROUP + radar_group::STATE,
            &(STATE_OBJECT as u64).to_le_bytes(),
        );
        array(
            &mut bytes,
            RADAR_GROUP + radar_group::PROVINCES,
            RADAR_PROVINCES,
            2,
        );
        put(&mut bytes, RADAR_PROVINCES, &11_u32.to_le_bytes());
        put(&mut bytes, RADAR_PROVINCES + 4, &12_u32.to_le_bytes());
        put(
            &mut bytes,
            STATE_OBJECT,
            &(cstate::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, STATE_OBJECT + cstate::ID, &33_u32.to_le_bytes());
        array(&mut bytes, COUNTRY + ccountry::VOLUNTEER_HOSTS, HOSTS, 1);
        array(
            &mut bytes,
            COUNTRY + ccountry::VOLUNTEER_TALLIES,
            TALLIES,
            1,
        );
        put(&mut bytes, HOSTS, &9_u32.to_le_bytes());
        put(&mut bytes, TALLIES, &(-3_i32).to_le_bytes());
        array(&mut bytes, COUNTRY + ccountry::THEATRES, 0x10b00, 1);
        put(&mut bytes, 0x10b00, &(THEATRE as u64).to_le_bytes());
        put(
            &mut bytes,
            THEATRE,
            &(ctheatre::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            THEATRE + ctheatre::HANDLE,
            &((5_u64 << 32) | u64::from(ctheatre::REFERENCE_TYPE)).to_le_bytes(),
        );
        put(
            &mut bytes,
            THEATRE + ctheatre::VOLUNTEERS_THEATRE,
            &4_i32.to_le_bytes(),
        );
        array(
            &mut bytes,
            THEATRE + ctheatre::CONTROLLER_AREA_DATA,
            0x10c00,
            1,
        );
        put(&mut bytes, 0x10c00, &(AREA as u64).to_le_bytes());
        put(
            &mut bytes,
            AREA,
            &(ccontroller_area::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            AREA + ccontroller_area::REPRESENTATIVE_PROVINCE,
            &(PROVINCE_A as u64).to_le_bytes(),
        );
        array(
            &mut bytes,
            AREA + ccontroller_area::PROVINCE_DATA,
            0x10d00,
            2,
        );
        put(&mut bytes, 0x10d00, &(PROVINCE_A as u64).to_le_bytes());
        put(&mut bytes, 0x10d08, &(PROVINCE_B as u64).to_le_bytes());
        for (province, id) in [(PROVINCE_A, 21_u32), (PROVINCE_B, 22)] {
            put(
                &mut bytes,
                province,
                &(cprovince::VTABLE_RVA as u64).to_le_bytes(),
            );
            put(&mut bytes, province + cprovince::ID, &id.to_le_bytes());
        }
        array(
            &mut bytes,
            THEATRE + ctheatre::STATIONED_UNIT_DATA,
            0x10e00,
            1,
        );
        put(&mut bytes, 0x10e00, &(DIVISION as u64).to_le_bytes());
        array(&mut bytes, COUNTRY + ccountry::DIVISIONS, 0x10f00, 1);
        put(&mut bytes, 0x10f00, &(DIVISION as u64).to_le_bytes());
        put(
            &mut bytes,
            DIVISION,
            &(carmy::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            DIVISION + cunit::HANDLE,
            &((6_u64 << 32) | 1).to_le_bytes(),
        );
        array(&mut bytes, DIVISION + cunit::VIEWER_DATA, VIEWERS, 2);
        for (index, (viewer, level)) in [(1_u32, 3_u8), (2, 255)].into_iter().enumerate() {
            put(
                &mut bytes,
                VIEWERS + index * unit_viewer_entry::STRIDE,
                &viewer.to_le_bytes(),
            );
            put(
                &mut bytes,
                VIEWERS + index * unit_viewer_entry::STRIDE + unit_viewer_entry::LEVEL,
                &[level],
            );
        }
        array(&mut bytes, COUNTRY + ccountry::FLEETS, 0x11400, 1);
        put(&mut bytes, 0x11400, &(FLEET as u64).to_le_bytes());
        put(
            &mut bytes,
            FLEET,
            &(cfleet::VTABLE_RVA as u64).to_le_bytes(),
        );
        array(&mut bytes, FLEET + cfleet::TASK_FORCES, 0x11500, 1);
        put(&mut bytes, 0x11500, &(TASK_FORCE as u64).to_le_bytes());
        put(
            &mut bytes,
            TASK_FORCE,
            &(ctask_force::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            TASK_FORCE + cunit::HANDLE,
            &((8_u64 << 32) | 61).to_le_bytes(),
        );
        put(
            &mut bytes,
            BASE + cgame_state::STRATEGIC_AIR_MANAGER,
            &(MANAGER as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            MANAGER,
            &(engine::cstrategic_air_manager::VTABLE_RVA as u64).to_le_bytes(),
        );
        array(
            &mut bytes,
            MANAGER + engine::cstrategic_air_manager::STATE_AIR_BASES,
            0x11600,
            1,
        );
        put(&mut bytes, 0x11600, &(AIR_BASE as u64).to_le_bytes());
        put(
            &mut bytes,
            AIR_BASE,
            &(cair_base::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            AIR_BASE + cair_base::HANDLE,
            &((4_u64 << 32) | u64::from(cair_base::TYPE)).to_le_bytes(),
        );
        array(
            &mut bytes,
            AIR_BASE + cair_base::COUNTRY_CONTAINERS,
            0x11700,
            2,
        );
        put(&mut bytes, 0x11708, &(CONTAINER as u64).to_le_bytes());
        put(
            &mut bytes,
            CONTAINER,
            &(engine::ccountry_air_container::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            CONTAINER + engine::ccountry_air_container::COUNTRY,
            &2_u32.to_le_bytes(),
        );
        array(
            &mut bytes,
            CONTAINER + engine::ccountry_air_container::POOLS,
            0x11800,
            1,
        );
        put(&mut bytes, 0x11800, &(POOL as u64).to_le_bytes());
        put(
            &mut bytes,
            POOL,
            &(engine::cair_wing_pool::VTABLE_RVA as u64).to_le_bytes(),
        );
        array(&mut bytes, POOL + engine::cair_wing_pool::WINGS, 0x11900, 1);
        put(&mut bytes, 0x11900, &(WING as u64).to_le_bytes());
        put(
            &mut bytes,
            WING,
            &(engine::cair_wing::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            WING + engine::cair_wing::SECONDARY,
            &(engine::cair_wing::SECONDARY_VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            WING + engine::cair_wing::REFERENCE_TYPE,
            &((77_u64 << 32) | u64::from(engine::cair_wing::TYPE)).to_le_bytes(),
        );
        bytes
    }

    fn small() -> (Capacities, Captured) {
        let capacities = Capacities {
            rows: [4; N],
            radar_provinces: 8,
            area_provinces: 8,
            wings: 8,
        };
        let captured = Captured::with_capacities(&capacities);
        (capacities, captured)
    }

    fn complete<S: TableSchema>(view: &TableView) -> &[S::Row] {
        let TableView::Complete(rows) = view else {
            panic!("expected complete table, found {view:?}")
        };
        rows.rows::<S>().unwrap()
    }

    #[test]
    fn every_family_keeps_native_order_widths_and_absent_values() {
        let (_, mut captured) = small();
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let _ticks = captured.capture(&memory, 0, BASE, 0, true);
        let views = captured.tables();
        assert_eq!(
            complete::<tables::RadarCoverage>(&views[0]),
            &[rows::RadarCoverageRow {
                country: 2,
                ordinal: 0,
                state: Some(33),
                provinces: vec![11, 12],
            }]
        );
        assert_eq!(
            complete::<tables::VolunteerHosts>(&views[1]),
            &[rows::VolunteerHostRow {
                country: 2,
                position: 0,
                host: 9,
                deployed_divisions: -3,
            }]
        );
        assert_eq!(
            complete::<tables::TheatreAreas>(&views[2]),
            &[rows::TheatreAreaRow {
                theatre: (5 << 32) | u64::from(ctheatre::REFERENCE_TYPE),
                ordinal: 0,
                representative: Some(21),
                provinces: vec![21, 22],
            }]
        );
        assert_eq!(
            complete::<tables::StationedUnits>(&views[3]),
            &[rows::StationedUnitRow {
                theatre: (5 << 32) | u64::from(ctheatre::REFERENCE_TYPE),
                ordinal: 0,
                unit: (6 << 32) | 1,
            }]
        );
        let intel = complete::<tables::UnitIntel>(&views[4]);
        assert_eq!(intel.len(), 2);
        assert_eq!(
            (
                intel[0].viewer,
                intel[0].level,
                intel[1].viewer,
                intel[1].level
            ),
            (1, 3, 2, -1)
        );
        assert_eq!(
            complete::<tables::AirBaseRegistrations>(&views[5]),
            &[rows::AirBaseRegistrationRow {
                base: (4 << 32) | u64::from(cair_base::TYPE),
                slot: 1,
                country: 2,
                wings: vec![(77 << 32) | u64::from(engine::cair_wing::TYPE)],
            }]
        );
    }

    #[test]
    fn arena_pressure_truncates_and_growth_restores_the_full_census() {
        let mut capacities = Capacities {
            rows: [4; N],
            radar_provinces: 1,
            area_provinces: 8,
            wings: 8,
        };
        let mut captured = Captured::with_capacities(&capacities);
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let _ticks = captured.capture(&memory, 0, BASE, 0, true);
        assert_eq!(
            captured.outcomes()[0],
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        assert!(matches!(
            captured.tables()[0],
            TableView::Truncated { missing: 1, .. }
        ));
        captured.grow(&mut capacities);
        assert!(capacities.radar_provinces >= 2);
        let _ticks = captured.capture(&memory, 0, BASE, 0, true);
        assert_eq!(captured.outcomes()[0], Outcome::Rows { len: 1 });
    }

    #[test]
    fn torn_sources_stay_unavailable_and_never_become_empty_tables() {
        let (_, mut captured) = small();
        let mut bytes = fixture();
        // A null inside the counted stationed-unit vector is a torn read.
        put(&mut bytes, 0x10e00, &0_u64.to_le_bytes());
        // A radar group whose parent record moved cannot be this country's.
        put(
            &mut bytes,
            RADAR_GROUP + radar_group::PARENT,
            &0x1234_u64.to_le_bytes(),
        );
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let _ticks = captured.capture(&memory, 0, BASE, 0, true);
        let outcomes = captured.outcomes();
        assert!(matches!(outcomes[0], Outcome::Unavailable { .. }));
        assert!(matches!(outcomes[3], Outcome::Unavailable { .. }));
        assert_eq!(outcomes[2], Outcome::Rows { len: 1 });
        let views = captured.tables();
        assert!(matches!(views[0], TableView::Unavailable { .. }));
        assert!(matches!(views[3], TableView::Unavailable { .. }));
    }

    #[test]
    fn an_empty_world_produces_honest_empty_tables() {
        let (_, mut captured) = small();
        let mut bytes = fixture();
        for (address, count) in [
            (COUNTRY + ccountry::RADAR_GROUPS_COUNT, 0_u32),
            (COUNTRY + ccountry::VOLUNTEER_HOSTS_COUNT, 0),
            (COUNTRY + ccountry::VOLUNTEER_TALLY_COUNT, 0),
            (COUNTRY + ccountry::THEATRES_COUNT, 0),
            (DIVISION + cunit::VIEWER_COUNT, 0),
            (AIR_BASE + cair_base::COUNTRY_CONTAINERS_COUNT, 0),
        ] {
            put(&mut bytes, address, &count.to_le_bytes());
        }
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let _ticks = captured.capture(&memory, 0, BASE, 0, true);
        assert_eq!(captured.outcomes(), [Outcome::Rows { len: 0 }; N]);
        for view in captured.tables() {
            assert!(matches!(&view, TableView::Complete(rows) if rows.is_empty()));
        }
    }

    #[test]
    fn free_fog_acquisition_preserves_frame_state_until_owned_decode() {
        let (_, mut captured) = small();
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let before = captured.outcomes();
        let radar_ctx = captured.radar_copy_context(0, BASE, 0, true);
        let intel_ctx = captured.intel_copy_context(0, BASE);
        assert_eq!(
            (
                radar_ctx.row_limit,
                radar_ctx.province_limit,
                intel_ctx.row_limit
            ),
            (4, 8, 4)
        );
        let mut radar_first = RadarJob::new();
        let mut intel_first = IntelJob::new();
        let radar = copy_radar_refresh(&memory, radar_ctx, &mut radar_first);
        let intel = copy_intel_refresh_with_divisions(&memory, intel_ctx, &mut intel_first, None);
        assert!(radar.raw_ok && intel.raw_ok);
        assert_eq!(captured.outcomes(), before);
        captured.decode_radar_payload(&radar_first, &mut [0]);
        captured.decode_intel_payload(&intel_first, &mut [0]);
        assert!(matches!(captured.outcomes()[0], Outcome::Rows { len: 1 }));
        assert!(matches!(captured.outcomes()[4], Outcome::Rows { len: 2 }));
    }

    #[test]
    fn free_fog_capture_checks_destination_capacity_before_any_live_observation() {
        struct NoReads;
        impl Memory for NoReads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("native read before capacity admission")
            }
        }
        let (_, captured) = small();
        let radar_ctx = captured.radar_copy_context(0, BASE, 0, true);
        for mut job in [RadarJob::with_storage(0, 8), RadarJob::with_storage(4, 0)] {
            let report = copy_radar_refresh(&NoReads, radar_ctx, &mut job);
            assert!(!report.raw_ok && !job.acquired);
            assert_eq!(report.raw_bytes, [0]);
            assert!(job.grow_for_next());
        }
        let mut job = IntelJob::default();
        let ctx = captured.intel_copy_context(0, BASE);
        let report = copy_intel_refresh_with_divisions(&NoReads, ctx, &mut job, None);
        assert!(!report.raw_ok && !job.acquired);
        assert_eq!(report.raw_bytes, [0]);
        assert!(job.grow_for_next());
    }

    #[test]
    fn accepted_fog_failures_are_terminal_and_leave_other_tables_independent() {
        struct Reserved<'a>(SliceMemory<'a>);
        impl Memory for Reserved<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                assert_ne!(
                    address,
                    COUNTRY + ccountry::RADARS_POOL,
                    "reserved radar retried"
                );
                assert_ne!(
                    address,
                    DIVISION + cunit::VIEWER_DATA,
                    "reserved division intel retried"
                );
                assert_ne!(
                    address,
                    TASK_FORCE + cunit::VIEWER_DATA,
                    "reserved naval intel retried"
                );
                assert_ne!(
                    address,
                    BASE + cgame_state::STRATEGIC_AIR_MANAGER,
                    "reserved registrations retried"
                );
                self.0.copy(address, output)
            }
        }
        let (_, mut captured) = small();
        let bytes = fixture();
        let memory = Reserved(SliceMemory {
            base: BASE,
            bytes: &bytes,
        });
        captured.radar_raw_mode = true;
        captured.intel_raw_mode = true;
        captured.registrations_raw_mode = true;
        let ticks = captured.capture(&memory, 0, BASE, 0, true);
        assert_eq!((ticks[0], ticks[4], ticks[5]), (0, 0, 0));
        let before = captured.outcomes();
        captured.fail_radar_capture("radar worker panicked");
        assert_eq!(
            captured.radar.outcome,
            Outcome::Unavailable {
                reason: "radar worker panicked"
            }
        );
        assert_eq!(captured.intel.outcome, before[4]);
        assert!(!captured.radar_raw_mode && captured.intel_raw_mode);
        assert_eq!(
            (
                captured.radar_provinces.len,
                captured.radar_provinces.required
            ),
            (0, 0)
        );
        captured.fail_intel_capture("intel worker stopped");
        assert_eq!(
            captured.intel.outcome,
            Outcome::Unavailable {
                reason: "intel worker stopped"
            }
        );
        assert!(!captured.intel_raw_mode);
        let after = captured.outcomes();
        for index in [1, 2, 3, 5] {
            assert_eq!(after[index], before[index]);
        }
        assert!(matches!(
            captured.tables()[0],
            TableView::Unavailable { .. }
        ));
        assert!(matches!(
            captured.tables()[4],
            TableView::Unavailable { .. }
        ));
        assert!(captured.registrations_raw_mode);
        captured.fail_registrations_capture("shared bases worker panicked");
        assert_eq!(
            captured.registrations.outcome,
            Outcome::Unavailable {
                reason: "shared bases worker panicked"
            }
        );
        assert!(!captured.registrations_raw_mode);
        assert_eq!((captured.wings.len, captured.wings.required), (0, 0));
        assert_eq!(&captured.outcomes()[..5], &after[..5]);
        assert!(matches!(
            captured.tables()[5],
            TableView::Unavailable { .. }
        ));
        captured.fail_registrations_capture("shared bases worker stopped");
        assert!(matches!(
            captured.tables()[5],
            TableView::Unavailable { .. }
        ));
    }

    #[test]
    fn offslot_areas_keep_stationed_units_and_ignore_private_capacity_and_faults() {
        struct NoAreas<'a> {
            inner: SliceMemory<'a>,
            unit_roots: std::cell::Cell<usize>,
        }
        impl Memory for NoAreas<'_> {
            fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
                assert_ne!(address, THEATRE + ctheatre::CONTROLLER_AREA_DATA);
                if address == THEATRE + ctheatre::STATIONED_UNIT_DATA {
                    self.unit_roots.set(self.unit_roots.get() + 1);
                }
                self.inner.copy(address, bytes)
            }
        }
        let bytes = fixture();
        let mut captured = Captured::with_capacities(&Capacities {
            rows: [4, 4, 0, 4, 4, 4],
            radar_provinces: 8,
            area_provinces: 0,
            wings: 8,
        });
        for hour in (0..24).filter(|hour| *hour != 18) {
            let memory = NoAreas {
                inner: SliceMemory {
                    base: BASE,
                    bytes: &bytes,
                },
                unit_roots: std::cell::Cell::new(0),
            };
            let ticks = captured.capture(&memory, 0, BASE, hour, false);
            assert_eq!(captured.areas.outcome, SKIPPED);
            assert_eq!(captured.units.outcome, Outcome::Rows { len: 1 });
            assert_eq!(memory.unit_roots.get(), 1);
            assert_eq!(ticks[2], 0);
            assert_eq!(
                (
                    captured.area_provinces.len,
                    captured.area_provinces.required
                ),
                (0, 0)
            );
        }
        let _ = captured.capture(
            &SliceMemory {
                base: BASE,
                bytes: &[],
            },
            0,
            BASE,
            0,
            false,
        );
        assert_eq!(captured.areas.outcome, SKIPPED);
        assert!(matches!(
            captured.units.outcome,
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn radar_offslot_checks_neither_storage_nor_native_memory() {
        struct NoReads;
        impl Memory for NoReads {
            fn copy(&self, address: usize, _: &mut [u8]) -> bool {
                panic!("inactive radar read at {address:#x}");
            }
        }
        let mut job = RadarJob::default();
        let mut captured = Captured::with_capacities(&Capacities::default());
        for hour in (0..24).filter(|hour| *hour != 21) {
            let report = copy_radar_refresh(
                &NoReads,
                RadarCopyContext {
                    base: 0,
                    game_state: BASE,
                    hour,
                    force_full: false,
                    row_limit: usize::MAX,
                    province_limit: usize::MAX,
                },
                &mut job,
            );
            assert!(!report.raw_ok && !job.grow_for_next());
            assert_eq!((report.copy_ticks, report.raw_bytes), ([0], [0]));
            captured.prepare_hour(hour, false);
            captured.fail_radar_capture("worker failed");
            let mut ticks = [0];
            captured.decode_radar_payload(&job, &mut ticks);
            assert_eq!(captured.radar.outcome, SKIPPED);
            assert_eq!(ticks, [0]);
        }
    }

    #[test]
    fn radar_uses_slot_21() {
        use replay::rows::TableId;
        assert!(replay::cadence::is_staggered(TableId::RadarCoverage));
        for hour in -24_i32..48 {
            assert_eq!(
                replay::cadence::is_refresh(TableId::RadarCoverage, hour),
                hour.rem_euclid(24) == 21
            );
        }
    }

    #[test]
    fn radar_split_copy_matches_direct_walk() {
        use super::radar_test_kit as kit;
        use engine::memory::{CountedMemory, ReadCounts};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let memory = CountedMemory::new(memory);
        let capture = read_accounting::Capture::new();
        let (split, report, mut job) = kit::split(&memory);
        let counts = capture.finish();
        assert!(report.raw_ok);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.outcome, Outcome::Rows { len: 1 });
        assert_eq!(
            (
                split.rows[0].country,
                split.rows[0].ordinal,
                split.rows[0].state
            ),
            (1, 0, Some(77))
        );
        let anchor = TableId::RadarCoverage;
        let mut expected = read_accounting::ZERO;
        // 144 scalar bytes include both country authentications; the pool
        // and group each gather three spans, adding 60 bytes.
        expected[anchor.index()] = ReadCounts {
            copy_calls: 21,
            scattered_calls: 2,
            record_calls: 0,
            plan_calls: 0,
            spans: 27,
            bytes: 204,
        };
        assert_eq!(counts, expected);
        let mut refreshes = 0;
        for hour in 0..24 {
            if replay::cadence::is_refresh(anchor, hour) {
                refreshes += 1;
                continue;
            }
            let capture = read_accounting::Capture::new();
            let skipped = copy_radar_refresh(
                &memory,
                RadarCopyContext {
                    base: kit::BASE,
                    game_state: kit::GAME_STATE,
                    hour,
                    force_full: false,
                    row_limit: usize::MAX,
                    province_limit: usize::MAX,
                },
                &mut job,
            );
            assert_eq!(capture.finish(), read_accounting::ZERO);
            assert!(!skipped.raw_ok && !job.grow_for_next());
            assert_eq!((skipped.copy_ticks, skipped.raw_bytes), ([0], [0]));
        }
        assert_eq!(refreshes, 1);
        let measured = counts[anchor.index()];
        let per_refresh = [
            measured.copy_calls,
            measured.scattered_calls,
            measured.record_calls,
            measured.plan_calls,
            measured.spans,
            measured.bytes,
        ];
        println!(
            "daily_fixture cohort=radar_split_copy_matches_direct_walk anchor={anchor:?} counters=[copy,scattered,records,plans,spans,bytes] per_refresh={per_refresh:?} before_24h={:?} after_24h={per_refresh:?} basis=unchanged_refresh_x_24_legacy_hourly_vs_1_daily",
            per_refresh.map(|value| value * 24)
        );
    }

    #[test]
    fn radar_truncation_matrix_matches_direct_with_same_caps() {
        // B3: zero/one/exact-fit/overflow-by-one/massive run typed AND
        // copy→decode against the SAME memory with the SAME pooled caps,
        // asserting identical outcome (incl. len/unread) and identical rows.
        use super::radar_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        for (rows, provinces) in [(0, 0), (1, 1), (1, 2), (64, 64)] {
            let direct = kit::direct_with(&memory, rows, provinces);
            let (split, report, _) = kit::split_with(&memory, rows, provinces);
            assert!(report.raw_ok, "rows={rows} provinces={provinces}");
            kit::assert_matches_direct(&split, &direct);
        }
        let direct = kit::direct_with(&memory, 1, 2);
        assert_eq!(direct.outcome, Outcome::Rows { len: 1 });
    }

    #[test]
    fn radar_split_copy_matches_direct_walk_on_fault_matrix() {
        use super::radar_test_kit as kit;
        use engine::memory::SliceMemory;
        struct Fault<'a> {
            memory: SliceMemory<'a>,
            fault: std::ops::Range<usize>,
        }
        impl engine::Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let Some(end) = address.checked_add(output.len()) else {
                    return false;
                };
                if address < self.fault.end && end > self.fault.start {
                    return false;
                }
                self.memory.copy(address, output)
            }
        }
        for fault in [
            kit::GAME_STATE + engine::cgame_state::COUNTRIES,
            kit::C0,
            kit::C0 + engine::ccountry::RADARS_POOL,
            kit::GROUP,
            kit::GROUP + engine::radar_group::STATE,
        ] {
            let bytes = kit::fixture();
            let memory = Fault {
                memory: SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                fault: fault..fault + 1,
            };
            let direct = kit::direct(&memory);
            let (split, report, _) = kit::split(&memory);
            assert!(report.raw_ok, "fault={fault:x}");
            kit::assert_matches_direct(&split, &direct);
        }
    }

    #[test]
    fn radar_staging_overflow_falls_back_to_typed_walk() {
        use super::radar_test_kit as kit;
        use super::{RadarCopyContext, RadarJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = RadarCopyContext {
            hour: 0,
            force_full: true,
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            row_limit: 1024,
            province_limit: 65_536,
        };
        let mut tiny = RadarJob::default();
        let report = copy_radar_refresh(&memory, ctx, &mut tiny);
        assert!(!report.raw_ok);
        assert!(tiny.grow_for_next());
    }

    #[test]
    fn radar_pools_cover_appendix_worst_case() {
        let job = RadarJob::new();
        assert!(job.rows.len() >= 18);
        assert!(job.provinces.len() >= 65_536);
    }

    #[test]
    fn radar_copy_sizes_cover_documented_read_set() {
        // S3: every Memory::copy size the radar copy pass issues, recorded
        // through the real pass. Census words (4/8), array headers (16),
        // group words (8/4) and coverage chunks (4-byte words).
        use super::radar_test_kit as kit;
        use engine::Memory;
        use std::collections::BTreeSet;
        struct Recorder<'a> {
            inner: engine::memory::SliceMemory<'a>,
            sizes: std::cell::RefCell<BTreeSet<usize>>,
        }
        impl Memory for Recorder<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.sizes.borrow_mut().insert(output.len());
                self.inner.copy(address, output)
            }
        }
        let bytes = kit::fixture();
        let memory = Recorder {
            inner: kit::memory(&bytes),
            sizes: std::cell::RefCell::new(BTreeSet::new()),
        };
        let (_split, report, _) = kit::split(&memory);
        assert!(report.raw_ok);
        let sizes = memory.sizes.borrow().clone();
        for required in [4, 8, 16] {
            assert!(
                sizes.contains(&required),
                "missing size {required} in {sizes:?}"
            );
        }
    }

    #[test]
    fn intel_tables_are_hourly_tier_h_with_no_refresh_gate() {
        use replay::rows::TableId;
        assert!(!replay::cadence::is_staggered(TableId::UnitIntel));
        for hour in [0, 1, 2, 3, 60_820_531] {
            assert!(replay::cadence::is_refresh(TableId::UnitIntel, hour));
        }
    }

    #[test]
    fn intel_split_copy_matches_direct_walk() {
        use super::intel_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let (split, report, _) = kit::split(&memory);
        assert!(report.raw_ok);
        kit::assert_matches_direct(&split, &direct);
        // Two viewer entries on the division; the task force contributes
        // none. Both unit paths decode from the same shared spans.
        assert_eq!(split.outcome, Outcome::Rows { len: 2 });
        assert_eq!((split.rows[0].viewer, split.rows[0].level), (1, 3));
        assert_eq!((split.rows[1].viewer, split.rows[1].level), (2, -1));
        kit::assert_paths_match_direct(&split, &direct);
    }

    #[test]
    fn intel_truncation_matrix_matches_direct_with_same_caps() {
        // B3: zero/one/exact-fit(2)/overflow-by-one/massive run typed AND
        // copy→decode against the SAME memory with the SAME pooled caps,
        // asserting identical outcome (incl. len/unread) and identical rows.
        use super::intel_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        for cap in [0, 1, 2, 64] {
            let direct = kit::direct_with(&memory, cap);
            let (split, report, _) = kit::split_with(&memory, cap);
            assert!(report.raw_ok, "cap={cap}");
            kit::assert_matches_direct(&split, &direct);
            kit::assert_paths_match_direct(&split, &direct);
        }
        let direct = kit::direct_with(&memory, 2);
        assert_eq!(direct.outcome, Outcome::Rows { len: 2 });
    }

    #[test]
    fn intel_split_copy_matches_direct_walk_on_fault_matrix() {
        use super::intel_test_kit as kit;
        use engine::memory::SliceMemory;
        struct Fault<'a> {
            memory: SliceMemory<'a>,
            fault: std::ops::Range<usize>,
        }
        impl engine::Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let Some(end) = address.checked_add(output.len()) else {
                    return false;
                };
                if address < self.fault.end && end > self.fault.start {
                    return false;
                }
                self.memory.copy(address, output)
            }
        }
        for fault in [
            kit::GAME_STATE + engine::cgame_state::COUNTRIES,
            kit::C0,
            kit::C0 + engine::ccountry::DIVISIONS,
            kit::DIVISION,
            kit::DIVISION + engine::cunit::HANDLE,
            kit::VIEWERS,
            kit::C0 + engine::ccountry::FLEETS,
        ] {
            let bytes = kit::fixture();
            let memory = Fault {
                memory: SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                fault: fault..fault + 1,
            };
            let direct = kit::direct(&memory);
            let (split, report, _) = kit::split(&memory);
            assert!(report.raw_ok, "fault={fault:x}");
            kit::assert_matches_direct(&split, &direct);
            kit::assert_paths_match_direct(&split, &direct);
        }
    }

    #[test]
    fn intel_staging_overflow_falls_back_to_typed_walk() {
        use super::intel_test_kit as kit;
        use super::{IntelCopyContext, IntelJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = IntelCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            row_limit: 32_768,
        };
        let mut tiny = IntelJob::default();
        let report = copy_intel_refresh_with_divisions(&memory, ctx, &mut tiny, None);
        assert!(!report.raw_ok);
        assert!(tiny.grow_for_next());
    }

    #[test]
    fn intel_pools_cover_appendix_worst_case() {
        let job = IntelJob::new();
        assert!(job.rows.len() >= 3_200 + 500);
        assert!(job.provinces.is_empty());
    }

    #[test]
    fn intel_copy_sizes_cover_documented_read_set() {
        // S3: every Memory::copy size the intel copy pass issues, recorded
        // through the real pass. Census words (4/8/16), roster headers (16),
        // unit words (8) and viewer batches (count x 24).
        use super::intel_test_kit as kit;
        use engine::Memory;
        use std::collections::BTreeSet;
        struct Recorder<'a> {
            inner: engine::memory::SliceMemory<'a>,
            sizes: std::cell::RefCell<BTreeSet<usize>>,
        }
        impl Memory for Recorder<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.sizes.borrow_mut().insert(output.len());
                self.inner.copy(address, output)
            }
        }
        let bytes = kit::fixture();
        let memory = Recorder {
            inner: kit::memory(&bytes),
            sizes: std::cell::RefCell::new(BTreeSet::new()),
        };
        let (_split, report, _) = kit::split(&memory);
        assert!(report.raw_ok);
        let sizes = memory.sizes.borrow().clone();
        for required in [4, 8, 16, 48] {
            assert!(
                sizes.contains(&required),
                "missing size {required} in {sizes:?}"
            );
        }
    }
}

/// Shared synthetic address space for the radar copy/decode harness: one
/// country with one radar group covering two provinces. Mirrors the engine
/// radar fixture shapes.
#[cfg(test)]
pub(crate) mod radar_test_kit {
    use super::{
        Capacities, Captured, RadarCopyContext, RadarCopyReport, RadarJob, copy_radar_refresh,
    };
    use engine::{Memory, memory::SliceMemory, walk::Outcome};

    pub(crate) const BASE: usize = 0;
    pub(crate) const GAME_STATE: usize = 0x10000;
    const CTAB: usize = 0x11000;
    const LTAB: usize = 0x12000;
    pub(crate) const C0: usize = 0x20000;
    pub(crate) const GROUP: usize = 0x50000;
    const GVEC: usize = 0x40000;
    const COVERED: usize = 0x70000;
    const STATE_A: usize = 0x60000;

    fn put32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        put64(bytes, at, data as u64);
        put32(bytes, at + 8, count);
        put32(bytes, at + 12, count);
    }

    pub(crate) fn fixture() -> Vec<u8> {
        use engine::{ccountry, cgame_state, cradars_pool, cstate, radar_group};
        let mut bytes = vec![0; 0x80000];
        header(&mut bytes, GAME_STATE + cgame_state::COUNTRIES, CTAB, 2);
        put64(&mut bytes, CTAB, 0);
        put64(&mut bytes, CTAB + 8, C0 as u64);
        header(&mut bytes, GAME_STATE + cgame_state::COUNTRY_LINKS, LTAB, 3);
        put32(&mut bytes, LTAB, 0);
        put32(&mut bytes, LTAB + 4, 1);
        put32(&mut bytes, LTAB + 8, 0);
        put64(&mut bytes, C0, ccountry::VTABLE_RVA as u64);
        put32(&mut bytes, C0 + ccountry::ID, 1);
        put64(
            &mut bytes,
            C0 + ccountry::RADARS_POOL,
            cradars_pool::VTABLE_RVA as u64,
        );
        put32(&mut bytes, C0 + ccountry::RADARS_POOL_COUNTRY, 1);
        header(&mut bytes, C0 + ccountry::RADAR_GROUPS, GVEC, 1);
        put64(&mut bytes, GVEC, GROUP as u64);
        put64(
            &mut bytes,
            GROUP + radar_group::PARENT,
            (C0 + ccountry::RADARS_POOL) as u64,
        );
        put64(&mut bytes, GROUP + radar_group::STATE, STATE_A as u64);
        header(&mut bytes, GROUP + radar_group::PROVINCES, COVERED, 2);
        put32(&mut bytes, COVERED, 11);
        put32(&mut bytes, COVERED + 4, 12);
        put64(&mut bytes, STATE_A, cstate::VTABLE_RVA as u64);
        put32(&mut bytes, STATE_A + cstate::ID, 77);
        bytes
    }

    pub(crate) fn memory(bytes: &[u8]) -> SliceMemory<'_> {
        SliceMemory { base: 0, bytes }
    }

    pub(crate) struct Split {
        pub outcome: Outcome,
        pub rows: Vec<engine::radar_group::Row>,
        pub provinces: Vec<u32>,
    }

    fn collect_typed(memory: &impl Memory, rows_cap: usize, provinces_cap: usize) -> Split {
        use engine::radar_group;
        let capacities = Capacities {
            rows: [rows_cap, 4, 4, 4, 4, 4],
            radar_provinces: provinces_cap,
            area_provinces: 8,
            wings: 8,
        };
        let mut captured = Captured::with_capacities(&capacities);
        let walked = radar_group::walk(
            memory,
            BASE,
            GAME_STATE,
            &mut captured.radar.rows,
            &mut captured.radar_provinces.first,
        );
        super::arena_first(
            &mut captured.radar,
            &mut captured.radar_provinces,
            walked.outcome,
            walked.provinces_len,
            walked.required_provinces,
        );
        Split {
            outcome: captured.radar.outcome,
            rows: captured.radar.rows.clone(),
            provinces: captured.radar_provinces.first.clone(),
        }
    }

    fn collect_split(
        memory: &impl Memory,
        rows_cap: usize,
        provinces_cap: usize,
    ) -> (Split, RadarCopyReport, RadarJob) {
        let ctx = RadarCopyContext {
            hour: 0,
            force_full: true,
            base: BASE,
            game_state: GAME_STATE,
            row_limit: rows_cap,
            province_limit: provinces_cap,
        };
        let capacities = Capacities {
            rows: [rows_cap, 4, 4, 4, 4, 4],
            radar_provinces: provinces_cap,
            area_provinces: 8,
            wings: 8,
        };
        let mut captured = Captured::with_capacities(&capacities);
        let mut first = RadarJob::new();
        let report = copy_radar_refresh(memory, ctx, &mut first);
        assert!(report.raw_ok);
        let mut ticks = [0u64; 1];
        captured.decode_radar_payload(&first, &mut ticks);
        (
            Split {
                outcome: captured.radar.outcome,
                rows: captured.radar.rows.clone(),
                provinces: captured.radar_provinces.first.clone(),
            },
            report,
            first,
        )
    }

    pub(crate) fn direct(memory: &impl Memory) -> Split {
        collect_typed(memory, 1024, 65_536)
    }

    pub(crate) fn split(memory: &impl Memory) -> (Split, RadarCopyReport, RadarJob) {
        collect_split(memory, 1024, 65_536)
    }

    pub(crate) fn direct_with(memory: &impl Memory, rows: usize, provinces: usize) -> Split {
        collect_typed(memory, rows, provinces)
    }

    pub(crate) fn split_with(
        memory: &impl Memory,
        rows: usize,
        provinces: usize,
    ) -> (Split, RadarCopyReport, RadarJob) {
        collect_split(memory, rows, provinces)
    }

    pub(crate) fn assert_matches_direct(split: &Split, direct: &Split) {
        assert_eq!(split.outcome, direct.outcome, "radar outcome");
        assert_eq!(
            &split.rows[..split.outcome.len()],
            &direct.rows[..direct.outcome.len()],
            "radar rows"
        );
        assert_eq!(split.provinces, direct.provinces, "radar provinces");
    }
}

/// Shared synthetic address space for the intel copy/decode harness: one
/// country with one division (two viewer entries) and one task force (no
/// entries). Mirrors the engine viewer-entry fixture shapes.
#[cfg(test)]
pub(crate) mod intel_test_kit {
    use super::{
        Capacities, Captured, IntelCopyContext, IntelCopyReport, IntelJob,
        copy_intel_refresh_with_divisions,
    };
    use engine::{Memory, memory::SliceMemory, walk::Outcome};

    pub(crate) const BASE: usize = 0;
    pub(crate) const GAME_STATE: usize = 0x10000;
    const CTAB: usize = 0x11000;
    const LTAB: usize = 0x12000;
    pub(crate) const C0: usize = 0x20000;
    pub(crate) const DIVISION: usize = 0x50000;
    const FLEET: usize = 0x58000;
    const TASK_FORCE: usize = 0x60000;
    pub(crate) const VIEWERS: usize = 0x70000;

    fn put32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        put64(bytes, at, data as u64);
        put32(bytes, at + 8, count);
        put32(bytes, at + 12, count);
    }

    pub(crate) fn fixture() -> Vec<u8> {
        use engine::{carmy, ccountry, cfleet, cgame_state, ctask_force, cunit, unit_viewer_entry};
        let mut bytes = vec![0; 0x80000];
        header(&mut bytes, GAME_STATE + cgame_state::COUNTRIES, CTAB, 2);
        put64(&mut bytes, CTAB, 0);
        put64(&mut bytes, CTAB + 8, C0 as u64);
        header(&mut bytes, GAME_STATE + cgame_state::COUNTRY_LINKS, LTAB, 3);
        put32(&mut bytes, LTAB, 0);
        put32(&mut bytes, LTAB + 4, 1);
        put32(&mut bytes, LTAB + 8, 0);
        put64(&mut bytes, C0, ccountry::VTABLE_RVA as u64);
        put32(&mut bytes, C0 + ccountry::ID, 1);
        header(&mut bytes, C0 + ccountry::DIVISIONS, 0x40000, 1);
        put64(&mut bytes, 0x40000, DIVISION as u64);
        header(&mut bytes, C0 + ccountry::FLEETS, 0x41000, 1);
        put64(&mut bytes, 0x41000, FLEET as u64);
        put64(&mut bytes, FLEET, cfleet::VTABLE_RVA as u64);
        header(&mut bytes, FLEET + cfleet::TASK_FORCES, 0x42000, 1);
        put64(&mut bytes, 0x42000, TASK_FORCE as u64);
        put64(&mut bytes, DIVISION, carmy::VTABLE_RVA as u64);
        put64(&mut bytes, DIVISION + cunit::HANDLE, (7u64 << 32) | 3);
        header(&mut bytes, DIVISION + cunit::VIEWER_DATA, VIEWERS, 2);
        for (index, (viewer, level)) in [(1u32, 3u8), (2, 255)].into_iter().enumerate() {
            put32(
                &mut bytes,
                VIEWERS + index * unit_viewer_entry::STRIDE,
                viewer,
            );
            bytes[VIEWERS + index * unit_viewer_entry::STRIDE + unit_viewer_entry::LEVEL] = level;
        }
        put64(&mut bytes, TASK_FORCE, ctask_force::VTABLE_RVA as u64);
        put64(&mut bytes, TASK_FORCE + cunit::HANDLE, u64::MAX);
        header(&mut bytes, TASK_FORCE + cunit::VIEWER_DATA, 0, 0);
        bytes
    }

    pub(crate) fn memory(bytes: &[u8]) -> SliceMemory<'_> {
        SliceMemory { base: 0, bytes }
    }

    pub(crate) struct Split {
        pub outcome: Outcome,
        pub rows: Vec<engine::unit_viewer_entry::Row>,
    }

    fn collect_typed(memory: &impl Memory, cap: usize) -> Split {
        use engine::unit_viewer_entry;
        let capacities = Capacities {
            rows: [4, 4, 4, 4, cap, 4],
            radar_provinces: 8,
            area_provinces: 8,
            wings: 8,
        };
        let mut captured = Captured::with_capacities(&capacities);
        let walked = unit_viewer_entry::walk(memory, BASE, GAME_STATE, &mut captured.intel.rows);
        captured.intel.outcome = super::bounded(walked, captured.intel.rows.len());
        Split {
            outcome: captured.intel.outcome,
            rows: captured.intel.rows.clone(),
        }
    }

    fn collect_split(memory: &impl Memory, cap: usize) -> (Split, IntelCopyReport, IntelJob) {
        let ctx = IntelCopyContext {
            base: BASE,
            game_state: GAME_STATE,
            row_limit: cap,
        };
        let capacities = Capacities {
            rows: [4, 4, 4, 4, cap, 4],
            radar_provinces: 8,
            area_provinces: 8,
            wings: 8,
        };
        let mut captured = Captured::with_capacities(&capacities);
        let mut first = IntelJob::new();
        let report = copy_intel_refresh_with_divisions(memory, ctx, &mut first, None);
        assert!(report.raw_ok);
        let mut ticks = [0u64; 1];
        captured.decode_intel_payload(&first, &mut ticks);
        (
            Split {
                outcome: captured.intel.outcome,
                rows: captured.intel.rows.clone(),
            },
            report,
            first,
        )
    }

    pub(crate) fn direct(memory: &impl Memory) -> Split {
        collect_typed(memory, 32_768)
    }

    pub(crate) fn split(memory: &impl Memory) -> (Split, IntelCopyReport, IntelJob) {
        collect_split(memory, 32_768)
    }

    pub(crate) fn direct_with(memory: &impl Memory, cap: usize) -> Split {
        collect_typed(memory, cap)
    }

    pub(crate) fn split_with(
        memory: &impl Memory,
        cap: usize,
    ) -> (Split, IntelCopyReport, IntelJob) {
        collect_split(memory, cap)
    }

    pub(crate) fn assert_matches_direct(split: &Split, direct: &Split) {
        assert_eq!(split.outcome, direct.outcome, "intel outcome");
        assert_eq!(
            &split.rows[..split.outcome.len()],
            &direct.rows[..direct.outcome.len()],
            "intel rows"
        );
    }

    /// Partitioned dual-path proof: division-family and fleet-family rows
    /// decoded from the same shared spans equal the direct walk's
    /// per-family partitions.
    pub(crate) fn assert_paths_match_direct(split: &Split, direct: &Split) {
        use engine::ctask_force;
        // Task-force handles carry the naval type tag in the low word;
        // divisions carry another.
        let is_fleet =
            |unit: u64| (unit & 0xffff_ffff) == u64::from(ctask_force::DEFAULT_HANDLE_TYPE);
        for fleet in [false, true] {
            let split_path: Vec<_> = split.rows[..split.outcome.len()]
                .iter()
                .filter(|row| is_fleet(row.unit) == fleet)
                .collect();
            let direct_path: Vec<_> = direct.rows[..direct.outcome.len()]
                .iter()
                .filter(|row| is_fleet(row.unit) == fleet)
                .collect();
            assert_eq!(split_path, direct_path, "intel path fleet={fleet}");
        }
    }
}
