use dashboard_query::identity::Holders;
use super::*;
use dashboard_world::products::groups::MembershipRun;

fn number_row(hour: i32, key: u128, cells: &[(usize, log::Cell)], width: usize) -> log::Row {
    let mut row = vec![log::Cell::Missing; width];
    for (at, cell) in cells {
        row[*at] = cell.clone();
    }
    log::Row { hour, key, facet: 0, cells: row }
}

#[test]
fn a_finalized_combat_is_its_log_rows_observers_and_losses_and_a_cell_not_recorded_leaves_it_out() {
    let columns = ["combat", "observers", "losses"].map(|name| dashboard_declare::field(TableId::LandCombatResults, name).expect("a result column"));
    let observers = vec![LandTerminalObserver { attacker: true, kind: CombatLogObserverKind::State as u8, item: 0, values: [5, 6, 0, 0, 0] }];
    let losses = [
        LandSideLosses { manpower: 10, equipment: vec![LandEquipmentLoss { variant: 77, amount_raw: 150_000 }] },
        LandSideLosses { manpower: 20, equipment: Vec::new() },
    ];
    let (mut observed, mut lost) = (Vec::new(), Vec::new());
    observers.encode(&mut observed).expect("encodes");
    losses.encode(&mut lost).expect("encodes");
    let width = columns.iter().max().unwrap() + 1;
    let row = number_row(40, 3, &[(columns[0], log::Cell::Number(9)), (columns[1], log::Cell::List(observed)), (columns[2], log::Cell::List(lost))], width);
    assert_eq!(finalized(&row, &columns), Some(Finalized { combat: 9, observers, losses }));
    let mut unrecorded = row;
    unrecorded.cells[columns[2]] = log::Cell::Unrecorded;
    assert_eq!(finalized(&unrecorded, &columns), None, "losses not recorded are never read as none");
}

#[test]
fn split_result_observers_reconstruct_ordered_facts_and_withhold_incomplete_combats() {
    use replay_data::rows::{CombatLogObserverRow, RowCodec as _};
    let observer_columns = ["kind", "item", "index", "identity", "identity_2", "identity_3", "a", "b", "c", "d", "e"]
        .map(|name| dashboard_declare::field(TableId::LandCombatResultObservers, name).expect("an observer column"));
    let width = observer_columns.iter().max().unwrap() + 1;
    let to_log = |obs: &CombatLogObserverRow| {
        number_row(
            40,
            obs.key(),
            &[
                (observer_columns[0], log::Cell::Number(i64::from(obs.kind))),
                (observer_columns[1], log::Cell::Number(obs.item as i64)),
                (observer_columns[2], log::Cell::Number(obs.index as i64)),
                (observer_columns[3], log::Cell::Number(obs.identity as i64)),
                (observer_columns[4], log::Cell::Number(obs.identity_2 as i64)),
                (observer_columns[5], log::Cell::Number(obs.identity_3 as i64)),
                (observer_columns[6], log::Cell::Number(obs.a)),
                (observer_columns[7], log::Cell::Number(obs.b)),
                (observer_columns[8], log::Cell::Number(obs.c)),
                (observer_columns[9], log::Cell::Number(obs.d)),
                (observer_columns[10], log::Cell::Number(obs.e)),
            ],
            width,
        )
    };
    let mut state = CombatLogObserverRow::from_fact(9, true, CombatLogObserverKind::State as u8, 0, [5, 6, 3, 0, 0]).unwrap();
    state.ordinal = 0;
    let mut equip = CombatLogObserverRow::from_fact(9, true, CombatLogObserverKind::ActivityEquipment as u8, 2, [1, u64::MAX - 5, 100_000, 0, 0]).unwrap();
    equip.ordinal = 1;
    let mut def = CombatLogObserverRow::from_fact(9, false, CombatLogObserverKind::State as u8, 0, [7, 8, 0, 0, 0]).unwrap();
    def.ordinal = 2;
    let mut damaged = to_log(&CombatLogObserverRow::from_fact(10, true, CombatLogObserverKind::State as u8, 0, [1, 2, 0, 0, 0]).unwrap());
    damaged.cells[observer_columns[6]] = log::Cell::Unrecorded;
    let (by_combat, incomplete) = result_observers(&[to_log(&def), to_log(&equip), to_log(&state), damaged], &observer_columns);
    assert_eq!(incomplete, BTreeSet::from([10]));
    assert!(!by_combat.contains_key(&10));
    assert_eq!(
        by_combat[&9],
        vec![
            LandTerminalObserver { attacker: true, kind: CombatLogObserverKind::State as u8, item: 0, values: [5, 6, 3, 0, 0] },
            LandTerminalObserver { attacker: true, kind: CombatLogObserverKind::ActivityEquipment as u8, item: 2, values: [1, u64::MAX - 5, 100_000, 0, 0] },
            LandTerminalObserver { attacker: false, kind: CombatLogObserverKind::State as u8, item: 0, values: [7, 8, 0, 0, 0] },
        ]
    );
}

#[test]
fn a_battle_records_modifiers_are_the_newest_rows_its_record_inserted_never_an_earlier_battle_of_its_slot() {
    let columns = [0, 1];
    let key = |battle: u32, side: u8, ordinal: u32| u128::from(battle) << 40 | u128::from(side) << 32 | u128::from(ordinal);
    let row = |hour, key, modifier, hours| number_row(hour, key, &[(0, log::Cell::Number(modifier)), (1, log::Cell::Number(hours))], 2);
    let rows = [
        row(60, key(4, 0, 0), 1, 5),
        row(100, key(4, 0, 0), 2, 6),
        row(100, key(4, 1, 0), 3, 7),
        row(120, key(4, 1, 0), 3, 8),
        row(110, key(5, 0, 0), 9, 9),
    ];
    let battles = BTreeMap::from([(4, 100)]);
    assert_eq!(modifiers_of(&rows, &battles, &columns), BTreeMap::from([(4, vec![(0, 0, 2, 6), (1, 0, 3, 8)])]));
}

#[test]
fn a_side_is_chosen_by_a_country_on_it_and_never_left_out_by_one_not_known() {
    let tags = BattleCountryFilter { countries: Some(vec!["GER".into()]), ..BattleCountryFilter::default() };
    let some = |tag: &str| Some(tag.to_owned());
    assert!(matches_side(&tags, &Holders::default(), [some("ITA"), some("GER")], 10));
    assert!(!matches_side(&tags, &Holders::default(), [some("ITA")], 10));
    assert!(matches_side(&tags, &Holders::default(), [some("ITA"), None], 10), "a participant not known may be chosen");
    assert!(matches_side(&tags, &Holders::default(), [], 10), "a side with no country is not left out");
    let runs = BattleCountryFilter {
        membership: Some(vec![MembershipRun { tag: "GER".into(), first_hour: 0, last_hour: 50 }]),
        ..BattleCountryFilter::default()
    };
    assert!(matches_country(&runs, &Holders::default(), "GER", 50));
    assert!(!matches_country(&runs, &Holders::default(), "GER", 51));
    assert_eq!(fixed(-150_000), "-1.5");
}
