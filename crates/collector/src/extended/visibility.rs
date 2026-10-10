//! Stored simulation inputs of the viewer's fog and unit visibility: the DLC
//! mask, country and occupied-state modifiers, and the relation slots. The
//! relation pass also records each country's enemy list for the raid gate,
//! and each country's strategic navy supplies its region access modes.
//! Each input fails alone, except that one diplomacy pass produces both
//! RelationLinks and CountryWars; the viewer derives fog per country.
use super::country_directed_wars as directed;
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::allocation::{copy_vec, owned_vec};
use engine::{
    Memory, crelation_status, cstrategic_navy, cweather_manager, visibility_inputs as native,
    walk::Outcome,
};
use replay::{
    cadence::{SKIPPED_REASON, should_walk},
    frame::TableView,
    rows::{
        CountryVisibilityRow, CountryWarRow, LocalPlayerRow, NavalRegionAccessRow, RelationLinkRow,
        SimulationFeatureRow, StateVisibilityRow, TableId, tables,
    },
};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

static RUNTIME_SCALAR_REFUSALS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
pub(crate) fn runtime_scalars_data() -> impl serde::Serialize {
    crate::status::map(
        engine::political_runtime_scalars::NAMES
            .into_iter()
            .enumerate()
            .map(|(index, name)| (name, RUNTIME_SCALAR_REFUSALS[index].load(Relaxed))),
    )
}
static INSTANT_DECISIONS_REFUSALS: AtomicU64 = AtomicU64::new(0);
static TRANSPORT_SUBJECT_REFUSALS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn transport_subject_data() -> impl serde::Serialize {
    crate::status::status!({ "transport current subject access refused":
        TRANSPORT_SUBJECT_REFUSALS.load(Relaxed) })
}

pub(crate) fn instant_decisions_data() -> impl serde::Serialize {
    let refused = INSTANT_DECISIONS_REFUSALS.load(Relaxed);
    crate::status::status!({
        "refused": refused,
        "refusals": crate::status::map(std::iter::once(("instant decisions byte unreadable", refused)).filter(|(_, count)| *count != 0)),
        "scope": "cumulative appended instant-decision read failures; other simulation features remain available",
    })
}

pub const N: usize = 7;
pub const TABLES: [TableId; N] = [
    TableId::SimulationFeatures,
    TableId::CountryVisibility,
    TableId::StateVisibility,
    TableId::RelationLinks,
    TableId::CountryWars,
    TableId::NavalRegionAccess,
    TableId::LocalPlayer,
];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    countries: usize,
    states: usize,
    links: usize,
    wars: usize,
    /// Enemy links of every war row.
    war_links: usize,
    naval_access: usize,
    directed: directed::Capacities,
    queries: usize,
}
impl Capacities {
    /// Advisory live demand at attach and the input-demand cadence.
    pub(crate) fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        use crate::country_scale::{raise, table_demand::rows};
        let mut failure = None;
        let mut accept = |capacity: &mut usize, outcome: Outcome| match rows(outcome) {
            Ok(count) => raise(capacity, count),
            Err(reason) => {
                failure.get_or_insert(reason);
            }
        };
        let (countries, slots) = native::countries(memory, base, state, &mut []);
        accept(&mut self.countries, countries);
        self.raise_for_countries(slots);
        accept(
            &mut self.states,
            native::states(memory, base, state, &mut []),
        );
        accept(
            &mut self.naval_access,
            cstrategic_navy::access_modes(memory, base, state, &mut []),
        );
        let mut resolver = crelation_status::Resolver::default();
        let (links, directed) = crelation_status::links_observed_transport_demands(
            memory,
            base,
            state,
            &mut [],
            crelation_status::WarOutput {
                rows: &mut [],
                countries: &mut [],
            },
            &mut resolver,
            Some(crelation_status::DirectedWarOutput {
                owners: &mut [],
                cells: &mut [],
            }),
            0,
            &[],
        );
        accept(&mut self.links, links.links);
        accept(&mut self.wars, links.wars);
        raise(&mut self.war_links, links.required_countries);
        raise(&mut self.directed.owners, directed.required_owners);
        raise(&mut self.directed.cells, directed.required_cells);
        self.queries = self.queries.max(crate::directed_war_queries::demand());
        failure.map_or(Ok(()), Err)
    }

    /// At most one modifier or war row per country slot; relation links and
    /// war links start at the same size and grow from observed demand. Never
    /// lowers a capacity.
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        self.directed.raise_for_countries(slots);
        self.queries = self.queries.max(slots);
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
    weather_suspend: Option<[u8; 2]>,
    instant_decisions: Option<u8>,
    runtime_scalars: replay::rows::PoliticalRuntimeScalars,
}

pub struct Captured {
    directed: directed::Captured,
    queries: Arena<engine::directed_war_queries::Demand>,
    query_admission: Result<(), &'static str>,
    dlc_mask: CaptureRows<FeatureInputs>,
    local_player: CaptureRows<[u32; 2]>,
    countries: CaptureRows<native::CountryRow>,
    states: CaptureRows<native::StateRow>,
    links: CaptureRows<crelation_status::LinkRow>,
    /// Country authentication cache, grown on workers from the original
    /// country/link headers before the sparse pass.
    resolver: crelation_status::Resolver,
    wars: CaptureRows<crelation_status::WarRow>,
    war_links: Arena<u32>,
    naval_access: CaptureRows<cstrategic_navy::AccessRow>,
    /// Live country slots seen by the last walk.
    slots: usize,
    weather_suspend_failure: Option<&'static str>,
}

impl Captured {
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.directed.fail(reason);
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
    }

    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self {
            directed: directed::Captured::new(Default::default()),
            queries: Arena::new(0),
            query_admission: Err("directed war query original callback witness unavailable"),
            dlc_mask: CaptureRows::new(0),
            local_player: CaptureRows::new(0),
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
            directed: directed::Captured::new(capacity.directed),
            queries: Arena::new(capacity.queries),
            query_admission: Err("directed war query original callback witness unavailable"),
            dlc_mask: CaptureRows::new(1),
            local_player: CaptureRows::new(1),
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
        self.directed.reset();
        self.queries.reset();
        self.weather_suspend_failure = None;
        self.dlc_mask.reset();
        self.local_player.reset();
        self.countries.reset();
        self.states.reset();
        self.links.reset();
        self.wars.reset();
        self.war_links.reset();
        self.naval_access.reset();
    }
    pub(crate) fn weather_suspend_failure(&self) -> Option<&'static str> {
        self.weather_suspend_failure
    }

    pub fn allocated_bytes(&self) -> usize {
        self.directed.allocated_bytes()
            + self.queries.allocated_bytes()
            + self.local_player.allocated_bytes()
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
        self.queries.required = self
            .queries
            .required
            .max(crate::directed_war_queries::demand());
        self.queries.grow(&mut capacity.queries);
        self.directed.grow(&mut capacity.directed);
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

    #[cfg(test)]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.capture_storage(memory, base, game_state, hour, force_full, false)
    }

    /// Worker-owned rows and arenas grow from original headers before publication.
    pub fn capture_growing(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.capture_storage(memory, base, game_state, hour, force_full, true)
    }

    fn capture_storage(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        growing: bool,
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
                    _ => self.local_player.outcome = skipped,
                }
                continue;
            }
            // The RelationLinks pass already produced CountryWars; share its
            // outcome instead of walking the diplomacy statuses twice.
            if index == 4 && should_walk(TABLES[3], hour, force_full) {
                continue;
            }
            let started = qpc();
            match index {
                0 => {
                    let globals = engine::political_runtime_scalars::capture(memory, base, hour);
                    let values = globals.values.map(|value| value.ok());
                    for (index, value) in globals.values.iter().enumerate() {
                        if value.is_err() {
                            RUNTIME_SCALAR_REFUSALS[index].fetch_add(1, Relaxed);
                        }
                    }
                    let runtime_scalars = replay::rows::PoliticalRuntimeScalars {
                        sample_hour: globals.sample_hour,
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
                                human_ai: native::human_ai(memory, base)?,
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
                    (self.countries.outcome, self.slots) = if growing {
                        native::countries_growing(
                            memory,
                            base,
                            game_state,
                            &mut self.countries.rows,
                        )
                    } else {
                        native::countries(memory, base, game_state, &mut self.countries.rows)
                    }
                }
                2 => {
                    self.states.outcome = if growing {
                        native::states_growing(memory, base, game_state, &mut self.states.rows)
                    } else {
                        native::states(memory, base, game_state, &mut self.states.rows)
                    }
                }
                5 => {
                    self.naval_access.outcome = cstrategic_navy::access_modes_buffer(
                        memory,
                        base,
                        game_state,
                        if growing {
                            engine::walk::Buffer::Growing(&mut self.naval_access.rows)
                        } else {
                            engine::walk::Buffer::Fixed(&mut self.naval_access.rows)
                        },
                    )
                }
                6 => {
                    self.local_player.outcome =
                        match engine::cgame_state::local_player(memory, game_state) {
                            Ok(row) => {
                                self.local_player.rows[0] = row;
                                Outcome::Rows { len: 1 }
                            }
                            Err(reason) => Outcome::Unavailable { reason },
                        };
                }
                _ => {
                    self.directed.begin(hour);
                    // The callback verdict is copied before dispatch, never
                    // reconstructed from a worker's TLS. Completed graphics
                    // and the same world protect these fresh borrowed cells.
                    let query_world = crate::world_lifetime::token();
                    if growing
                        && let Err(reason) = engine::walk::Buffer::Growing(&mut self.queries.first)
                            .ensure(crate::directed_war_queries::demand())
                    {
                        self.directed.fail(reason);
                        self.links.outcome = Outcome::Unavailable { reason };
                        self.wars.outcome = Outcome::Unavailable { reason };
                        ticks[index] = qpc().saturating_sub(started).max(0) as u64;
                        continue;
                    }
                    let requests = crate::directed_war_queries::drain(&mut self.queries.first);
                    self.queries.required = requests.required;
                    self.queries.len = if self.query_admission.is_ok() && requests.reason.is_none()
                    {
                        requests.len
                    } else {
                        if self.query_admission.is_err() && requests.len != 0 {
                            crate::directed_war_queries::refuse_admission();
                        }
                        0
                    };
                    let (walk, directed) = if growing {
                        crelation_status::links_observed_transport_demands_growing(
                            memory,
                            base,
                            game_state,
                            &mut self.links.rows,
                            crelation_status::WarBuffers {
                                rows: engine::walk::Buffer::Growing(&mut self.wars.rows),
                                countries: engine::walk::Buffer::Growing(&mut self.war_links.first),
                            },
                            &mut self.resolver,
                            Some(self.directed.output_growing()),
                            hour,
                            &self.queries.first[..self.queries.len],
                        )
                    } else {
                        crelation_status::links_observed_transport_demands(
                            memory,
                            base,
                            game_state,
                            &mut self.links.rows,
                            crelation_status::WarOutput {
                                rows: &mut self.wars.rows,
                                countries: &mut self.war_links.first,
                            },
                            &mut self.resolver,
                            Some(self.directed.output()),
                            hour,
                            &self.queries.first[..self.queries.len],
                        )
                    };
                    ORDERED_ENEMY_REFUSALS.fetch_add(u64::from(walk.ordered_refusals), Relaxed);
                    self.directed.finish(directed);
                    if self.queries.len != 0 && !crate::world_lifetime::current(query_world) {
                        self.directed
                            .fail("directed war query world changed during fresh acquisition");
                    }
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

    pub(crate) fn directed_wars(&self) -> &directed::Captured {
        &self.directed
    }

    pub(crate) fn set_directed_query_admission(&mut self, admission: Result<(), &'static str>) {
        self.query_admission = admission;
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
                    weather_suspend: replay::rows::Recorded::Value(r.weather_suspend),
                    instant_decisions: replay::rows::Recorded::Value(r.instant_decisions),
                    runtime_scalars: replay::rows::Recorded::Value(r.runtime_scalars),
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
                    transport_subject: replay::rows::Recorded::Value(
                        r.transport_subject.and_then(Result::ok).map(|value| {
                            replay::rows::TransportSubjectAccess {
                                sample_hour: value.sample_hour,
                                complete_owner: value.complete_owner,
                                target_raw: value.target_raw,
                                subject: value.subject.map(|subject| {
                                    replay::rows::ZeroSubjectInputs {
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
            ordered: replay::rows::Recorded::Value(r.ordered.and_then(|(start, len)| {
                Some(replay::rows::OrderedEnemyInputs {
                    sample_hour: r.ordered_hour?,
                    entries: owned_vec(copy_vec(span(start, len)?)),
                })
            })),
        })
    }
}

#[cfg(test)]
mod tests;

static ORDERED_ENEMY_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub(crate) fn ordered_enemy_data() -> impl serde::Serialize {
    crate::status::status!({"ordered_diplomacy_enemy_capture_refused":ORDERED_ENEMY_REFUSALS.load(Relaxed)})
}
