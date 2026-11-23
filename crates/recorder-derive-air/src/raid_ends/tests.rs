use super::*;
use recorder_derive_framework::{
    forces::testing::{put, rows, store},
    framework::Writers,
};
use engine_native::craid_instance::{OUTCOME_NONE, OUTCOME_SUCCESS, PHASE_PREPARING};
use replay_columnar::store::Store;
use replay_data::rows::{RaidRow, RaidTargetRow};

#[derive(Clone, Copy)]
struct Raid {
    id: u32,
    raider: u32,
    phase: i32,
    outcome: i32,
    travelled: Option<i64>,
    province: u32,
    state: u32,
}

const REFERENCE: u32 = 7;

const fn raid(id: u32, phase: i32, outcome: i32, travelled: Option<i64>, province: u32, state: u32) -> Raid {
    Raid { id, raider: 9, phase, outcome, travelled, province, state }
}

fn feed(store: &mut Store, hour: i32, raids: &[Raid], status: Status, controller: u32) {
    store.begin_hour(hour, if hour == 0 { &[b"x"] } else { &[] }).unwrap();
    let rows: Vec<RaidRow> = raids
        .iter()
        .map(|raid| RaidRow { id: raid.id, reference_type: REFERENCE, parent_slot: raid.raider, phase: raid.phase, outcome: raid.outcome, travelled: raid.travelled, preparation_hours: 3, ..Default::default() })
        .collect();
    let targets: Vec<RaidTargetRow> = raids
        .iter()
        .map(|raid| RaidTargetRow { id: raid.id, reference_type: REFERENCE, scope: RaidScope { province: raid.province, state: raid.state, ..Default::default() }, raid_type: 40 + raid.id, ..Default::default() })
        .collect();
    put(store, hour, TableId::Raids, &rows, status);
    put(store, hour, TableId::RaidTargets, &targets, Status::Complete);
    put(store, hour, TableId::Provinces, &[ProvinceRow { id: 10, state: 5, controller, ..Default::default() }], Status::Complete);
    put(store, hour, TableId::States, &[StateRow { id: 5, controller: 200, ..Default::default() }], Status::Complete);
    put(store, hour, TableId::CountryLinks, &[CountryLinkRow { id: 100, canonical_slot: 1 }, CountryLinkRow { id: 200, canonical_slot: 2 }], Status::Complete);
    store.settle();
}

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(RaidEnds::default())]);
    writers.start_run();
    writers
}

const PREPARING: i32 = PHASE_PREPARING;
const FLYING: i32 = PHASE_IN_PROGRESS;
const ENDED: i32 = PHASE_ENDED;

type Hours = [(&'static [Raid], Status, u32)];

const HOURS: &Hours = &[
    (&[raid(1, PREPARING, OUTCOME_NONE, None, 10, 0), raid(2, ENDED, OUTCOME_SUCCESS, Some(500), 10, 0)], Status::Complete, 100),
    (&[raid(1, FLYING, OUTCOME_NONE, Some(0), 10, 0), raid(2, ENDED, OUTCOME_SUCCESS, Some(500), 10, 0), raid(3, PREPARING, OUTCOME_NONE, None, 0, 5), raid(4, PREPARING, OUTCOME_NONE, None, 99, 0)], Status::Complete, 100),
    (&[raid(1, ENDED, OUTCOME_SUCCESS, Some(800), 10, 0), raid(3, PREPARING, OUTCOME_NONE, None, 0, 5), raid(4, PREPARING, OUTCOME_NONE, None, 99, 0)], Status::Complete, 100),
    (&[raid(1, ENDED, OUTCOME_SUCCESS, Some(800), 10, 0), raid(4, ENDED, OUTCOME_NONE, None, 99, 0), raid(6, PREPARING, OUTCOME_NONE, None, 10, 0)], Status::Complete, 200),
    (&[raid(1, ENDED, OUTCOME_SUCCESS, Some(800), 10, 0)], Status::Truncated { missing: 1 }, 200),
    (&[raid(1, ENDED, OUTCOME_SUCCESS, Some(800), 10, 0), raid(5, ENDED, OUTCOME_NONE, None, 10, 0)], Status::Complete, 200),
];

type End = (i32, u32, u8, i32, Option<i32>, bool, Option<i32>, Option<i32>, Option<Option<u32>>, Option<Option<u32>>, u32, u8);

fn ends(store: &Store) -> Vec<End> {
    let view = store.view(TableId::RaidEnds.index());
    let mut out: Vec<End> = view
        .rows()
        .map(|slot| {
            let key = view.key(slot).unwrap();
            let number = |column: usize| view.number(column, slot);
            let optional = |column: usize| match view.value(column, slot) {
                Held::Number(bits) => Some(Some(bits)),
                Held::Missing => Some(None),
                _ => None,
            };
            (
                (key >> 64) as u32 as i32,
                (key >> 32) as u32,
                number(0).unwrap() as u8,
                number(1).unwrap() as i32,
                optional(2).unwrap().map(|hour| hour as i32),
                number(3).unwrap() != 0,
                optional(4).unwrap().map(|hour| hour as i32),
                number(6).map(|phase| phase as i32),
                optional(20).map(|state| state.map(|state| state as u32)),
                optional(21).map(|victim| victim.map(|victim| victim as u32)),
                number(22).unwrap() as u32,
                number(23).unwrap() as u8,
            )
        })
        .collect();
    out.sort_unstable_by_key(|end| end.1);
    out
}

#[test]
fn a_raid_ends_once_at_its_first_ended_read_or_its_deletion_with_its_victim_read_then() {
    let mut store = store();
    let mut writers = writers();
    let none = TagHolderRow::NONE;
    for (hour, &(raids, status, controller)) in HOURS.iter().enumerate() {
        feed(&mut store, hour as i32, raids, status, controller);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        let withheld = matches!(store.view(TableId::RaidEnds.index()).status(), Status::Unavailable { .. });
        assert_eq!(withheld, hour == 4, "hour {hour}");
        let expected: Vec<End> = match hour {
            2 => vec![(2, 1, RaidEndRow::ENDED, 1, Some(0), false, Some(1), Some(ENDED), Some(Some(5)), Some(Some(100)), 1, RaidEndRow::VICTIM_KNOWN)],
            3 => vec![
                (3, 3, RaidEndRow::DELETED, 2, Some(1), true, None, Some(PREPARING), Some(Some(5)), Some(Some(200)), 2, RaidEndRow::VICTIM_KNOWN),
                (3, 4, RaidEndRow::ENDED, 2, Some(1), true, None, Some(ENDED), None, None, none, RaidEndRow::LOCATION_UNKNOWN),
            ],
            4 => ends(&store),
            5 => vec![
                (5, 5, RaidEndRow::ENDED, 3, Some(5), false, None, Some(ENDED), Some(Some(5)), Some(Some(200)), 2, RaidEndRow::VICTIM_KNOWN),
                (5, 6, RaidEndRow::DELETED, 3, Some(3), true, None, None, None, None, none, RaidEndRow::TARGET_UNKNOWN),
            ],
            _ => Vec::new(),
        };
        assert_eq!(ends(&store), expected, "hour {hour}");
    }
}

#[test]
fn an_end_row_carries_the_raids_last_state_and_its_target() {
    let mut store = store();
    let mut writers = writers();
    for (hour, &(raids, status, controller)) in HOURS.iter().take(3).enumerate() {
        feed(&mut store, hour as i32, raids, status, controller);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
    }
    let view = store.view(TableId::RaidEnds.index());
    let slot = view.slot(RaidEndRow::key_of(2, u128::from(1_u32) << 32 | u128::from(REFERENCE))).unwrap();
    let number = |column: usize| view.number(column, slot);
    assert_eq!([5, 7, 8, 9, 10, 11, 19].map(number), [Some(9), Some(i64::from(OUTCOME_SUCCESS)), Some(800), Some(3), Some(10), Some(0), Some(41)]);
}

#[test]
fn a_resume_writes_what_a_run_that_never_stopped_writes() {
    let run = |resume: Option<usize>| {
        let mut store = store();
        let mut writers = writers();
        let mut seen = Vec::new();
        for (hour, &(raids, status, controller)) in HOURS.iter().enumerate() {
            feed(&mut store, hour as i32, raids, status, controller);
            writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
            seen.push(rows(&store, TableId::RaidEnds));
            if resume == Some(hour) {
                let saved = writers.save().unwrap();
                writers = Writers::new(vec![Box::new(RaidEnds::default())]);
                writers.load(&saved, &mut store).unwrap();
            }
        }
        seen
    };
    let whole = run(None);
    for resume in 0..HOURS.len() - 1 {
        assert_eq!(run(Some(resume)), whole, "resumed after hour {resume}");
    }
}
