//! Stored simulation inputs of the viewer's fog and unit visibility: the DLC
//! mask, country and occupied-state modifiers, and the attache and air-base
//! access slots. Each table fails alone; the viewer derives fog per country.
use crate::{sampling::qpc, spooling::table, table_capture::CaptureRows};
use engine::{Memory, crelation_status, visibility_inputs as native, walk::Outcome};
use replay::{
    cadence::{SKIPPED_REASON, should_walk},
    frame::TableView,
    rows::{
        CountryVisibilityRow, RelationLinkRow, SimulationFeatureRow, StateVisibilityRow, TableId,
        tables,
    },
};

pub const N: usize = 4;
pub const TABLES: [TableId; N] = [
    TableId::SimulationFeatures,
    TableId::CountryVisibility,
    TableId::StateVisibility,
    TableId::RelationLinks,
];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    countries: usize,
    states: usize,
    links: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            countries: 256,
            states: 1024,
            links: 2048,
        }
    }
}
impl Capacities {
    /// At most one modifier row per country slot; relation links start at the
    /// same size. Never lowers a capacity.
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        self.countries = self.countries.max(slots);
        self.links = self.links.max(slots);
    }
}

pub struct Captured {
    dlc_mask: CaptureRows<u32>,
    countries: CaptureRows<native::CountryRow>,
    states: CaptureRows<native::StateRow>,
    links: CaptureRows<crelation_status::LinkRow>,
    /// Country authentication cache for the link walk, sized by `grow` from
    /// the headers the last walk saw; the walk itself never allocates.
    resolver: crelation_status::Resolver,
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
    }

    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self {
            dlc_mask: CaptureRows::new(0),
            countries: CaptureRows::new(0),
            states: CaptureRows::new(0),
            links: CaptureRows::new(0),
            resolver: crelation_status::Resolver::default(),
            slots: 0,
        }
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            dlc_mask: CaptureRows::new(1),
            countries: CaptureRows::new(capacity.countries),
            states: CaptureRows::new(capacity.states),
            links: CaptureRows::new(capacity.links),
            resolver: crelation_status::Resolver::default(),
            slots: 0,
        }
    }
    pub fn reset(&mut self) {
        self.dlc_mask.reset();
        self.countries.reset();
        self.states.reset();
        self.links.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.dlc_mask.allocated_bytes()
            + self.countries.allocated_bytes()
            + self.states.allocated_bytes()
            + self.links.allocated_bytes()
            + self.resolver.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        capacity.raise_for_countries(self.slots);
        self.countries.grow(&mut capacity.countries);
        self.states.grow(&mut capacity.states);
        self.links.grow(&mut capacity.links);
        self.resolver.grow();
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
                    _ => self.links.outcome = skipped,
                }
                continue;
            }
            let started = qpc();
            match index {
                0 => {
                    self.dlc_mask.outcome = match native::dlc_mask(memory, base) {
                        Ok(mask) => {
                            self.dlc_mask.rows[0] = mask;
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
                _ => {
                    self.links.outcome =
                        crelation_status::links(
                            memory,
                            base,
                            game_state,
                            &mut self.links.rows,
                            &mut self.resolver,
                        )
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
        ]
    }

    pub fn tables(&self) -> [TableView; N] {
        [
            table::<tables::SimulationFeatures, _>(
                self.dlc_mask.outcome,
                &self.dlc_mask.rows,
                |&dlc_mask| SimulationFeatureRow { dlc_mask },
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
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn growth_sizes_country_rows_from_the_live_slot_count() {
        let mut capacity = Capacities::default();
        let mut captured = Captured::with_capacities(&capacity);
        captured.slots = 5_000;
        captured.grow(&mut capacity);
        assert_eq!((capacity.countries, capacity.links), (5_000, 5_000));
        assert!(captured.countries.rows.len() >= 5_000);
    }

    #[test]
    fn one_failed_input_leaves_the_others_and_skips_off_slot_static_mask() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.fail_capture(1, false, "worker stopped");
        let outcomes = captured.outcomes();
        assert_eq!(
            outcomes[0],
            Outcome::Unavailable {
                reason: SKIPPED_REASON
            }
        );
        assert!(outcomes[1..].iter().all(|outcome| *outcome
            == Outcome::Unavailable {
                reason: "worker stopped"
            }));
        captured.links.outcome = Outcome::Rows { len: 1 };
        captured.links.rows[0] = crelation_status::LinkRow {
            country: 5,
            target: 9,
            links: crelation_status::LINK_ATTACHE,
        };
        let [_, _, _, TableView::Complete(rows)] = captured.tables() else {
            panic!("links table stays complete");
        };
        assert_eq!(rows.len(), 1);
    }
}
