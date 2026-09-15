//! Native fog-derivation input acquisition (registry domain D102). Only growth
//! and replay conversion allocate; every engine read happens in the callback.

use crate::{
    raw::RawLog,
    sampling::{full_match, qpc},
    table_capture::{Arena, CaptureRows},
};
use engine::{
    Memory, cair_base, ccontroller_area, ccountry, ctheatre,
    memory::{RecordLayout, RecordSources},
    radar_group, unit_viewer_entry, walk::Outcome,
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

impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        Self {
            radar: CaptureRows::new(capacity.rows[0], successful[0]),
            radar_provinces: Arena::new(capacity.radar_provinces, successful[0]),
            volunteers: CaptureRows::new(capacity.rows[1], successful[1]),
            areas: CaptureRows::new(capacity.rows[2], successful[2]),
            area_provinces: Arena::new(capacity.area_provinces, successful[2]),
            units: CaptureRows::new(capacity.rows[3], successful[3]),
            intel: CaptureRows::new(capacity.rows[4], successful[4]),
            registrations: CaptureRows::new(capacity.rows[5], successful[5]),
            wings: Arena::new(capacity.wings, successful[5]),
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

    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        self.radar.grow(&mut capacity.rows[0], successful[0]);
        self.radar_provinces
            .grow(&mut capacity.radar_provinces, successful[0]);
        self.volunteers.grow(&mut capacity.rows[1], successful[1]);
        self.areas.grow(&mut capacity.rows[2], successful[2]);
        self.area_provinces
            .grow(&mut capacity.area_provinces, successful[2]);
        self.units.grow(&mut capacity.rows[3], successful[3]);
        self.intel.grow(&mut capacity.rows[4], successful[4]);
        self.registrations.grow(&mut capacity.rows[5], successful[5]);
        self.wings.grow(&mut capacity.wings, successful[5]);
    }

    /// Every walk reads independently. Areas and stationed units share one
    /// theatre traversal, and its second pass runs when either needs a check.
    /// No allocation, encoding or I/O occurs here; times are raw QPC ticks.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        self.reset();
        let mut matched = [false; N];
        let mut ticks = [0; N];

        // A staged owned payload already contains this hour's native rows.
        // Its acquisition checks and costs are applied by the caller.
        if self.radar_raw_mode {
            // Refresh payload staged: leave the reset outcome for the decode
            // worker, which installs identical rows and outcomes.
            matched[0] = false;
            ticks[0] = 0;
        } else {
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
            if needed[0] {
                let second = radar_group::walk(
                    memory,
                    base,
                    game_state,
                    &mut self.radar.second,
                    &mut self.radar_provinces.second,
                );
                matched[0] = arena_second(
                    &mut self.radar,
                    &mut self.radar_provinces,
                    second.outcome,
                    second.provinces_len,
                    second.required_provinces,
                );
            }
            ticks[0] = elapsed(begin);
        }

        let begin = qpc();
        self.volunteers.outcome = bounded(
            ccountry::walk_volunteer_hosts(memory, base, game_state, &mut self.volunteers.rows),
            self.volunteers.rows.len(),
        );
        if needed[1] {
            self.volunteers.checked = true;
            let second = bounded(
                ccountry::walk_volunteer_hosts(
                    memory,
                    base,
                    game_state,
                    &mut self.volunteers.second,
                ),
                self.volunteers.second.len(),
            );
            matched[1] = self.volunteers.compare(second);
        }
        ticks[1] = elapsed(begin);

        let begin = qpc();
        let first = ctheatre::walk_stationed(
            memory,
            base,
            game_state,
            ctheatre::StationedBuffers {
                areas: &mut self.areas.rows,
                area_provinces: &mut self.area_provinces.first,
                units: &mut self.units.rows,
            },
        );
        arena_first(
            &mut self.areas,
            &mut self.area_provinces,
            first.areas,
            first.provinces_len,
            first.required_provinces,
        );
        self.units.outcome = bounded(first.units, self.units.rows.len());
        if needed[2] || needed[3] {
            let second = ctheatre::walk_stationed(
                memory,
                base,
                game_state,
                ctheatre::StationedBuffers {
                    areas: &mut self.areas.second,
                    area_provinces: &mut self.area_provinces.second,
                    units: &mut self.units.second,
                },
            );
            if needed[2] {
                matched[2] = arena_second(
                    &mut self.areas,
                    &mut self.area_provinces,
                    second.areas,
                    second.provinces_len,
                    second.required_provinces,
                );
            }
            if needed[3] {
                self.units.checked = true;
                matched[3] = self
                    .units
                    .compare(bounded(second.units, self.units.second.len()));
            }
        }
        // ticks[3] stays zero: the named read group above owns this traversal.
        ticks[2] = elapsed(begin);

        // Shared census acquisition already produced this hour's owned rows.
        if self.intel_raw_mode {
            // Refresh payload staged: leave the reset outcome for the decode
            // worker, which installs identical rows and outcomes.
            matched[4] = false;
            ticks[4] = 0;
        } else {
            let begin = qpc();
            self.intel.outcome = bounded(
                unit_viewer_entry::walk(memory, base, game_state, &mut self.intel.rows),
                self.intel.rows.len(),
            );
            if needed[4] {
                self.intel.checked = true;
                let second = bounded(
                    unit_viewer_entry::walk(memory, base, game_state, &mut self.intel.second),
                    self.intel.second.len(),
                );
                matched[4] = self.intel.compare(second);
            }
            ticks[4] = elapsed(begin);
        }

        // Shared bases payload's registrations half: on hours with a valid
        // raw payload staged via the shared air copy, the decode worker
        // fills the typed rows before the ready handoff, so the typed walk
        // is skipped here.
        if self.registrations_raw_mode {
            // Refresh payload staged: leave the reset outcome for the decode
            // worker, which installs identical rows and outcomes.
            matched[5] = false;
            ticks[5] = 0;
        } else {
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
            if needed[5] {
                let second = cair_base::walk_registrations(
                    memory,
                    base,
                    game_state,
                    cair_base::RegistrationBuffers {
                        registrations: &mut self.registrations.second,
                        wings: &mut self.wings.second,
                    },
                );
                matched[5] = arena_second(
                    &mut self.registrations,
                    &mut self.wings,
                    second.outcome,
                    second.wings_len,
                    second.required_wings,
                );
            }
            ticks[5] = elapsed(begin);
        }
        (matched, ticks)
    }

    pub fn checked_mismatches(&self) -> [usize; N] {
        [
            self.radar.mismatches,
            self.volunteers.mismatches,
            self.areas.mismatches,
            self.units.mismatches,
            self.intel.mismatches,
            self.registrations.mismatches,
        ]
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            effective(&self.radar),
            effective(&self.volunteers),
            effective(&self.areas),
            effective(&self.units),
            effective(&self.intel),
            effective(&self.registrations),
        ]
    }

    #[cfg(test)]
    pub fn tables(&self) -> [TableView; N] {
        self.table_inputs(None).map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("no projection supplied"),
        })
    }

    pub(crate) fn table_inputs(&self, mut projection: Option<&mut crate::projection::Projection<'_>>) -> [TableInput; N] {
        use crate::projection::scalar_table;
        let outcomes = self.outcomes();
        [
            self.radar_table(outcomes[0], projection.as_deref_mut()),
            scalar_table::<tables::VolunteerHosts, _>(projection.as_deref_mut(), outcomes[1], &self.volunteers.rows,
                |r| (u128::from(r.country) << 32) | u128::from(r.position), |r| rows::VolunteerHostRow {
                    country: r.country, position: r.position, host: r.host, deployed_divisions: r.deployed_divisions,
                }),
            self.area_table(outcomes[2], projection.as_deref_mut()),
            scalar_table::<tables::StationedUnits, _>(projection.as_deref_mut(), outcomes[3], &self.units.rows,
                |r| (u128::from(r.theatre) << 32) | u128::from(r.ordinal), |r| rows::StationedUnitRow {
                    theatre: r.theatre, ordinal: r.ordinal, unit: r.unit,
                }),
            scalar_table::<tables::UnitIntel, _>(projection.as_deref_mut(), outcomes[4], &self.intel.rows,
                |r| (u128::from(r.unit) << 32) | u128::from(r.ordinal), |r| rows::UnitIntelRow {
                    unit: r.unit, ordinal: r.ordinal, viewer: r.viewer, level: r.level,
                }),
            self.registration_table(outcomes[5], projection),
        ]
    }

    fn radar_table(&self, outcome: Outcome, projection: Option<&mut crate::projection::Projection<'_>>) -> TableInput {
        if !valid_arena(outcome, &self.radar.rows, &self.radar_provinces) {
            return crate::spooling::unavailable("radar coverage vector lies outside the captured arena").into();
        }
        let provinces = |r: &radar_group::Row| &self.radar_provinces.first[r.provinces_start..r.provinces_start + r.provinces_len];
        crate::projection::project_table::<tables::RadarCoverage, _>(projection, outcome, &self.radar.rows, ArenaRow::key,
            |r, old| r.country == old.country && r.ordinal == old.ordinal && r.state == old.state && provinces(r) == old.provinces,
            |r| rows::RadarCoverageRow { country: r.country, ordinal: r.ordinal, state: r.state, provinces: provinces(r).to_vec() })
    }

    fn area_table(&self, outcome: Outcome, projection: Option<&mut crate::projection::Projection<'_>>) -> TableInput {
        if !valid_arena(outcome, &self.areas.rows, &self.area_provinces) {
            return crate::spooling::unavailable("controller area province vector lies outside the captured arena").into();
        }
        let provinces = |r: &ccontroller_area::Row| &self.area_provinces.first[r.provinces_start..r.provinces_start + r.provinces_len];
        crate::projection::project_table::<tables::TheatreAreas, _>(projection, outcome, &self.areas.rows, ArenaRow::key,
            |r, old| r.theatre == old.theatre && r.ordinal == old.ordinal && r.representative == old.representative && provinces(r) == old.provinces,
            |r| rows::TheatreAreaRow { theatre: r.theatre, ordinal: r.ordinal, representative: r.representative, provinces: provinces(r).to_vec() })
    }

    fn registration_table(&self, outcome: Outcome, projection: Option<&mut crate::projection::Projection<'_>>) -> TableInput {
        if !valid_arena(outcome, &self.registrations.rows, &self.wings) {
            return crate::spooling::unavailable("air registration wing vector lies outside the captured arena").into();
        }
        let wings = |r: &cair_base::RegistrationRow| &self.wings.first[r.wings_start..r.wings_start + r.wings_len];
        crate::projection::project_table::<tables::AirBaseRegistrations, _>(projection, outcome, &self.registrations.rows, ArenaRow::key,
            |r, old| r.base == old.base && r.slot == old.slot && r.country == old.country && wings(r) == old.wings,
            |r| rows::AirBaseRegistrationRow { base: r.base, slot: r.slot, country: r.country, wings: wings(r).to_vec() })
    }

    /// Game-thread bookkeeping for the shared bases/registrations/aces copy:
    /// mirrors the typed path's checked/mismatches for the registrations
    /// slot from the raw-compare verdict.
    pub(crate) fn note_registrations_copy(&mut self, needed: bool, matched: bool) {
        self.registrations.checked = needed;
        self.registrations.mismatches = usize::from(needed && !matched);
    }

    /// Decode-worker interpretation of the shared bases payload's
    /// registrations half: replays `cair_base::walk_registrations` against
    /// the same recorded spans the bases and aces decodes read (see
    /// `super::air::Captured::decode_air_bases_payload`), installing
    /// identical rows, arena bytes, truncation counts and failure reasons.
    pub(crate) fn decode_registrations_payload(
        &mut self,
        base: usize,
        job: &super::air::AirBasesJob,
        decode_ticks: &mut [u64; super::air::AIR_BASES_N],
    ) {
        use crate::raw::Recorded;
        let recorded = Recorded::build(&job.log);
        let started = qpc();
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
        // Reproduce the typed path's duplicate-key scan: `arena_second`
        // sorts both censuses and blocks the check on duplicate keys in
        // either. The spool conversion sorts by unique key downstream, so
        // buffer order is unchanged here; only the verdict matters. The scan
        // runs exactly when the typed path would run it (checked window with
        // a Rows outcome); the raw-compare bookkeeping already covers every
        // case where the two copies disagree.
        if self.registrations.checked
            && matches!(self.registrations.outcome, Outcome::Rows { .. })
        {
            let len = self.registrations.outcome.len();
            let mut keys: Vec<u128> = self.registrations.rows[..len]
                .iter()
                .map(ArenaRow::key)
                .collect();
            keys.sort_unstable();
            if keys.windows(2).any(|pair| pair[0] == pair[1]) {
                self.registrations.mismatches += 1;
            }
        }
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
        (self.registrations_raw_mode
            && !self.registrations.checked
            && self.registrations.mismatches == 0)
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
        // Keep this frame's checked/mismatches and comparison storage unchanged.
    }
}

#[cfg(test)]
mod registrations_memo_tests {
    use super::*;

    fn captured() -> Captured {
        Captured::with_capacities(
            &Capacities {
                rows: [0, 0, 0, 0, 0, 4],
                radar_provinces: 0,
                area_provinces: 0,
                wings: 8,
            },
            &[200; N],
        )
    }

    #[test]
    fn registrations_memo_validates_ranges_and_preserves_required_and_check_storage() {
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
        target
            .registrations
            .second
            .push(cair_base::RegistrationRow {
                base: 99,
                ..Default::default()
            });
        target.wings.second.extend_from_slice(&[999]);
        assert_eq!(target.registrations_memo_context(), Some([4, 8]));
        target.restore_registrations_memo(&memo);
        assert_eq!(target.registrations.outcome, Outcome::Rows { len: 1 });
        assert_eq!(target.registrations.rows[0], source.registrations.rows[0]);
        assert_eq!(target.wings.len, 2);
        assert_eq!(target.wings.required, 5);
        assert_eq!(&target.wings.first[..2], &[100, 200]);
        assert_eq!(target.registrations.second[0].base, 99);
        assert_eq!(target.wings.second, [999]);
        assert!(!target.registrations_raw_mode && !target.registrations.checked);
        assert_eq!(target.registrations.mismatches, 0);
        source.registrations.rows[0].wings_start = usize::MAX;
        assert!(!source.registrations_memo_complete());
        source.registrations.rows[0].wings_start = 0;
        source.wings.required = 1;
        assert!(!source.registrations_memo_complete());
        source.wings.required = 5;
        source.wings.len = 9;
        assert!(!source.registrations_memo_complete());
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
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
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
    /// The same row with its span cleared, so two censuses compare by value.
    fn without_span(&self) -> Self;
}

impl ArenaRow<u32> for radar_group::Row {
    fn key(&self) -> u128 {
        (u128::from(self.country) << 32) | u128::from(self.ordinal)
    }
    fn span(&self) -> (usize, usize) {
        (self.provinces_start, self.provinces_len)
    }
    fn without_span(&self) -> Self {
        Self {
            provinces_start: 0,
            provinces_len: 0,
            ..*self
        }
    }
}

impl ArenaRow<u32> for ccontroller_area::Row {
    fn key(&self) -> u128 {
        (u128::from(self.theatre) << 32) | u128::from(self.ordinal)
    }
    fn span(&self) -> (usize, usize) {
        (self.provinces_start, self.provinces_len)
    }
    fn without_span(&self) -> Self {
        Self {
            provinces_start: 0,
            provinces_len: 0,
            ..*self
        }
    }
}

impl ArenaRow<u64> for cair_base::RegistrationRow {
    fn key(&self) -> u128 {
        (u128::from(self.base) << 32) | u128::from(self.slot)
    }
    fn span(&self) -> (usize, usize) {
        (self.wings_start, self.wings_len)
    }
    fn without_span(&self) -> Self {
        Self {
            wings_start: 0,
            wings_len: 0,
            ..*self
        }
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

fn effective<R>(captured: &CaptureRows<R>) -> Outcome {
    if captured.checked && captured.mismatches != 0 {
        Outcome::Unavailable {
            reason: "table changed during double read",
        }
    } else {
        captured.outcome
    }
}

fn valid_arena<R: ArenaRow<T>, T>(outcome: Outcome, rows: &[R], arena: &Arena<T>) -> bool {
    arena.len <= arena.first.len()
        && arena.required >= arena.len
        && rows.get(..outcome.len()).is_some_and(|rows| {
            rows.iter().all(|row| {
                let (start, len) = row.span();
                start
                    .checked_add(len)
                    .is_some_and(|end| end <= arena.len)
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

/// Compares two independent censuses that share one arena layout. Rows are
/// ordered by key first, because a walk order is not a census identity, and
/// their arena spans are compared by value rather than by offset.
fn arena_second<R: ArenaRow<T>, T: Default + Clone + PartialEq>(
    captured: &mut CaptureRows<R>,
    arena: &mut Arena<T>,
    outcome: Outcome,
    len: usize,
    required: usize,
) -> bool {
    captured.checked = true;
    let outcome = arena_outcome(outcome, &captured.second, arena.second.len(), len, required);
    if let (Outcome::Rows { len: first_rows }, Outcome::Rows { len: second_rows }) =
        (captured.outcome, outcome)
    {
        captured.rows[..first_rows].sort_unstable_by_key(ArenaRow::key);
        captured.second[..second_rows].sort_unstable_by_key(ArenaRow::key);
        captured.mismatches = usize::from(
            first_rows != second_rows || arena.len != len || arena.required != required,
        ) + captured.rows[..first_rows]
            .iter()
            .zip(&captured.second[..second_rows])
            .filter(|(first, second)| {
                let (first_start, first_len) = first.span();
                let (second_start, second_len) = second.span();
                first.without_span() != second.without_span()
                    || arena.first.get(first_start..first_start + first_len)
                        != arena.second.get(second_start..second_start + second_len)
            })
            .count();
        if captured.rows[..first_rows]
            .windows(2)
            .any(|rows| rows[0].key() == rows[1].key())
            || captured.second[..second_rows]
                .windows(2)
                .any(|rows| rows[0].key() == rows[1].key())
        {
            captured.mismatches += 1;
        }
    } else {
        // Two partial censuses need not contain the same prefix.
        captured.mismatches = usize::from(
            matches!(captured.outcome, Outcome::Rows { .. })
                || matches!(outcome, Outcome::Rows { .. }),
        );
    }
    arena.required = arena.required.max(required);
    full_match(captured.outcome, outcome, captured.mismatches)
}

/// These jobs own the final native table inputs. Source witnesses exist only
/// during the independent cold checks; no address lookup or memory replay is
/// used to install a payload on the worker.
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
    checking: bool,
    source_witness: RawLog<1>,
    raw_bytes: u64,
    capacity_charge: crate::raw_memory::Charge,
}

pub(crate) type RadarJob = OwnedFogJob<radar_group::Row>;
pub(crate) type IntelJob = OwnedFogJob<unit_viewer_entry::Row>;

impl<R: Default + Copy> Default for OwnedFogJob<R> {
    fn default() -> Self {
        Self::with_storage(0, 0, 0, 0)
    }
}

impl<R: Default + Copy> OwnedFogJob<R> {
    fn with_storage(rows: usize, provinces: usize, spans: usize, bytes: usize) -> Self {
        let mut job = Self {
            rows: vec![R::default(); rows],
            provinces: vec![0; provinces],
            outcome: Outcome::Rows { len: 0 },
            provinces_len: 0,
            required_provinces: 0,
            row_limit: 0,
            province_limit: 0,
            acquired: false,
            checking: false,
            source_witness: RawLog::new(spans, bytes),
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
        self.raw_bytes = 0;
        self.source_witness.clear();
    }

    pub fn initialized(&self) -> bool {
        self.rows.capacity() != 0
    }

    pub fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * size_of::<R>()
            + self.provinces.capacity() * size_of::<u32>()
            + self.source_witness.spans.capacity() * size_of::<crate::raw::Span>()
            + self.source_witness.bytes.capacity()
    }

    pub fn grow_for_next(&self) -> bool {
        self.row_limit > self.rows.len()
            || self.province_limit > self.provinces.len()
            || self.source_witness.grow_for_next()
            || (self.checking && self.source_witness.bytes.capacity() == 0)
            || (self.acquired && !self.checking && self.source_witness.bytes.capacity() != 0)
    }

    /// Called only during worker-side recycling, including witness retirement.
    pub fn grow(&mut self) {
        if self.row_limit > self.rows.len() {
            self.rows.resize(self.row_limit, R::default());
        }
        if self.province_limit > self.provinces.len() {
            self.provinces.resize(self.province_limit, 0);
        }
        if self.checking {
            if self.source_witness.bytes.capacity() == 0 {
                self.source_witness = RawLog::new(1 << 15, 2 << 20);
            }
            if self.source_witness.grow_for_next() {
                self.source_witness.grow();
            }
        } else if self.acquired {
            self.source_witness = RawLog::default();
        }
        self.update_capacity_charge();
    }

    pub fn note_growth(&self) {
        if self.grow_for_next() {
            crate::raw::publish_growth(std::any::type_name::<R>(), serde_json::json!({
                "row_capacity": self.rows.len(), "required_rows": self.row_limit,
                "province_capacity": self.provinces.len(), "required_provinces": self.province_limit,
                "cold_witness_overflow": self.source_witness.overflowed,
                "cold_witness_retiring": self.acquired && !self.checking && self.source_witness.bytes.capacity() != 0,
            }));
        }
    }

    fn prepare(&mut self, rows: usize, provinces: usize, checking: bool) {
        self.clear();
        self.row_limit = rows;
        self.province_limit = provinces;
        self.checking = checking;
    }

    fn fits(&self, rows: usize, provinces: usize) -> bool {
        rows <= self.rows.len() && provinces <= self.provinces.len()
    }

    fn matches(&self, second: &Self) -> bool
    where
        R: PartialEq,
    {
        self.acquired && second.acquired && self.checking && second.checking
            && !self.source_witness.overflowed && !second.source_witness.overflowed
            && !self.source_witness.fault[0] && !second.source_witness.fault[0]
            && self.source_witness.partition_equal(&second.source_witness, 0)
            && self.outcome == second.outcome
            && self.rows[..self.outcome.len()] == second.rows[..second.outcome.len()]
            && self.provinces_len == second.provinces_len
            && self.required_provinces == second.required_provinces
            && self.provinces[..self.provinces_len] == second.provinces[..second.provinces_len]
    }
}

impl RadarJob {
    pub fn new() -> Self {
        Self::with_storage(1024, 65_536, 1 << 14, 1 << 20)
    }
}

impl IntelJob {
    pub fn new() -> Self {
        Self::with_storage(32_768, 0, 1 << 15, 2 << 20)
    }
}

/// A source counter in warm acquisition and an exact independent observation
/// witness during cold acquisition. It never serves a previously read value.
struct FogMemory<'a, M> {
    inner: &'a M,
    witness: Option<std::cell::RefCell<&'a mut RawLog<1>>>,
    bytes: std::cell::Cell<u64>,
}

impl<M: Memory> FogMemory<'_, M> {
    fn count(&self, bytes: usize) {
        self.bytes.set(self.bytes.get().saturating_add(bytes as u64));
    }
}

impl<M: Memory> Memory for FogMemory<'_, M> {
    fn authenticated_countries(&self, base: usize, state: usize) -> Option<ccountry::AuthenticatedRoster<'_>> {
        if self.witness.is_some() { None } else { self.inner.authenticated_countries(base, state) }
    }

    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let succeeded = if let Some(witness) = &self.witness {
            let mut witness = witness.borrow_mut();
            if let Some(start) = witness.take(self.inner, address, destination.len(), 1) {
                destination.copy_from_slice(&witness.bytes[start..start + destination.len()]);
                true
            } else { false }
        } else {
            self.inner.copy(address, destination)
        };
        if succeeded { self.count(destination.len()); }
        succeeded
    }

    fn copy_scattered<const K: usize>(&self, addresses: [usize; K], destinations: &mut [&mut [u8]; K]) -> usize {
        let completed = if let Some(witness) = &self.witness {
            let mut witness = witness.borrow_mut();
            let lengths = std::array::from_fn(|index| destinations[index].len());
            let Some((starts, completed)) = witness.take_scattered(self.inner, addresses, lengths, [1; K]) else { return 0; };
            for index in 0..completed {
                destinations[index].copy_from_slice(&witness.bytes[starts[index]..starts[index] + lengths[index]]);
            }
            completed
        } else {
            self.inner.copy_scattered(addresses, destinations)
        };
        for field in destinations.iter().take(completed) { self.count(field.len()); }
        completed
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        // Warm shared-source batches retain one native guarded call. Cold
        // captures keep their original ordered source observations instead.
        if self.witness.is_none() {
            let completed = self.inner.copy_records(sources, layout, destination);
            self.count(layout.completed_bytes(completed));
            return completed;
        }
        if sources.len().checked_mul(layout.field_count()).is_none()
            || sources
                .len()
                .checked_mul(layout.stride())
                .is_none_or(|bytes| bytes > destination.len())
        {
            return 0;
        }
        let mut completed = 0;
        for index in 0..sources.len() {
            for field in layout.fields() {
                let Some(start) = index
                    .checked_mul(layout.stride())
                    .and_then(|start| start.checked_add(field.destination_offset))
                else {
                    return completed;
                };
                let Some(end) = start.checked_add(field.len) else { return completed; };
                let Some(output) = destination.get_mut(start..end) else { return completed; };
                if !output.is_empty()
                    && !sources
                        .pointer(index)
                        .filter(|&pointer| pointer != 0)
                        .and_then(|pointer| pointer.checked_add(field.source_offset))
                        .filter(|&address| address.checked_add(field.len).is_some())
                        .is_some_and(|address| self.copy(address, output))
                {
                    return completed;
                }
                completed += 1;
            }
        }
        completed
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct RadarCopyReport {
    pub raw_ok: bool,
    pub matched: [bool; RADAR_N],
    pub mismatches: [usize; RADAR_N],
    pub copy_ticks: [u64; RADAR_N],
    /// Actual successful source bytes, including the one native acquisition.
    pub raw_bytes: [u64; RADAR_N],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RadarCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub needed: [bool; RADAR_N],
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct IntelCopyReport {
    pub raw_ok: bool,
    pub matched: [bool; INTEL_N],
    pub mismatches: [usize; INTEL_N],
    pub copy_ticks: [u64; INTEL_N],
    /// Actual successful source bytes; shared census costs remain with Divisions.
    pub raw_bytes: [u64; INTEL_N],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct IntelCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub needed: [bool; INTEL_N],
}

fn acquire_radar(job: &mut RadarJob, memory: &impl Memory, ctx: RadarCopyContext) {
    let source = FogMemory {
        inner: memory,
        witness: job.checking.then(|| std::cell::RefCell::new(&mut job.source_witness)),
        bytes: std::cell::Cell::new(0),
    };
    let result = radar_group::walk(&source, ctx.base, ctx.game_state,
        &mut job.rows[..job.row_limit], &mut job.provinces[..job.province_limit]);
    job.outcome = result.outcome;
    job.provinces_len = result.provinces_len;
    job.required_provinces = result.required_provinces;
    job.raw_bytes = source.bytes.get();
    job.acquired = true;
}

/// Independent pointer-census passes captured before the fog acquisition.
/// The match verdict excludes all fields private to the Divisions table.
#[derive(Clone, Copy)]
pub(crate) struct IntelDivisionSources<'a> {
    pub first: unit_viewer_entry::DivisionSources<'a>,
    pub second: Option<unit_viewer_entry::DivisionSources<'a>>,
    pub matched: bool,
}

fn acquire_intel(job: &mut IntelJob, memory: &impl Memory, ctx: IntelCopyContext, divisions: Option<unit_viewer_entry::DivisionSources<'_>>) {
    let source = FogMemory {
        inner: memory,
        witness: job.checking.then(|| std::cell::RefCell::new(&mut job.source_witness)),
        bytes: std::cell::Cell::new(0),
    };
    job.outcome = unit_viewer_entry::walk_with_divisions(&source, ctx.base, ctx.game_state, &mut job.rows[..job.row_limit], divisions);
    job.raw_bytes = source.bytes.get();
    job.acquired = true;
}

impl Captured {
    pub(crate) fn copy_radar_refresh(&mut self, memory: &impl Memory, ctx: RadarCopyContext,
        first: &mut RadarJob, mut second: Option<&mut RadarJob>) -> RadarCopyReport {
        let started = qpc();
        let row_limit = self.radar.rows.len();
        let province_limit = self.radar_provinces.first.len();
        let fits = first.fits(row_limit, province_limit) && second.as_deref().is_none_or(|job| job.fits(row_limit, province_limit));
        first.prepare(row_limit, province_limit, ctx.needed[0]);
        if fits { acquire_radar(first, memory, ctx); }
        let mut matched = !ctx.needed[0];
        if let Some(second) = second.as_deref_mut() {
            second.prepare(row_limit, province_limit, ctx.needed[0]);
            if fits { acquire_radar(second, memory, ctx); }
            matched = first.matches(second);
        }
        self.radar.checked = ctx.needed[0];
        self.radar.mismatches = usize::from(ctx.needed[0] && !matched);
        RadarCopyReport {
            raw_ok: first.acquired && !first.source_witness.overflowed && second.is_none_or(|job| job.acquired && !job.source_witness.overflowed),
            matched: [matched && matches!(first.outcome, Outcome::Rows { .. })],
            mismatches: [self.radar.mismatches], copy_ticks: [elapsed(started)], raw_bytes: [first.raw_bytes],
        }
    }

    pub(crate) fn decode_radar_payload(&mut self, _base: usize, job: &RadarJob, decode_ticks: &mut [u64; RADAR_N]) {
        let started = qpc();
        if !job.acquired || job.row_limit != self.radar.rows.len() || job.province_limit != self.radar_provinces.first.len() {
            self.radar.outcome = Outcome::Unavailable { reason: "radar owned payload capacity mismatch" };
        } else {
            let len = job.outcome.len();
            self.radar.rows[..len].copy_from_slice(&job.rows[..len]);
            self.radar_provinces.first[..job.provinces_len].copy_from_slice(&job.provinces[..job.provinces_len]);
            arena_first(&mut self.radar, &mut self.radar_provinces, job.outcome, job.provinces_len, job.required_provinces);
            if self.radar.checked && matches!(self.radar.outcome, Outcome::Rows { .. }) {
                let mut keys: Vec<u128> = self.radar.rows[..len].iter().map(ArenaRow::key).collect();
                keys.sort_unstable();
                if keys.windows(2).any(|pair| pair[0] == pair[1]) { self.radar.mismatches += 1; }
            }
        }
        decode_ticks[0] += elapsed(started);
        self.radar_raw_mode = false;
    }

    #[cfg(test)]
    pub(crate) fn copy_intel_refresh(&mut self, memory: &impl Memory, ctx: IntelCopyContext,
        first: &mut IntelJob, second: Option<&mut IntelJob>) -> IntelCopyReport {
        self.copy_intel_refresh_with_divisions(memory, ctx, first, second, None)
    }

    pub(crate) fn copy_intel_refresh_with_divisions(&mut self, memory: &impl Memory, ctx: IntelCopyContext,
        first: &mut IntelJob, mut second: Option<&mut IntelJob>, divisions: Option<IntelDivisionSources<'_>>) -> IntelCopyReport {
        let started = qpc();
        let row_limit = self.intel.rows.len();
        let fits = first.fits(row_limit, 0) && second.as_deref().is_none_or(|job| job.fits(row_limit, 0));
        first.prepare(row_limit, 0, ctx.needed[0]);
        if fits { acquire_intel(first, memory, ctx, divisions.map(|sources| sources.first)); }
        let mut matched = !ctx.needed[0];
        if let Some(second) = second.as_deref_mut() {
            second.prepare(row_limit, 0, ctx.needed[0]);
            if fits { acquire_intel(second, memory, ctx, divisions.map(|sources| sources.second.unwrap_or(unit_viewer_entry::DivisionSources {
                entries: &[], outcome: Outcome::Unavailable { reason: "independent division census not supplied" },
            }))); }
            matched = first.matches(second)
                && divisions.is_none_or(|sources| sources.second.is_some() && sources.matched);
        }
        self.intel.checked = ctx.needed[0];
        self.intel.mismatches = usize::from(ctx.needed[0] && !matched);
        IntelCopyReport {
            raw_ok: first.acquired && !first.source_witness.overflowed && second.is_none_or(|job| job.acquired && !job.source_witness.overflowed),
            matched: [matched && matches!(first.outcome, Outcome::Rows { .. })],
            mismatches: [self.intel.mismatches], copy_ticks: [elapsed(started)], raw_bytes: [first.raw_bytes],
        }
    }

    pub(crate) fn decode_intel_payload(&mut self, _base: usize, job: &IntelJob, decode_ticks: &mut [u64; INTEL_N]) {
        let started = qpc();
        if !job.acquired || job.row_limit != self.intel.rows.len() {
            self.intel.outcome = Outcome::Unavailable { reason: "intel owned payload capacity mismatch" };
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
    use crate::sampling::CONSISTENCY_TARGET;
    use engine::{
        carmy, cfleet, cgame_state, cprovince, cstate, ctask_force, cunit, memory::SliceMemory,
    };
    use replay::rows::TableSchema;
    use std::cell::Cell;

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
        array(&mut bytes, COUNTRY + ccountry::VOLUNTEER_TALLIES, TALLIES, 1);
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
        array(&mut bytes, THEATRE + ctheatre::CONTROLLER_AREA_DATA, 0x10c00, 1);
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
        array(&mut bytes, THEATRE + ctheatre::STATIONED_UNIT_DATA, 0x10e00, 1);
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
        put(&mut bytes, FLEET, &(cfleet::VTABLE_RVA as u64).to_le_bytes());
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
        array(&mut bytes, AIR_BASE + cair_base::COUNTRY_CONTAINERS, 0x11700, 2);
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
        array(
            &mut bytes,
            POOL + engine::cair_wing_pool::WINGS,
            0x11900,
            1,
        );
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
        let captured = Captured::with_capacities(&capacities, &[0; N]);
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
        let (matched, _) = captured.capture(&memory, 0, BASE, &[true; N]);
        assert_eq!(matched, [true; N]);
        assert_eq!(captured.checked_mismatches(), [0; N]);
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
            (intel[0].viewer, intel[0].level, intel[1].viewer, intel[1].level),
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
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let (matched, _) = captured.capture(&memory, 0, BASE, &[true; N]);
        assert!(!matched[0]);
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
        captured.grow(&mut capacities, &[0; N]);
        assert!(capacities.radar_provinces >= 2);
        let (matched, _) = captured.capture(&memory, 0, BASE, &[true; N]);
        assert_eq!(matched, [true; N]);
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
        let (matched, _) = captured.capture(&memory, 0, BASE, &[true; N]);
        assert_eq!(matched, [false, true, true, false, true, true]);
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
        let (matched, _) = captured.capture(&memory, 0, BASE, &[true; N]);
        assert_eq!(matched, [true; N]);
        assert_eq!(captured.outcomes(), [Outcome::Rows { len: 0 }; N]);
        for view in captured.tables() {
            assert!(matches!(&view, TableView::Complete(rows) if rows.is_empty()));
        }
    }

    struct ChangingLevel<'a> {
        bytes: &'a [u8],
        copies: Cell<usize>,
    }
    impl Memory for ChangingLevel<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if !(SliceMemory {
                base: BASE,
                bytes: self.bytes,
            })
            .copy(address, output)
            {
                return false;
            }
            if address == VIEWERS {
                let copies = self.copies.get();
                self.copies.set(copies + 1);
                if copies != 0 {
                    output[unit_viewer_entry::LEVEL] ^= 1;
                }
            }
            true
        }
    }

    #[test]
    fn a_changed_table_is_unavailable_and_finished_windows_release_their_buffers() {
        let (mut capacities, mut captured) = small();
        let bytes = fixture();
        let memory = ChangingLevel {
            bytes: &bytes,
            copies: Cell::new(0),
        };
        let (matched, _) = captured.capture(&memory, 0, BASE, &[true; N]);
        assert_eq!(matched, [true, true, true, true, false, true]);
        assert_eq!(captured.checked_mismatches()[4], 1);
        assert!(matches!(
            captured.tables()[4],
            TableView::Unavailable { .. }
        ));
        let before = captured.allocated_bytes();
        let mut successful = [CONSISTENCY_TARGET; N];
        successful[4] = 0;
        captured.grow(&mut capacities, &successful);
        assert!(captured.radar.second.is_empty());
        assert!(captured.radar_provinces.second.is_empty());
        assert!(!captured.intel.second.is_empty());
        assert!(captured.allocated_bytes() < before);
    }

    #[test]
    fn an_arena_census_that_disagrees_only_in_its_payload_is_a_mismatch() {
        let (_, mut captured) = small();
        let mut radar = CaptureRows::<radar_group::Row>::new(2, 0);
        let mut arena = Arena::<u32>::new(4, 0);
        radar.rows[0] = radar_group::Row {
            country: 1,
            ordinal: 0,
            state: None,
            provinces_start: 0,
            provinces_len: 2,
        };
        radar.second[0] = radar.rows[0];
        arena.first[..2].copy_from_slice(&[7, 8]);
        arena.second[..2].copy_from_slice(&[7, 9]);
        arena_first(&mut radar, &mut arena, Outcome::Rows { len: 1 }, 2, 2);
        assert!(!arena_second(
            &mut radar,
            &mut arena,
            Outcome::Rows { len: 1 },
            2,
            2
        ));
        assert_eq!(radar.mismatches, 1);
        assert!(matches!(effective(&radar), Outcome::Unavailable { .. }));
        // A span outside the reported arena length can never publish.
        captured.radar.rows[0] = radar_group::Row {
            provinces_start: 1,
            provinces_len: usize::MAX,
            ..Default::default()
        };
        captured.radar.outcome = Outcome::Rows { len: 1 };
        captured.radar_provinces.len = 1;
        captured.radar_provinces.required = 1;
        assert!(matches!(
            captured.tables()[0],
            TableView::Unavailable { .. }
        ));
    }

    #[test]
    fn newly_prepared_fog_jobs_keep_cold_witnesses_until_a_warm_capture() {
        let mut radar = RadarJob::new();
        let mut intel = IntelJob::new();
        assert!(!radar.grow_for_next());
        assert!(!intel.grow_for_next());
        let radar_bytes = radar.allocated_bytes();
        let intel_bytes = intel.allocated_bytes();
        // Prewarming runs the normal worker recycling path before a capture.
        // Neither clear nor an explicit growth call may retire untouched proof
        // storage; the first cold capture needs both independently owned pools.
        radar.grow();
        intel.grow();
        radar.clear();
        intel.clear();
        assert_eq!(radar.allocated_bytes(), radar_bytes);
        assert_eq!(intel.allocated_bytes(), intel_bytes);
        assert!(radar.source_witness.bytes.capacity() != 0);
        assert!(intel.source_witness.bytes.capacity() != 0);
    }

    #[test]
    fn owned_fog_witness_retires_and_regrows_offthread_without_accepting_missing_checks() {
        use super::intel_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0; N]);
        let mut first = IntelJob::default();
        let mut second = IntelJob::new();
        let mut ctx = IntelCopyContext { base: kit::BASE, game_state: kit::GAME_STATE, needed: [true] };
        let report = captured.copy_intel_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(!report.raw_ok);
        assert!(first.grow_for_next());
        first.grow();
        first.clear();
        let report = captured.copy_intel_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok && report.matched[0]);
        let before = first.allocated_bytes();
        ctx.needed = [false];
        assert!(captured.copy_intel_refresh(&memory, ctx, &mut first, None).raw_ok);
        assert!(first.grow_for_next());
        first.grow();
        first.clear();
        assert!(first.allocated_bytes() < before);
        assert_eq!(first.source_witness.bytes.capacity(), 0);
        let warm_capacity = first.allocated_bytes();
        assert!(captured.copy_intel_refresh(&memory, ctx, &mut first, None).raw_ok);
        assert_eq!(first.allocated_bytes(), warm_capacity);
        ctx.needed = [true];
        assert!(!captured.copy_intel_refresh(&memory, ctx, &mut first, Some(&mut second)).raw_ok);
        first.grow();
        first.clear();
        let report = captured.copy_intel_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok && report.matched[0]);
        let report = captured.copy_intel_refresh(&memory, ctx, &mut first, None);
        assert!(report.raw_ok);
        assert!(!report.matched[0]);
        assert_eq!(captured.intel.mismatches, 1);
    }

    #[test]
    fn cold_intel_check_retains_unpublished_native_timer_and_shared_census_verdict() {
        use super::intel_test_kit as kit;
        struct ChangingTimer<'a> { bytes: &'a [u8], calls: Cell<usize> }
        impl Memory for ChangingTimer<'_> {
            fn copy(&self, address: usize, out: &mut [u8]) -> bool {
                if !kit::memory(self.bytes).copy(address, out) { return false; }
                if address == kit::VIEWERS {
                    let calls = self.calls.replace(self.calls.get() + 1);
                    if calls != 0 { out[unit_viewer_entry::TIMER] ^= 1; }
                }
                true
            }
        }
        let bytes = kit::fixture();
        let memory = ChangingTimer { bytes: &bytes, calls: Cell::new(0) };
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0; N]);
        let ctx = IntelCopyContext { base: kit::BASE, game_state: kit::GAME_STATE, needed: [true] };
        let mut first = IntelJob::new();
        let mut second = IntelJob::new();
        let report = captured.copy_intel_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(&first.rows[..first.outcome.len()], &second.rows[..second.outcome.len()]);
        assert!(!report.matched[0]);
        let entries = [unit_viewer_entry::DivisionSource { pointer: kit::DIVISION, canonical: 1 }];
        let pass = unit_viewer_entry::DivisionSources { entries: &entries, outcome: Outcome::Rows { len: 1 } };
        for (second_source, matched) in [(Some(pass), false), (None, true)] {
            let report = captured.copy_intel_refresh_with_divisions(&kit::memory(&bytes), ctx, &mut first, Some(&mut second),
                Some(IntelDivisionSources { first: pass, second: second_source, matched }));
            assert!(report.raw_ok);
            assert!(!report.matched[0]);
        }
    }

    #[test]
    fn radar_tables_are_hourly_tier_h_with_no_refresh_gate() {
        use replay::rows::TableId;
        assert!(!replay::cadence::is_staggered(TableId::RadarCoverage));
        for hour in [0, 1, 2, 3, 60_820_531] {
            assert!(replay::cadence::is_refresh(TableId::RadarCoverage, hour));
        }
    }

    #[test]
    fn radar_split_copy_matches_direct_walk() {
        use super::radar_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let (split, report, _) = kit::split(&memory);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 1]);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.outcome, Outcome::Rows { len: 1 });
        assert_eq!(
            (split.rows[0].country, split.rows[0].ordinal, split.rows[0].state),
            (1, 0, Some(77))
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
            assert_eq!(report.matched, [matches!(direct.outcome, Outcome::Rows { .. })], "rows={rows} provinces={provinces}");
            assert_eq!(report.mismatches, [0]);
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
                memory: SliceMemory { base: 0, bytes: &bytes },
                fault: fault..fault + 1,
            };
            let direct = kit::direct(&memory);
            let (split, report, _) = kit::split(&memory);
            assert!(report.raw_ok, "fault={fault:x}");
            kit::assert_matches_direct(&split, &direct);
            assert_eq!(report.matched, [false; 1], "fault={fault:x}");
        }
    }

    #[test]
    fn radar_double_copy_compare_blocks_checks_on_tearing() {
        use super::radar_test_kit as kit;
        use super::{RadarCopyContext, RadarJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = RadarCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            needed: [true; 1],
        };
        let capacities = super::Capacities {
            rows: [1024, 4, 4, 4, 4, 4],
            radar_provinces: 65_536,
            area_provinces: 8,
            wings: 8,
        };
        let mut captured = super::Captured::with_capacities(&capacities, &[0; 6]);
        let mut first = RadarJob::new();
        let report = captured.copy_radar_refresh(&memory, ctx, &mut first, None);
        assert!(report.raw_ok);
        let mut torn = bytes.clone();
        torn[kit::GROUP + engine::radar_group::STATE + 2] ^= 0xff;
        let torn_memory = kit::memory(&torn);
        let mut second = RadarJob::new();
        let mut captured2 = super::Captured::with_capacities(&capacities, &[0; 6]);
        let torn_report = captured2.copy_radar_refresh(&torn_memory, ctx, &mut second, None);
        assert!(torn_report.raw_ok);
        assert!(!first.matches(&second));
    }

    #[test]
    fn radar_staging_overflow_falls_back_to_typed_walk() {
        use super::radar_test_kit as kit;
        use super::{RadarCopyContext, RadarJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = RadarCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            needed: [true; 1],
        };
        let mut tiny = RadarJob::default();
        let capacities = super::Capacities {
            rows: [1024, 4, 4, 4, 4, 4],
            radar_provinces: 65_536,
            area_provinces: 8,
            wings: 8,
        };
        let mut captured = super::Captured::with_capacities(&capacities, &[0; 6]);
        let report = captured.copy_radar_refresh(&memory, ctx, &mut tiny, None);
        assert!(!report.raw_ok);
    }

    #[test]
    fn radar_pools_cover_appendix_worst_case() {
        // Worst-case caps proof: the steady-state pools exceed a
        // conservative span/byte estimate built from the appendix census (18
        // coverage groups over 439 countries), so live refreshes never take
        // the overflow fallback in steady state.
        use super::RadarJob;
        let job = RadarJob::new();
        let spans = 2 + 439 * 8 + 18 * 8;
        assert!(
            job.source_witness.spans.capacity() >= spans,
            "span cap {} < worst case {spans}",
            job.source_witness.spans.capacity()
        );
        let bytes = 439 * 64 + 18 * 128;
        assert!(
            job.source_witness.bytes.capacity() >= bytes,
            "byte cap {} < worst case {bytes}",
            job.source_witness.bytes.capacity()
        );
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
            assert!(sizes.contains(&required), "missing size {required} in {sizes:?}");
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
        assert_eq!(report.matched, [true; 1]);
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
            assert_eq!(report.matched, [matches!(direct.outcome, Outcome::Rows { .. })], "cap={cap}");
            assert_eq!(report.mismatches, [0]);
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
                memory: SliceMemory { base: 0, bytes: &bytes },
                fault: fault..fault + 1,
            };
            let direct = kit::direct(&memory);
            let (split, report, _) = kit::split(&memory);
            assert!(report.raw_ok, "fault={fault:x}");
            kit::assert_matches_direct(&split, &direct);
            kit::assert_paths_match_direct(&split, &direct);
            assert_eq!(report.matched, [false; 1], "fault={fault:x}");
        }
    }

    #[test]
    fn intel_double_copy_compare_blocks_checks_on_tearing() {
        use super::intel_test_kit as kit;
        use super::{IntelCopyContext, IntelJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = IntelCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            needed: [true; 1],
        };
        let capacities = super::Capacities {
            rows: [4, 4, 4, 4, 32_768, 4],
            radar_provinces: 8,
            area_provinces: 8,
            wings: 8,
        };
        let mut captured = super::Captured::with_capacities(&capacities, &[0; 6]);
        let mut first = IntelJob::new();
        let report = captured.copy_intel_refresh(&memory, ctx, &mut first, None);
        assert!(report.raw_ok);
        let mut torn = bytes.clone();
        torn[kit::VIEWERS + engine::unit_viewer_entry::LEVEL] ^= 0xff;
        let torn_memory = kit::memory(&torn);
        let mut second = IntelJob::new();
        let mut captured2 = super::Captured::with_capacities(&capacities, &[0; 6]);
        let torn_report = captured2.copy_intel_refresh(&torn_memory, ctx, &mut second, None);
        assert!(torn_report.raw_ok);
        assert!(!first.matches(&second));
    }

    #[test]
    fn cold_intel_witness_covers_native_census_and_viewer_inputs() {
        // The cold direct-source path witnesses the full native acquisition,
        // including unpublished timer/padding bytes. Warm jobs retain no log.
        use super::intel_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let (_split, report, first) = kit::split(&memory);
        assert!(report.raw_ok);
        // Every span address in the log appears with the multiplicity the
        // typed walk reads it with; census addresses serve both unit paths
        // without a second copy of the walk.
        let census_addrs = [
            kit::GAME_STATE + engine::cgame_state::COUNTRIES,
            kit::GAME_STATE + engine::cgame_state::COUNTRY_LINKS,
            kit::C0 + engine::ccountry::DIVISIONS,
            kit::C0 + engine::ccountry::FLEETS,
        ];
        for addr in census_addrs {
            assert!(
                first.source_witness.spans.iter().any(|span| span.addr == addr),
                "census span missing at {addr:x}"
            );
        }
        // Viewer spans are taken exactly once per batch read.
        let viewer_takes = first
            .source_witness
            .spans
            .iter()
            .filter(|span| span.addr == kit::VIEWERS && span.len == 48)
            .count();
        assert_eq!(viewer_takes, 1);
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
            needed: [true; 1],
        };
        let mut tiny = IntelJob::default();
        let capacities = super::Capacities {
            rows: [4, 4, 4, 4, 32_768, 4],
            radar_provinces: 8,
            area_provinces: 8,
            wings: 8,
        };
        let mut captured = super::Captured::with_capacities(&capacities, &[0; 6]);
        let report = captured.copy_intel_refresh(&memory, ctx, &mut tiny, None);
        assert!(!report.raw_ok);
    }

    #[test]
    fn intel_pools_cover_appendix_worst_case() {
        // Cold witness storage covers the retained campaign census; the
        // native row pool is independent of witness retirement and growth.
        use super::IntelJob;
        let job = IntelJob::new();
        let spans = 2 + 439 * 7 + 439 * 2 + 3_200 * 2 + 500 * 4 + 24 * 4;
        assert!(
            job.source_witness.spans.capacity() >= spans,
            "span cap {} < worst case {spans}",
            job.source_witness.spans.capacity()
        );
        let bytes = 439 * 64 + 3_200 * 32 + 500 * 64 + 24 * 32;
        assert!(
            job.source_witness.bytes.capacity() >= bytes,
            "byte cap {} < worst case {bytes}",
            job.source_witness.bytes.capacity()
        );
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
            assert!(sizes.contains(&required), "missing size {required} in {sizes:?}");
        }
    }
}

/// Shared synthetic address space for the radar copy/decode harness: one
/// country with one radar group covering two provinces. Mirrors the engine
/// radar fixture shapes.
#[cfg(test)]
pub(crate) mod radar_test_kit {
    use super::{Capacities, Captured, RadarCopyContext, RadarCopyReport, RadarJob};
    use engine::{memory::SliceMemory, walk::Outcome, Memory};

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
        put64(&mut bytes, C0 + ccountry::RADARS_POOL, cradars_pool::VTABLE_RVA as u64);
        put32(&mut bytes, C0 + ccountry::RADARS_POOL_COUNTRY, 1);
        header(&mut bytes, C0 + ccountry::RADAR_GROUPS, GVEC, 1);
        put64(&mut bytes, GVEC, GROUP as u64);
        put64(&mut bytes, GROUP + radar_group::PARENT, (C0 + ccountry::RADARS_POOL) as u64);
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
        let mut captured = Captured::with_capacities(&capacities, &[0; 6]);
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
            base: BASE,
            game_state: GAME_STATE,
            needed: [true; 1],
        };
        let capacities = Capacities {
            rows: [rows_cap, 4, 4, 4, 4, 4],
            radar_provinces: provinces_cap,
            area_provinces: 8,
            wings: 8,
        };
        let mut captured = Captured::with_capacities(&capacities, &[0; 6]);
        let mut first = RadarJob::new();
        let mut second = RadarJob::new();
        let report = captured.copy_radar_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let mut ticks = [0u64; 1];
        captured.decode_radar_payload(BASE, &first, &mut ticks);
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
        assert_eq!(&split.rows[..split.outcome.len()], &direct.rows[..direct.outcome.len()], "radar rows");
        assert_eq!(split.provinces, direct.provinces, "radar provinces");
    }
}

/// Shared synthetic address space for the intel copy/decode harness: one
/// country with one division (two viewer entries) and one task force (no
/// entries). Mirrors the engine viewer-entry fixture shapes.
#[cfg(test)]
pub(crate) mod intel_test_kit {
    use super::{Capacities, Captured, IntelCopyContext, IntelCopyReport, IntelJob};
    use engine::{memory::SliceMemory, walk::Outcome, Memory};

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
            put32(&mut bytes, VIEWERS + index * unit_viewer_entry::STRIDE, viewer);
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
        let mut captured = Captured::with_capacities(&capacities, &[0; 6]);
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
            needed: [true; 1],
        };
        let capacities = Capacities {
            rows: [4, 4, 4, 4, cap, 4],
            radar_provinces: 8,
            area_provinces: 8,
            wings: 8,
        };
        let mut captured = Captured::with_capacities(&capacities, &[0; 6]);
        let mut first = IntelJob::new();
        let mut second = IntelJob::new();
        let report = captured.copy_intel_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let mut ticks = [0u64; 1];
        captured.decode_intel_payload(BASE, &first, &mut ticks);
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

    pub(crate) fn split_with(memory: &impl Memory, cap: usize) -> (Split, IntelCopyReport, IntelJob) {
        collect_split(memory, cap)
    }

    pub(crate) fn assert_matches_direct(split: &Split, direct: &Split) {
        assert_eq!(split.outcome, direct.outcome, "intel outcome");
        assert_eq!(&split.rows[..split.outcome.len()], &direct.rows[..direct.outcome.len()], "intel rows");
    }

    /// Partitioned dual-path proof: division-family and fleet-family rows
    /// decoded from the same shared spans equal the direct walk's
    /// per-family partitions.
    pub(crate) fn assert_paths_match_direct(split: &Split, direct: &Split) {
        use engine::ctask_force;
        // Task-force handles carry the naval type tag in the low word;
        // divisions carry another.
        let is_fleet = |unit: u64| (unit & 0xffff_ffff) == u64::from(ctask_force::DEFAULT_HANDLE_TYPE);
        for fleet in [false, true] {
            let split_path: Vec<_> = split
                .rows[..split.outcome.len()]
                .iter()
                .filter(|row| is_fleet(row.unit) == fleet)
                .collect();
            let direct_path: Vec<_> = direct
                .rows[..direct.outcome.len()]
                .iter()
                .filter(|row| is_fleet(row.unit) == fleet)
                .collect();
            assert_eq!(split_path, direct_path, "intel path fleet={fleet}");
        }
    }
}
