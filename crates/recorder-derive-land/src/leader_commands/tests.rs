use super::*;
use recorder_derive_framework::{framework::Writers, testing};
use replay_columnar::{manifest::Status, store::Store, unit::Discard};
use replay_data::rows::Recorded;

const TEXTS: &[&[u8]] = &[b"not read", b""];
const NOT_READ: u32 = 0;

const L1: u64 = 1 << 32 | 6;
const L2: u64 = 2 << 32 | 6;
const L3: u64 = 3 << 32 | 6;
const G1: u64 = 10;
const G2: u64 = 11;
const G3: u64 = 12;
const F1: u64 = 20;

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(LeaderCommands::default())]);
    writers.start_run();
    writers
}

fn group(handle: u64, leader: u64, army_group: bool) -> OrdersGroupRow {
    OrdersGroupRow { handle, leader, is_army_group: u8::from(army_group), name: Recorded::Value(Some(format!("group {handle}").into_bytes())), ..Default::default() }
}

fn fleet(id: u64, leader: Option<u64>) -> FleetRow {
    FleetRow { id, leader: Recorded::Value(leader), name: Recorded::Value(Some(format!("fleet {id}").into_bytes())), ..Default::default() }
}

fn run(store: &mut Store, writers: &mut Writers, hour: i32) {
    store.begin_hour(hour, if hour == 0 { TEXTS } else { &[] }).unwrap();
    let groups = if hour == 3 { Status::Unavailable { reason: NOT_READ } } else { Status::Complete };
    store.status(TableId::OrdersGroups.index(), groups, true, Some(hour), true).unwrap();
    store.status(TableId::Fleets.index(), Status::Complete, true, Some(hour), true).unwrap();
    let put = |store: &mut Store, row: OrdersGroupRow| testing::put(store, TableId::OrdersGroups, &row, TEXTS).unwrap();
    match hour {
        0 => {
            put(store, group(G1, L1, true));
            put(store, group(G2, L2, false));
            testing::put(store, TableId::Fleets, &fleet(F1, Some(L3)), TEXTS).unwrap();
        }
        1 => {
            put(store, group(G1, L2, true));
            testing::remove(store, TableId::OrdersGroups, u128::from(G2)).unwrap();
        }
        2 => {
            testing::put(store, TableId::Fleets, &fleet(F1, None), TEXTS).unwrap();
            put(store, group(G3, L1, false));
            put(store, group(G1, L2, false));
        }
        5 => put(store, group(G1, 0, false)),
        6 => put(store, group(G1, L2, false)),
        _ => {}
    }
    store.settle();
    writers.hour(1, hour, hour == 0, store).unwrap();
    store.end_hour(&mut Discard).unwrap();
}

type Span = (u64, u8, u64, u8, i32, i32, bool);

fn named(name: Option<&[u8]>, source: u8, formation: u64) -> bool {
    let expected = if source == Command::FLEET { format!("fleet {formation}") } else { format!("group {formation}") };
    let decoded = name.and_then(|bytes| <Option<Vec<u8>> as replay_data::value::Value>::decode(&mut replay_data::codec::Decoder::new(bytes)).ok());
    decoded == Some(Some(expected.into_bytes()))
}

fn ended(store: &Store) -> Vec<Span> {
    let view = store.view(TableId::LeaderCommands.index());
    testing::rows(store, TableId::LeaderCommands)
        .into_iter()
        .map(|(key, _)| {
            let slot = view.slot(key).unwrap();
            let number = |column| view.number(column, slot).unwrap();
            let (source, formation) = (number(0) as u8, number(1) as u64);
            ((key >> 64) as u64, source, formation, number(2) as u8, number(3) as i32, number(4) as i32, named(view.list(5, slot), source, formation))
        })
        .collect()
}

fn open(store: &Store) -> Vec<(u8, u64, u64, i32, u8, bool)> {
    let view = store.view(TableId::LeaderCommandSpans.index());
    testing::rows(store, TableId::LeaderCommandSpans)
        .into_iter()
        .map(|(key, _)| {
            let slot = view.slot(key).unwrap();
            let number = |column| view.number(column, slot).unwrap();
            let (source, formation) = ((key >> 64) as u8, key as u64);
            (source, formation, number(0) as u64, number(1) as i32, number(2) as u8, named(view.list(3, slot), source, formation))
        })
        .collect()
}

#[test]
fn a_command_spans_its_leader_column_s_hours_and_ends_where_it_changes_or_is_not_read() {
    let (group_kind, army, army_group, navy) = (Command::ORDERS_GROUP, Command::ARMY, Command::ARMY_GROUP, Command::NAVY);
    let (mut store, mut writers) = (testing::store(), writers());
    let mut seen = Vec::new();
    let mut listed = Vec::new();
    for hour in 0..=6 {
        run(&mut store, &mut writers, hour);
        seen.push((ended(&store), open(&store)));
        listed.push(testing::listed(&store, TableId::LeaderCommands));
    }
    assert_eq!(seen[0], (vec![], vec![(group_kind, G1, L1, 0, army_group, true), (group_kind, G2, L2, 0, army, true), (Command::FLEET, F1, L3, 0, navy, true)]), "the run's first hour opens every led formation");
    assert_eq!(seen[1].0, vec![(L1, group_kind, G1, army_group, 0, 0, true), (L2, group_kind, G2, army, 0, 0, true)], "a leader replaced and a formation deleted end the hour before");
    assert_eq!(seen[2].0, vec![(L3, Command::FLEET, F1, navy, 0, 1, true)], "a fleet without a leader ends its span");
    assert_eq!(seen[2].1, vec![(group_kind, G1, L2, 1, army_group, true), (group_kind, G3, L1, 2, army, true)], "a kind is its span's first hour's");
    assert_eq!(seen[4], (vec![(L1, group_kind, G3, army, 2, 2, true), (L2, group_kind, G1, army_group, 1, 2, true)], vec![(group_kind, G1, L2, 4, army, true), (group_kind, G3, L1, 4, army, true)]), "after an hour not read, every span ends at the last hour read and starts again");
    assert_eq!(seen[5].0, vec![(L2, group_kind, G1, army, 4, 4, true)]);
    assert_eq!(seen[6], (vec![], vec![(group_kind, G1, L2, 6, army, true), (group_kind, G3, L1, 4, army, true)]), "a leader back is a new span");
    assert!(seen[3].0.is_empty(), "an hour the writer did not run appends nothing");
    assert_eq!(listed, [vec![], vec![0, 1], vec![2], vec![2], vec![3, 4], vec![5], vec![]], "each ended span is appended once and forgotten at the writer's next hour; an hour it did not run forgets nothing");
}

#[test]
fn a_resume_in_the_middle_writes_what_a_run_never_stopped_writes() {
    let (mut unbroken, mut writing) = (testing::store(), writers());
    let mut expected = Vec::new();
    for hour in 0..=6 {
        run(&mut unbroken, &mut writing, hour);
        expected.push((testing::rows(&unbroken, TableId::LeaderCommands), testing::listed(&unbroken, TableId::LeaderCommands), testing::rows(&unbroken, TableId::LeaderCommandSpans)));
    }
    for stop in [1, 3, 4] {
        let (mut stopped, mut before) = (testing::store(), writers());
        for hour in 0..=stop {
            run(&mut stopped, &mut before, hour);
        }
        let saved = before.save().unwrap();
        let mut resumed = testing::resumed(&stopped, TEXTS);
        let mut after = writers();
        after.load(&saved, &mut resumed).unwrap();
        after.prime(&resumed, 1, stop);
        for hour in stop + 1..=6 {
            run(&mut resumed, &mut after, hour);
            let seen = (testing::rows(&resumed, TableId::LeaderCommands), testing::listed(&resumed, TableId::LeaderCommands), testing::rows(&resumed, TableId::LeaderCommandSpans));
            assert_eq!(seen, expected[hour as usize], "stopped after hour {stop}, hour {hour}");
        }
    }
}

#[test]
fn span_keys_are_the_row_codec_s() {
    use replay_data::rows::LeaderCommandSpanRow;
    let row = LeaderCommandSpanRow { source: Command::FLEET, formation: u64::MAX, ..Default::default() };
    assert_eq!(span_key(Command::FLEET, u64::MAX), row.key());
}
