use collector_primitives::{
    clock::qpc,
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
};
use collector_projection::table_conversion::{table, table_iter, unavailable};
use engine_allocation::{collect_vec, copy_vec, owned_vec};
use engine_native::{Memory, walk::Outcome};
use engine_native_air::game_score_inputs as native;
use replay_data::{format::GAME_SCORE_DISABLED_REASON, rows, rows::TableId, rows::tables};
use replay_frame::frame::TableView;
use std::sync::atomic::{AtomicUsize, Ordering};

static SITES_HINT: AtomicUsize = AtomicUsize::new(0);
static WINGS_HINT: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
std::thread_local! {
    static TEST_HINTS: std::cell::Cell<Option<[usize; 2]>> = const { std::cell::Cell::new(None) };
}
fn arena_hints() -> [usize; 2] {
    #[cfg(test)]
    if let Some(hints) = TEST_HINTS.with(std::cell::Cell::get) {
        return hints;
    }
    [
        SITES_HINT.load(Ordering::Relaxed),
        WINGS_HINT.load(Ordering::Relaxed),
    ]
}
fn note_arena_hints(sites: usize, wings: usize) -> bool {
    #[cfg(test)]
    if let Some(raised) = TEST_HINTS.with(|current| match current.get() {
        Some([previous_sites, previous_wings]) => {
            let next = [previous_sites.max(sites), previous_wings.max(wings)];
            current.set(Some(next));
            Some(next != [previous_sites, previous_wings])
        }
        None => None,
    }) {
        return raised;
    }
    let previous_sites = SITES_HINT.fetch_max(sites, Ordering::Relaxed);
    let previous_wings = WINGS_HINT.fetch_max(wings, Ordering::Relaxed);
    sites > previous_sites || wings > previous_wings
}
#[cfg(test)]
struct TestHints(Option<[usize; 2]>);
#[cfg(test)]
impl TestHints {
    fn new() -> Self {
        Self(TEST_HINTS.with(|current| current.replace(Some([0; 2]))))
    }
}
#[cfg(test)]
impl Drop for TestHints {
    fn drop(&mut self) {
        TEST_HINTS.with(|current| current.set(self.0));
    }
}

pub const N: usize = 3;
pub const TABLES: [TableId; N] = [
    TableId::CountryGameScoreInputs,
    TableId::AirWingGameScoreStats,
    TableId::GameScoreMultipliers,
];
pub const COUNTRY_TABLES: [TableId; 2] = [TABLES[0], TABLES[2]];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub enabled: bool,
    countries: usize,
    fleets: usize,
    sites: usize,
    wings: usize,
}

collector_primitives::capacity_max!(Capacities {
    enabled,
    countries,
    fleets,
    sites,
    wings
});
impl Default for Capacities {
    fn default() -> Self {
        Self {
            enabled: true,
            countries: 0,
            fleets: 0,
            sites: 0,
            wings: 0,
        }
    }
}
impl Capacities {
    pub fn raise_for_countries(&mut self, slots: usize) {
        collector_primitives::country_slots::raise(&mut self.countries, slots);
    }
    pub fn raise(&mut self, demand: native::ArenaDemand) -> bool {
        let previous = *self;
        collector_primitives::country_slots::note_slots(demand.countries);
        collector_primitives::country_slots::raise(&mut self.countries, demand.countries);
        collector_primitives::country_slots::raise(&mut self.fleets, demand.fleets);
        collector_primitives::country_slots::raise(&mut self.sites, demand.sites);
        collector_primitives::country_slots::raise(&mut self.wings, demand.wings);
        let hints_raised = note_arena_hints(demand.sites, demand.wings);
        *self != previous || hints_raised
    }
}

#[derive(Debug, Clone, Copy)]
struct AirCountry {
    read: native::AirRead,
    start: usize,
}
impl Default for AirCountry {
    fn default() -> Self {
        Self {
            read: native::AirRead {
                state: native::AirState::Unknown,
                error: "game score strategic-air country not witnessed",
                len: 0,
                required: 0,
            },
            start: 0,
        }
    }
}
impl AirCountry {
    fn short_storage() -> Self {
        Self {
            read: native::AirRead {
                error: "game score air-country storage exhausted",
                ..Self::default().read
            },
            start: 0,
        }
    }
}

#[cfg(test)]
#[derive(Debug, PartialEq)]
pub struct Witnessed<'a> {
    pub countries: Vec<(native::AirRead, &'a [native::AirSite])>,
    wings: &'a [native::WingRow],
    required: [usize; 2],
    outcome: Outcome,
    allocation_failure: Option<&'static str>,
}

pub struct AirInputs {
    enabled: bool,
    allocation_failure: Option<&'static str>,
    countries: Vec<AirCountry>,
    required_countries: usize,
    required_wings: usize,
    written_wings: usize,
    sites: Arena<native::AirSite>,
    wings: CaptureRows<native::WingRow>,
}
impl std::fmt::Debug for AirInputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AirInputs")
            .field("enabled", &self.enabled)
            .field("countries", &self.countries.len())
            .field("sites", &self.sites.len)
            .field("wings", &self.wings.outcome)
            .finish()
    }
}
impl Default for AirInputs {
    fn default() -> Self {
        Self::new(false, 0, 0, 0)
    }
}
impl AirInputs {
    fn new(enabled: bool, countries: usize, sites: usize, wings: usize) -> Self {
        Self {
            enabled,
            allocation_failure: None,
            countries: owned_vec(engine_allocation::default_vec(countries)),
            required_countries: 0,
            required_wings: 0,
            written_wings: 0,
            sites: Arena::new(sites),
            wings: CaptureRows::new(wings),
        }
    }
    pub fn for_wing_job(wings: usize) -> Self {
        let [sites, observed_wings] = arena_hints();
        let inputs = Self::new(
            false,
            collector_primitives::country_slots::census_rows(0),
            sites,
            wings.max(observed_wings),
        );
        inputs
    }
    pub fn presize(&mut self) {
        let [sites, wings] = arena_hints();
        grow_vec(
            &mut self.countries,
            collector_primitives::country_slots::census_rows(0),
        );
        grow_vec(&mut self.sites.first, sites);
        grow_vec(&mut self.wings.rows, wings);
    }
    pub fn begin(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.allocation_failure = None;
        self.countries.fill(AirCountry::default());
        self.required_countries = 0;
        self.required_wings = 0;
        self.written_wings = 0;
        self.sites.reset();
        self.wings.reset();
        self.wings.outcome = Outcome::Unavailable {
            reason: if enabled {
                "game score air census not run"
            } else {
                GAME_SCORE_DISABLED_REASON
            },
        };
    }
    pub fn owner(&mut self, memory: &impl Memory, base: usize, slot: u32, entry: Option<usize>) {
        if !self.enabled {
            return;
        }
        let slot_index = slot as usize;
        self.required_countries = self.required_countries.max(slot_index.saturating_add(1));
        let start = self.sites.len;
        let read = {
            if let Err(error) = engine_native::walk::Buffer::Growing(&mut self.countries)
                .ensure(self.required_countries)
            {
                self.allocation_failure = Some(error);
                self.wings.outcome = Outcome::Unavailable { reason: error };
                return;
            }
            native::read_first_sites_buffer(
                memory,
                base,
                slot,
                entry,
                engine_native::walk::Buffer::Growing(&mut self.sites.first).suffix(start),
            )
        };
        self.sites.required = self.sites.required.saturating_add(read.required);
        self.sites.len = self.sites.len.saturating_add(read.len);
        if let Some(destination) = self.countries.get_mut(slot_index) {
            *destination = AirCountry { read, start };
        }
    }
    pub fn wing(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
        row: &engine_native_air::cair_wing::ScoreLeaf,
    ) {
        if !self.enabled {
            return;
        }
        self.required_wings = self.required_wings.saturating_add(1);
        let len = self.written_wings;
        if let Err(reason) = engine_native::walk::Buffer::Growing(&mut self.wings.rows)
                .ensure(self.required_wings)
        {
            self.allocation_failure = Some(reason);
            self.wings.outcome = Outcome::Unavailable { reason };
            return;
        }
        if let Some(destination) = self.wings.rows.get_mut(len) {
            *destination = native::read_wing(memory, pointer, row);
            self.written_wings = len + 1;
        }
    }
    pub fn absorb(&mut self, part: &AirInputs, first: usize) -> Result<(), &'static str> {
        if !self.enabled {
            return Ok(());
        }
        self.allocation_failure = self.allocation_failure.or(part.allocation_failure);
        let offset = self.sites.len;
        let end = offset
            .checked_add(part.sites.len)
            .ok_or("game score air-site join overflow")?;
        engine_native::walk::Buffer::Growing(&mut self.sites.first).ensure(end)?;
        self.sites.first[offset..end].copy_from_slice(&part.sites.first[..part.sites.len]);
        self.sites.len = end;
        self.sites.required = self.sites.required.saturating_add(part.sites.required);
        self.required_countries = self.required_countries.max(part.required_countries);
        let end = part.required_countries.min(part.countries.len());
        let slots = first.min(end)..end;
        engine_native::walk::Buffer::Growing(&mut self.countries).ensure(end)?;
        for (destination, country) in self.countries[slots.clone()]
            .iter_mut()
            .zip(&part.countries[slots])
        {
            *destination = AirCountry {
                start: country.start + offset,
                ..*country
            };
        }
        let written = self.written_wings;
        let end = written
            .checked_add(part.written_wings)
            .ok_or("game score wing join overflow")?;
        engine_native::walk::Buffer::Growing(&mut self.wings.rows).ensure(end)?;
        self.wings.rows[written..end].copy_from_slice(&part.wings.rows[..part.written_wings]);
        self.written_wings = end;
        self.required_wings = self.required_wings.saturating_add(part.required_wings);
        Ok(())
    }
    #[cfg(test)]
    pub fn witnessed(&self) -> Witnessed<'_> {
        Witnessed {
            countries: self.countries[..self.required_countries]
                .iter()
                .map(|country| {
                    let end = country.start + country.read.len;
                    (country.read, &self.sites.first[country.start..end])
                })
                .collect(),
            wings: &self.wings.rows[..self.written_wings],
            required: [self.required_wings, self.sites.required],
            outcome: self.wings.outcome,
            allocation_failure: self.allocation_failure,
        }
    }
    pub fn finish(&mut self, parent: Outcome) {
        if !self.enabled {
            return;
        }
        if let Some(reason) = self.allocation_failure {
            self.wings.fail(Outcome::Unavailable { reason });
            return;
        }
        let required = match parent {
            Outcome::Rows { len } => len,
            Outcome::Truncated { len, unread_rows } => len.saturating_add(unread_rows as usize),
            Outcome::Unavailable { .. } => self.required_wings,
        };
        self.required_wings = self.required_wings.max(required);
        let current = match u32::try_from(self.required_wings.saturating_sub(self.written_wings)) {
            Ok(0) => Outcome::Rows {
                len: self.written_wings,
            },
            Ok(unread_rows) => Outcome::Truncated {
                len: self.written_wings,
                unread_rows,
            },
            Err(_) => Outcome::Unavailable {
                reason: "game score wing demand overflow",
            },
        };
        self.wings.outcome = current;
        let outcome = super::air::require_parent(parent, current);
        if matches!(outcome, Outcome::Unavailable { .. }) {
            self.wings.fail(outcome);
        } else {
            self.wings.outcome = outcome;
        }
        if let Outcome::Unavailable { reason } = parent {
            for country in &mut self.countries {
                if country.read.state == native::AirState::Unknown && country.read.required == 0 {
                    country.read.error = reason;
                }
            }
        }
    }
    fn grow(&mut self, countries: &mut usize, sites: &mut usize, wings: &mut usize) {
        grow_capacity(countries, self.required_countries);
        grow_vec(&mut self.countries, *countries);
        self.sites.grow(sites);
        grow_capacity(wings, self.required_wings);
        self.wings.grow(wings);
    }
    pub fn grow_for_next(&self) -> bool {
        self.required_countries > self.countries.len()
            || self.sites.required > self.sites.first.len()
            || self.required_wings > self.wings.rows.len()
    }
    pub fn grow_owned(&mut self) {
        let (mut countries, mut sites, mut wings) = (
            self.countries.len(),
            self.sites.first.len(),
            self.wings.rows.len(),
        );
        self.grow(&mut countries, &mut sites, &mut wings);
    }
    pub fn allocated_bytes(&self) -> usize {
        self.countries.capacity() * std::mem::size_of::<AirCountry>()
            + self.sites.allocated_bytes()
            + self.wings.allocated_bytes()
    }
}

pub struct Captured {
    enabled: bool,
    countries: CaptureRows<native::CountryRow>,
    country_len: usize,
    slot_zero_null: bool,
    slot_zero_refusal: Option<&'static str>,
    fleets: Arena<native::FleetCount>,
    air: AirInputs,
    multipliers: Option<native::Multipliers>,
    multiplier_outcome: Outcome,
}
impl Captured {
    pub fn swap_country(&mut self, other: &mut Self) {
        other.enabled = self.enabled;
        std::mem::swap(&mut self.countries, &mut other.countries);
        std::mem::swap(&mut self.country_len, &mut other.country_len);
        std::mem::swap(&mut self.slot_zero_null, &mut other.slot_zero_null);
        std::mem::swap(&mut self.slot_zero_refusal, &mut other.slot_zero_refusal);
        std::mem::swap(&mut self.fleets, &mut other.fleets);
        std::mem::swap(&mut self.multipliers, &mut other.multipliers);
        std::mem::swap(&mut self.multiplier_outcome, &mut other.multiplier_outcome);
    }
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            enabled: false,
            ..Default::default()
        })
    }
    pub fn with_capacities(c: &Capacities) -> Self {
        let c = if c.enabled {
            *c
        } else {
            Capacities {
                enabled: false,
                ..Default::default()
            }
        };
        Self {
            enabled: c.enabled,
            countries: CaptureRows::new(c.countries),
            country_len: 0,
            slot_zero_null: false,
            slot_zero_refusal: None,
            fleets: Arena::new(c.fleets),
            air: AirInputs::new(c.enabled, c.countries, c.sites, c.wings),
            multipliers: None,
            multiplier_outcome: Outcome::Unavailable {
                reason: "game score multipliers not read",
            },
        }
    }
    pub fn reset(&mut self) {
        self.countries.reset();
        self.fleets.reset();
        self.air.begin(self.enabled);
        self.country_len = 0;
        self.slot_zero_null = false;
        self.slot_zero_refusal = None;
        self.multipliers = None;
        self.multiplier_outcome = Outcome::Unavailable {
            reason: "game score multipliers not read",
        };
    }
    pub fn allocated_bytes(&self) -> usize {
        self.countries.allocated_bytes()
            + self.fleets.allocated_bytes()
            + self.air.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        if !self.enabled {
            return;
        }
        grow_capacity(&mut capacity.countries, self.country_slots_required());
        self.countries.grow(&mut capacity.countries);
        self.fleets.grow(&mut capacity.fleets);
        self.air.grow(
            &mut capacity.countries,
            &mut capacity.sites,
            &mut capacity.wings,
        );
    }
    fn country_slots_required(&self) -> usize {
        self.countries.rows[..self.country_len]
            .iter()
            .map(|row| (row.canonical_slot as usize).saturating_add(1))
            .max()
            .unwrap_or(0)
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.countries.fail(Outcome::Unavailable { reason });
        self.multiplier_outcome = Outcome::Unavailable { reason };
        self.multipliers = None;
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        self.country_len = 0;
        self.slot_zero_null = false;
        self.slot_zero_refusal = None;
        if !self.enabled {
            self.fail(GAME_SCORE_DISABLED_REASON);
            self.air.begin(false);
            return [0; N];
        }
        let mut reads = collector_memory::read_accounting::table(TABLES[0]);
        let started = qpc();
        let capture = native::walk_country_inputs(
            memory,
            base,
            state,
            &mut self.countries.rows,
            &mut self.fleets.first,
        );
        self.countries.outcome = capture.outcome;
        self.country_len = capture.country_len;
        self.slot_zero_null = capture.slot_zero_null;
        self.slot_zero_refusal = capture.slot_zero_refusal;
        self.fleets.len = capture.fleet_len;
        self.fleets.required = capture.fleet_required;
        let country_ticks = collector_extended_support::outcomes::elapsed(started);
        reads.select(TABLES[2]);
        let started = qpc();
        self.multipliers = Some(native::read_multipliers(memory, base));
        self.multiplier_outcome = Outcome::Rows { len: 1 };
        [
            country_ticks,
            0,
            collector_extended_support::outcomes::elapsed(started),
        ]
    }
    pub fn fail_air(&mut self, reason: &'static str) {
        self.air.finish(Outcome::Unavailable { reason });
    }
    pub fn install_air(&mut self, air: &mut AirInputs) {
        std::mem::swap(&mut self.air, air);
        air.required_countries = air.required_countries.max(self.air.required_countries);
        air.required_wings = air.required_wings.max(self.air.required_wings);
        air.sites.required = air.sites.required.max(self.air.sites.required);
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.countries.outcome,
            self.air.wings.outcome,
            self.multiplier_outcome,
        ]
    }
    pub fn tables(&self) -> [TableView; N] {
        let countries = table::<tables::CountryGameScoreInputs, _>(
            self.countries.outcome,
            &self.countries.rows,
            |row| {
                let context = self
                    .air
                    .countries
                    .get(row.canonical_slot as usize)
                    .copied()
                    .unwrap_or_else(AirCountry::short_storage);
                let sites = self
                    .air
                    .sites
                    .first
                    .get(..self.air.sites.len)
                    .and_then(|sites| {
                        sites.get(context.start..context.start.saturating_add(context.read.len))
                    });
                let fleets = self.fleets.first.get(..self.fleets.len).and_then(|fleets| {
                    fleets.get(row.fleet_start..row.fleet_start.saturating_add(row.fleet_len))
                });
                let (army_count, army_error) = read_value(row.army_count);
                let (navy_count, navy_error) = read_value(row.navy_count);
                let (land_count, land_error) = read_value(row.land_count);
                let mut result = rows::CountryGameScoreInputRow {
                    country: row.country,
                    army_count,
                    army_error,
                    navy_count,
                    navy_error,
                    land_count,
                    land_error,
                    air_state: replay_air_state(context.read.state),
                    air_error: error_bytes(context.read.error),
                    first_sites: owned_vec(collect_vec(sites.unwrap_or(&[]).iter().map(|site| {
                        let (value, error) = read_value(site.access);
                        rows::GameScoreAirSite {
                            base: site.base,
                            access: match value {
                                None => rows::GameScoreAirSite::ACCESS_UNKNOWN,
                                Some(false) => rows::GameScoreAirSite::ACCESS_FALSE,
                                Some(true) => rows::GameScoreAirSite::ACCESS_TRUE,
                            },
                            error,
                        }
                    }))),
                    fleets: owned_vec(collect_vec(fleets.unwrap_or(&[]).iter().map(|fleet| {
                        let (task_forces, error) = read_value(fleet.task_forces);
                        rows::GameScoreFleetCount {
                            fleet: fleet.fleet,
                            task_forces,
                            error,
                        }
                    }))),
                };
                if sites.is_none() {
                    result.air_state = rows::CountryGameScoreInputRow::AIR_UNKNOWN;
                    result.air_error =
                        error_bytes("game score first air-site span outside captured arena");
                }
                if fleets.is_none() {
                    result.navy_count = None;
                    result.navy_error = error_bytes("game score fleet span outside captured arena");
                }
                result
            },
        );
        let wings = table::<tables::AirWingGameScoreStats, _>(
            self.air.wings.outcome,
            &self.air.wings.rows,
            |row| {
                let (planes, planes_error) = read_value(row.planes);
                let (stat_raw, stat_error) = read_value(row.stat_raw);
                let (mission_index, mission_error) = read_value(row.mission_index);
                rows::AirWingGameScoreStatsRow {
                    id: row.id,
                    planes,
                    planes_error,
                    stat_raw,
                    stat_error,
                    mission_index,
                    mission_error,
                }
            },
        );
        let multipliers = match &self.multipliers {
            None => match self.multiplier_outcome {
                Outcome::Unavailable { reason } => unavailable(reason),
                _ => unavailable("game score multiplier payload not supplied"),
            },
            Some(row) => table_iter::<tables::GameScoreMultipliers, _>(
                self.multiplier_outcome,
                std::iter::once(row),
                |row| {
                    let (total_raw, total_error) = read_value(row.total_raw);
                    let (army_raw, army_error) = read_value(row.army_raw);
                    let (navy_raw, navy_error) = read_value(row.navy_raw);
                    let (air_raw, air_error) = read_value(row.air_raw);
                    let (industry_raw, industry_error) = read_value(row.industry_raw);
                    let (land_raw, land_error) = read_value(row.land_raw);
                    rows::GameScoreMultiplierRow {
                        id: 0,
                        total_raw,
                        total_error,
                        army_raw,
                        army_error,
                        navy_raw,
                        navy_error,
                        air_raw,
                        air_error,
                        industry_raw,
                        industry_error,
                        land_raw,
                        land_error,
                    }
                },
            ),
        };
        [countries, wings, multipliers]
    }
}
fn replay_air_state(state: native::AirState) -> u8 {
    match state {
        native::AirState::Unknown => rows::CountryGameScoreInputRow::AIR_UNKNOWN,
        native::AirState::Absent => rows::CountryGameScoreInputRow::AIR_ABSENT,
        native::AirState::Present => rows::CountryGameScoreInputRow::AIR_PRESENT,
    }
}
fn error_bytes(error: &str) -> Vec<u8> {
    owned_vec(copy_vec(error.as_bytes()))
}
fn read_value<T>(read: Result<T, &'static str>) -> (Option<T>, Vec<u8>) {
    match read {
        Ok(value) => (Some(value), Vec::new()),
        Err(error) => (None, error_bytes(error)),
    }
}

#[cfg(test)]
mod tests;
