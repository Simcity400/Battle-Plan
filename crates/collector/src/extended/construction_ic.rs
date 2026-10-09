//! ConstructionLineOutput: the civilian-factory output each construction line
//! consumed, totalled per recording branch from the daily progress the
//! `construction_ic` hooks observe.
//!
//! Nothing is read from the game for it. Every clocked hour the callback hands the
//! hook log over into this pack's preallocated entries ([`Captured::drain`]); the
//! spool adds them to the branch's [`Totals`] and publishes the table with that
//! hour's BuildingConstruction and RailwayConstruction census, which decides which
//! lines ended and which were created after the observation began. The table shares
//! the census's daily slot: off it, the spool carries the last publication, until a
//! day's output goes unobserved.
use super::output_continuity::OutputMap;
pub(crate) use super::output_continuity::holds_publication;
use super::output_continuity::{self, Progress, Reasons, State};
use crate::construction_ic::{Drained, Entry};
use crate::spooling::unavailable;
use engine::allocation::{collect_vec, mul, owned_vec, reserve_map};
use replay::{
    BranchId,
    frame::{TableInput, TableView},
    rows::{
        BuildingConstructionRow, ConstructionCompletion, ConstructionLineOutputRow,
        RailwayConstructionRow, Recorded, TableId, TableRows,
    },
};
use std::io;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ConstructionLineOutput];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

/// The spool replaces this placeholder with the published table.
pub(crate) const ASSEMBLED_BY_SPOOL: &str =
    "assembled by the spool from the construction output the hooks observed";

pub use super::output_continuity::Capacities;
pub struct Observation;
impl output_continuity::Observation for Observation {
    type Entry = Entry;
    const ASSEMBLED_BY_SPOOL: &'static str = ASSEMBLED_BY_SPOOL;
    fn drain(entries: &mut [Entry]) -> Result<Drained, &'static str> {
        crate::construction_ic::drain(entries)
    }
}
pub type Captured = output_continuity::Captured<Observation>;

/// One line's output on the branch.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Line {
    /// The first hour whose drain or census showed the line.
    first_hour: i32,
    railway: bool,
    construction: i64,
    bank: i64,
    carry: i64,
    repair: i64,
    instant: i64,
    instant_seen: bool,
    ticks: i32,
    repair_ticks: i32,
    linked: bool,
    /// Its factories contributed to a partner whose combined output is recorded.
    unknown_share: bool,
    /// Published as ended; dropped at the next written publication.
    ended: bool,
    completion: Option<ConstructionCompletion>,
}

/// The branch's per-line totals, owned by the spool for one campaign.
#[derive(Debug, Default)]
pub struct Totals {
    shared: output_continuity::Totals<Line>,
}

/// A census table this hour: its rows, skipped (the slot is not read), or why not.
enum Census<'a, T> {
    Rows(&'a [T]),
    Skipped,
    Missing(&'static str),
}

fn census<'a, T>(
    input: &'a TableInput,
    rows: impl Fn(&'a TableRows) -> Option<&'a [T]>,
    missing: &'static str,
) -> Census<'a, T> {
    match input {
        TableInput::Sampled(TableView::Complete(table)) => {
            rows(table).map_or(Census::Missing(missing), Census::Rows)
        }
        TableInput::SharedComplete(table) => {
            rows(table).map_or(Census::Missing(missing), Census::Rows)
        }
        TableInput::Sampled(TableView::Unavailable { reason })
            if reason == replay::cadence::SKIPPED_REASON =>
        {
            Census::Skipped
        }
        _ => Census::Missing(missing),
    }
}

impl Totals {
    /// Account this frame's drain and publish ConstructionLineOutput into
    /// `tables`, beside the frame's construction queues. Runs before the
    /// staggered carry, so an off-slot hour's skip is carried like any daily table's.
    pub(crate) fn assemble(
        &mut self,
        branch: BranchId,
        hour: i32,
        captured: &Captured,
        tables: &mut [TableInput],
    ) {
        if self.observe(branch, hour, captured).is_err() {
            return;
        }
        let published = self.publish(
            hour,
            &tables[TableId::BuildingConstruction.index()],
            &tables[TableId::RailwayConstruction.index()],
        );
        tables[TableId::ConstructionLineOutput.index()] = TableInput::Sampled(published);
    }

    /// After a frame of this campaign was written, with whether it held the
    /// last publication ([`holds_publication`], taken before the write).
    pub(crate) fn written(&mut self, held_publication: bool) {
        self.shared.written(held_publication);
    }

    fn observe(&mut self, branch: BranchId, hour: i32, captured: &Captured) -> io::Result<()> {
        let progress = self.shared.observe(
            branch,
            hour,
            captured.drained,
            &Reasons {
                first: "the branch's first frame drained no output",
                restarted: "the construction line observation restarted",
                lost: "construction output was lost",
                absent: "a frame drained no output",
            },
        );
        match progress {
            Progress::Restart => {}
            Progress::Entries(range) => {
                reserve_map(&mut self.shared.lines, mul(range.len(), 2)?)?;
                for entry in &captured.handed_over()[range] {
                    if let Err(reason) = add(&mut self.shared.lines, entry, hour) {
                        self.shared.state = State::Withheld(reason);
                        break;
                    }
                }
            }
            Progress::None => {}
        }
        Ok(())
    }

    fn publish(&mut self, hour: i32, buildings: &TableInput, railways: &TableInput) -> TableView {
        let since_hour = match &self.shared.state {
            State::Observing { since_hour, .. } => *since_hour,
            State::Withheld(reason) => return unavailable(reason),
        };
        let buildings = census(
            buildings,
            |table| match table {
                TableRows::BuildingConstruction(rows) => Some(rows),
                _ => None,
            },
            "BuildingConstruction is incomplete at this hour",
        );
        let railways = census(
            railways,
            |table| match table {
                TableRows::RailwayConstruction(rows) => Some(rows),
                _ => None,
            },
            "RailwayConstruction is incomplete at this hour",
        );
        let (buildings, railways) = match (buildings, railways) {
            (Census::Rows(buildings), Census::Rows(railways)) => (buildings, railways),
            (Census::Skipped, Census::Skipped) => {
                return unavailable(replay::cadence::SKIPPED_REASON);
            }
            (Census::Missing(reason), _) | (_, Census::Missing(reason)) => {
                return unavailable(reason);
            }
            _ => return unavailable("the construction queues were not read together"),
        };
        if !self.shared.complete {
            return unavailable(
                "construction output before this hour's sample is still in the log",
            );
        }
        TableView::Complete(TableRows::ConstructionLineOutput(owned_vec(
            self.rows(hour, since_hour, buildings, railways),
        )))
    }

    fn rows(
        &mut self,
        hour: i32,
        since_hour: i32,
        buildings: &[BuildingConstructionRow],
        railways: &[RailwayConstructionRow],
    ) -> io::Result<Vec<ConstructionLineOutputRow>> {
        let mut live: OutputMap<bool> = OutputMap::default();
        reserve_map(
            &mut live,
            engine::allocation::add(buildings.len(), railways.len())?,
        )?;
        for row in buildings {
            live.insert(row.id, false);
        }
        for row in railways {
            live.insert(row.id, true);
        }
        self.shared
            .prepare_publication(hour, live.keys().copied(), |line| line.ended)?;
        reserve_map(&mut self.shared.lines, live.len())?;
        let baseline = &self.shared.census;
        // Every live line has a row; linked partners retain an unknown share.
        for (&handle, &railway) in &live {
            self.shared.lines.entry(handle).or_insert_with(|| Line {
                first_hour: hour,
                railway,
                ..Line::default()
            });
        }
        let mut rows = collect_vec(self.shared.lines.iter_mut().map(|(&handle, line)| {
            line.ended = !live.contains_key(&handle);
            // Absent from the first census and first seen after it: created
            // after that census, so every day of its life was observed.
            let observed_since_creation = baseline.observed_since_creation(handle, line.first_hour);
            ConstructionLineOutputRow {
                line: handle,
                railway: line.railway,
                construction_raw: line.construction,
                bank_raw: line.bank,
                carry_raw: line.carry,
                exact: !line.instant_seen && !line.unknown_share,
                repair_raw: line.repair,
                instant_raw: line.instant,
                ticks: line.ticks,
                repair_ticks: line.repair_ticks,
                since_hour,
                observed_since_creation,
                linked: line.linked,
                ended: line.ended,
                completion: Recorded::Value(line.completion),
            }
        }))?;
        rows.sort_unstable_by_key(|row| row.line);
        Ok(rows)
    }
}

fn add(lines: &mut OutputMap<Line>, entry: &Entry, hour: i32) -> Result<(), &'static str> {
    let line = lines.entry(entry.line).or_insert_with(|| Line {
        first_hour: hour,
        railway: entry.railway,
        ..Line::default()
    });
    if line.railway != entry.railway {
        return Err("construction output line changed kind");
    }
    if let Some(completion) = entry.completion {
        if completion.hour > hour
            || (completion.cancelled && !entry.railway)
            || line
                .completion
                .is_some_and(|previous| previous != completion)
        {
            return Err("construction completion outcome inconsistent");
        }
        line.completion = Some(completion);
        line.linked |= entry.linked_completion;
        line.unknown_share |= entry.linked_completion;
        return Ok(());
    }
    if line.completion.is_some() {
        return Err("construction output after native completion");
    }
    let sum = |a: i64, b: i64| a.checked_add(b).ok_or("construction output total overflow");
    if entry.instant {
        line.instant = sum(line.instant, entry.amount)?;
        line.instant_seen = true;
    } else if entry.carry {
        line.carry = sum(line.carry, entry.amount)?;
    } else {
        let total = if entry.repair {
            &mut line.repair
        } else {
            &mut line.construction
        };
        // Inverse of the native speed+bank qword ADD at 140F59453.
        // Restore the original signed factory operand even if it wrapped.
        *total = sum(*total, entry.amount.wrapping_sub(entry.bank))?;
        line.bank = sum(line.bank, entry.bank)?;
    }
    line.ticks = line
        .ticks
        .checked_add(i32::from(!entry.carry))
        .ok_or("construction output tick overflow")?;
    line.repair_ticks = line
        .repair_ticks
        .checked_add(i32::from(entry.repair && !entry.carry))
        .ok_or("construction output repair tick overflow")?;
    line.linked |= entry.partner.is_some();
    if let Some(partner) = entry.partner {
        let secondary = lines.entry(partner).or_insert_with(|| Line {
            first_hour: hour,
            ..Line::default()
        });
        if secondary.railway {
            return Err("construction output partner is a railway");
        }
        secondary.linked = true;
        secondary.unknown_share = true;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::frame::TABLE_COUNT;

    const LINE_A: u64 = 0x10_0000_003b;
    const LINE_B: u64 = 0x11_0000_003b;
    const RAIL: u64 = 0x12_0000_1269;

    #[test]
    fn native_completion_survives_deletion_without_spending_or_tick_changes() {
        let mut lines = OutputMap::default();
        add(&mut lines, &entry(LINE_A, 35), 100).unwrap();
        let outcome = ConstructionCompletion {
            hour: 120,
            cancelled: false,
        };
        add(
            &mut lines,
            &Entry {
                line: LINE_A,
                completion: Some(outcome),
                ..Entry::default()
            },
            124,
        )
        .unwrap();
        let line = &lines[&LINE_A];
        assert_eq!(
            (line.construction, line.ticks, line.repair_ticks),
            (35, 1, 0)
        );
        let mut totals = Totals::default();
        totals.shared.lines = lines;
        let rows = totals.rows(124, 100, &[], &[]).unwrap();
        assert_eq!(rows[0].completion, Recorded::Value(Some(outcome)));
        assert!(rows[0].ended);
        assert_eq!(rows[0].construction_raw, 35);
    }

    #[test]
    fn linked_partner_completion_retains_its_own_outcome_and_unknown_output_share() {
        let mut lines = OutputMap::default();
        add(
            &mut lines,
            &Entry {
                partner: Some(LINE_B),
                ..entry(LINE_A, 35)
            },
            100,
        )
        .unwrap();
        let outcome = ConstructionCompletion {
            hour: 120,
            cancelled: false,
        };
        // Partner removed inside AdvanceProduction; its daily getter is never
        // reached. The primary's ordinary getter completes separately.
        for line in [LINE_B, LINE_A] {
            add(
                &mut lines,
                &Entry {
                    line,
                    completion: Some(outcome),
                    linked_completion: line == LINE_B,
                    ..Entry::default()
                },
                124,
            )
            .unwrap();
        }
        let mut totals = Totals::default();
        totals.shared.lines = lines;
        let rows = totals.rows(124, 100, &[], &[]).unwrap();
        assert!(rows.iter().all(|row| row.linked
            && row.ended
            && row.completion == Recorded::Value(Some(outcome))));
        let partner = rows.iter().find(|row| row.line == LINE_B).unwrap();
        assert!(!partner.exact);
        assert_eq!((partner.construction_raw, partner.ticks), (0, 0));
        // Attaching after the primary's spend must not make its completed
        // partner's output look like an exact unlinked zero.
        let mut after_spend = Totals::default();
        add(
            &mut after_spend.shared.lines,
            &Entry {
                line: LINE_B,
                completion: Some(outcome),
                linked_completion: true,
                ..Entry::default()
            },
            124,
        )
        .unwrap();
        let row = after_spend.rows(124, 120, &[], &[]).unwrap().remove(0);
        assert!(row.linked && !row.exact && row.completion == Recorded::Value(Some(outcome)));
    }

    #[test]
    fn signed_factory_output_and_wrapped_bank_add_keep_separate_channels() {
        let mut lines = OutputMap::default();
        add(&mut lines, &entry(LINE_A, 50), 100).unwrap();
        add(&mut lines, &entry(LINE_A, -25), 124).unwrap();
        add(
            &mut lines,
            &Entry {
                repair: true,
                ..entry(LINE_A, -10)
            },
            148,
        )
        .unwrap();
        let line = &lines[&LINE_A];
        assert_eq!(
            (
                line.construction,
                line.repair,
                line.bank,
                line.ticks,
                line.repair_ticks
            ),
            (25, -10, 0, 3, 1)
        );
        add(
            &mut lines,
            &Entry {
                bank: 1,
                ..entry(LINE_B, i64::MIN)
            },
            124,
        )
        .unwrap();
        let line = &lines[&LINE_B];
        assert_eq!((line.construction, line.bank, line.ticks), (i64::MAX, 1, 1));
    }

    fn census_of(buildings: &[u64], railways: &[u64]) -> Vec<TableInput> {
        let mut tables: Vec<TableInput> = (0..TABLE_COUNT)
            .map(|_| {
                TableInput::Sampled(TableView::Unavailable {
                    reason: "fixture".into(),
                })
            })
            .collect();
        tables[TableId::BuildingConstruction.index()] =
            TableInput::Sampled(TableView::Complete(TableRows::BuildingConstruction(
                buildings
                    .iter()
                    .map(|&id| BuildingConstructionRow {
                        id,
                        country: 5001,
                        ..Default::default()
                    })
                    .collect(),
            )));
        tables[TableId::RailwayConstruction.index()] =
            TableInput::Sampled(TableView::Complete(TableRows::RailwayConstruction(
                railways
                    .iter()
                    .map(|&id| RailwayConstructionRow {
                        id,
                        country: 5001,
                        ..Default::default()
                    })
                    .collect(),
            )));
        tables
    }

    fn skipped() -> Vec<TableInput> {
        let mut tables = census_of(&[], &[]);
        for table in [TableId::BuildingConstruction, TableId::RailwayConstruction] {
            tables[table.index()] = TableInput::Sampled(TableView::Unavailable {
                reason: replay::cadence::SKIPPED_REASON.into(),
            });
        }
        tables
    }

    fn entry(line: u64, amount: i64) -> Entry {
        Entry {
            line,
            amount,
            ..Entry::default()
        }
    }

    fn captured(first: u64, entries: &[Entry]) -> Captured {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.set_drained_for_test(
            Ok(Drained {
                epoch: 1,
                first,
                cursor: first + entries.len() as u64,
                copied: entries.len(),
                pending: entries.len(),
            }),
            entries,
        );
        captured
    }

    fn published(
        totals: &mut Totals,
        hour: i32,
        captured: &Captured,
        mut tables: Vec<TableInput>,
    ) -> TableView {
        totals.assemble(1, hour, captured, &mut tables);
        match std::mem::replace(
            &mut tables[TableId::ConstructionLineOutput.index()],
            TableInput::Sampled(TableView::Unavailable {
                reason: String::new(),
            }),
        ) {
            TableInput::Sampled(view) => view,
            _ => unreachable!(),
        }
    }

    fn rows(view: TableView) -> Vec<ConstructionLineOutputRow> {
        match view {
            TableView::Complete(TableRows::ConstructionLineOutput(rows)) => rows,
            other => panic!("{other:?}"),
        }
    }

    /// Output splits into construction, repair and instant; the bank draw is kept
    /// apart; a line created after the first census covers its whole life; an
    /// ended line is published once more, then dropped.
    #[test]
    fn totals_split_by_kind_and_publish_with_the_census() {
        let mut totals = Totals::default();
        // The branch's first frame: its drained output came before it.
        published(
            &mut totals,
            0,
            &captured(0, &[entry(LINE_A, 99)]),
            census_of(&[LINE_A], &[]),
        );
        let day = [
            Entry {
                bank: 5,
                ..entry(LINE_A, 100)
            },
            Entry {
                repair: true,
                bank: 2,
                ..entry(LINE_A, 40)
            },
            Entry {
                railway: true,
                ..entry(RAIL, 180)
            },
            Entry {
                instant: true,
                ..entry(LINE_B, 1_000)
            },
            Entry {
                partner: Some(LINE_A),
                ..entry(LINE_B, 70)
            },
        ];
        let first = rows(published(
            &mut totals,
            24,
            &captured(1, &day),
            census_of(&[LINE_A, LINE_B], &[RAIL]),
        ));
        let a = &first[0];
        assert_eq!(
            (
                a.line,
                a.construction_raw,
                a.bank_raw,
                a.repair_raw,
                a.ticks
            ),
            (LINE_A, 95, 7, 38, 2)
        );
        assert!(!a.observed_since_creation);
        let b = &first[1];
        assert_eq!(
            (
                b.instant_raw,
                b.construction_raw,
                b.linked,
                b.observed_since_creation
            ),
            (1_000, 70, true, true)
        );
        let rail = &first[2];
        assert_eq!((rail.railway, rail.construction_raw), (true, 180));
        assert!(first.iter().all(|row| row.since_hour == 0 && !row.ended));
        // LINE_A leaves the queue: published as ended until a written frame held it.
        let ended = rows(published(
            &mut totals,
            48,
            &captured(6, &[]),
            census_of(&[LINE_B], &[RAIL]),
        ));
        assert!(ended.iter().find(|row| row.line == LINE_A).unwrap().ended);
        totals.written(true);
        let after = rows(published(
            &mut totals,
            72,
            &captured(6, &[]),
            census_of(&[LINE_B], &[RAIL]),
        ));
        assert!(after.iter().all(|row| row.line != LINE_A));
    }

    #[test]
    fn linked_partner_is_an_unknown_share_even_without_its_own_entry() {
        let mut totals = Totals::default();
        published(
            &mut totals,
            0,
            &captured(0, &[]),
            census_of(&[LINE_A, LINE_B], &[]),
        );
        let spend = Entry {
            partner: Some(LINE_B),
            ..entry(LINE_A, 100)
        };
        let output = rows(published(
            &mut totals,
            24,
            &captured(0, &[spend]),
            census_of(&[LINE_A, LINE_B], &[]),
        ));
        assert_eq!(
            (
                output[0].construction_raw,
                output[0].ticks,
                output[0].linked,
                output[0].exact
            ),
            (100, 1, true, true)
        );
        assert_eq!(
            (
                output[1].construction_raw,
                output[1].ticks,
                output[1].linked,
                output[1].exact
            ),
            (0, 0, true, false)
        );
        totals.written(true);
        let ended = rows(published(
            &mut totals,
            48,
            &captured(1, &[]),
            census_of(&[], &[]),
        ));
        assert!(ended.iter().all(|row| row.ended && row.linked));
        assert!(!ended[1].exact);
    }

    /// Off the census's slot the table is skipped (and carried); a lost drain
    /// withholds it for the rest of the branch; output still in the log leaves
    /// the hour unavailable.
    #[test]
    fn lost_output_withholds_and_an_incomplete_hour_is_unavailable() {
        let mut totals = Totals::default();
        published(&mut totals, 0, &captured(0, &[]), census_of(&[LINE_A], &[]));
        assert!(matches!(
            published(&mut totals, 1, &captured(0, &[]), skipped()),
            TableView::Unavailable { reason } if reason == replay::cadence::SKIPPED_REASON
        ));
        let mut waiting = captured(0, &[entry(LINE_A, 5)]);
        waiting.set_drained_for_test(
            Ok(Drained {
                epoch: 1,
                first: 0,
                cursor: 3,
                copied: 1,
                pending: 3,
            }),
            &[entry(LINE_A, 5)],
        );
        assert!(matches!(
            published(&mut totals, 24, &waiting, census_of(&[LINE_A], &[])),
            TableView::Unavailable { reason } if reason.contains("still in the log")
        ));
        // Entries 1 and 2 are lost: the next drain starts past them.
        let view = published(
            &mut totals,
            48,
            &captured(5, &[]),
            census_of(&[LINE_A], &[]),
        );
        assert!(
            matches!(view, TableView::Unavailable { reason } if reason == "construction output was lost")
        );
        let view = published(
            &mut totals,
            72,
            &captured(5, &[]),
            census_of(&[LINE_A], &[]),
        );
        assert!(matches!(view, TableView::Unavailable { .. }));
    }

    #[test]
    fn every_continuity_failure_withholds_until_a_new_branch() {
        for broken in [
            Some(Ok(Drained {
                epoch: 1,
                first: 1,
                cursor: 1,
                ..Default::default()
            })),
            Some(Ok(Drained {
                epoch: 1,
                first: LOG_OVERFLOW,
                cursor: LOG_OVERFLOW,
                ..Default::default()
            })),
            Some(Ok(Drained {
                epoch: 2,
                ..Default::default()
            })),
            Some(Err("unobserved drain fixture")),
            None,
        ] {
            let mut totals = Totals::default();
            published(&mut totals, 0, &captured(0, &[]), census_of(&[LINE_A], &[]));
            let mut lost = captured(0, &[]);
            lost.drained = broken;
            assert!(
                matches!(published(&mut totals, 1, &lost, skipped()), TableView::Unavailable {reason} if reason != replay::cadence::SKIPPED_REASON)
            );
            assert!(matches!(
                published(
                    &mut totals,
                    24,
                    &captured(0, &[]),
                    census_of(&[LINE_A], &[])
                ),
                TableView::Unavailable { .. }
            ));
            let mut tables = census_of(&[LINE_A], &[]);
            totals.assemble(2, 30, &captured(0, &[entry(LINE_A, 9)]), &mut tables);
            let TableInput::Sampled(TableView::Complete(TableRows::ConstructionLineOutput(rows))) =
                &tables[TableId::ConstructionLineOutput.index()]
            else {
                panic!("new branch withheld")
            };
            assert_eq!((rows[0].construction_raw, rows[0].since_hour), (0, 30));
        }
    }
    const LOG_OVERFLOW: u64 = crate::output_log::CAPACITY as u64 + 1;

    #[test]
    fn zero_output_building_advances_still_count_as_ticks() {
        let mut totals = Totals::default();
        published(&mut totals, 0, &captured(0, &[]), census_of(&[LINE_A], &[]));
        for day in 1..=2 {
            let rows = rows(published(
                &mut totals,
                day * 24,
                &captured((day - 1) as u64, &[entry(LINE_A, 0)]),
                census_of(&[LINE_A], &[]),
            ));
            assert_eq!(rows.len(), 1);
            let row = &rows[0];
            assert_eq!((row.ticks, row.repair_ticks), (day, 0));
            assert_eq!(
                (
                    row.construction_raw,
                    row.repair_raw,
                    row.bank_raw,
                    row.instant_raw
                ),
                (0, 0, 0, 0)
            );
            assert!(row.exact && !row.ended);
        }
    }

    #[test]
    fn carries_and_zero_ic_repairs_do_not_invent_factory_output() {
        let mut totals = Totals::default();
        published(
            &mut totals,
            0,
            &captured(0, &[]),
            census_of(&[LINE_A], &[RAIL]),
        );
        let entries = [
            Entry {
                repair: true,
                ..entry(LINE_A, 0)
            },
            Entry {
                railway: true,
                ..entry(RAIL, 100)
            },
            Entry {
                railway: true,
                carry: true,
                ..entry(RAIL, 40)
            },
        ];
        let rows = rows(published(
            &mut totals,
            24,
            &captured(0, &entries),
            census_of(&[LINE_A], &[RAIL]),
        ));
        assert_eq!(
            (rows[0].repair_raw, rows[0].repair_ticks, rows[0].ticks),
            (0, 1, 1)
        );
        assert_eq!(
            (rows[1].construction_raw, rows[1].carry_raw, rows[1].ticks),
            (100, 40, 1)
        );
    }

    #[test]
    fn unwritten_ended_lines_repeat_and_overflow_withholds() {
        let mut totals = Totals::default();
        published(&mut totals, 0, &captured(0, &[]), census_of(&[LINE_A], &[]));
        let final_row = rows(published(
            &mut totals,
            24,
            &captured(0, &[entry(LINE_A, 7)]),
            census_of(&[], &[]),
        ));
        assert!(final_row[0].ended);
        assert_eq!(
            rows(published(
                &mut totals,
                25,
                &captured(1, &[]),
                census_of(&[], &[])
            )),
            final_row
        );
        totals.written(true);
        assert!(
            rows(published(
                &mut totals,
                48,
                &captured(1, &[]),
                census_of(&[], &[])
            ))
            .is_empty()
        );
        let mut totals = Totals::default();
        published(&mut totals, 0, &captured(0, &[]), census_of(&[LINE_A], &[]));
        published(
            &mut totals,
            24,
            &captured(0, &[entry(LINE_A, i64::MAX)]),
            census_of(&[LINE_A], &[]),
        );
        assert!(
            matches!(published(&mut totals,48,&captured(1,&[entry(LINE_A,1)]),census_of(&[LINE_A],&[])), TableView::Unavailable {reason} if reason.contains("overflow"))
        );
    }
}
