mod carry;
use crate::{
    clock::qpc,
    table_capture::{Arena, CaptureRows},
    table_conversion::{table, unavailable},
    world_input_changes::{self as changes, Changes, Domain, Held, Plan},
};
use engine_allocation::{copy_vec, owned_vec};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::{crelation::status as crelation_status, cweather_manager};
use engine_native_naval::cstrategic_navy;
use engine_visibility::visibility_inputs as native;
use replay_data::{
    cadence::SKIPPED_REASON, cadence::should_walk, rows::CountryVisibilityRow, rows::CountryWarRow,
    rows::LandUnitOrderRow, rows::LocalPlayerRow, rows::NavalRegionAccessRow, rows::RelationLinkRow,
    rows::SimulationFeatureRow, rows::StateVisibilityRow, rows::TableId, rows::tables,
};
use replay_frame::frame::TableView;
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering::Relaxed},
};

static HELD_ACCESS: Mutex<Held<Vec<cstrategic_navy::AccessRow>>> =
    Mutex::new(Held::new(Vec::new()));

static HELD_VISIBILITY: Mutex<carry::Store> = Mutex::new(carry::Store::new());

static RUNTIME_SCALAR_REFUSALS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
static MOVEMENT_DEFINE_REFUSALS: [AtomicU64; engine_land_inputs::movement_defines::DEFINES.len()] =
    [const { AtomicU64::new(0) }; engine_land_inputs::movement_defines::DEFINES.len()];
pub fn movement_defines_data() -> impl serde::Serialize {
    crate::status::map(
        engine_land_inputs::movement_defines::DEFINES
            .into_iter()
            .enumerate()
            .map(|(index, define)| (define.name, MOVEMENT_DEFINE_REFUSALS[index].load(Relaxed))),
    )
}
pub fn runtime_scalars_data() -> impl serde::Serialize {
    crate::status::map(
        engine_stability::political_runtime_scalars::NAMES
            .into_iter()
            .enumerate()
            .map(|(index, name)| (name, RUNTIME_SCALAR_REFUSALS[index].load(Relaxed))),
    )
}
static INSTANT_DECISIONS_REFUSALS: AtomicU64 = AtomicU64::new(0);
static TRANSPORT_SUBJECT_REFUSALS: AtomicU64 = AtomicU64::new(0);

pub fn transport_subject_data() -> impl serde::Serialize {
    crate::status::status!({ "transport current subject access refused":
        TRANSPORT_SUBJECT_REFUSALS.load(Relaxed) })
}

pub fn instant_decisions_data() -> impl serde::Serialize {
    let refused = INSTANT_DECISIONS_REFUSALS.load(Relaxed);
    crate::status::status!({
        "refused": refused,
        "refusals": crate::status::map(std::iter::once(("instant decisions byte unreadable", refused)).filter(|(_, count)| *count != 0)),
        "scope": "cumulative appended instant-decision read failures; other simulation features remain available",
    })
}

pub const N: usize = 8;
pub const TABLES: [TableId; N] = [
    TableId::SimulationFeatures,
    TableId::CountryVisibility,
    TableId::StateVisibility,
    TableId::RelationLinks,
    TableId::CountryWars,
    TableId::NavalRegionAccess,
    TableId::LocalPlayer,
    TableId::LandUnitOrder,
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    countries: usize,
    states: usize,
    links: usize,
    wars: usize,
    war_links: usize,
    naval_access: usize,
}

collector_primitives::capacity_max!(Capacities {
    countries,
    states,
    links,
    wars,
    war_links,
    naval_access
});
impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        use crate::country_scale::raise;
        use collector_extended_support::demand::rows;
        let mut failure = None;
        let mut accept = |capacity: &mut usize, outcome: Outcome| match rows(outcome) {
            Ok(count) => raise(capacity, count),
            Err(reason) => {
                failure.get_or_insert(reason);
            }
        };
        let (countries, slots) = native::count_countries(memory, base, state);
        accept(&mut self.countries, countries);
        self.raise_for_countries(slots);
        accept(
            &mut self.states,
            native::count_states(memory, base, state),
        );
        accept(
            &mut self.naval_access,
            cstrategic_navy::count_access_modes(memory, base, state),
        );
        let mut resolver = crelation_status::Resolver::default();
        let links = crelation_status::count_links_observed_transport(memory, base, state, &mut resolver, 0);
        accept(&mut self.links, links.links);
        accept(&mut self.wars, links.wars);
        raise(&mut self.war_links, links.required_countries);
        failure.map_or(Ok(()), Err)
    }

    pub fn raise_for_countries(&mut self, slots: usize) {
        self.countries = self.countries.max(slots);
        self.links = self.links.max(slots);
        self.wars = self.wars.max(slots);
        self.war_links = self.war_links.max(slots);
        self.naval_access = self.naval_access.max(slots);
    }
}

#[derive(Default, Clone, Copy)]
struct FeatureInputs {
    dlc_mask: u32,
    human_ai: u8,
    state_array_len: Option<u32>,
    weather_suspend: Option<[u8; 2]>,
    instant_decisions: Option<u8>,
    runtime_scalars: replay_data::rows::PoliticalRuntimeScalars,
    movement_defines: replay_data::rows::MovementDefines,
}

fn movement_defines(memory: &impl Memory, base: usize) -> replay_data::rows::MovementDefines {
    let values = engine_land_inputs::movement_defines::capture(memory, base);
    for (index, value) in values.iter().enumerate() {
        if value.is_err() {
            MOVEMENT_DEFINE_REFUSALS[index].fetch_add(1, Relaxed);
        }
    }
    let [
        out_of_supply_speed,
        retreat_speed,
        withdrawal_speed,
        out_of_fuel_speed,
        fuel_capacity_scale,
        supply_days,
        supply_grace,
        air_speed,
        air_defense_denominator,
        air_defense_reduction,
        out_of_supply_attrition,
    ] = values.map(Result::ok);
    replay_data::rows::MovementDefines {
        out_of_supply_speed,
        retreat_speed,
        withdrawal_speed,
        out_of_fuel_speed,
        fuel_capacity_scale,
        supply_days,
        supply_grace,
        air_speed,
        air_defense_denominator,
        air_defense_reduction,
        out_of_supply_attrition,
    }
}

pub struct Captured {
    dlc_mask: CaptureRows<FeatureInputs>,
    local_player: CaptureRows<[u32; 2]>,
    land_order: CaptureRows<Option<usize>>,
    land_units: Vec<u64>,
    countries: CaptureRows<native::CountryRow>,
    states: CaptureRows<native::StateRow>,
    links: CaptureRows<crelation_status::LinkRow>,
    resolver: crelation_status::Resolver,
    wars: CaptureRows<crelation_status::WarRow>,
    war_links: Arena<u32>,
    naval_access: CaptureRows<cstrategic_navy::AccessRow>,
    slots: usize,
    weather_suspend_failure: Option<&'static str>,
}

impl Captured {
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        if let Ok(mut held) = HELD_VISIBILITY.try_lock() {
            held.forget();
        }
        self.dlc_mask
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
        self.countries
            .fail(super::failed_capture(TABLES[1], hour, force_full, reason));
        self.states
            .fail(super::failed_capture(TABLES[2], hour, force_full, reason));
        self.links
            .fail(super::failed_capture(TABLES[3], hour, force_full, reason));
        self.wars
            .fail(super::failed_capture(TABLES[4], hour, force_full, reason));
        self.naval_access
            .fail(super::failed_capture(TABLES[5], hour, force_full, reason));
        self.local_player
            .fail(super::failed_capture(TABLES[6], hour, force_full, reason));
        self.land_order
            .fail(super::failed_capture(TABLES[7], hour, force_full, reason));
    }

    pub fn vacant() -> Self {
        Self {
            dlc_mask: CaptureRows::new(0),
            local_player: CaptureRows::new(0),
            land_order: CaptureRows::new(0),
            land_units: Vec::new(),
            countries: CaptureRows::new(0),
            states: CaptureRows::new(0),
            links: CaptureRows::new(0),
            resolver: crelation_status::Resolver::default(),
            wars: CaptureRows::new(0),
            war_links: Arena::new(0),
            naval_access: CaptureRows::new(0),
            slots: 0,
            weather_suspend_failure: None,
        }
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            dlc_mask: CaptureRows::new(1),
            local_player: CaptureRows::new(1),
            land_order: CaptureRows::new(1),
            land_units: Vec::new(),
            countries: CaptureRows::new(capacity.countries),
            states: CaptureRows::new(capacity.states),
            links: CaptureRows::new(capacity.links),
            resolver: crelation_status::Resolver::default(),
            wars: CaptureRows::new(capacity.wars),
            war_links: Arena::new(capacity.war_links),
            naval_access: CaptureRows::new(capacity.naval_access),
            slots: 0,
            weather_suspend_failure: None,
        }
    }
    pub fn reset(&mut self) {
        self.weather_suspend_failure = None;
        self.dlc_mask.reset();
        self.local_player.reset();
        self.land_order.reset();
        self.countries.reset();
        self.states.reset();
        self.links.reset();
        self.wars.reset();
        self.war_links.reset();
        self.naval_access.reset();
    }
    pub fn weather_suspend_failure(&self) -> Option<&'static str> {
        self.weather_suspend_failure
    }

    pub fn allocated_bytes(&self) -> usize {
        self.local_player.allocated_bytes()
            + self.land_order.allocated_bytes()
            + self.land_units.capacity() * size_of::<u64>()
            + self.dlc_mask.allocated_bytes()
            + self.countries.allocated_bytes()
            + self.states.allocated_bytes()
            + self.links.allocated_bytes()
            + self.resolver.allocated_bytes()
            + self.wars.allocated_bytes()
            + self.war_links.allocated_bytes()
            + self.naval_access.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        capacity.raise_for_countries(self.slots);
        self.countries.grow(&mut capacity.countries);
        self.states.grow(&mut capacity.states);
        self.links.grow(&mut capacity.links);
        if self.resolver.grow().is_err() {
            return;
        }
        self.wars.grow(&mut capacity.wars);
        self.war_links.grow(&mut capacity.war_links);
        self.naval_access.grow(&mut capacity.naval_access);
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.capture_storage(memory, base, game_state, hour, force_full)
    }

    fn capture_naval_access(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        force_full: bool,
        changes: &impl Changes,
        held: &Mutex<Held<Vec<cstrategic_navy::AccessRow>>>,
    ) {
        let rows = &mut self.naval_access.rows;
        let mut held = held.try_lock().ok();
        let plan = held.as_ref().map_or(Plan::READ, |held| {
            held.plan(Domain::RegionAccess, changes, force_full)
        });
        if plan.carry
            && let Some(held) = &held
        {
            let carried = held.carried(Domain::RegionAccess);
            let ensured =
                engine_native_memory::walk::Buffer::Growing(rows).ensure(carried.len());
            self.naval_access.outcome = match ensured {
                Ok(_) if carried.len() <= rows.len() => {
                    rows[..carried.len()].copy_from_slice(carried);
                    Outcome::Rows { len: carried.len() }
                }
                Ok(_) => Outcome::Truncated {
                    len: 0,
                    unread_rows: carried.len() as u32,
                },
                Err(reason) => Outcome::Unavailable { reason },
            };
            return;
        }
        let outcome = cstrategic_navy::access_modes_buffer(
            memory,
            base,
            game_state,
            engine_native_memory::walk::Buffer::Growing(rows),
        );
        self.naval_access.outcome = outcome;
        if let Some(held) = &mut held {
            let read = match outcome {
                Outcome::Rows { len } => Some(&rows[..len]),
                _ => None,
            };
            held.settle(
                Domain::RegionAccess,
                plan.read(Domain::RegionAccess, changes),
                read.is_some(),
                |rows| read.is_some_and(|read| changes::assign(rows, read)),
            );
        }
    }

    fn capture_storage(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.reset();
        let mut ticks = [0; N];
        let skipped = Outcome::Unavailable {
            reason: SKIPPED_REASON,
        };
        for (index, table) in TABLES.into_iter().enumerate() {
            let _reads = crate::read_accounting::table(table);
            if !should_walk(table, hour, force_full) {
                match index {
                    0 => self.dlc_mask.outcome = skipped,
                    1 => self.countries.outcome = skipped,
                    2 => self.states.outcome = skipped,
                    3 => self.links.outcome = skipped,
                    4 => self.wars.outcome = skipped,
                    5 => self.naval_access.outcome = skipped,
                    6 => self.local_player.outcome = skipped,
                    _ => self.land_order.outcome = skipped,
                }
                continue;
            }
            if index == 4 && should_walk(TABLES[3], hour, force_full) {
                continue;
            }
            let started = qpc();
            match index {
                0 => {
                    let globals =
                        engine_stability::political_runtime_scalars::capture(memory, base, hour);
                    let values = globals.values.map(|value| value.ok());
                    for (index, value) in globals.values.iter().enumerate() {
                        if value.is_err() {
                            RUNTIME_SCALAR_REFUSALS[index].fetch_add(1, Relaxed);
                        }
                    }
                    let runtime_scalars = replay_data::rows::PoliticalRuntimeScalars {
                        sample_hour: replay_data::rows::PacketHour::Host,
                        river_small: values[0],
                        river_large: values[1],
                        night: values[2],
                        fort: values[3],
                        move_strength_threshold: values[4],
                        move_organisation_threshold: values[5],
                    };
                    self.dlc_mask.outcome =
                        match native::dlc_mask(memory, base).and_then(|dlc_mask| {
                            Ok(FeatureInputs {
                                dlc_mask,
                                runtime_scalars,
                                movement_defines: movement_defines(memory, base),
                                human_ai: native::human_ai(memory, base)?,
                                state_array_len: None,
                                instant_decisions: native::instant_decisions(memory, base)
                                    .inspect_err(|_| {
                                        INSTANT_DECISIONS_REFUSALS.fetch_add(1, Relaxed);
                                    })
                                    .ok(),
                                weather_suspend: cweather_manager::Reader::new(
                                    memory, base, game_state,
                                )
                                .and_then(|reader| reader.suspend_flags())
                                .inspect_err(|reason| {
                                    self.weather_suspend_failure = Some(*reason);
                                })
                                .ok(),
                            })
                        }) {
                            Ok(row) => {
                                self.dlc_mask.rows[0] = row;
                                Outcome::Rows { len: 1 }
                            }
                            Err(reason) => Outcome::Unavailable { reason },
                        };
                }
                1 => {
                    let held = (cfg!(feature = "visibility_changes"))
                        .then(|| HELD_VISIBILITY.try_lock().ok())
                        .flatten();
                    (self.countries.outcome, self.slots) = if let Some(mut held) = held {
                        held.capture(
                            memory,
                            base,
                            game_state,
                            engine_native_memory::walk::Buffer::Growing(&mut self.countries.rows),
                            &crate::country_input_changes::Observer,
                            force_full,
                        )
                        .0
                    } else {
                        native::countries_buffer(
                            memory,
                            base,
                            game_state,
                            engine_native_memory::walk::Buffer::Growing(&mut self.countries.rows),
                        )
                    };
                }
                2 => {
                    let mut extent = None;
                    self.states.outcome = native::states_buffer_extent(
                        memory,
                        base,
                        game_state,
                        engine_native_memory::walk::Buffer::Growing(&mut self.states.rows),
                        &mut extent,
                    );
                    if let Some(features) = self.dlc_mask.rows.first_mut() {
                        features.state_array_len = extent;
                    }
                }
                5 => self.capture_naval_access(
                    memory,
                    base,
                    game_state,
                    force_full,
                    &changes::Observer,
                    &HELD_ACCESS,
                ),
                6 => {
                    self.local_player.outcome =
                        match engine_native::cgame_state::local_player(memory, game_state) {
                            Ok(row) => {
                                self.local_player.rows[0] = row;
                                Outcome::Rows { len: 1 }
                            }
                            Err(reason) => Outcome::Unavailable { reason },
                        };
                }
                7 => {
                    let units = engine_native::cunit::viewer_land_units(
                        memory,
                        base,
                        game_state,
                        engine_native_memory::walk::Buffer::Growing(&mut self.land_units),
                    );
                    self.land_order.rows[0] = match units {
                        Outcome::Rows { len } => Some(len),
                        _ => None,
                    };
                    self.land_order.outcome = Outcome::Rows { len: 1 };
                }
                _ => {
                    let walk = {
                        crelation_status::links_observed_transport(
                            memory,
                            base,
                            game_state,
                            &mut self.links.rows,
                            crelation_status::WarBuffers {
                                rows: engine_native_memory::walk::Buffer::Growing(&mut self.wars.rows),
                                countries: engine_native_memory::walk::Buffer::Growing(
                                    &mut self.war_links.first,
                                ),
                            },
                            &mut self.resolver,
                            hour,
                        )
                    };
                    ORDERED_ENEMY_REFUSALS.fetch_add(u64::from(walk.ordered_refusals), Relaxed);
                    self.links.outcome = walk.links;
                    if let Outcome::Rows { len } | Outcome::Truncated { len, .. } = walk.links {
                        for row in &self.links.rows[..len] {
                            if row.transport_subject.is_none_or(|value| value.is_err()) {
                                TRANSPORT_SUBJECT_REFUSALS.fetch_add(1, Relaxed);
                            }
                        }
                    }
                    self.wars.outcome = walk.wars;
                    self.war_links.len = walk.countries_len;
                    self.war_links.required = walk.required_countries;
                }
            }
            ticks[index] = qpc().saturating_sub(started).max(0) as u64;
        }
        ticks
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.dlc_mask.outcome,
            self.countries.outcome,
            self.states.outcome,
            self.links.outcome,
            self.wars.outcome,
            self.naval_access.outcome,
            self.local_player.outcome,
            self.land_order.outcome,
        ]
    }

    pub fn tables(&self) -> [TableView; N] {
        [
            table::<tables::SimulationFeatures, _>(
                self.dlc_mask.outcome,
                &self.dlc_mask.rows,
                |r| SimulationFeatureRow {
                    dlc_mask: r.dlc_mask,
                    human_ai: r.human_ai,
                    state_array_len: replay_data::rows::Recorded::Value(r.state_array_len),
                    weather_suspend: replay_data::rows::Recorded::Value(r.weather_suspend),
                    instant_decisions: replay_data::rows::Recorded::Value(r.instant_decisions),
                    runtime_scalars: replay_data::rows::Recorded::Value(r.runtime_scalars),
                    movement_defines: replay_data::rows::Recorded::Value(r.movement_defines),
                },
            ),
            table::<tables::CountryVisibility, _>(
                self.countries.outcome,
                &self.countries.rows,
                |r| CountryVisibilityRow {
                    country: r.country,
                    unit_modifier: r.unit_modifier,
                },
            ),
            table::<tables::StateVisibility, _>(self.states.outcome, &self.states.rows, |r| {
                StateVisibilityRow {
                    state: r.state,
                    country: r.country,
                    network_modifier: r.network_modifier,
                }
            }),
            table::<tables::RelationLinks, _>(self.links.outcome, &self.links.rows, |r| {
                RelationLinkRow {
                    country: r.country,
                    target_index: r.target,
                    links: r.links,
                    transport_subject: replay_data::rows::Recorded::Value(
                        r.transport_subject.and_then(Result::ok).map(|value| {
                            replay_data::rows::TransportSubjectAccess {
                                sample_hour: replay_data::rows::PacketHour::Host,
                                complete_owner: value.complete_owner,
                                target_raw: value.target_raw,
                                subject: value.subject.map(|subject| {
                                    replay_data::rows::ZeroSubjectInputs {
                                        relation_type: subject.relation_type,
                                        parties: subject.parties,
                                    }
                                }),
                            }
                        }),
                    ),
                }
            }),
            self.wars_table(),
            table::<tables::NavalRegionAccess, _>(
                self.naval_access.outcome,
                &self.naval_access.rows,
                |r| NavalRegionAccessRow {
                    country: r.country,
                    region: r.region,
                    mode: r.mode,
                },
            ),
            table::<tables::LocalPlayer, _>(
                self.local_player.outcome,
                &self.local_player.rows,
                |&[country, observer_country]| LocalPlayerRow {
                    country,
                    observer_country,
                },
            ),
            table::<tables::LandUnitOrder, _>(self.land_order.outcome, &self.land_order.rows, |len| {
                LandUnitOrderRow {
                    units: replay_data::rows::Recorded::Value(
                        len.and_then(|len| self.land_units.get(..len))
                            .map(|units| owned_vec(copy_vec(units))),
                    ),
                }
            }),
        ]
    }

    fn wars_table(&self) -> TableView {
        let Some(links) = self.war_links.first.get(..self.war_links.len) else {
            return unavailable("country war link arena length invalid");
        };
        let span =
            |start: usize, len: usize| start.checked_add(len).and_then(|end| links.get(start..end));
        let outcome = self.wars.outcome;
        let rows = &self.wars.rows[..outcome.len().min(self.wars.rows.len())];
        if rows
            .iter()
            .any(|r| span(r.enemies_start, r.enemies_len).is_none())
        {
            return unavailable("country war links outside captured arena");
        }
        table::<tables::CountryWars, _>(outcome, &self.wars.rows, |r| CountryWarRow {
            country: r.country,
            can_occupy_non_war: r.can_occupy_non_war,
            enemies_read: u8::from(r.enemies_read),
            enemies: owned_vec(copy_vec(span(r.enemies_start, r.enemies_len).unwrap())),
            ordered: replay_data::rows::Recorded::Value(r.ordered.and_then(|(start, len)| {
                r.ordered_hour?;
                Some(replay_data::rows::OrderedEnemyInputs {
                    sample_hour: replay_data::rows::PacketHour::Host,
                    entries: owned_vec(copy_vec(span(start, len)?)),
                })
            })),
        })
    }
}

#[cfg(test)]
mod tests;

static ORDERED_ENEMY_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub fn ordered_enemy_data() -> impl serde::Serialize {
    crate::status::status!({"ordered_diplomacy_enemy_capture_refused":ORDERED_ENEMY_REFUSALS.load(Relaxed)})
}
