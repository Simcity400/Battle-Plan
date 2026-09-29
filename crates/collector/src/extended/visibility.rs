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
use engine::{
    Memory, crelation_status, cstrategic_navy, visibility_inputs as native, walk::Outcome,
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

pub struct Captured {
    dlc_mask: CaptureRows<(u32, u8)>,
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
        }
    }
    pub fn reset(&mut self) {
        self.dlc_mask.reset();
        self.local_player.reset();
        self.countries.reset();
        self.states.reset();
        self.links.reset();
        self.wars.reset();
        self.war_links.reset();
        self.naval_access.reset();
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
        self.resolver.grow();
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
                    self.dlc_mask.outcome = match native::dlc_mask(memory, base).and_then(
                        |mask| Ok((mask, native::human_ai(memory, base)?)),
                    ) {
                        Ok(row) => {
                            self.dlc_mask.rows[0] = row;
                            Outcome::Rows { len: 1 }
                        }
                        Err(reason) => Outcome::Unavailable { reason },
                    }
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
                |&(dlc_mask, human_ai)| SimulationFeatureRow { dlc_mask, human_ai },
            ),
            table::<tables::CountryVisibility, _>(
                self.countries.outcome,
                &self.countries.rows,
                |r| CountryVisibilityRow {
                    country: r.country,
                    unit_modifier: r.unit_modifier,
                },
            ),
            table::<tables::StateVisibility, _>(
                self.states.outcome,
                &self.states.rows,
                |r| StateVisibilityRow {
                    state: r.state,
                    country: r.country,
                    network_modifier: r.network_modifier,
                },
            ),
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
        let span = |start: usize, len: usize| start.checked_add(len).and_then(|end| links.get(start..end));
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
            enemies: span(r.enemies_start, r.enemies_len).unwrap().to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_player_switch_observer_failure_and_recovery_round_trip() {
        use replay::frame::{DecodedFrame, Frame, FrameDecoder, FrameEncoder, Metrics};
        let mut captured = Captured::with_capacities(&Capacities::default());
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        // Unchanged, a controller change, an observer view, no view, a failed
        // read and recovery, each decoded through the codec.
        let samples = [
            Some([4_999_u32, 0]),
            Some([4_999, 0]),
            Some([5_000, 0]),
            Some([0, 4_999]),
            Some([0, 0]),
            None,
            Some([5_000, 0]),
        ];
        for (hour, sample) in samples.into_iter().enumerate() {
            let bytes: Vec<_> = sample
                .into_iter()
                .flatten()
                .flat_map(u32::to_le_bytes)
                .collect();
            let memory = engine::memory::SliceMemory {
                base: 0x1000 + engine::cgame_state::LOCAL_PLAYER_COUNTRY,
                bytes: &bytes,
            };
            captured.capture(&memory, 0, 0x1000, hour as i32, hour == 0);
            let mut tables = std::array::from_fn(|_| TableView::Unavailable {
                reason: "fixture".into(),
            });
            tables[TableId::LocalPlayer.index()] = captured.tables()[6].clone();
            let frame = encoder
                .encode(
                    DecodedFrame {
                        branch: 1,
                        hour: hour as i32,
                        tables: tables.map(Into::into),
                        metrics: Metrics::default(),
                    },
                    false,
                )
                .unwrap();
            let mut bytes = Vec::new();
            frame.encode(&mut bytes).unwrap();
            let wire = Frame::decode(&mut replay::codec::Decoder::new(&bytes)).unwrap();
            let decoded = decoder.apply(&wire).unwrap();
            match sample {
                Some([country, observer_country]) => assert_eq!(
                    decoded.complete::<tables::LocalPlayer>().unwrap(),
                    &[LocalPlayerRow {
                        country,
                        observer_country
                    }]
                ),
                None => assert!(decoded.complete::<tables::LocalPlayer>().is_err()),
            }
        }
        captured.fail_capture(9, false, "worker stopped");
        assert_eq!(
            captured.outcomes()[6],
            Outcome::Unavailable {
                reason: "worker stopped"
            }
        );
    }

    #[test]
    fn growth_sizes_country_rows_from_the_live_slot_count() {
        let mut capacity = Capacities::default();
        let mut captured = Captured::with_capacities(&capacity);
        captured.slots = 5_000;
        captured.grow(&mut capacity);
        assert_eq!(
            (
                capacity.countries,
                capacity.links,
                capacity.wars,
                capacity.war_links,
                capacity.naval_access
            ),
            (5_000, 5_000, 5_000, 5_000, 5_000)
        );
        assert!(captured.countries.rows.len() >= 5_000);
    }

    #[test]
    fn one_failed_input_leaves_the_others_and_the_hourly_features_fail_too() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.fail_capture(1, false, "worker stopped");
        let outcomes = captured.outcomes();
        // SimulationFeatures is hourly: `human_ai` can be toggled at runtime.
        // NavalRegionAccess is daily: off its slot a failure is a skip.
        assert!(outcomes[..5].iter().all(|outcome| *outcome
            == Outcome::Unavailable {
                reason: "worker stopped"
            }));
        assert_eq!(
            outcomes[5],
            Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON
            }
        );
        captured.links.outcome = Outcome::Rows { len: 1 };
        captured.links.rows[0] = crelation_status::LinkRow {
            country: 5,
            target: 9,
            links: crelation_status::LINK_ATTACHE,
        };
        let [_, _, _, TableView::Complete(rows), _, _, _] = captured.tables() else {
            panic!("links table stays complete");
        };
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn war_rows_convert_their_arena_spans_and_refuse_one_outside_it() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.war_links.first[..3].copy_from_slice(&[7, 8, 9]);
        captured.war_links.len = 3;
        captured.wars.rows[0] = crelation_status::WarRow {
            country: 4,
            can_occupy_non_war: 1,
            enemies_read: true,
            enemies_start: 1,
            enemies_len: 2,
        };
        captured.wars.outcome = Outcome::Rows { len: 1 };
        let TableView::Complete(replay::rows::TableRows::CountryWars(rows)) =
            captured.wars_table()
        else {
            panic!("war table stays complete");
        };
        assert_eq!(
            rows,
            [CountryWarRow {
                country: 4,
                can_occupy_non_war: 1,
                enemies_read: 1,
                enemies: vec![8, 9],
            }]
        );
        captured.wars.rows[0].enemies_len = 3;
        assert!(matches!(
            captured.wars_table(),
            TableView::Unavailable { .. }
        ));
    }
}
