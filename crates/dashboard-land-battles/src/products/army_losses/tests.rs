use super::*;
use replay_data::rows::*;

const ARMY_A: u64 = 7 << 32 | 53;
const ARMY_B: u64 = 8 << 32 | 53;
const GONE: u64 = 6 << 32 | 53;
const FIELD_MARSHAL: u64 = 20 << 32 | 53;

/// GER (raw 11) keeps logs for armies 7 and 8 (8 has no army group), for an
/// army that no longer exists, and for its army group; another country (raw 99,
/// unlinked) keeps one more.
fn frame() -> DecodedFrame {
    let mut f = dashboard_base::test_fixtures::source(1, 60_000_001, None);
    for id in TABLES {
        if !matches!(id, TableId::Countries | TableId::CountryLinks) {
            *f.tables[id.index()] = TableView::Complete(TableRows::empty(id));
        }
    }
    *f.tables[TableId::CombatLogGroups.index()] = TableView::Complete(TableRows::CombatLogGroups(vec![
        CombatLogGroupRow { country: 11, group: GONE, ordinal: 2 },
        CombatLogGroupRow { country: 11, group: ARMY_A, ordinal: 0 },
        CombatLogGroupRow { country: 11, group: ARMY_B, ordinal: 1 },
        CombatLogGroupRow { country: 11, group: FIELD_MARSHAL, ordinal: 3 },
        CombatLogGroupRow { country: 99, group: 9 << 32 | 53, ordinal: 0 },
    ]));
    *f.tables[TableId::OrdersGroups.index()] = TableView::Complete(TableRows::OrdersGroups(vec![
        OrdersGroupRow { handle: ARMY_A, country: 11, parent: FIELD_MARSHAL, ..Default::default() },
        OrdersGroupRow { handle: ARMY_B, country: 11, name: Recorded::Value(None), ..Default::default() },
        OrdersGroupRow { handle: FIELD_MARSHAL, country: 11, is_army_group: 1, ..Default::default() },
    ]));
    *f.tables[TableId::CombatLogManpower.index()] = TableView::Complete(TableRows::CombatLogManpower(vec![
        CombatLogManpowerRow { country: 11, group: 6, ordinal: 0, date: 60_000_000, losses_0: 999, ..Default::default() },
        CombatLogManpowerRow { country: 11, group: 7, ordinal: 0, date: 59_000_000, losses_0: i32::MIN, losses_1: -7, losses_2: i32::MAX, ..Default::default() },
        CombatLogManpowerRow { country: 11, group: 8, ordinal: 0, date: 60_000_000, losses_0: 1_200, losses_1: 30, losses_2: 0, ..Default::default() },
        CombatLogManpowerRow { country: 11, group: 20, ordinal: 0, date: 60_000_000, losses_0: 5_000, ..Default::default() },
        CombatLogManpowerRow { country: 99, group: 9, ordinal: 0, date: 60_000_000, losses_0: 5, ..Default::default() },
    ]));
    *f.tables[TableId::CombatLogMonths.index()] = TableView::Complete(TableRows::CombatLogMonths(vec![
        CombatLogMonthRow { country: 11, group: 7, kind: COMBAT_LEDGER, ordinal: 0, date: 60_000_000, ..Default::default() },
        CombatLogMonthRow { country: 11, group: 7, kind: RECOVERED_LEDGER, ordinal: 0, date: 60_000_000, ..Default::default() },
        CombatLogMonthRow { country: 11, group: 7, kind: ENEMY_LEDGER, ordinal: 0, date: 60_000_000, ..Default::default() },
    ]));
    *f.tables[TableId::CombatLogEquipment.index()] = TableView::Complete(TableRows::CombatLogEquipment(vec![
        CombatLogEquipmentRow { country: 11, group: 7, kind: COMBAT_LEDGER, month: month_key(60_000_000), variant: 3, date: 60_000_000, amount_raw: -150_000 },
    ]));
    *f.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(vec![read(11, 60_000_001)]));
    *f.tables[TableId::CombatLogEnemyEquipment.index()] = TableView::Complete(TableRows::CombatLogEnemyEquipment(vec![
        CombatLogEnemyEquipmentRow { country: 11, group: 7, month: 0, ordinal: 0, variant: 4 << 32 | 70, amount_raw: 200_000 },
    ]));
    f
}

/// The hour `owner`'s Combat Log equipment was read at.
fn read(owner: u32, sample_hour: i32) -> SliceReadRow {
    SliceReadRow { table: TableId::CombatLogEquipment.index() as u32, owner, sample_hour }
}

fn reply(frame: &DecodedFrame, tag: &str) -> ArmyLossesReply {
    reply_with(frame, tag, &mut Earlier::default())
}

fn reply_with(frame: &DecodedFrame, tag: &str, history: &mut dyn ledgers::History) -> ArmyLossesReply {
    build(frame, tag, None, None, history, &AtomicBool::new(false)).unwrap()
}

/// Earlier reads as the rows each entry's latest read held; `complete` says
/// whether every possible read was recorded. Records what was asked.
#[derive(Default)]
struct Earlier {
    held: BTreeMap<(u32, ledgers::Entry), Vec<CombatLogEquipmentRow>>,
    complete: bool,
    asked: Vec<ledgers::Wanted>,
}
impl ledgers::History for Earlier {
    fn latest(&mut self, wanted: &[ledgers::Wanted]) -> io::Result<Vec<ledgers::Lookup>> {
        self.asked.extend_from_slice(wanted);
        Ok(wanted
            .iter()
            .map(|&(owner, entry)| match self.held.get(&(owner, entry)) {
                Some(rows) => ledgers::Lookup::Found(ledgers::Witness::Rows(rows.clone())),
                None => ledgers::Lookup::Unwitnessed { complete: self.complete },
            })
            .collect())
    }
}

fn own(month: i32, amount_raw: i64) -> CombatLogEquipmentRow {
    CombatLogEquipmentRow { country: 11, group: 7, kind: COMBAT_LEDGER, month: month_key(month), variant: 3, date: month, amount_raw }
}

/// Army 7's combat ledger holds three months, a thousand hours apart (each
/// its own calendar month); its country's latest read, a windowed one, holds
/// only the last two.
const MONTHS: [i32; 3] = [59_997_000, 59_998_000, 59_999_000];
fn windowed() -> DecodedFrame {
    let mut f = frame();
    let TableView::Complete(TableRows::CombatLogMonths(rows)) = &mut *f.tables[TableId::CombatLogMonths.index()] else { unreachable!() };
    rows.retain(|row| row.kind != COMBAT_LEDGER);
    for (ordinal, date) in MONTHS.into_iter().enumerate() {
        rows.push(CombatLogMonthRow { country: 11, group: 7, kind: COMBAT_LEDGER, ordinal: ordinal as u32, date, ..Default::default() });
    }
    rows.sort_by_key(|row| (row.country, row.group, row.kind, row.ordinal));
    *f.tables[TableId::CombatLogEquipment.index()] =
        TableView::Complete(TableRows::CombatLogEquipment(vec![own(MONTHS[1], 222), own(MONTHS[2], 333)]));
    f
}

fn combat(r: &ArmyLossesReply) -> Vec<(i32, &str)> {
    let mut rows: Vec<_> = r.equipment.iter().filter(|row| row.ledger == COMBAT_LEDGER).map(|row| (row.date, row.amount_raw.as_str())).collect();
    rows.sort_unstable();
    rows
}

#[test]
fn armies_join_their_logs_by_reference_and_army_groups_or_deleted_armies_are_refused() {
    let r = reply(&frame(), "GER");
    assert!(!r.absent);
    assert_eq!(r.armies.iter().map(|a| a.reference.clone()).collect::<Vec<_>>(), [ARMY_A.to_string(), ARMY_B.to_string()]);
    assert_eq!(r.armies[0].parent.as_deref(), Some(FIELD_MARSHAL.to_string().as_str()), "the current army group, even though it keeps no log");
    assert!(r.armies[1].parent.is_none() && r.armies[1].parent_missing.is_none(), "an army with no army group");
    assert!(r.armies[1].name.is_none() && r.armies[1].name_missing.is_some());
    assert_eq!(r.manpower.len(), 2, "rows of refused logs and another country's never join");
    assert!(r.warnings.iter().any(|w| w.contains("no longer exists")));
    assert!(r.warnings.iter().any(|w| w.contains("army group is left out")));
    assert!(r.warnings.iter().any(|w| w.contains("rows of a log that was left out (2)")));
}

#[test]
fn months_keep_signed_channels_the_game_calendar_month_and_each_ledger_apart() {
    let r = reply(&frame(), "GER");
    let a = r.manpower.iter().find(|row| row.army == ARMY_A.to_string()).unwrap();
    assert_eq!(a.losses, [i32::MIN, -7, i32::MAX], "signed counters are kept as stored");
    assert_eq!(a.month, month_bucket(59_000_000));
    assert_eq!(r.equipment.len(), 2);
    let own = r.equipment.iter().find(|row| row.ledger == COMBAT_LEDGER).unwrap();
    assert_eq!(own.amount_raw, "-150000");
    assert_eq!(own.variant, ((3u64 << 32) | u64::from(COMBAT_LOG_VARIANT_TYPE)).to_string());
    let dealt = r.equipment.iter().find(|row| row.ledger == ENEMY_LEDGER).unwrap();
    assert_eq!((dealt.month, dealt.amount_raw.as_str()), (month_bucket(60_000_000), "200000"));
    assert_eq!(r.empty_months.len(), 1, "a recovered month without amounts is kept apart, not as zero");
    assert_eq!(r.empty_months[0].ledger, RECOVERED_LEDGER);
    assert!(r.equipment.iter().all(|row| row.name.is_none()), "names need the content stack");
}

#[test]
fn an_absent_country_and_unread_tables_are_named_not_empty_months() {
    assert!(reply(&frame(), "FRA").absent);
    let mut f = frame();
    *f.tables[TableId::CombatLogManpower.index()] = TableView::Unavailable { reason: "attachment interval".into() };
    let r = reply(&f, "GER");
    assert!(r.manpower.is_empty());
    assert_eq!(r.availability.manpower.as_deref(), Some("attachment interval"));
    assert!(r.availability.armies.is_none());
    let mut f = frame();
    *f.tables[TableId::OrdersGroups.index()] = TableView::Unavailable { reason: "gap".into() };
    let r = reply(&f, "GER");
    assert!(r.armies.is_empty() && r.manpower.is_empty() && r.availability.armies.is_some(), "no army is guessed without the armies");
}

#[test]
fn the_product_reads_the_frame_from_a_recording() {
    let source = Source::from_replay(
        dashboard_base::source::SourceKey { source_id: "army-losses".into(), revision: "1".into() },
        "army-losses.battleplan".into(),
        Arc::new(dashboard_base::test_fixtures::replay(vec![frame()])),
        &AtomicBool::new(false),
    )
    .unwrap();
    let r = ArmyLosses::load(
        &source,
        &ArmyLossesParams { branch_id: "1".into(), hour: 60_000_001, tag: "GER".into() },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!((r.branch_id.as_str(), r.hour, r.manpower.len()), ("1", 60_000_001, 2));
}

#[test]
fn an_older_month_that_left_a_windowed_read_keeps_its_latest_witnessed_amounts() {
    let oldest = (7, COMBAT_LEDGER, MONTHS[0]);
    let mut history = Earlier::default();
    history.held.insert((11, oldest), vec![own(MONTHS[0], 111)]);
    let r = reply_with(&windowed(), "GER", &mut history);
    assert_eq!(history.asked, [(11, oldest)], "only the month the latest read could not hold is looked up");
    assert_eq!(combat(&r), [(MONTHS[0], "111"), (MONTHS[1], "222"), (MONTHS[2], "333")]);
    assert!(r.unknown_months.is_empty() && r.empty_months.iter().all(|month| month.ledger != COMBAT_LEDGER));
}

#[test]
fn an_older_month_no_read_held_is_unknown_never_zero() {
    let r = reply(&windowed(), "GER");
    assert_eq!(combat(&r), [(MONTHS[1], "222"), (MONTHS[2], "333")]);
    assert_eq!(r.unknown_months.iter().map(|month| month.date).collect::<Vec<_>>(), [MONTHS[0]]);
    assert!(r.empty_months.iter().all(|month| month.date != MONTHS[0]), "a month that left the read is not empty");
    assert!(r.warnings.iter().any(|w| w.contains("not all recorded")), "{:?}", r.warnings);
    let r = reply_with(&windowed(), "GER", &mut Earlier { complete: true, ..Earlier::default() });
    assert_eq!(r.unknown_months.iter().map(|month| month.date).collect::<Vec<_>>(), [MONTHS[0]]);
    assert!(r.warnings.iter().any(|w| w.contains("no read covered")), "{:?}", r.warnings);
}

#[test]
fn a_month_is_empty_only_when_the_latest_read_after_it_covered_it_without_rows() {
    let r = reply(&windowed(), "GER");
    assert!(r.empty_months.iter().any(|month| month.ledger == RECOVERED_LEDGER),
        "the latest read holds every array's last two months");
    let mut f = frame();
    *f.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(vec![read(11, 59_999_999)]));
    let r = reply(&f, "GER");
    assert!(r.empty_months.is_empty(), "a month first written after the latest read is not known empty");
    assert!(r.unknown_months.iter().any(|month| month.ledger == RECOVERED_LEDGER));
}

/// The recording starts with a full read holding `full`; the country's next,
/// windowed read, at its fourth hour, holds only the last two months.
fn recording_start_with(full: Vec<CombatLogEquipmentRow>) -> Vec<DecodedFrame> {
    let windowed_rows = vec![own(MONTHS[1], 222), own(MONTHS[2], 333)];
    (0..5)
        .map(|offset| {
            let mut f = windowed();
            f.hour = 60_000_001 + offset;
            let (sample, rows) = if offset < 3 { (60_000_001, full.clone()) } else { (60_000_004, windowed_rows.clone()) };
            *f.tables[TableId::CombatLogEquipment.index()] = TableView::Complete(TableRows::CombatLogEquipment(rows));
            *f.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(vec![read(11, sample)]));
            f
        })
        .collect()
}

fn recording_start() -> Vec<DecodedFrame> {
    recording_start_with(vec![own(MONTHS[0], 111), own(MONTHS[1], 200), own(MONTHS[2], 300)])
}

fn load_at(frames: Vec<DecodedFrame>, hour: i32) -> ArmyLossesReply {
    let source = Source::from_replay(
        dashboard_base::source::SourceKey { source_id: "army-losses-start".into(), revision: "1".into() },
        "army-losses-start.battleplan".into(),
        Arc::new(dashboard_base::test_fixtures::replay(frames)),
        &AtomicBool::new(false),
    )
    .unwrap();
    ArmyLosses::load(
        &source,
        &ArmyLossesParams { branch_id: "1".into(), hour, tag: "GER".into() },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap()
}

#[test]
fn the_recordings_full_first_read_witnesses_a_month_later_windowed_reads_drop() {
    let r = load_at(recording_start(), 60_000_005);
    assert_eq!(combat(&r), [(MONTHS[0], "111"), (MONTHS[1], "222"), (MONTHS[2], "333")], "{:?}", r.warnings);
}

#[test]
fn an_older_month_the_full_first_read_covered_without_rows_stays_known_empty() {
    let r = load_at(recording_start_with(vec![own(MONTHS[1], 200), own(MONTHS[2], 300)]), 60_000_005);
    assert!(r.empty_months.iter().any(|month| month.ledger == COMBAT_LEDGER && month.date == MONTHS[0]), "{:?}", r.warnings);
    assert!(r.unknown_months.is_empty(), "{:?}", r.unknown_months);
}

fn january() -> ledgers::Entry {
    (7, COMBAT_LEDGER, MONTHS[0])
}
fn wanted(entry: ledgers::Entry) -> [ledgers::Wanted; 1] {
    [(11, entry)]
}
fn amounts(witness: ledgers::Witness) -> Vec<i64> {
    match witness {
        ledgers::Witness::Rows(rows) => rows.iter().map(|row| row.amount_raw).collect(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_blocks_index_keeps_the_latest_covering_read_and_its_unread_hours() {
    let entry = |month| (7, COMBAT_LEDGER, month);
    let whole = ledgers::Provenance::read(&recording_start(), 60_000_001, 60_000_005, true);
    assert!(whole.unread_hours().is_empty());
    let found = |provenance: &ledgers::Provenance, month| provenance.covering(11, entry(month)).map(|(hour, witness)| (hour, amounts(witness)));
    assert_eq!(found(&whole, MONTHS[0]), Some((60_000_001, vec![111])), "the full read's month the windowed read dropped");
    assert_eq!(found(&whole, MONTHS[1]), Some((60_000_004, vec![222])));
    let early = ledgers::Provenance::read(&recording_start(), 60_000_001, 60_000_003, true);
    assert_eq!(found(&early, MONTHS[1]), Some((60_000_001, vec![200])), "indexed to an hour, never a read after it");
    let empty = ledgers::Provenance::read(&recording_start_with(vec![own(MONTHS[1], 200)]), 60_000_001, 60_000_005, true);
    assert_eq!(empty.covering(11, entry(MONTHS[0])), Some((60_000_001, ledgers::Witness::Empty)),
        "a full read covering a month without rows is an explicit empty witness");
    let windowed_only = ledgers::Provenance::read(&recording_start_with(vec![own(MONTHS[1], 200)]), 60_000_001, 60_000_005, false);
    assert_eq!(windowed_only.covering(11, entry(MONTHS[0])), None, "a windowed read never covers an older month");
    assert_eq!(whole.covering(11, entry(MONTHS[0] + 1)), None, "a later entry of the same month is another entry");
    let mut unread = recording_start();
    *unread[1].tables[TableId::SliceReads.index()] = TableView::Unavailable { reason: "not read".into() };
    assert_eq!(ledgers::Provenance::read(&unread, 60_000_001, 60_000_005, true).unread_hours(), [(60_000_002, 60_000_002)]);
    let mut gapped = recording_start();
    gapped.remove(2);
    assert_eq!(ledgers::Provenance::read(&gapped, 60_000_001, 60_000_005, true).unread_hours(), [(60_000_003, 60_000_003)]);
}

fn block(frames: &[DecodedFrame], first: i32, last: i32, full_start: bool) -> io::Result<((i32, i32), Arc<ledgers::Provenance>)> {
    Ok(((first, last), Arc::new(ledgers::Provenance::read(frames, first, last, full_start))))
}

#[test]
fn an_older_witness_needs_every_later_hour_read_in_its_block() {
    let frames = recording_start();
    let found = ledgers::fold([block(&frames, 60_000_001, 60_000_005, true)], &wanted(january())).unwrap();
    assert_eq!(found, [ledgers::Lookup::Found(ledgers::Witness::Rows(vec![own(MONTHS[0], 111)]))]);
    let mut unread = recording_start();
    *unread[2].tables[TableId::CombatLogEquipment.index()] = TableView::Unavailable { reason: "slice gap".into() };
    assert_eq!(ledgers::fold([block(&unread, 60_000_001, 60_000_005, true)], &wanted(january())).unwrap(),
        [ledgers::Lookup::Unwitnessed { complete: false }], "an unread hour after the witness hides any later read");
    assert_eq!(ledgers::fold([block(&unread, 60_000_001, 60_000_002, true)], &wanted(january())).unwrap(), found,
        "an unread hour after the reached hour does not matter");
}

/// The full first read, an unread hour, then the country read in full again
/// with the same amounts, then windowed reads that leave January out.
fn reread(full: Vec<CombatLogEquipmentRow>, gap: bool) -> Vec<DecodedFrame> {
    let mut frames = recording_start_with(full.clone());
    *frames[2].tables[TableId::CombatLogEquipment.index()] = TableView::Complete(TableRows::CombatLogEquipment(full));
    *frames[2].tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(vec![read(11, 60_000_003)]));
    if gap {
        frames.remove(1);
    } else {
        *frames[1].tables[TableId::SliceReads.index()] = TableView::Unavailable { reason: "not read".into() };
    }
    frames
}

#[test]
fn an_identical_reread_after_an_unread_hour_witnesses_a_month_again() {
    // A full read holding January's rows is full: its identical rows count again.
    let frames = reread(vec![own(MONTHS[0], 111), own(MONTHS[1], 200), own(MONTHS[2], 300)], false);
    assert_eq!(ledgers::fold([block(&frames, 60_000_001, 60_000_005, true)], &wanted(january())).unwrap(),
        [ledgers::Lookup::Found(ledgers::Witness::Rows(vec![own(MONTHS[0], 111)]))]);
    // The hour after a gap is a full hour: January read empty again stays known empty.
    let frames = reread(vec![own(MONTHS[1], 200), own(MONTHS[2], 300)], true);
    assert_eq!(ledgers::fold([block(&frames, 60_000_001, 60_000_005, true)], &wanted(january())).unwrap(),
        [ledgers::Lookup::Found(ledgers::Witness::Empty)]);
}

#[test]
fn an_older_witness_needs_every_newer_block_read_and_no_hour_between_blocks_missing() {
    let frames = recording_start();
    let older = || block(&frames[..3], 60_000_001, 60_000_003, true);
    let found = ledgers::fold([block(&frames[3..], 60_000_004, 60_000_005, false), older()], &wanted(january())).unwrap();
    assert_eq!(found, [ledgers::Lookup::Found(ledgers::Witness::Rows(vec![own(MONTHS[0], 111)]))]);
    let mut newer = frames[3..].to_vec();
    *newer[1].tables[TableId::SliceReads.index()] = TableView::Unavailable { reason: "not read".into() };
    assert_eq!(ledgers::fold([block(&newer, 60_000_004, 60_000_005, false), older()], &wanted(january())).unwrap(),
        [ledgers::Lookup::Unwitnessed { complete: false }], "an unread hour in a newer block");
    assert_eq!(ledgers::fold([block(&frames[4..], 60_000_005, 60_000_005, false), older()], &wanted(january())).unwrap(),
        [ledgers::Lookup::Unwitnessed { complete: false }], "an hour missing between the blocks");
}

#[test]
fn one_backward_walk_settles_every_month_and_stops_once_they_are() {
    let frames = recording_start();
    let both = [(11, january()), (11, (7, COMBAT_LEDGER, MONTHS[1]))];
    let mut pulled = 0;
    let blocks = (0..2).map(|index| {
        pulled += 1;
        assert_eq!(index, 0, "an older block is never indexed once every month is settled");
        block(&frames, 60_000_001, 60_000_005, true)
    });
    let settled = ledgers::fold(blocks, &both).unwrap();
    assert_eq!(settled, [ledgers::Lookup::Found(ledgers::Witness::Rows(vec![own(MONTHS[0], 111)])),
        ledgers::Lookup::Found(ledgers::Witness::Rows(vec![own(MONTHS[1], 222)]))]);
    assert_eq!(pulled, 1, "the walk pulls no block after the one settling the last month");
    let mut pulled = 0;
    let blocks = std::iter::from_fn(|| {
        pulled += 1;
        None::<io::Result<((i32, i32), Arc<ledgers::Provenance>)>>
    });
    assert!(ledgers::fold(blocks, &[]).unwrap().is_empty());
    assert_eq!(pulled, 0, "nothing asked reads nothing");
    let none = ledgers::fold([block(&frames[3..], 60_000_004, 60_000_005, false)], &[(11, (7, COMBAT_LEDGER, 60_000_004))]).unwrap();
    assert_eq!(none, [ledgers::Lookup::Unwitnessed { complete: true }], "no read covered it since its first write");
}

#[test]
fn unread_own_or_dealt_pools_leave_their_months_unknown_not_empty() {
    let mut f = frame();
    *f.tables[TableId::CombatLogEquipment.index()] = TableView::Unavailable { reason: "slice gap".into() };
    *f.tables[TableId::CombatLogEnemyEquipment.index()] = TableView::Unavailable { reason: "attachment interval".into() };
    let r = reply(&f, "GER");
    assert!(r.empty_months.is_empty() && r.equipment.is_empty());
    assert_eq!(r.unknown_months.len(), 3);
    assert!(r.availability.own_equipment.is_some() && r.availability.enemy_equipment.is_some());
    let mut f = frame();
    *f.tables[TableId::SliceReads.index()] = TableView::Unavailable { reason: "not read".into() };
    let r = reply(&f, "GER");
    assert!(r.availability.own_equipment.is_some(), "own amounts cannot be dated without their read hours");
    assert!(r.empty_months.iter().all(|month| month.ledger == ENEMY_LEDGER));
}

#[test]
fn own_rows_of_a_month_the_game_no_longer_keeps_are_left_out() {
    let mut f = frame();
    *f.tables[TableId::CombatLogEquipment.index()] = TableView::Complete(TableRows::CombatLogEquipment(vec![
        own(58_000_000, 999),
        own(60_000_000, -150_000),
    ]));
    let r = reply(&f, "GER");
    assert_eq!(combat(&r), [(60_000_000, "-150000")], "a pruned month's lingering rows never return");
    assert!(r.warnings.iter().any(|w| w.contains("no longer keeps are left out (1)")), "{:?}", r.warnings);
}

#[test]
fn an_entry_written_again_in_the_same_month_never_takes_its_deleted_predecessors_rows() {
    // A retention below one deleted the entry read at the country's latest
    // read; a later loss that month wrote a new one.
    let recreated = |read_hour| {
        let mut f = frame();
        let TableView::Complete(TableRows::CombatLogMonths(months)) = &mut *f.tables[TableId::CombatLogMonths.index()] else {
            unreachable!()
        };
        months[0].date = 60_000_002;
        assert_eq!(month_key(60_000_002), month_key(60_000_000), "the same calendar month");
        *f.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(vec![read(11, read_hour)]));
        reply(&f, "GER")
    };
    let r = recreated(60_000_001);
    assert!(combat(&r).is_empty(), "{:?}", r.equipment);
    assert!(r.unknown_months.iter().any(|month| month.ledger == COMBAT_LEDGER && month.date == 60_000_002));
    assert!(r.warnings.iter().any(|w| w.contains("written after their country's latest read")), "{:?}", r.warnings);
    assert!(r.warnings.iter().any(|w| w.contains("no longer keeps are left out (1)")), "{:?}", r.warnings);
    // Rows dated otherwise than a read after the entry's first write witness nothing.
    let r = recreated(60_000_002);
    assert!(combat(&r).is_empty() && !r.empty_months.iter().any(|month| month.ledger == COMBAT_LEDGER));
    assert!(r.warnings.iter().any(|w| w.contains("another entry of the same month")), "{:?}", r.warnings);
}

/// `frame` as a row frame leaving `tables` to the native capture of a full recording.
fn left_to_native(mut frame: DecodedFrame, tables: &[TableId]) -> DecodedFrame {
    // Full leaves DivisionModelInputs out, so a frame in its mode marks it so.
    for &table in tables.iter().chain(&[TableId::DivisionModelInputs]) {
        *frame.tables[table.index()] = TableView::NotRecorded { mode: replay_data::scope::CaptureMode::Full };
    }
    frame
}

#[test]
fn an_hour_left_to_the_native_capture_is_read_from_its_index_never_shown_as_unrecorded() {
    let cut = left_to_native(frame(), &[TableId::CombatLogGroups, TableId::CombatLogManpower]);
    let (source, root) = dashboard_base::test_fixtures::file_source(dashboard_base::test_fixtures::replay(vec![cut]));
    let error = ArmyLosses::load(
        &source,
        &ArmyLossesParams { branch_id: "1".into(), hour: 60_000_001, tag: "GER".into() },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap_err()
    .to_string();
    drop(source);
    std::fs::remove_dir_all(root).ok();
    assert_eq!(
        error,
        "The recording holds [CombatLogGroups, CombatLogManpower] in its native capture, whose change-run index is not prepared yet"
    );
}

#[test]
fn a_walk_reaching_a_block_left_to_the_native_capture_leaves_its_months_unprepared_never_unread() {
    let native: Vec<_> = recording_start()
        .into_iter()
        .map(|frame| left_to_native(frame, &[TableId::CombatLogEquipment, TableId::CombatLogMonths, TableId::SliceReads]))
        .collect();
    let rows = recording_start();
    let later = (7, COMBAT_LEDGER, 60_000_004);
    let found = ledgers::fold(
        [block(&rows[3..], 60_000_004, 60_000_005, false), block(&native[..3], 60_000_001, 60_000_003, true)],
        &[(11, january()), (11, later)],
    )
    .unwrap();
    assert_eq!(found, [ledgers::Lookup::Unprepared, ledgers::Lookup::Unwitnessed { complete: true }],
        "a month first written after the native block is settled by the row blocks after it");
    assert_eq!(ledgers::fold([block(&native, 60_000_001, 60_000_005, true)], &wanted(january())).unwrap(),
        [ledgers::Lookup::Unprepared]);
}

/// Earlier reads held only by the native capture.
struct Native;
impl ledgers::History for Native {
    fn latest(&mut self, wanted: &[ledgers::Wanted]) -> io::Result<Vec<ledgers::Lookup>> {
        Ok(vec![ledgers::Lookup::Unprepared; wanted.len()])
    }
}

#[test]
fn an_older_month_only_the_native_capture_holds_is_unknown_and_named_unprepared() {
    let r = reply_with(&windowed(), "GER", &mut Native);
    assert_eq!(combat(&r), [(MONTHS[1], "222"), (MONTHS[2], "333")], "the latest read's months stand");
    assert_eq!(r.unknown_months.iter().map(|month| month.date).collect::<Vec<_>>(), [MONTHS[0]]);
    assert!(r.warnings.iter().any(|w| w.contains("not prepared from native recordings yet (1)")), "{:?}", r.warnings);
    assert!(!r.warnings.iter().any(|w| w.contains("not all recorded")), "{:?}", r.warnings);
}
