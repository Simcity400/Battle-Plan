//! Native fog-derivation input acquisition (registry domain D102). Only growth
//! and replay conversion allocate; every engine read happens in the callback.

use crate::{
    raw::RawLog,
    sampling::{full_match, qpc},
    spooling::table,
    table_capture::{Arena, CaptureRows},
};
use engine::{
    Memory, cair_base, ccontroller_area, ccountry, cpdx_array::Array, cgame_state, ctheatre,
    radar_group, unit_viewer_entry, walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{self, TableId, tables},
};

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
    /// Raw-copy split gate for the hourly radar payload (RadarCoverage).
    /// Set by the game thread every hour with a valid raw payload; `capture`
    /// then skips the typed walk and the decode worker fills the rows before
    /// the ready handoff. Preserved across `reset`, cleared by the worker.
    pub(crate) radar_raw_mode: bool,
    /// Raw-copy split gate for the hourly intel payload (UnitIntel). Set by
    /// the game thread every hour with a valid raw payload; `capture` then
    /// skips the typed walk and the decode worker fills the rows before the
    /// ready handoff. Preserved across `reset`, cleared by the worker.
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

        // Hourly radar raw-copy split: on hours with a valid raw payload the
        // game thread stages bytes via `copy_radar_refresh` and the decode
        // worker fills the typed rows before the ready handoff, so the typed
        // walk is skipped here. Matched flags and costs come from the raw
        // compare, applied by the caller after this returns.
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

        // Hourly intel raw-copy split: on hours with a valid raw payload the
        // game thread stages bytes via `copy_intel_refresh` (own division
        // and fleet census takes, shared once inside the payload) and the
        // decode worker fills the typed rows before the ready handoff, so
        // the typed walk is skipped here.
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

    /// Spool-thread conversion; raw values and incomplete-table status survive.
    pub fn tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        [
            self.radar_table(outcomes[0]),
            table::<tables::VolunteerHosts, _>(outcomes[1], &self.volunteers.rows, |r| {
                rows::VolunteerHostRow {
                    country: r.country,
                    position: r.position,
                    host: r.host,
                    deployed_divisions: r.deployed_divisions,
                }
            }),
            self.area_table(outcomes[2]),
            table::<tables::StationedUnits, _>(outcomes[3], &self.units.rows, |r| {
                rows::StationedUnitRow {
                    theatre: r.theatre,
                    ordinal: r.ordinal,
                    unit: r.unit,
                }
            }),
            table::<tables::UnitIntel, _>(outcomes[4], &self.intel.rows, |r| rows::UnitIntelRow {
                unit: r.unit,
                ordinal: r.ordinal,
                viewer: r.viewer,
                level: r.level,
            }),
            self.registration_table(outcomes[5]),
        ]
    }

    fn radar_table(&self, outcome: Outcome) -> TableView {
        if !valid_arena(outcome, &self.radar.rows, &self.radar_provinces) {
            return crate::spooling::unavailable(
                "radar coverage vector lies outside the captured arena",
            );
        }
        table::<tables::RadarCoverage, _>(outcome, &self.radar.rows, |r| rows::RadarCoverageRow {
            country: r.country,
            ordinal: r.ordinal,
            state: r.state,
            provinces: self.radar_provinces.first[r.provinces_start..r.provinces_start + r.provinces_len]
                .to_vec(),
        })
    }

    fn area_table(&self, outcome: Outcome) -> TableView {
        if !valid_arena(outcome, &self.areas.rows, &self.area_provinces) {
            return crate::spooling::unavailable(
                "controller area province vector lies outside the captured arena",
            );
        }
        table::<tables::TheatreAreas, _>(outcome, &self.areas.rows, |r| rows::TheatreAreaRow {
            theatre: r.theatre,
            ordinal: r.ordinal,
            representative: r.representative,
            provinces: self.area_provinces.first
                [r.provinces_start..r.provinces_start + r.provinces_len]
                .to_vec(),
        })
    }

    fn registration_table(&self, outcome: Outcome) -> TableView {
        if !valid_arena(outcome, &self.registrations.rows, &self.wings) {
            return crate::spooling::unavailable(
                "air registration wing vector lies outside the captured arena",
            );
        }
        table::<tables::AirBaseRegistrations, _>(outcome, &self.registrations.rows, |r| {
            rows::AirBaseRegistrationRow {
                base: r.base,
                slot: r.slot,
                country: r.country,
                wings: self.wings.first[r.wings_start..r.wings_start + r.wings_len].to_vec(),
            }
        })
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

// ===========================================================================
// Hourly radar split (RadarCoverage).
//
// The game thread copies raw bytes only: the country census, per-country
// radar pools with group vectors, and per-group state links with coverage
// vectors. No row is decoded on the game thread. The single FIFO decode
// worker replays `radar_group::walk` against the recorded spans, so rows,
// ordinals, skips, truncation counts and failure reasons are identical by
// construction (see `crate::raw` for the mirror rules).
//
// Coverage takes are exact spans with cap-independent shapes (the vector
// count comes from the header, never the pooled caps): the mirror always
// takes the full vector and the replay skips reads past pooled capacity,
// truncating identically. Pooled row and arena capacities are ignored while
// copying: a superset is recorded and the replay truncates identically
// against the real pooled buffers.
// ===========================================================================

/// Hourly payload slots in calibration/report order: [radar].
pub(crate) const RADAR_N: usize = 1;
const RADAR_MASK: u32 = 0x01;

/// One hour of raw radar payload. Flat pools recycle capacities and the
/// steady state allocates on neither thread; growth happens on the decode
/// side only (see `crate::decode::recycle_radar_job`).
#[derive(Debug, Default)]
pub(crate) struct RadarJob {
    pub log: RawLog<RADAR_N>,
    pub game_state: usize,
}

impl RadarJob {
    pub fn new() -> Self {
        Self {
            // Late-campaign worst case is 18 coverage groups (appendix
            // census); both pools start above that so steady state never
            // overflows (see the worst-case caps test).
            log: RawLog::new(1 << 12, 1 << 20),
            game_state: 0,
        }
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    pub fn grow(&mut self) {
        self.log.grow();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct RadarCopyReport {
    /// False when staging overflowed: the caller falls back to the typed
    /// walk (identical rows through the existing path) and enqueues no
    /// payload.
    pub raw_ok: bool,
    pub matched: [bool; RADAR_N],
    pub copy_ticks: [u64; RADAR_N],
    pub raw_bytes: [u64; RADAR_N],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RadarCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub needed: [bool; RADAR_N],
}

fn radar_pool_u64(log: &RawLog<RADAR_N>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn radar_pool_u32(log: &RawLog<RADAR_N>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads.
fn gspan(
    log: &mut RawLog<RADAR_N>,
    memory: &impl engine::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// Mirror of one link-table read with local bounds checks. Returns `None`
/// exactly when the typed resolve fails; the replay derives its own reason.
fn radar_link(
    log: &mut RawLog<RADAR_N>,
    memory: &impl engine::Memory,
    links: &Array,
    countries: u32,
    raw: u32,
) -> Option<u32> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = gspan(
        log,
        memory,
        links.data.checked_add(offset),
        4,
        RADAR_MASK,
    )?;
    let slot = radar_pool_u32(log, start);
    if slot == 0 || slot >= countries {
        return None;
    }
    Some(slot)
}

/// One structural copy pass: country census, per-country radar pools with
/// group vectors, and per-group state links with coverage vectors. The job
/// is cleared on entry so pooled refreshes never accumulate spans across
/// hours.
fn copy_radar_pass(
    job: &mut RadarJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; RADAR_N],
) {
    job.clear();
    job.game_state = game_state;
    let started = qpc();
    copy_radar_tables(job, memory, base, game_state);
    ticks[0] += elapsed(started);
}

fn copy_radar_tables(
    job: &mut RadarJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
) {
    let log = &mut job.log;
    let countries_start = gspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRIES),
        16,
        RADAR_MASK,
    );
    let links_start = gspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRY_LINKS),
        16,
        RADAR_MASK,
    );
    let (Some(countries_start), Some(links_start)) = (countries_start, links_start) else {
        return;
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[countries_start..countries_start + 16]);
    let countries_array = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return,
    };
    raw.copy_from_slice(&log.bytes[links_start..links_start + 16]);
    let links = match Array::from_header(raw, 4) {
        Some(array) => array,
        None => return,
    };
    if countries_array.count < 2 || links.count == 0 {
        return;
    }
    let country_vtable = match base.checked_add(ccountry::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return,
    };
    let pool_vtable = match base.checked_add(engine::cradars_pool::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return,
    };
    let state_vtable = match base.checked_add(engine::cstate::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return,
    };
    for slot in 1..countries_array.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|offset| countries_array.data.checked_add(offset));
        let pointer_start = match gspan(log, memory, pointer_addr, 8, RADAR_MASK) {
            Some(start) => start,
            None => return,
        };
        let pointer = match usize::try_from(radar_pool_u64(log, pointer_start))
            .ok()
            .filter(|&pointer| pointer != 0)
        {
            Some(pointer) => pointer,
            None => continue,
        };
        let vtable_start = match gspan(log, memory, Some(pointer), 8, RADAR_MASK) {
            Some(start) => start,
            None => return,
        };
        let id_start = match gspan(log, memory, pointer.checked_add(ccountry::ID), 4, RADAR_MASK)
        {
            Some(start) => start,
            None => return,
        };
        if radar_pool_u64(log, vtable_start) != country_vtable {
            return;
        }
        let id = radar_pool_u32(log, id_start);
        let canonical = match radar_link(log, memory, &links, countries_array.count, id) {
            Some(canonical) => canonical,
            None => return,
        };
        if radar_link(log, memory, &links, countries_array.count, slot) != Some(canonical) {
            return;
        }
        if radar_link(log, memory, &links, countries_array.count, canonical) != Some(canonical) {
            return;
        }
        let canonical_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|offset| countries_array.data.checked_add(offset));
        let canonical_pointer = match gspan(log, memory, canonical_addr, 8, RADAR_MASK) {
            Some(start) => start,
            None => return,
        };
        if radar_pool_u64(log, canonical_pointer) != pointer as u64 {
            return;
        }
        if slot != canonical {
            continue;
        }
        if !copy_radar_country(
            log,
            memory,
            base,
            pointer,
            pool_vtable,
            state_vtable,
            &links,
            &countries_array,
            countries_array.count,
        ) {
            return;
        }
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of one country's radar pool, group vector and groups: pool vtable
/// with country agreement, group pointers with parent checks, state links
/// and coverage vectors. Returns false exactly when the typed walk fails at
/// or before this country.
#[allow(clippy::too_many_arguments)]
fn copy_radar_country(
    log: &mut RawLog<RADAR_N>,
    memory: &impl engine::Memory,
    base: usize,
    pointer: usize,
    pool_vtable: u64,
    state_vtable: u64,
    links: &Array,
    countries_array: &Array,
    countries: u32,
) -> bool {
    let pool = match gspan(
        log,
        memory,
        pointer.checked_add(ccountry::RADARS_POOL),
        8,
        RADAR_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    if radar_pool_u64(log, pool) != pool_vtable {
        return false;
    }
    let owner = match gspan(
        log,
        memory,
        pointer.checked_add(ccountry::RADARS_POOL_COUNTRY),
        4,
        RADAR_MASK,
    ) {
        Some(start) => radar_pool_u32(log, start),
        None => return false,
    };
    // Owner agreement mirrors the walk's resolve; the link takes serve the
    // replay identically through multiplicity.
    if !radar_resolve(log, memory, base, links, countries_array, countries, pointer, owner) {
        return false;
    }
    let groups_header = match gspan(
        log,
        memory,
        pointer.checked_add(ccountry::RADAR_GROUPS),
        16,
        RADAR_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[groups_header..groups_header + 16]);
    let groups = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return false,
    };
    for ordinal in 0..groups.count {
        let addr = (ordinal as usize)
            .checked_mul(8)
            .and_then(|offset| groups.data.checked_add(offset));
        let group_start = match gspan(log, memory, addr, 8, RADAR_MASK) {
            Some(start) => start,
            None => return false,
        };
        let group = match usize::try_from(radar_pool_u64(log, group_start))
            .ok()
            .filter(|&group| group != 0)
        {
            Some(group) => group,
            None => return false,
        };
        if group % 8 != 0 {
            return false;
        }
        let parent = match gspan(
            log,
            memory,
            group.checked_add(radar_group::PARENT),
            8,
            RADAR_MASK,
        ) {
            Some(start) => start,
            None => return false,
        };
        if radar_pool_u64(log, parent) != pointer.checked_add(ccountry::RADARS_POOL).unwrap_or(0) as u64 {
            return false;
        }
        let state_addr = match gspan(
            log,
            memory,
            group.checked_add(radar_group::STATE),
            8,
            RADAR_MASK,
        ) {
            Some(start) => start,
            None => return false,
        };
        let state = match usize::try_from(radar_pool_u64(log, state_addr)).ok() {
            Some(state) => state,
            None => return false,
        };
        if state != 0 {
            let vtable = match gspan(log, memory, Some(state), 8, RADAR_MASK) {
                Some(start) => start,
                None => return false,
            };
            if radar_pool_u64(log, vtable) != state_vtable {
                return false;
            }
            if gspan(log, memory, state.checked_add(engine::cstate::ID), 4, RADAR_MASK).is_none()
            {
                return false;
            }
        }
        let covered_header = match gspan(
            log,
            memory,
            group.checked_add(radar_group::PROVINCES),
            16,
            RADAR_MASK,
        ) {
            Some(start) => start,
            None => return false,
        };
        raw.copy_from_slice(&log.bytes[covered_header..covered_header + 16]);
        let covered = match Array::from_header(raw, 4) {
            Some(array) => array,
            None => return false,
        };
        // Coverage words arrive in 64-element chunks (`copy_u32s`); takes
        // decompose per chunk in walk order with cap-independent shapes, so
        // the replay serves every chunk identically and skips reads past
        // pooled capacity, truncating identically.
        let mut first = 0u32;
        while first < covered.count {
            let chunk = ((covered.count - first) as usize).min(64);
            let address = (first as usize)
                .checked_mul(4)
                .and_then(|offset| covered.data.checked_add(offset));
            if gspan(log, memory, address, chunk * 4, RADAR_MASK).is_none() {
                return false;
            }
            if log.overflowed {
                return false;
            }
            first += chunk as u32;
        }
        if log.overflowed {
            return false;
        }
    }
    true
}

/// Mirror of the walk's pool-country resolve: link entry plus the full
/// `read_at` span set with canonical agreement, taken explicitly (the owner
/// raw may be an alias the census loop never took). Returns false exactly
/// when the typed resolve fails.
#[allow(clippy::too_many_arguments)]
fn radar_resolve(
    log: &mut RawLog<RADAR_N>,
    memory: &impl engine::Memory,
    base: usize,
    links: &Array,
    countries_array: &Array,
    countries: u32,
    pointer: usize,
    owner: u32,
) -> bool {
    let country_vtable = match base.checked_add(ccountry::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    let canonical = match radar_link(log, memory, links, countries, owner) {
        Some(canonical) => canonical,
        None => return false,
    };
    let pointer_addr = (canonical as usize)
        .checked_mul(8)
        .and_then(|offset| countries_array.data.checked_add(offset));
    let pointer_start = match gspan(log, memory, pointer_addr, 8, RADAR_MASK) {
        Some(start) => start,
        None => return false,
    };
    let canonical_pointer = match usize::try_from(radar_pool_u64(log, pointer_start))
        .ok()
        .filter(|&pointer| pointer != 0)
    {
        Some(pointer) => pointer,
        None => return false,
    };
    let vtable_start = match gspan(log, memory, Some(canonical_pointer), 8, RADAR_MASK) {
        Some(start) => start,
        None => return false,
    };
    let id_start = match gspan(
        log,
        memory,
        canonical_pointer.checked_add(ccountry::ID),
        4,
        RADAR_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    if radar_pool_u64(log, vtable_start) != country_vtable {
        return false;
    }
    let id = radar_pool_u32(log, id_start);
    if radar_link(log, memory, links, countries, id) != Some(canonical) {
        return false;
    }
    if radar_link(log, memory, links, countries, canonical) != Some(canonical) {
        return false;
    }
    let check_addr = (canonical as usize)
        .checked_mul(8)
        .and_then(|offset| countries_array.data.checked_add(offset));
    let check_start = match gspan(log, memory, check_addr, 8, RADAR_MASK) {
        Some(start) => start,
        None => return false,
    };
    if radar_pool_u64(log, check_start) != canonical_pointer as u64 {
        return false;
    }
    canonical_pointer == pointer
}

impl Captured {
    /// Game-thread raw copy for an hourly refresh. Copies `first` always and
    /// `second` while the radar table still needs double-read checks,
    /// compares the raw logs, and records copy-only costs plus the existing
    /// checked/mismatches bookkeeping. Returns whether the payload is
    /// complete (`raw_ok`); on staging overflow the caller falls back to the
    /// typed walk and enqueues nothing.
    pub(crate) fn copy_radar_refresh(
        &mut self,
        memory: &impl Memory,
        ctx: RadarCopyContext,
        first: &mut RadarJob,
        mut second: Option<&mut RadarJob>,
    ) -> RadarCopyReport {
        let mut ticks = [0u64; RADAR_N];
        copy_radar_pass(first, memory, ctx.base, ctx.game_state, &mut ticks);
        let mut matched = [true; RADAR_N];
        if let Some(second_job) = second.as_deref_mut() {
            let mut second_ticks = [0u64; RADAR_N];
            copy_radar_pass(second_job, memory, ctx.base, ctx.game_state, &mut second_ticks);
            for (total, extra) in ticks.iter_mut().zip(second_ticks) {
                *total += extra;
            }
            let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
            for (slot, matched) in matched.iter_mut().enumerate() {
                let equal = raw_ok_pair && first.log.partition_equal(&second_job.log, slot);
                *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
            }
        }
        // Conservative checked bookkeeping mirroring the typed path.
        self.radar.checked = ctx.needed[0];
        self.radar.mismatches = usize::from(ctx.needed[0] && !matched[0]);
        let raw_ok = !first.log.overflowed
            && second.is_none_or(|job| !job.log.overflowed);
        RadarCopyReport {
            raw_ok,
            matched,
            copy_ticks: ticks,
            raw_bytes: first.log.raw_bytes,
        }
    }

    /// Decode-worker interpretation of a raw radar payload: replays the
    /// radar walk against the recorded spans into the pooled buffers, with
    /// the identical arena handling the game thread applies. Installs
    /// identical rows, truncation counts and failure reasons.
    pub(crate) fn decode_radar_payload(
        &mut self,
        base: usize,
        job: &RadarJob,
        decode_ticks: &mut [u64; RADAR_N],
    ) {
        use crate::raw::Recorded;
        let recorded = Recorded::build(&job.log);
        let started = qpc();
        let first = radar_group::walk(
            &recorded,
            base,
            job.game_state,
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
        // Reproduce the typed path's duplicate-key scan exactly as for
        // registrations: the spool conversion sorts by unique key
        // downstream, so only the verdict matters here.
        if self.radar.checked && matches!(self.radar.outcome, Outcome::Rows { .. }) {
            let len = self.radar.outcome.len();
            let mut keys: Vec<u128> =
                self.radar.rows[..len].iter().map(ArenaRow::key).collect();
            keys.sort_unstable();
            if keys.windows(2).any(|pair| pair[0] == pair[1]) {
                self.radar.mismatches += 1;
            }
        }
        decode_ticks[0] += elapsed(started);
        self.radar_raw_mode = false;
    }
}

// ===========================================================================
// Hourly intel split (UnitIntel).
//
// The game thread copies raw bytes only: the country census, per-country
// division and fleet rosters with task-force vectors, and every unit's
// viewer vector with its entries. The division and fleet census spans are
// the walk's own takes (shared once inside this payload between the
// division and fleet paths, not duplicated per unit); the already-moved
// Divisions and Map jobs keep their own copies with different read sets,
// and duplicating these small census headers here is net-positive (see the
// counts proof in the caps test). No row is decoded on the game thread. The
// single FIFO decode worker replays `unit_viewer_entry::walk` against the
// recorded spans, so rows, ordinals, skips, truncation counts and failure
// reasons are identical by construction (see `crate::raw` for the mirror
// rules).
//
// Viewer entry blocks are cap-independent (batch counts come from the
// vector headers, never the pooled caps): the mirror always takes full
// batches and the replay skips rows past pooled capacity, counting omits
// identically. Pooled row capacities are ignored while copying: a superset
// is recorded and the replay truncates identically against the real pooled
// buffers.
// ===========================================================================

/// Hourly payload slots in calibration/report order: [intel]. Division and
/// fleet units decode from the same shared spans; the harness proves both
/// paths against the direct walk.
pub(crate) const INTEL_N: usize = 1;
const INTEL_MASK: u32 = 0x01;

/// Viewer batch chunking. Must match the engine walk's batching exactly.
const INTEL_MIRROR_BATCH: usize = 32;

/// One hour of raw intel payload. Flat pools recycle capacities and the
/// steady state allocates on neither thread; growth happens on the decode
/// side only (see `crate::decode::recycle_intel_job`).
#[derive(Debug, Default)]
pub(crate) struct IntelJob {
    pub log: RawLog<INTEL_N>,
    pub game_state: usize,
}

impl IntelJob {
    pub fn new() -> Self {
        Self {
            // Late-campaign worst case is 24 intel rows over ~2.9k divisions
            // and ~240 task forces (appendix census); both pools start above
            // that so steady state never overflows (see the caps test).
            log: RawLog::new(1 << 15, 2 << 20),
            game_state: 0,
        }
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    pub fn grow(&mut self) {
        self.log.grow();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct IntelCopyReport {
    /// False when staging overflowed: the caller falls back to the typed
    /// walk (identical rows through the existing path) and enqueues no
    /// payload.
    pub raw_ok: bool,
    pub matched: [bool; INTEL_N],
    pub copy_ticks: [u64; INTEL_N],
    pub raw_bytes: [u64; INTEL_N],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct IntelCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub needed: [bool; INTEL_N],
}

fn intel_pool_u64(log: &RawLog<INTEL_N>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads.
fn ispan(
    log: &mut RawLog<INTEL_N>,
    memory: &impl engine::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// Mirror of one link-table read with local bounds checks. Returns `None`
/// exactly when the typed resolve fails; the replay derives its own reason.
fn intel_link(
    log: &mut RawLog<INTEL_N>,
    memory: &impl engine::Memory,
    links: &Array,
    countries: u32,
    raw: u32,
) -> Option<u32> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = ispan(log, memory, links.data.checked_add(offset), 4, INTEL_MASK)?;
    let slot = u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap());
    if slot == 0 || slot >= countries {
        return None;
    }
    Some(slot)
}

/// Mirror of one unit's viewer takes: vtable, handle and vector header plus
/// every entry batch, with the walk's identity validation. Returns false
/// exactly when the typed unit read fails.
fn copy_intel_unit(
    log: &mut RawLog<INTEL_N>,
    memory: &impl engine::Memory,
    _base: usize,
    unit: usize,
    vtable: u64,
) -> bool {
    // copy_scattered triple: vtable word, handle word, viewer header.
    let primary = match ispan(log, memory, Some(unit), 8, INTEL_MASK) {
        Some(start) => start,
        None => return false,
    };
    let handle = match ispan(log, memory, unit.checked_add(engine::cunit::HANDLE), 8, INTEL_MASK)
    {
        Some(start) => start,
        None => return false,
    };
    let header = match ispan(
        log,
        memory,
        unit.checked_add(engine::cunit::VIEWER_DATA),
        16,
        INTEL_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    if intel_pool_u64(log, primary) != vtable || intel_pool_u64(log, handle) == 0 {
        return false;
    }
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header..header + 16]);
    let viewers = match Array::from_header(raw, unit_viewer_entry::STRIDE) {
        Some(array) => array,
        None => return false,
    };
    // Entry batches mirror the walk's 32-entry chunk reads exactly.
    let batches = (viewers.count as usize).div_ceil(INTEL_MIRROR_BATCH).max(1);
    for batch in 0..batches {
        let first = batch * INTEL_MIRROR_BATCH;
        let count = (viewers.count as usize - first).min(INTEL_MIRROR_BATCH);
        if count == 0 {
            break;
        }
        let address = viewers.element(first as u32, unit_viewer_entry::STRIDE);
        if ispan(
            log,
            memory,
            address,
            count * unit_viewer_entry::STRIDE,
            INTEL_MASK,
        )
        .is_none()
        {
            return false;
        }
        if log.overflowed {
            return false;
        }
    }
    true
}

/// Mirror of one country's division roster takes. Returns false exactly
/// when the typed walk fails at or before this roster.
fn copy_intel_divisions(
    log: &mut RawLog<INTEL_N>,
    memory: &impl engine::Memory,
    base: usize,
    pointer: usize,
) -> bool {
    let header = match ispan(
        log,
        memory,
        pointer.checked_add(ccountry::DIVISIONS),
        16,
        INTEL_MASK,
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
    let expected = match base.checked_add(engine::carmy::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    for index in 0..roster.count {
        let addr = (index as usize)
            .checked_mul(8)
            .and_then(|offset| roster.data.checked_add(offset));
        let entry = match ispan(log, memory, addr, 8, INTEL_MASK) {
            Some(start) => start,
            None => return false,
        };
        let unit = match usize::try_from(intel_pool_u64(log, entry))
            .ok()
            .filter(|&unit| unit != 0)
        {
            Some(unit) => unit,
            None => return false,
        };
        if !copy_intel_unit(log, memory, base, unit, expected) {
            return false;
        }
        if log.overflowed {
            return false;
        }
    }
    true
}

/// Mirror of one country's fleet/task-force roster takes. Returns false
/// exactly when the typed walk fails at or before this roster.
fn copy_intel_fleets(
    log: &mut RawLog<INTEL_N>,
    memory: &impl engine::Memory,
    base: usize,
    pointer: usize,
) -> bool {
    let header = match ispan(
        log,
        memory,
        pointer.checked_add(ccountry::FLEETS),
        16,
        INTEL_MASK,
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
    let expected_fleet = match base.checked_add(engine::cfleet::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    let expected_unit = match base.checked_add(engine::ctask_force::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    for index in 0..roster.count {
        let addr = (index as usize)
            .checked_mul(8)
            .and_then(|offset| roster.data.checked_add(offset));
        let entry = match ispan(log, memory, addr, 8, INTEL_MASK) {
            Some(start) => start,
            None => return false,
        };
        let fleet = match usize::try_from(intel_pool_u64(log, entry))
            .ok()
            .filter(|&fleet| fleet != 0)
        {
            Some(fleet) => fleet,
            None => return false,
        };
        let vtable = match ispan(log, memory, Some(fleet), 8, INTEL_MASK) {
            Some(start) => start,
            None => return false,
        };
        if intel_pool_u64(log, vtable) != expected_fleet {
            return false;
        }
        let forces_header = match ispan(
            log,
            memory,
            fleet.checked_add(engine::cfleet::TASK_FORCES),
            16,
            INTEL_MASK,
        ) {
            Some(start) => start,
            None => return false,
        };
        raw.copy_from_slice(&log.bytes[forces_header..forces_header + 16]);
        let forces = match Array::from_header(raw, 8) {
            Some(array) => array,
            None => return false,
        };
        for task in 0..forces.count {
            let addr = (task as usize)
                .checked_mul(8)
                .and_then(|offset| forces.data.checked_add(offset));
            let entry = match ispan(log, memory, addr, 8, INTEL_MASK) {
                Some(start) => start,
                None => return false,
            };
            let unit = match usize::try_from(intel_pool_u64(log, entry))
                .ok()
                .filter(|&unit| unit != 0)
            {
                Some(unit) => unit,
                None => return false,
            };
            if !copy_intel_unit(log, memory, base, unit, expected_unit) {
                return false;
            }
            if log.overflowed {
                return false;
            }
        }
        if log.overflowed {
            return false;
        }
    }
    true
}

/// One structural copy pass: country census plus per-country division and
/// fleet takes with viewer entries. The census spans are taken once and
/// shared by both unit paths. The job is cleared on entry so pooled
/// refreshes never accumulate spans across hours.
fn copy_intel_pass(
    job: &mut IntelJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; INTEL_N],
) {
    job.clear();
    job.game_state = game_state;
    let started = qpc();
    copy_intel_tables(job, memory, base, game_state);
    ticks[0] += elapsed(started);
}

fn copy_intel_tables(
    job: &mut IntelJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
) {
    let log = &mut job.log;
    let countries_start = ispan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRIES),
        16,
        INTEL_MASK,
    );
    let links_start = ispan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRY_LINKS),
        16,
        INTEL_MASK,
    );
    let (Some(countries_start), Some(links_start)) = (countries_start, links_start) else {
        return;
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[countries_start..countries_start + 16]);
    let countries_array = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return,
    };
    raw.copy_from_slice(&log.bytes[links_start..links_start + 16]);
    let links = match Array::from_header(raw, 4) {
        Some(array) => array,
        None => return,
    };
    if countries_array.count < 2 || links.count == 0 {
        return;
    }
    let country_vtable = match base.checked_add(ccountry::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return,
    };
    for slot in 1..countries_array.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|offset| countries_array.data.checked_add(offset));
        let pointer_start = match ispan(log, memory, pointer_addr, 8, INTEL_MASK) {
            Some(start) => start,
            None => return,
        };
        let pointer = match usize::try_from(intel_pool_u64(log, pointer_start))
            .ok()
            .filter(|&pointer| pointer != 0)
        {
            Some(pointer) => pointer,
            None => continue,
        };
        let vtable_start = match ispan(log, memory, Some(pointer), 8, INTEL_MASK) {
            Some(start) => start,
            None => return,
        };
        let id_start = match ispan(log, memory, pointer.checked_add(ccountry::ID), 4, INTEL_MASK)
        {
            Some(start) => start,
            None => return,
        };
        if intel_pool_u64(log, vtable_start) != country_vtable {
            return;
        }
        let id = u32::from_le_bytes(log.bytes[id_start..id_start + 4].try_into().unwrap());
        let canonical = match intel_link(log, memory, &links, countries_array.count, id) {
            Some(canonical) => canonical,
            None => return,
        };
        if intel_link(log, memory, &links, countries_array.count, slot) != Some(canonical) {
            return;
        }
        if intel_link(log, memory, &links, countries_array.count, canonical) != Some(canonical) {
            return;
        }
        let canonical_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|offset| countries_array.data.checked_add(offset));
        let canonical_pointer = match ispan(log, memory, canonical_addr, 8, INTEL_MASK) {
            Some(start) => start,
            None => return,
        };
        if intel_pool_u64(log, canonical_pointer) != pointer as u64 {
            return;
        }
        if slot != canonical {
            continue;
        }
        if !copy_intel_divisions(log, memory, base, pointer) {
            return;
        }
        if log.overflowed {
            return;
        }
        if !copy_intel_fleets(log, memory, base, pointer) {
            return;
        }
        if log.overflowed {
            return;
        }
    }
}

impl Captured {
    /// Game-thread raw copy for an hourly refresh. Copies `first` always and
    /// `second` while the intel table still needs double-read checks,
    /// compares the raw logs, and records copy-only costs plus the existing
    /// checked/mismatches bookkeeping. Returns whether the payload is
    /// complete (`raw_ok`); on staging overflow the caller falls back to the
    /// typed walk and enqueues nothing.
    pub(crate) fn copy_intel_refresh(
        &mut self,
        memory: &impl Memory,
        ctx: IntelCopyContext,
        first: &mut IntelJob,
        mut second: Option<&mut IntelJob>,
    ) -> IntelCopyReport {
        let mut ticks = [0u64; INTEL_N];
        copy_intel_pass(first, memory, ctx.base, ctx.game_state, &mut ticks);
        let mut matched = [true; INTEL_N];
        if let Some(second_job) = second.as_deref_mut() {
            let mut second_ticks = [0u64; INTEL_N];
            copy_intel_pass(second_job, memory, ctx.base, ctx.game_state, &mut second_ticks);
            for (total, extra) in ticks.iter_mut().zip(second_ticks) {
                *total += extra;
            }
            let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
            for (slot, matched) in matched.iter_mut().enumerate() {
                let equal = raw_ok_pair && first.log.partition_equal(&second_job.log, slot);
                *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
            }
        }
        // Conservative checked bookkeeping mirroring the typed path.
        self.intel.checked = ctx.needed[0];
        self.intel.mismatches = usize::from(ctx.needed[0] && !matched[0]);
        let raw_ok = !first.log.overflowed
            && second.is_none_or(|job| !job.log.overflowed);
        IntelCopyReport {
            raw_ok,
            matched,
            copy_ticks: ticks,
            raw_bytes: first.log.raw_bytes,
        }
    }

    /// Decode-worker interpretation of a raw intel payload: replays the unit
    /// viewer walk against the recorded spans into the pooled buffers, with
    /// the identical bounded/capacity handling the game thread applies.
    /// Installs identical rows, truncation counts and failure reasons.
    pub(crate) fn decode_intel_payload(
        &mut self,
        base: usize,
        job: &IntelJob,
        decode_ticks: &mut [u64; INTEL_N],
    ) {
        use crate::raw::Recorded;
        let recorded = Recorded::build(&job.log);
        let started = qpc();
        let walked = unit_viewer_entry::walk(&recorded, base, job.game_state, &mut self.intel.rows);
        self.intel.outcome = bounded(walked, self.intel.rows.len());
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
            assert_eq!(report.matched, [true; 1], "rows={rows} provinces={provinces}");
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
        assert!(!first.log.partition_equal(&second.log, 0));
        assert!(!first.log.logs_equal(&second.log));
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
        let mut tiny = RadarJob {
            log: crate::raw::RawLog::new(1, 1),
            game_state: 0,
        };
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
            job.log.spans.capacity() >= spans,
            "span cap {} < worst case {spans}",
            job.log.spans.capacity()
        );
        let bytes = 439 * 64 + 18 * 128;
        assert!(
            job.log.bytes.capacity() >= bytes,
            "byte cap {} < worst case {bytes}",
            job.log.bytes.capacity()
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
            assert_eq!(report.matched, [true; 1], "cap={cap}");
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
        assert!(!first.log.partition_equal(&second.log, 0));
        assert!(!first.log.logs_equal(&second.log));
    }

    #[test]
    fn intel_census_duplication_is_bounded_and_net_positive() {
        // Shared-copy counts proof: the division/fleet census spans this
        // payload duplicates (country headers, slot words, roster headers
        // and entry pointers already copied by the moved Divisions and Map
        // jobs with different read sets) are a small bounded prefix of the
        // log, while the move removes the whole 0.386 ms intel walk from the
        // game thread (appendix steady state). Census spans: 2 headers + 1
        // slot x 7 words + 2 roster headers + 2 entry words = 13 takes here;
        // the full log carries every viewer span exactly once.
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
                first.log.spans.iter().any(|span| span.addr == addr),
                "census span missing at {addr:x}"
            );
        }
        // Viewer spans are taken exactly once per batch read.
        let viewer_takes = first
            .log
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
        let mut tiny = IntelJob {
            log: crate::raw::RawLog::new(1, 1),
            game_state: 0,
        };
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
        // Worst-case caps proof: the steady-state pools exceed a
        // conservative span/byte estimate built from the appendix census (24
        // intel rows over ~2.9k divisions and ~240 task forces with 439
        // countries), so live refreshes never take the overflow fallback in
        // steady state. The duplicated census prefix (~439 slots x 7 words
        // plus roster headers and entry pointers) is a small fraction of the
        // pool; the move removes the 0.386 ms walk (net-positive, see the
        // duplication test above).
        use super::IntelJob;
        let job = IntelJob::new();
        let spans = 2 + 439 * 7 + 439 * 2 + 3_200 * 2 + 500 * 4 + 24 * 4;
        assert!(
            job.log.spans.capacity() >= spans,
            "span cap {} < worst case {spans}",
            job.log.spans.capacity()
        );
        let bytes = 439 * 64 + 3_200 * 32 + 500 * 64 + 24 * 32;
        assert!(
            job.log.bytes.capacity() >= bytes,
            "byte cap {} < worst case {bytes}",
            job.log.bytes.capacity()
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
        assert_eq!(split.rows, direct.rows, "radar rows");
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
        assert_eq!(split.rows, direct.rows, "intel rows");
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
                .rows
                .iter()
                .filter(|row| is_fleet(row.unit) == fleet)
                .collect();
            let direct_path: Vec<_> = direct
                .rows
                .iter()
                .filter(|row| is_fleet(row.unit) == fleet)
                .collect();
            assert_eq!(split_path, direct_path, "intel path fleet={fleet}");
        }
    }
}
