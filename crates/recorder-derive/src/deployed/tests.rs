use super::*;
use crate::framework::{Definitions, Writers};
use crate::testing::store;
use replay_columnar::{manifest::Status, state::Cell, store::Store, unit::Discard};
use replay_data::rows::{
    CountryDeployedManpowerRow, DivisionManpowerContributor, Field, List, ManpowerReferences, Opaque,
};

enum Owned {
    Plain(Cell<'static>),
    Text(Vec<u8>),
    Opaque(Vec<u8>),
    U32s(Vec<u32>),
    U64s(Vec<u64>),
    Unrecorded,
}

fn owned(field: Field<'_>) -> Owned {
    match field {
        Field::NotRecorded => Owned::Unrecorded,
        Field::U8(value) => Owned::Plain(Cell::U8(value)),
        Field::I8(value) => Owned::Plain(Cell::I8(value)),
        Field::U16(value) => Owned::Plain(Cell::U16(value)),
        Field::I16(value) => Owned::Plain(Cell::I16(value)),
        Field::U32(value) => Owned::Plain(Cell::U32(value)),
        Field::I32(value) => Owned::Plain(Cell::I32(value)),
        Field::U64(value) => Owned::Plain(Cell::U64(value)),
        Field::I64(value) => Owned::Plain(Cell::I64(value)),
        Field::Bool(value) => Owned::Plain(Cell::Bool(value)),
        Field::OptionalI32(value) => Owned::Plain(Cell::OptionalI32(value)),
        Field::OptionalU32(value) => Owned::Plain(Cell::OptionalU32(value)),
        Field::OptionalI64(value) => Owned::Plain(Cell::OptionalI64(value)),
        Field::OptionalU64(value) => Owned::Plain(Cell::OptionalU64(value)),
        Field::Bytes(text) => Owned::Text(text.to_vec()),
        Field::U32List(List::Native(values)) => Owned::U32s(values.to_vec()),
        Field::U32List(List::Encoded(bytes)) => Owned::U32s(bytes.chunks_exact(4).map(|item| u32::from_le_bytes(item.try_into().unwrap())).collect()),
        Field::U64List(List::Native(values)) => Owned::U64s(values.to_vec()),
        Field::U64List(List::Encoded(bytes)) => Owned::U64s(bytes.chunks_exact(8).map(|item| u64::from_le_bytes(item.try_into().unwrap())).collect()),
        Field::Opaque(Opaque::Encoded(bytes, _)) => Owned::Opaque(bytes.to_vec()),
        Field::Opaque(Opaque::Native(value)) => {
            let mut bytes = Vec::new();
            value.encode_value(&mut bytes).unwrap();
            Owned::Opaque(bytes)
        }
        Field::Empty => panic!("an empty field"),
    }
}

#[derive(Default)]
pub(crate) struct Edits {
    rows: Vec<(TableId, u128, Vec<Owned>)>,
    deletes: Vec<(TableId, u128)>,
    statuses: Vec<(TableId, Result<Status, Vec<u8>>)>,
    unstamped: Vec<TableId>,
}

impl Edits {
    pub fn put<R: RowCodec>(&mut self, table: TableId, row: R) -> &mut Self {
        let mut cells = Vec::new();
        row.visit_wire(&mut |_, field| {
            cells.push(owned(field));
            Ok(())
        })
        .unwrap();
        self.rows.push((table, row.key(), cells));
        self
    }

    pub fn delete(&mut self, table: TableId, key: u128) -> &mut Self {
        self.deletes.push((table, key));
        self
    }

    pub fn status(&mut self, table: TableId, status: Status) -> &mut Self {
        assert!(!matches!(status, Status::Unavailable { .. }), "an unavailable status names its reason with Edits::unavailable");
        self.statuses.push((table, Ok(status)));
        self
    }

    pub fn unavailable(&mut self, table: TableId, reason: &[u8]) -> &mut Self {
        self.statuses.push((table, Err(reason.to_vec())));
        self
    }

    pub fn unstamped(&mut self, table: TableId) -> &mut Self {
        self.unstamped.push(table);
        self
    }
}

pub(crate) struct Run {
    pub store: Store,
    writers: Writers,
    make: fn() -> Vec<Box<dyn Writer>>,
    inputs: &'static [TableId],
    next: i32,
}

impl Run {
    pub fn new(make: fn() -> Vec<Box<dyn Writer>>, inputs: &'static [TableId]) -> Self {
        let mut writers = Writers::new(make());
        writers.start_run();
        Self { store: store(), writers, make, inputs, next: 0 }
    }

    pub fn hour(&mut self, edits: &Edits) {
        let hour = self.next;
        self.next += 1;
        self.store.begin_hour(hour, &[]).unwrap();
        let mut tables: Vec<TableId> = edits.rows.iter().map(|edit| edit.0).chain(edits.deletes.iter().map(|edit| edit.0)).chain(edits.statuses.iter().map(|edit| edit.0)).collect();
        if hour == 0 {
            tables.extend_from_slice(self.inputs);
        }
        tables.sort_unstable();
        tables.dedup();
        for table in tables {
            let index = table.index();
            let status = match edits.statuses.iter().rev().find(|edit| edit.0 == table) {
                None => Status::Complete,
                Some((_, Ok(status))) => *status,
                Some((_, Err(reason))) => Status::Unavailable { reason: self.store.intern(reason).unwrap() },
            };
            let texts: Vec<Vec<Option<u32>>> = edits
                .rows
                .iter()
                .filter(|edit| edit.0 == table)
                .map(|edit| edit.2.iter().map(|cell| if let Owned::Text(text) = cell { Some(self.store.intern(text).unwrap()) } else { None }).collect())
                .collect();
            let edited = edits.rows.iter().any(|edit| edit.0 == table) || edits.deletes.iter().any(|edit| edit.0 == table);
            let stamped = !edits.unstamped.contains(&table);
            let sampled = if stamped { Some(hour) } else { self.store.view(index).sampled_at() };
            self.store.status(index, status, stamped, sampled, edited).unwrap();
            let mut changes = self.store.table(index).unwrap();
            for &(_, key) in edits.deletes.iter().filter(|edit| edit.0 == table) {
                if let Some(slot) = changes.slot(key) {
                    changes.delete(slot).unwrap();
                }
            }
            for ((_, key, cells), ids) in edits.rows.iter().filter(|edit| edit.0 == table).zip(&texts) {
                let (slot, fresh) = match changes.slot(*key) {
                    Some(slot) => (slot, false),
                    None => (changes.insert(*key).unwrap(), true),
                };
                for (column, (cell, id)) in cells.iter().zip(ids).enumerate() {
                    let value = match cell {
                        Owned::Plain(cell) => *cell,
                        Owned::Text(_) => Cell::Bytes(id.unwrap()),
                        Owned::Opaque(bytes) => Cell::Opaque(bytes),
                        Owned::U32s(values) => Cell::U32List(values),
                        Owned::U64s(values) => Cell::U64List(values),
                        Owned::Unrecorded => {
                            if fresh || changes.held().columns()[column].recorded(slot) {
                                changes.unrecord(slot, column).unwrap();
                            }
                            continue;
                        }
                    };
                    if fresh {
                        changes.write(slot, column, value).unwrap();
                    } else {
                        changes.offer(slot, column, value).unwrap();
                    }
                }
            }
        }
        self.store.settle();
        self.writers.hour(1, hour, hour == 0, &mut self.store).unwrap();
        self.store.end_hour(&mut Discard).unwrap();
    }

    pub fn define(&mut self, definitions: Definitions) {
        self.writers.define(definitions);
    }

    pub fn resumed(&self) -> Self {
        let saved = self.writers.save().unwrap();
        let mut store = crate::testing::resumed_holding(&self.store);
        let mut writers = Writers::new((self.make)());
        writers.define(self.writers.definitions().clone());
        writers.load(&saved, &mut store).unwrap();
        writers.prime(&store, 1, self.next - 1);
        Self { store, writers, make: self.make, inputs: self.inputs, next: self.next }
    }

    pub fn rows<R: RowCodec>(&self, table: TableId) -> Vec<R> {
        let view = self.store.view(table.index());
        let every: Vec<usize> = (0..table.field_count()).collect();
        let mut rows: Vec<(u128, R)> = view.rows().map(|slot| (view.key(slot).unwrap(), row(&view, slot, &every).unwrap())).collect();
        rows.sort_unstable_by_key(|(key, _)| *key);
        rows.into_iter().map(|(_, row)| row).collect()
    }

    pub fn status(&self, table: TableId) -> Status {
        self.store.view(table.index()).status()
    }

    pub fn why(&self, table: TableId) -> String {
        let view = self.store.view(table.index());
        match view.status() {
            Status::Unavailable { reason } => format!("unavailable: {}", String::from_utf8_lossy(view.string(reason).unwrap_or_default())),
            status => format!("{status:?}"),
        }
    }
}

pub(crate) fn country(id: u32, slot: u32, tag: &[u8]) -> CountryRow {
    CountryRow { id, canonical_slot: slot, tag: tag.to_vec(), ..Default::default() }
}

pub(crate) fn link(id: u32, slot: u32) -> CountryLinkRow {
    CountryLinkRow { id, canonical_slot: slot }
}

pub(crate) fn division(id: u64, roster: u32, logical: u32, expeditionary: u32, template: u64) -> DivisionRow {
    DivisionRow { id, roster_owner: roster, logical_owner: logical, expeditionary_owner: expeditionary, template, ..Default::default() }
}

pub(crate) fn counters(country: u32, sent: Option<&[u64]>) -> CountryCounterInputRow {
    CountryCounterInputRow { country, sent_expeditionaries: Recorded::Value(sent.map(|list| ManpowerReferences::from(list.to_vec()))), ..Default::default() }
}

pub(crate) fn volunteer(country: u32, ordinal: u32, division: u64) -> SentVolunteerRow {
    SentVolunteerRow { country, ordinal, division }
}

pub(crate) fn world(edits: &mut Edits) -> &mut Edits {
    edits
        .put(TableId::Countries, country(10, 1, b"GER"))
        .put(TableId::Countries, country(20, 2, b"ITA"))
        .put(TableId::Countries, country(30, 3, b"ROM"))
        .put(TableId::CountryLinks, link(10, 1))
        .put(TableId::CountryLinks, link(11, 1))
        .put(TableId::CountryLinks, link(20, 2))
        .put(TableId::CountryLinks, link(30, 3))
}

const INPUTS: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Divisions,
    TableId::DivisionLandInputs,
    TableId::DivisionTemplateIcons,
    TableId::CountryCounterInputs,
    TableId::SentVolunteers,
    TableId::Ships,
    TableId::TaskForces,
    TableId::Fleets,
    TableId::AirWings,
];

fn writers() -> Vec<Box<dyn Writer>> {
    vec![Box::new(Deployed::default())]
}

fn land(division: u64, entries: &[(u32, i32)]) -> DivisionLandInputsRow {
    DivisionLandInputsRow { division, manpower: entries.iter().map(|&(country, amount)| DivisionManpowerContributor { country, amount }).collect(), ..Default::default() }
}

fn template(template: u64, exile: Option<bool>) -> DivisionTemplateIconRow {
    DivisionTemplateIconRow { template, manpower_exile: exile.map_or(Recorded::NotRecorded, Recorded::Value), ..Default::default() }
}

fn ship(id: u64, task_force: u64, acquired: Option<i32>, required: Option<i32>) -> ShipRow {
    ShipRow { id, task_force, acquired_manpower: acquired.into(), required_manpower: required.into(), ..Default::default() }
}

fn task(id: u64, fleet: u64) -> TaskForceRow {
    TaskForceRow { id, fleet, ..Default::default() }
}

fn fleet(id: u64, owner: u32) -> FleetRow {
    FleetRow { id, owner, ..Default::default() }
}

fn wing(id: u64, owner: u32, exile: Option<i32>, acquired: Option<i32>) -> AirWingRow {
    AirWingRow { id, owner, exile_country: exile.into(), acquired_manpower: acquired.into(), ..Default::default() }
}

fn forces() -> Edits {
    let mut edits = Edits::default();
    world(&mut edits)
        .put(TableId::DivisionTemplateIcons, template(7, Some(false)))
        .put(TableId::DivisionTemplateIcons, template(8, Some(true)))
        .put(TableId::Divisions, division(1, 10, 10, 0, 7))
        .put(TableId::DivisionLandInputs, land(1, &[(20, 50), (11, 1000)]))
        .put(TableId::Divisions, division(2, 10, 20, 0, 7))
        .put(TableId::DivisionLandInputs, land(2, &[(10, 5)]))
        .put(TableId::Divisions, division(3, 10, 10, 0, 8))
        .put(TableId::DivisionLandInputs, land(3, &[(10, 5)]))
        .put(TableId::Divisions, division(4, 20, 20, 10, 7))
        .put(TableId::DivisionLandInputs, land(4, &[(10, 300), (20, 7)]))
        .put(TableId::CountryCounterInputs, counters(10, Some(&[4, 4, 0, 99])))
        .put(TableId::CountryCounterInputs, counters(20, Some(&[])))
        .put(TableId::CountryCounterInputs, counters(30, Some(&[])))
        .put(TableId::Divisions, division(5, 30, 30, 0, 7))
        .put(TableId::DivisionLandInputs, land(5, &[(20, 40)]))
        .put(TableId::SentVolunteers, volunteer(20, 0, 5))
        .put(TableId::TaskForces, task(100, 200))
        .put(TableId::Fleets, fleet(200, 11))
        .put(TableId::Ships, ship(1000, 100, Some(500), Some(600)))
        .put(TableId::TaskForces, task(101, 201))
        .put(TableId::Fleets, fleet(201, 20))
        .put(TableId::Ships, ship(1001, 101, Some(70), Some(80)))
        .put(TableId::AirWings, wing(1, 10, Some(0), Some(30)))
        .put(TableId::AirWings, wing(2, 20, Some(10), Some(9)))
        .put(TableId::AirWings, wing(3, 30, Some(-1), Some(4)));
    edits
}

const U: Option<i32> = None;

fn deployed(run: &Run) -> Vec<(u32, [Option<i32>; 5])> {
    let value = |cell: &Recorded<i32>| match cell {
        Recorded::Value(value) => Some(*value),
        Recorded::NotRecorded => None,
    };
    run.rows::<CountryDeployedManpowerRow>(TableId::CountryDeployedManpower)
        .iter()
        .map(|row| (row.slot, [value(&row.army), value(&row.navy), value(&row.navy_required), value(&row.air), value(&row.exile_air)]))
        .collect()
}

#[test]
fn deployed_manpower_attributes_each_term_as_the_game_does() {
    let mut run = Run::new(writers, INPUTS);
    run.hour(&forces());
    assert_eq!(run.status(TableId::CountryDeployedManpower), Status::Complete);
    assert_eq!(
        deployed(&run),
        [(1, [Some(1600), Some(500), Some(600), Some(30), Some(9)]), (2, [Some(40), Some(70), Some(80), Some(0), Some(0)]), (3, [Some(0), Some(0), Some(0), Some(4), Some(0)])]
    );
    let mut edits = Edits::default();
    edits
        .put(TableId::DivisionLandInputs, land(1, &[(10, 1200)]))
        .delete(TableId::Divisions, 4)
        .delete(TableId::DivisionLandInputs, 4)
        .put(TableId::Fleets, fleet(201, 30))
        .put(TableId::AirWings, wing(3, 30, Some(-1), Some(i32::MAX)))
        .put(TableId::AirWings, wing(4, 30, Some(0), Some(i32::MAX)));
    run.hour(&edits);
    assert_eq!(
        deployed(&run),
        [(1, [Some(1200), Some(500), Some(600), Some(30), Some(9)]), (2, [Some(40), Some(0), Some(0), Some(0), Some(0)]), (3, [Some(0), Some(70), Some(80), Some(-2), Some(0)])],
        "the dead expeditionary leaves GER's list, the fleet's new owner takes its ship, crews wrap"
    );
    let mut edits = Edits::default();
    edits.delete(TableId::Countries, 30);
    run.hour(&edits);
    assert_eq!(deployed(&run).iter().map(|row| row.0).collect::<Vec<_>>(), [1, 2], "a slot no live country holds loses its row");
}

#[test]
fn what_deployed_manpower_cannot_know_is_not_recorded() {
    let mut run = Run::new(writers, INPUTS);
    run.hour(&forces());
    let mut edits = Edits::default();
    edits
        .put(TableId::Divisions, division(6, 20, 20, 0, 9))
        .put(TableId::DivisionLandInputs, land(6, &[(20, 1)]))
        .put(TableId::Ships, ship(1002, 999, Some(1), Some(1)))
        .put(TableId::AirWings, wing(2, 20, None, Some(9)))
        .put(TableId::CountryCounterInputs, counters(30, None));
    run.hour(&edits);
    assert_eq!(
        deployed(&run),
        [(1, [Some(1600), U, U, Some(30), U]), (2, [U, U, U, U, U]), (3, [U, U, U, Some(4), U])],
        "ITA's division of a template with no row, a ship of no task force, ITA's wing of no exile read, ROM's list not recorded"
    );
    let mut edits = Edits::default();
    edits
        .put(TableId::DivisionTemplateIcons, template(9, Some(false)))
        .delete(TableId::Ships, 1002)
        .put(TableId::AirWings, wing(2, 20, Some(10), Some(9)))
        .put(TableId::CountryCounterInputs, counters(30, Some(&[])))
        .put(TableId::DivisionTemplateIcons, template(7, None));
    run.hour(&edits);
    assert_eq!(
        deployed(&run),
        [(1, [U, Some(500), Some(600), Some(30), Some(9)]), (2, [U, Some(70), Some(80), Some(0), Some(0)]), (3, [U, Some(0), Some(0), Some(4), Some(0)])],
        "template 9 now ordinary, but template 7's exile flag is not recorded"
    );
    let mut edits = Edits::default();
    edits.put(TableId::DivisionTemplateIcons, template(7, Some(false))).unavailable(TableId::Ships, b"not read");
    run.hour(&edits);
    assert_eq!(deployed(&run), [(1, [Some(1600), U, U, Some(30), Some(9)]), (2, [Some(41), U, U, Some(0), Some(0)]), (3, [Some(0), U, U, Some(4), Some(0)])]);
    let mut edits = Edits::default();
    edits.status(TableId::Ships, Status::Complete);
    run.hour(&edits);
    assert_eq!(deployed(&run)[1], (2, [Some(41), Some(70), Some(80), Some(0), Some(0)]), "Ships complete again");
}

#[test]
fn a_resumed_deployed_writer_writes_the_tables_of_one_never_stopped() {
    let mut unbroken = Run::new(writers, INPUTS);
    unbroken.hour(&forces());
    let mut stopped = Run::new(writers, INPUTS);
    stopped.hour(&forces());
    let mut resumed = stopped.resumed();
    let mut edits = Edits::default();
    edits
        .put(TableId::DivisionLandInputs, land(1, &[(10, 7)]))
        .put(TableId::SentVolunteers, volunteer(10, 0, 5))
        .put(TableId::DivisionLandInputs, land(5, &[(20, 40), (10, 3)]))
        .put(TableId::Ships, ship(1000, 101, Some(500), Some(600)));
    unbroken.hour(&edits);
    resumed.hour(&edits);
    assert_eq!(deployed(&unbroken), deployed(&resumed));
    assert_eq!(deployed(&resumed)[0], (1, [Some(7 + 600 + 3), Some(0), Some(0), Some(30), Some(9)]));
}
