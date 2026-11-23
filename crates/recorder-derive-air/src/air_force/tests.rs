use super::*;
use recorder_derive_framework::{
    forces::testing::{cells, put, rows, store},
    framework::Writers,
};
use replay_columnar::{manifest::Status, store::Store};
use replay_data::rows::{AirForcePartRow, AirForceWingKindRow, CountryLinkRow, CountryRow};

const S: i64 = SCALE;
const V: u64 = 1000;
const W: u64 = 2000;

#[derive(Clone)]
struct World {
    countries: Vec<(u32, u32)>,
    links: Vec<(u32, u32)>,
    wings: Vec<(u64, u32, i32, i32)>,
    entries: Vec<(u64, u32, u64, i64)>,
    equipment: Status,
}

fn world(wings: &[(u64, u32, i32, i32)], entries: &[(u64, u32, u64, i64)]) -> World {
    World {
        countries: vec![(10, 1), (20, 2), (30, 3)],
        links: vec![(10, 1), (20, 2), (30, 3), (40, 7)],
        wings: wings.to_vec(),
        entries: entries.to_vec(),
        equipment: Status::Complete,
    }
}

fn feed(store: &mut Store, hour: i32, world: &World) {
    store.begin_hour(hour, if hour == 0 { &[b"x"] } else { &[] }).unwrap();
    let countries: Vec<CountryRow> = world.countries.iter().map(|&(id, slot)| CountryRow { id, canonical_slot: slot, ..Default::default() }).collect();
    put(store, hour, TableId::Countries, &countries, Status::Complete);
    let links: Vec<CountryLinkRow> = world.links.iter().map(|&(id, slot)| CountryLinkRow { id, canonical_slot: slot }).collect();
    put(store, hour, TableId::CountryLinks, &links, Status::Complete);
    let wings: Vec<AirWingRow> = world
        .wings
        .iter()
        .map(|&(id, owner, country_override, planes)| AirWingRow { id, owner, country_override, planes, ..Default::default() })
        .collect();
    put(store, hour, TableId::AirWings, &wings, Status::Complete);
    let entries: Vec<WingEquipmentRow> = world.entries.iter().map(|&(wing, ordinal, variant, amount_raw)| WingEquipmentRow { wing, ordinal, variant, amount_raw }).collect();
    put(store, hour, TableId::WingEquipment, &entries, world.equipment);
    store.settle();
}

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(AirForce::default()), Box::new(AirForceComposition::default())]);
    writers.start_run();
    writers
}

const TABLES: [TableId; 4] = [TableId::AirForceTotals, TableId::AirForceParts, TableId::AirForceWingKinds, TableId::AirForceWingSignatures];

fn run(hours: &[World], resume: Option<usize>) -> Vec<Vec<Vec<(u128, Vec<String>)>>> {
    let mut store = store();
    let mut writers = writers();
    let mut seen = Vec::new();
    for (hour, world) in hours.iter().enumerate() {
        feed(&mut store, hour as i32, world);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        seen.push(TABLES.iter().map(|&table| rows(&store, table)).collect());
        if resume == Some(hour) {
            let saved = writers.save().unwrap();
            writers = Writers::new(vec![Box::new(AirForce::default()), Box::new(AirForceComposition::default())]);
            writers.load(&saved, &mut store).unwrap();
        }
    }
    seen
}

fn totals(store: &Store, slot: u32) -> Option<Vec<Option<i64>>> {
    cells(store, TableId::AirForceTotals, u128::from(slot))
}

fn part(store: &Store, slot: u32, side: usize, part: Part) -> Option<Option<i64>> {
    cells(store, TableId::AirForceParts, part_key(slot, side, part)).map(|cells| cells[0])
}

fn kind(store: &Store, slot: u32, side: usize, number: u32) -> Option<i64> {
    cells(store, TableId::AirForceWingKinds, kind_key(slot, side, number)).map(|cells| cells[0].unwrap())
}

#[test]
fn air_force_totals_follow_owners_and_say_what_is_not_known() {
    let hours = [
        world(&[(1, 10, 0, 10), (2, 10, 20, 5)], &[]),
        world(&[(1, 10, 0, -1), (2, 10, 0, 5)], &[]),
        world(&[(1, 10, 0, -1), (2, 10, 0, 5), (3, 99, 0, 4)], &[]),
        world(&[(1, 10, 0, 7), (2, 10, 0, 5)], &[]),
        world(&[(1, 20, -3, 7), (2, 40, 0, 5)], &[]),
        world(&[(1, 20, -3, 7)], &[]),
    ];
    let mut store = store();
    let mut writers = writers();
    let unknown = Some(vec![None; 4]);
    for (hour, world) in hours.iter().enumerate() {
        feed(&mut store, hour as i32, world);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        let expected: [(u32, Option<Vec<Option<i64>>>); 4] = match hour {
            0 => [(1, Some(vec![Some(15), Some(2), Some(10), Some(1)])), (2, Some(vec![Some(0), Some(0), Some(5), Some(1)])), (3, Some(vec![Some(0); 4])), (7, None)],
            1 => [(1, Some(vec![None, Some(2), None, Some(2)])), (2, Some(vec![Some(0); 4])), (3, Some(vec![Some(0); 4])), (7, None)],
            2 => [(1, unknown.clone()), (2, unknown.clone()), (3, unknown.clone()), (7, None)],
            3 => [(1, Some(vec![Some(12), Some(2), Some(12), Some(2)])), (2, Some(vec![Some(0); 4])), (3, Some(vec![Some(0); 4])), (7, None)],
            4 => [(1, Some(vec![Some(0); 4])), (2, Some(vec![Some(7), Some(1), Some(7), Some(1)])), (3, Some(vec![Some(0); 4])), (7, Some(vec![Some(5), Some(1), Some(5), Some(1)]))],
            _ => [(1, Some(vec![Some(0); 4])), (2, Some(vec![Some(7), Some(1), Some(7), Some(1)])), (3, Some(vec![Some(0); 4])), (7, None)],
        };
        for (slot, cells) in expected {
            assert_eq!(totals(&store, slot), cells, "hour {hour}, slot {slot}");
        }
    }
}

#[test]
fn air_force_composition_splits_planes_and_numbers_compositions() {
    let full = [(1, 0, V, 6 * S), (1, 1, W, 4 * S), (2, 0, V, 2 * S)];
    let hours = [
        world(&[(1, 10, 0, 10), (2, 10, 0, 3)], &full),
        world(&[(1, 10, 0, 10), (2, 10, 0, 3)], &[(1, 0, V, 6 * S), (1, 1, W, 4 * S), (2, 0, V, 2 * S + 1)]),
        world(&[(1, 10, 0, 10), (2, 10, 0, 3)], &[(2, 0, V, 2 * S + 1)]),
        world(&[(1, 10, 0, 10), (2, 10, 0, 3), (3, 99, 0, 1)], &[(2, 0, V, 2 * S + 1)]),
        world(&[(1, 10, 0, 10), (2, 10, 0, 3)], &full),
    ];
    let mut store = store();
    let mut writers = writers();
    let mut named: BTreeMap<u128, Vec<u64>> = BTreeMap::new();
    let signatures = |store: &Store| -> Vec<(u128, Vec<u64>)> {
        let view = store.view(TableId::AirForceWingSignatures.index());
        let mut held: Vec<(u128, Vec<u64>)> = view
            .rows()
            .map(|slot| {
                let text = view.string(view.text(0, slot).unwrap()).unwrap().to_vec();
                let number = view.key(slot).unwrap();
                (number, AirForceWingSignatureRow { number: number as u32, variants: text }.variants().unwrap())
            })
            .collect();
        held.sort_unstable();
        held
    };
    for (hour, world) in hours.iter().enumerate() {
        feed(&mut store, hour as i32, world);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        match hour {
            0 | 4 => {
                assert_eq!(part(&store, 1, 0, Some(V)), Some(Some(8)), "hour {hour}");
                assert_eq!(part(&store, 1, 0, Some(W)), Some(Some(4)));
                assert_eq!(part(&store, 1, 0, None), Some(Some(1)));
                assert_eq!(part(&store, 1, 1, Some(V)), Some(Some(8)), "the effective country is the owner");
                let again = if hour == 0 { 1 } else { 3 };
                assert_eq!((kind(&store, 1, 0, 0), kind(&store, 1, 0, again)), (Some(1), Some(1)));
            }
            1 => {
                assert_eq!(part(&store, 1, 0, Some(V)), Some(None), "a wing that cannot split leaves its variant unknown");
                assert_eq!(part(&store, 1, 0, Some(W)), Some(Some(4)));
                assert_eq!(part(&store, 1, 0, None), Some(Some(3)));
            }
            2 => {
                assert_eq!(part(&store, 1, 0, Some(W)), None);
                assert_eq!(part(&store, 1, 0, None), Some(Some(13)));
                assert_eq!((kind(&store, 1, 0, 0), kind(&store, 1, 0, 1), kind(&store, 1, 0, 2)), (Some(1), None, Some(1)));
            }
            _ => {
                assert!(rows(&store, TableId::AirForceParts).is_empty() && rows(&store, TableId::AirForceWingKinds).is_empty(), "an unresolved owner leaves no rows");
            }
        }
        let expected: &[(u128, Vec<u64>)] = match hour {
            0 | 1 => &[(0, vec![V]), (1, vec![V, W])],
            2 | 3 => &[(0, vec![V]), (2, vec![])],
            _ => &[(0, vec![V]), (3, vec![V, W])],
        };
        let held = signatures(&store);
        assert_eq!(held, expected, "hour {hour}");
        for (key, _) in rows(&store, TableId::AirForceWingKinds) {
            assert!(held.iter().any(|(number, _)| *number == u128::from(kind_of(key).2)), "hour {hour}: kinds row {key} names a held signature");
        }
        for (number, variants) in held {
            assert_eq!(named.entry(number).or_insert_with(|| variants.clone()), &variants, "number {number} names one composition in the run");
        }
    }
}

#[test]
fn equipment_not_complete_withholds_the_composition_alone() {
    let mut hours = vec![world(&[(1, 10, 0, 10)], &[(1, 0, V, 10 * S)]); 3];
    hours[1].equipment = Status::Truncated { missing: 1 };
    hours[1].wings[0].3 = 9;
    let mut store = store();
    let mut writers = writers();
    for (hour, world) in hours.iter().enumerate() {
        feed(&mut store, hour as i32, world);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        let withheld = matches!(store.view(TableId::AirForceParts.index()).status(), Status::Unavailable { .. });
        assert_eq!(withheld, hour == 1, "hour {hour}");
        assert_eq!(store.view(TableId::AirForceTotals.index()).status(), Status::Complete);
        assert_eq!(totals(&store, 1).unwrap()[0], Some(if hour == 1 { 9 } else { 10 }));
    }
    assert_eq!(part(&store, 1, 0, Some(V)), Some(Some(10)));
}

#[test]
fn a_resume_writes_what_a_run_that_never_stopped_writes() {
    let hours = [
        world(&[(1, 10, 0, 10), (2, 10, 20, 3)], &[(1, 0, V, 6 * S), (1, 1, W, 4 * S), (2, 0, V, 2 * S)]),
        world(&[(1, 10, 0, 10), (2, 10, 20, 3), (4, 30, 0, 2)], &[(1, 0, V, 6 * S), (2, 0, W, 2 * S), (4, 0, W, S)]),
        world(&[(1, 10, 0, 8), (4, 30, 0, 2)], &[(1, 0, V, 6 * S), (4, 0, V, S), (4, 1, W, S)]),
        world(&[(1, 10, 0, 8), (4, 99, 0, 2)], &[(1, 0, V, 6 * S), (4, 0, V, S)]),
        world(&[(1, 20, 0, 8), (4, 30, 10, 2), (5, 10, 0, 1)], &[(1, 0, W, 8 * S), (5, 0, V, S)]),
    ];
    let whole = run(&hours, None);
    for resume in 0..hours.len() - 1 {
        assert_eq!(run(&hours, Some(resume)), whole, "resumed after hour {resume}");
    }
}

#[test]
fn a_wing_splits_its_plane_word_by_its_entries_exactly_or_not_at_all() {
    assert_eq!(split(Some(10), &[(V, 6 * S), (W, 2 * S), (V, S)]), vec![(None, Some(1)), (Some(V), Some(7)), (Some(W), Some(2))]);
    assert_eq!(split(Some(3), &[(V, 3 * S), (W, 0)]), vec![(Some(V), Some(3)), (Some(W), Some(0))]);
    assert_eq!(split(Some(3), &[(V, 2 * S + 1)]), vec![(None, Some(3)), (Some(V), None)], "a fractional amount");
    assert_eq!(split(Some(3), &[(V, 4 * S)]), vec![(None, Some(3)), (Some(V), None)], "more than the word");
    assert_eq!(split(Some(3), &[(V, -S)]), vec![(None, Some(3)), (Some(V), None)], "a negative amount");
    assert_eq!(split(None, &[(V, S)]), vec![(None, None), (Some(V), None)], "a negative word");
    assert_eq!(split(Some(4), &[]), vec![(None, Some(4))]);
    assert_eq!(signature(&[(W, S), (V, 0), (W, S), (V + 1, 1)]), vec![V + 1, W]);
    assert_eq!(owners(10, 20), [10, 20]);
    assert_eq!(owners(10, 0), [10, 10]);
    assert_eq!(owners(10, -5), [10, 10]);
}

#[test]
fn part_and_kind_keys_are_the_row_codec_s() {
    for (slot, ownership, variant) in [(0, 0, None), (7, 1, Some(0)), (u32::MAX, 1, Some(u64::MAX))] {
        let key = part_key(slot, usize::from(ownership), variant);
        assert_eq!(key, AirForcePartRow { slot, ownership, variant, ..Default::default() }.key());
        assert_eq!(part_of(key), (slot, usize::from(ownership), variant));
    }
    let key = kind_key(u32::MAX, 1, 9);
    assert_eq!(key, AirForceWingKindRow { slot: u32::MAX, ownership: 1, signature: 9, wings: 0 }.key());
    assert_eq!(kind_of(key), (u32::MAX, 1, 9));
    let row = AirForceWingSignatureRow { number: 0, variants: AirForceWingSignatureRow::encode(&[V, u64::MAX]) };
    assert_eq!(row.variants().unwrap(), vec![V, u64::MAX]);
    assert!(AirForceWingSignatureRow { number: 0, variants: vec![1, 2, 3] }.variants().is_err());
}
