use super::output_continuity::OutputMap;
use super::output_continuity::{self, Progress, Reasons, State};
pub use super::output_continuity::{Receipt, holds_publication};
use crate::construction_ic::{Drained, Entry};
use crate::table_conversion::unavailable;
use engine_allocation::{collect_vec, mul, owned_vec, reserve_map};
use engine_production::construction_ic_sites::Identity;
use replay_data::{
    BranchId, rows::BuildingConstructionRow, rows::ConstructionCompletion,
    rows::ConstructionLineOutputRow, rows::RailwayConstructionRow, rows::Recorded, rows::TableId,
    rows::TableRows,
};
use replay_frame::{frame::TableInput, frame::TableView};
use std::{
    io,
    sync::atomic::{AtomicU64, Ordering},
};

static IDENTITY_CONFLICTS: AtomicU64 = AtomicU64::new(0);

pub fn identity_conflicts() -> u64 {
    IDENTITY_CONFLICTS.load(Ordering::Relaxed)
}

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ConstructionLineOutput];

pub const ASSEMBLED_BY_SPOOL: &str =
    "assembled by the spool from the construction output the hooks observed";

pub use super::output_continuity::Capacities;
pub struct Observation;
impl output_continuity::Observation for Observation {
    type Entry = Entry;
    const ASSEMBLED_BY_SPOOL: &'static str = ASSEMBLED_BY_SPOOL;
    /// # Safety
    /// Only the admitted hour's sole observer boundary (or an isolated test holding
    /// `construction_ic::test_lock()`) may drain the live construction-IC interval once while its
    /// storage remains live.
    unsafe fn drain(entries: &mut Vec<Entry>) -> Result<Drained, &'static str> {
        crate::construction_ic::drain(entries)
    }
}
pub type Captured = output_continuity::Captured<Observation>;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Line {
    first_hour: i32,
    railway: bool,
    identity: Option<Identity>,
    identity_conflicted: bool,
    construction: i64,
    bank: i64,
    carry: i64,
    repair: i64,
    instant: i64,
    instant_seen: bool,
    ticks: i32,
    repair_ticks: i32,
    linked: bool,
    unknown_share: bool,
    ended: Option<u64>,
    completion: Option<ConstructionCompletion>,
    count_reset: i64,
}

impl Line {
    fn observe_identity(&mut self, identity: Identity) {
        if self.identity_conflicted {
            return;
        }
        if self
            .identity
            .is_some_and(|held| held.country != identity.country)
        {
            self.identity = None;
            self.identity_conflicted = true;
            IDENTITY_CONFLICTS.fetch_add(1, Ordering::Relaxed);
        } else {
            self.identity = Some(identity);
        }
    }

    fn census_identity(&mut self, identity: Identity) {
        if !self.identity_conflicted {
            self.identity.get_or_insert(identity);
        }
    }
}

#[derive(Debug, Default)]
pub struct Totals {
    shared: output_continuity::Totals<Line>,
}

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
            if reason == replay_data::cadence::SKIPPED_REASON =>
        {
            Census::Skipped
        }
        _ => Census::Missing(missing),
    }
}

impl Totals {
    pub fn assemble(
        &mut self,
        branch: BranchId,
        hour: i32,
        captured: &Captured,
        tables: &mut [TableInput],
    ) {
        self.shared.retire(|line| line.ended);
        let observed = self.observe(branch, hour, captured);
        let carried = self.shared.take_held();
        if observed.is_err() {
            return;
        }
        let published = self.publish(
            hour,
            &tables[TableId::BuildingConstruction.index()],
            &tables[TableId::RailwayConstruction.index()],
        );
        self.shared.carry(&published, carried);
        tables[TableId::ConstructionLineOutput.index()] = TableInput::Sampled(published);
    }

    pub fn receipt(&self, output: &TableInput) -> Receipt {
        self.shared.receipt(output)
    }
    pub fn written(&mut self, receipt: Receipt) {
        self.shared.written(receipt);
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
                return unavailable(replay_data::cadence::SKIPPED_REASON);
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
            engine_allocation::add(buildings.len(), railways.len())?,
        )?;
        for row in buildings {
            live.insert(row.id, false);
        }
        for row in railways {
            live.insert(row.id, true);
        }
        let publication = self
            .shared
            .prepare_publication(hour, live.keys().copied())?;
        reserve_map(&mut self.shared.lines, live.len())?;
        let baseline = &self.shared.census;
        for (&handle, &railway) in &live {
            self.shared.lines.entry(handle).or_insert_with(|| Line {
                first_hour: hour,
                railway,
                ..Line::default()
            });
        }
        for row in buildings {
            self.shared
                .lines
                .get_mut(&row.id)
                .unwrap()
                .census_identity(Identity {
                    country: row.country,
                    building_type: row.building_type,
                    target_flags: row.target_flags,
                    target_location: row.target_location,
                    target_state: row.target_state,
                    target_country: row.target_country,
                    ..Identity::default()
                });
        }
        for row in railways {
            self.shared
                .lines
                .get_mut(&row.id)
                .unwrap()
                .census_identity(Identity {
                    country: row.country,
                    route_first: row.route.first().copied(),
                    route_last: row.route.last().copied(),
                    ..Identity::default()
                });
        }
        let mut rows = collect_vec(self.shared.lines.iter_mut().map(|(&handle, line)| {
            line.ended = if live.contains_key(&handle) {
                None
            } else {
                line.ended.or(Some(publication))
            };
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
                ended: line.ended.is_some(),
                completion: Recorded::Value(line.completion),
                count_reset: Recorded::Value(line.count_reset),
                country: Recorded::Value(line.identity.map(|identity| identity.country)),
                building_type: Recorded::Value(
                    line.identity
                        .filter(|_| !line.railway)
                        .map(|identity| identity.building_type),
                ),
                target_flags: Recorded::Value(
                    line.identity
                        .filter(|_| !line.railway)
                        .map(|identity| identity.target_flags),
                ),
                target_location: Recorded::Value(
                    line.identity
                        .filter(|_| !line.railway)
                        .map(|identity| identity.target_location),
                ),
                target_state: Recorded::Value(
                    line.identity
                        .filter(|_| !line.railway)
                        .map(|identity| identity.target_state),
                ),
                target_country: Recorded::Value(
                    line.identity
                        .filter(|_| !line.railway)
                        .map(|identity| identity.target_country),
                ),
                route_first: Recorded::Value(
                    line.identity
                        .filter(|_| line.railway)
                        .and_then(|identity| identity.route_first),
                ),
                route_last: Recorded::Value(
                    line.identity
                        .filter(|_| line.railway)
                        .and_then(|identity| identity.route_last),
                ),
            }
        }))?;
        rows.sort_unstable_by_key(|row| row.line);
        self.shared.published(publication);
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
    if let Some(identity) = entry.identity {
        line.observe_identity(identity);
    }
    line.ended = None;
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
    if let Some(count) = entry.count_reset {
        line.count_reset = line
            .count_reset
            .checked_add(i64::from(count))
            .ok_or("construction count reset total overflow")?;
        return Ok(());
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
        secondary.ended = None;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_entry() -> Entry {
        Entry {
            identity: Some(Identity {
                country: 5001,
                ..Identity::default()
            }),
            ..Entry::default()
        }
    }

    #[test]
    fn output_entries_publish_the_latest_target_and_void_conflicting_owners() {
        let conflicts = identity_conflicts();
        let mut totals = Totals::default();
        published(&mut totals, 100, &captured(0, &[]), census_of(&[], &[]));
        let mut entry = entry(LINE_A, 37);
        entry.identity = Some(Identity {
            country: 5001,
            building_type: 9,
            target_flags: 5,
            target_location: 2003,
            target_state: 1501,
            target_country: 0,
            ..Identity::default()
        });
        let mut latest = entry;
        latest.identity.as_mut().unwrap().target_state = 1502;
        let row = rows(published(
            &mut totals,
            124,
            &captured(0, &[entry, latest]),
            census_of(&[], &[]),
        ))
        .remove(0);
        assert_eq!(row.country, Recorded::Value(Some(5001)));
        assert_eq!(row.building_type, Recorded::Value(Some(9)));
        assert_eq!(row.target_flags, Recorded::Value(Some(5)));
        assert_eq!(row.target_location, Recorded::Value(Some(2003)));
        assert_eq!(row.target_state, Recorded::Value(Some(1502)));
        assert_eq!(row.target_country, Recorded::Value(Some(0)));
        assert_eq!(row.route_first, Recorded::Value(None));
        let mut changed = latest;
        changed.identity.as_mut().unwrap().country = 5002;
        let view = published(
            &mut totals,
            148,
            &captured(2, &[changed]),
            census_of(&[], &[]),
        );
        let row = rows(view).remove(0);
        assert_eq!(row.country, Recorded::Value(None));
        assert_eq!(row.target_state, Recorded::Value(None));
        assert_eq!(row.construction_raw, 111);
        assert!(row.exact);
        let row = rows(published(
            &mut totals,
            172,
            &captured(3, &[entry]),
            census_of(&[LINE_A], &[]),
        ))
        .remove(0);
        assert_eq!(row.country, Recorded::Value(None));
        assert_eq!(row.target_state, Recorded::Value(None));
        assert_eq!(row.construction_raw, 148);
        assert!(row.exact);
        assert!(identity_conflicts() > conflicts);
    }

    #[test]
    fn unread_identity_preserves_output_and_recovers_from_an_entry_or_census() {
        let mut totals = Totals::default();
        published(&mut totals, 100, &captured(0, &[]), census_of(&[], &[]));
        let unread = Entry {
            identity: None,
            ..entry(LINE_A, 37)
        };
        let row = rows(published(
            &mut totals,
            124,
            &captured(0, &[unread]),
            census_of(&[], &[]),
        ))
        .remove(0);
        for field in [
            row.country,
            row.building_type,
            row.target_flags,
            row.target_location,
            row.target_state,
            row.target_country,
            row.route_first,
            row.route_last,
        ] {
            assert_eq!(field, Recorded::Value(None));
        }
        assert_eq!(row.construction_raw, 37);
        assert!(row.exact);
        let second = Entry {
            line: LINE_B,
            ..unread
        };
        let output = rows(published(
            &mut totals,
            148,
            &captured(1, &[entry(LINE_A, 11), second]),
            census_of(&[LINE_B], &[]),
        ));
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].construction_raw, 48);
        assert_eq!(output[1].construction_raw, 37);
        assert!(
            output
                .iter()
                .all(|row| row.exact && row.country == Recorded::Value(Some(5001)))
        );
        let row = rows(published(
            &mut totals,
            172,
            &captured(3, &[unread]),
            census_of(&[LINE_A, LINE_B], &[]),
        ))
        .remove(0);
        assert_eq!(row.country, Recorded::Value(Some(5001)));
        assert_eq!(row.construction_raw, 85);
    }

    #[test]
    fn railway_output_publishes_its_own_route_endpoints() {
        let mut totals = Totals::default();
        let mut entry = test_entry();
        entry.line = RAIL;
        entry.railway = true;
        entry.identity.as_mut().unwrap().route_first = Some(120001);
        entry.identity.as_mut().unwrap().route_last = Some(120099);
        add(&mut totals.shared.lines, &entry, 100).unwrap();
        let row = totals.rows(124, 100, &[], &[]).unwrap().remove(0);
        assert_eq!(row.route_first, Recorded::Value(Some(120001)));
        assert_eq!(row.route_last, Recorded::Value(Some(120099)));
        assert_eq!(row.building_type, Recorded::Value(None));
    }

    #[test]
    fn native_count_resets_are_separate_from_completed_cost_debits() {
        let mut lines = OutputMap::default();
        add(
            &mut lines,
            &Entry {
                line: 19,
                amount: 1_000_350,
                ..Default::default()
            },
            100,
        )
        .unwrap();
        add(
            &mut lines,
            &Entry {
                line: 19,
                count_reset: Some(2),
                ..Default::default()
            },
            100,
        )
        .unwrap();
        let line = &lines[&19];
        assert_eq!(
            (line.construction, line.count_reset, line.ticks),
            (1_000_350, 2, 1)
        );
        assert_eq!(line.completion, None);
    }
    use replay_data::rows::TABLE_COUNT;

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
                ..test_entry()
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
        for line in [LINE_B, LINE_A] {
            add(
                &mut lines,
                &Entry {
                    line,
                    completion: Some(outcome),
                    linked_completion: line == LINE_B,
                    ..test_entry()
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
        let mut after_spend = Totals::default();
        add(
            &mut after_spend.shared.lines,
            &Entry {
                line: LINE_B,
                completion: Some(outcome),
                linked_completion: true,
                ..test_entry()
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
                reason: replay_data::cadence::SKIPPED_REASON.into(),
            });
        }
        tables
    }

    fn entry(line: u64, amount: i64) -> Entry {
        Entry {
            line,
            amount,
            ..test_entry()
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
        tables: Vec<TableInput>,
    ) -> TableView {
        queued(totals, 1, hour, captured, tables).0
    }

    fn queued(
        totals: &mut Totals,
        branch: BranchId,
        hour: i32,
        captured: &Captured,
        mut tables: Vec<TableInput>,
    ) -> (TableView, Receipt) {
        totals.assemble(branch, hour, captured, &mut tables);
        let receipt = totals.receipt(&tables[TableId::ConstructionLineOutput.index()]);
        let view = match std::mem::replace(
            &mut tables[TableId::ConstructionLineOutput.index()],
            TableInput::Sampled(TableView::Unavailable {
                reason: String::new(),
            }),
        ) {
            TableInput::Sampled(view) => view,
            _ => unreachable!(),
        };
        (view, receipt)
    }

    fn rows(view: TableView) -> Vec<ConstructionLineOutputRow> {
        match view {
            TableView::Complete(TableRows::ConstructionLineOutput(rows)) => rows,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn totals_split_by_kind_and_publish_with_the_census() {
        let mut totals = Totals::default();
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
        let (ended, receipt) = queued(
            &mut totals,
            1,
            48,
            &captured(6, &[]),
            census_of(&[LINE_B], &[RAIL]),
        );
        assert!(
            rows(ended)
                .iter()
                .find(|row| row.line == LINE_A)
                .unwrap()
                .ended
        );
        totals.written(receipt);
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
        let (output, receipt) = queued(
            &mut totals,
            1,
            24,
            &captured(0, &[spend]),
            census_of(&[LINE_A, LINE_B], &[]),
        );
        let output = rows(output);
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
        totals.written(receipt);
        let ended = rows(published(
            &mut totals,
            48,
            &captured(1, &[]),
            census_of(&[], &[]),
        ));
        assert!(ended.iter().all(|row| row.ended && row.linked));
        assert!(!ended[1].exact);
    }

    #[test]
    fn lost_output_withholds_and_an_incomplete_hour_is_unavailable() {
        let mut totals = Totals::default();
        published(&mut totals, 0, &captured(0, &[]), census_of(&[LINE_A], &[]));
        assert!(matches!(
            published(&mut totals, 1, &captured(0, &[]), skipped()),
            TableView::Unavailable { reason } if reason == replay_data::cadence::SKIPPED_REASON
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
                matches!(published(&mut totals, 1, &lost, skipped()), TableView::Unavailable {reason} if reason != replay_data::cadence::SKIPPED_REASON)
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
        let (again, receipt) = queued(&mut totals, 1, 25, &captured(1, &[]), census_of(&[], &[]));
        assert_eq!(rows(again), final_row);
        totals.written(receipt);
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

    #[test]
    fn a_late_write_retires_only_the_ends_its_publication_showed() {
        let mut totals = Totals::default();
        published(
            &mut totals,
            0,
            &captured(0, &[]),
            census_of(&[LINE_A, LINE_B], &[]),
        );
        let (first, ended_a) = queued(
            &mut totals,
            1,
            24,
            &captured(0, &[]),
            census_of(&[LINE_B], &[]),
        );
        assert!(rows(first)[0].ended);
        let (second, _lost_b) = queued(&mut totals, 1, 48, &captured(0, &[]), census_of(&[], &[]));
        assert!(rows(second).iter().all(|row| row.ended));
        totals.written(ended_a);
        let third = rows(published(
            &mut totals,
            72,
            &captured(0, &[]),
            census_of(&[], &[]),
        ));
        assert_eq!(
            third
                .iter()
                .map(|row| (row.line, row.ended))
                .collect::<Vec<_>>(),
            [(LINE_B, true)]
        );
        let (fourth, held_b) = queued(&mut totals, 1, 96, &captured(0, &[]), census_of(&[], &[]));
        assert_eq!(rows(fourth).len(), 1);
        totals.written(held_b);
        assert!(
            rows(published(
                &mut totals,
                120,
                &captured(0, &[]),
                census_of(&[], &[])
            ))
            .is_empty()
        );
    }

    #[test]
    fn output_after_an_unwritten_end_survives_its_late_write() {
        let mut totals = Totals::default();
        published(&mut totals, 0, &captured(0, &[]), census_of(&[LINE_A], &[]));
        let (ended, receipt) = queued(&mut totals, 1, 24, &captured(0, &[]), census_of(&[], &[]));
        assert_eq!(rows(ended)[0].construction_raw, 0);
        let late = rows(published(
            &mut totals,
            25,
            &captured(0, &[entry(LINE_A, 9)]),
            census_of(&[], &[]),
        ));
        totals.written(receipt);
        let after = rows(published(
            &mut totals,
            48,
            &captured(1, &[]),
            census_of(&[], &[]),
        ));
        assert_eq!(after, late);
        assert_eq!((after[0].construction_raw, after[0].ended), (9, true));
    }

    #[test]
    fn an_old_branch_write_never_retires_the_new_branch_lines() {
        let mut totals = Totals::default();
        published(&mut totals, 0, &captured(0, &[]), census_of(&[LINE_A], &[]));
        let (_, old) = queued(&mut totals, 1, 24, &captured(0, &[]), census_of(&[], &[]));
        queued(
            &mut totals,
            2,
            30,
            &captured(0, &[]),
            census_of(&[LINE_A], &[]),
        );
        let (ended, _) = queued(&mut totals, 2, 54, &captured(0, &[]), census_of(&[], &[]));
        assert!(rows(ended)[0].ended);
        totals.written(old);
        let (again, _) = queued(&mut totals, 2, 78, &captured(0, &[]), census_of(&[], &[]));
        assert_eq!(
            rows(again)
                .iter()
                .map(|row| (row.line, row.ended))
                .collect::<Vec<_>>(),
            [(LINE_A, true)]
        );
    }
}
