use super::*;
use crate::{carmy, ctheatre, cunit};
use engine_native_parts::{carmy_group, ctheater_group};
use engine_native_memory::memory::SliceMemory;

const ARMY: usize = 0x1000;
const GROUP: usize = 0x2000;
const THEATRE_PTR: usize = 0x3000;
const THEATER_GROUP_PTR: usize = 0x3100;
const LEADER_PTR: usize = 0x3200;
const D1: usize = 0x4000;
const D2: usize = 0x5000;
const D3: usize = 0x6000;
const MEMBERS: usize = 0x7000;
const CHILDREN: usize = 0x7100;
const REGULAR_ROSTER: usize = 0x7200;
const ARMY_ROSTER: usize = 0x7300;

fn p32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn p64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn vector(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    p64(bytes, at, data as u64);
    p32(bytes, at + 8, count);
    p32(bytes, at + 12, count);
}
fn handle(id: u32, kind: u32) -> u64 {
    (u64::from(id) << 32) | u64::from(kind)
}
fn division(bytes: &mut [u8], at: usize, id: u32, group: usize) {
    p64(bytes, at, carmy::VTABLE_RVA.0 as u64);
    p64(
        bytes,
        at + carmy::SECONDARY_BASE,
        carmy::SECONDARY_VTABLE_RVA.0 as u64,
    );
    p64(bytes, at + cunit::HANDLE, handle(id, 4713));
    p64(bytes, at + carmy::ORDERS_GROUP, group as u64);
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x9000];
    p64(&mut bytes, THEATRE_PTR, ctheatre::VTABLE_RVA.0 as u64);
    vector(
        &mut bytes,
        THEATRE_PTR + ctheatre::ORDERS_GROUP_DATA,
        REGULAR_ROSTER,
        1,
    );
    vector(
        &mut bytes,
        THEATRE_PTR + ctheatre::ARMY_GROUP_DATA,
        ARMY_ROSTER,
        1,
    );
    p64(&mut bytes, REGULAR_ROSTER, ARMY as u64);
    p64(&mut bytes, ARMY_ROSTER, GROUP as u64);
    p64(
        &mut bytes,
        THEATRE_PTR + 8,
        handle(2, ctheatre::REFERENCE_TYPE),
    );
    p64(
        &mut bytes,
        THEATER_GROUP_PTR,
        ctheater_group::VTABLE_RVA.0 as u64,
    );
    p64(
        &mut bytes,
        THEATER_GROUP_PTR + 8,
        handle(45, ctheater_group::REFERENCE_TYPE),
    );
    p64(
        &mut bytes,
        LEADER_PTR,
        crate::carmy_leader::VTABLE_RVA.0 as u64,
    );
    p64(&mut bytes, LEADER_PTR + 8, handle(279, 4713));

    p64(&mut bytes, GROUP, carmy_group::VTABLE_RVA.0 as u64);
    p64(&mut bytes, GROUP + HANDLE, handle(431, REFERENCE_TYPE));
    bytes[GROUP + IS_ARMY_GROUP] = 1;
    bytes[GROUP + MOTORIZATION_LEVEL] = 3;
    p32(&mut bytes, GROUP + COUNTRY, 7);
    p64(&mut bytes, GROUP + THEATRE, THEATRE_PTR as u64);
    p64(&mut bytes, GROUP + THEATER_GROUP, THEATER_GROUP_PTR as u64);
    vector(&mut bytes, GROUP + MEMBER_DATA, MEMBERS, 0);
    vector(&mut bytes, GROUP + carmy_group::MEMBER_DATA, CHILDREN, 1);
    p64(&mut bytes, CHILDREN, ARMY as u64);

    p64(&mut bytes, ARMY, VTABLE_RVA.0 as u64);
    p64(&mut bytes, ARMY + HANDLE, handle(57, REFERENCE_TYPE));
    p32(&mut bytes, ARMY + COUNTRY, 7);
    p64(&mut bytes, ARMY + THEATRE, THEATRE_PTR as u64);
    p64(&mut bytes, ARMY + THEATER_GROUP, THEATER_GROUP_PTR as u64);
    p64(&mut bytes, ARMY + PARENT, GROUP as u64);
    p64(&mut bytes, ARMY + LEADER, LEADER_PTR as u64);
    p64(
        &mut bytes,
        ARMY + HQ_PROVIDER,
        (D1 + corders_group_member::HOST_OFFSET) as u64,
    );
    vector(&mut bytes, ARMY + MEMBER_DATA, MEMBERS, 2);
    p64(
        &mut bytes,
        MEMBERS,
        (D1 + corders_group_member::HOST_OFFSET) as u64,
    );
    p64(
        &mut bytes,
        MEMBERS + 8,
        (D2 + corders_group_member::HOST_OFFSET) as u64,
    );
    division(&mut bytes, D1, 12456, ARMY);
    division(&mut bytes, D2, 14578, ARMY);
    division(&mut bytes, D3, 20427, 0);
    bytes
}

fn roots() -> [Root; 2] {
    [
        Root {
            pointer: ARMY,
            handle: handle(57, REFERENCE_TYPE),
            country: 7,
        },
        Root {
            pointer: GROUP,
            handle: handle(431, REFERENCE_TYPE),
            country: 7,
        },
    ]
}

fn run(
    bytes: &[u8],
    capacity: [usize; 3],
) -> (
    OrderOfBattleWalk,
    Vec<GroupRow>,
    Vec<MemberRow>,
    Vec<ChildRow>,
) {
    run_memory(&SliceMemory { base: 0, bytes }, capacity, 2)
}

fn run_memory(
    memory: &impl Memory,
    capacity: [usize; 3],
    scratch_capacity: usize,
) -> (
    OrderOfBattleWalk,
    Vec<GroupRow>,
    Vec<MemberRow>,
    Vec<ChildRow>,
) {
    let mut groups = vec![GroupRow::default(); capacity[0]];
    let mut members = vec![MemberRow::default(); capacity[1]];
    let mut children = vec![ChildRow::default(); capacity[2]];
    let mut roster_scratch = vec![RosterEntry::default(); scratch_capacity];
    let walk = walk_order_of_battle(
        memory,
        0,
        &roots(),
        &mut OrderOfBattle {
            groups: &mut groups,
            members: &mut members,
            children: &mut children,
            roster_scratch: &mut roster_scratch,
        },
    );
    (walk, groups, members, children)
}

#[test]
fn completed_roster_proof_binds_actual_theatre_pointer_handle_subtype_and_ordinal() {
    let bytes = fixture();
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut groups = vec![GroupRow::default(); 2];
    let mut members = vec![MemberRow::default(); 4];
    let mut children = vec![ChildRow::default(); 4];
    let mut scratch = vec![RosterEntry::default(); 2];
    let walk = walk_order_of_battle(
        &memory,
        0,
        &roots(),
        &mut OrderOfBattle {
            groups: &mut groups,
            members: &mut members,
            children: &mut children,
            roster_scratch: &mut scratch,
        },
    );
    assert_eq!(walk.groups, Outcome::Rows { len: 2 });
    let theatre = handle(2, ctheatre::REFERENCE_TYPE);
    assert_eq!(
        captured_theatre_rosters(&groups, &scratch, THEATRE_PTR, theatre),
        Ok(true)
    );
    assert_eq!(
        captured_theatre_rosters(
            &groups,
            &scratch,
            0x8000,
            handle(3, ctheatre::REFERENCE_TYPE)
        ),
        Ok(false)
    );
    assert!(captured_theatre_rosters(&groups, &scratch, 0x8000, theatre).is_err());
    let mut altered = groups.clone();
    altered[0].theatre_ordinal = None;
    assert!(captured_theatre_rosters(&altered, &scratch, THEATRE_PTR, theatre).is_err());
    altered = groups.clone();
    altered[0].is_army_group ^= 1;
    assert!(captured_theatre_rosters(&altered, &scratch, THEATRE_PTR, theatre).is_err());
    altered = groups.clone();
    altered[1].handle = altered[0].handle;
    assert!(captured_theatre_rosters(&altered, &scratch, THEATRE_PTR, theatre).is_err());
    assert!(captured_theatre_rosters(&groups, &scratch[..1], THEATRE_PTR, theatre).is_err());
}

#[test]
fn identity_membership_and_child_vectors_keep_native_order_and_null_pointers() {
    let bytes = fixture();
    let (walk, groups, members, children) = run(&bytes, [4, 4, 4]);
    assert_eq!(walk.groups, Outcome::Rows { len: 2 });
    assert_eq!(walk.members, Outcome::Rows { len: 2 });
    assert_eq!(walk.children, Outcome::Rows { len: 1 });
    assert_eq!(
        groups[0],
        GroupRow {
            handle: handle(57, REFERENCE_TYPE),
            country: 7,
            is_army_group: 0,
            motorization_level: 0,
            theatre: handle(2, ctheatre::REFERENCE_TYPE),
            theatre_ordinal: Some(0),
            theater_group: handle(45, ctheater_group::REFERENCE_TYPE),
            parent: handle(431, REFERENCE_TYPE),
            leader: handle(279, 4713),
            hq_provider: handle(12456, 4713),
            pointer: ARMY,
            name_start: 0,
            name_len: 0,
            name_known: false,
        }
    );
    assert_eq!(
        groups[1],
        GroupRow {
            handle: handle(431, REFERENCE_TYPE),
            country: 7,
            is_army_group: 1,
            motorization_level: 3,
            theatre: handle(2, ctheatre::REFERENCE_TYPE),
            theatre_ordinal: Some(0),
            theater_group: handle(45, ctheater_group::REFERENCE_TYPE),
            parent: 0,
            leader: 0,
            hq_provider: 0,
            pointer: GROUP,
            name_start: 0,
            name_len: 0,
            name_known: false,
        }
    );
    assert_eq!(
        &members[..2],
        &[
            MemberRow {
                group: handle(57, REFERENCE_TYPE),
                ordinal: 0,
                unit: handle(12456, 4713),
            },
            MemberRow {
                group: handle(57, REFERENCE_TYPE),
                ordinal: 1,
                unit: handle(14578, 4713),
            },
        ]
    );
    assert_eq!(
        children[0],
        ChildRow {
            group: handle(431, REFERENCE_TYPE),
            ordinal: 0,
            child: handle(57, REFERENCE_TYPE),
        }
    );
}

#[test]
fn a_member_that_is_not_an_authenticated_unit_fails_every_table() {
    let mut bytes = fixture();
    p64(&mut bytes, MEMBERS, (D3 + 1) as u64);
    let (walk, ..) = run(&bytes, [4, 4, 4]);
    assert!(matches!(walk.members, Outcome::Unavailable { .. }));
    assert!(matches!(walk.groups, Outcome::Unavailable { .. }));

    let mut bytes = fixture();
    p32(&mut bytes, D2 + 8, 13);
    let (walk, ..) = run(&bytes, [4, 4, 4]);
    assert_eq!(
        walk.members,
        Outcome::Unavailable {
            reason: "orders group member host is not a class-zero army"
        }
    );
    assert!(matches!(walk.groups, Outcome::Unavailable { .. }));

    let mut bytes = fixture();
    p64(&mut bytes, D2 + carmy::ORDERS_GROUP, GROUP as u64);
    let (walk, ..) = run(&bytes, [4, 4, 4]);
    assert_eq!(
        walk.members,
        Outcome::Unavailable {
            reason: "orders group member does not point back at its group"
        }
    );

    let mut bytes = fixture();
    p64(&mut bytes, MEMBERS + 8, 0);
    let (walk, ..) = run(&bytes, [4, 4, 4]);
    assert_eq!(
        walk.members,
        Outcome::Unavailable {
            reason: "orders group vector entry unreadable or null"
        }
    );
}

#[test]
fn theatre_ordinals_follow_raw_roster_order_and_preserve_absence() {
    let mut bytes = fixture();
    bytes[GROUP + IS_ARMY_GROUP] = 0;
    p64(&mut bytes, GROUP, VTABLE_RVA.0 as u64);
    p64(&mut bytes, ARMY + PARENT, 0);
    vector(
        &mut bytes,
        THEATRE_PTR + ctheatre::ORDERS_GROUP_DATA,
        REGULAR_ROSTER,
        2,
    );
    vector(&mut bytes, THEATRE_PTR + ctheatre::ARMY_GROUP_DATA, 0, 0);
    p64(&mut bytes, REGULAR_ROSTER, GROUP as u64);
    p64(&mut bytes, REGULAR_ROSTER + 8, ARMY as u64);
    let (walk, groups, ..) = run(&bytes, [4, 4, 4]);
    assert_eq!(walk.groups, Outcome::Rows { len: 2 });
    assert_eq!(groups[0].theatre_ordinal, Some(1));
    assert_eq!(groups[1].theatre_ordinal, Some(0));

    vector(
        &mut bytes,
        THEATRE_PTR + ctheatre::ORDERS_GROUP_DATA,
        REGULAR_ROSTER,
        1,
    );
    let (walk, groups, ..) = run(&bytes, [4, 4, 4]);
    assert_eq!(walk.groups, Outcome::Rows { len: 2 });
    assert_eq!(groups[0].theatre_ordinal, None);
    assert_eq!(groups[1].theatre_ordinal, Some(0));

    p64(&mut bytes, ARMY + THEATRE, 0);
    let (walk, groups, ..) = run(&bytes, [4, 4, 4]);
    assert_eq!(walk.groups, Outcome::Rows { len: 2 });
    assert_eq!(groups[0].theatre, 0);
    assert_eq!(groups[0].theatre_ordinal, None);
}

#[test]
fn root_order_theatre_ordinals_match_the_sorted_walk() {
    struct Counted<'a> {
        memory: SliceMemory<'a>,
        reads: std::cell::RefCell<std::collections::BTreeMap<usize, usize>>,
    }
    impl Memory for Counted<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            *self.reads.borrow_mut().entry(address).or_default() += 1;
            self.memory.copy(address, destination)
        }
    }
    const BASE: usize = 0x10_0000;
    const THEATRE_SIZE: usize = 0x100;
    const LISTS: usize = 0x8000;
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut random = move |bound: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % bound as u64) as usize
    };
    let mut failures = std::collections::BTreeSet::new();
    let mut successes = 0;
    for case in 0..3000 {
        let theatres = 1 + random(6);
        let count = random(24);
        let faulty = case % 3 != 0;
        let mut scratch: Vec<RosterEntry> = (0..count)
            .map(|root| RosterEntry {
                pointer: 0x40_0000 + random(1 << 16) * 0x40,
                theatre: match random(theatres + 1) {
                    0 => 0,
                    t => BASE + (t - 1) * THEATRE_SIZE,
                },
                root,
                is_army_group: random(3) == 0,
                ..Default::default()
            })
            .collect();
        let mut bytes = vec![0_u8; LISTS + theatres * 2 * 64 * 8];
        let mut next_list = LISTS;
        for t in 0..theatres {
            let theatre = BASE + t * THEATRE_SIZE;
            for (offset, army_group) in [
                (ctheatre::ORDERS_GROUP_DATA, false),
                (ctheatre::ARMY_GROUP_DATA, true),
            ] {
                let mut list: Vec<usize> = scratch
                    .iter()
                    .filter(|entry| {
                        entry.theatre == theatre
                            && entry.is_army_group == army_group
                            && random(8) != 0
                    })
                    .map(|entry| entry.pointer)
                    .collect();
                for index in (1..list.len()).rev() {
                    list.swap(index, random(index + 1));
                }
                if faulty && random(6) == 0 && !scratch.is_empty() {
                    let at = random(list.len() + 1);
                    let pointer = match random(4) {
                        0 => scratch[random(scratch.len())].pointer,
                        1 => 0x7f_0000,
                        2 => 0,
                        _ => list.first().copied().unwrap_or(0x7f_0040),
                    };
                    list.insert(at, pointer);
                }
                let header = theatre - BASE + offset;
                vector(&mut bytes, header, BASE + next_list, list.len() as u32);
                for (index, pointer) in list.iter().enumerate() {
                    p64(&mut bytes, next_list + index * 8, *pointer as u64);
                }
                if faulty && random(10) == 0 {
                    match random(2) {
                        0 => p64(&mut bytes, header, 0x7fff_0000),
                        _ => {
                            p32(&mut bytes, header + 8, 0);
                            p32(&mut bytes, header + 12, list.len().max(1) as u32);
                        }
                    }
                }
                next_list += 64 * 8;
            }
        }
        if faulty && count > 1 && random(8) == 0 {
            let from = random(count);
            let to = random(count);
            scratch[to].pointer = scratch[from].pointer;
        }
        let run = |sorted: bool| {
            let memory = Counted {
                memory: SliceMemory {
                    base: BASE,
                    bytes: &bytes,
                },
                reads: Default::default(),
            };
            let reader = Reader {
                memory: &memory,
                base: 0,
            };
            let mut scratch = scratch.clone();
            let mut groups = vec![GroupRow::default(); count];
            let result = if sorted {
                theatre_ordinals_sorted(&reader, &mut scratch, &mut groups)
            } else {
                theatre_ordinals(&reader, &mut scratch, &mut groups)
            };
            let ordinals: Vec<_> = groups.iter().map(|row| row.theatre_ordinal).collect();
            (result, ordinals, memory.reads.into_inner())
        };
        let (expected, expected_ordinals, expected_reads) = run(true);
        let (actual, actual_ordinals, actual_reads) = run(false);
        assert_eq!(actual, expected, "case {case}");
        match expected {
            Ok(()) => {
                successes += 1;
                assert_eq!(actual_ordinals, expected_ordinals, "case {case}");
                assert!(actual_reads.keys().eq(expected_reads.keys()), "case {case}");
            }
            Err(reason) => {
                failures.insert(reason);
            }
        }
    }
    assert!(successes > 500, "{successes} successes");
    for reason in [
        "theatre orders group roster unreadable or invalid",
        "orders group vector entry unreadable or null",
        "theatre roster group absent from census",
        "theatre roster group backlink mismatch",
        "theatre roster group subtype mismatch",
        "duplicate group in theatre roster",
    ] {
        assert!(
            failures.contains(reason),
            "{reason} not reached: {failures:?}"
        );
    }
}

#[test]
fn theatre_rosters_reject_wrong_subtype_backlink_duplicates_and_unknown_roots() {
    for (pointer, expected) in [
        (GROUP, "theatre roster group subtype mismatch"),
        (D3, "theatre roster group absent from census"),
    ] {
        let mut bytes = fixture();
        p64(&mut bytes, REGULAR_ROSTER, pointer as u64);
        let (walk, ..) = run(&bytes, [4, 4, 4]);
        assert_eq!(walk.groups, Outcome::Unavailable { reason: expected });
    }
    let mut bytes = fixture();
    p64(&mut bytes, ARMY + THEATRE, 0);
    vector(
        &mut bytes,
        THEATRE_PTR + ctheatre::ARMY_GROUP_DATA,
        ARMY_ROSTER,
        2,
    );
    p64(&mut bytes, ARMY_ROSTER + 8, ARMY as u64);
    let (walk, ..) = run(&bytes, [4, 4, 4]);
    assert_eq!(
        walk.groups,
        Outcome::Unavailable {
            reason: "theatre roster group backlink mismatch"
        }
    );

    let mut bytes = fixture();
    vector(
        &mut bytes,
        THEATRE_PTR + ctheatre::ORDERS_GROUP_DATA,
        REGULAR_ROSTER,
        2,
    );
    p64(&mut bytes, REGULAR_ROSTER + 8, ARMY as u64);
    let (walk, ..) = run(&bytes, [4, 4, 4]);
    assert_eq!(
        walk.groups,
        Outcome::Unavailable {
            reason: "duplicate group in theatre roster"
        }
    );

    let mut bytes = fixture();
    bytes[GROUP + IS_ARMY_GROUP] = 0;
    p64(&mut bytes, GROUP, VTABLE_RVA.0 as u64);
    p64(&mut bytes, ARMY + PARENT, 0);
    let (walk, ..) = run(&bytes, [4, 4, 4]);
    assert_eq!(
        walk.groups,
        Outcome::Unavailable {
            reason: "theatre roster group subtype mismatch"
        }
    );
}

#[test]
fn theatre_roster_reads_are_once_per_parent_and_failures_never_become_absence() {
    struct Observed<'a> {
        bytes: &'a [u8],
        calls: std::cell::RefCell<Vec<(usize, usize)>>,
        failure: Option<usize>,
    }
    impl Memory for Observed<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, destination.len()));
            if self.failure == Some(address) {
                return false;
            }
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, destination)
        }
    }
    let bytes = fixture();
    let memory = Observed {
        bytes: &bytes,
        calls: Default::default(),
        failure: None,
    };
    let (walk, ..) = run_memory(&memory, [4, 4, 4], 2);
    assert_eq!(walk.groups, Outcome::Rows { len: 2 });
    for span in [
        (THEATRE_PTR + ctheatre::ORDERS_GROUP_DATA, 16),
        (THEATRE_PTR + ctheatre::ARMY_GROUP_DATA, 16),
        (REGULAR_ROSTER, 8),
        (ARMY_ROSTER, 8),
    ] {
        assert_eq!(
            memory
                .calls
                .borrow()
                .iter()
                .filter(|&&call| call == span)
                .count(),
            1
        );
    }
    for failure in [THEATRE_PTR + ctheatre::ORDERS_GROUP_DATA, REGULAR_ROSTER] {
        let memory = Observed {
            bytes: &bytes,
            calls: Default::default(),
            failure: Some(failure),
        };
        let (walk, ..) = run_memory(&memory, [0, 0, 0], 2);
        assert!(matches!(walk.groups, Outcome::Unavailable { .. }));
        assert!(matches!(walk.members, Outcome::Unavailable { .. }));
        assert!(matches!(walk.children, Outcome::Unavailable { .. }));
        assert_eq!(
            memory
                .calls
                .borrow()
                .iter()
                .filter(|&&(address, _)| address == failure)
                .count(),
            1
        );
    }
}

#[test]
fn referenced_objects_are_authenticated_and_primary_country_is_retained() {
    for (offset, value) in [
        (THEATRE_PTR, 7_u64),
        (THEATER_GROUP_PTR, 1),
        (LEADER_PTR, 1),
    ] {
        let mut bytes = fixture();
        p64(&mut bytes, offset, value);
        let (walk, ..) = run(&bytes, [4, 4, 4]);
        assert!(
            matches!(walk.groups, Outcome::Unavailable { .. }),
            "{offset:#x}"
        );
    }
    let mut bytes = fixture();
    bytes[GROUP + IS_ARMY_GROUP] = 0;
    let (walk, ..) = run(&bytes, [4, 4, 4]);
    assert_eq!(
        walk.groups,
        Outcome::Unavailable {
            reason: "orders group parent is not an army group"
        }
    );
    let mut bytes = fixture();
    p32(&mut bytes, ARMY + COUNTRY, 9);
    let (walk, groups, ..) = run(&bytes, [4, 4, 4]);
    assert_eq!(walk.groups, Outcome::Rows { len: 2 });
    assert_eq!(groups[0].country, 7);
}

#[test]
fn an_empty_census_is_an_empty_order_of_battle() {
    let bytes = fixture();
    let mut groups = vec![GroupRow::default(); 2];
    let mut members = vec![MemberRow::default(); 2];
    let mut children = vec![ChildRow::default(); 2];
    let mut roster_scratch = vec![RosterEntry::default(); 2];
    let walk = walk_order_of_battle(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        &[],
        &mut OrderOfBattle {
            groups: &mut groups,
            members: &mut members,
            children: &mut children,
            roster_scratch: &mut roster_scratch,
        },
    );
    assert_eq!(walk.groups, Outcome::Rows { len: 0 });
    assert_eq!(walk.members, Outcome::Rows { len: 0 });
    assert_eq!(walk.children, Outcome::Rows { len: 0 });
}

#[test]
fn an_unreadable_world_never_produces_a_prefix() {
    struct Unreadable;
    impl Memory for Unreadable {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            false
        }
    }
    let mut groups = vec![GroupRow::default(); 2];
    let mut members = vec![MemberRow::default(); 2];
    let mut children = vec![ChildRow::default(); 2];
    let mut roster_scratch = vec![RosterEntry::default(); 2];
    let walk = walk_order_of_battle(
        &Unreadable,
        0,
        &roots(),
        &mut OrderOfBattle {
            groups: &mut groups,
            members: &mut members,
            children: &mut children,
            roster_scratch: &mut roster_scratch,
        },
    );
    assert!(matches!(walk.groups, Outcome::Unavailable { .. }));
    assert!(matches!(walk.members, Outcome::Unavailable { .. }));
    assert!(matches!(walk.children, Outcome::Unavailable { .. }));
}

#[test]
fn worker_order_of_battle_grows_original_nested_members_and_children_before_publication() {
    let mut groups = Vec::new();
    let mut members = Vec::new();
    let mut children = Vec::new();
    let mut scratch = Vec::new();
    for count in [50_usize, 200] {
        let mut bytes = fixture();
        let members_data = bytes.len();
        bytes.resize(members_data + count * 8, 0);
        for ordinal in 0..count {
            p64(
                &mut bytes,
                members_data + ordinal * 8,
                (D1 + corders_group_member::HOST_OFFSET) as u64,
            );
        }
        vector(&mut bytes, ARMY + MEMBER_DATA, members_data, count as u32);
        let children_data = bytes.len();
        bytes.resize(children_data + count * 8, 0);
        for ordinal in 0..count {
            p64(&mut bytes, children_data + ordinal * 8, ARMY as u64);
        }
        vector(
            &mut bytes,
            GROUP + carmy_group::MEMBER_DATA,
            children_data,
            count as u32,
        );
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let walked = walk_order_of_battle_buffers(
            &memory,
            0,
            &roots(),
            &mut OrderOfBattleStorage {
                groups: engine_native_memory::walk::Buffer::Growing(&mut groups),
                members: engine_native_memory::walk::Buffer::Growing(&mut members),
                children: engine_native_memory::walk::Buffer::Growing(&mut children),
                roster_scratch: engine_native_memory::walk::Buffer::Growing(&mut scratch),
            },
        );
        assert_eq!(walked.groups, Outcome::Rows { len: 2 });
        assert_eq!(walked.members, Outcome::Rows { len: count });
        assert_eq!(walked.children, Outcome::Rows { len: count });
        assert_eq!(members[count - 1].ordinal, count as u32 - 1);
        assert_eq!(children[count - 1].ordinal, count as u32 - 1);
    }
}
