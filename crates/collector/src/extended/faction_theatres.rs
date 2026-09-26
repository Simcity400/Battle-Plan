//! Faction theatres for the political map's faction-theatre pins: tens of rows
//! an hour, sized from the live theatre demand and grown off-thread.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, grow_capacity},
};
use engine::{Memory, cfaction_theater_manager as native, walk::Outcome};
use replay::{
    cadence::{SKIPPED_REASON, should_walk},
    frame::TableView,
    rows::{FactionTheatreRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::FactionTheatres];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

/// Starting sizes only: any short buffer truncates explicitly and the spool
/// thread grows it to the walk's reported demand.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    icons: usize,
    regions: usize,
    hidden: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 64,
            icons: 2048,
            regions: 1024,
            hidden: 256,
        }
    }
}

pub struct Captured {
    rows: CaptureRows<native::Row>,
    icons: Arena<u8>,
    regions: Arena<u32>,
    hidden: Arena<u32>,
    required_rows: usize,
}

impl Captured {
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.rows
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
    }
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            icons: 0,
            regions: 0,
            hidden: 0,
        })
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            icons: Arena::new(capacity.icons),
            regions: Arena::new(capacity.regions),
            hidden: Arena::new(capacity.hidden),
            required_rows: 0,
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.icons.reset();
        self.regions.reset();
        self.hidden.reset();
        self.required_rows = 0;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.icons.allocated_bytes()
            + self.regions.allocated_bytes()
            + self.hidden.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        grow_capacity(&mut capacity.rows, self.required_rows);
        self.rows.grow(&mut capacity.rows);
        self.icons.grow(&mut capacity.icons);
        self.regions.grow(&mut capacity.regions);
        self.hidden.grow(&mut capacity.hidden);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        self.reset();
        if !should_walk(TABLES[0], hour, force_full) {
            self.rows.outcome = Outcome::Unavailable {
                reason: SKIPPED_REASON,
            };
            return [0];
        }
        let started = qpc();
        let walk = native::walk(
            memory,
            base,
            game_state,
            &mut self.rows.rows,
            &mut self.icons.first,
            &mut self.regions.first,
            &mut self.hidden.first,
        );
        self.rows.outcome = walk.outcome;
        self.icons.len = walk.icons_len;
        self.icons.required = walk.required_icons;
        self.regions.len = walk.regions_len;
        self.regions.required = walk.required_regions;
        self.hidden.len = walk.hidden_len;
        self.hidden.required = walk.required_hidden;
        self.required_rows = walk.required_rows;
        [qpc().saturating_sub(started).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        let (Some(icons), Some(regions), Some(hidden)) = (
            self.icons.first.get(..self.icons.len),
            self.regions.first.get(..self.regions.len),
            self.hidden.first.get(..self.hidden.len),
        ) else {
            return [unavailable("faction theatre arena length invalid")];
        };
        let Some(rows) = self.rows.rows.get(..self.rows.outcome.len()) else {
            return [unavailable("faction theatre row count invalid")];
        };
        fn span<T>(arena: &[T], start: usize, len: usize) -> Option<&[T]> {
            arena.get(start..start.checked_add(len)?)
        }
        let spans = |row: &native::Row| {
            Some((
                span(icons, row.icon_start, row.icon_len)?,
                span(regions, row.regions_start, row.regions_len)?,
                span(hidden, row.hidden_start, row.hidden_len)?,
            ))
        };
        if rows.iter().any(|row| spans(row).is_none()) {
            return [unavailable("faction theatre outside captured arena")];
        }
        [table::<tables::FactionTheatres, _>(
            self.rows.outcome,
            rows,
            |row| {
                let (icon, regions, hidden) = spans(row).unwrap();
                FactionTheatreRow {
                    faction: row.faction,
                    ordinal: row.ordinal,
                    icon: icon.to_vec(),
                    regions: regions.to_vec(),
                    country_count: row.country_count,
                    hidden_for: hidden.to_vec(),
                }
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::TableRows;

    #[test]
    fn growth_follows_the_walk_demand_for_rows_and_every_arena() {
        let mut capacity = Capacities {
            rows: 1,
            icons: 1,
            regions: 1,
            hidden: 1,
        };
        let mut captured = Captured::with_capacities(&capacity);
        captured.required_rows = 13;
        captured.icons.required = 300;
        captured.regions.required = 70;
        captured.hidden.required = 5;
        captured.grow(&mut capacity);
        assert!(capacity.rows >= 13 && captured.rows.rows.len() >= 13);
        assert!(capacity.icons >= 300 && captured.icons.first.len() >= 300);
        assert!(capacity.regions >= 70 && captured.regions.first.len() >= 70);
        assert!(capacity.hidden >= 5 && captured.hidden.first.len() >= 5);
    }

    #[test]
    fn rows_convert_their_spans_and_escapes_are_refused() {
        let mut captured = Captured::with_capacities(&Capacities {
            rows: 1,
            icons: 3,
            regions: 2,
            hidden: 1,
        });
        captured.rows.rows[0] = native::Row {
            faction: (4 << 32) | 88,
            ordinal: 2,
            icon_len: 3,
            regions_len: 2,
            country_count: 4,
            hidden_len: 1,
            ..Default::default()
        };
        captured.icons.first.copy_from_slice(b"GFX");
        captured.icons.len = 3;
        captured.regions.first.copy_from_slice(&[7, u32::MAX]);
        captured.regions.len = 2;
        captured.hidden.first[0] = 9;
        captured.hidden.len = 1;
        captured.rows.outcome = Outcome::Rows { len: 1 };
        let [TableView::Complete(TableRows::FactionTheatres(rows))] = captured.tables() else {
            panic!("expected complete faction theatre rows");
        };
        assert_eq!(
            rows,
            vec![FactionTheatreRow {
                faction: (4 << 32) | 88,
                ordinal: 2,
                icon: b"GFX".to_vec(),
                regions: vec![7, u32::MAX],
                country_count: 4,
                hidden_for: vec![9],
            }]
        );
        captured.rows.rows[0].regions_len = 3;
        assert!(matches!(
            captured.tables()[0],
            TableView::Unavailable { .. }
        ));
    }
}

/// Ignored capture-cost bench; see `bench_world` for the assumed scale.
#[cfg(test)]
mod l3b_bench {
    use super::*;
    use crate::extended::bench_world::{self, Shape};

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
                regions: rows * 8,
                hidden: rows * 4,
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
                assert_eq!(captured.outcomes(), [Outcome::Rows { len: rows }]);
                println!(
                    "faction theatres countries={countries} factions={} rows={rows} source={source}: capture {median:.1} us (p95 {p95:.1})",
                    rows / 3
                );
            }
        }
    }
}
