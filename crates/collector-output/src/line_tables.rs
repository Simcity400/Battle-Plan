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
pub use super::output_continuity::holds_publication;
use super::output_continuity::{self, Progress, Reasons, State};
use crate::line_output::{Drained, Entry, OutputKind};
use crate::table_conversion::unavailable;
use engine_allocation::{owned_vec, reserve, reserve_exact, reserve_map};
use replay_data::{
    BranchId, rows::ProductionLineOutputRow, rows::ProductionLineRow, rows::Recorded,
    rows::TableId, rows::TableRows,
};
use replay_frame::{frame::TableInput, frame::TableView};
use std::io;

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [
    TableId::ProductionLineOutput,
    TableId::RocketProductionOutput,
];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

/// The spool replaces this placeholder with the published table.
pub const ASSEMBLED_BY_SPOOL: &str =
    "assembled by the spool from the production ticks the line output hooks observed";

pub use super::output_continuity::Capacities;
pub struct Observation;
impl output_continuity::Observation for Observation {
    type Entry = Entry;
    const ASSEMBLED_BY_SPOOL: &'static str = ASSEMBLED_BY_SPOOL;
    fn drain_growing(entries: &mut Vec<Entry>) -> Result<Drained, &'static str> {
        crate::line_output::drain_growing(entries)
    }
}
pub type Captured = output_continuity::Captured<Observation, N>;

/// What a line produced under one (country, variant, organisation), in
/// first-seen order: one row.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Output {
    country: u32,
    variant: u64,
    mio: u64,
    units: i64,
    spent: Option<i64>,
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
    rocket: super::rocket_output::Totals,
    unknown_delivery: bool,
}

impl Totals {
    /// Account this frame's drain and publish ProductionLineOutput into
    /// `tables`, beside the frame's ProductionLines. Runs before the staggered
    /// carry, so an off-slot hour's skip is carried like any daily table's.
    pub fn assemble(
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
        let rockets = match self
            .rocket
            .publication(&tables[TableId::ProductionLines.index()])
        {
            Ok(Ok(rows)) => TableView::Complete(TableRows::RocketProductionOutput(rows)),
            Ok(Err(reason)) => unavailable(reason),
            Err(_) => return,
        };
        tables[TableId::RocketProductionOutput.index()] = TableInput::Sampled(rockets);
    }

    /// The same retained rows are used by the source regressions.
    #[cfg(any(test, feature = "test-support"))]
    pub fn rocket_publication(
        &self,
        production_lines: &TableInput,
    ) -> io::Result<Result<Vec<replay_data::rows::RocketProductionOutputRow>, &'static str>> {
        self.rocket.publication(production_lines)
    }
    /// After a frame of this campaign was written, with whether it held the
    /// last publication ([`holds_publication`], taken before the write).
    pub fn written(&mut self, held_publication: bool) {
        self.shared.written(held_publication);
    }

    fn observe(&mut self, branch: BranchId, hour: i32, captured: &Captured) -> io::Result<()> {
        self.rocket.observe(
            branch,
            hour,
            captured.drained,
            captured.handed_over(),
            Entry::rocket_event,
        )?;
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
            Progress::Restart => {
                self.unknown_delivery = false;
            }
            Progress::Entries(range) => {
                let entries = &captured.handed_over()[range];
                self.unknown_delivery |= entries
                    .iter()
                    .any(|entry| matches!(entry.kind, OutputKind::UnknownDelivery));
                let equipment = || {
                    entries
                        .iter()
                        .filter(|entry| matches!(entry.kind, OutputKind::Equipment))
                };
                reserve_map(&mut self.shared.lines, equipment().count())?;
                for entry in equipment() {
                    let line = self.shared.lines.entry(entry.line).or_insert_with(|| Line {
                        first_hour: hour,
                        outputs: Vec::new(),
                        ended: false,
                        pending: 0,
                    });
                    line.pending = line.pending.saturating_add(1);
                }
                for entry in equipment() {
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
                if reason == replay_data::cadence::SKIPPED_REASON =>
            {
                return unavailable(replay_data::cadence::SKIPPED_REASON);
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
                    spent: Some(0),
                    exact: true,
                });
            }
        }
        let count = self.shared.lines.values().try_fold(0usize, |count, line| {
            engine_allocation::add(count, line.outputs.len())
        })?;
        let mut rows = Vec::new();
        reserve_exact(&mut rows, count)?;
        let delivery_exact = !self.unknown_delivery;
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
                    mio_credit: replay_data::rows::RetiredI64::default(),
                    since_hour,
                    observed_since_creation,
                    ended: line.ended,
                    exact: output.exact && delivery_exact,
                    spent_raw: Recorded::Value(output.spent),
                    instant_raw: Recorded::Value(replay_data::rows::RetiredOptionalI64::default()),
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
                spent: Some(0),
                exact: true,
            });
            line.outputs.len() - 1
        }
    };
    let output = &mut line.outputs[index];
    // The organisation credit and the instant progress are not recorded;
    // only the delivered units and the factory spending accumulate.
    if let Some(next) = output.units.checked_add(i64::from(entry.units)) {
        output.units = next;
    } else {
        output.exact = false;
    }
    output.exact &= entry.exact;
    // Classification is coupled by the native instant predicate. Once known,
    // branch-total overflow is field-local and can never recover to zero.
    let spent = entry.spending.map(|(spent, _instant)| spent);
    let next = output
        .spent
        .zip(spent)
        .and_then(|(total, value)| total.checked_add(value));
    if output.spent.is_some() && spent.is_some() && next.is_none() {
        crate::line_output::refuse_spending("production line spent_raw cumulative overflow");
    }
    output.spent = next;
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TABLE_COUNT;
    use replay_data::{rows::RetiredI64, rows::RetiredOptionalI64};

    /// The retired instant spending every row publishes.
    fn no_instant() -> Recorded<RetiredOptionalI64> {
        Recorded::Value(RetiredOptionalI64::default())
    }

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
            reason: replay_data::cadence::SKIPPED_REASON.into(),
        })
    }

    fn entry(line: u64, variant: u64, mio: u64, units: i32, credit: i32) -> Entry {
        Entry {
            kind: OutputKind::Equipment,
            country: 5001,
            line,
            variant,
            mio,
            units,
            credit,
            spending: Some((0, 0)),
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
        units: i64,
        (observed_since_creation, ended): (bool, bool),
    ) -> ProductionLineOutputRow {
        ProductionLineOutputRow {
            line,
            ordinal,
            country: 5001,
            variant,
            mio,
            units,
            mio_credit: RetiredI64::default(),
            since_hour: 100,
            observed_since_creation,
            ended,
            exact: true,
            spent_raw: Recorded::Value(Some(0)),
            instant_raw: no_instant(),
        }
    }

    #[test]
    fn retired_credit_and_instant_never_change_published_output() {
        let mut totals = Totals::default();
        let lines = [(LINE_A, TANK, ORG)];
        assemble(&mut totals, 7, 100, &frame(1, 0, &[]), census(&lines));
        // Credits and instant progress that would overflow their running
        // totals: neither is recorded, so units stay exact and spending
        // stays known.
        let ticks: Vec<Entry> = (0..3)
            .map(|_| Entry {
                spending: Some((5, i64::MAX)),
                ..entry(LINE_A, TANK, ORG, 1, i32::MAX)
            })
            .collect();
        let output = rows(assemble(
            &mut totals,
            7,
            124,
            &frame(1, 0, &ticks),
            census(&lines),
        ));
        assert_eq!(output[0].units, 3);
        assert!(output[0].exact);
        assert_eq!(output[0].spent_raw, Recorded::Value(Some(15)));
        assert_eq!(output[0].mio_credit, RetiredI64::default());
        assert_eq!(output[0].instant_raw, no_instant());
    }

    #[test]
    fn signed_zero_unit_spending_and_field_local_overflow_survive_publication() {
        let mut totals = Totals::default();
        let lines = [(LINE_A, TANK, ORG)];
        assemble(&mut totals, 7, 100, &frame(1, 0, &[]), census(&lines));
        let mut tick = entry(LINE_A, TANK, ORG, 0, 0);
        tick.spending = Some((i64::MAX, -11));
        let first = rows(assemble(
            &mut totals,
            7,
            124,
            &frame(1, 0, &[tick]),
            census(&lines),
        ));
        assert_eq!(first[0].spent_raw, Recorded::Value(Some(i64::MAX)));
        assert_eq!(first[0].instant_raw, no_instant());
        assert!(first[0].exact);
        assert_eq!(
            (first[0].units, first[0].mio_credit),
            (0, RetiredI64::default())
        );
        tick.spending = Some((1, -7));
        tick.units = 2;
        tick.credit = 2;
        let second = rows(assemble(
            &mut totals,
            7,
            148,
            &frame(1, 1, &[tick]),
            census(&lines),
        ));
        assert_eq!(second[0].spent_raw, Recorded::Value(None));
        assert_eq!(second[0].instant_raw, no_instant());
        assert!(second[0].exact);
        assert_eq!(
            (second[0].units, second[0].mio_credit),
            (2, RetiredI64::default())
        );
        tick.spending = Some((-1, 0));
        let last = rows(assemble(
            &mut totals,
            7,
            172,
            &frame(1, 2, &[tick]),
            census(&[]),
        ));
        assert!(last[0].ended && last[0].exact);
        assert_eq!(last[0].spent_raw, Recorded::Value(None));
        assert_eq!(last[0].instant_raw, no_instant());
    }

    #[test]
    fn failed_classification_keeps_known_units_and_both_spending_fields_unavailable() {
        let mut totals = Totals::default();
        let lines = [(LINE_A, TANK, ORG)];
        assemble(&mut totals, 7, 100, &frame(1, 0, &[]), census(&lines));
        let mut tick = entry(LINE_A, TANK, ORG, 3, 3);
        tick.spending = None;
        let known = rows(assemble(
            &mut totals,
            7,
            124,
            &frame(1, 0, &[tick]),
            census(&lines),
        ));
        assert_eq!(
            (known[0].units, known[0].mio_credit, known[0].exact),
            (3, RetiredI64::default(), true)
        );
        assert_eq!(known[0].spent_raw, Recorded::Value(None));
        assert_eq!(known[0].instant_raw, no_instant());
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
                row(LINE_A, 0, (TANK, ORG), 0, (false, false)),
                row(LINE_B, 0, (SHIP, 0), 0, (false, false)),
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
            replay_data::cadence::SKIPPED_REASON
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
                row(LINE_A, 0, (TANK, ORG), 3, (false, false)),
                row(LINE_A, 1, (TANK_2, ORG), 2, (false, false)),
                row(LINE_B, 0, (SHIP, 0), 1, (false, true)),
                row(LINE_C, 0, (TANK, ORG), 1, (true, false)),
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
        let ended_b = row(LINE_B, 0, (SHIP, 0), 1, (false, true));
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
                ..row(LINE_A, 0, (TANK, 0), 3, (true, true))
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
        assert_eq!(
            (published[0].units, published[0].mio_credit),
            (3, RetiredI64::default())
        );
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
                ..row(LINE_B, 0, (SHIP, 0), 1, (false, true))
            }
        );
    }

    #[test]
    fn admitted_empty_rocket_drain_publishes_world_coverage() {
        let mut totals = Totals::default();
        let mut tables: Vec<TableInput> = (0..TABLE_COUNT)
            .map(|_| TableInput::Sampled(unavailable("absent")))
            .collect();
        tables[TableId::ProductionLines.index()] = census(&[]);
        totals.assemble(7, 100, &frame(1, 0, &[]), &mut tables);
        let TableInput::Sampled(TableView::Complete(TableRows::RocketProductionOutput(rows))) =
            tables.swap_remove(TableId::RocketProductionOutput.index())
        else {
            panic!("empty admitted drain must publish");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].country, rows[0].variant, rows[0].since_hour),
            (0, 0, 100)
        );
        assert!(rows[0].exact && rows[0].units_exact);
    }

    #[test]
    fn mixed_common_indices_and_unknown_delivery_preserve_spending_and_refuse_units() {
        use engine_production::line_output_sites::RocketFailure;
        let mut totals = Totals::default();
        let lines = [(LINE_A, TANK, ORG)];
        assemble(&mut totals, 7, 100, &frame(1, 0, &[]), census(&lines));
        let mut first = entry(LINE_A, TANK, ORG, 3, 3);
        first.spending = Some((11, 0));
        let mut second = entry(LINE_A, TANK, ORG, 4, 4);
        second.spending = Some((13, 0));
        let mixed = [
            Entry {
                kind: OutputKind::Spending(RocketFailure::None),
                country: 5001,
                variant: SHIP,
                spending: Some((-7, 0)),
                exact: true,
                ..Entry::default()
            },
            first,
            Entry {
                kind: OutputKind::UnknownDelivery,
                ..Entry::default()
            },
            Entry {
                kind: OutputKind::Delivery { units_known: true },
                country: 5001,
                variant: SHIP,
                units: -2,
                exact: true,
                ..Entry::default()
            },
            second,
        ];
        let equipment = rows(assemble(
            &mut totals,
            7,
            124,
            &frame(1, 0, &mixed),
            census(&lines),
        ));
        assert_eq!(equipment.len(), 1); // no Rocket/fake line0 in equipment
        assert_eq!(
            (equipment[0].units, equipment[0].mio_credit),
            (7, RetiredI64::default())
        );
        assert_eq!(equipment[0].spent_raw, Recorded::Value(Some(24)));
        assert!(!equipment[0].exact);
        let rocket = totals.rocket_publication(&census(&lines)).unwrap().unwrap();
        let world = rocket
            .iter()
            .find(|row| (row.country, row.variant) == (0, 0))
            .unwrap();
        assert!(world.exact && !world.units_exact);
        assert_eq!(
            (world.ticks, world.deliveries, world.inexact_deliveries),
            (1, 2, 1)
        );
        let design = rocket.iter().find(|row| row.variant == SHIP).unwrap();
        assert_eq!((design.spent_raw, design.units), (-7, -2));
        assert!(design.exact && design.units_exact);
        // Replay of already-consumed common indices never duplicates either arm.
        let repeated = rows(assemble(
            &mut totals,
            7,
            125,
            &frame(1, 0, &mixed),
            census(&lines),
        ));
        assert_eq!(
            (repeated[0].units, repeated[0].spent_raw),
            (7, Recorded::Value(Some(24)))
        );
        let quiet = rows(assemble(
            &mut totals,
            7,
            148,
            &frame(1, 5, &[]),
            census(&lines),
        ));
        assert!(!quiet[0].exact);
        let world_again = totals.rocket_publication(&census(&lines)).unwrap().unwrap();
        assert_eq!(
            world_again
                .iter()
                .find(|row| row.country == 0)
                .unwrap()
                .deliveries,
            2
        );
        // Synthetic admitted branch input resets observation; it does not prove a load.
        let reset = rows(assemble(
            &mut totals,
            8,
            200,
            &frame(1, 5, &[]),
            census(&lines),
        ));
        assert!(reset[0].exact);
        assert_eq!(reset[0].units, 0);
    }

    #[test]
    fn short_mixed_drain_keeps_original_common_indices_and_finishes_once() {
        use engine_production::line_output_sites::RocketFailure;
        let mut totals = Totals::default();
        let lines = [(LINE_A, TANK, ORG)];
        assemble(&mut totals, 7, 100, &frame(1, 0, &[]), census(&lines));
        let rocket = Entry {
            kind: OutputKind::Spending(RocketFailure::None),
            country: 5001,
            variant: SHIP,
            spending: Some((17, 0)),
            exact: true,
            ..Entry::default()
        };
        let equipment = entry(LINE_A, TANK, ORG, 2, 2);
        let mut short = Captured::with_capacities(&Capacities::default());
        short.set_drained_for_test(
            Ok(Drained {
                epoch: 1,
                first: 0,
                cursor: 3,
                copied: 2,
                pending: 3,
            }),
            &[rocket, equipment],
        );
        assert_eq!(
            reason(assemble(&mut totals, 7, 124, &short, census(&lines))),
            "production ticks before this hour's sample are still in the log"
        );
        assert_eq!(
            totals
                .rocket_publication(&census(&lines))
                .unwrap()
                .unwrap_err(),
            "rocket events before this sample are still in the log"
        );
        let last = Entry {
            kind: OutputKind::Delivery { units_known: true },
            country: 5001,
            variant: SHIP,
            units: 1,
            exact: true,
            ..Entry::default()
        };
        let known = rows(assemble(
            &mut totals,
            7,
            125,
            &frame(1, 2, &[last]),
            census(&lines),
        ));
        assert_eq!(known[0].units, 2);
        let rocket = totals.rocket_publication(&census(&lines)).unwrap().unwrap();
        let design = rocket.iter().find(|row| row.variant == SHIP).unwrap();
        assert_eq!((design.spent_raw, design.units), (17, 1));
    }
}
