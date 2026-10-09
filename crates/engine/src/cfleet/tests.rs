use super::*;
use crate::{cunit, memory::SliceMemory};
#[test]
fn healthy_read_accounting_partitions_fleet_graph_without_changing_its_read_trace() {
    use crate::memory::{CountedMemory, ReadCounts};
    use std::cell::RefCell;

    struct Traced<'a> {
        memory: SliceMemory<'a>,
        reads: RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, destination.len()));
            self.memory.copy(address, destination)
        }
    }

    for corrupt_second in [false, true] {
        let mut bytes = fixture();
        if corrupt_second {
            put64(&mut bytes, SHIP2, 0);
        }
        let original = Traced {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            reads: RefCell::default(),
        };
        let mut expected_fleets = [Row::default(); 1];
        let mut expected_tasks = [ctask_force::Row::default(); 1];
        let mut expected_ships = [cship::Row::default(); 2];
        let expected = walk(
            &original,
            0,
            GS,
            &mut expected_fleets,
            &mut expected_tasks,
            &mut expected_ships,
        );
        let counted = CountedMemory::new(Traced {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            reads: RefCell::default(),
        });
        let mut fleets = [Row::default(); 1];
        let mut tasks = [ctask_force::Row::default(); 1];
        let mut ships = [cship::Row::default(); 2];
        let mut by_table = [ReadCounts::ZERO; 3];
        let mut active = 0;
        ReadCounts::take_current();
        let actual = walk_timed(
            &counted,
            0,
            GS,
            &mut fleets,
            &mut tasks,
            &mut ships,
            |following| {
                by_table[active] += ReadCounts::take_current();
                active = following;
                0
            },
        );
        by_table[active] += ReadCounts::take_current();
        assert_eq!(
            active, 0,
            "success and failed ship reads restore the fleet anchor"
        );
        assert_eq!(actual.capture, expected);
        assert_eq!(fleets, expected_fleets);
        assert_eq!(tasks, expected_tasks);
        assert_eq!(ships, expected_ships);
        assert_eq!(*counted.0.reads.borrow(), *original.reads.borrow());
        assert_eq!(actual.capture.fleets, Outcome::Rows { len: 1 });
        assert_eq!(actual.capture.task_forces, Outcome::Rows { len: 1 });
        if !corrupt_second {
            assert_eq!(actual.capture.ships, Outcome::Rows { len: 2 });
        }
        assert!(
            by_table
                .iter()
                .all(|counts| counts.spans > 0 && counts.bytes > 0)
        );
        let mut total = ReadCounts::ZERO;
        for counts in by_table {
            total += counts;
        }
        let trace = counted.0.reads.borrow();
        assert_eq!(
            total.spans,
            trace.iter().filter(|(_, len)| *len != 0).count() as u64
        );
        assert_eq!(
            total.bytes,
            trace.iter().map(|(_, len)| *len as u64).sum::<u64>()
        );
        eprintln!("fleet graph corrupt_second={corrupt_second}: {by_table:?}");
    }
}

#[test]
fn ship_visitors_receive_only_authenticated_emitted_rows_and_never_hide_incomplete_censuses() {
    for (capacity, corrupt_second) in [(0, false), (1, true), (2, true), (2, false)] {
        let mut bytes = fixture();
        if corrupt_second {
            put64(&mut bytes, SHIP2, 0);
        }
        let mut fleets = [Row::default(); 1];
        let mut tasks = [ctask_force::Row::default(); 1];
        let mut ships = [cship::Row::default(); 2];
        let mut visited = Vec::new();
        let captured = walk_timed_with(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            GS,
            &mut fleets,
            &mut tasks,
            &mut ships[..capacity],
            |_| 0,
            |pointer, row| visited.push((pointer, *row)),
        );
        assert_eq!(captured.capture.fleets, Outcome::Rows { len: 1 });
        assert_eq!(captured.capture.task_forces, Outcome::Rows { len: 1 });
        match capacity {
            0 => {
                assert!(visited.is_empty());
                assert_eq!(
                    captured.capture.ships,
                    Outcome::Truncated {
                        len: 0,
                        unread_rows: 2
                    }
                );
            }
            1 => {
                assert_eq!(visited, vec![(SHIP, ships[0])]);
                assert_eq!(
                    captured.capture.ships,
                    Outcome::Truncated {
                        len: 1,
                        unread_rows: 1
                    }
                );
            }
            _ if corrupt_second => {
                assert_eq!(visited, vec![(SHIP, ships[0])]);
                assert!(matches!(
                    captured.capture.ships,
                    Outcome::Unavailable { .. }
                ));
            }
            _ => {
                assert_eq!(visited, vec![(SHIP, ships[0]), (SHIP2, ships[1])]);
                assert_eq!(captured.capture.ships, Outcome::Rows { len: 2 });
            }
        }
    }
}

const GS: usize = 0x1000;
const COUNTRIES: usize = 0x3000;
const LINKS: usize = 0x3100;
const PROVINCES: usize = 0x3200;
const COUNTRY: usize = 0x4000;
const FLEET: usize = 0x6000;
const TASK: usize = 0x7000;
const SHIP: usize = 0x8000;
const SHIP2: usize = 0x9000;
const PROVINCE: usize = 0xa000;
const FLEET_ROSTER: usize = 0xb000;
const TASK_ROSTER: usize = 0xb100;
const SHIP_ROSTER: usize = 0xb200;

fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    put64(bytes, at, data as u64);
    put32(bytes, at + 8, count);
    put32(bytes, at + 12, count);
}
fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0xc000];
    array(&mut bytes, GS + cgame_state::COUNTRIES, COUNTRIES, 4);
    array(&mut bytes, GS + cgame_state::COUNTRY_LINKS, LINKS, 4);
    array(&mut bytes, GS + cgame_state::PROVINCES, PROVINCES, 2);
    put64(&mut bytes, COUNTRIES + 8, COUNTRY as u64);
    put64(&mut bytes, COUNTRIES + 24, COUNTRY as u64);
    put32(&mut bytes, LINKS + 4, 1);
    put32(&mut bytes, LINKS + 12, 1);
    put64(&mut bytes, COUNTRY, ccountry::VTABLE_RVA as u64);
    put32(&mut bytes, COUNTRY + ccountry::ID, 3);
    array(&mut bytes, COUNTRY + ccountry::FLEETS, FLEET_ROSTER, 1);
    put64(&mut bytes, FLEET_ROSTER, FLEET as u64);
    put64(&mut bytes, FLEET, VTABLE_RVA as u64);
    put64(&mut bytes, FLEET + HANDLE, (11_u64 << 32) | 61);
    put64(&mut bytes, FLEET + OWNER, COUNTRY as u64);
    put64(&mut bytes, FLEET + HOME_BASE, PROVINCE as u64);
    array(&mut bytes, FLEET + TASK_FORCES, TASK_ROSTER, 1);
    put64(&mut bytes, TASK_ROSTER, TASK as u64);
    put64(&mut bytes, TASK, ctask_force::VTABLE_RVA as u64);
    put64(
        &mut bytes,
        TASK + ctask_force::SECONDARY_BASE,
        ctask_force::SECONDARY_VTABLE_RVA as u64,
    );
    // The task force uses the ship id space. Its complete handle is retained.
    put64(&mut bytes, TASK + cunit::HANDLE, (11_u64 << 32) | 51);
    put64(&mut bytes, TASK + ctask_force::FLEET, FLEET as u64);
    put32(&mut bytes, TASK + cunit::ROSTER_OWNER, 1);
    put32(&mut bytes, TASK + cunit::LOGICAL_OWNER, 3);
    put32(&mut bytes, TASK + cunit::EXPEDITIONARY_OWNER, 0);
    put64(&mut bytes, TASK + cunit::LOCATION, PROVINCE as u64);
    put32(&mut bytes, TASK + ctask_force::MISSION, 8);
    array(&mut bytes, TASK + ctask_force::SHIPS, SHIP_ROSTER, 2);
    for (index, pointer) in [SHIP, SHIP2].into_iter().enumerate() {
        put64(&mut bytes, SHIP_ROSTER + index * 8, pointer as u64);
        put64(&mut bytes, pointer, cship::VTABLE_RVA as u64);
        put64(
            &mut bytes,
            pointer + cship::HANDLE,
            ((index as u64 + 12) << 32) | 51,
        );
        put64(&mut bytes, pointer + cship::STRENGTH, 0x1_1234_5678);
        put64(&mut bytes, pointer + cship::ORGANISATION, (-7_i64) as u64);
        put64(&mut bytes, pointer + cship::EXPERIENCE, i64::MAX as u64);
        put32(
            &mut bytes,
            pointer + cship::CLASS_DATABASE_ID,
            (-5_i32) as u32,
        );
        put32(&mut bytes, pointer + cship::CATEGORY, 10);
        put32(&mut bytes, pointer + cship::SUNK_CONVOYS, (-7_i32) as u32);
        put64(&mut bytes, pointer + cship::TASK_FORCE, TASK as u64);
    }
    put64(&mut bytes, PROVINCES + 8, PROVINCE as u64);
    put64(&mut bytes, PROVINCE, cprovince::VTABLE_RVA as u64);
    put32(&mut bytes, PROVINCE + cprovince::ID, 1);
    bytes
}

#[test]
fn cached_fuel_traversal_keeps_all_missions_without_reading_leader_or_mission_body() {
    use std::cell::RefCell;
    struct MissingBody<'a> {
        bytes: &'a [u8],
        reads: RefCell<Vec<usize>>,
    }
    impl Memory for MissingBody<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push(address);
            if [FLEET + LEADER, TASK + ctask_force::MISSION].contains(&address) {
                return false;
            }
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, output)
        }
    }
    for mission in [0, 4, 8, -1_i32] {
        let mut bytes = fixture();
        put32(&mut bytes, TASK + ctask_force::MISSION, mission as u32);
        put64(&mut bytes, TASK + 0x500, (-17_i64) as u64);
        let memory = MissingBody {
            bytes: &bytes,
            reads: RefCell::default(),
        };
        let countries = ccountry::Reader::new(&memory, 0, GS).unwrap();
        let mut rates = Vec::new();
        visit_fuel_task_forces(&memory, 0, &countries, |visit| {
            if let Visit::TaskForce(task, ordinal) = visit {
                rates.push((
                    task.id,
                    ordinal,
                    read::<i64>(&memory, task.task + 0x500).unwrap(),
                ));
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(rates, vec![((11_u64 << 32) | 51, 0, -17)]);
        assert!(!memory.reads.borrow().contains(&(FLEET + LEADER)));
        assert!(
            !memory
                .reads
                .borrow()
                .contains(&(TASK + ctask_force::MISSION))
        );
        assert!(visit_task_forces(&memory, 0, &countries, |_| Ok(())).is_err());
    }
}

fn capture(
    bytes: &[u8],
    fleets: &mut [Row],
    tasks: &mut [ctask_force::Row],
    ships: &mut [cship::Row],
) -> Capture {
    walk(&SliceMemory { base: 0, bytes }, 0, GS, fleets, tasks, ships)
}

#[test]
fn naval_graph_preserves_handles_aliases_and_full_signed_scalars() {
    let bytes = fixture();
    let mut fleets = [Row::default(); 2];
    let mut tasks = [ctask_force::Row::default(); 2];
    let mut ships = [cship::Row::default(); 3];
    let result = capture(&bytes, &mut fleets, &mut tasks, &mut ships);
    assert_eq!(
        result,
        Capture {
            fleets: Outcome::Rows { len: 1 },
            task_forces: Outcome::Rows { len: 1 },
            ships: Outcome::Rows { len: 2 }
        }
    );
    assert_eq!(
        fleets[0],
        Row {
            id: (11_u64 << 32) | 61,
            owner: 3,
            home_base: 1,
            leader: None,
            name_start: 0,
            name_len: 0,
            name_known: false,
        }
    );
    assert_eq!(
        tasks[0],
        ctask_force::Row {
            id: (11_u64 << 32) | 51,
            fleet: fleets[0].id,
            roster_owner: 1,
            logical_owner: 3,
            expeditionary_owner: 0,
            province: 1,
            mission: 8,
            route_count: 0,
            combat_references: 0,
        }
    );
    assert_eq!(
        ships[0],
        cship::Row {
            roster_ordinal: 0,
            id: (12_u64 << 32) | 51,
            task_force: tasks[0].id,
            strength: 0x1_1234_5678,
            organisation: -7,
            experience: i64::MAX,
            category: 10,
            sunk_convoys: -7,
            class_id: -5
        }
    );
}

#[test]
fn naval_capacity_is_independent_and_does_not_hide_descendants() {
    let mut bytes = fixture();
    // Body fields of an omitted ship must not be read.
    put64(&mut bytes, SHIP2, 0);
    let mut ships = [cship::Row::default(); 1];
    let result = capture(&bytes, &mut [], &mut [], &mut ships);
    assert_eq!(
        result,
        Capture {
            fleets: Outcome::Truncated {
                len: 0,
                unread_rows: 1
            },
            task_forces: Outcome::Truncated {
                len: 0,
                unread_rows: 1
            },
            ships: Outcome::Truncated {
                len: 1,
                unread_rows: 1
            },
        }
    );
}

#[test]
fn naval_empty_rosters_and_null_locations_are_valid() {
    let mut bytes = fixture();
    put64(&mut bytes, FLEET + HOME_BASE, 0);
    put64(&mut bytes, TASK + cunit::LOCATION, 0);
    array(&mut bytes, TASK + ctask_force::SHIPS, 0, 0);
    let mut fleets = [Row::default(); 1];
    let mut tasks = [ctask_force::Row::default(); 1];
    let result = capture(&bytes, &mut fleets, &mut tasks, &mut []);
    assert_eq!(result.ships, Outcome::Rows { len: 0 });
    assert_eq!(fleets[0].home_base, 0);
    assert_eq!(tasks[0].province, 0);
    array(&mut bytes, COUNTRY + ccountry::FLEETS, 0, 0);
    assert_eq!(
        capture(&bytes, &mut [], &mut [], &mut []),
        Capture {
            fleets: Outcome::Rows { len: 0 },
            task_forces: Outcome::Rows { len: 0 },
            ships: Outcome::Rows { len: 0 }
        }
    );
}

#[test]
fn naval_body_failures_are_scoped_but_broken_parent_links_are_not_silenced() {
    let mut bytes = fixture();
    let mut fleets = [Row::default(); 1];
    let mut tasks = [ctask_force::Row::default(); 1];
    let mut ships = [cship::Row::default(); 2];
    put64(&mut bytes, FLEET + HOME_BASE, 1);
    let result = capture(&bytes, &mut fleets, &mut tasks, &mut ships);
    assert!(matches!(result.fleets, Outcome::Unavailable { .. }));
    assert_eq!(result.task_forces, Outcome::Rows { len: 1 });
    assert_eq!(result.ships, Outcome::Rows { len: 2 });
    put64(&mut bytes, FLEET + HOME_BASE, 0);
    put64(&mut bytes, SHIP2, 0);
    let result = capture(&bytes, &mut fleets, &mut tasks, &mut ships);
    assert_eq!(result.fleets, Outcome::Rows { len: 1 });
    assert_eq!(result.task_forces, Outcome::Rows { len: 1 });
    assert!(matches!(result.ships, Outcome::Unavailable { .. }));
    put64(&mut bytes, TASK + ctask_force::FLEET, 0);
    let result = capture(&bytes, &mut fleets, &mut tasks, &mut ships);
    assert_eq!(result.fleets, Outcome::Rows { len: 1 });
    assert!(matches!(
        result.task_forces,
        Outcome::Unavailable {
            reason: "task force parent differs from fleet roster"
        }
    ));
}

#[test]
fn naval_broken_headers_and_location_registry_are_not_empty_data() {
    let mut bytes = fixture();
    let mut fleets = [Row::default(); 1];
    let mut tasks = [ctask_force::Row::default(); 1];
    let mut ships = [cship::Row::default(); 2];
    put64(&mut bytes, PROVINCES + 8, 0);
    let result = capture(&bytes, &mut fleets, &mut tasks, &mut ships);
    assert!(matches!(result.fleets, Outcome::Unavailable { .. }));
    assert!(matches!(result.task_forces, Outcome::Unavailable { .. }));
    assert_eq!(result.ships, Outcome::Rows { len: 2 });
    array(&mut bytes, TASK + ctask_force::SHIPS, 0, 2);
    let result = capture(&bytes, &mut fleets, &mut tasks, &mut ships);
    assert_eq!(
        result.fleets,
        Outcome::Unavailable {
            reason: "naval province differs from registered slot"
        }
    );
    assert_eq!(
        result.task_forces,
        Outcome::Unavailable {
            reason: "naval province differs from registered slot"
        }
    );
    assert!(matches!(
        result.ships,
        Outcome::Unavailable {
            reason: "ship roster unreadable or invalid"
        }
    ));
}

#[test]
fn counter_inputs_read_mission_links_colour_and_ordered_roster_with_bounds() {
    const REGIONS: usize = 0xb300;
    const REGION: usize = 0xb400;
    use crate::{cnaval_mission as mission, cstrategic_region};
    let mut bytes = fixture();
    let root = TASK + ctask_force::NAVAL_MISSION;
    put64(&mut bytes, root, mission::VTABLE_RVA as u64);
    put64(&mut bytes, root + mission::TASK_FORCE, TASK as u64);
    bytes[root + mission::IS_IN_REGIONS] = 255;
    bytes[root + mission::RUNNING_AWAY] = 1;
    put32(&mut bytes, root + mission::ENGAGEMENT_RULE, 3);
    array(&mut bytes, root + mission::ACCESSIBLE_REGIONS, REGIONS, 5);
    put64(
        &mut bytes,
        root + mission::BOMBARDMENT_REGION,
        REGION as u64,
    );
    put64(&mut bytes, REGION, cstrategic_region::VTABLE_RVA as u64);
    put32(&mut bytes, REGION + cstrategic_region::ID, 2);
    array(&mut bytes, GS + cgame_state::STRATEGIC_REGIONS, REGIONS, 3);
    put64(&mut bytes, REGIONS + 16, REGION as u64);
    put64(
        &mut bytes,
        TASK + ctask_force::REPAIR_PARENT,
        (564_u64 << 32) | 51,
    );
    put32(&mut bytes, TASK + ctask_force::DETACHED_ACTIVITY, 2);
    put32(&mut bytes, TASK + ctask_force::REPAIR_LAST_MISSION, 7);
    put64(&mut bytes, TASK + cunit::RECEIVED_SUPPLY_RATIO, 35_000);
    for (index, word) in [0x3f80_0000_u32, 0, 0x8000_0000, u32::MAX]
        .into_iter()
        .enumerate()
    {
        put32(&mut bytes, FLEET + COLOR_COMPONENTS + index * 4, word);
    }
    array(&mut bytes, FLEET + MISSION_REGIONS, REGIONS, 4);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut fleets = [CounterRow::default(); 1];
    let mut rosters = [0_u64; 1];
    let mut tasks = [ctask_force::CounterRow::default(); 1];
    let census = crate::read_census::ReadCensus::new(&memory);
    let walk = walk_counter_inputs(
        &census,
        0,
        GS,
        &mut fleets,
        &mut rosters,
        &mut tasks,
        |_, _| {},
    );
    let complete_cost = (census.calls(), census.bytes());
    assert_eq!(walk.fleets, Outcome::Rows { len: 1 });
    assert_eq!(walk.task_forces, Outcome::Rows { len: 1 });
    assert_eq!((walk.roster_len, walk.roster_required), (1, 1));
    assert_eq!(rosters, [(11_u64 << 32) | 51]);
    assert_eq!(
        fleets[0],
        CounterRow {
            id: (11_u64 << 32) | 61,
            rgba: [0x3f80_0000, 0, 0x8000_0000, u32::MAX],
            mission_region_count: 4,
            start: 0,
            len: 1,
        }
    );
    assert_eq!(
        tasks[0],
        ctask_force::CounterRow {
            id: (11_u64 << 32) | 51,
            is_in_regions: 255,
            running_away: 1,
            engagement_rule: 3,
            mission_region_count: 5,
            bombardment_region: 2,
            repair_parent: (564_u64 << 32) | 51,
            detached_activity: 2,
            repair_last_mission: 7,
            received_supply_ratio: 35_000,
            attached_headquarters: 0,
        }
    );
    census.reset();
    assert_eq!(
        ctask_force::read_counter_inputs(&census, 0, GS, TASK, tasks[0].id),
        Ok(tasks[0])
    );
    assert_eq!(
        (census.calls(), census.spans(), census.bytes()),
        (7, 12, 116)
    );
    // A roster that does not fit omits the fleet whole; task forces are independent.
    census.reset();
    let walk = walk_counter_inputs(&census, 0, GS, &mut fleets, &mut [], &mut tasks, |_, _| {});
    assert_eq!(
        walk.fleets,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    assert_eq!(walk.task_forces, Outcome::Rows { len: 1 });
    assert_eq!(walk.roster_required, 1);
    assert_eq!(
        (
            complete_cost.0 - census.calls(),
            complete_cost.1 - census.bytes()
        ),
        (2, 32),
        "an emitted fleet adds only colour and mission-region fields"
    );
    // A region outside its registered slot and a foreign mission vtable both fail.
    put64(&mut bytes, REGIONS + 16, 0);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let walk = walk_counter_inputs(
        &memory,
        0,
        GS,
        &mut fleets,
        &mut rosters,
        &mut tasks,
        |_, _| {},
    );
    assert_eq!(
        walk.task_forces,
        Outcome::Unavailable {
            reason: "bombardment region differs from registered slot"
        }
    );
    put64(&mut bytes, REGIONS + 16, REGION as u64);
    put64(&mut bytes, root, 1);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let walk = walk_counter_inputs(
        &memory,
        0,
        GS,
        &mut fleets,
        &mut rosters,
        &mut tasks,
        |_, _| {},
    );
    assert_eq!(
        walk.task_forces,
        Outcome::Unavailable {
            reason: "naval mission identity mismatch"
        }
    );
}

/// The fleet walk visits a task force before it reads the counter row, which checks the
/// mission too; with the counter buffer exhausted the mission collector still authenticates
/// every mission it observes.
#[test]
fn missions_are_authenticated_when_the_counter_buffer_is_exhausted() {
    use crate::cnaval_mission as mission;
    const REGIONS: usize = 0xb300;
    const REGION: usize = 0xb400;
    const ACCESSIBLE: usize = 0xb500;
    let mut bytes = fixture();
    let root = TASK + ctask_force::NAVAL_MISSION;
    put64(&mut bytes, root, mission::VTABLE_RVA as u64);
    put64(&mut bytes, root + mission::TASK_FORCE, TASK as u64);
    array(&mut bytes, GS + cgame_state::STRATEGIC_REGIONS, REGIONS, 2);
    put64(&mut bytes, REGIONS + 8, REGION as u64);
    put32(&mut bytes, REGION + crate::cstrategic_region::ID, 1);
    array(
        &mut bytes,
        root + mission::ACCESSIBLE_REGIONS,
        ACCESSIBLE,
        1,
    );
    put64(&mut bytes, ACCESSIBLE, REGION as u64);
    let run = |bytes: &[u8]| {
        let memory = SliceMemory { base: 0, bytes };
        let (mut missions, mut arena) = ([ctask_force::MissionRow::default(); 1], [0; 1]);
        let mut collector =
            ctask_force::MissionCollector::new(&memory, 0, GS, &mut missions, &mut arena);
        let walk = walk_counter_inputs(&memory, 0, GS, &mut [], &mut [], &mut [], |pointer, id| {
            collector.visit(pointer, id)
        });
        assert_eq!(
            walk.task_forces,
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        collector.finish(None).outcome
    };
    assert_eq!(run(&bytes), Outcome::Rows { len: 1 });
    for (at, value) in [(root, 1), (root + mission::TASK_FORCE, 0)] {
        let mut forged = bytes.clone();
        put64(&mut forged, at, value);
        assert_eq!(
            run(&forged),
            Outcome::Unavailable {
                reason: "naval mission identity mismatch"
            },
            "{at:#x}"
        );
    }
}
