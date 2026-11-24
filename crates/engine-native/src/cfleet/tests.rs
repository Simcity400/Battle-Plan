use super::*;
use crate::cunit;
use engine_native_memory::memory::SliceMemory;

struct OriginalCounterMemory<'a> {
    bytes: &'a [u8],
    fail_once_at: Option<usize>,
    failed: std::cell::Cell<bool>,
    reads: std::cell::RefCell<Vec<(usize, usize)>>,
}

impl Memory for OriginalCounterMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        if self.fail_once_at == Some(address) && !self.failed.replace(true) {
            return false;
        }
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(address, output)
    }
}

fn original_counter_fixture() -> Vec<u8> {
    let mut bytes = fixture();
    let root = TASK + ctask_force::NAVAL_MISSION;
    put64(&mut bytes, root, engine_native_parts::cnaval_mission::VTABLE_RVA.0 as u64);
    put64(
        &mut bytes,
        root + engine_native_parts::cnaval_mission::TASK_FORCE,
        TASK as u64,
    );
    bytes[root + engine_native_parts::cnaval_mission::IS_IN_REGIONS] = 255;
    put64(
        &mut bytes,
        TASK + cunit::RECEIVED_SUPPLY_RATIO,
        (-35_001_i64) as u64,
    );
    bytes
}

#[test]
fn original_counter_borrower_receives_same_prefix_without_an_extra_native_read() {
    let bytes = original_counter_fixture();
    let legacy = OriginalCounterMemory {
        bytes: &bytes,
        fail_once_at: None,
        failed: std::cell::Cell::new(false),
        reads: Default::default(),
    };
    let borrowed = OriginalCounterMemory {
        bytes: &bytes,
        fail_once_at: None,
        failed: std::cell::Cell::new(false),
        reads: Default::default(),
    };
    let mut expected_fleets = vec![CounterRow::default(); 1];
    let mut expected_tasks = vec![ctask_force::CounterRow::default(); 1];
    let mut expected_rosters = vec![0; 1];
    let expected = walk_counter_inputs(
        &legacy,
        0,
        GS,
        &mut expected_fleets,
        &mut expected_rosters,
        &mut expected_tasks,
        |_, _| {},
    );
    let mut fleets = vec![CounterRow::default(); 1];
    let mut tasks = vec![ctask_force::CounterRow::default(); 1];
    let mut rosters = vec![0; 1];
    let mut observed = Vec::new();
    let actual = walk_counter_inputs_with_port_observed(
        &borrowed,
        0,
        GS,
        &mut fleets,
        &mut rosters,
        (&mut tasks, None, None, None),
        |_, _| {},
        |pointer, id, original| observed.push((pointer, id, original)),
    );
    assert_eq!(actual, expected);
    assert_eq!(tasks, expected_tasks);
    assert_eq!(
        observed,
        vec![(TASK, (11_u64 << 32) | 51, Some(Ok(tasks[0])))]
    );
    assert_eq!(
        (tasks[0].is_in_regions, tasks[0].received_supply_ratio),
        (255, -35_001)
    );
    assert_eq!(
        borrowed.reads.borrow().as_slice(),
        legacy.reads.borrow().as_slice()
    );
    for address in [
        TASK + ctask_force::NAVAL_MISSION,
        TASK + cunit::RECEIVED_SUPPLY_RATIO,
    ] {
        assert_eq!(
            borrowed
                .reads
                .borrow()
                .iter()
                .filter(|(at, _)| *at == address)
                .count(),
            1
        );
    }
}

#[test]
fn fleet_observer_token_precedes_the_original_region_header_without_extra_reads() {
    use std::cell::Cell;
    struct HeaderRace<'a> {
        original: OriginalCounterMemory<'a>,
        sequence: Cell<u64>,
    }
    impl Memory for HeaderRace<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let copied = self.original.copy(address, output);
            if copied && address == FLEET + MISSION_REGIONS {
                self.sequence.set(self.sequence.get() + 1);
            }
            copied
        }
    }
    let bytes = original_counter_fixture();
    let original = || OriginalCounterMemory {
        bytes: &bytes,
        fail_once_at: None,
        failed: Cell::new(false),
        reads: Default::default(),
    };
    let legacy = original();
    let mut old_fleets = vec![];
    let mut old_rosters = vec![];
    let mut old_tasks = vec![];
    let expected = walk_counter_inputs_with_port_observed(
        &legacy,
        0,
        GS,
        &mut old_fleets,
        &mut old_rosters,
        (&mut old_tasks, None, None, None),
        |_, _| {},
        |_, _, _| {},
    );
    let memory = HeaderRace {
        original: original(),
        sequence: Cell::new(1),
    };
    let mut fleets = vec![];
    let mut rosters = vec![];
    let mut tasks = vec![];
    let mut observed = vec![];
    let actual = walk_counter_inputs_with_port_and_fleet_observed(
        &memory,
        0,
        GS,
        &mut fleets,
        &mut rosters,
        (&mut tasks, None, None, None),
        (|_: usize| (), |_: usize, _: u64| {}),
        |_, _, _| {},
        |_, _| memory.sequence.get(),
        |pointer, id, header, before| {
            observed.push((pointer, id, header.count, before, memory.sequence.get()))
        },
    );
    assert_eq!(actual, expected);
    assert_eq!(fleets, old_fleets);
    assert_eq!(tasks, old_tasks);
    assert_eq!(rosters, old_rosters);
    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].0, FLEET);
    assert_eq!((observed[0].3, observed[0].4), (1, 2));
    assert_eq!(*memory.original.reads.borrow(), *legacy.reads.borrow());
}

#[test]
fn growing_counters_follow_a_native_roster_larger_than_the_old_chosen_limit() {
    let mut bytes = original_counter_fixture();
    let count = 65_537_usize;
    let pointers = bytes.len();
    bytes.resize(pointers + count * 8, 0);
    array(&mut bytes, FLEET + TASK_FORCES, pointers, count as u32);
    for index in 0..count {
        put64(&mut bytes, pointers + index * 8, TASK as u64);
    }
    let memory = OriginalCounterMemory {
        bytes: &bytes,
        fail_once_at: None,
        failed: std::cell::Cell::new(false),
        reads: Default::default(),
    };
    let mut fleets = Vec::new();
    let mut tasks = Vec::new();
    let mut rosters = Vec::new();
    let actual = walk_counter_inputs_with_port_observed(
        &memory,
        0,
        GS,
        &mut fleets,
        &mut rosters,
        (&mut tasks, None, None, None),
        |_, _| {},
        |_, _, _| {},
    );
    assert_eq!(actual.fleets, Outcome::Rows { len: 1 });
    assert_eq!(actual.task_forces, Outcome::Rows { len: count });
    assert_eq!((actual.roster_len, actual.roster_required), (count, count));
    assert_eq!(fleets[0].len, count);
    assert_eq!(tasks[count - 1].id, (11_u64 << 32) | 51);
    assert_eq!(rosters[count - 1], tasks[count - 1].id);
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at == FLEET + TASK_FORCES)
            .count(),
        1
    );
}

#[test]
fn growing_counter_names_follow_original_headers_on_first_and_later_publications() {
    let mut fleets = Vec::new();
    let mut tasks = Vec::new();
    let mut rosters = Vec::new();
    let mut text = Vec::new();
    for count in [50_usize, 120] {
        let mut bytes = original_counter_fixture();
        let name = b"Retained task force name including a mod-authored suffix";
        let name_data = bytes.len();
        bytes.extend_from_slice(name);
        bytes.push(0);
        put64(&mut bytes, TASK + 0x258, name_data as u64);
        put64(&mut bytes, TASK + 0x258 + 16, name.len() as u64);
        put64(&mut bytes, TASK + 0x258 + 24, name.len() as u64);
        let pointers = bytes.len();
        bytes.resize(pointers + count * 8, 0);
        array(&mut bytes, FLEET + TASK_FORCES, pointers, count as u32);
        for index in 0..count {
            put64(&mut bytes, pointers + index * 8, TASK as u64);
        }
        let memory = OriginalCounterMemory {
            bytes: &bytes,
            fail_once_at: None,
            failed: std::cell::Cell::new(false),
            reads: Default::default(),
        };
        let mut names = crate::cname_group_member::Names {
            bytes: engine_native_memory::walk::Buffer::Growing(&mut text),
            len: 0,
            required: 0,
        };
        let capture = walk_counter_inputs_with_port_observed(
            &memory,
            0,
            GS,
            &mut fleets,
            &mut rosters,
            (&mut tasks, Some(&mut names), None, None),
            |_, _| {},
            |_, _, _| {},
        );
        assert_eq!(capture.task_forces, Outcome::Rows { len: count });
        assert_eq!(
            (names.len, names.required),
            (count * name.len(), count * name.len())
        );
        for (index, task) in tasks[..count].iter().enumerate() {
            let span = task.name.unwrap();
            assert_eq!(span, (index * name.len(), name.len()));
            assert_eq!(&names.bytes[span.0..span.0 + span.1], name);
        }
        assert_eq!(
            memory
                .reads
                .borrow()
                .iter()
                .filter(|(at, len)| *at == TASK + 0x258 && *len == 32)
                .count(),
            count
        );
    }
}

#[test]
fn original_counter_borrower_reports_a_failed_prefix() {
    let bytes = original_counter_fixture();
    let root = TASK + ctask_force::NAVAL_MISSION;
    let memory = OriginalCounterMemory {
        bytes: &bytes,
        fail_once_at: Some(root),
        failed: std::cell::Cell::new(false),
        reads: Default::default(),
    };
    let mut tasks = Vec::new();
    let mut observed = Vec::new();
    let actual = walk_counter_inputs_with_port_observed(
        &memory,
        0,
        GS,
        &mut Vec::new(),
        &mut Vec::new(),
        (&mut tasks, None, None, None),
        |_, _| {},
        |pointer, id, original| observed.push((pointer, id, original)),
    );
    assert_eq!(
        observed,
        vec![(
            TASK,
            (11_u64 << 32) | 51,
            Some(Err("naval mission identity unreadable"))
        )]
    );
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at == root)
            .count(),
        1
    );
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|(at, _)| *at == TASK + cunit::RECEIVED_SUPPLY_RATIO)
    );
    assert_eq!(
        actual.task_forces,
        Outcome::Unavailable {
            reason: "naval mission identity unreadable"
        }
    );
}

#[test]
fn ship_visitors_receive_only_authenticated_emitted_rows_and_never_hide_incomplete_censuses() {
    for corrupt_second in [true, false] {
        let mut bytes = fixture();
        if corrupt_second {
            put64(&mut bytes, SHIP2, 0);
        }
        let mut fleets = Vec::new();
        let mut tasks = Vec::new();
        let mut ships = Vec::new();
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
            &mut ships,
            |_| 0,
            |pointer, row| visited.push((pointer, *row)),
        );
        assert_eq!(captured.capture.fleets, Outcome::Rows { len: 1 });
        assert_eq!(captured.capture.task_forces, Outcome::Rows { len: 1 });
        if corrupt_second {
            assert_eq!(visited, vec![(SHIP, ships[0])]);
            assert!(matches!(
                captured.capture.ships,
                Outcome::Unavailable { .. }
            ));
        } else {
            assert_eq!(visited, vec![(SHIP, ships[0]), (SHIP2, ships[1])]);
            assert_eq!(captured.capture.ships, Outcome::Rows { len: 2 });
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
    put64(&mut bytes, COUNTRY, ccountry::VTABLE_RVA.0 as u64);
    put32(&mut bytes, COUNTRY + ccountry::ID, 3);
    array(&mut bytes, COUNTRY + ccountry::FLEETS, FLEET_ROSTER, 1);
    put64(&mut bytes, FLEET_ROSTER, FLEET as u64);
    put64(&mut bytes, FLEET, VTABLE_RVA.0 as u64);
    put64(&mut bytes, FLEET + HANDLE, (11_u64 << 32) | 61);
    put64(&mut bytes, FLEET + OWNER, COUNTRY as u64);
    put64(&mut bytes, FLEET + HOME_BASE, PROVINCE as u64);
    array(&mut bytes, FLEET + TASK_FORCES, TASK_ROSTER, 1);
    put64(&mut bytes, TASK_ROSTER, TASK as u64);
    put64(&mut bytes, TASK, ctask_force::VTABLE_RVA.0 as u64);
    put64(
        &mut bytes,
        TASK + ctask_force::SECONDARY_BASE,
        ctask_force::SECONDARY_VTABLE_RVA.0 as u64,
    );
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
        put64(&mut bytes, pointer, cship::VTABLE_RVA.0 as u64);
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
    put64(&mut bytes, PROVINCE, cprovince::VTABLE_RVA.0 as u64);
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
    fleets: &mut Vec<Row>,
    tasks: &mut Vec<ctask_force::Row>,
    ships: &mut Vec<cship::Row>,
) -> Capture {
    walk(&SliceMemory { base: 0, bytes }, 0, GS, fleets, tasks, ships)
}

#[test]
fn naval_graph_preserves_handles_aliases_and_full_signed_scalars() {
    let bytes = fixture();
    let mut fleets = vec![Row::default(); 2];
    let mut tasks = vec![ctask_force::Row::default(); 2];
    let mut ships = vec![cship::Row::default(); 3];
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
            name: None,
            naming: None,
            acquired_manpower: None,
            required_manpower: None,
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
fn foreign_task_force_is_recorded_with_its_own_owners() {
    const FOREIGN: usize = 0x5000;
    let mut bytes = fixture();
    put64(&mut bytes, COUNTRIES + 16, FOREIGN as u64);
    put32(&mut bytes, LINKS + 8, 2);
    put64(&mut bytes, FOREIGN, ccountry::VTABLE_RVA.0 as u64);
    put32(&mut bytes, FOREIGN + ccountry::ID, 2);
    put32(&mut bytes, TASK + cunit::ROSTER_OWNER, 2);
    put32(&mut bytes, TASK + cunit::LOGICAL_OWNER, 2);
    put32(&mut bytes, TASK + cunit::EXPEDITIONARY_OWNER, 3);
    let mut fleets = vec![Row::default(); 1];
    let mut tasks = vec![ctask_force::Row::default(); 1];
    let mut ships = vec![cship::Row::default(); 2];
    let result = capture(&bytes, &mut fleets, &mut tasks, &mut ships);
    assert_eq!(
        result,
        Capture {
            fleets: Outcome::Rows { len: 1 },
            task_forces: Outcome::Rows { len: 1 },
            ships: Outcome::Rows { len: 2 }
        }
    );
    assert_eq!(fleets[0].owner, 3);
    assert_eq!(
        (
            tasks[0].fleet,
            tasks[0].roster_owner,
            tasks[0].logical_owner,
            tasks[0].expeditionary_owner
        ),
        (fleets[0].id, 2, 2, 3)
    );
    assert_eq!(
        ships.iter().map(|ship| ship.task_force).collect::<Vec<_>>(),
        [tasks[0].id; 2]
    );
}

#[test]
fn ship_census_reads_a_run_of_ships_in_one_guarded_call_and_its_roster_a_page_per_call() {
    let calls = |bytes: &[u8], ships: engine_native_memory::walk::Buffer<'_, cship::Row>| {
        let memory = engine_native_memory::memory::CountingMemory::new(bytes);
        let mut fleets = vec![Row::default(); 1];
        let mut tasks = vec![ctask_force::Row::default(); 1];
        let capture = walk_named(
            &memory,
            0,
            GS,
            engine_native_memory::walk::Buffer::Growing(&mut fleets),
            engine_native_memory::walk::Buffer::Growing(&mut tasks),
            ships,
            |_| 0,
            |_, _| {},
            None,
            true,
        );
        (capture.capture.ships, memory.calls.get())
    };
    let bytes = fixture();
    let mut ships = Vec::new();
    let (read, with_bodies) = calls(&bytes, engine_native_memory::walk::Buffer::Growing(&mut ships));
    assert_eq!(read, Outcome::Rows { len: 2 });
    assert_eq!(
        ships.iter().map(|ship| ship.roster_ordinal).collect::<Vec<_>>(),
        [0, 1],
        "roster order is kept"
    );
    let (_, roster_only) = calls(&bytes, engine_native_memory::walk::Buffer::Count);
    assert_eq!(with_bodies - roster_only, 1, "one call per run of ships");
    let mut empty = fixture();
    array(&mut empty, TASK + ctask_force::SHIPS, 0, 0);
    let (_, no_roster) = calls(&empty, engine_native_memory::walk::Buffer::Count);
    assert_eq!(roster_only - no_roster, 1, "one call per roster page");

    struct Faulting<'a> {
        bytes: &'a [u8],
        fault: usize,
    }
    impl Memory for Faulting<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            !(address..address + destination.len()).contains(&self.fault)
                && SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(address, destination)
        }
    }
    let census = |bytes: &[u8], fault| {
        let mut fleets = vec![Row::default(); 1];
        let mut tasks = vec![ctask_force::Row::default(); 1];
        let mut ships = vec![cship::Row::default(); 2];
        let memory = Faulting { bytes, fault };
        walk(&memory, 0, GS, &mut fleets, &mut tasks, &mut ships).ships
    };
    let mut wrong = fixture();
    put64(&mut wrong, SHIP2, 0);
    let bytes = fixture();
    for (bytes, fault, reason) in [
        (&wrong, 0, "ship vtable mismatch"),
        (&bytes, SHIP2, "ship vtable mismatch"),
        (&bytes, SHIP2 + cship::TASK_FORCE, "ship fields unreadable"),
    ] {
        assert_eq!(census(bytes, fault), Outcome::Unavailable { reason });
    }
}

#[test]
fn the_naval_count_probe_counts_each_table_and_reads_no_omitted_ship_body() {
    let mut bytes = fixture();
    put64(&mut bytes, SHIP2, 0);
    assert_eq!(
        count(&SliceMemory { base: 0, bytes: &bytes }, 0, GS),
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
                len: 0,
                unread_rows: 2
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
    let mut fleets = vec![Row::default(); 1];
    let mut tasks = vec![ctask_force::Row::default(); 1];
    let result = capture(&bytes, &mut fleets, &mut tasks, &mut Vec::new());
    assert_eq!(result.ships, Outcome::Rows { len: 0 });
    assert_eq!(fleets[0].home_base, 0);
    assert_eq!(tasks[0].province, 0);
    array(&mut bytes, COUNTRY + ccountry::FLEETS, 0, 0);
    assert_eq!(
        capture(&bytes, &mut Vec::new(), &mut Vec::new(), &mut Vec::new()),
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
    let mut fleets = vec![Row::default(); 1];
    let mut tasks = vec![ctask_force::Row::default(); 1];
    let mut ships = vec![cship::Row::default(); 2];
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
    let mut fleets = vec![Row::default(); 1];
    let mut tasks = vec![ctask_force::Row::default(); 1];
    let mut ships = vec![cship::Row::default(); 2];
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
    use engine_native_parts::{cnaval_mission as mission, cstrategic_region};
    let mut bytes = fixture();
    let root = TASK + ctask_force::NAVAL_MISSION;
    put64(&mut bytes, root, mission::VTABLE_RVA.0 as u64);
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
    put64(&mut bytes, REGION, cstrategic_region::VTABLE_RVA.0 as u64);
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
    let mut fleets = vec![CounterRow::default(); 1];
    let mut rosters = vec![0_u64; 1];
    let mut tasks = vec![ctask_force::CounterRow::default(); 1];
    let census = engine_native_memory::read_census::ReadCensus::new(&memory);
    let walk = walk_counter_inputs(
        &census,
        0,
        GS,
        &mut fleets,
        &mut rosters,
        &mut tasks,
        |_, _| {},
    );
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
            name: None,
            supply: None,
            supply_origins_start: 0,
            supply_failure: None,
            port_mission: None,
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

#[test]
fn task_force_cached_name_failure_preserves_counter_prefix_and_admission_order() {
    use crate::cname_group_member::Names;
    use engine_native_parts::cnaval_mission;
    let mut bytes = fixture();
    let root = TASK + ctask_force::NAVAL_MISSION;
    put64(&mut bytes, root, cnaval_mission::VTABLE_RVA.0 as u64);
    put64(&mut bytes, root + cnaval_mission::TASK_FORCE, TASK as u64);
    array(
        &mut bytes,
        root + cnaval_mission::ACCESSIBLE_REGIONS,
        0xb300,
        0,
    );
    array(&mut bytes, FLEET + MISSION_REGIONS, 0xb300, 0);
    put64(&mut bytes, TASK + cunit::RECEIVED_SUPPLY_RATIO, 35_000);
    let name = TASK + 0x258;
    bytes[name..name + 6].copy_from_slice(b"Guard\0");
    put64(&mut bytes, name + 16, 6);
    put64(&mut bytes, name + 24, 15);
    let handle = (11_u64 << 32) | 51;
    let prefix = ctask_force::read_counter_inputs(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        TASK,
        handle,
    )
    .unwrap();
    let mut fleets = vec![CounterRow::default(); 1];
    let mut tasks = vec![ctask_force::CounterRow::default(); 1];
    let mut rosters = vec![0; 1];
    for capacity in [0, 6] {
        let mut owned = vec![0; capacity];
        let mut names = Names {
            bytes: engine_native_memory::walk::Buffer::Growing(&mut owned),
            len: 0,
            required: 0,
        };
        let mut visited = Vec::new();
        let captured = walk_counter_inputs_with_names(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            GS,
            &mut fleets,
            &mut rosters,
            (&mut tasks, Some(&mut names)),
            |pointer, id| visited.push((pointer, id)),
        );
        assert_eq!(captured.task_forces, Outcome::Rows { len: 1 });
        assert_eq!(visited, [(TASK, handle)]);
        assert_eq!(rosters, [handle]);
        assert_eq!(names.required, 6);
        assert_eq!(names.len, 6, "capacity {capacity}");
        assert_eq!(tasks[0].name, Some((0, 6)));
        let mut actual = tasks[0];
        actual.name = None;
        assert_eq!(actual, prefix);
        assert_eq!(&owned[..6], b"Guard\0");
    }
    put64(&mut bytes, name + 24, 1);
    let mut names = Names {
        bytes: engine_native_memory::walk::Buffer::Count,
        len: 0,
        required: 0,
    };
    let captured = walk_counter_inputs_with_names(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        &mut fleets,
        &mut rosters,
        (&mut tasks, Some(&mut names)),
        |_, _| {},
    );
    assert_eq!(captured.task_forces, Outcome::Rows { len: 1 });
    assert_eq!(tasks[0], prefix);
    assert_eq!(names.required, 0);

    struct Traced<'a> {
        bytes: &'a [u8],
        reads: std::cell::RefCell<Vec<usize>>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push(address);
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, output)
        }
    }
    put64(&mut bytes, root, 0);
    let memory = Traced {
        bytes: &bytes,
        reads: std::cell::RefCell::default(),
    };
    let captured = walk_counter_inputs_with_names(
        &memory,
        0,
        GS,
        &mut fleets,
        &mut rosters,
        (&mut tasks, Some(&mut names)),
        |_, _| {},
    );
    assert!(matches!(captured.task_forces, Outcome::Unavailable { .. }));
    assert!(
        !memory.reads.borrow().contains(&name),
        "failed prefixes acquire no name suffix"
    );
}

#[test]
fn missions_are_authenticated_by_the_counter_walk_and_the_mission_collector() {
    use engine_native_parts::cnaval_mission as mission;
    const REGIONS: usize = 0xb300;
    const REGION: usize = 0xb400;
    const ACCESSIBLE: usize = 0xb500;
    let mut bytes = fixture();
    let root = TASK + ctask_force::NAVAL_MISSION;
    put64(&mut bytes, root, mission::VTABLE_RVA.0 as u64);
    put64(&mut bytes, root + mission::TASK_FORCE, TASK as u64);
    array(&mut bytes, GS + cgame_state::STRATEGIC_REGIONS, REGIONS, 2);
    put64(&mut bytes, REGIONS + 8, REGION as u64);
    put32(&mut bytes, REGION + engine_native_parts::cstrategic_region::ID, 1);
    array(
        &mut bytes,
        root + mission::ACCESSIBLE_REGIONS,
        ACCESSIBLE,
        1,
    );
    put64(&mut bytes, ACCESSIBLE, REGION as u64);
    let run = |bytes: &[u8]| {
        let memory = SliceMemory { base: 0, bytes };
        let (mut missions, mut arena) = (Vec::new(), Vec::new());
        let mut collector =
            ctask_force::MissionCollector::new(&memory, 0, GS, &mut missions, &mut arena);
        let walk = walk_counter_inputs(&memory, 0, GS, &mut Vec::new(), &mut Vec::new(), &mut Vec::new(), |pointer, id| {
            collector.visit(pointer, id)
        });
        (walk.task_forces, collector.finish(None).outcome)
    };
    assert_eq!(
        run(&bytes),
        (Outcome::Rows { len: 1 }, Outcome::Rows { len: 1 })
    );
    let mismatch = Outcome::Unavailable {
        reason: "naval mission identity mismatch",
    };
    for (at, value) in [(root, 1), (root + mission::TASK_FORCE, 0)] {
        let mut forged = bytes.clone();
        put64(&mut forged, at, value);
        assert_eq!(run(&forged), (mismatch, mismatch), "{at:#x}");
    }
}

#[test]
fn task_force_supply_uses_the_admitted_primary_root_and_preserves_name_and_scalar_fragments() {
    use crate::cname_group_member::Names;
    use engine_native_parts::{cnaval_mission, cunit_supply_inputs as supply};
    let mut bytes = fixture();
    let root = TASK + ctask_force::NAVAL_MISSION;
    put64(&mut bytes, root, cnaval_mission::VTABLE_RVA.0 as u64);
    put64(&mut bytes, root + cnaval_mission::TASK_FORCE, TASK as u64);
    array(
        &mut bytes,
        root + cnaval_mission::ACCESSIBLE_REGIONS,
        0xb300,
        0,
    );
    array(&mut bytes, FLEET + MISSION_REGIONS, 0xb300, 0);
    let name = TASK + 0x258;
    bytes[name..name + 5].copy_from_slice(b"Navy\0");
    put64(&mut bytes, name + 16, 4);
    put64(&mut bytes, name + 24, 15);
    put64(&mut bytes, TASK + supply::ASKED, 200_000);
    put64(&mut bytes, TASK + supply::RECEIVED, 31_000);
    put32(&mut bytes, TASK + supply::SHIP_COUNT, 2);
    put64(&mut bytes, TASK + supply::SHIP_DEMAND, 200_000);
    put64(&mut bytes, TASK + supply::DISRUPTION, (-9_i64) as u64);
    array(&mut bytes, TASK + supply::ORIGINS, 0xb700, 2);
    for (index, (requested, received)) in [(3_i64, 5_i64), (-7, -11)].into_iter().enumerate() {
        let origin = 0xb700 + index * supply::ORIGIN_STRIDE;
        put64(
            &mut bytes,
            origin + supply::ORIGIN_REQUESTED,
            requested as u64,
        );
        put64(
            &mut bytes,
            origin + supply::ORIGIN_RECEIVED,
            received as u64,
        );
        put32(&mut bytes, origin + supply::ORIGIN_PROVINCE, 13);
    }
    let prefix = ctask_force::read_counter_inputs(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        TASK,
        (11 << 32) | 51,
    )
    .unwrap();
    for bad_name in [false, true] {
        put64(&mut bytes, name + 24, if bad_name { 1 } else { 15 });
        {
            let mut name_bytes = vec![0; 4];
            let mut names = Names {
                bytes: engine_native_memory::walk::Buffer::Growing(&mut name_bytes),
                len: 0,
                required: 0,
            };
            let mut origin_bytes = Vec::new();
            let mut origins = ctask_force::SupplyOrigins {
                origins: engine_native_memory::walk::Buffer::Growing(&mut origin_bytes),
                len: 0,
                required: 0,
            };
            let (mut fleets, mut tasks, mut rosters) = (Vec::new(), Vec::new(), Vec::new());
            let walk = walk_counter_inputs_with_tooltips(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                0,
                GS,
                &mut fleets,
                &mut rosters,
                (&mut tasks, Some(&mut names), Some(&mut origins)),
                |_, _| {},
            );
            assert_eq!(walk.task_forces, Outcome::Rows { len: 1 });
            let input = tasks[0].supply.unwrap();
            assert_eq!(
                (input.asked, input.received, input.disruption),
                (200_000, 31_000, -9)
            );
            assert_eq!(
                (input.ship_demand.count, input.ship_demand.value),
                (2, 200_000)
            );
            assert_eq!(origins.required, 2);
            assert_eq!(origins.len, 2);
            assert_eq!(tasks[0].name, (!bad_name).then_some((0, 4)));
            assert_eq!(input.origins_len, Ok(2));
            assert_eq!(
                origins.origins[1],
                supply::Origin {
                    province: 13,
                    requested: -7,
                    received: -11
                }
            );
            let mut actual = tasks[0];
            actual.name = None;
            actual.supply = None;
            actual.supply_origins_start = 0;
            actual.supply_failure = None;
            assert_eq!(
                actual, prefix,
                "optional fields preserve every admitted prefix word"
            );
        }
    }
}

#[test]
fn growing_task_force_supply_and_port_arenas_capture_every_owner_before_publication() {
    use engine_native_parts::cunit_supply_inputs as supply;
    use ctask_force::port_mission::{Collector, ReferenceAnswer, Searcher};
    const COUNT: usize = 3;
    const BLOCK: usize = 0x1000;
    let mut bytes = fixture();
    let countries = bytes.len();
    let links = countries + (COUNT + 1) * 8;
    let blocks = links + (COUNT + 1) * 4;
    bytes.resize(blocks + COUNT * BLOCK, 0);
    array(
        &mut bytes,
        GS + cgame_state::COUNTRIES,
        countries,
        COUNT as u32 + 1,
    );
    array(
        &mut bytes,
        GS + cgame_state::COUNTRY_LINKS,
        links,
        COUNT as u32 + 1,
    );
    for index in 0..COUNT {
        let country = blocks + index * BLOCK;
        let fleet = country + 0x300;
        let task = country + 0x500;
        let fleet_roster = country + 0xd00;
        let task_roster = country + 0xd08;
        let slot = index as u32 + 1;
        put64(&mut bytes, countries + (index + 1) * 8, country as u64);
        put32(&mut bytes, links + (index + 1) * 4, slot);
        put64(&mut bytes, country, ccountry::VTABLE_RVA.0 as u64);
        put32(&mut bytes, country + ccountry::ID, slot);
        array(&mut bytes, country + ccountry::FLEETS, fleet_roster, 1);
        put64(&mut bytes, fleet_roster, fleet as u64);
        put64(&mut bytes, fleet, VTABLE_RVA.0 as u64);
        put64(&mut bytes, fleet + HANDLE, ((index as u64 + 1) << 32) | 61);
        put64(&mut bytes, fleet + OWNER, country as u64);
        array(&mut bytes, fleet + TASK_FORCES, task_roster, 1);
        put64(&mut bytes, task_roster, task as u64);
        put64(&mut bytes, task, ctask_force::VTABLE_RVA.0 as u64);
        put64(
            &mut bytes,
            task + ctask_force::SECONDARY_BASE,
            ctask_force::SECONDARY_VTABLE_RVA.0 as u64,
        );
        put64(
            &mut bytes,
            task + cunit::HANDLE,
            ((index as u64 + 1) << 32) | 51,
        );
        put64(&mut bytes, task + ctask_force::FLEET, fleet as u64);
        put32(&mut bytes, task + ctask_force::MISSION, 2);
        put64(
            &mut bytes,
            task + ctask_force::NAVAL_MISSION,
            engine_native_parts::cnaval_mission::VTABLE_RVA.0 as u64,
        );
        put64(
            &mut bytes,
            task + ctask_force::NAVAL_MISSION + engine_native_parts::cnaval_mission::TASK_FORCE,
            task as u64,
        );
        put32(
            &mut bytes,
            task + ctask_force::NAVAL_MISSION + engine_native_parts::cnaval_mission::ENGAGEMENT_RULE,
            2,
        );
    }
    let mut fleets = Vec::new();
    let mut tasks = Vec::new();
    let mut rosters = Vec::new();
    let mut origin_rows = Vec::new();
    let (mut searchers, mut avoided, mut text, mut answers): (
        Vec<Searcher>,
        Vec<Option<crate::cname_group_member::Span>>,
        Vec<u8>,
        Vec<ReferenceAnswer>,
    ) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (hour, origins, searching, strings) in [(1, 3, 1, 1), (2, 37, 17, 19)] {
        let origin_data = bytes.len();
        let search_data = origin_data + origins * supply::ORIGIN_STRIDE;
        let string_data = search_data + searching * 8;
        bytes.resize(string_data + strings * 32, 0);
        for index in 0..origins {
            let at = origin_data + index * supply::ORIGIN_STRIDE;
            put64(&mut bytes, at + supply::ORIGIN_REQUESTED, index as u64);
            put64(
                &mut bytes,
                at + supply::ORIGIN_RECEIVED,
                (-1 - index as i64) as u64,
            );
            put32(&mut bytes, at + supply::ORIGIN_PROVINCE, index as u32 % 7);
        }
        for index in 0..strings {
            let at = string_data + index * 32;
            bytes[at..at + 3].copy_from_slice(&[0xff, 0, index as u8]);
            put64(&mut bytes, at + 16, 3);
            put64(&mut bytes, at + 24, 15);
        }
        for index in 0..COUNT {
            let task = blocks + index * BLOCK + 0x500;
            array(
                &mut bytes,
                task + supply::ORIGINS,
                origin_data,
                origins as u32,
            );
            array(&mut bytes, task + 0x610, search_data, searching as u32);
            array(&mut bytes, task + 0x740, string_data, strings as u32);
        }
        let memory = OriginalCounterMemory {
            bytes: &bytes,
            fail_once_at: None,
            failed: std::cell::Cell::new(false),
            reads: Default::default(),
        };
        let mut supply = ctask_force::SupplyOrigins {
            origins: engine_native_memory::walk::Buffer::Growing(&mut origin_rows),
            len: 0,
            required: 0,
        };
        let mut port = Collector::new(
            &memory,
            0,
            hour,
            &mut searchers,
            &mut avoided,
            &mut text,
            &mut answers,
        );
        let actual = walk_counter_inputs_with_port_observed(
            &memory,
            0,
            GS,
            &mut fleets,
            &mut rosters,
            (&mut tasks, None, Some(&mut supply), Some(&mut port)),
            |_, _| {},
            |_, _, _| {},
        );
        let port = port.finish();
        assert_eq!(actual.fleets, Outcome::Rows { len: COUNT });
        assert_eq!(actual.task_forces, Outcome::Rows { len: COUNT });
        assert_eq!(
            (supply.len, supply.required),
            (COUNT * origins, COUNT * origins)
        );
        assert_eq!(port.searching_len, COUNT * searching);
        assert_eq!(port.avoided_len, COUNT * strings);
        assert_eq!(port.bytes_len, COUNT * strings * 3);
        let mut reads = std::collections::HashMap::<usize, usize>::new();
        for &(at, _) in memory.reads.borrow().iter() {
            *reads.entry(at).or_default() += 1;
        }
        for (index, task) in tasks[..COUNT].iter().enumerate() {
            assert_eq!(task.supply_origins_start, index * origins);
            assert_eq!(task.supply.unwrap().origins_len, Ok(origins));
            assert_eq!(task.supply_failure, None);
            let inputs = task.port_mission.unwrap();
            assert_eq!(inputs.sample_hour, hour);
            assert_eq!(inputs.searching, Some((index * searching, searching)));
            assert_eq!(inputs.avoided_ship_types, Some((index * strings, strings)));
            assert_eq!(inputs.engagement_target.unwrap().resolves, Some(false));
            for origin in 0..origins {
                assert_eq!(
                    supply.origins[index * origins + origin].received,
                    -1 - origin as i64
                );
            }
            let pointer = blocks + index * BLOCK + 0x500;
            assert_eq!(reads.get(&(pointer + supply::ORIGINS)), Some(&2));
            assert_eq!(reads.get(&(pointer + 0x610)), Some(&1));
            assert_eq!(reads.get(&(pointer + 0x740)), Some(&1));
        }
    }
}

#[derive(Clone, Default)]
struct CensusObjects {
    fleets: Vec<usize>,
    tasks: Vec<usize>,
    ships: Vec<usize>,
}

const WORLD_PROVINCES: usize = 0x2000;
const WORLD_PROVINCE: usize = 0x2100;
const WORLD_HEAP: usize = 0x3000;

fn world_alloc(bytes: &mut Vec<u8>, len: usize) -> usize {
    let at = bytes.len().next_multiple_of(16);
    bytes.resize(at + len, 0);
    at
}

fn census_world(count: usize) -> (Vec<u8>, Vec<CensusObjects>) {
    let countries = WORLD_HEAP;
    let links = countries + (count + 1) * 8;
    let mut bytes = vec![0; links + (count + 1) * 4];
    array(
        &mut bytes,
        GS + cgame_state::COUNTRIES,
        countries,
        count as u32 + 1,
    );
    array(
        &mut bytes,
        GS + cgame_state::COUNTRY_LINKS,
        links,
        count as u32 + 1,
    );
    array(&mut bytes, GS + cgame_state::PROVINCES, WORLD_PROVINCES, 2);
    put64(&mut bytes, WORLD_PROVINCES + 8, WORLD_PROVINCE as u64);
    put64(&mut bytes, WORLD_PROVINCE, cprovince::VTABLE_RVA.0 as u64);
    put32(&mut bytes, WORLD_PROVINCE + cprovince::ID, 1);
    let mut objects = vec![CensusObjects::default(); count + 1];
    let mut handle = 0_u64;
    for (slot, found) in objects.iter_mut().enumerate().skip(1) {
        if slot % 7 == 0 {
            continue;
        }
        let country = world_alloc(&mut bytes, 0x300);
        put64(&mut bytes, countries + slot * 8, country as u64);
        put32(&mut bytes, links + slot * 4, slot as u32);
        put64(&mut bytes, country, ccountry::VTABLE_RVA.0 as u64);
        put32(&mut bytes, country + ccountry::ID, slot as u32);
        let fleets = slot % 3;
        let roster = world_alloc(&mut bytes, 8 * fleets.max(1));
        array(
            &mut bytes,
            country + ccountry::FLEETS,
            roster,
            fleets as u32,
        );
        for f in 0..fleets {
            let fleet = world_alloc(&mut bytes, 0x140);
            found.fleets.push(fleet);
            handle += 1;
            put64(&mut bytes, roster + f * 8, fleet as u64);
            put64(&mut bytes, fleet, VTABLE_RVA.0 as u64);
            put64(&mut bytes, fleet + HANDLE, (handle << 32) | 61);
            put64(&mut bytes, fleet + OWNER, country as u64);
            if (slot + f) % 2 == 0 {
                put64(&mut bytes, fleet + HOME_BASE, WORLD_PROVINCE as u64);
            }
            if (slot + f) % 4 != 0 {
                let name = format!("F{slot}.{f}");
                let at = fleet + NAME;
                bytes[at..at + name.len()].copy_from_slice(name.as_bytes());
                put64(&mut bytes, at + 16, name.len() as u64);
                put64(&mut bytes, at + 24, 15);
            }
            let tasks = (slot + f) % 3;
            let task_roster = world_alloc(&mut bytes, 8 * tasks.max(1));
            array(&mut bytes, fleet + TASK_FORCES, task_roster, tasks as u32);
            for t in 0..tasks {
                let task = world_alloc(&mut bytes, 0x380);
                found.tasks.push(task);
                handle += 1;
                put64(&mut bytes, task_roster + t * 8, task as u64);
                put64(&mut bytes, task, ctask_force::VTABLE_RVA.0 as u64);
                put64(
                    &mut bytes,
                    task + ctask_force::SECONDARY_BASE,
                    ctask_force::SECONDARY_VTABLE_RVA.0 as u64,
                );
                put64(&mut bytes, task + cunit::HANDLE, (handle << 32) | 51);
                put64(&mut bytes, task + ctask_force::FLEET, fleet as u64);
                put32(&mut bytes, task + cunit::ROSTER_OWNER, slot as u32);
                put32(&mut bytes, task + cunit::LOGICAL_OWNER, slot as u32);
                if (slot + t) % 2 == 0 {
                    put64(&mut bytes, task + cunit::LOCATION, WORLD_PROVINCE as u64);
                }
                put32(&mut bytes, task + ctask_force::MISSION, 2);
                let ships = (slot + f + t) % 4 + usize::from(slot % 11 == 0) * 70;
                let ship_roster = world_alloc(&mut bytes, 8 * ships.max(1));
                array(
                    &mut bytes,
                    task + ctask_force::SHIPS,
                    ship_roster,
                    ships as u32,
                );
                for s in 0..ships {
                    let ship = world_alloc(&mut bytes, 0x730);
                    found.ships.push(ship);
                    handle += 1;
                    put64(&mut bytes, ship_roster + s * 8, ship as u64);
                    put64(&mut bytes, ship, cship::VTABLE_RVA.0 as u64);
                    put64(&mut bytes, ship + cship::HANDLE, (handle << 32) | 51);
                    put64(&mut bytes, ship + cship::STRENGTH, handle * 1_000);
                    put64(&mut bytes, ship + cship::ORGANISATION, handle * 7);
                    put64(
                        &mut bytes,
                        ship + cship::EXPERIENCE,
                        (-(handle as i64)) as u64,
                    );
                    put32(&mut bytes, ship + cship::CATEGORY, (s % 3) as u32);
                    put32(&mut bytes, ship + cship::SUNK_CONVOYS, s as u32);
                    put32(&mut bytes, ship + cship::CLASS_DATABASE_ID, (s % 5) as u32);
                    put64(&mut bytes, ship + cship::TASK_FORCE, task as u64);
                }
            }
        }
    }
    (bytes, objects)
}

struct CensusMemory<'a> {
    bytes: &'a [u8],
    fault: Option<usize>,
    once: bool,
    failures: std::cell::Cell<usize>,
}

impl Memory for CensusMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if let Some(at) = self.fault
            && address <= at
            && at - address < output.len()
            && (!self.once || self.failures.get() == 0)
        {
            self.failures.set(self.failures.get() + 1);
            return false;
        }
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(address, output)
    }
}

#[derive(Debug, PartialEq)]
struct Walked {
    capture: Capture,
    fleets: Vec<Row>,
    tasks: Vec<ctask_force::Row>,
    ships: Vec<cship::Row>,
    names: Vec<u8>,
    visits: Vec<(usize, cship::Row)>,
}

fn census_walk(memory: &impl Memory, batched: bool) -> Walked {
    census_walk_in(memory, batched, false)
}

fn census_walk_in(memory: &impl Memory, batched: bool, count: bool) -> Walked {
    let mut fleets = Vec::new();
    let mut tasks = Vec::new();
    let mut ships = Vec::new();
    let mut names = Vec::new();
    let mut visits = Vec::new();
    let walked = walk_named(
        memory,
        0,
        GS,
        buffer_of(&mut fleets, count),
        buffer_of(&mut tasks, count),
        buffer_of(&mut ships, count),
        |_| 0,
        |pointer, row| visits.push((pointer, *row)),
        Some(&mut names),
        batched,
    );
    let rows = |outcome: Outcome| match outcome {
        Outcome::Rows { len } | Outcome::Truncated { len, .. } => len,
        Outcome::Unavailable { .. } => 0,
    };
    fleets.truncate(rows(walked.capture.fleets));
    tasks.truncate(rows(walked.capture.task_forces));
    ships.truncate(rows(walked.capture.ships));
    Walked {
        capture: walked.capture,
        fleets,
        tasks,
        ships,
        names,
        visits,
    }
}

fn buffer_of<T>(rows: &mut Vec<T>, count: bool) -> engine_native_memory::walk::Buffer<'_, T> {
    if count {
        engine_native_memory::walk::Buffer::Count
    } else {
        engine_native_memory::walk::Buffer::Growing(rows)
    }
}

fn census_faults(objects: &[CensusObjects]) -> Vec<usize> {
    let ship = |slot: usize, index: usize| objects[slot].ships[index];
    vec![
        GS + cgame_state::PROVINCES,
        WORLD_PROVINCE + cprovince::ID,
        objects[8].fleets[0],
        objects[8].fleets[1] + HANDLE,
        objects[4].fleets[0] + OWNER,
        objects[10].fleets[0] + HOME_BASE,
        objects[11].fleets[0] + NAME,
        objects[13].tasks[0] + cunit::HANDLE,
        objects[16].tasks[0] + ctask_force::SECONDARY_BASE,
        objects[17].tasks[0] + cunit::ROSTER_OWNER,
        objects[19].tasks[0] + cunit::LOCATION,
        objects[20].tasks[0] + ctask_force::FLEET,
        objects[23].tasks[0] + ctask_force::MISSION,
        objects[5].tasks[0] + ctask_force::SHIPS,
        ship(5, 0),
        ship(10, 1) + cship::HANDLE,
        ship(11, 3) + cship::STRENGTH,
        ship(11, 69) + cship::TASK_FORCE,
        ship(22, 40) + cship::CATEGORY,
        ship(26, 0) + cship::CLASS_DATABASE_ID,
    ]
}

#[test]
fn batched_ship_census_keeps_lasting_faults_and_is_never_harsher_under_one_touch_faults() {
    let (bytes, objects) = census_world(40);
    for fault in census_faults(&objects) {
        let memory = |once| CensusMemory {
            bytes: &bytes,
            fault: Some(fault),
            once,
            failures: Default::default(),
        };
        use engine_native_memory::batch_fallback::{Batch, count};
        let (fleet_fallbacks, task_fallbacks) =
            (count(Batch::FleetGathers), count(Batch::TaskForceGathers));
        assert_eq!(
            census_walk(&memory(false), true),
            census_walk(&memory(false), false),
            "lasting fault at {fault:#x}"
        );
        let inside =
            |at: &[usize], size: usize| at.iter().any(|&at| at <= fault && fault < at + size);
        if objects.iter().any(|slot| inside(&slot.fleets, 0x140)) {
            assert!(
                count(Batch::FleetGathers) > fleet_fallbacks,
                "a fleet fault reads its fields alone"
            );
        }
        if objects.iter().any(|slot| inside(&slot.tasks, 0x380)) {
            assert!(
                count(Batch::TaskForceGathers) > task_fallbacks,
                "a task force fault reads its fields alone"
            );
        }
        let fallbacks = count(Batch::ShipRuns);
        let batched = census_walk(&memory(true), true);
        let serial = census_walk(&memory(true), false);
        let tables = |walked: &Walked| {
            [
                walked.capture.fleets,
                walked.capture.task_forces,
                walked.capture.ships,
            ]
        };
        for (batched_table, serial_table) in tables(&batched).into_iter().zip(tables(&serial)) {
            assert!(
                !matches!(batched_table, Outcome::Unavailable { .. })
                    || matches!(serial_table, Outcome::Unavailable { .. }),
                "one-touch fault at {fault:#x}: {batched_table:?} where the walk gave {serial_table:?}"
            );
            assert!(batched_table.len() >= serial_table.len(), "{fault:#x}");
        }
        assert!(batched.visits.len() >= serial.visits.len(), "{fault:#x}");
        if objects.iter().any(|slot| {
            slot.ships
                .iter()
                .any(|&ship| ship <= fault && fault < ship + 0x730)
        }) {
            assert!(
                count(Batch::ShipRuns) > fallbacks,
                "a ship fault reads its batch again one ship at a time"
            );
        }
    }
}

#[test]
fn ships_queued_before_a_walk_ending_failure_are_visited_as_the_per_ship_walk_visits_them() {
    let (bytes, objects) = census_world(40);
    for fault in [
        objects[8].fleets[0],
        objects[13].fleets[0],
        objects[34].fleets[0],
    ] {
        let memory = |once| CensusMemory {
            bytes: &bytes,
            fault: Some(fault),
            once,
            failures: Default::default(),
        };
        let serial = census_walk(&memory(false), false);
        assert!(matches!(serial.capture.ships, Outcome::Unavailable { .. }));
        assert!(!serial.visits.is_empty());
        assert_eq!(census_walk(&memory(false), true), serial, "{fault:#x}");
    }
}

#[test]
fn batched_count_census_equals_the_per_ship_count() {
    let (bytes, _) = census_world(40);
    let memory = CensusMemory {
        bytes: &bytes,
        fault: None,
        once: false,
        failures: Default::default(),
    };
    let whole = census_walk(&memory, false);
    assert!(whole.ships.len() > 3 * SHIP_RUN);
    let serial = census_walk_in(&memory, false, true);
    assert_eq!(census_walk_in(&memory, true, true), serial);
    assert_eq!(
        serial.capture.ships,
        Outcome::Truncated {
            len: 0,
            unread_rows: whole.ships.len() as u32
        }
    );
    assert!(serial.ships.is_empty());
}

struct Scattered<'a> {
    inner: CensusMemory<'a>,
    first: std::cell::RefCell<Vec<usize>>,
}

impl Memory for Scattered<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.inner.copy(address, output)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.first.borrow_mut().push(addresses[0]);
        for (done, (address, destination)) in addresses
            .into_iter()
            .zip(destinations.iter_mut())
            .enumerate()
        {
            if destination.is_empty() {
                continue;
            }
            if address == 0 || !self.inner.copy(address, destination) {
                return done;
            }
        }
        N
    }
}

#[test]
fn a_short_run_reads_each_remaining_ship_alone_at_most_once() {
    let (bytes, objects) = census_world(40);
    let ships: Vec<usize> = objects
        .iter()
        .flat_map(|slot| slot.ships.iter().copied())
        .collect();
    let failing = objects[11].ships[3];
    for once in [false, true] {
        let memory = Scattered {
            inner: CensusMemory {
                bytes: &bytes,
                fault: Some(failing + cship::STRENGTH),
                once,
                failures: Default::default(),
            },
            first: Default::default(),
        };
        let walked = census_walk(&memory, true);
        let alone: Vec<usize> = memory
            .first
            .borrow()
            .iter()
            .copied()
            .filter(|at| ships.contains(at))
            .collect();
        let mut distinct = alone.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), alone.len(), "each ship at most once");
        assert_eq!(alone.first(), Some(&failing));
        let position = ships.iter().position(|&ship| ship == failing).unwrap();
        if once {
            assert!(alone.len() <= SHIP_RUN);
            assert!(alone.iter().all(|at| {
                let index = ships.iter().position(|ship| ship == at).unwrap();
                index >= position && index < position + SHIP_RUN
            }));
            assert!(matches!(walked.capture.ships, Outcome::Rows { .. }));
        } else {
            assert_eq!(alone, vec![failing]);
            assert_eq!(walked.visits.len(), position);
            assert_eq!(
                walked.capture.ships,
                Outcome::Unavailable {
                    reason: "ship fields unreadable"
                }
            );
        }
    }
}

#[test]
fn certified_fleet_region_payload_keeps_occurrences_and_authenticates_each_reference() {
    let mut bytes = vec![0_u8; 0x1000];
    let region = 0x400;
    put64(
        &mut bytes,
        region,
        engine_native_parts::cstrategic_region::VTABLE_RVA.0 as u64,
    );
    put32(&mut bytes, region + engine_native_parts::cstrategic_region::ID, 7);
    let other = 0x600;
    put64(
        &mut bytes,
        other,
        engine_native_parts::cstrategic_region::VTABLE_RVA.0 as u64,
    );
    put32(&mut bytes, other + engine_native_parts::cstrategic_region::ID, 3);
    for (at, pointer) in [region, other, region].into_iter().enumerate() {
        put64(&mut bytes, 0x100 + at * 8, pointer as u64);
    }
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert_eq!(
        read_mission_regions(
            &memory,
            0,
            Array {
                data: 0x100,
                count: 3
            }
        ),
        Ok(vec![3, 7, 7])
    );
    assert_eq!(
        read_mission_regions(&memory, 0, Array { data: 0, count: 0 }),
        Ok(Vec::new())
    );
    put64(&mut bytes, region, 0);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert_eq!(
        read_mission_regions(
            &memory,
            0,
            Array {
                data: 0x100,
                count: 3
            }
        ),
        Err("strategic region vtable mismatch")
    );
}
