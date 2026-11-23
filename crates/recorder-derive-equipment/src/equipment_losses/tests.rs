use super::*;
use recorder_derive_framework::framework::Writers;
use replay_columnar::{
    state::Kind as Column,
    store::{Store, Value as Cell},
    unit::Discard,
};
use replay_data::rows::{
    CombatLogEquipmentRow, CombatLogMonthRow, CountryLinkRow, CountryRow, EquipmentDestructionCause, EquipmentDestructionRow, Field, Kind, LendLeaseEquipmentRow, Opaque, RowCodec, SunkConvoyRow, SunkShipsRow, TABLE_COUNT,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

type Op = Box<dyn FnOnce(&mut Rows) + Send>;

struct Feed(Arc<Mutex<Vec<Op>>>);

const FED: &[Output] = &[
    Output::State(TableId::Countries),
    Output::State(TableId::CountryLinks),
    Output::State(TableId::EquipmentDestruction),
    Output::State(TableId::CombatLogEquipment),
    Output::State(TableId::SunkShips),
    Output::State(TableId::SunkConvoys),
    Output::State(TableId::LendLeaseEquipment),
    Output::State(TableId::DivisionCensus),
    Output::State(TableId::CombatLogState),
    Output::State(TableId::Players),
    Output::State(TableId::SimulationFeatures),
];

impl Writer for Feed {
    fn name(&self) -> &'static str {
        "Feed"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        FED
    }

    fn reads(&self) -> &'static [Read] {
        &[]
    }

    fn hourly(&self) -> bool {
        true
    }

    fn start(&mut self) {}

    fn hour(&mut self, _: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        for op in self.0.lock().unwrap().drain(..) {
            op(out);
        }
        Ok(())
    }

    fn save(&self, _: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
}

fn column(kind: Kind) -> Column {
    match kind {
        Kind::U8 => Column::U8,
        Kind::I8 => Column::I8,
        Kind::U16 => Column::U16,
        Kind::I16 => Column::I16,
        Kind::U32 => Column::U32,
        Kind::I32 => Column::I32,
        Kind::U64 => Column::U64,
        Kind::I64 => Column::I64,
        Kind::Bool => Column::Bool,
        Kind::OptionalI32 => Column::OptionalI32,
        Kind::OptionalU32 => Column::OptionalU32,
        Kind::OptionalI64 => Column::OptionalI64,
        Kind::OptionalU64 => Column::OptionalU64,
        Kind::Bytes => Column::Bytes,
        Kind::U32List => Column::U32List,
        Kind::U64List => Column::U64List,
        Kind::Opaque => Column::Opaque,
    }
}

fn store() -> Store {
    let kinds = TableId::ALL.iter().map(|table| table.schema().iter().map(|&kind| column(kind)).collect()).collect();
    let mut store = Store::new(kinds, vec![false; TABLE_COUNT]);
    store.begin_run(0);
    store
}

fn set<R: RowCodec + Send + 'static>(table: TableId, row: R) -> Op {
    Box::new(move |out: &mut Rows| {
        let mut cells = Vec::new();
        row.visit_wire(&mut |_, field| {
            cells.push(match field {
                Field::NotRecorded => Value::Unrecorded,
                Field::U8(value) => Value::U8(value),
                Field::I8(value) => Value::I8(value),
                Field::U16(value) => Value::U16(value),
                Field::I16(value) => Value::I16(value),
                Field::U32(value) => Value::U32(value),
                Field::I32(value) => Value::I32(value),
                Field::U64(value) => Value::U64(value),
                Field::I64(value) => Value::I64(value),
                Field::Bool(value) => Value::Bool(value),
                Field::OptionalI32(value) => Value::OptionalI32(value),
                Field::OptionalU32(value) => Value::OptionalU32(value),
                Field::OptionalI64(value) => Value::OptionalI64(value),
                Field::OptionalU64(value) => Value::OptionalU64(value),
                Field::Bytes(text) => out.text(text),
                Field::Opaque(Opaque::Encoded(bytes, _)) => out.opaque(bytes.to_vec()),
                Field::Opaque(Opaque::Native(value)) => {
                    let mut bytes = Vec::new();
                    value.encode_value(&mut bytes)?;
                    out.opaque(bytes)
                }
                Field::Empty | Field::U32List(_) | Field::U64List(_) => return Err(invalid("a list field in a test row")),
            });
            Ok(())
        })
        .expect("a test row's cells");
        out.set(table, row.key(), cells);
    })
}

fn gone(table: TableId, key: u128) -> Op {
    Box::new(move |out: &mut Rows| out.delete(table, key))
}

fn unavailable(table: TableId, reason: &'static str) -> Op {
    Box::new(move |out: &mut Rows| out.unavailable(table, reason.as_bytes()))
}

const GER: u32 = 1;
const ENG: u32 = 2;
const GER_ALIAS: u32 = 11;
const STRAY: u32 = 99;

fn handle(variant: u32) -> u64 {
    u64::from(variant) << 32 | u64::from(engine_native::cequipment_variant::REFERENCE_TYPE)
}

fn countries() -> Vec<Op> {
    let mut ops = Vec::new();
    for id in [GER, ENG] {
        ops.push(set(TableId::Countries, CountryRow { id, canonical_slot: id, tag: b"TAG".to_vec(), ..Default::default() }));
        ops.push(set(TableId::CountryLinks, CountryLinkRow { id, canonical_slot: id }));
    }
    ops.push(set(TableId::CountryLinks, CountryLinkRow { id: GER_ALIAS, canonical_slot: GER }));
    ops
}

fn drained(country: u32, variant: u32, amount: i64) -> Op {
    set(
        TableId::EquipmentDestruction,
        EquipmentDestructionRow {
            country,
            cause: EquipmentDestructionCause::LandCombat,
            variant: handle(variant),
            amount_raw: amount,
            coverage: Recorded::Value(EquipmentDestructionCoverage::Complete),
            unavailable_reason: Recorded::Value(String::new()),
        },
    )
}

fn partial_drain(reason: &'static str) -> Op {
    set(
        TableId::EquipmentDestruction,
        EquipmentDestructionRow {
            coverage: Recorded::Value(EquipmentDestructionCoverage::PartialInterval),
            unavailable_reason: Recorded::Value(reason.to_owned()),
            ..Default::default()
        },
    )
}

fn drain_key(country: u32, variant: u32) -> u128 {
    EquipmentDestructionRow { country, cause: EquipmentDestructionCause::LandCombat, variant: handle(variant), ..Default::default() }.key()
}

fn logged(country: u32, group: u32, month: u16, variant: u32, amount: i64) -> Op {
    set(TableId::CombatLogEquipment, CombatLogEquipmentRow { country, group, kind: 0, month, variant, date: i32::from(month) * 10, amount_raw: amount })
}

fn log_key(country: u32, group: u32, month: u16, variant: u32) -> u128 {
    CombatLogEquipmentRow { country, group, kind: 0, month, variant, ..Default::default() }.key()
}

fn convoys(owner: u32, month: i32, count: i32) -> Op {
    set(TableId::SunkConvoys, SunkConvoyRow { month, killer_country: ENG, owner_country: owner, convoys: count, ordinal: 0 })
}

fn pool(giver: u32, exchange: u32, variant: u32, amount: i64) -> Op {
    set(TableId::LendLeaseEquipment, LendLeaseEquipmentRow { giver, exchange, kind: 0, variant, receiver: ENG, amount_raw: amount })
}

fn ships(entries: Vec<SunkShipEntry>) -> Op {
    set(TableId::SunkShips, SunkShipsRow { entries })
}

fn ship(name: &[u8], sink_hour: i32) -> SunkShipEntry {
    SunkShipEntry { ship_name: name.to_vec(), victim_country: GER, sink_hour, victim_equipment_variant: handle(7), ..Default::default() }
}

struct Run {
    store: Store,
    writers: Writers,
    feed: Arc<Mutex<Vec<Op>>>,
    hour: i32,
    start: i32,
}

fn writers(feed: &Arc<Mutex<Vec<Op>>>) -> Writers {
    let mut writers = Writers::new(vec![Box::new(Feed(Arc::clone(feed))), Box::new(EquipmentLosses::default())]);
    writers.start_run();
    writers
}

impl Run {
    fn new() -> Self {
        let feed = Arc::new(Mutex::new(Vec::new()));
        Self { store: store(), writers: writers(&feed), feed, hour: 0, start: 0 }
    }

    fn step(&mut self, ops: Vec<Op>) {
        self.feed.lock().unwrap().extend(ops);
        self.store.begin_hour(self.hour, &[]).unwrap();
        self.writers.hour(1, self.hour, self.hour == self.start, &mut self.store).unwrap();
        self.store.end_hour(&mut Discard).unwrap();
        self.hour += 1;
    }

    fn resume(&mut self) {
        let saved = self.writers.save().unwrap();
        let mut store = store();
        store.resume(self.store.tables().to_vec(), self.store.strings().next()).unwrap();
        let mut writers = writers(&self.feed);
        writers.load(&saved, &mut store).unwrap();
        writers.prime(&store, 1, self.hour - 1);
        (self.store, self.writers) = (store, writers);
    }

    fn cell(&self, table: TableId, key: u128, column: usize) -> Option<Cell<'_>> {
        let view = self.store.view(table.index());
        view.slot(key).map(|slot| view.value(column, slot))
    }

    fn total(&self, slot: u32, id: u32, source: u8, variant: Option<u32>) -> Option<Cell<'_>> {
        self.cell(TableId::EquipmentLossTotals, leaf(slot, id, source, variant), 0)
    }

    fn run_cell(&self, column: usize) -> Option<Cell<'_>> {
        self.cell(TableId::EquipmentLossCoverage, RUN_KEY, column)
    }

    fn drain(&self) -> String {
        let view = self.store.view(TableId::EquipmentLossCoverage.index());
        let slot = view.slot(RUN_KEY).expect("the run's row");
        String::from_utf8_lossy(view.string(view.text(4, slot).expect("a reason")).expect("its text")).into_owned()
    }

    fn status(&self, table: TableId) -> Status {
        self.store.view(table.index()).status()
    }

    fn dump(&self) -> Vec<(TableId, u128, Vec<String>)> {
        let mut rows = Vec::new();
        for table in TABLES.iter().map(|output| output.table()) {
            let view = self.store.view(table.index());
            for slot in view.rows() {
                let cells = (0..view.kinds().len())
                    .map(|column| match view.value(column, slot) {
                        Cell::Text(id) => format!("{:?}", view.string(id)),
                        other => format!("{other:?}"),
                    })
                    .collect();
                rows.push((table, view.key(slot).unwrap(), cells));
            }
        }
        rows.sort();
        rows
    }
}

fn number(value: i64) -> Option<Cell<'static>> {
    Some(Cell::Number(value as u64))
}

const LAND: u8 = Total::DESTRUCTION + EquipmentDestructionCause::LandCombat as u8;

#[test]
fn equal_consecutive_drains_are_two_events_and_the_partial_startup_claims_nothing() {
    let mut run = Run::new();
    run.writers.set_format(EQUIPMENT_DESTRUCTION_STARTUP_FORMAT - 1);
    let mut first = countries();
    first.push(drained(GER, 7, 400));
    first.push(unavailable(TableId::EquipmentDestruction, "equipment destruction first/branch interval is partial"));
    run.step(first);
    assert_eq!(run.total(GER, GER, LAND, Some(7)), None, "the partial startup interval claims nothing");
    assert_eq!(run.run_cell(6), Some(Cell::Missing), "nothing counted yet");
    assert_eq!(run.run_cell(7), Some(Cell::Missing), "historical partial startup is never withheld");
    assert_eq!(run.run_cell(15), number(i64::from(Coverage::DRAIN_STATE_PARTIAL_INTERVAL)));
    run.step(vec![drained(GER, 7, 5)]);
    assert_eq!(run.total(GER, GER, LAND, Some(7)), number(5));
    assert_eq!(run.run_cell(15), number(i64::from(Coverage::DRAIN_STATE_COMPLETE)));
    run.step(Vec::new());
    assert_eq!(run.total(GER, GER, LAND, Some(7)), number(10), "an unchanged drain read again is a second drain");
    run.step(vec![gone(TableId::EquipmentDestruction, drain_key(GER, 7)), drained(ENG, 7, 3)]);
    assert_eq!(run.total(GER, GER, LAND, Some(7)), number(10));
    assert_eq!(run.total(ENG, ENG, LAND, Some(7)), number(3));
    assert_eq!(run.run_cell(6), number(1), "counted from the first complete drain");
    assert_eq!(run.drain(), "");
}

#[test]
fn unavailable_legacy_partial_text_withholds_the_run_at_current_format() {
    let mut run = Run::new();
    let mut first = countries();
    first.push(unavailable(TableId::EquipmentDestruction, "equipment destruction first/branch interval is partial"));
    run.step(first);
    assert_eq!(run.total(GER, GER, LAND, Some(7)), None);
    assert_eq!(run.run_cell(6), Some(Cell::Missing));
    assert_eq!(run.run_cell(7), number(0), "at format 145+, Status::Unavailable always withholds");
    assert_eq!(run.run_cell(15), number(i64::from(Coverage::DRAIN_STATE_WITHHELD)));
    assert_eq!(run.drain(), "equipment destruction first/branch interval is partial");

    run.step(vec![drained(GER, 7, 5)]);
    assert_eq!(run.total(GER, GER, LAND, Some(7)), None, "a withheld run never resumes counting");
    assert_eq!(run.run_cell(6), Some(Cell::Missing));
    assert_eq!(run.run_cell(7), number(0));
    assert_eq!(run.run_cell(15), number(i64::from(Coverage::DRAIN_STATE_WITHHELD)));
}

#[test]
fn structured_partial_intervals_with_reworded_text_never_withhold_the_run() {
    let mut run = Run::new();
    let mut first = countries();
    first.push(partial_drain("reworded startup coverage wording"));
    run.step(first);
    assert_eq!(run.total(GER, GER, LAND, Some(7)), None, "startup partial interval claims nothing");
    assert_eq!(run.run_cell(6), Some(Cell::Missing), "nothing counted yet");
    assert_eq!(run.run_cell(7), Some(Cell::Missing), "startup partial interval is not withheld");
    assert_eq!(run.run_cell(8), Some(Cell::Missing), "no lower bound before counting starts");
    assert_eq!(run.run_cell(15), number(i64::from(Coverage::DRAIN_STATE_PARTIAL_INTERVAL)));
    assert_eq!(run.drain(), "reworded startup coverage wording");

    run.step(vec![gone(TableId::EquipmentDestruction, 0), drained(GER, 7, 5)]);
    assert_eq!(run.total(GER, GER, LAND, Some(7)), number(5));
    assert_eq!(run.run_cell(6), number(1));
    assert_eq!(run.run_cell(7), Some(Cell::Missing));
    assert_eq!(run.run_cell(8), Some(Cell::Missing));
    assert_eq!(run.run_cell(15), number(i64::from(Coverage::DRAIN_STATE_COMPLETE)));
    assert_eq!(run.drain(), "");

    run.step(vec![gone(TableId::EquipmentDestruction, drain_key(GER, 7)), partial_drain("reworded branch transition wording")]);
    assert_eq!(run.total(GER, GER, LAND, Some(7)), number(5), "branch partial interval keeps prior total as a lower bound");
    assert_eq!(run.run_cell(6), number(1));
    assert_eq!(run.run_cell(7), Some(Cell::Missing), "branch partial interval does not withhold the run");
    assert_eq!(run.run_cell(8), number(2), "lower bound begins at the branch partial interval");
    assert_eq!(run.run_cell(15), number(i64::from(Coverage::DRAIN_STATE_PARTIAL_INTERVAL)));
    assert_eq!(run.drain(), "reworded branch transition wording");

    run.step(vec![gone(TableId::EquipmentDestruction, 0), drained(GER, 7, 3)]);
    assert_eq!(run.total(GER, GER, LAND, Some(7)), number(8), "complete drains after a branch partial interval keep counting");
    assert_eq!(run.run_cell(7), Some(Cell::Missing));
    assert_eq!(run.run_cell(8), number(2));
    assert_eq!(run.run_cell(15), number(i64::from(Coverage::DRAIN_STATE_COMPLETE)));

    run.step(vec![unavailable(TableId::EquipmentDestruction, "drain overflowed")]);
    assert_eq!(run.total(GER, GER, LAND, Some(7)), Some(Cell::Unrecorded), "a refused drain withholds the run");
    assert_eq!(run.run_cell(7), number(4));
    assert_eq!(run.run_cell(15), number(i64::from(Coverage::DRAIN_STATE_WITHHELD)));
    assert_eq!(run.drain(), "drain overflowed");
}

#[test]
fn a_partial_interval_marker_mixed_with_loss_rows_is_refused() {
    let mut run = Run::new();
    let mut first = countries();
    first.push(partial_drain("reworded startup coverage wording"));
    first.push(drained(GER, 7, 5));
    run.step(first);
    let Status::Unavailable { reason } = run.status(TableId::EquipmentLossCoverage) else {
        panic!("expected unavailable status for mixed partial marker and loss rows");
    };
    let view = run.store.view(TableId::EquipmentLossCoverage.index());
    let text = String::from_utf8_lossy(view.string(reason).unwrap_or_default());
    assert!(text.contains("partial interval marker"), "{text}");
}

#[test]
fn a_refused_drain_withholds_the_drain_leaves_for_the_run_and_nothing_else() {
    let mut run = Run::new();
    let mut first = countries();
    first.push(drained(GER, 7, 5));
    first.push(convoys(GER, 1, 2));
    run.step(first);
    run.step(vec![convoys(GER, 1, 3)]);
    assert_eq!(run.total(GER, GER, LAND, Some(7)), number(10));
    run.step(vec![unavailable(TableId::EquipmentDestruction, "drain overflowed")]);
    assert_eq!(run.total(GER, GER, LAND, Some(7)), Some(Cell::Unrecorded), "a withheld drain's total is not known, never kept as if complete");
    assert_eq!(run.run_cell(7), number(2));
    assert_eq!(run.drain(), "drain overflowed");
    run.step(vec![convoys(GER, 1, 4)]);
    assert_eq!(run.total(GER, GER, LAND, Some(7)), Some(Cell::Unrecorded), "withheld for the rest of the run");
    assert_eq!(run.total(GER, GER, Total::SUNK_CONVOYS, None), number(2 * AMOUNT_SCALE), "other sources go on");
    assert_eq!(run.status(TableId::EquipmentLossTotals), Status::Complete);
}

#[test]
fn an_owner_no_link_resolves_makes_the_drain_a_lower_bound_from_then() {
    let mut run = Run::new();
    let mut first = countries();
    first.push(drained(GER, 7, 5));
    run.step(first);
    assert_eq!(run.run_cell(8), Some(Cell::Missing));
    run.step(vec![drained(STRAY, 7, 2)]);
    assert_eq!(run.drain(), OWNER_UNKNOWN);
    assert_eq!(run.run_cell(8), number(1));
    assert_eq!((run.run_cell(13), run.run_cell(14)), (number(1), number(1)), "one raw country no link resolves, counted for both land modes");
    assert_eq!(run.total(GER, GER, LAND, Some(7)), number(10), "known owners still count");
    run.step(vec![gone(TableId::EquipmentDestruction, drain_key(STRAY, 7))]);
    assert_eq!(run.drain(), "");
    assert_eq!(run.run_cell(8), number(1), "the first uncertain hour stays");
}

#[test]
fn rows_the_game_deletes_keep_their_amount_and_a_returning_or_aliased_entry_counts_once() {
    let mut run = Run::new();
    let mut first = countries();
    first.push(logged(GER, 4, 100, 7, 3));
    run.step(first);
    fn combat(run: &Run) -> Option<Cell<'_>> {
        run.total(GER, GER, Total::COMBAT_LOG, Some(7))
    }
    assert_eq!(combat(&run), number(3), "a run's first read holds what the log lists");
    run.step(vec![logged(GER, 4, 100, 7, 5)]);
    assert_eq!(combat(&run), number(5));
    run.step(vec![gone(TableId::CombatLogEquipment, log_key(GER, 4, 100, 7)), logged(GER, 4, 101, 7, 1)]);
    assert_eq!(combat(&run), number(6), "a deleted entry keeps its last amount");
    assert_eq!(run.cell(TableId::CombatLogDropped, log_key(GER, 4, 100, 7), 1), number(5));
    assert_eq!(run.cell(TableId::CombatLogDropped, log_key(GER, 4, 100, 7), 0), number(1000), "with its month entry's first write, which army losses matches entries by");
    run.step(vec![logged(GER, 4, 100, 7, 5)]);
    assert_eq!(combat(&run), number(6), "a returning entry counts once");
    assert_eq!(run.cell(TableId::CombatLogDropped, log_key(GER, 4, 100, 7), 1), None);
    run.step(vec![logged(GER_ALIAS, 4, 101, 7, 2)]);
    assert_eq!(combat(&run), number(7), "one entry under two aliases counts once, its highest alias standing");
    run.step(vec![gone(TableId::CombatLogEquipment, log_key(GER_ALIAS, 4, 101, 7))]);
    assert_eq!(combat(&run), number(7), "the alias's entry kept as dropped still stands over the other");
}

fn sunk_key(slot: u32, country: u32, name: &[u8], sunk: i32) -> (u32, u32, u64) {
    (slot, country, signature(&ship(name, sunk), SIGNATURE_SEED) & ShipLossRow::SIGNATURE)
}

impl Run {
    fn ship_log(&self) -> BTreeMap<(u32, u32, u64), (i64, i64, i64)> {
        let view = self.store.view(TableId::ShipLosses.index());
        view.rows()
            .map(|slot| {
                let key = view.key(slot).unwrap();
                assert_eq!((key >> 48) as u32 as i32, self.hour - 1, "a row lives the hour it was read");
                let number = |column| view.number(column, slot).unwrap();
                assert_eq!(number(1) as u64, handle(7));
                (((key >> 104) as u32, (key >> 80) as u32 & ShipLossRow::FIELD, key as u64 & ShipLossRow::SIGNATURE), (number(0), number(2), number(3)))
            })
            .collect()
    }
}

#[test]
fn sunk_ships_count_the_most_one_read_listed_at_their_sinking_hour() {
    let hood = SunkShipEntry { victim_country: STRAY, ..ship(b"Hood", -10) };
    let alias = SunkShipEntry { victim_country: GER_ALIAS, ..ship(b"Graf", -3) };
    let mut first = countries();
    first.push(ships(vec![ship(b"Bismarck", -40), ship(b"Graf", -3), ship(b"Graf", -3), hood.clone()]));
    let hours = vec![
        first,
        vec![ships(vec![ship(b"Graf", -3), hood.clone()])],
        vec![ships(vec![ship(b"Graf", -3), ship(b"Graf", -3), alias, hood])],
        vec![set(TableId::CountryLinks, CountryLinkRow { id: STRAY, canonical_slot: ENG })],
        vec![unavailable(TableId::SunkShips, "ships not read")],
        Vec::new(),
    ];
    let (bismarck, graf, sunk_hood) = (sunk_key(GER, GER, b"Bismarck", -40), sunk_key(GER, GER, b"Graf", -3), sunk_key(ENG, ENG, b"Hood", -10));
    let mut run = Run::new();
    let mut folded: BTreeMap<(u32, u32, u64), (i64, i64, i64)> = BTreeMap::new();
    let mut logs = Vec::new();
    let mut tables = Vec::new();
    for ops in hours {
        run.step(ops);
        let log = run.ship_log();
        for (&key, &(sunk, count, check)) in &log {
            let held = folded.entry(key).or_insert((sunk, 0, check));
            assert_eq!((held.0, held.2), (sunk, check), "one signature, one ship");
            held.1 = held.1.max(count);
        }
        logs.push(log.into_iter().map(|(key, (sunk, count, _))| (key, sunk, count)).collect::<Vec<_>>());
        tables.push(folded.iter().map(|(&key, &(sunk, count, _))| (key, sunk, count)).collect::<Vec<_>>());
    }
    let sorted = |mut rows: Vec<((u32, u32, u64), i64, i64)>| {
        rows.sort_unstable();
        rows
    };
    let start = sorted(vec![(bismarck, -40, 1), (graf, -3, 2)]);
    let both = sorted(vec![(graf, -3, 3), (sunk_hood, -10, 1)]);
    assert_eq!(logs, [start.clone(), vec![], vec![(graf, -3, 3)], vec![(sunk_hood, -10, 1)], vec![], both], "the first hour, a rise, a link resolving, a read after one not complete");
    let risen = sorted(vec![(bismarck, -40, 1), (graf, -3, 3)]);
    let later = sorted(vec![(bismarck, -40, 1), (graf, -3, 3), (sunk_hood, -10, 1)]);
    assert_eq!(tables, [start.clone(), start, risen, later.clone(), later.clone(), later], "the reply's fold is the table the log replaced");
}

#[test]
fn convoys_and_lend_lease_count_from_their_first_read() {
    let mut run = Run::new();
    let mut first = countries();
    first.push(convoys(GER, 1, 5));
    first.push(pool(GER, 30, 7, 100));
    run.step(first);
    assert_eq!(run.total(GER, GER, Total::SUNK_CONVOYS, None), None, "a run's first read is its baseline");
    assert_eq!(run.run_cell(9), number(0));
    run.step(vec![convoys(GER, 1, 7), pool(GER, 30, 7, 150)]);
    assert_eq!(run.total(GER, GER, Total::SUNK_CONVOYS, None), number(2 * AMOUNT_SCALE));
    assert_eq!(run.total(GER, GER, Total::LEND_LEASE, Some(7)), number(50));
    run.step(vec![convoys(GER, 1, 1)]);
    assert_eq!(run.total(GER, GER, Total::SUNK_CONVOYS, None), number(3 * AMOUNT_SCALE), "a count that fell counts whole as a new entry");
    assert_eq!(run.cell(TableId::EquipmentLossCoverage, identity_key(GER, GER), 3), number(1));
    run.step(vec![gone(TableId::LendLeaseEquipment, LendLeaseEquipmentRow { giver: GER, exchange: 30, kind: 0, variant: 7, ..Default::default() }.key())]);
    assert_eq!(run.total(GER, GER, Total::LEND_LEASE, Some(7)), number(50));
    assert_eq!(run.cell(TableId::EquipmentLossTotals, leaf(GER, GER, Total::LEND_LEASE, Some(7)), 1), number(3), "an ended exchange: a lower bound from the hour after the read before");
    let coverage = identity_key(GER, GER);
    assert_eq!((run.cell(TableId::EquipmentLossCoverage, coverage, 11), run.cell(TableId::EquipmentLossCoverage, coverage, 12)), (number(1), number(3)), "one exchange ended, lost from hour 3");
}

#[test]
fn hours_without_countries_are_not_run_and_lose_their_drains_by_name() {
    let mut run = Run::new();
    let mut first = countries();
    first.push(drained(GER, 7, 5));
    run.step(first);
    run.step(vec![unavailable(TableId::Countries, "countries failed")]);
    assert!(matches!(run.status(TableId::EquipmentLossTotals), Status::Unavailable { .. }));
    assert_eq!(run.total(GER, GER, LAND, Some(7)), number(5), "rows stay as they were");
    run.step(Vec::new());
    assert_eq!(run.total(GER, GER, LAND, Some(7)), number(10));
    assert_eq!(run.run_cell(8), number(1), "the lost hour's drain makes the drain a lower bound from it");
    assert_eq!(run.drain(), LOST);
    assert_eq!(run.run_cell(15), number(i64::from(Coverage::DRAIN_STATE_LOST)));
}

#[test]
fn a_lost_drain_keeps_its_bit_and_reason_when_the_next_hour_also_flags_an_unresolved_owner() {
    let mut run = Run::new();
    let mut first = countries();
    first.push(drained(GER, 7, 5));
    run.step(first);
    run.step(vec![unavailable(TableId::Countries, "countries failed")]);
    run.step(vec![drained(STRAY, 7, 2)]);
    assert_eq!(run.run_cell(8), number(1));
    assert_eq!(run.run_cell(15), number(i64::from(Coverage::DRAIN_STATE_LOST | Coverage::DRAIN_STATE_OWNER_UNKNOWN)));
    assert_eq!(run.drain(), format!("{LOST}; {OWNER_UNKNOWN}"));
}

#[test]
fn resuming_after_a_lost_and_withheld_hour_drops_the_transient_lost_prefix() {
    let steps = || -> Vec<Vec<Op>> {
        let mut first = countries();
        first.push(drained(GER, 7, 5));
        vec![
            first,
            vec![unavailable(TableId::Countries, "countries failed")],
            vec![unavailable(TableId::EquipmentDestruction, "hooks unavailable")],
            vec![drained(GER, 7, 3)],
        ]
    };
    let mut unbroken = Run::new();
    for ops in steps() {
        unbroken.step(ops);
    }
    assert_eq!(unbroken.run_cell(15), number(i64::from(Coverage::DRAIN_STATE_WITHHELD)));
    assert_eq!(unbroken.drain(), "hooks unavailable");

    let mut resumed = Run::new();
    for (at, ops) in steps().into_iter().enumerate() {
        resumed.step(ops);
        if at == 2 {
            assert_eq!(resumed.run_cell(15), number(i64::from(Coverage::DRAIN_STATE_LOST | Coverage::DRAIN_STATE_WITHHELD)));
            assert_eq!(resumed.drain(), format!("{LOST}; hooks unavailable"));
            resumed.resume();
        }
    }
    assert_eq!(resumed.run_cell(15), number(i64::from(Coverage::DRAIN_STATE_WITHHELD)));
    assert_eq!(resumed.drain(), "hooks unavailable");
    assert_eq!(resumed.dump(), unbroken.dump());
}

#[test]
fn a_resume_in_the_middle_leaves_the_tables_a_run_never_stopped_leaves() {
    let hours = || -> Vec<Vec<Op>> {
        let mut first = countries();
        first.extend([drained(GER, 7, 5), logged(GER, 4, 100, 7, 3), convoys(GER, 1, 5), pool(GER, 30, 7, 100), ships(vec![ship(b"Graf", -3)])]);
        vec![
            first,
            vec![logged(GER, 4, 100, 7, 4), convoys(GER, 1, 6), pool(GER, 30, 7, 120), drained(STRAY, 7, 1), ships(vec![ship(b"Graf", -3), ship(b"Graf", -3)])],
            vec![gone(TableId::CombatLogEquipment, log_key(GER, 4, 100, 7)), convoys(GER, 1, 8)],
            vec![logged(GER, 4, 100, 7, 4), pool(GER, 30, 7, 130), drained(ENG, 7, 2), ships(vec![ship(b"Graf", -3), ship(b"Graf", -3), ship(b"Bismarck", -40)])],
            vec![gone(TableId::LendLeaseEquipment, LendLeaseEquipmentRow { giver: GER, exchange: 30, kind: 0, variant: 7, ..Default::default() }.key()), convoys(GER, 1, 2)],
        ]
    };
    let mut unbroken = Run::new();
    let mut logged_unbroken = Vec::new();
    for ops in hours() {
        unbroken.step(ops);
        logged_unbroken.push(unbroken.ship_log());
    }
    let mut stopped = Run::new();
    let mut logged_stopped = Vec::new();
    for (at, ops) in hours().into_iter().enumerate() {
        if at == 2 {
            stopped.resume();
        }
        stopped.step(ops);
        logged_stopped.push(stopped.ship_log());
    }
    assert_eq!(stopped.dump(), unbroken.dump());
    assert_eq!(logged_stopped, logged_unbroken, "the sunk ships each hour logs");
    assert_eq!(logged_unbroken[3].keys().copied().collect::<Vec<_>>(), [sunk_key(GER, GER, b"Bismarck", -40)], "compared with the read before the resume");
}

#[test]
fn land_coverage_follows_the_census_of_an_admitted_country() {
    assert_eq!(land_coverage(false, 0, 0, true), (Coverage::UNKNOWN, None), "no census row is not known");
    assert_eq!(land_coverage(true, 0, 4, true), (Coverage::NOT_LOGGED, None), "no division in an orders group logs nothing");
    assert_eq!(land_coverage(true, 3, 4, true), (Coverage::PARTIAL, Some(4)));
    assert_eq!(land_coverage(true, 3, 0, false), (Coverage::PRUNE_UNOBSERVED, None));
    assert_eq!(land_coverage(true, 3, 0, true), (Coverage::LOGGED, None));
}

fn land<R: RowCodec>(store: &mut Store, table: TableId, row: &R) {
    use replay_columnar::state::Cell as Put;
    let mut cells: Vec<Option<Put<'static>>> = Vec::new();
    row.visit_wire(&mut |_, field| {
        cells.push(match field {
            Field::NotRecorded => None,
            Field::U8(value) => Some(Put::U8(value)),
            Field::I8(value) => Some(Put::I8(value)),
            Field::U16(value) => Some(Put::U16(value)),
            Field::I16(value) => Some(Put::I16(value)),
            Field::U32(value) => Some(Put::U32(value)),
            Field::I32(value) => Some(Put::I32(value)),
            Field::U64(value) => Some(Put::U64(value)),
            Field::I64(value) => Some(Put::I64(value)),
            Field::Bool(value) => Some(Put::Bool(value)),
            Field::OptionalI32(value) => Some(Put::OptionalI32(value)),
            Field::OptionalU32(value) => Some(Put::OptionalU32(value)),
            Field::OptionalI64(value) => Some(Put::OptionalI64(value)),
            Field::OptionalU64(value) => Some(Put::OptionalU64(value)),
            Field::Bytes(text) => Some(Put::Bytes(store.intern(text).unwrap())),
            _ => return Err(invalid("a list or opaque field in a landed test row")),
        });
        Ok(())
    })
    .expect("a landed row's cells");
    let mut changes = store.table(table.index()).unwrap();
    let slot = changes.insert(row.key()).unwrap();
    for (column, cell) in cells.into_iter().enumerate() {
        if let Some(cell) = cell {
            changes.write(slot, column, cell).unwrap();
        }
    }
}

const HELD: [TableId; 3] = [TableId::Countries, TableId::CountryLinks, TableId::EquipmentDestruction];

fn through_journal(journal: &mut replay_columnar::store::journal::Journal, store: &mut Store, hour: i32, read: &[TableId]) {
    let mut statuses = vec![Status::NotRecorded; TABLE_COUNT];
    for table in HELD {
        statuses[table.index()] = Status::Complete;
    }
    let mut bits = vec![0_u64; TABLE_COUNT.div_ceil(64)];
    for table in read {
        bits[table.index() / 64] |= 1 << (table.index() % 64);
    }
    let manifest = replay_columnar::manifest::Manifest {
        branch: 1,
        hour,
        flags: 0,
        total_ns: 0,
        pool_bytes: 0,
        dropped_frames: 0,
        appends: Vec::new(),
        statuses,
        read: bits,
        samples: Vec::new(),
        sections: Vec::new(),
    };
    journal.begin_hour(store, &manifest).unwrap();
    journal.applied(store);
}

#[test]
fn a_drain_counts_at_every_hour_that_read_it_and_at_none_that_did_not() {
    let mut store = store();
    let mut writers = Writers::new(vec![Box::new(EquipmentLosses::default())]);
    writers.start_run();
    let derived: Vec<usize> = TABLES.iter().map(|output| output.table().index()).collect();
    let mut journal = replay_columnar::store::journal::Journal::leaving(&derived, Default::default());
    store.begin_hour(0, &[]).unwrap();
    for table in HELD {
        store.status(table.index(), Status::Complete, true, Some(0), true).unwrap();
    }
    for id in [GER, ENG] {
        land(&mut store, TableId::Countries, &CountryRow { id, canonical_slot: id, tag: b"TAG".to_vec(), ..Default::default() });
        land(&mut store, TableId::CountryLinks, &CountryLinkRow { id, canonical_slot: id });
    }
    let row = EquipmentDestructionRow {
        country: GER,
        cause: EquipmentDestructionCause::LandCombat,
        variant: handle(7),
        amount_raw: 5,
        coverage: Recorded::Value(EquipmentDestructionCoverage::Complete),
        unavailable_reason: Recorded::Value(String::new()),
    };
    land(&mut store, TableId::EquipmentDestruction, &row);
    store.settle();
    writers.hour(1, 0, true, &mut store).unwrap();
    store.end_hour(&mut Discard).unwrap();
    let drain = TableId::EquipmentDestruction.index();
    let run_hour = |store: &Store, column: usize| -> Option<Option<i64>> {
        let held = store.view(TableId::EquipmentLossCoverage.index());
        let slot = held.slot(RUN_KEY)?;
        match held.value(column, slot) {
            Cell::Missing => Some(None),
            Cell::Number(bits) => Some(Some(bits as i64)),
            other => panic!("a run row's hour cell holds {other:?}"),
        }
    };
    let total = |store: &Store| {
        let held = store.view(TableId::EquipmentLossTotals.index());
        held.slot(leaf(GER, GER, LAND, Some(7))).and_then(|slot| held.number(0, slot))
    };
    let reason = |store: &Store| {
        let held = store.view(TableId::EquipmentLossCoverage.index());
        let slot = held.slot(RUN_KEY).expect("the run's row");
        String::from_utf8_lossy(held.string(held.text(4, slot).expect("a reason")).expect("its text")).into_owned()
    };
    assert_eq!(total(&store), Some(5), "hour 0's complete drain counts");
    assert_eq!(run_hour(&store, 6), Some(Some(0)), "counted from hour 0");
    let every = [TableId::Countries, TableId::CountryLinks, TableId::EquipmentDestruction];
    let not_the_drain = [TableId::Countries, TableId::CountryLinks];
    for (hour, read, expected) in [(1, &every[..], 10), (2, &not_the_drain[..], 10), (3, &every[..], 15)] {
        through_journal(&mut journal, &mut store, hour, read);
        let sampled = store.view(drain).sampled_at();
        assert_eq!(store.view(drain).status(), Status::Complete, "hour {hour}: the drain stays complete");
        assert_eq!(sampled, Some(if hour == 2 { 1 } else { hour }), "hour {hour}: stamped read exactly when its read bit is set");
        writers.hour(1, hour, false, &mut store).unwrap();
        assert_eq!(total(&store), Some(expected), "hour {hour}");
        match hour {
            2 => assert_eq!(reason(&store), NOT_READ, "an unread hour is named, never skipped silently"),
            _ => assert_eq!(reason(&store), "", "hour {hour}: a complete drain missing nothing"),
        }
        assert_eq!(run_hour(&store, 8), Some((hour >= 2).then_some(2)), "hour {hour}: a lower bound from the unread hour on");
        store.end_hour(&mut Discard).unwrap();
    }
}

const FRA: u32 = 3;

#[derive(Debug, Default)]
struct Noted {
    totals: Vec<u128>,
    dropped: usize,
}

struct Noting {
    inner: EquipmentLosses,
    noted: Arc<Mutex<Vec<Noted>>>,
}

impl Writer for Noting {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn version(&self) -> u32 {
        self.inner.version()
    }

    fn tables(&self) -> &'static [Output] {
        self.inner.tables()
    }

    fn reads(&self) -> &'static [Read] {
        self.inner.reads()
    }

    fn hourly(&self) -> bool {
        self.inner.hourly()
    }

    fn definitions(&self) -> &'static [recorder_derive_framework::framework::Uses] {
        self.inner.definitions()
    }

    fn start(&mut self) {
        self.inner.start();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows<'_>) -> io::Result<()> {
        self.inner.hour(hour, out)?;
        let (totals, _) = out.keys(TableId::EquipmentLossTotals);
        let (set, deleted) = out.keys(TableId::CombatLogDropped);
        self.noted.lock().unwrap().push(Noted { totals, dropped: set.len() + deleted.len() });
        Ok(())
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        self.inner.save(blob)
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        self.inner.load(blob)
    }

    fn prime(&mut self, hour: &Hour<'_>) {
        self.inner.prime(hour);
    }
}

impl Run {
    fn noting() -> (Self, Arc<Mutex<Vec<Noted>>>) {
        let feed = Arc::new(Mutex::new(Vec::new()));
        let noted = Arc::new(Mutex::new(Vec::new()));
        let mut writers = Writers::new(vec![Box::new(Feed(Arc::clone(&feed))), Box::new(Noting { inner: EquipmentLosses::default(), noted: Arc::clone(&noted) })]);
        writers.start_run();
        (Self { store: store(), writers, feed, hour: 0, start: 0 }, noted)
    }
}

fn three_countries(groups: u32) -> Vec<Op> {
    let mut ops = countries();
    ops.push(set(TableId::Countries, CountryRow { id: FRA, canonical_slot: FRA, tag: b"TAG".to_vec(), ..Default::default() }));
    ops.push(set(TableId::CountryLinks, CountryLinkRow { id: FRA, canonical_slot: FRA }));
    ops.extend((0..groups).map(|group| logged(FRA, group, 100, 8, 1)));
    ops.push(logged(GER, 4, 100, 7, 3));
    ops.push(logged(GER_ALIAS, 5, 100, 7, 2));
    ops
}

#[test]
fn a_holder_change_counts_again_only_the_slots_it_moved() {
    let (mut run, noted) = Run::noting();
    run.step(three_countries(200));
    assert_eq!(run.total(FRA, FRA, Total::COMBAT_LOG, Some(8)), number(200));
    assert_eq!(run.total(GER, GER, Total::COMBAT_LOG, Some(7)), number(5), "both raw ids of GER, in different groups");
    run.step(vec![set(TableId::CountryLinks, CountryLinkRow { id: GER_ALIAS, canonical_slot: ENG })]);
    assert_eq!(run.total(GER, GER, Total::COMBAT_LOG, Some(7)), number(3), "the alias's rows left GER");
    assert_eq!(run.total(ENG, ENG, Total::COMBAT_LOG, Some(7)), number(2), "and count for ENG");
    assert_eq!(run.total(FRA, FRA, Total::COMBAT_LOG, Some(8)), number(200));
    let moved = noted.lock().unwrap().pop().expect("the holder change's hour");
    let fra = leaf(FRA, FRA, Total::COMBAT_LOG, Some(8));
    assert!(!moved.totals.contains(&fra), "FRA's leaf is not counted again: {:?}", moved.totals);
    assert!(moved.totals.contains(&leaf(GER, GER, Total::COMBAT_LOG, Some(7))) && moved.totals.contains(&leaf(ENG, ENG, Total::COMBAT_LOG, Some(7))));
    assert_eq!(moved.dropped, 0, "a holder change sets nothing of the dropped rows");
    run.step(Vec::new());
    let quiet = noted.lock().unwrap().pop().expect("a quiet hour");
    assert!(quiet.totals.iter().all(|&key| leaf_source(key) >= Total::SUNK_CONVOYS), "a quiet hour sets no Combat Log leaf: {:?}", quiet.totals);
}

#[test]
fn a_holder_change_right_after_a_resume_counts_again_only_the_slots_it_moved() {
    let hours = || vec![three_countries(50), vec![logged(FRA, 0, 100, 8, 4)], vec![set(TableId::CountryLinks, CountryLinkRow { id: GER_ALIAS, canonical_slot: ENG }), logged(FRA, 1, 100, 8, 6)]];
    let mut unbroken = Run::new();
    for ops in hours() {
        unbroken.step(ops);
    }
    let (mut stopped, noted) = Run::noting();
    for (at, ops) in hours().into_iter().enumerate() {
        if at == 2 {
            let saved = stopped.writers.save().unwrap();
            let mut store = store();
            store.resume(stopped.store.tables().to_vec(), stopped.store.strings().next()).unwrap();
            let mut writers = Writers::new(vec![Box::new(Feed(Arc::clone(&stopped.feed))), Box::new(Noting { inner: EquipmentLosses::default(), noted: Arc::clone(&noted) })]);
            writers.start_run();
            writers.load(&saved, &mut store).unwrap();
            writers.prime(&store, 1, stopped.hour - 1);
            (stopped.store, stopped.writers) = (store, writers);
        }
        stopped.step(ops);
    }
    assert_eq!(stopped.dump(), unbroken.dump());
    assert_eq!(stopped.total(FRA, FRA, Total::COMBAT_LOG, Some(8)), number(50 + 3 + 5));
    let moved = noted.lock().unwrap().pop().expect("the hour after the resume");
    let combat: Vec<u128> = moved.totals.iter().copied().filter(|&key| leaf_source(key) < Total::SUNK_CONVOYS).collect();
    assert_eq!(combat.len(), 3, "GER's and ENG's design 7 counted again, FRA's design 8 by its one changed row: {combat:?}");
}

#[cfg(feature = "memory-labels")]
mod labelled {
    use super::*;
    use engine_allocation::labels;

    #[test]
    fn a_holder_change_allocates_for_the_slots_it_moved_not_for_the_log() {
        let mut run = Run::new();
        run.step(three_countries(100_000));
        assert_eq!(run.total(FRA, FRA, Total::COMBAT_LOG, Some(8)), number(100_000));
        labels::phase("g1: holder change");
        run.step(vec![set(TableId::CountryLinks, CountryLinkRow { id: GER_ALIAS, canonical_slot: ENG })]);
        labels::phase("g1: after");
        assert_eq!(run.total(ENG, ENG, Total::COMBAT_LOG, Some(7)), number(2));
        let phases = labels::phases();
        let phase = phases.iter().find(|phase| phase.name == "g1: holder change").expect("the holder change's phase");
        let peak = phase.labels.iter().filter(|charge| charge.owner == "writer: EquipmentLosses").map(|charge| charge.peak).max().unwrap_or(0);
        assert!(peak < 1 << 20, "the writer's label peaked at {peak} bytes in the holder change's hour");
    }
}

const JAN_1943: i32 = 43_800_000 + 1943 * 8760;
const JAN: u16 = 1943 * 12;
const FEB_1943: i32 = JAN_1943 + 31 * 24;

impl Run {
    fn retaining(start: i32, months: i64) -> Self {
        let mut run = Self::new();
        run.store.begin_run(start);
        (run.hour, run.start) = (start, start);
        run.writers.define(recorder_derive_framework::framework::Definitions::read_none().with_define(RETENTION, Ok(months)));
        run
    }

    fn step_at(&mut self, hour: i32, ops: Vec<Op>) {
        self.hour = hour;
        self.step(ops);
    }

    fn freed(&self, slot: u32, id: u32, variant: u32) -> Option<Cell<'_>> {
        self.cell(TableId::EquipmentLossTotals, leaf(slot, id, Total::COMBAT_LOG, Some(variant)), FREED)
    }
}

fn three_months() -> Vec<Vec<Op>> {
    let mut first = countries();
    first.extend([logged(GER, 4, JAN - 10, 7, 3), logged(GER, 4, JAN - 1, 7, 5), logged(GER, 4, JAN, 7, 1)]);
    vec![first, vec![gone(TableId::CombatLogEquipment, log_key(GER, 4, JAN - 10, 7)), gone(TableId::CombatLogEquipment, log_key(GER, 4, JAN - 1, 7))]]
}

#[test]
fn dropped_rows_whose_lines_the_game_freed_leave_the_table_and_stay_in_their_leaf() {
    assert_eq!((month_bucket(JAN_1943), month_bucket(FEB_1943), month_bucket(FEB_1943 - 1)), (i32::from(JAN), i32::from(JAN) + 1, i32::from(JAN)));
    let mut run = Run::retaining(JAN_1943, 6);
    for ops in three_months() {
        run.step(ops);
    }
    fn combat(run: &Run) -> Option<Cell<'_>> {
        run.total(GER, GER, Total::COMBAT_LOG, Some(7))
    }
    assert_eq!(run.cell(TableId::CombatLogDropped, log_key(GER, 4, JAN - 10, 7), 1), None, "freed by the game before it left the log: never held");
    assert_eq!(run.cell(TableId::CombatLogDropped, log_key(GER, 4, JAN - 1, 7), 1), number(5), "last month's entry is still the game's");
    assert_eq!((combat(&run), run.freed(GER, GER, 7)), (number(9), number(3)), "the freed amount stays in its leaf");
    run.step_at(FEB_1943, Vec::new());
    assert_eq!(run.cell(TableId::CombatLogDropped, log_key(GER, 4, JAN - 1, 7), 1), number(5), "February frees nothing more");
    run.step_at(first_hour(JAN + 6) + 5, Vec::new());
    assert_eq!(run.cell(TableId::CombatLogDropped, log_key(GER, 4, JAN - 1, 7), 1), number(5), "within July last month's entry is not yet certain to be freed");
    run.step_at(first_hour(JAN + 7), Vec::new());
    assert_eq!(run.cell(TableId::CombatLogDropped, log_key(GER, 4, JAN - 1, 7), 1), None, "August's turn frees it");
    assert_eq!((combat(&run), run.freed(GER, GER, 7)), (number(9), number(8)));
    run.step(vec![set(TableId::CountryLinks, CountryLinkRow { id: 12, canonical_slot: GER })]);
    assert_eq!((combat(&run), run.freed(GER, GER, 7)), (number(9), number(8)), "a recount takes the log, the dropped rows and the freed amount");
    let mut unknown = Run::retaining(JAN_1943, 0);
    for ops in three_months() {
        unknown.step(ops);
    }
    unknown.step_at(FEB_1943, Vec::new());
    assert_eq!(unknown.cell(TableId::CombatLogDropped, log_key(GER, 4, JAN - 10, 7), 1), number(3), "under a retention below one nothing is freed");
}

#[test]
fn a_resume_before_a_month_turns_frees_what_a_run_never_stopped_frees() {
    let mut unbroken = Run::retaining(JAN_1943, 6);
    for ops in three_months() {
        unbroken.step(ops);
    }
    unbroken.step_at(FEB_1943, Vec::new());
    unbroken.step(vec![logged(GER, 4, JAN + 1, 7, 2)]);
    let mut stopped = Run::retaining(JAN_1943, 6);
    for ops in three_months() {
        stopped.step(ops);
    }
    let saved = stopped.writers.save().unwrap();
    let mut store = store();
    store.resume(stopped.store.tables().to_vec(), stopped.store.strings().next()).unwrap();
    let mut writers = writers(&stopped.feed);
    writers.define(recorder_derive_framework::framework::Definitions::read_none().with_define(RETENTION, Ok(6)));
    writers.load(&saved, &mut store).unwrap();
    writers.prime(&store, 1, stopped.hour - 1);
    (stopped.store, stopped.writers) = (store, writers);
    stopped.step_at(FEB_1943, Vec::new());
    stopped.step(vec![logged(GER, 4, JAN + 1, 7, 2)]);
    assert_eq!(stopped.dump(), unbroken.dump());
    assert_eq!(stopped.freed(GER, GER, 7), number(3));
}

struct MonthsFeed(Arc<Mutex<Vec<Op>>>);

impl Writer for MonthsFeed {
    fn name(&self) -> &'static str {
        "MonthsFeed"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::CombatLogMonths)]
    }

    fn reads(&self) -> &'static [Read] {
        &[]
    }

    fn hourly(&self) -> bool {
        true
    }

    fn start(&mut self) {}

    fn hour(&mut self, _: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        for op in self.0.lock().unwrap().drain(..) {
            op(out);
        }
        Ok(())
    }

    fn save(&self, _: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
}

fn first_hour(month: u16) -> i32 {
    const DAYS: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let (year, within) = (i32::from(month / 12), usize::from(month % 12));
    43_800_000 + year * 8760 + DAYS[..within].iter().sum::<i32>() * 24
}

fn listed(group: u32, ordinal: u32, month: u16) -> Op {
    set(TableId::CombatLogMonths, CombatLogMonthRow { country: GER, group, kind: 0, ordinal, reason: 0, date: first_hour(month), allow_zero: false })
}

fn unlisted(group: u32, ordinal: u32) -> Op {
    gone(TableId::CombatLogMonths, CombatLogMonthRow { country: GER, group, kind: 0, ordinal, ..Default::default() }.key())
}

#[test]
fn without_the_retention_define_the_month_entries_the_game_lists_tell_which_lines_it_freed() {
    for month in [JAN - 10, JAN - 1, JAN] {
        assert_eq!(month_bucket(first_hour(month)), i32::from(month));
    }
    let months = Arc::new(Mutex::new(Vec::new()));
    let mut run = Run::new();
    run.store.begin_run(JAN_1943);
    (run.hour, run.start) = (JAN_1943, JAN_1943);
    run.writers = Writers::new(vec![Box::new(Feed(Arc::clone(&run.feed))), Box::new(MonthsFeed(Arc::clone(&months))), Box::new(EquipmentLosses::default())]);
    run.writers.start_run();
    let step = |run: &mut Run, listing: Vec<Op>, ops: Vec<Op>| {
        months.lock().unwrap().extend(listing);
        run.step(ops);
    };
    let mut first = countries();
    first.extend(four_lines());
    step(&mut run, vec![listed(4, 0, JAN - 10), listed(4, 1, JAN - 1), listed(4, 2, JAN), listed(5, 0, JAN - 1)], first);
    let lines = [log_key(GER, 4, JAN - 10, 7), log_key(GER, 4, JAN - 1, 7), log_key(GER, 5, JAN - 1, 7)];
    step(&mut run, Vec::new(), lines.iter().map(|&key| gone(TableId::CombatLogEquipment, key)).collect());
    fn combat(run: &Run) -> Option<Cell<'_>> {
        run.total(GER, GER, Total::COMBAT_LOG, Some(7))
    }
    assert_eq!((combat(&run), run.freed(GER, GER, 7)), (number(11), number(0)));
    assert!(lines.iter().all(|&key| run.cell(TableId::CombatLogDropped, key, 1).is_some()), "the three dropped lines are held");
    step(&mut run, vec![unlisted(4, 0)], Vec::new());
    assert_eq!(run.cell(TableId::CombatLogDropped, lines[0], 1), None, "group 4's oldest entry pruned: its line is freed");
    assert_eq!(run.cell(TableId::CombatLogDropped, lines[1], 1), number(5), "the next entry is still listed");
    assert_eq!((combat(&run), run.freed(GER, GER, 7)), (number(11), number(3)));
    step(&mut run, vec![unlisted(5, 0)], Vec::new());
    assert_eq!(run.cell(TableId::CombatLogDropped, lines[2], 1), None, "group 5's log deleted: its line is freed");
    assert_eq!((combat(&run), run.freed(GER, GER, 7)), (number(11), number(5)));
    assert_eq!(run.cell(TableId::CombatLogDropped, lines[1], 1), number(5));
}

fn four_lines() -> [Op; 4] {
    [logged(GER, 4, JAN - 10, 7, 3), logged(GER, 4, JAN - 1, 7, 5), logged(GER, 4, JAN, 7, 1), logged(GER, 5, JAN - 1, 7, 2)]
}

type Listing = (Vec<Op>, Vec<Op>);

fn pruned() -> Vec<Listing> {
    let mut first = countries();
    first.extend(four_lines());
    vec![
        (vec![listed(4, 0, JAN - 10), listed(4, 1, JAN - 1), listed(4, 2, JAN), listed(5, 0, JAN - 1)], first),
        (vec![unlisted(5, 0)], vec![gone(TableId::CombatLogEquipment, log_key(GER, 5, JAN - 1, 7))]),
        (vec![unlisted(4, 0)], vec![gone(TableId::CombatLogEquipment, log_key(GER, 4, JAN - 10, 7))]),
        (Vec::new(), vec![gone(TableId::CombatLogEquipment, log_key(GER, 4, JAN - 1, 7))]),
        (vec![listed(4, 1, JAN), listed(4, 2, JAN + 1)], vec![logged(GER, 4, JAN + 1, 7, 4)]),
    ]
}

fn listing_writers(feed: &Arc<Mutex<Vec<Op>>>, months: &Arc<Mutex<Vec<Op>>>) -> Writers {
    let mut writers = Writers::new(vec![Box::new(Feed(Arc::clone(feed))), Box::new(MonthsFeed(Arc::clone(months))), Box::new(EquipmentLosses::default())]);
    writers.start_run();
    writers
}

fn listing_run(months: &Arc<Mutex<Vec<Op>>>) -> Run {
    let mut run = Run::new();
    run.store.begin_run(JAN_1943);
    (run.hour, run.start) = (JAN_1943, JAN_1943);
    run.writers = listing_writers(&run.feed, months);
    run
}

fn listing_step(run: &mut Run, months: &Arc<Mutex<Vec<Op>>>, (listing, ops): Listing) {
    months.lock().unwrap().extend(listing);
    run.step(ops);
}

#[test]
fn a_line_leaving_with_its_month_entry_is_freed_that_hour_not_held_for_the_run() {
    let months = Arc::new(Mutex::new(Vec::new()));
    let mut run = listing_run(&months);
    let mut hours = pruned().into_iter();
    fn combat(run: &Run) -> Option<Cell<'_>> {
        run.total(GER, GER, Total::COMBAT_LOG, Some(7))
    }
    fn held(run: &Run, group: u32, month: u16) -> Option<Cell<'_>> {
        run.cell(TableId::CombatLogDropped, log_key(GER, group, month, 7), 1)
    }
    listing_step(&mut run, &months, hours.next().unwrap());
    assert_eq!((combat(&run), run.freed(GER, GER, 7)), (number(11), number(0)));
    listing_step(&mut run, &months, hours.next().unwrap());
    assert_eq!(held(&run, 5, JAN - 1), None, "group 5's log deleted whole: its line is freed in that hour");
    assert_eq!((combat(&run), run.freed(GER, GER, 7)), (number(11), number(2)));
    listing_step(&mut run, &months, hours.next().unwrap());
    assert_eq!(held(&run, 4, JAN - 10), None, "group 4's oldest entry pruned with its line: freed in that hour");
    assert_eq!((combat(&run), run.freed(GER, GER, 7)), (number(11), number(5)));
    listing_step(&mut run, &months, hours.next().unwrap());
    assert_eq!(held(&run, 4, JAN - 1), number(5), "its entry is listed: held");
    listing_step(&mut run, &months, hours.next().unwrap());
    assert_eq!(held(&run, 4, JAN - 1), None, "its entry pruned with no entry deleted: freed");
    assert_eq!((combat(&run), run.freed(GER, GER, 7)), (number(15), number(10)));
    assert!(run.store.view(TableId::CombatLogDropped.index()).rows().next().is_none(), "nothing the game freed is held");
}

#[test]
fn a_run_taken_up_after_any_hour_holds_the_dropped_lines_and_totals_of_one_never_stopped() {
    let months = Arc::new(Mutex::new(Vec::new()));
    let mut unbroken = listing_run(&months);
    let mut expected = Vec::new();
    for hour in pruned() {
        listing_step(&mut unbroken, &months, hour);
        expected.push(unbroken.dump());
    }
    for stop in 0..expected.len() - 1 {
        let months = Arc::new(Mutex::new(Vec::new()));
        let mut run = listing_run(&months);
        for (at, hour) in pruned().into_iter().enumerate() {
            listing_step(&mut run, &months, hour);
            if at == stop {
                let saved = run.writers.save().unwrap();
                let mut store = store();
                store.resume(run.store.tables().to_vec(), run.store.strings().next()).unwrap();
                let mut writers = listing_writers(&run.feed, &months);
                writers.load(&saved, &mut store).unwrap();
                writers.prime(&store, 1, run.hour - 1);
                (run.store, run.writers) = (store, writers);
            }
            if at > stop {
                assert_eq!(run.dump(), expected[at], "taken up after hour {stop}, at hour {at}");
            }
        }
    }
}
