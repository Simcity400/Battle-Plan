//! Native fog-derivation input acquisition (registry domain D102). Only growth
//! and replay conversion allocate; every engine read happens in the callback.

use crate::{
    sampling::{full_match, qpc},
    spooling::table,
    table_capture::{Arena, CaptureRows},
};
use engine::{
    Memory, cair_base, ccontroller_area, ccountry, ctheatre, radar_group, unit_viewer_entry,
    walk::Outcome,
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
}
