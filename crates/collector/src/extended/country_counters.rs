//! Hourly country counter scalars, stored ordered state/leader rosters,
//! civil-war references and owned wargoals. Arenas follow live header sums.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{Memory, ccountry, cwar_goal, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{
        CountryCivilWarRow, CountryCounterInputRow, OwnedWargoalRow, Recorded, TableId, tables,
    },
};

pub const N: usize = 3;
pub const TABLES: [TableId; N] = [
    TableId::CountryCounterInputs,
    TableId::CountryCivilWars,
    TableId::OwnedWargoals,
];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Default)]
pub(crate) struct RosterWork {
    captured: u64,
    refusals: Vec<(&'static str, u64)>,
    states_required: usize,
    leaders_required: usize,
}

impl RosterWork {
    fn observe(&mut self, capture: &Captured) -> std::io::Result<()> {
        let rows = capture
            .rows
            .rows
            .get(..capture.rows.outcome.len())
            .ok_or_else(|| std::io::Error::other("country counter row count exceeds buffer"))?;
        for row in rows {
            let reason = row.roster_reason.or_else(|| {
                (row.state_rosters.is_none() || row.leader_rosters.is_none())
                    .then_some("country roster inputs unobserved")
            });
            if let Some(reason) = reason {
                let index = match self.refusals.binary_search_by_key(&reason, |entry| entry.0) {
                    Ok(index) => index,
                    Err(index) => {
                        engine::allocation::reserve(&mut self.refusals, 1)?;
                        self.refusals.insert(index, (reason, 0));
                        index
                    }
                };
                self.refusals[index].1 = self.refusals[index].1.saturating_add(1);
            } else {
                self.captured = self.captured.saturating_add(1);
            }
        }
        self.states_required = capture.states.required;
        self.leaders_required = capture.leaders.required;
        Ok(())
    }

    pub(crate) fn status(&self) -> impl serde::Serialize + '_ {
        crate::status::status!({
            "captured": self.captured,
            "refused": self.refusals.iter().fold(0_u64, |sum, (_, count)| sum.saturating_add(*count)),
            "refusals": crate::status::map(self.refusals.iter().map(|&(reason, count)| (reason, count))),
            "states_required": self.states_required,
            "leaders_required": self.leaders_required,
            "scope": "hourly country roster inputs; refused lists are absent while country scalar operands remain available",
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    civil_wars: usize,
    states: usize,
    leaders: usize,
    wargoals: usize,
    wargoal_states: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 512,
            civil_wars: 64,
            states: 0,
            leaders: 0,
            wargoals: 0,
            wargoal_states: 0,
        }
    }
}
impl Capacities {
    /// One row per canonical country slot.
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(&mut self.rows, crate::country_scale::rows(slots, 1));
    }

    /// CountryCivilWars from the live entry total
    /// (`country_scale::observe_politics`), with the three-quarters growth
    /// rule's headroom. Never lowers.
    pub(crate) fn raise_for_civil_wars(&mut self, entries: usize) {
        crate::country_scale::raise(&mut self.civil_wars, crate::country_scale::rows(entries, 1));
    }

    pub(crate) fn raise_for_rosters(
        &mut self,
        rosters: ccountry::RosterDemand,
        goals: cwar_goal::Demand,
    ) {
        let raise = crate::country_scale::raise;
        let rows = crate::country_scale::rows;
        raise(&mut self.states, rows(rosters.states, 1));
        raise(&mut self.leaders, rows(rosters.leaders, 1));
        raise(&mut self.wargoals, rows(goals.rows, 1));
        raise(&mut self.wargoal_states, rows(goals.states, 1));
    }
}

pub struct Captured {
    rows: CaptureRows<ccountry::CounterInputsRow>,
    civil_wars: CaptureRows<ccountry::CivilWarRow>,
    states: Arena<u32>,
    leaders: Arena<u64>,
    wargoals: CaptureRows<cwar_goal::Row>,
    wargoal_states: Arena<u32>,
}
impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            civil_wars: 0,
            ..Default::default()
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            civil_wars: CaptureRows::new(capacity.civil_wars),
            states: Arena::new(capacity.states),
            leaders: Arena::new(capacity.leaders),
            wargoals: CaptureRows::new(capacity.wargoals),
            wargoal_states: Arena::new(capacity.wargoal_states),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.civil_wars.reset();
        self.states.reset();
        self.leaders.reset();
        self.wargoals.reset();
        self.wargoal_states.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.civil_wars.allocated_bytes()
            + self.states.allocated_bytes()
            + self.leaders.allocated_bytes()
            + self.wargoals.allocated_bytes()
            + self.wargoal_states.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        self.civil_wars.grow(&mut capacity.civil_wars);
        self.states.grow(&mut capacity.states);
        self.leaders.grow(&mut capacity.leaders);
        self.wargoals.grow(&mut capacity.wargoals);
        self.wargoal_states.grow(&mut capacity.wargoal_states);
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let mut ticks = [0; N];
        {
            let _reads = crate::read_accounting::table(TABLES[0]);
            let begin = qpc();
            let mut rosters = ccountry::CounterRosters {
                states: &mut self.states.first,
                leaders: &mut self.leaders.first,
                state_len: 0,
                leader_len: 0,
                required: Default::default(),
            };
            self.rows.outcome = ccountry::walk_counter_inputs_with_rosters(
                memory,
                base,
                state,
                &mut self.rows.rows,
                &mut rosters,
            );
            self.states.len = rosters.state_len;
            self.states.required = rosters.required.states;
            self.leaders.len = rosters.leader_len;
            self.leaders.required = rosters.required.leaders;
            ticks[0] = qpc().saturating_sub(begin).max(0) as u64;
        }
        {
            let _reads = crate::read_accounting::table(TABLES[1]);
            let begin = qpc();
            self.civil_wars.outcome =
                ccountry::walk_civil_wars(memory, base, state, &mut self.civil_wars.rows);
            ticks[1] = qpc().saturating_sub(begin).max(0) as u64;
        }
        let _reads = crate::read_accounting::table(TABLES[2]);
        let begin = qpc();
        let goals = cwar_goal::walk(
            memory,
            base,
            state,
            &mut self.wargoals.rows,
            &mut self.wargoal_states.first,
        );
        self.wargoals.outcome = goals.outcome;
        self.wargoal_states.len = goals.state_len;
        self.wargoal_states.required = goals.required.states;
        ticks[2] = qpc().saturating_sub(begin).max(0) as u64;
        ticks
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.rows.outcome,
            self.civil_wars.outcome,
            self.wargoals.outcome,
        ]
    }

    pub(crate) fn observe_rosters(&self, work: &mut RosterWork) -> std::io::Result<()> {
        work.observe(self)
    }

    pub fn tables(&self) -> [TableView; N] {
        [
            self.counter_table(),
            table::<tables::CountryCivilWars, _>(
                self.civil_wars.outcome,
                &self.civil_wars.rows,
                |r| CountryCivilWarRow {
                    country: r.country,
                    ordinal: r.ordinal,
                    target: r.target,
                },
            ),
            self.wargoal_table(),
        ]
    }

    fn counter_table(&self) -> TableView {
        let Some(rows) = self.rows.rows.get(..self.rows.outcome.len()) else {
            return unavailable("country counter row count exceeds allocated buffer");
        };
        for row in rows {
            if row.state_rosters.is_some_and(|spans| {
                spans
                    .iter()
                    .any(|span| !valid_span(&self.states, span.start, span.count))
            }) {
                return unavailable("country state roster span outside acquired arena");
            }
            if row.leader_rosters.is_some_and(|spans| {
                spans
                    .iter()
                    .any(|span| !valid_span(&self.leaders, span.start, span.count))
            }) {
                return unavailable("country leader roster span outside acquired arena");
            }
        }
        table::<tables::CountryCounterInputs, _>(self.rows.outcome, rows, |r| {
            CountryCounterInputRow {
                country: r.country,
                pride_of_the_fleet: r.pride_of_the_fleet,
                owned_state_count: r.owned_state_count,
                controlled_state_count: r.controlled_state_count,
                first_division: r.first_division,
                owned_states: Recorded::Value(
                    r.state_rosters.map(|spans| self.state_list(spans[0])),
                ),
                controlled_states: Recorded::Value(
                    r.state_rosters.map(|spans| self.state_list(spans[1])),
                ),
                core_states: Recorded::Value(
                    r.state_rosters.map(|spans| self.state_list(spans[2])),
                ),
                army_leaders: Recorded::Value(
                    r.leader_rosters.map(|spans| self.leader_list(spans[0])),
                ),
                navy_leaders: Recorded::Value(
                    r.leader_rosters.map(|spans| self.leader_list(spans[1])),
                ),
            }
        })
    }

    fn wargoal_table(&self) -> TableView {
        let Some(rows) = self.wargoals.rows.get(..self.wargoals.outcome.len()) else {
            return unavailable("owned wargoal row count exceeds allocated buffer");
        };
        if rows
            .iter()
            .any(|row| !valid_span(&self.wargoal_states, row.state_start, row.state_count))
        {
            return unavailable("owned wargoal state span outside acquired arena");
        }
        table::<tables::OwnedWargoals, _>(self.wargoals.outcome, rows, |r| OwnedWargoalRow {
            country: r.country,
            ordinal: r.ordinal,
            recipient: r.recipient,
            goal_type: r.goal_type,
            states: engine::allocation::owned_vec(engine::allocation::copy_vec(
                &self.wargoal_states.first[r.state_start..r.state_start + r.state_count],
            )),
        })
    }

    fn state_list(&self, span: ccountry::RosterSpan) -> Vec<u32> {
        engine::allocation::owned_vec(engine::allocation::copy_vec(
            &self.states.first[span.start..span.start + span.count],
        ))
    }

    fn leader_list(&self, span: ccountry::RosterSpan) -> Vec<u64> {
        engine::allocation::owned_vec(engine::allocation::copy_vec(
            &self.leaders.first[span.start..span.start + span.count],
        ))
    }
}

fn valid_span<T>(arena: &Arena<T>, start: usize, count: usize) -> bool {
    start
        .checked_add(count)
        .is_some_and(|end| end <= arena.len && end <= arena.first.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::SliceMemory;
    #[test]
    fn unreadable_census_is_unavailable_and_country_operands_preserve_primary_values() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 1,
            civil_wars: 2,
            ..Default::default()
        });
        capture.capture(&memory, 0, 0);
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        assert!(matches!(capture.tables()[1], TableView::Unavailable { .. }));
        capture.reset();
        capture.rows.rows[0] = ccountry::CounterInputsRow {
            country: 7,
            pride_of_the_fleet: (32 << 32) | 51,
            owned_state_count: 3,
            controlled_state_count: 5,
            first_division: (9 << 32) | 4,
            ..Default::default()
        };
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture.civil_wars.rows[0] = ccountry::CivilWarRow {
            country: 7,
            ordinal: 0,
            target: 0x8000_0002,
        };
        capture.civil_wars.outcome = Outcome::Rows { len: 1 };
        let [
            TableView::Complete(replay::rows::TableRows::CountryCounterInputs(rows)),
            TableView::Complete(replay::rows::TableRows::CountryCivilWars(wars)),
            _,
        ] = capture.tables()
        else {
            panic!("complete");
        };
        assert_eq!(rows[0].pride_of_the_fleet, (32 << 32) | 51);
        assert_eq!(rows[0].country, 7);
        assert_eq!(rows[0].owned_state_count, 3);
        assert_eq!(rows[0].first_division, (9 << 32) | 4);
        assert_eq!(rows[0].controlled_state_count, 5);
        assert_eq!(
            wars,
            [CountryCivilWarRow {
                country: 7,
                ordinal: 0,
                target: 0x8000_0002,
            }]
        );
    }

    #[test]
    fn civil_war_capacity_follows_the_live_entry_total() {
        let mut capacity = Capacities::default();
        capacity.raise_for_civil_wars(3_000);
        assert!(capacity.civil_wars >= 4_000);
        let raised = capacity;
        capacity.raise_for_civil_wars(1);
        assert!(capacity == raised);
    }

    #[test]
    fn live_roster_demand_sizes_5000_country_arenas_and_projection_owns_every_operand() {
        let mut capacity = Capacities::default();
        capacity.raise_for_countries(5_001);
        capacity.raise_for_rosters(
            ccountry::RosterDemand {
                states: 45_000,
                leaders: 30_000,
            },
            cwar_goal::Demand {
                rows: 5_000,
                states: 20_000,
            },
        );
        assert!(capacity.states >= 60_000 && capacity.leaders >= 40_000);
        assert!(capacity.wargoals >= 6_667 && capacity.wargoal_states >= 26_667);
        let mut captured = Captured::with_capacities(&capacity);
        for index in 0..5_000 {
            let country = index as u32 + 1;
            let state_start = index * 9;
            for list in 0..3 {
                captured.states.first[state_start + list * 3..state_start + list * 3 + 3]
                    .copy_from_slice(&[country + 10_000, 0, country]);
            }
            let leader_start = index * 6;
            let handle = (u64::from(country) << 32) | 4713;
            for list in 0..2 {
                captured.leaders.first[leader_start + list * 3..leader_start + list * 3 + 3]
                    .copy_from_slice(&[handle, 0, handle]);
            }
            captured.rows.rows[index] = ccountry::CounterInputsRow {
                country,
                owned_state_count: 3,
                controlled_state_count: 3,
                state_rosters: Some(std::array::from_fn(|list| ccountry::RosterSpan {
                    start: state_start + list * 3,
                    count: 3,
                })),
                leader_rosters: Some(std::array::from_fn(|list| ccountry::RosterSpan {
                    start: leader_start + list * 3,
                    count: 3,
                })),
                ..Default::default()
            };
            captured.wargoal_states.first[index * 4..index * 4 + 4].copy_from_slice(&[
                country,
                0,
                country + 10_000,
                country,
            ]);
            captured.wargoals.rows[index] = cwar_goal::Row {
                country,
                recipient: 0x8000_0000 | country,
                goal_type: 123,
                state_start: index * 4,
                state_count: 4,
                ..Default::default()
            };
        }
        captured.rows.outcome = Outcome::Rows { len: 5_000 };
        captured.wargoals.outcome = Outcome::Rows { len: 5_000 };
        captured.states.len = 45_000;
        captured.leaders.len = 30_000;
        captured.wargoal_states.len = 20_000;
        let [
            TableView::Complete(replay::rows::TableRows::CountryCounterInputs(rows)),
            _,
            TableView::Complete(replay::rows::TableRows::OwnedWargoals(goals)),
        ] = captured.tables()
        else {
            panic!("complete");
        };
        captured.states.first.fill(0);
        captured.leaders.first.fill(0);
        captured.wargoal_states.first.fill(0);
        for (index, row) in rows.iter().enumerate() {
            let country = index as u32 + 1;
            let expected_states = [country + 10_000, 0, country];
            assert_eq!(
                row.owned_states
                    .require("CountryCounterInputs.owned_states")
                    .unwrap()
                    .as_deref(),
                Some(expected_states.as_slice())
            );
            assert_eq!(
                row.controlled_states
                    .require("CountryCounterInputs.controlled_states")
                    .unwrap()
                    .as_deref(),
                Some(expected_states.as_slice())
            );
            assert_eq!(
                row.core_states
                    .require("CountryCounterInputs.core_states")
                    .unwrap()
                    .as_deref(),
                Some(expected_states.as_slice())
            );
            let handle = (u64::from(country) << 32) | 4713;
            let expected_leaders = [handle, 0, handle];
            assert_eq!(
                row.army_leaders
                    .require("CountryCounterInputs.army_leaders")
                    .unwrap()
                    .as_deref(),
                Some(expected_leaders.as_slice())
            );
            assert_eq!(
                row.navy_leaders
                    .require("CountryCounterInputs.navy_leaders")
                    .unwrap()
                    .as_deref(),
                Some(expected_leaders.as_slice())
            );
            assert_eq!(goals[index].states, [country, 0, country + 10_000, country]);
        }
        let raised = capacity;
        capacity.raise_for_rosters(Default::default(), Default::default());
        assert!(capacity == raised);
    }

    #[test]
    fn observed_empty_owned_goals_are_complete_and_unobserved_inputs_remain_absent() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.rows.rows[0] = ccountry::CounterInputsRow {
            country: 1,
            ..Default::default()
        };
        captured.rows.outcome = Outcome::Rows { len: 1 };
        captured.wargoals.outcome = Outcome::Rows { len: 0 };
        let [
            TableView::Complete(replay::rows::TableRows::CountryCounterInputs(rows)),
            _,
            TableView::Complete(replay::rows::TableRows::OwnedWargoals(goals)),
        ] = captured.tables()
        else {
            panic!("complete");
        };
        assert_eq!(rows[0].owned_states, Recorded::Value(None));
        assert_eq!(rows[0].army_leaders, Recorded::Value(None));
        assert!(goals.is_empty());
        captured.rows.rows[0].state_rosters = Some([ccountry::RosterSpan::default(); 3]);
        captured.rows.rows[0].leader_rosters = Some([ccountry::RosterSpan::default(); 2]);
        let views = captured.tables();
        let TableView::Complete(replay::rows::TableRows::CountryCounterInputs(rows)) = &views[0]
        else {
            panic!("observed empty rosters are present");
        };
        assert_eq!(rows[0].owned_states, Recorded::Value(Some(Vec::new())));
        assert_eq!(rows[0].controlled_states, Recorded::Value(Some(Vec::new())));
        assert_eq!(rows[0].core_states, Recorded::Value(Some(Vec::new())));
        assert_eq!(rows[0].army_leaders, Recorded::Value(Some(Vec::new())));
        assert_eq!(rows[0].navy_leaders, Recorded::Value(Some(Vec::new())));
        captured.reset();
        assert!(matches!(
            captured.tables()[2],
            TableView::Unavailable { .. }
        ));
    }

    #[test]
    fn roster_shortfalls_keep_5000_scalar_rows_and_count_each_native_reason() {
        let mut capacity = Capacities::default();
        capacity.raise_for_countries(5_001);
        let mut captured = Captured::with_capacities(&capacity);
        for (index, row) in captured.rows.rows[..5_000].iter_mut().enumerate() {
            let country = index as u32 + 1;
            *row = ccountry::CounterInputsRow {
                country,
                pride_of_the_fleet: (u64::from(country) << 32) | 51,
                owned_state_count: 7,
                controlled_state_count: 11,
                first_division: (u64::from(country) << 32) | 4,
                roster_reason: Some(if index % 2 == 0 {
                    "country state roster arena capacity exceeded"
                } else {
                    "unit leader vtable mismatch"
                }),
                ..Default::default()
            };
        }
        captured.rows.outcome = Outcome::Rows { len: 5_000 };
        captured.states.required = 45_000;
        captured.leaders.required = 30_000;
        let [
            TableView::Complete(replay::rows::TableRows::CountryCounterInputs(rows)),
            _,
            _,
        ] = captured.tables()
        else {
            panic!("scalar rows unavailable");
        };
        assert_eq!(rows.len(), 5_000);
        for row in rows {
            assert_eq!(row.pride_of_the_fleet, (u64::from(row.country) << 32) | 51);
            assert_eq!(row.first_division, (u64::from(row.country) << 32) | 4);
            assert_eq!((row.owned_state_count, row.controlled_state_count), (7, 11));
            assert_eq!(row.owned_states, Recorded::Value(None));
            assert_eq!(row.controlled_states, Recorded::Value(None));
            assert_eq!(row.core_states, Recorded::Value(None));
            assert_eq!(row.army_leaders, Recorded::Value(None));
            assert_eq!(row.navy_leaders, Recorded::Value(None));
        }
        let mut work = RosterWork::default();
        captured.observe_rosters(&mut work).unwrap();
        assert_eq!(work.captured, 0);
        assert_eq!(
            work.refusals,
            [
                ("country state roster arena capacity exceeded", 2_500),
                ("unit leader vtable mismatch", 2_500),
            ]
        );
        let status = serde_json::to_value(work.status()).unwrap();
        assert_eq!(status["refused"], 5_000);
        assert_eq!(status["states_required"], 45_000);
        captured.reset();
        captured.observe_rosters(&mut work).unwrap();
        assert_eq!(work.refusals[0].1, 2_500);
    }

    #[test]
    fn invalid_arena_spans_are_named_unavailable_before_any_slice() {
        let mut captured = Captured::with_capacities(&Capacities {
            rows: 1,
            states: 4,
            leaders: 4,
            wargoals: 1,
            wargoal_states: 4,
            ..Default::default()
        });
        captured.rows.outcome = Outcome::Rows { len: 1 };
        captured.wargoals.outcome = Outcome::Rows { len: 1 };
        captured.states.len = 2;
        captured.leaders.len = 2;
        captured.wargoal_states.len = 2;
        for (start, count) in [(usize::MAX, 1), (1, 2), (4, 1)] {
            let span = ccountry::RosterSpan { start, count };
            captured.rows.rows[0].state_rosters = Some([span; 3]);
            assert!(matches!(
                &captured.tables()[0],
                TableView::Unavailable { reason }
                    if reason == "country state roster span outside acquired arena"
            ));
            captured.rows.rows[0].state_rosters = None;
            captured.rows.rows[0].leader_rosters = Some([span; 2]);
            assert!(matches!(
                &captured.tables()[0],
                TableView::Unavailable { reason }
                    if reason == "country leader roster span outside acquired arena"
            ));
            captured.rows.rows[0].leader_rosters = None;
            captured.wargoals.rows[0].state_start = start;
            captured.wargoals.rows[0].state_count = count;
            assert!(matches!(
                &captured.tables()[2],
                TableView::Unavailable { reason }
                    if reason == "owned wargoal state span outside acquired arena"
            ));
        }
    }
}
