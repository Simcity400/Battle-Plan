use super::*;
use recorder_derive_framework::{framework::Writers, testing};
use recorder_derive_world::identity::TagHolders;
use replay_columnar::{store::Store, unit::Discard};
use replay_data::rows::{CountryLinkRow, CountryRow};

const TEXTS: &[&[u8]] = &[b"GER", b"ITA", b"not read"];
const NONE: u32 = AirLossTotalRow::NONE;

fn make() -> Vec<Box<dyn Writer>> {
    vec![Box::new(TagHolders::default()), Box::new(AirLossTotals::default()), Box::new(BombingTotals::default())]
}

fn writers() -> Writers {
    let mut writers = Writers::new(make());
    writers.start_run();
    writers
}

#[derive(Default)]
struct Input {
    losses: Vec<AirLossEventRow>,
    damage: Vec<BuildingBombingDamageRow>,
    unread: Option<TableId>,
}

fn run(store: &mut Store, writers: &mut Writers, hour: i32, input: &Input) {
    store.begin_hour(hour, if hour == 0 { TEXTS } else { &[] }).unwrap();
    for table in [TableId::Countries, TableId::CountryLinks, TableId::AirLossEvents, TableId::BuildingBombingDamage] {
        let status = if input.unread == Some(table) { Status::Unavailable { reason: 2 } } else { Status::Complete };
        store.status(table.index(), status, true, Some(hour), true).unwrap();
    }
    if hour == 0 {
        for (id, slot, tag) in [(10, 1, b"GER"), (20, 2, b"ITA")] {
            testing::put(store, TableId::Countries, &CountryRow { id, canonical_slot: slot, tag: tag.to_vec(), ..Default::default() }, TEXTS).unwrap();
        }
        for (id, canonical_slot) in [(10, 1), (11, 1), (20, 2)] {
            testing::put(store, TableId::CountryLinks, &CountryLinkRow { id, canonical_slot }, TEXTS).unwrap();
        }
    }
    for row in &input.losses {
        testing::put(store, TableId::AirLossEvents, row, TEXTS).unwrap();
    }
    for row in &input.damage {
        testing::put(store, TableId::BuildingBombingDamage, row, TEXTS).unwrap();
    }
    store.settle();
    writers.hour(1, hour, hour == 0, store).unwrap();
    store.end_hour(&mut Discard).unwrap();
}

fn loss(id: u64, game_hour: i32, country: u32, (region, native_cause): (u32, u32), credited_enemy: bool, amount_raw: i64, opponent: Option<u32>) -> AirLossEventRow {
    AirLossEventRow { id, game_hour, country, region, native_cause, family_base: 0, credited_enemy, amount_raw, opponent, daily_slot: 0 }
}

fn bombing(id: u64, attacker: Option<u32>, victim: u32, cause: BuildingBombingDamageCause, amount_raw: i64) -> BuildingBombingDamageRow {
    BuildingBombingDamageRow { id, game_hour: 0, attacker, victim, state: 3, province: None, building_type: 9, native_cause: 0, cause, amount_raw, credited_raw: amount_raw }
}

type Totals<const N: usize> = Vec<(u128, [Option<i64>; N], i64)>;

fn totals<const N: usize>(store: &Store, table: TableId) -> Totals<N> {
    let view = store.view(table.index());
    let mut rows: Totals<N> = view.rows().map(|slot| (view.key(slot).unwrap(), std::array::from_fn(|column| view.number(column, slot)), view.number(N, slot).unwrap())).collect();
    rows.sort_unstable();
    rows
}

#[test]
fn losses_add_at_their_read_under_the_holder_resolved_then_and_only_where_the_ledger_counts_them() {
    let (mut store, mut writers) = (testing::store(), writers());
    let first = Input { losses: vec![loss(1, 0, 10, (5, 1), false, 300, Some(20)), loss(2, 0, 11, (5, 1), true, 100, None), loss(3, 0, 55, (5, 2), false, 70, Some(10))], ..Input::default() };
    run(&mut store, &mut writers, 0, &first);
    run(&mut store, &mut writers, 1, &Input { losses: vec![loss(4, -5, 20, (6, 3), false, 50, None), loss(5, 1, 10, (5, 1), false, -40, Some(20))], ..Input::default() });
    run(&mut store, &mut writers, 2, &Input { losses: vec![loss(6, 2, 10, (5, 1), false, 1_000, Some(20))], unread: Some(TableId::AirLossEvents), ..Input::default() });
    assert_eq!(store.view(TableId::AirLossTotals.index()).status(), Status::Complete, "an unread log adds nothing; what was counted stays known");
    run(&mut store, &mut writers, 3, &Input { losses: vec![loss(7, 3, 10, (5, 1), false, 1_000, Some(20))], unread: Some(TableId::CountryLinks), ..Input::default() });
    run(&mut store, &mut writers, 4, &Input { losses: vec![loss(8, 4, 20, (6, 3), false, 5, None)], ..Input::default() });
    assert_eq!(
        totals::<4>(&store, TableId::AirLossTotals),
        vec![
            (AirLossTotalRow::key_of(1, 10, 5, 1), [Some(0), Some(100), Some(260), Some(0)], 0),
            (AirLossTotalRow::key_of(2, 20, 6, 3), [Some(0), Some(0), Some(0), Some(55)], 6),
            (AirLossTotalRow::key_of(NONE, NONE, 5, 2), [Some(0), Some(0), Some(70), Some(0)], 0),
        ]
    );
}

#[test]
fn bombing_counts_aircraft_callers_once_on_each_side() {
    let (mut store, mut writers) = (testing::store(), writers());
    let damage = vec![
        bombing(1, Some(10), 20, BuildingBombingDamageCause::AirMission, 2_500_000),
        bombing(2, None, 20, BuildingBombingDamageCause::AirNaval, 1_000_000),
        bombing(3, Some(10), 20, BuildingBombingDamageCause::LandCombat, 5_000_000),
        bombing(4, Some(10), 20, BuildingBombingDamageCause::Unknown, 3_000_000),
    ];
    run(&mut store, &mut writers, 0, &Input { damage, ..Input::default() });
    let (mission, naval) = (BuildingBombingDamageCause::AirMission as u8, BuildingBombingDamageCause::AirNaval as u8);
    assert_eq!(
        totals::<2>(&store, TableId::BombingTotals),
        vec![
            (BombingTotalRow::key_of(1, 10, mission), [Some(2_500_000), Some(0)], 0),
            (BombingTotalRow::key_of(2, 20, mission), [Some(0), Some(2_500_000)], 0),
            (BombingTotalRow::key_of(2, 20, naval), [Some(0), Some(1_000_000)], 0),
            (BombingTotalRow::key_of(NONE, NONE, naval), [Some(1_000_000), Some(0)], 0),
        ]
    );
}

#[test]
fn a_total_past_64_bits_is_not_recorded_and_a_resume_adds_to_the_held_totals() {
    let (mut store, mut writers) = (testing::store(), writers());
    let big = i64::MAX / 2 + 1;
    run(&mut store, &mut writers, 0, &Input { losses: vec![loss(1, 0, 10, (5, 1), false, big, Some(20)), loss(2, 0, 10, (5, 1), true, 7, Some(20))], ..Input::default() });
    run(&mut store, &mut writers, 1, &Input { losses: vec![loss(3, 1, 10, (5, 1), false, big, Some(20))], ..Input::default() });
    run(&mut store, &mut writers, 2, &Input { losses: vec![loss(4, 2, 10, (5, 1), false, -big, Some(20))], ..Input::default() });
    let key = AirLossTotalRow::key_of(1, 10, 5, 1);
    assert_eq!(totals::<4>(&store, TableId::AirLossTotals), vec![(key, [Some(7), Some(0), None, Some(0)], 0)], "not recorded is never wrapped or recovered");
    let saved = writers.save().unwrap();
    let mut resumed = testing::resumed_holding(&store);
    let mut taken = Writers::new(make());
    taken.load(&saved, &mut resumed).unwrap();
    taken.prime(&resumed, 1, 2);
    run(&mut resumed, &mut taken, 3, &Input { losses: vec![loss(5, 1, 11, (5, 1), true, 3, Some(20))], ..Input::default() });
    assert_eq!(totals::<4>(&resumed, TableId::AirLossTotals), vec![(key, [Some(10), Some(0), None, Some(0)], 2)]);
}
