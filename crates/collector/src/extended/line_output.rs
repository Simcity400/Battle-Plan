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
use crate::{
    line_output::{Drained, Entry},
    table_capture::{grow_capacity, grow_vec},
};
use engine::walk::Outcome;
use replay::{
    BranchId,
    frame::{TableInput, TableView},
    rows::{ProductionLineOutputRow, ProductionLineRow, TableId, TableRows},
};
use std::collections::BTreeMap;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ProductionLineOutput];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

/// The spool replaces this placeholder with the published table.
pub(crate) const ASSEMBLED_BY_SPOOL: &str =
    "assembled by the spool from the production ticks the line output hooks observed";

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    /// Log entries one drain can take, presized from the live equipment-line
    /// population. Any remainder waits in the log for off-thread growth.
    pub entries: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self { entries: 4096 }
    }
}

pub struct Captured {
    entries: Vec<Entry>,
    /// This hour's drain; `None` before a clocked callback drained.
    drained: Option<Result<Drained, &'static str>>,
}
impl Captured {
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            entries: vec![Entry::default(); capacity.entries],
            drained: None,
        }
    }
    pub fn reset(&mut self) {
        self.drained = None;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>()
    }
    /// Off-thread: room for every entry the last drain found waiting.
    pub fn grow(&mut self, capacity: &mut Capacities) {
        if let Some(Ok(drained)) = self.drained {
            grow_capacity(&mut capacity.entries, drained.pending);
        }
        grow_vec(&mut self.entries, capacity.entries);
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [Outcome::Unavailable {
            reason: ASSEMBLED_BY_SPOOL,
        }]
    }
    pub fn tables(&self) -> [TableView; N] {
        [TableView::Unavailable {
            reason: ASSEMBLED_BY_SPOOL.into(),
        }]
    }
    /// The clocked callback's hand-over of the hook log, copied without
    /// allocating.
    pub fn drain(&mut self) {
        self.drained = Some(crate::line_output::drain(&mut self.entries));
    }
    /// The entries this hour's drain handed over.
    fn handed_over(&self) -> &[Entry] {
        match self.drained {
            Some(Ok(drained)) => &self.entries[..drained.copied],
            _ => &[],
        }
    }
    #[cfg(test)]
    pub(crate) fn set_drained_for_test(
        &mut self,
        drained: Result<Drained, &'static str>,
        entries: &[Entry],
    ) {
        self.entries[..entries.len()].copy_from_slice(entries);
        self.drained = Some(drained);
    }
}

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
}

#[derive(Debug)]
enum State {
    Observing {
        epoch: u64,
        /// The first log index not yet accounted for.
        next: u64,
        /// The hour whose sample began the observation.
        since_hour: i32,
    },
    /// Some ticks since the branch began went unobserved: nothing is published
    /// until the next branch.
    Withheld(String),
}

/// The branch's per-line totals, owned by the spool for one campaign.
#[derive(Debug)]
pub struct Totals {
    branch: Option<BranchId>,
    state: State,
    /// Whether every tick before this hour's sample has been accounted for.
    complete: bool,
    /// The hour and lines of the branch's first complete ProductionLines census.
    census: Option<(i32, Vec<u64>)>,
    lines: BTreeMap<u64, Line>,
    /// A written frame held the last publication, published or carried
    /// ([`Totals::written`]). Until one has, its ended lines are kept and
    /// published again, so a failed write cannot drop their final totals.
    publication_written: bool,
}

impl Default for Totals {
    fn default() -> Self {
        Self {
            branch: None,
            state: State::Withheld("no frame observed yet".into()),
            complete: false,
            census: None,
            lines: BTreeMap::new(),
            publication_written: false,
        }
    }
}

/// Whether a frame whose ProductionLineOutput input is `output` holds the
/// branch's last publication: the publication itself, or a carry of it. An
/// off-slot hour keeps the skip marker only when the encoder carries the rows
/// it last wrote; otherwise the spool replaces it with its cached last sample.
pub(crate) fn holds_publication(output: &TableInput) -> bool {
    match output {
        TableInput::Sampled(TableView::Complete(_)) | TableInput::SharedComplete(_) => true,
        TableInput::Sampled(TableView::Unavailable { reason }) => {
            reason == replay::cadence::SKIPPED_REASON
        }
        _ => false,
    }
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
        self.observe(branch, hour, captured);
        let published = self.publish(hour, &tables[TableId::ProductionLines.index()]);
        tables[TableId::ProductionLineOutput.index()] = TableInput::Sampled(published);
    }

    /// After a frame of this campaign was written, with whether it held the
    /// last publication ([`holds_publication`], taken before the write).
    pub(crate) fn written(&mut self, held_publication: bool) {
        self.publication_written |= held_publication;
    }

    fn observe(&mut self, branch: BranchId, hour: i32, captured: &Captured) {
        if self.branch != Some(branch) {
            // A new branch counts from its first sample: the ticks drained now
            // or later below this drain's cursor came before it.
            *self = Self {
                branch: Some(branch),
                state: match captured.drained {
                    Some(Ok(drained)) => State::Observing {
                        epoch: drained.epoch,
                        next: drained.cursor,
                        since_hour: hour,
                    },
                    Some(Err(reason)) => State::Withheld(reason.into()),
                    None => State::Withheld("the branch's first frame drained no ticks".into()),
                },
                complete: true,
                ..Self::default()
            };
            return;
        }
        let State::Observing { epoch, next, .. } = &mut self.state else {
            return;
        };
        let withheld = match captured.drained {
            Some(Ok(drained)) if drained.epoch != *epoch => {
                "the production line observation restarted"
            }
            // A frame dropped after its drain, or an overflowed log, lost ticks.
            Some(Ok(drained)) if drained.first > *next => "production ticks were lost",
            Some(Ok(drained)) => {
                // Entries below `next` were accounted for, or came before the
                // branch began.
                let skip = (*next - drained.first) as usize;
                let entries = captured.handed_over();
                *next = (*next).max(drained.first + drained.copied as u64);
                self.complete = *next >= drained.cursor;
                for entry in entries.iter().skip(skip) {
                    add(&mut self.lines, entry, hour);
                }
                return;
            }
            Some(Err(reason)) => reason,
            None => "a frame drained no ticks",
        };
        self.state = State::Withheld(withheld.into());
    }

    fn publish(&mut self, hour: i32, lines: &TableInput) -> TableView {
        // A lost tick shows from the hour it is seen, not the next census.
        let since_hour = match &self.state {
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
                return TableView::Unavailable {
                    reason: replay::cadence::SKIPPED_REASON.into(),
                };
            }
            _ => return unavailable("ProductionLines is incomplete at this hour"),
        };
        if !self.complete {
            return unavailable("production ticks before this hour's sample are still in the log");
        }
        TableView::Complete(TableRows::ProductionLineOutput(
            self.rows(hour, since_hour, census),
        ))
    }

    fn rows(
        &mut self,
        hour: i32,
        since_hour: i32,
        census: &[ProductionLineRow],
    ) -> Vec<ProductionLineOutputRow> {
        let live = |handle: u64| {
            census
                .binary_search_by_key(&handle, |row| row.handle)
                .is_ok()
        };
        let (census_hour, baseline) = self
            .census
            .get_or_insert_with(|| (hour, census.iter().map(|row| row.handle).collect()));
        let census_hour = *census_hour;
        // Lines published as ended have had their publication once a written
        // frame held it; otherwise they are published as ended again.
        if std::mem::take(&mut self.publication_written) {
            self.lines.retain(|_, line| !line.ended);
        }
        for row in census {
            let line = self.lines.entry(row.handle).or_insert_with(|| Line {
                first_hour: hour,
                outputs: Vec::new(),
                ended: false,
            });
            // Every live line has a row, zero until it produces.
            if line.outputs.is_empty() {
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
        let mut rows = Vec::new();
        for (&handle, line) in &mut self.lines {
            line.ended = !live(handle);
            // Absent from the first census and first seen after it: created
            // after that census, so every tick of its life was observed.
            let observed_since_creation =
                baseline.binary_search(&handle).is_err() && line.first_hour > census_hour;
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
        rows
    }
}

fn add(lines: &mut BTreeMap<u64, Line>, entry: &Entry, hour: i32) {
    let line = lines.entry(entry.line).or_insert_with(|| Line {
        first_hour: hour,
        outputs: Vec::new(),
        ended: false,
    });
    let index = match line.outputs.iter().position(|output| {
        output.country == entry.country && output.variant == entry.variant && output.mio == entry.mio
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

fn unavailable(reason: &str) -> TableView {
    TableView::Unavailable {
        reason: reason.into(),
    }
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
