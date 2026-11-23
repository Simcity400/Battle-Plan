use crate::{
    clock::qpc,
    table_capture::{Arena, CaptureRows, grow_capacity},
    table_conversion::{table, unavailable},
};
use engine_allocation::{copy_vec, owned_vec};
use engine_land_inputs::cfaction_theater_manager as native;
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::cfaction;
use replay_data::{
    cadence::SKIPPED_REASON, cadence::should_walk, rows::FactionAdvisorRow,
    rows::FactionTheatreRow, rows::TableId, rows::tables,
};
use replay_frame::frame::TableView;

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::FactionTheatres, TableId::FactionAdvisors];

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    icons: usize,
    names: usize,
    countries: usize,
    regions: usize,
    hidden: usize,
    advisors: usize,
    advisor_bytes: usize,
}

collector_primitives::capacity_max!(Capacities {
    rows,
    icons,
    names,
    countries,
    regions,
    hidden,
    advisors,
    advisor_bytes
});

impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let demand = native::count(memory, base, state);
        if let Outcome::Unavailable { reason } = demand.outcome {
            return Err(reason);
        }
        self.raise_for_theatres(demand);
        Ok(())
    }
    pub fn raise_for_theatres(&mut self, demand: native::Walk) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows, rows(demand.required_rows, 1));
        raise(&mut self.icons, rows(demand.required_icons, 1));
        raise(&mut self.names, rows(demand.required_names, 1));
        raise(&mut self.countries, rows(demand.required_countries, 1));
        raise(&mut self.regions, rows(demand.required_regions, 1));
        raise(&mut self.hidden, rows(demand.required_hidden, 1));
    }
    pub fn raise_for_faction_advisors(&mut self, advisors: usize, bytes: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.advisors, rows(advisors, 1));
        raise(&mut self.advisor_bytes, rows(bytes, 1));
    }
}

pub struct Captured {
    rows: CaptureRows<native::Row>,
    icons: Arena<u8>,
    names: Arena<u8>,
    countries: Arena<u32>,
    regions: Arena<u32>,
    hidden: Arena<u32>,
    required_rows: usize,
    advisors: CaptureRows<cfaction::AdvisorRow>,
    advisor_bytes: Arena<u8>,
}

impl Captured {
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.rows
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
        self.advisors
            .fail(super::failed_capture(TABLES[1], hour, force_full, reason));
    }
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities::default())
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            icons: Arena::new(capacity.icons),
            names: Arena::new(capacity.names),
            countries: Arena::new(capacity.countries),
            regions: Arena::new(capacity.regions),
            hidden: Arena::new(capacity.hidden),
            required_rows: 0,
            advisors: CaptureRows::new(capacity.advisors),
            advisor_bytes: Arena::new(capacity.advisor_bytes),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.icons.reset();
        self.names.reset();
        self.countries.reset();
        self.regions.reset();
        self.hidden.reset();
        self.required_rows = 0;
        self.advisors.reset();
        self.advisor_bytes.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.icons.allocated_bytes()
            + self.names.allocated_bytes()
            + self.countries.allocated_bytes()
            + self.regions.allocated_bytes()
            + self.hidden.allocated_bytes()
            + self.advisors.allocated_bytes()
            + self.advisor_bytes.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        grow_capacity(&mut capacity.rows, self.required_rows);
        self.rows.grow(&mut capacity.rows);
        self.icons.grow(&mut capacity.icons);
        self.names.grow(&mut capacity.names);
        self.countries.grow(&mut capacity.countries);
        self.regions.grow(&mut capacity.regions);
        self.hidden.grow(&mut capacity.hidden);
        self.advisors.grow(&mut capacity.advisors);
        self.advisor_bytes.grow(&mut capacity.advisor_bytes);
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
        if should_walk(TABLES[0], hour, force_full) {
            let _reads = crate::read_accounting::table(TABLES[0]);
            let started = qpc();
            let walk = native::walk(
                memory,
                base,
                game_state,
                native::Storage {
                    rows: &mut self.rows.rows,
                    icons: &mut self.icons.first,
                    names: &mut self.names.first,
                    countries: &mut self.countries.first,
                    regions: &mut self.regions.first,
                    hidden: &mut self.hidden.first,
                },
            );
            self.rows.outcome = walk.outcome;
            self.icons.len = walk.icons_len;
            self.icons.required = walk.required_icons;
            self.names.len = walk.names_len;
            self.names.required = walk.required_names;
            self.countries.len = walk.countries_len;
            self.countries.required = walk.required_countries;
            self.regions.len = walk.regions_len;
            self.regions.required = walk.required_regions;
            self.hidden.len = walk.hidden_len;
            self.hidden.required = walk.required_hidden;
            self.required_rows = walk.required_rows;
            ticks[0] = qpc().saturating_sub(started).max(0) as u64;
        } else {
            self.rows.outcome = Outcome::Unavailable {
                reason: SKIPPED_REASON,
            };
        }
        if should_walk(TABLES[1], hour, force_full) {
            let _reads = crate::read_accounting::table(TABLES[1]);
            let started = qpc();
            let walk = cfaction::walk_advisors(
                memory,
                base,
                game_state,
                &mut self.advisors.rows,
                &mut self.advisor_bytes.first,
            );
            self.advisors.outcome = walk.outcome;
            self.advisor_bytes.len = walk.bytes_len;
            self.advisor_bytes.required = walk.required_bytes;
            ticks[1] = qpc().saturating_sub(started).max(0) as u64;
        } else {
            self.advisors.outcome = Outcome::Unavailable {
                reason: SKIPPED_REASON,
            };
        }
        ticks
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome, self.advisors.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        [self.theatre_table(), self.advisor_table()]
    }
    fn advisor_table(&self) -> TableView {
        let Some(bytes) = self.advisor_bytes.first.get(..self.advisor_bytes.len) else {
            return unavailable("faction advisor arena length invalid");
        };
        let token = |row: &cfaction::AdvisorRow| {
            row.token_start
                .checked_add(row.token_len)
                .and_then(|end| bytes.get(row.token_start..end))
        };
        let Some(rows) = self.advisors.rows.get(..self.advisors.outcome.len()) else {
            return unavailable("faction advisor row count invalid");
        };
        if rows.iter().any(|row| token(row).is_none()) {
            return unavailable("faction advisor token outside captured arena");
        }
        table::<tables::FactionAdvisors, _>(self.advisors.outcome, rows, |row| FactionAdvisorRow {
            faction: row.faction,
            ordinal: row.ordinal,
            token: owned_vec(copy_vec(token(row).unwrap())),
        })
    }
    fn theatre_table(&self) -> TableView {
        let (Some(icons), Some(names), Some(countries), Some(regions), Some(hidden)) = (
            self.icons.first.get(..self.icons.len),
            self.names.first.get(..self.names.len),
            self.countries.first.get(..self.countries.len),
            self.regions.first.get(..self.regions.len),
            self.hidden.first.get(..self.hidden.len),
        ) else {
            return unavailable("faction theatre arena length invalid");
        };
        let Some(rows) = self.rows.rows.get(..self.rows.outcome.len()) else {
            return unavailable("faction theatre row count invalid");
        };
        fn span<T>(arena: &[T], start: usize, len: usize) -> Option<&[T]> {
            arena.get(start..start.checked_add(len)?)
        }
        let spans = |row: &native::Row| {
            Some((
                span(icons, row.icon_start, row.icon_len)?,
                span(names, row.name_start, row.name_len)?,
                span(countries, row.countries_start, row.countries_len)?,
                span(regions, row.regions_start, row.regions_len)?,
                span(hidden, row.hidden_start, row.hidden_len)?,
            ))
        };
        if rows.iter().any(|row| spans(row).is_none()) {
            return unavailable("faction theatre outside captured arena");
        }
        table::<tables::FactionTheatres, _>(self.rows.outcome, rows, |row| {
            let (icon, name, countries, regions, hidden) = spans(row).unwrap();
            FactionTheatreRow {
                faction: row.faction,
                ordinal: row.ordinal,
                icon: owned_vec(copy_vec(icon)),
                regions: owned_vec(copy_vec(regions)),
                country_count: row.country_count,
                hidden_for: owned_vec(copy_vec(hidden)),
                name: owned_vec(copy_vec(name)).into(),
                owner: row.owner.into(),
                commander: (row.commander != 0).then_some(row.commander).into(),
                countries: owned_vec(copy_vec(countries)).into(),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TableRows;

    #[test]
    fn growth_follows_the_walk_demand_for_rows_and_every_arena() {
        let mut capacity = Capacities {
            rows: 1,
            icons: 1,
            names: 1,
            countries: 1,
            regions: 1,
            hidden: 1,
            advisors: 1,
            advisor_bytes: 1,
        };
        let mut captured = Captured::with_capacities(&capacity);
        captured.required_rows = 13;
        captured.icons.required = 300;
        captured.names.required = 200;
        captured.countries.required = 40;
        captured.regions.required = 70;
        captured.hidden.required = 5;
        captured.grow(&mut capacity);
        assert!(capacity.rows >= 13 && captured.rows.rows.len() >= 13);
        assert!(capacity.icons >= 300 && captured.icons.first.len() >= 300);
        assert!(capacity.names >= 200 && captured.names.first.len() >= 200);
        assert!(capacity.countries >= 40 && captured.countries.first.len() >= 40);
        assert!(capacity.regions >= 70 && captured.regions.first.len() >= 70);
        assert!(capacity.hidden >= 5 && captured.hidden.first.len() >= 5);
        captured.advisors.outcome = Outcome::Truncated {
            len: 0,
            unread_rows: 9,
        };
        captured.advisor_bytes.required = 400;
        captured.grow(&mut capacity);
        assert!(capacity.advisors >= 9 && captured.advisors.rows.len() >= 9);
        assert!(capacity.advisor_bytes >= 400 && captured.advisor_bytes.first.len() >= 400);
        let raised = capacity;
        capacity.raise_for_faction_advisors(3, 30);
        assert!(capacity == raised);
        capacity.raise_for_faction_advisors(3_000, 90_000);
        assert!(capacity.advisors >= 4_000 && capacity.advisor_bytes >= 120_000);
    }

    #[test]
    fn faction_advisors_convert_their_tokens_and_refuse_escapes() {
        let mut captured = Captured::with_capacities(&Capacities {
            advisors: 1,
            advisor_bytes: 12,
            ..Default::default()
        });
        captured.advisor_bytes.first[..12].copy_from_slice(b"GER_theorist");
        captured.advisor_bytes.len = 12;
        captured.advisors.rows[0] = cfaction::AdvisorRow {
            faction: (1 << 32) | 88,
            ordinal: 2,
            token_start: 0,
            token_len: 12,
        };
        captured.advisors.outcome = Outcome::Rows { len: 1 };
        let [_, TableView::Complete(TableRows::FactionAdvisors(rows))] = captured.tables() else {
            panic!("expected complete faction advisor rows");
        };
        assert_eq!(
            rows,
            vec![FactionAdvisorRow {
                faction: (1 << 32) | 88,
                ordinal: 2,
                token: b"GER_theorist".to_vec(),
            }]
        );
        captured.advisors.rows[0].token_len = 13;
        assert!(matches!(
            captured.tables()[1],
            TableView::Unavailable { .. }
        ));
        captured.fail_capture(1, false, "job failed");
        assert_eq!(
            captured.outcomes(),
            [Outcome::Unavailable {
                reason: "job failed"
            }; 2]
        );
    }

    #[test]
    fn rows_convert_their_spans_and_escapes_are_refused() {
        let mut captured = Captured::with_capacities(&Capacities {
            rows: 2,
            icons: 3,
            names: 5,
            countries: 2,
            regions: 2,
            hidden: 1,
            ..Default::default()
        });
        captured.rows.rows[0] = native::Row {
            faction: (4 << 32) | 88,
            ordinal: 2,
            icon_len: 3,
            name_len: 5,
            owner: 11,
            commander: (844 << 32) | 4713,
            countries_len: 2,
            regions_len: 2,
            country_count: 2,
            hidden_len: 1,
            ..Default::default()
        };
        captured.rows.rows[1] = native::Row {
            faction: (4 << 32) | 88,
            ordinal: 3,
            name_start: 5,
            owner: 0x8000_0002,
            countries_start: 2,
            regions_start: 2,
            hidden_start: 1,
            ..Default::default()
        };
        captured.icons.first.copy_from_slice(b"GFX");
        captured.icons.len = 3;
        captured.names.first.copy_from_slice(b"North");
        captured.names.len = 5;
        captured.countries.first.copy_from_slice(&[11, 12]);
        captured.countries.len = 2;
        captured.regions.first.copy_from_slice(&[7, u32::MAX]);
        captured.regions.len = 2;
        captured.hidden.first[0] = 9;
        captured.hidden.len = 1;
        captured.rows.outcome = Outcome::Rows { len: 2 };
        let [TableView::Complete(TableRows::FactionTheatres(rows)), _] = captured.tables() else {
            panic!("expected complete faction theatre rows");
        };
        assert_eq!(
            rows,
            vec![
                FactionTheatreRow {
                    faction: (4 << 32) | 88,
                    ordinal: 2,
                    icon: b"GFX".to_vec(),
                    regions: vec![7, u32::MAX],
                    country_count: 2,
                    hidden_for: vec![9],
                    name: b"North".to_vec().into(),
                    owner: replay_data::rows::Recorded::Value(11),
                    commander: Some((844 << 32) | 4713).into(),
                    countries: vec![11, 12].into(),
                },
                FactionTheatreRow {
                    faction: (4 << 32) | 88,
                    ordinal: 3,
                    owner: replay_data::rows::Recorded::Value(0x8000_0002),
                    commander: None.into(),
                    ..Default::default()
                },
            ]
        );
        for escape in [
            (|row: &mut native::Row| row.regions_len = 3) as fn(&mut native::Row),
            |row| row.name_len = 6,
            |row| row.countries_start = 1,
        ] {
            let saved = captured.rows.rows[0];
            escape(&mut captured.rows.rows[0]);
            assert!(matches!(
                captured.tables()[0],
                TableView::Unavailable { .. }
            ));
            captured.rows.rows[0] = saved;
        }
    }
}

#[cfg(test)]
mod l3b_bench {
    use super::*;
    use collector_extended_support::bench_world::{self, Shape};

    #[test]
    #[ignore = "timing bench"]
    fn l3b_bench_hourly_faction_theatres() {
        for countries in [1_500, 5_000] {
            let world = bench_world::build(
                countries,
                Shape {
                    named: true,
                    theatre_links: true,
                },
            );
            let rows = world.faction_theatres as usize;
            let mut captured = Captured::with_capacities(&Capacities {
                rows,
                icons: rows * 16,
                names: rows * 16,
                countries: rows * 8,
                regions: rows * 8,
                hidden: rows * 4,
                ..Capacities::default()
            });
            let ((), counts) = world.counted(|memory| {
                captured.capture(memory, world.base, world.game_state, 1, false);
            });
            println!(
                "faction-work countries={countries}: FactionTheatres {}",
                bench_world::work(bench_world::total(&counts, &TABLES))
            );
            for source in ["slice", "live"] {
                let (median, p95) = if source == "slice" {
                    let memory = world.slice();
                    bench_world::time(50, || {
                        captured.capture(&memory, world.base, world.game_state, 1, false);
                    })
                } else {
                    let memory = world.live();
                    bench_world::time(50, || {
                        captured.capture(&memory, world.base, world.game_state, 1, false);
                    })
                };
                assert_eq!(captured.outcomes()[0], Outcome::Rows { len: rows });
                println!(
                    "faction theatres countries={countries} factions={} rows={rows} source={source}: capture {median:.1} us (p95 {p95:.1})",
                    rows / 3
                );
            }
        }
    }
}
