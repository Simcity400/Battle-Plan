//! ProductionLineOutput: the units each production line has completed, totalled
//! per recording branch from the production ticks the `line_output` hooks
//! observe.
//!
//! Nothing is read from the game for it. Every clocked hour the callback hands
//! the hook log over into this pack's preallocated entries ([`Captured::drain`]);
//! the spool adds them to the branch's [`Totals`] and publishes the table with
//! that hour's ProductionLines census, which decides which lines ended and
//! which were created after the observation began. The table shares the
//! census's daily slot: off it, the spool carries the last publication, until
//! a tick goes unobserved.
use super::output_continuity::OutputMap;
pub(crate) use super::output_continuity::holds_publication;
use super::output_continuity::{self, Progress, Reasons, State};
use crate::line_output::{Drained, Entry};
use crate::spooling::unavailable;
use engine::allocation::{owned_vec, reserve, reserve_exact, reserve_map};
use replay::{
    BranchId,
    frame::{TableInput, TableView},
    rows::{ProductionLineOutputRow, ProductionLineRow, TableId, TableRows},
};
use std::io;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ProductionLineOutput];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

/// The spool replaces this placeholder with the published table.
pub(crate) const ASSEMBLED_BY_SPOOL: &str =
    "assembled by the spool from the production ticks the line output hooks observed";

pub use super::output_continuity::Capacities;
pub struct Observation;
impl output_continuity::Observation for Observation {
    type Entry = Entry;
    const ASSEMBLED_BY_SPOOL: &'static str = ASSEMBLED_BY_SPOOL;
    fn drain(entries: &mut [Entry]) -> Result<Drained, &'static str> {
        crate::line_output::drain(entries)
    }
}
pub type Captured = output_continuity::Captured<Observation>;

/// What a line produced under one (country, variant, organisation), in
/// first-seen order: one row.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Output {
    country: u32,
    variant: u64,
    mio: u64,
    units: i64,
    credit: i64,
    exact: bool,
}

#[derive(Debug)]
struct Line {
    /// The first hour whose drain or census showed the line.
    first_hour: i32,
    outputs: Vec<Output>,
    /// Published as ended; dropped at the next publication.
    ended: bool,
    pending: usize,
}

/// The branch's per-line totals, owned by the spool for one campaign.
#[derive(Debug, Default)]
pub struct Totals {
    shared: output_continuity::Totals<Line>,
}

impl Totals {
    /// Account this frame's drain and publish ProductionLineOutput into
    /// `tables`, beside the frame's ProductionLines. Runs before the staggered
    /// carry, so an off-slot hour's skip is carried like any daily table's.
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
        let published = self.publish(hour, &tables[TableId::ProductionLines.index()]);
        tables[TableId::ProductionLineOutput.index()] = TableInput::Sampled(published);
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
                first: "the branch's first frame drained no ticks",
                restarted: "the production line observation restarted",
                lost: "production ticks were lost",
                absent: "a frame drained no ticks",
            },
        );
        match progress {
            Progress::Restart => {}
            Progress::Entries(range) => {
                let entries = &captured.handed_over()[range];
                reserve_map(&mut self.shared.lines, entries.len())?;
                for entry in entries {
                    let line = self.shared.lines.entry(entry.line).or_insert_with(|| Line {
                        first_hour: hour,
                        outputs: Vec::new(),
                        ended: false,
                        pending: 0,
                    });
                    line.pending = line.pending.saturating_add(1);
                }
                for entry in entries {
                    let line = self.shared.lines.get_mut(&entry.line).unwrap();
                    if line.pending != 0 {
                        reserve(&mut line.outputs, line.pending)?;
                        line.pending = 0;
                    }
                    add(&mut self.shared.lines, entry, hour);
                }
            }
            Progress::None => {}
        }
        Ok(())
    }

    fn publish(&mut self, hour: i32, lines: &TableInput) -> TableView {
        // A lost tick shows from the hour it is seen, not the next census.
        let since_hour = match &self.shared.state {
            State::Observing { since_hour, .. } => *since_hour,
            State::Withheld(reason) => return unavailable(reason),
        };
        let census = match lines {
            TableInput::Sampled(TableView::Complete(TableRows::ProductionLines(rows))) => rows,
            TableInput::SharedComplete(rows) => match &**rows {
                TableRows::ProductionLines(rows) => rows,
                _ => return unavailable("ProductionLines holds another table"),
            },
            TableInput::Sampled(TableView::Unavailable { reason })
                if reason == replay::cadence::SKIPPED_REASON =>
            {
                return unavailable(replay::cadence::SKIPPED_REASON);
            }
            _ => return unavailable("ProductionLines is incomplete at this hour"),
        };
        if !self.shared.complete {
            return unavailable("production ticks before this hour's sample are still in the log");
        }
        TableView::Complete(TableRows::ProductionLineOutput(owned_vec(
            self.rows(hour, since_hour, census),
        )))
    }

    fn rows(
        &mut self,
        hour: i32,
        since_hour: i32,
        census: &[ProductionLineRow],
    ) -> io::Result<Vec<ProductionLineOutputRow>> {
        let live = |handle: u64| {
            census
                .binary_search_by_key(&handle, |row| row.handle)
                .is_ok()
        };
        self.shared
            .prepare_publication(hour, census.iter().map(|row| row.handle), |line| line.ended)?;
        reserve_map(&mut self.shared.lines, census.len())?;
        let baseline = &self.shared.census;
        for row in census {
            let line = self.shared.lines.entry(row.handle).or_insert_with(|| Line {
                first_hour: hour,
                outputs: Vec::new(),
                ended: false,
                pending: 0,
            });
            // Every live line has a row, zero until it produces.
            if line.outputs.is_empty() {
                reserve(&mut line.outputs, 1)?;
                line.outputs.push(Output {
                    country: row.country,
                    variant: row.variant,
                    mio: row.mio,
                    units: 0,
                    credit: 0,
                    exact: true,
                });
            }
        }
        let count = self.shared.lines.values().try_fold(0usize, |count, line| {
            engine::allocation::add(count, line.outputs.len())
        })?;
        let mut rows = Vec::new();
        reserve_exact(&mut rows, count)?;
        for (&handle, line) in &mut self.shared.lines {
            line.ended = !live(handle);
            // Absent from the first census and first seen after it: created
            // after that census, so every tick of its life was observed.
            let observed_since_creation = baseline.observed_since_creation(handle, line.first_hour);
            rows.extend(line.outputs.iter().enumerate().map(|(ordinal, output)| {
                ProductionLineOutputRow {
                    line: handle,
                    ordinal: ordinal as u32,
                    country: output.country,
                    variant: output.variant,
                    mio: output.mio,
                    units: output.units,
                    mio_credit: output.credit,
                    since_hour,
                    observed_since_creation,
                    ended: line.ended,
                    exact: output.exact,
                }
            }));
        }
        rows.sort_unstable_by_key(|row| (row.line, row.ordinal));
        Ok(rows)
    }
}

fn add(lines: &mut OutputMap<Line>, entry: &Entry, hour: i32) {
    let line = lines
        .get_mut(&entry.line)
        .expect("batch admitted every output line");
    let _ = hour;
    let index = match line.outputs.iter().position(|output| {
        output.country == entry.country
            && output.variant == entry.variant
            && output.mio == entry.mio
    }) {
        Some(index) => index,
        None => {
            line.outputs.push(Output {
                country: entry.country,
                variant: entry.variant,
                mio: entry.mio,
                units: 0,
                credit: 0,
                exact: true,
            });
            line.outputs.len() - 1
        }
    };
    let output = &mut line.outputs[index];
    output.units += i64::from(entry.units);
    output.credit += i64::from(entry.credit);
    output.exact &= entry.exact;
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::frame::TABLE_COUNT;

    const LINE_A: u64 = 0x10_0000_0021;
    const LINE_B: u64 = 0x11_0000_0021;
    const LINE_C: u64 = 0x12_0000_0021;
    const TANK: u64 = 0x5_0000_0046;
    const TANK_2: u64 = 0x6_0000_0046;
    const SHIP: u64 = 0x7_0000_0046;
    const ORG: u64 = 0x3_0000_004f;

    fn census(lines: &[(u64, u64, u64)]) -> TableInput {
        TableInput::Sampled(TableView::Complete(TableRows::ProductionLines(
            lines
                .iter()
                .map(|&(handle, variant, mio)| ProductionLineRow {
                    handle,
                    country: 5001,
                    variant,
                    mio,
                    ..Default::default()
                })
                .collect(),
        )))
    }

    fn skipped() -> TableInput {
        TableInput::Sampled(TableView::Unavailable {
            reason: replay::cadence::SKIPPED_REASON.into(),
        })
    }

    fn entry(line: u64, variant: u64, mio: u64, units: i32, credit: i32) -> Entry {
        Entry {
            country: 5001,
            line,
            variant,
            mio,
            units,
            credit,
            exact: true,
        }
    }

    /// A frame's drain of `entries`, continuing the log at `first`.
    fn frame(epoch: u64, first: u64, entries: &[Entry]) -> Captured {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let end = first + entries.len() as u64;
        captured.set_drained_for_test(
            Ok(Drained {
                epoch,
                first,
                cursor: end,
                copied: entries.len(),
                pending: entries.len(),
            }),
            entries,
        );
        captured
    }

    /// One frame through the totals, written.
    fn assemble(
        totals: &mut Totals,
        branch: BranchId,
        hour: i32,
        captured: &Captured,
        lines: TableInput,
    ) -> TableInput {
        let output = unwritten(totals, branch, hour, captured, lines);
        totals.written(holds_publication(&output));
        output
    }

    /// One frame through the totals whose write failed.
    fn unwritten(
        totals: &mut Totals,
        branch: BranchId,
        hour: i32,
        captured: &Captured,
        lines: TableInput,
    ) -> TableInput {
        let mut tables: Vec<TableInput> = (0..TABLE_COUNT)
            .map(|_| TableInput::Sampled(unavailable("absent")))
            .collect();
        tables[TableId::ProductionLines.index()] = lines;
        totals.assemble(branch, hour, captured, &mut tables);
        tables.swap_remove(TableId::ProductionLineOutput.index())
    }

    fn rows(input: TableInput) -> Vec<ProductionLineOutputRow> {
        match input {
            TableInput::Sampled(TableView::Complete(TableRows::ProductionLineOutput(rows))) => rows,
            other => panic!("not published: {other:?}"),
        }
    }

    fn reason(input: TableInput) -> String {
        match input {
            TableInput::Sampled(TableView::Unavailable { reason }) => reason,
            other => panic!("published: {other:?}"),
        }
    }

    fn row(
        line: u64,
        ordinal: u32,
        (variant, mio): (u64, u64),
        (units, mio_credit): (i64, i64),
        (observed_since_creation, ended): (bool, bool),
    ) -> ProductionLineOutputRow {
        ProductionLineOutputRow {
            line,
            ordinal,
            country: 5001,
            variant,
            mio,
            units,
            mio_credit,
            since_hour: 100,
            observed_since_creation,
            ended,
            exact: true,
        }
    }

    /// Ticks drained at the branch's first frame came before its sample; later
    /// ticks add up per line and row; a pre-existing line counts from the
    /// observation start, a line created later since its creation; an ended
    /// line is published once and dropped; a line that produced nothing still
    /// has a zero row.
    #[test]
    fn totals_count_from_the_branch_start_and_publish_ended_lines_once() {
        let mut totals = Totals::default();
        let lines = [(LINE_A, TANK, ORG), (LINE_B, SHIP, 0)];
        // The branch's first frame: its entries precede the observation.
        let first = frame(1, 10, &[entry(LINE_A, TANK, ORG, 9, 9)]);
        let published = assemble(&mut totals, 7, 100, &first, census(&lines));
        assert_eq!(
            rows(published),
            [
                row(LINE_A, 0, (TANK, ORG), (0, 0), (false, false)),
                row(LINE_B, 0, (SHIP, 0), (0, 0), (false, false)),
            ]
        );
        // A day of production, off the census slot: carried, still counted.
        let day = frame(
            1,
            11,
            &[
                entry(LINE_A, TANK, ORG, 3, 3),
                // A blocked ship credits without delivering.
                entry(LINE_B, SHIP, 0, 0, 2),
                // A new line produces before any census sees it.
                entry(LINE_C, TANK, ORG, 1, 1),
            ],
        );
        assert_eq!(
            reason(assemble(&mut totals, 7, 120, &day, skipped())),
            replay::cadence::SKIPPED_REASON
        );
        // The line switches variant; line B completes and is deleted.
        let next = frame(
            1,
            14,
            &[
                entry(LINE_A, TANK_2, ORG, 2, 2),
                entry(LINE_B, SHIP, 0, 1, 1),
            ],
        );
        let lines = [(LINE_A, TANK_2, ORG), (LINE_C, TANK, ORG)];
        assert_eq!(
            rows(assemble(&mut totals, 7, 124, &next, census(&lines))),
            [
                row(LINE_A, 0, (TANK, ORG), (3, 3), (false, false)),
                row(LINE_A, 1, (TANK_2, ORG), (2, 2), (false, false)),
                row(LINE_B, 0, (SHIP, 0), (1, 3), (false, true)),
                row(LINE_C, 0, (TANK, ORG), (1, 1), (true, false)),
            ]
        );
        // The ended line was published once.
        let quiet = frame(1, 16, &[]);
        assert_eq!(
            rows(assemble(&mut totals, 7, 148, &quiet, census(&lines)))
                .iter()
                .map(|row| (row.line, row.ordinal))
                .collect::<Vec<_>>(),
            [(LINE_A, 0), (LINE_A, 1), (LINE_C, 0)]
        );
    }

    /// A publication lost with its frame's write keeps its ended lines, so the
    /// next publication, even a census at the next hour, repeats their final
    /// totals; a written carry of the publication is their publication too.
    #[test]
    fn ended_lines_repeat_until_a_written_frame_holds_their_publication() {
        let mut totals = Totals::default();
        let both = [(LINE_A, TANK, ORG), (LINE_B, SHIP, 0)];
        let only_a = [(LINE_A, TANK, ORG)];
        assemble(&mut totals, 7, 100, &frame(1, 10, &[]), census(&both));
        let ended_b = row(LINE_B, 0, (SHIP, 0), (1, 1), (false, true));
        // Line B completes and is deleted; that publication is not written.
        let day = frame(1, 10, &[entry(LINE_B, SHIP, 0, 1, 1)]);
        let lost = rows(unwritten(&mut totals, 7, 124, &day, census(&only_a)));
        assert_eq!(lost[1], ended_b);
        let again = rows(assemble(
            &mut totals,
            7,
            125,
            &frame(1, 11, &[]),
            census(&only_a),
        ));
        assert_eq!(again[1], ended_b);
        let later = assemble(&mut totals, 7, 148, &frame(1, 11, &[]), census(&only_a));
        assert_eq!(rows(later).len(), 1);
        // Line A ends unwritten too, but a written carry holds its publication.
        let day = frame(1, 11, &[entry(LINE_A, TANK, ORG, 2, 2)]);
        let lost = rows(unwritten(&mut totals, 7, 172, &day, census(&[])));
        assert!(lost[0].ended);
        assemble(&mut totals, 7, 173, &frame(1, 12, &[]), skipped());
        let last = assemble(&mut totals, 7, 196, &frame(1, 12, &[]), census(&[]));
        assert!(rows(last).is_empty());
    }

    #[test]
    fn unseen_line_without_mio_keeps_its_producer_in_its_final_publication() {
        let mut totals = Totals::default();
        assemble(&mut totals, 7, 100, &frame(1, 10, &[]), census(&[]));
        let mut delivered = entry(LINE_A, TANK, 0, 3, 3);
        delivered.country = 6001;
        // Created, completed and deleted between censuses. Neither its MIO
        // nor a ProductionLines row can supply this country.
        assemble(&mut totals, 7, 101, &frame(1, 10, &[delivered]), skipped());
        let ended = rows(assemble(
            &mut totals,
            7,
            124,
            &frame(1, 11, &[]),
            census(&[]),
        ));
        assert_eq!(
            ended,
            [ProductionLineOutputRow {
                country: 6001,
                ..row(LINE_A, 0, (TANK, 0), (3, 3), (true, true))
            }]
        );
        assert!(
            rows(assemble(
                &mut totals,
                7,
                148,
                &frame(1, 11, &[]),
                census(&[])
            ))
            .is_empty()
        );
    }

    /// The game never moves a line to another country (the oracle's producer
    /// check says why), but a tick naming another producer still gets its own
    /// row rather than relabelling earlier output.
    #[test]
    fn producer_change_does_not_relabel_earlier_output() {
        let mut totals = Totals::default();
        assemble(&mut totals, 7, 100, &frame(1, 10, &[]), census(&[]));
        let first = entry(LINE_A, TANK, 0, 3, 3);
        let second = Entry {
            country: 6001,
            ..first
        };
        let published = rows(assemble(
            &mut totals,
            7,
            124,
            &frame(1, 10, &[first, second]),
            census(&[]),
        ));
        assert_eq!(
            published
                .iter()
                .map(|row| (row.ordinal, row.country, row.units))
                .collect::<Vec<_>>(),
            [(0, 5001, 3), (1, 6001, 3)]
        );
    }

    /// A lost drain, a new epoch or an unobserved drain withholds the table
    /// from that hour until the next branch; the next branch starts afresh.
    #[test]
    fn lost_ticks_withhold_the_table_until_the_next_branch_resets_it() {
        let lines = [(LINE_A, TANK, ORG)];
        for (broken, expected) in [
            (frame(1, 12, &[]), "production ticks were lost"),
            (
                frame(2, 11, &[]),
                "the production line observation restarted",
            ),
        ] {
            let mut totals = Totals::default();
            assemble(&mut totals, 7, 100, &frame(1, 10, &[]), census(&lines));
            assemble(
                &mut totals,
                7,
                101,
                &frame(1, 10, &[entry(LINE_A, TANK, ORG, 4, 4)]),
                skipped(),
            );
            // Seen off the census slot, the loss is not hidden behind a carry.
            assert_eq!(
                reason(assemble(&mut totals, 7, 102, &broken, skipped())),
                expected
            );
            // Still withheld at the census, even with a continuous drain.
            assert_eq!(
                reason(assemble(
                    &mut totals,
                    7,
                    103,
                    &frame(1, 11, &[]),
                    census(&lines)
                )),
                expected
            );
            // A load: the new branch counts from its own first sample.
            let reset = assemble(
                &mut totals,
                8,
                200,
                &frame(1, 11, &[entry(LINE_A, TANK, ORG, 5, 5)]),
                census(&lines),
            );
            assert_eq!(rows(reset)[0].units, 0);
            let later = assemble(
                &mut totals,
                8,
                224,
                &frame(1, 12, &[entry(LINE_A, TANK, ORG, 6, 6)]),
                census(&lines),
            );
            let later = rows(later);
            assert_eq!((later[0].units, later[0].since_hour), (6, 200));
        }
        let mut totals = Totals::default();
        let mut unobserved = Captured::with_capacities(&Capacities::default());
        unobserved.set_drained_for_test(Err("production line hooks are not installed"), &[]);
        assert_eq!(
            reason(assemble(&mut totals, 7, 100, &unobserved, census(&lines))),
            "production line hooks are not installed"
        );
    }

    /// Entries left in the log by short storage make the hour incomplete; they
    /// arrive with a later drain and count once. Entries below the branch's
    /// first cursor, handed over late, never count.
    #[test]
    fn deferred_entries_count_once_and_leave_the_hour_unpublished_until_then() {
        let lines = [(LINE_A, TANK, ORG)];
        let mut totals = Totals::default();
        // At the branch start two old entries are still waiting (cursor 12).
        let mut start = Captured::with_capacities(&Capacities::default());
        start.set_drained_for_test(
            Ok(Drained {
                epoch: 1,
                first: 10,
                cursor: 12,
                copied: 0,
                pending: 2,
            }),
            &[],
        );
        assemble(&mut totals, 7, 100, &start, census(&lines));
        // Two new ticks arrive; storage takes three of the four waiting.
        let mut short = Captured::with_capacities(&Capacities::default());
        short.set_drained_for_test(
            Ok(Drained {
                epoch: 1,
                first: 10,
                cursor: 14,
                copied: 3,
                pending: 4,
            }),
            &[
                entry(LINE_A, TANK, ORG, 50, 50),
                entry(LINE_A, TANK, ORG, 60, 60),
                entry(LINE_A, TANK, ORG, 1, 1),
            ],
        );
        assert_eq!(
            reason(assemble(&mut totals, 7, 124, &short, census(&lines))),
            "production ticks before this hour's sample are still in the log"
        );
        let rest = frame(1, 13, &[entry(LINE_A, TANK, ORG, 2, 2)]);
        let published = rows(assemble(&mut totals, 7, 125, &rest, census(&lines)));
        assert_eq!((published[0].units, published[0].mio_credit), (3, 3));
    }

    /// A line absent from the first census whose ticks were drained no later
    /// than it ended before that census: it may have existed before the
    /// observation, so it is not observed since creation. An inexact tick
    /// marks its row.
    #[test]
    fn lines_that_ended_before_the_first_census_are_not_observed_since_creation() {
        let mut totals = Totals::default();
        let lines = [(LINE_A, TANK, ORG)];
        // The first frame's census is unavailable: no census yet.
        let start = frame(1, 10, &[]);
        let incomplete = TableInput::Sampled(unavailable("walk failed"));
        assert_eq!(
            reason(assemble(&mut totals, 7, 100, &start, incomplete)),
            "ProductionLines is incomplete at this hour"
        );
        let mut inexact = entry(LINE_B, SHIP, 0, 0, 1);
        inexact.exact = false;
        let day = frame(1, 10, &[entry(LINE_B, SHIP, 0, 1, 1), inexact]);
        let published = rows(assemble(&mut totals, 7, 108, &day, census(&lines)));
        assert_eq!(
            published[1],
            ProductionLineOutputRow {
                exact: false,
                ..row(LINE_B, 0, (SHIP, 0), (1, 2), (false, true))
            }
        );
    }
}
