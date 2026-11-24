use super::*;
use engine_naval_history::naval_combat_demand as breakdown;

use super::fixtures::*;

#[test]
fn naval_empty_hour_keeps_all_tables_complete_without_storage() {
    let memory = Fixture::new(0);
    let mut captured = Captured::with_capacities(&Capacities::default());
    assert_eq!(captured.allocated_bytes(), 0);
    captured.capture(&memory, IMAGE, STATE);
    assert_eq!(captured.outcomes(), [Outcome::Rows { len: 0 }; N]);
    assert!(
        captured
            .tables()
            .iter()
            .all(|view| matches!(view, TableView::Complete(_)))
    );
}

#[test]
fn cold_defaults_capture_the_first_live_shape_before_publication() {
    let mut capacities = Capacities::default();
    let mut captured = Captured::with_capacities(&capacities);
    assert_eq!(captured.allocated_bytes(), 0);
    let memory = live_shape();
    captured.capture(&memory, IMAGE, STATE);
    assert!(
        captured
            .outcomes()
            .into_iter()
            .all(|outcome| matches!(outcome, Outcome::Rows { .. }))
    );
    assert_eq!(captured.details.outcome, Outcome::Rows { len: 3 });
    assert_eq!(captured.ships.outcome, Outcome::Rows { len: 90 });
    assert_eq!(captured.dense_bases.outcome, Outcome::Rows { len: 12 });
    assert_eq!(captured.dense_wings.len, 12);
    captured.grow(&mut capacities);
    let prepared = Captured::with_capacities(&capacities);
    assert!(prepared.details.rows.len() >= captured.details.rows.len());
    assert!(prepared.ships.rows.len() >= captured.ships.rows.len());
    assert!(prepared.dense_bases.rows.len() >= captured.dense_bases.rows.len());
    assert!(prepared.dense_wings.first.len() >= captured.dense_wings.first.len());
}

#[test]
fn cold_and_later_terrain_names_keep_mod_heap_bytes_in_publication() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    assert_eq!(captured.allocated_bytes(), 0);
    for length in [257, 4097, 5000] {
        let mut memory = Fixture::new(1);
        let name = vec![b'x'; length];
        let name_data = memory.allocate(length + 1);
        memory.put(name_data, &name);
        memory.u64(TERRAIN + 0x18, name_data as u64);
        memory.u64(TERRAIN + 0x28, length as u64);
        memory.u64(TERRAIN + 0x30, length as u64);
        let empty_bucket = memory.allocate(12);
        memory.u64(TERRAIN + 0x1b8, empty_bucket as u64);
        let province_weather = memory.allocate(2 * 0x330);
        memory.array(WEATHER + 0x370, province_weather, 2);
        for country in 0..COUNTRY_COUNT {
            let task = TASK_BASE + country * TASK_STRIDE;
            memory.u64(
                task + engine_native::cunit::LOCATION,
                (BODIES + 0xb00) as u64,
            );
            let diplomacy = memory.allocate(0x400);
            memory.u64(
                diplomacy,
                (IMAGE + engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0) as u64,
            );
            let relations = memory.allocate(8);
            memory.array(
                diplomacy + engine_native_parts::cdiplomacy_status_layout::RELATIONS,
                relations,
                1,
            );
            memory.u64(
                COUNTRY_BASE + country * COUNTRY_STRIDE + engine_native::ccountry::DIPLOMACY_STATUS,
                diplomacy as u64,
            );
        }
        assert!(engine_native_memory::msvc_string::read_text::<256>(&memory, TERRAIN + 0x18).is_none());
        captured.reset();
        captured.capture(&memory, IMAGE, STATE);
        assert_eq!(captured.details.outcome, Outcome::Rows { len: 1 });
        assert_eq!(
            captured.ships.outcome,
            Outcome::Rows {
                len: SHIPS_PER_COMBAT
            }
        );
        assert!(
            captured.ships.rows[..SHIPS_PER_COMBAT].iter().all(|row| row
                .weapon_context
                .terrain_name
                .as_bytes()
                == name)
        );
        let TableView::Complete(rows::TableRows::NavalCombatShips(ships)) = &captured.tables()[1]
        else {
            panic!("complete terrain operands were not projected");
        };
        assert!(
            ships
                .iter()
                .all(|row| row.weapon_context.terrain_name == name)
        );
        assert!(captured.allocated_bytes() >= SHIPS_PER_COMBAT * length);
    }
}

#[test]
fn naval_live_headers_presize_attach_and_combat_growth_before_capture() {
    let mut capacities = Capacities::default();
    assert_eq!(capacities.details, 0);
    for combats in [1, 66, 71] {
        let memory = Fixture::new(combats);
        let demand = breakdown::demand(&memory, IMAGE, STATE).unwrap();
        assert_eq!(demand.rows, combats);
        let old = capacities;
        capacities.raise_for_live_details(demand);
        let mut captured = Captured::with_capacities(&old);
        captured.try_grow_hint(&capacities).unwrap();
        captured.capture(&memory, IMAGE, STATE);
        assert_eq!(captured.details.outcome, Outcome::Rows { len: combats });
        assert!(captured.ships.rows.len() <= capacities.ships.max(captured.ships.rows.len()));
        let retained = capacities;
        capacities.raise_for_live_details(Default::default());
        assert!(capacities == retained);
    }
}

#[test]
fn naval_hint_reserve_failure_keeps_existing_lengths() {
    let capacities = Capacities::default();
    let mut captured = Captured::with_capacities(&capacities);
    let mut hint = capacities;
    hint.details += 1;
    hint.dense_wings = usize::MAX;
    assert!(captured.try_grow_hint(&hint).is_err());
    assert_eq!(captured.details.rows.len(), capacities.details);
    assert!(captured.dense_wings.first.is_empty());
    captured.try_grow_hint(&capacities).unwrap();
}

const DOCTRINE_NODES: usize = 30;

#[test]
fn doctrine_sources_reemitted_equal_the_walks() {
    let capture = |sources: bool| {
        super::SOURCE_MEMO_OFF.set(!sources);
        let memory = Fixture::live_sized(2)
            .with_doctrines(DOCTRINE_NODES)
            .with_locations();
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.capture(&memory, IMAGE, STATE);
        super::SOURCE_MEMO_OFF.set(false);
        assert_eq!(
            captured.ships.outcome,
            Outcome::Rows {
                len: 2 * SHIPS_PER_COMBAT
            }
        );
        (
            captured.weapon_sources.first[..captured.weapon_sources.len].to_vec(),
            captured.weapon_terms.first[..captured.weapon_terms.len].to_vec(),
            captured.ships.rows[..2 * SHIPS_PER_COMBAT]
                .iter()
                .map(|row| row.weapon_context.clone())
                .collect::<Vec<_>>(),
        )
    };
    let walked = capture(false);
    assert!(
        walked.0.iter().filter(|source| source.scope == 4).count()
            >= 2 * SHIPS_PER_COMBAT * DOCTRINE_NODES
    );
    for scope in [5, 6, 7, 8, 20] {
        assert_eq!(
            walked
                .0
                .iter()
                .filter(|source| source.scope == scope)
                .count(),
            2 * SHIPS_PER_COMBAT,
            "scope {scope}"
        );
    }
    assert_eq!(capture(true), walked);
}

