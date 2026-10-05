//! Stored simulation inputs of the viewer's fog and unit visibility: the DLC
//! mask, country and occupied-state modifiers, and the relation slots. The
//! relation pass also records each country's enemy list for the raid gate,
//! and each country's strategic navy supplies its region access modes.
//! Each input fails alone, except that one diplomacy pass produces both
//! RelationLinks and CountryWars; the viewer derives fog per country.
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    countries: usize,
    states: usize,
    links: usize,
    wars: usize,
    /// Enemy links of every war row.
    war_links: usize,
    naval_access: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            countries: 256,
            states: 1024,
            links: 2048,
            wars: 256,
            war_links: 2048,
            naval_access: 1024,
        }
    }
}
impl Capacities {
    /// At most one modifier or war row per country slot; relation links and
    /// war links start at the same size and grow from observed demand. Never
    /// lowers a capacity.
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
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
}

pub struct Captured {
    dlc_mask: CaptureRows<FeatureInputs>,
    local_player: CaptureRows<[u32; 2]>,
    countries: CaptureRows<native::CountryRow>,
    states: CaptureRows<native::StateRow>,
    links: CaptureRows<crelation_status::LinkRow>,
    /// Country authentication cache for the link walk, sized by `grow` from
    /// the headers the last walk saw; the walk itself never allocates.
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
        self.local_player.allocated_bytes()
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
                    self.dlc_mask.outcome =
                        match native::dlc_mask(memory, base).and_then(|dlc_mask| {
                            Ok(FeatureInputs {
                                dlc_mask,
                                human_ai: native::human_ai(memory, base)?,
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
                    (self.countries.outcome, self.slots) =
                        native::countries(memory, base, game_state, &mut self.countries.rows)
                }
                2 => {
                    self.states.outcome =
                        native::states(memory, base, game_state, &mut self.states.rows)
                }
                5 => {
                    self.naval_access.outcome = cstrategic_navy::access_modes(
                        memory,
                        base,
                        game_state,
                        &mut self.naval_access.rows,
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
                    let walk = crelation_status::links(
                        memory,
                        base,
                        game_state,
                        &mut self.links.rows,
                        crelation_status::WarOutput {
                            rows: &mut self.wars.rows,
                            countries: &mut self.war_links.first,
                        },
                        &mut self.resolver,
                    );
                    self.links.outcome = walk.links;
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
                    weather_suspend: r.weather_suspend,
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
        })
    }
}

#[cfg(test)]
mod tests;
