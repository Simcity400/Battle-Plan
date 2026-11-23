use crate::{
    clock::qpc,
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
    table_conversion::{table, unavailable},
};
use engine_allocation::{collect_vec, copy_vec, default_vec, owned_vec};
use engine_native_memory::{Memory, walk::{Buffer, Outcome}};
use engine_native::cunit::FirstCombatMemo;
use engine_railways::railway_gun_counter_inputs;
use replay_data::{
    rows::FreshRailwayCounterEta, rows::OwnerTransportSpeedRow, rows::RailwayGunCounterInputRow,
    rows::RailwayGunEtaOperands, rows::Recorded, rows::TableId, rows::tables,
};
use replay_frame::frame::TableView;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

static RAILWAY_GUN_PATHS: AtomicUsize = AtomicUsize::new(0);
static RAILWAY_GUN_ROWS: AtomicUsize = AtomicUsize::new(0);
static RAILWAY_GUN_NAME_BYTES: AtomicUsize = AtomicUsize::new(0);
static RAILWAY_GUN_NAMED_BONUSES: AtomicUsize = AtomicUsize::new(0);
static RAILWAY_GUN_SUPPLY_ORIGINS: AtomicUsize = AtomicUsize::new(0);
static RAILWAY_GUN_COMBAT_REFERENCES: AtomicUsize = AtomicUsize::new(0);

pub fn railway_gun_name_demand() -> engine_railways::railway_gun_counter_inputs::NameDemand {
    engine_railways::railway_gun_counter_inputs::NameDemand {
        rows: RAILWAY_GUN_ROWS.load(Ordering::Relaxed),
        paths: RAILWAY_GUN_PATHS.load(Ordering::Relaxed),
        bytes: RAILWAY_GUN_NAME_BYTES.load(Ordering::Relaxed),
        named_bonuses: RAILWAY_GUN_NAMED_BONUSES.load(Ordering::Relaxed),
        supply_origins: RAILWAY_GUN_SUPPLY_ORIGINS.load(Ordering::Relaxed),
        combat_references: RAILWAY_GUN_COMBAT_REFERENCES.load(Ordering::Relaxed),
    }
}

pub fn note_railway_gun_names(demand: engine_railways::railway_gun_counter_inputs::NameDemand) {
    RAILWAY_GUN_ROWS.fetch_max(demand.rows, Ordering::Relaxed);
    RAILWAY_GUN_PATHS.fetch_max(demand.paths, Ordering::Relaxed);
    RAILWAY_GUN_NAME_BYTES.fetch_max(demand.bytes, Ordering::Relaxed);
    RAILWAY_GUN_NAMED_BONUSES.fetch_max(demand.named_bonuses, Ordering::Relaxed);
    RAILWAY_GUN_SUPPLY_ORIGINS.fetch_max(demand.supply_origins, Ordering::Relaxed);
    RAILWAY_GUN_COMBAT_REFERENCES.fetch_max(demand.combat_references, Ordering::Relaxed);
}

static NAME_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static ETA_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static WITHDRAWAL_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static FRESH_REFUSALS: [std::sync::atomic::AtomicU64; 4] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 4];

pub fn movement_arrow_data() -> impl serde::Serialize {
    crate::status::status!({
        "withdrawal_refused": WITHDRAWAL_REFUSALS.load(std::sync::atomic::Ordering::Relaxed),
        "scope": "cumulative field-local railway-gun movement-arrow scalar refusals; established counter inputs remain available",
    })
}

pub fn eta_data() -> impl serde::Serialize {
    let refused = ETA_REFUSALS.load(std::sync::atomic::Ordering::Relaxed);
    crate::status::status!({
        "refused": refused,
        "refusal": "railway gun first-edge ETA acquisition failed",
        "fresh_refusals": crate::status::map([
            "railway gun fresh ETA callback hour or observation arena unavailable",
            "railway gun fresh maximum-speed acquisition failed",
            "railway gun fresh land supply-factor acquisition failed",
            "railway gun fresh genuine orders-group acquisition failed",
        ].into_iter().zip(FRESH_REFUSALS.iter()).map(|(reason, count)|
            (reason, count.load(std::sync::atomic::Ordering::Relaxed))
        ).filter(|(_, count)| *count != 0)),
    })
}

fn count_refusal(refused: &mut Vec<(&'static str, u64)>, reason: &'static str) {
    match refused.iter_mut().find(|(known, _)| *known == reason) {
        Some((_, count)) => *count += 1,
        None => {
            if refused.try_reserve(1).is_ok() {
                refused.push((reason, 1));
            }
        }
    }
}

static TRANSPORT_REFUSALS: Mutex<Vec<(&'static str, u64)>> = Mutex::new(Vec::new());

fn note_transport_refusals(refused: &[(&'static str, u64)]) {
    if refused.is_empty() {
        return;
    }
    let mut totals = TRANSPORT_REFUSALS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    for &(reason, count) in refused {
        match totals.iter_mut().find(|(known, _)| *known == reason) {
            Some((_, total)) => *total += count,
            None => {
                if totals.try_reserve(1).is_ok() {
                    totals.push((reason, count));
                }
            }
        }
    }
}

pub fn transport_data() -> impl serde::Serialize {
    let refusals = TRANSPORT_REFUSALS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    crate::status::status!({
        "refusals": crate::status::map(refusals.into_iter()),
        "scope": "cumulative refused railway-gun owner transport-speed queries by reason, one per owner and strategic region, and gun or route provinces whose strategic region could not be read (skipped); every other region keeps its speed",
    })
}

pub fn name_data() -> impl serde::Serialize {
    let refused = NAME_REFUSALS.load(std::sync::atomic::Ordering::Relaxed);
    crate::status::status!({
        "refused": refused,
        "refusals": crate::status::map(std::iter::once(("railway gun name guarded read or arena refusal", refused)).filter(|(_, count)| *count != 0)),
        "scope": "cumulative stored railway-gun name acquisition refusals; other counter operands remain available",
    })
}

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [
    TableId::RailwayGunCounterInputs,
    TableId::OwnerTransportSpeeds,
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    paths: usize,
    names: usize,
    named_bonuses: usize,
    supply_origins: usize,
    combat_references: usize,
}

collector_primitives::capacity_max!(Capacities {
    rows,
    paths,
    names,
    named_bonuses,
    supply_origins,
    combat_references
});
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: railway_gun_name_demand().rows,
            paths: railway_gun_name_demand().paths,
            names: railway_gun_name_demand().bytes,
            named_bonuses: railway_gun_name_demand().named_bonuses,
            supply_origins: railway_gun_name_demand().supply_origins,
            combat_references: railway_gun_name_demand().combat_references,
        }
    }
}

impl Capacities {
    pub fn raise_for_names(&mut self, demand: railway_gun_counter_inputs::NameDemand) {
        grow_capacity(&mut self.rows, demand.rows);
        grow_capacity(&mut self.paths, demand.paths);
        grow_capacity(&mut self.names, demand.bytes);
        grow_capacity(&mut self.named_bonuses, demand.named_bonuses);
        grow_capacity(&mut self.supply_origins, demand.supply_origins);
        grow_capacity(&mut self.combat_references, demand.combat_references);
    }
}

fn layout_buffer_len() -> usize {
    #[cfg(any(test, feature = "test-support"))]
    if !engine_executable::executable_tables::is_initialized() {
        return 0;
    }
    engine_fixed_layout::fixed_layout::longest_name()
}

#[derive(Clone, Copy)]
enum TransportSpeed {
    Observed(Option<engine_naval_history::cnaval_unit_transfer::SpeedInputs>),
    Ready(Option<i64>),
}

struct TransportRow {
    owner: u32,
    region: u32,
    speed: TransportSpeed,
}

pub struct Captured {
    owned_pending: bool,
    transport_pending: bool,
    rows: CaptureRows<railway_gun_counter_inputs::Row>,
    combat_references: Arena<engine_native::cunit::MapCombatReference>,
    sample_hour: Option<i32>,
    paths: Arena<u32>,
    names: Arena<u8>,
    tooltip_guns: Vec<(usize, usize)>,
    fresh_observations: Vec<railway_gun_counter_inputs::FreshEtaObservations>,
    named_bonuses: Vec<usize>,
    required_named_bonuses: usize,
    fixed_layout: Option<engine_fixed_layout::fixed_layout::Report>,
    fixed_layout_buffer: Vec<u8>,
    supply_origins: Arena<engine_native_parts::cunit_supply_inputs::Origin>,
    combats: FirstCombatMemo,
    regions: Vec<engine_naval_history::cnaval_unit_transfer::TransportRegion>,
    owners: Vec<u32>,
    transport: Vec<TransportRow>,
    transport_outcome: Outcome,
    transport_refused: Vec<(&'static str, u64)>,
    transport_demand: Vec<(u32, u32)>,
    transport_provinces: Vec<u32>,
    transport_located: Vec<(u32, u32)>,
    transport_wanted: Vec<(u32, usize)>,
    transport_indices: Vec<usize>,
    speed_cache: engine_naval_history::cnaval_unit_transfer::SpeedCache,
}

const TRANSPORT_UNCAPTURED: Outcome = Outcome::Unavailable {
    reason: "owner transport speeds not captured",
};
impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities_and_layout(
            &Capacities {
                rows: 0,
                paths: 0,
                names: 0,
                named_bonuses: 0,
                supply_origins: 0,
                combat_references: 0,
            },
            false,
        )
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self::with_capacities_and_layout(capacity, true)
    }

    fn with_capacities_and_layout(capacity: &Capacities, prepare_layout: bool) -> Self {
        let layout_bytes = if prepare_layout {
            layout_buffer_len()
        } else {
            0
        };
        Self {
            owned_pending: false,
            transport_pending: false,
            rows: CaptureRows::new(capacity.rows),
            combat_references: Arena::new(capacity.combat_references),
            sample_hour: None,
            paths: Arena::new(capacity.paths),
            names: Arena::new(capacity.names),
            tooltip_guns: owned_vec(default_vec(capacity.rows)),
            fresh_observations: owned_vec(default_vec(capacity.rows)),
            named_bonuses: owned_vec(default_vec(capacity.named_bonuses)),
            required_named_bonuses: 0,
            fixed_layout: None,
            fixed_layout_buffer: owned_vec(default_vec(layout_bytes)),
            supply_origins: Arena::new(capacity.supply_origins),
            combats: FirstCombatMemo::try_new(capacity.rows)
                .map_err(crate::allocation::io)
                .unwrap_or_default(),
            regions: Vec::new(),
            owners: Vec::new(),
            transport: Vec::new(),
            transport_outcome: TRANSPORT_UNCAPTURED,
            transport_refused: Vec::new(),
            transport_demand: Vec::new(),
            transport_provinces: Vec::new(),
            transport_located: Vec::new(),
            transport_wanted: Vec::new(),
            transport_indices: Vec::new(),
            speed_cache: Default::default(),
        }
    }

    pub fn reset(&mut self) {
        self.decode_owned();
        self.decode_transport();
        self.speed_cache.finish();
        self.combat_references.reset();
        self.sample_hour = None;
        self.rows.reset();
        self.paths.reset();
        self.names.reset();
        self.supply_origins.reset();
        self.combats.clear();
        self.required_named_bonuses = 0;
        self.fresh_observations.fill(Default::default());
        self.regions.clear();
        self.owners.clear();
        self.transport.clear();
        self.transport_outcome = TRANSPORT_UNCAPTURED;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.combat_references.allocated_bytes()
            + self.rows.allocated_bytes()
            + self.paths.allocated_bytes()
            + self.names.allocated_bytes()
            + self.combats.allocated_bytes()
            + self.tooltip_guns.capacity() * std::mem::size_of::<(usize, usize)>()
            + self.fresh_observations.capacity()
                * std::mem::size_of::<railway_gun_counter_inputs::FreshEtaObservations>()
            + self.named_bonuses.capacity() * std::mem::size_of::<usize>()
            + self.fixed_layout_buffer.capacity()
            + self.supply_origins.allocated_bytes()
            + self.regions.capacity()
                * std::mem::size_of::<engine_naval_history::cnaval_unit_transfer::TransportRegion>()
            + self.owners.capacity() * std::mem::size_of::<u32>()
            + self.transport.capacity() * std::mem::size_of::<TransportRow>()
            + (self.transport_demand.capacity() + self.transport_located.capacity())
                * std::mem::size_of::<(u32, u32)>()
            + self.transport_provinces.capacity() * std::mem::size_of::<u32>()
            + self.transport_wanted.capacity() * std::mem::size_of::<(u32, usize)>()
            + self.transport_indices.capacity() * std::mem::size_of::<usize>()
            + self.speed_cache.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        capacity.raise_for_names(railway_gun_name_demand());
        self.rows.grow(&mut capacity.rows);
        self.combat_references.grow(&mut capacity.combat_references);
        grow_capacity(&mut capacity.paths, self.paths.required);
        self.paths.grow(&mut capacity.paths);
        self.names.grow(&mut capacity.names);
        grow_vec(&mut self.tooltip_guns, capacity.rows);
        grow_vec(&mut self.fresh_observations, capacity.rows);
        grow_capacity(&mut capacity.named_bonuses, self.required_named_bonuses);
        grow_vec(&mut self.named_bonuses, capacity.named_bonuses);
        grow_vec(&mut self.fixed_layout_buffer, layout_buffer_len());
        self.supply_origins.grow(&mut capacity.supply_origins);
        let _ = self
            .combats
            .reserve(capacity.rows)
            .map_err(crate::allocation::io);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn capture(&mut self, memory: &impl Memory, base: usize, game_state: usize) -> [u64; N] {
        self.capture_at(memory, base, game_state, None, false)
    }

    pub fn acquire_with_hour(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
    ) -> [u64; N] {
        self.capture_at(memory, base, game_state, Some(hour), true)
    }

    fn capture_at(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: Option<i32>,
        defer_owned: bool,
    ) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        self.combats.clear();
        let mut names = railway_gun_counter_inputs::NameArena {
            bytes: Buffer::Growing(&mut self.names.first),
            used: 0,
            required: 0,
            failures: 0,
        };
        self.sample_hour = hour;
        let mut references = railway_gun_counter_inputs::CombatReferencesArena {
            entries: Buffer::Growing(&mut self.combat_references.first),
            used: 0,
            required: 0,
        };
        let mut tooltip = railway_gun_counter_inputs::TooltipScratch {
            guns: Buffer::Growing(&mut self.tooltip_guns),
            fresh_observations: Buffer::Growing(&mut self.fresh_observations),
            sample_hour: hour,
            named: engine_railways::railway_gun_speed::NamedBonuses {
                pointers: Buffer::Growing(&mut self.named_bonuses),
                required: 0,
            },
            layout: &mut self.fixed_layout,
            layout_buffer: &mut self.fixed_layout_buffer,
            supply_origins: Buffer::Growing(&mut self.supply_origins.first),
            supply_origins_len: 0,
            required_supply_origins: 0,
        };
        let walk = railway_gun_counter_inputs::walk_with_tooltip_and_combat_references(
            memory,
            base,
            game_state,
            &mut self.rows.rows,
            &mut self.paths.first,
            &mut self.combats,
            &mut names,
            &mut tooltip,
            &mut references,
        );
        self.combat_references.len = references.used;
        self.combat_references.required = references.required;
        self.required_named_bonuses = tooltip.named.required;
        self.supply_origins.len = tooltip.supply_origins_len;
        self.supply_origins.required = tooltip.required_supply_origins;
        self.rows.outcome = walk.outcome;
        self.paths.len = walk.paths_len;
        self.paths.required = walk.required_paths;
        self.names.len = names.used;
        self.names.required = names.required;
        NAME_REFUSALS.fetch_add(names.failures as u64, std::sync::atomic::Ordering::Relaxed);
        self.owned_pending = true;
        let walked = qpc();
        let guns = walked.saturating_sub(begin).max(0) as u64;
        drop(_reads);
        let _reads = crate::read_accounting::table(TABLES[1]);
        self.acquire_transport(memory, base, game_state);
        let mut ticks = [guns, qpc().saturating_sub(walked).max(0) as u64];
        if !defer_owned {
            let start = qpc();
            self.decode_owned();
            ticks[0] += qpc().saturating_sub(start).max(0) as u64;
        }
        ticks
    }

    pub fn decode_owned(&mut self) {
        if !std::mem::take(&mut self.owned_pending) {
            return;
        }
        for row in &self.rows.rows[..self.rows.outcome.len()] {
            if let Some(reason) = row.combat_references_failure {
                super::unit_combat_references::observe_failure(reason);
            }
        }
        super::unit_combat_references::demand(
            true,
            self.combat_references.required,
            self.combat_references.first.len(),
        );
        WITHDRAWAL_REFUSALS.fetch_add(
            self.rows.rows[..self.rows.outcome.len()]
                .iter()
                .filter(|row| row.withdrawal.is_none())
                .count() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        ETA_REFUSALS.fetch_add(
            self.rows.rows[..self.rows.outcome.len()]
                .iter()
                .filter(|row| row.ordinary_len != 0 && row.eta.is_none())
                .count() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        let mut fresh_refusals = [0_u64; 4];
        for row in &self.rows.rows[..self.rows.outcome.len()] {
            fresh_refusals[0] += u64::from(row.fresh_failure.is_some());
            for (count, failure) in fresh_refusals[1..].iter_mut().zip(row.fresh_failures) {
                *count += u64::from(failure.is_some());
            }
        }
        for (count, refused) in FRESH_REFUSALS.iter().zip(fresh_refusals) {
            count.fetch_add(refused, std::sync::atomic::Ordering::Relaxed);
        }
        self.decode_transport();
    }

    #[cfg(test)]
    fn capture_transport(&mut self, memory: &impl Memory, base: usize, game_state: usize) {
        self.acquire_transport(memory, base, game_state);
        self.decode_transport();
    }

    fn acquire_transport(&mut self, memory: &impl Memory, base: usize, game_state: usize) {
        self.transport_pending = false;
        self.transport.clear();
        self.owners.clear();
        self.transport_refused.clear();
        self.transport_demand.clear();
        self.transport_provinces.clear();
        self.transport_located.clear();
        self.transport_wanted.clear();
        self.transport_indices.clear();
        self.transport_outcome = match self.transport_rows(memory, base, game_state) {
            Ok(len) => Outcome::Rows { len },
            Err(reason) => {
                self.transport.clear();
                Outcome::Unavailable { reason }
            }
        };
        self.regions.clear();
        self.transport_pending = true;
    }

    fn decode_transport(&mut self) {
        if !std::mem::take(&mut self.transport_pending) {
            return;
        }
        for row in &mut self.transport {
            if let TransportSpeed::Observed(inputs) = row.speed {
                row.speed = TransportSpeed::Ready(inputs.map(|inputs| inputs.evaluate()));
            }
        }
        note_transport_refusals(&self.transport_refused);
        self.speed_cache.finish();
    }

    fn transport_rows(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
    ) -> Result<usize, &'static str> {
        const ALLOCATION: &str = "owner transport speed allocation failed";
        let guns = match self.rows.outcome {
            Outcome::Rows { len } => self
                .rows
                .rows
                .get(..len)
                .ok_or("railway gun census outside its rows")?,
            Outcome::Truncated { .. } => return Err("railway gun census truncated"),
            Outcome::Unavailable { reason } => return Err(reason),
        };
        let paths = self
            .paths
            .first
            .get(..self.paths.len)
            .ok_or("railway-gun path arena length invalid")?;
        let demand = &mut self.transport_demand;
        for gun in guns.iter().filter(|gun| gun.logical_owner as i32 > 0) {
            let route = |start: usize, len: usize| {
                start
                    .checked_add(len)
                    .and_then(|end| paths.get(start..end))
                    .ok_or("railway-gun route lies outside the captured arena")
            };
            let ordinary = route(gun.ordinary_start, gun.ordinary_len)?;
            let alternate = route(gun.alternate_start, gun.alternate_len)?;
            demand
                .try_reserve(1 + ordinary.len() + alternate.len())
                .map_err(|_| ALLOCATION)?;
            demand.extend(
                std::iter::once(gun.province)
                    .chain(ordinary.iter().copied())
                    .chain(alternate.iter().copied())
                    .filter(|&province| province != 0)
                    .map(|province| (gun.logical_owner, province)),
            );
        }
        demand.sort_unstable();
        demand.dedup();
        self.owners
            .try_reserve(demand.len())
            .map_err(|_| ALLOCATION)?;
        self.owners.extend(demand.iter().map(|&(owner, _)| owner));
        self.owners.dedup();
        if self.owners.is_empty() {
            return Ok(0);
        }
        let provinces = &mut self.transport_provinces;
        provinces
            .try_reserve(demand.len())
            .map_err(|_| ALLOCATION)?;
        provinces.extend(demand.iter().map(|&(_, province)| province));
        provinces.sort_unstable();
        provinces.dedup();
        let located = &mut self.transport_located;
        located
            .try_reserve(provinces.len())
            .map_err(|_| ALLOCATION)?;
        self.regions
            .try_reserve(provinces.len())
            .map_err(|_| ALLOCATION)?;
        for &province in provinces.iter() {
            match engine_naval_history::cnaval_unit_transfer::province_region(
                memory, base, game_state, province,
            ) {
                Ok(region) => {
                    self.regions.push(region);
                    located.push((province, region.id));
                }
                Err(reason) => count_refusal(&mut self.transport_refused, reason),
            }
        }
        self.regions.sort_unstable_by_key(|region| region.id);
        self.regions.dedup_by_key(|region| region.id);
        let wanted = &mut self.transport_wanted;
        wanted.try_reserve(demand.len()).map_err(|_| ALLOCATION)?;
        for &(owner, province) in demand.iter() {
            let Ok(at) = located.binary_search_by_key(&province, |&(known, _)| known) else {
                continue;
            };
            let region = located[at].1;
            let index = self
                .regions
                .binary_search_by_key(&region, |known| known.id)
                .map_err(|_| "railway gun province region unlocated")?;
            wanted.push((owner, index));
        }
        wanted.sort_unstable();
        wanted.dedup();
        self.transport
            .try_reserve(wanted.len())
            .map_err(|_| ALLOCATION)?;
        self.speed_cache.begin(self.regions.len())?;
        let indices = &mut self.transport_indices;
        indices
            .try_reserve(self.regions.len())
            .map_err(|_| ALLOCATION)?;
        let (transport, refused, cache) = (
            &mut self.transport,
            &mut self.transport_refused,
            &mut self.speed_cache,
        );
        for &owner in &self.owners {
            indices.clear();
            indices.extend(
                wanted
                    .iter()
                    .filter(|&&(known, _)| known == owner)
                    .map(|&(_, index)| index),
            );
            engine_naval_history::cnaval_unit_transfer::movement_speeds_inputs(
                memory,
                base,
                game_state,
                owner,
                &self.regions,
                indices,
                cache,
                |region, speed| {
                    if let Err(reason) = speed {
                        count_refusal(refused, reason);
                    }
                    transport.push(TransportRow {
                        owner,
                        region: region.id,
                        speed: TransportSpeed::Observed(speed.ok()),
                    });
                },
            );
        }
        Ok(transport.len())
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome, self.transport_outcome]
    }

    fn supply_origins(
        &self,
        row: &railway_gun_counter_inputs::Row,
    ) -> Option<&[engine_native_parts::cunit_supply_inputs::Origin]> {
        let len = row.supply?.origins_len.ok()?;
        let end = row.supply_origins_start.checked_add(len)?;
        self.supply_origins
            .first
            .get(..self.supply_origins.len)?
            .get(row.supply_origins_start..end)
    }

    pub fn tables(&self) -> [TableView; N] {
        [
            self.gun_table(),
            table::<tables::OwnerTransportSpeeds, _>(
                self.transport_outcome,
                &self.transport,
                |r| {
                    let TransportSpeed::Ready(speed) = r.speed else {
                        panic!("owner transport operands not decoded");
                    };
                    OwnerTransportSpeedRow {
                        owner: r.owner,
                        region: r.region,
                        speed,
                    }
                },
            ),
        ]
    }

    fn gun_table(&self) -> TableView {
        let Some(paths) = self.paths.first.get(..self.paths.len) else {
            return unavailable("railway-gun path arena length invalid");
        };
        let rows = &self.rows.rows[..self.rows.outcome.len().min(self.rows.rows.len())];
        if rows.iter().any(|row| {
            [
                row.ordinary_start.checked_add(row.ordinary_len),
                row.alternate_start.checked_add(row.alternate_len),
            ]
            .iter()
            .any(|end| end.is_none_or(|end| end > paths.len()))
        }) {
            return unavailable("railway-gun route lies outside the captured arena");
        }
        let mut handles: Vec<u64> = match collect_vec(rows.iter().map(|row| row.gun)) {
            Ok(handles) => handles,
            Err(_) => return unavailable("collector allocation failed"),
        };
        handles.sort_unstable();
        if handles.windows(2).any(|pair| pair[0] == pair[1]) {
            return unavailable("railway gun appears in more than one roster");
        }
        let route = |start: usize, len: usize| owned_vec(copy_vec(&paths[start..start + len]));
        table::<tables::RailwayGunCounterInputs, _>(self.rows.outcome, &self.rows.rows, |r| {
            RailwayGunCounterInputRow {
                transport: Recorded::Value(replay_data::rows::Retired),
                combat_references: Recorded::Value(super::unit_combat_references::project(
                    self.sample_hour,
                    r.combat_references.and_then(|(start, len)| {
                        self.combat_references
                            .first
                            .get(..self.combat_references.len)?
                            .get(start..start.checked_add(len)?)
                    }),
                    r.combat_count,
                )),
                gun: r.gun,
                roster_owner: r.roster_owner,
                expeditionary_owner: r.expeditionary_owner,
                logical_owner: r.logical_owner,
                province: r.province,
                naval_transfer: u8::from(r.naval_transfer),
                transfer_action_status: u8::from(r.transfer_action_status),
                exiled: u8::from(r.exiled),
                retreat: u8::from(r.retreat),
                strategic_redeployment: u8::from(r.strategic_redeployment),
                reference_entry_count: r.reference_entry_count,
                assignee_group: r.assignee_group,
                subunit_token: r.subunit_token,
                current_strength: r.current_strength,
                maximum_strength: r.maximum_strength,
                received_supply_ratio: r.received_supply_ratio,
                railway_building_level: r.railway_building_level,
                target: r.target,
                combat_role: r.combat_role,
                combat_province: r.combat_province,
                combat_count: r.combat_count,
                attacking: u8::from(r.attacking),
                ordinary_path: route(r.ordinary_start, r.ordinary_len),
                alternate_path: route(r.alternate_start, r.alternate_len),
                name: Recorded::Value(r.name.and_then(|(start, len)| {
                    let bytes = self.names.first.get(..self.names.len)?;
                    let text = bytes.get(start..start.checked_add(len)?)?;
                    Some(owned_vec(copy_vec(text)))
                })),
                eta: Recorded::Value(r.eta.map(|eta| RailwayGunEtaOperands {
                    numerator: eta.numerator,
                    current_speed: eta.current_speed,
                })),
                supply: Recorded::Value(super::supply_inputs::project(
                    r.supply,
                    self.supply_origins(r),
                )),
                withdrawal: Recorded::Value(r.withdrawal),
                fresh: Recorded::Value(
                    r.fresh
                        .filter(|inputs| Some(inputs.sample_hour) == self.sample_hour)
                        .map(|inputs| FreshRailwayCounterEta {
                            sample_hour: replay_data::rows::PacketHour::Host,
                            maximum_speed: inputs.maximum_speed,
                            land_supply_factor: inputs.land_supply_factor,
                            orders_group: inputs.orders_group,
                        }),
                ),
                theatre: Recorded::Value(r.theatre),
                queued_actions: Recorded::Value(r.queued_actions),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;
    use replay_data::rows::TableRows;

    #[test]
    fn sibling_failure_reset_settles_completed_gun_diagnostics_once() {
        use std::sync::atomic::Ordering::Relaxed;
        let counts = || [WITHDRAWAL_REFUSALS.load(Relaxed), ETA_REFUSALS.load(Relaxed)];
        let before = counts();
        let mut captured = Captured::vacant();
        captured.rows.rows.push(railway_gun_counter_inputs::Row {
            ordinary_len: 1,
            ..Default::default()
        });
        captured.rows.outcome = Outcome::Rows { len: 1 };
        captured.owned_pending = true;
        captured.reset();
        assert_eq!(counts(), before.map(|count| count + 1));
        captured.reset();
        captured.decode_owned();
        assert_eq!(counts(), before.map(|count| count + 1));
    }

    #[test]
    fn staged_transport_refusals_keep_native_demand_and_reuse_live_scratch() {
        struct Refused(std::cell::Cell<usize>);
        impl Memory for Refused {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                self.0.set(self.0.get() + 1);
                false
            }
        }
        for count in [0, 1, 7] {
            let memory = Refused(Default::default());
            let mut captured = Captured::vacant();
            captured.rows.rows = (0..count)
                .map(|index| railway_gun_counter_inputs::Row {
                    gun: index as u64 + 1,
                    logical_owner: (index % 23 + 1) as u32,
                    province: (index % 17 + 1) as u32,
                    ..Default::default()
                })
                .collect();
            captured.rows.outcome = Outcome::Rows { len: count };
            captured.acquire_transport(&memory, 0, 0x10);
            let reads = memory.0.get();
            let refusals = captured.transport_refused.clone();
            let capacity = (
                captured.transport_demand.capacity(),
                captured.transport_provinces.capacity(),
                captured.transport_located.capacity(),
                captured.transport_wanted.capacity(),
                captured.transport_indices.capacity(),
            );
            captured.decode_transport();
            assert_eq!(memory.0.get(), reads, "owned decode performs no native read");
            assert!(matches!(captured.transport_outcome, Outcome::Rows { len: 0 }));
            assert!(captured.regions.is_empty());
            memory.0.set(0);
            captured.capture_transport(&memory, 0, 0x10);
            assert_eq!(memory.0.get(), reads);
            assert_eq!(captured.transport_refused, refusals);
            assert_eq!(
                (
                    captured.transport_demand.capacity(),
                    captured.transport_provinces.capacity(),
                    captured.transport_located.capacity(),
                    captured.transport_wanted.capacity(),
                    captured.transport_indices.capacity(),
                ),
                capacity,
            );
        }
    }

    fn gun(handle: u64) -> railway_gun_counter_inputs::Row {
        railway_gun_counter_inputs::Row {
            gun: handle,
            roster_owner: 88,
            expeditionary_owner: 9,
            logical_owner: 10,
            province: 12,
            retreat: true,
            reference_entry_count: 2,
            assignee_group: (3_u64 << 32) | 70,
            subunit_token: 42,
            current_strength: -123_456,
            maximum_strength: 234_567,
            received_supply_ratio: -500,
            railway_building_level: Some(-3),
            target: 13,
            combat_role: 2,
            combat_province: 13,
            combat_count: 3,
            attacking: true,
            ordinary_start: 0,
            ordinary_len: 2,
            alternate_start: 2,
            alternate_len: 1,
            ..Default::default()
        }
    }

    #[test]
    fn projects_native_operands_and_refuses_bad_arenas_and_repeated_guns() {
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 2,
            paths: 3,
            names: 0,
            named_bonuses: 0,
            supply_origins: 0,
            combat_references: 0,
        });
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        capture.capture(&memory, 0, 0);
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.reset();
        capture.rows.outcome = Outcome::Rows { len: 0 };
        assert!(matches!(
            capture.tables()[0],
            TableView::Complete(TableRows::RailwayGunCounterInputs(ref rows)) if rows.is_empty()
        ));
        capture.rows.rows[0] = gun((7_u64 << 32) | 4713);
        capture.paths.first.copy_from_slice(&[13, 14, 15]);
        capture.paths.len = 3;
        capture.rows.outcome = Outcome::Rows { len: 1 };
        let views = capture.tables();
        let TableView::Complete(TableRows::RailwayGunCounterInputs(rows)) = &views[0] else {
            panic!("complete railway-gun table");
        };
        let row = &rows[0];
        assert_eq!(
            (row.ordinary_path.as_slice(), row.alternate_path.as_slice()),
            (&[13, 14][..], &[15][..])
        );
        assert_eq!(row.railway_building_level, Some(-3));
        assert_eq!(
            (row.retreat, row.reference_entry_count, row.exiled),
            (1, 2, 0)
        );
        assert_eq!(row.received_supply_ratio, -500);

        capture.rows.rows[0].alternate_len = 2;
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.rows.rows[0].alternate_len = 1;
        capture.rows.rows[1] = gun((7_u64 << 32) | 4713);
        capture.rows.outcome = Outcome::Rows { len: 2 };
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.rows.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 2,
        };
        assert!(matches!(capture.tables()[0], TableView::Truncated { .. }));
    }

    #[test]
    fn truncated_route_demand_grows_the_arena() {
        let mut capacity = Capacities {
            rows: 1,
            paths: 2,
            names: 0,
            named_bonuses: 0,
            supply_origins: 0,
            combat_references: 0,
        };
        let mut capture = Captured::with_capacities(&capacity);
        capture.rows.outcome = Outcome::Truncated {
            len: 0,
            unread_rows: 1,
        };
        capture.paths.required = 9;
        capture.names.required = 37;
        capture.required_named_bonuses = 73;
        capture.supply_origins.required = 41;
        capture.grow(&mut capacity);
        assert!(capacity.paths >= 9 && capture.paths.first.len() >= 9);
        assert!(capacity.rows >= 1);
        assert!(capacity.names >= 37 && capture.names.first.len() >= 37);
        assert!(capacity.named_bonuses >= 73 && capture.named_bonuses.len() >= 73);
        assert!(capture.tooltip_guns.len() >= capacity.rows);
        assert!(capacity.supply_origins >= 41 && capture.supply_origins.first.len() >= 41);
    }

    #[test]
    fn stored_names_project_exact_bytes_and_bad_spans_refuse_only_the_name() {
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 1,
            paths: 3,
            names: 3,
            named_bonuses: 0,
            supply_origins: 0,
            combat_references: 0,
        });
        capture.paths.first.copy_from_slice(&[13, 14, 15]);
        capture.paths.len = 3;
        capture.names.first.copy_from_slice(&[0xff, 0, b'G']);
        capture.names.len = 3;
        capture.rows.outcome = Outcome::Rows { len: 1 };
        for (span, expected) in [
            (Some((0, 3)), Some(vec![0xff, 0, b'G'])),
            (Some((3, 0)), Some(Vec::new())),
            (Some((2, 2)), None),
            (Some((usize::MAX, 1)), None),
            (None, None),
        ] {
            let mut row = gun((7_u64 << 32) | 4713);
            row.name = span;
            row.eta = Some(railway_gun_counter_inputs::EtaOperands {
                numerator: -12_345,
                current_speed: 54_321,
            });
            capture.rows.rows[0] = row;
            let [
                TableView::Complete(TableRows::RailwayGunCounterInputs(rows)),
                _,
            ] = capture.tables()
            else {
                panic!("name span cannot withhold old operands");
            };
            assert_eq!(rows[0].name, Recorded::Value(expected));
            assert_eq!(
                rows[0].eta,
                Recorded::Value(Some(RailwayGunEtaOperands {
                    numerator: -12_345,
                    current_speed: 54_321,
                }))
            );
            assert_eq!(rows[0].current_strength, -123_456);
            assert_eq!(rows[0].ordinary_path, [13, 14]);
        }
    }

    #[test]
    fn missing_eta_keeps_stored_name_and_counter_operands() {
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 1,
            paths: 3,
            names: 3,
            named_bonuses: 0,
            supply_origins: 0,
            combat_references: 0,
        });
        capture.paths.first.copy_from_slice(&[13, 14, 15]);
        capture.paths.len = 3;
        capture.names.first.copy_from_slice(b"gun");
        capture.names.len = 3;
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture.rows.rows[0] = gun((7_u64 << 32) | 4713);
        capture.rows.rows[0].name = Some((0, 3));
        let [
            TableView::Complete(TableRows::RailwayGunCounterInputs(rows)),
            _,
        ] = capture.tables()
        else {
            panic!("ETA refusal cannot withhold names or counter operands");
        };
        assert_eq!(rows[0].eta, Recorded::Value(None));
        assert_eq!(rows[0].name, Recorded::Value(Some(b"gun".to_vec())));
        assert_eq!(rows[0].current_strength, -123_456);
    }

    #[test]
    fn fresh_eta_projects_independently_of_old_eta_and_refused_domains() {
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 1,
            paths: 3,
            names: 0,
            named_bonuses: 0,
            supply_origins: 0,
            combat_references: 0,
        });
        capture.paths.first.copy_from_slice(&[13, 14, 15]);
        capture.paths.len = 3;
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture.rows.rows[0] = gun((7_u64 << 32) | 4713);
        for fresh in [
            Some(railway_gun_counter_inputs::FreshEta {
                sample_hour: -42,
                maximum_speed: Some(i64::MIN),
                land_supply_factor: Some(i64::MAX),
                orders_group: Some(0),
            }),
            Some(railway_gun_counter_inputs::FreshEta {
                sample_hour: 81,
                maximum_speed: None,
                land_supply_factor: Some(0),
                orders_group: None,
            }),
            None,
        ] {
            capture.rows.rows[0].fresh = fresh;
            capture.sample_hour = Some(0);
            let [
                TableView::Complete(TableRows::RailwayGunCounterInputs(stale)),
                _,
            ] = capture.tables()
            else {
                panic!("fresh refusal cannot withhold counter prefix");
            };
            assert_eq!(stale[0].fresh, Recorded::Value(None));
            capture.sample_hour = fresh.map(|input| input.sample_hour);
            let [
                TableView::Complete(TableRows::RailwayGunCounterInputs(rows)),
                _,
            ] = capture.tables()
            else {
                panic!("fresh refusal cannot withhold counter prefix");
            };
            assert_eq!(
                rows[0].fresh,
                Recorded::Value(fresh.map(|input| FreshRailwayCounterEta {
                    sample_hour: replay_data::rows::PacketHour::Host,
                    maximum_speed: input.maximum_speed,
                    land_supply_factor: input.land_supply_factor,
                    orders_group: input.orders_group,
                }))
            );
            assert_eq!(rows[0].eta, Recorded::Value(None));
            assert_eq!(rows[0].ordinary_path, [13, 14]);
            assert_eq!(rows[0].current_strength, -123_456);
        }
        assert_eq!(capture.fresh_observations.len(), capture.rows.rows.len());
    }

    #[test]
    fn supply_origin_refusals_preserve_exact_scalars_and_other_tooltip_suffixes() {
        use engine_native_parts::cunit_supply_inputs::{Failure, Inputs, Origin};
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 1,
            paths: 3,
            names: 3,
            named_bonuses: 0,
            supply_origins: 2,
            combat_references: 0,
        });
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture.paths.first.copy_from_slice(&[13, 14, 15]);
        capture.paths.len = 3;
        capture.names.first.copy_from_slice(b"gun");
        capture.names.len = 3;
        capture.supply_origins.first.copy_from_slice(&[
            Origin {
                province: 13,
                requested: 200_000,
                received: 50_000,
            },
            Origin {
                province: 13,
                requested: -123,
                received: 0,
            },
        ]);
        capture.supply_origins.len = 2;
        for origins_len in [
            Ok(2),
            Err(Failure::Unreadable),
            Err(Failure::OriginsCapacity { required: 3 }),
        ] {
            let mut row = gun((7_u64 << 32) | 4713);
            row.name = Some((0, 3));
            row.eta = Some(railway_gun_counter_inputs::EtaOperands {
                numerator: 3,
                current_speed: 4,
            });
            row.supply = Some(Inputs {
                asked: -12_345,
                received: 34_500,
                disruption: -56,
                origins_len,
                ..Default::default()
            });
            row.supply_failure = origins_len.err();
            capture.rows.rows[0] = row;
            let [
                TableView::Complete(TableRows::RailwayGunCounterInputs(rows)),
                _,
            ] = capture.tables()
            else {
                panic!("supply origins cannot withhold the counter or scalar footer");
            };
            let Recorded::Value(Some(supply)) = &rows[0].supply else {
                panic!("scalar supply values");
            };
            assert_eq!(
                (supply.asked, supply.received, supply.disruption),
                (-12_000, 35_000, -1_000)
            );
            assert_eq!(supply.origins.as_ref().map(Vec::len), origins_len.ok());
            if let Some(origins) = &supply.origins {
                assert_eq!(
                    (
                        origins[1].province,
                        origins[1].requested,
                        origins[1].received
                    ),
                    (13, -1_000, 0)
                );
            }
            assert_eq!(rows[0].name, Recorded::Value(Some(b"gun".to_vec())));
            assert!(matches!(rows[0].eta, Recorded::Value(Some(_))));
            assert_eq!(rows[0].current_strength, -123_456);
        }
        capture.rows.rows[0].supply = None;
        let [
            TableView::Complete(TableRows::RailwayGunCounterInputs(rows)),
            _,
        ] = capture.tables()
        else {
            panic!("counter prefix");
        };
        assert_eq!(rows[0].supply, Recorded::Value(None));
        assert_eq!(rows[0].name, Recorded::Value(Some(b"gun".to_vec())));
    }
    #[test]
    fn owner_transport_speeds_follow_the_gun_census_and_its_owners() {
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 3,
            paths: 0,
            names: 0,
            named_bonuses: 0,
            supply_origins: 0,
            combat_references: 0,
        });
        let memory = SliceMemory {
            base: 0,
            bytes: &[0; 0x100],
        };
        let transport = |capture: &Captured| capture.tables()[1].clone();
        capture.rows.outcome = Outcome::Unavailable {
            reason: "gun roster refused",
        };
        capture.capture_transport(&memory, 0, 0x10);
        assert!(matches!(
            transport(&capture),
            TableView::Unavailable { ref reason } if reason == "gun roster refused"
        ));
        for owner in [0, u32::MAX] {
            capture.rows.rows[0] = railway_gun_counter_inputs::Row {
                logical_owner: owner,
                ..gun(1)
            };
            capture.rows.outcome = Outcome::Rows { len: 1 };
            capture.capture_transport(&memory, 0, 0x10);
            assert!(matches!(
                transport(&capture),
                TableView::Complete(TableRows::OwnerTransportSpeeds(ref rows)) if rows.is_empty()
            ));
        }
        capture.rows.rows[0] = railway_gun_counter_inputs::Row {
            logical_owner: 10,
            ..gun(1)
        };
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture.capture_transport(&memory, 0, 0x10);
        assert!(matches!(
            transport(&capture),
            TableView::Unavailable { ref reason }
                if reason == "railway-gun route lies outside the captured arena"
        ));
        for (index, owner) in [10, 4, 10].into_iter().enumerate() {
            capture.rows.rows[index] = railway_gun_counter_inputs::Row {
                logical_owner: owner,
                ordinary_len: 0,
                alternate_start: 0,
                alternate_len: 0,
                ..gun(index as u64 + 1)
            };
        }
        capture.rows.outcome = Outcome::Rows { len: 3 };
        capture.capture_transport(&memory, 0, 0x10);
        assert_eq!(capture.owners, [4, 10]);
        assert!(matches!(
            transport(&capture),
            TableView::Complete(TableRows::OwnerTransportSpeeds(ref rows)) if rows.is_empty()
        ));
        assert_eq!(
            capture.transport_refused,
            [("transport province array invalid", 1)]
        );
        assert!(capture.regions.is_empty());
    }
}

#[cfg(test)]
mod combat_reference_projection_tests {
    use super::*;
    use replay_data::rows::TableRows;

    #[test]
    fn railway_owned_reference_projection_and_refusal_preserve_counter_prefix() {
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 1,
            paths: 0,
            names: 0,
            named_bonuses: 0,
            supply_origins: 0,
            combat_references: 2,
        });
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture.rows.rows[0] = railway_gun_counter_inputs::Row {
            gun: u64::MAX,
            current_strength: i64::MIN,
            combat_count: 2,
            combat_references: Some((0, 2)),
            ..Default::default()
        };
        capture.combat_references.first[..2].fill(engine_native::cunit::MapCombatReference {
            combat: 0x8000_0001_0000_1b5a,
            kind: 2,
        });
        capture.combat_references.len = 2;
        capture.sample_hour = Some(31);
        let [
            TableView::Complete(TableRows::RailwayGunCounterInputs(before)),
            _,
        ] = capture.tables()
        else {
            panic!("counter prefix unavailable");
        };
        let Recorded::Value(Some(references)) = &before[0].combat_references else {
            panic!("complete owned suffix unavailable");
        };
        assert_eq!(
            (references.sample_hour, references.entries.len()),
            (replay_data::rows::PacketHour::Host, 2)
        );
        assert_eq!(references.entries[0], references.entries[1]);
        capture.combat_references.len = 1;
        let [
            TableView::Complete(TableRows::RailwayGunCounterInputs(refused)),
            _,
        ] = capture.tables()
        else {
            panic!("reference refusal erased the established prefix");
        };
        assert_eq!(refused[0].combat_references, Recorded::Value(None));
        let mut expected = before[0].clone();
        expected.combat_references = Recorded::Value(None);
        assert_eq!(refused[0], expected);
        capture.reset();
        assert_eq!(
            (capture.sample_hour, capture.combat_references.len),
            (None, 0)
        );
    }
}
