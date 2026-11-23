use super::*;
use engine_native::{ccountry, cname_group_member, cprovince, ctask_force, cunit};
use engine_native_memory::{memory::SliceMemory, msvc_string};
use replay_capture_spool::spool::Record;
use replay_data::{identity::Roster, rows::ShipRow};
use replay_frame::frame::TableUpdate;
use std::cell::RefCell;

fn reference(id: u32) -> u64 {
    (u64::from(id) << 32) | u64::from(cship::HANDLE_TYPE)
}
fn word(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn integer(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn allocate(bytes: &mut Vec<u8>, size: usize) -> usize {
    let start = bytes.len().next_multiple_of(8);
    bytes.resize(start + size, 0);
    start
}
fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    word(bytes, at, data as u64);
    integer(bytes, at + 8, count);
    integer(bytes, at + 12, count);
}
fn string(bytes: &mut Vec<u8>, at: usize, text: &[u8]) {
    integer(bytes, at + msvc_string::LENGTH, text.len() as u32);
    if text.len() <= msvc_string::INLINE_CAPACITY as usize {
        bytes[at..at + text.len()].copy_from_slice(text);
        word(
            bytes,
            at + msvc_string::CAPACITY,
            msvc_string::INLINE_CAPACITY,
        );
    } else {
        let heap = allocate(bytes, text.len() + 1);
        bytes[heap..heap + text.len()].copy_from_slice(text);
        word(bytes, at, heap as u64);
        word(bytes, at + msvc_string::CAPACITY, text.len() as u64);
    }
}
fn native_ship(bytes: &mut Vec<u8>, id: u64, text: &[u8], task: usize) -> (usize, usize) {
    let ship = allocate(bytes, cship::SIZE);
    let member = allocate(bytes, cname_group_member::INPUT_SPAN);
    word(bytes, ship, cship::VTABLE_RVA.0 as u64);
    word(bytes, ship + cship::HANDLE, id);
    word(bytes, ship + cship::NAME_MEMBER, member as u64);
    word(bytes, ship + cship::TASK_FORCE, task as u64);
    word(bytes, member, cname_group_member::VTABLE_RVA.0 as u64);
    integer(bytes, member + cname_group_member::TYPE, i32::MIN as u32);
    string(bytes, member + cname_group_member::CACHED_NAME, text);
    (ship, member)
}
struct Tracked<'a> {
    bytes: &'a [u8],
    denied: Option<usize>,
    reads: RefCell<Vec<(usize, usize)>>,
}
impl Memory for Tracked<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, destination.len()));
        if self
            .denied
            .is_some_and(|at| address <= at && at < address.saturating_add(destination.len()))
        {
            return false;
        }
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(address, destination)
    }
    fn prefetch(&self, _: usize, _: usize) {}
}
fn tracked(bytes: &[u8], denied: Option<usize>) -> Tracked<'_> {
    Tracked {
        bytes,
        denied,
        reads: RefCell::default(),
    }
}

#[test]
fn ship_name_witness_copies_original_guarded_cache_bytes_without_name_selection() {
    for text in [
        vec![],
        b"Same\0suffix\xff".to_vec(),
        vec![0xff],
        vec![b'x'; 65_537],
    ] {
        let mut bytes = vec![0; 8];
        let id = reference(0xfedc_ba98);
        let (pointer, member) = native_ship(&mut bytes, id, &text, 0);
        let memory = tracked(&bytes, None);
        let read = ship(&memory, 0, pointer, id, 77);
        assert_eq!((read.reference, read.task_force), (id, 77));
        let Cache::Read { bytes } = read.cache else {
            panic!("readable cache refused")
        };
        assert_eq!(bytes, text);
        let reads = memory.reads.borrow();
        assert_eq!(
            reads
                .iter()
                .filter(|&&(at, len)| at == pointer && len == 16)
                .count(),
            1
        );
        assert_eq!(
            reads
                .iter()
                .filter(|&&(at, len)| at == member + cname_group_member::CACHED_NAME
                    && len == msvc_string::STRIDE)
                .count(),
            1
        );
        assert!(!reads.iter().any(|&(at, _)| {
            [
                member + cname_group_member::TYPE,
                member + cname_group_member::OVERRIDE_NAME,
            ]
            .contains(&at)
        }));
    }
    let mut bytes = vec![0; 8];
    let (pointer, member) = native_ship(&mut bytes, reference(1), b"first", 0);
    let before = ship(&tracked(&bytes, None), 0, pointer, reference(1), 0);
    bytes[member + cname_group_member::CACHED_NAME..][..5].copy_from_slice(b"later");
    let after = ship(&tracked(&bytes, None), 0, pointer, reference(1), 0);
    assert!(matches!(before.cache, Cache::Read { bytes } if bytes == b"first"));
    assert!(matches!(after.cache, Cache::Read { bytes } if bytes == b"later"));
}

#[test]
fn ship_name_witness_refuses_stale_identity_and_faults_without_empty_substitution() {
    let mut bytes = vec![0; 8];
    let (pointer, member) = native_ship(&mut bytes, reference(1), b"retained", 0);
    for id in [0, reference(2), reference(1) ^ 1] {
        let memory = tracked(&bytes, None);
        assert!(matches!(
            ship(&memory, 0, pointer, id, 0).cache,
            Cache::Refused { .. }
        ));
        assert!(
            !memory
                .reads
                .borrow()
                .iter()
                .any(|&(at, _)| at == pointer + cship::NAME_MEMBER)
        );
    }
    for denied in [
        pointer + cship::NAME_MEMBER,
        member,
        member + cname_group_member::CACHED_NAME,
    ] {
        assert!(matches!(
            ship(&tracked(&bytes, Some(denied)), 0, pointer, reference(1), 0).cache,
            Cache::Refused { .. }
        ));
    }
    word(&mut bytes, pointer + cship::NAME_MEMBER, 0);
    assert!(matches!(
        ship(&tracked(&bytes, None), 0, pointer, reference(1), 0).cache,
        Cache::Refused { .. }
    ));
    word(&mut bytes, pointer + cship::NAME_MEMBER, member as u64);
    word(
        &mut bytes,
        member + cname_group_member::CACHED_NAME + msvc_string::CAPACITY,
        1,
    );
    assert!(matches!(
        ship(&tracked(&bytes, None), 0, pointer, reference(1), 0).cache,
        Cache::Refused { .. }
    ));
}

#[allow(clippy::type_complexity)]
fn world(countries: u32) -> (Vec<u8>, usize, Vec<(usize, u64, Vec<u8>)>) {
    let mut bytes = vec![0; 8];
    let state = allocate(&mut bytes, 0x1000);
    let roster = allocate(&mut bytes, (countries as usize + 1) * 8);
    let links = allocate(&mut bytes, (countries as usize + 1) * 4);
    let provinces = allocate(&mut bytes, 16);
    let province = allocate(&mut bytes, cprovince::ID + 8);
    word(&mut bytes, province, cprovince::VTABLE_RVA.0 as u64);
    integer(&mut bytes, province + cprovince::ID, 1);
    word(&mut bytes, provinces + 8, province as u64);
    array(
        &mut bytes,
        state + cgame_state::COUNTRIES,
        roster,
        countries + 1,
    );
    array(
        &mut bytes,
        state + cgame_state::COUNTRY_LINKS,
        links,
        countries + 1,
    );
    array(&mut bytes, state + cgame_state::PROVINCES, provinces, 2);
    let mut selected = Vec::new();
    for slot in 1..=countries {
        let country = allocate(&mut bytes, ccountry::FLEETS + 16);
        word(&mut bytes, roster + slot as usize * 8, country as u64);
        integer(&mut bytes, links + slot as usize * 4, slot);
        word(&mut bytes, country, ccountry::VTABLE_RVA.0 as u64);
        integer(&mut bytes, country + ccountry::ID, slot);
        let count = if slot == 1 { 513 } else { slot % 7 };
        if count == 0 {
            continue;
        }
        let fleet = allocate(&mut bytes, cfleet::NAME + msvc_string::STRIDE);
        let fleet_roster = allocate(&mut bytes, 8);
        word(&mut bytes, fleet_roster, fleet as u64);
        array(&mut bytes, country + ccountry::FLEETS, fleet_roster, 1);
        word(&mut bytes, fleet, cfleet::VTABLE_RVA.0 as u64);
        word(
            &mut bytes,
            fleet + cfleet::HANDLE,
            (u64::from(slot) << 32) | u64::from(cfleet::HANDLE_TYPE),
        );
        word(&mut bytes, fleet + cfleet::OWNER, country as u64);
        word(&mut bytes, fleet + cfleet::HOME_BASE, province as u64);
        let task = allocate(&mut bytes, 0x800);
        let tasks = allocate(&mut bytes, 8);
        word(&mut bytes, tasks, task as u64);
        array(&mut bytes, fleet + cfleet::TASK_FORCES, tasks, 1);
        word(&mut bytes, task, ctask_force::VTABLE_RVA.0 as u64);
        word(
            &mut bytes,
            task + ctask_force::SECONDARY_BASE,
            ctask_force::SECONDARY_VTABLE_RVA.0 as u64,
        );
        word(&mut bytes, task + ctask_force::FLEET, fleet as u64);
        let task_reference = (u64::from(slot) << 32) | u64::from(ctask_force::DEFAULT_HANDLE_TYPE);
        word(&mut bytes, task + cunit::HANDLE, task_reference);
        integer(&mut bytes, task + cunit::ROSTER_OWNER, slot);
        integer(&mut bytes, task + cunit::LOGICAL_OWNER, slot);
        word(&mut bytes, task + cunit::LOCATION, province as u64);
        let pointers = allocate(&mut bytes, count as usize * 8);
        array(&mut bytes, task + ctask_force::SHIPS, pointers, count);
        for index in 0..count {
            let id = reference(0x8000_0000 + selected.len() as u32);
            let text = if index == 0 {
                vec![b'm'; 257 + slot as usize % 31]
            } else {
                format!("synthetic-{slot}-{index}").into_bytes()
            };
            let (pointer, _) = native_ship(&mut bytes, id, &text, task);
            word(&mut bytes, pointers + index as usize * 8, pointer as u64);
            selected.push((pointer, id, text));
        }
    }
    (bytes, state, selected)
}

#[test]
fn ship_name_witness_reuses_one_census_for_countries_with_varied_navies() {
    let (bytes, state, expected) = world(12);
    let memory = tracked(&bytes, None);
    let (census, ships) = ships(&memory, 0, state);
    assert!(
        matches!(census, Census::Complete { rows } if rows == expected.len()),
        "{census:?}"
    );
    assert!(expected.len() > 513);
    assert_eq!(ships.len(), expected.len());
    for (ship, (_, id, text)) in ships.iter().zip(&expected) {
        assert_eq!(ship.reference, *id);
        assert!(matches!(&ship.cache, Cache::Read { bytes } if bytes == text));
    }
    let reads = memory.reads.borrow();
    assert_eq!(
        reads
            .iter()
            .filter(|&&(at, _)| at == state + cgame_state::COUNTRIES)
            .count(),
        1
    );
}

fn row(id: u32, text: &[u8]) -> ShipRow {
    ShipRow {
        id: reference(id),
        task_force: 61,
        name: Recorded::Value(Some(text.to_vec())),
        ..Default::default()
    }
}
fn recording(rows: Vec<ShipRow>, fresh: bool) -> Replay {
    let mut records = crate::test_support::allocation_records();
    for record in &mut records {
        match record {
            Record::SessionStart(session) => {
                session.identity.verification = Verification::Verified {
                    launcher: Roster::default(),
                    memory: Roster::default(),
                }
            }
            Record::Frame(frame) => {
                frame.tables[TableId::Ships.index()] =
                    TableUpdate::Snapshot(TableRows::Ships(rows.clone()));
                frame.metrics.table_ns[TableId::Ships.index()] = u64::from(fresh);
                frame.metrics.table_ns[TableId::Clock.index()] = 1;
            }
            _ => {}
        }
    }
    Replay::from_records(
        recorder_derive::writers(),
        crate::test_support::ALLOCATION_CAMPAIGN.to_vec(),
        &[records],
    )
    .unwrap()
}
fn report(rows: &[ShipRow], replay: &Replay) -> Report {
    let identity = ContentIdentity::from(&replay.identity);
    let clock = Clock {
        game_state: 8,
        campaign: replay.campaign_id.clone(),
        hour: 2,
        load_signal: 19,
        seed: 0,
        speed: 4,
        tick_busy: 0,
        tick_progress: 0,
    };
    Report {
        method: METHOD.into(),
        branch: 9,
        process_id: 2,
        process_start_time: 13,
        executable: PathBuf::from("synthetic/hoi4.exe"),
        identity,
        recorder_version: "synthetic".into(),
        recorder_executable: PathBuf::from("synthetic/recorder.exe"),
        recorder_hash: [3; 32],
        recorder_size: 1,
        expected_mounts: vec!["C:/synthetic/game".into()],
        observed_mounts: vec![b"C:/synthetic/game".to_vec()],
        paused_by_operator: true,
        before: clock.clone(),
        after: Some(clock),
        census: Census::Complete { rows: rows.len() },
        ships: rows
            .iter()
            .map(|row| Ship {
                reference: row.id,
                task_force: row.task_force,
                cache: match &row.name {
                    Recorded::Value(Some(bytes)) => Cache::Read {
                        bytes: bytes.clone(),
                    },
                    _ => Cache::Refused {
                        reason: "synthetic refusal".into(),
                    },
                },
            })
            .collect(),
        refusals: Vec::new(),
    }
}

#[test]
fn ship_name_witness_comparison_is_exact_for_full_references_empty_and_raw_bytes() {
    let rows = vec![
        row(1, b""),
        row(2, b"same"),
        row(3, b"same"),
        row(0xfedc_ba98, b"mod\0suffix\xff"),
    ];
    let replay = recording(rows.clone(), true);
    let report = report(&rows, &replay);
    let decoded: Report = serde_json::from_slice(&serde_json::to_vec(&report).unwrap()).unwrap();
    let result = compare(&decoded, &replay, 2, 9).unwrap();
    result.outcome().unwrap();
    assert_eq!(result.matched, 4);
    assert_eq!(result.replay_ships, Some(4));
}

#[test]
fn ship_name_witness_comparison_counts_each_missing_refused_mismatched_and_duplicate_ship() {
    let mut rows = vec![
        row(1, b"one"),
        row(2, b"two"),
        row(3, b"three"),
        row(4, b"four"),
        row(5, b"five"),
        row(6, b"six"),
    ];
    rows[2].name = Recorded::Value(None);
    let replay = recording(rows.clone(), true);
    let mut report = report(&rows, &replay);
    report.ships[0].cache = Cache::Read {
        bytes: b"ONE".to_vec(),
    };
    report.ships[1].cache = Cache::Refused {
        reason: "synthetic failed read".into(),
    };
    report.ships[2].cache = Cache::Read {
        bytes: b"three".to_vec(),
    };
    report.ships[3].task_force ^= 1;
    report.ships[4].reference = reference(99);
    report.ships.pop();
    report.ships.push(report.ships[3].clone());
    let result = compare(&report, &replay, 2, 9).unwrap();
    assert!(result.outcome().is_err());
    assert_eq!(result.mismatched, 1);
    assert_eq!((result.witness_refused, result.replay_refused), (1, 1));
    assert_eq!(
        (result.missing_from_replay, result.missing_from_witness),
        (1, 2)
    );
    assert_eq!(result.task_force_mismatched, 2);
    assert_eq!(result.duplicate_references, 1);
}

#[test]
fn ship_name_witness_comparison_refuses_wrong_identity_load_hour_branch_and_mount_proof() {
    let rows = vec![row(1, b"cache")];
    let replay = recording(rows.clone(), true);
    let original = report(&rows, &replay);
    for variant in 0..12 {
        let mut report = original.clone();
        match variant {
            0 => report.branch += 1,
            1 => {
                report.before.hour += 1;
                report.after = Some(report.before.clone());
            }
            2 => {
                report.before.load_signal += 1;
                report.after = Some(report.before.clone());
            }
            3 => report.process_start_time += 1,
            4 => report.process_id += 1,
            5 => report.after.as_mut().unwrap().tick_progress += 1,
            6 => report.before.tick_busy = 1,
            7 => report.paused_by_operator = false,
            8 => report.observed_mounts.clear(),
            9 => report.census = Census::Truncated { rows: 1, unread: 1 },
            10 => {
                report.census = Census::Refused {
                    reason: "synthetic census fault".into(),
                }
            }
            11 => report.identity.language = "synthetic-language".into(),
            _ => unreachable!(),
        }
        let result = compare(&report, &replay, 2, 9).unwrap();
        assert!(result.outcome().is_err(), "variant {variant}");
        assert!(!result.refusals.is_empty(), "variant {variant}");
    }
    let carried = recording(rows, false);
    assert!(
        compare(&original, &carried, 2, 9)
            .unwrap()
            .outcome()
            .is_err()
    );
    assert!(
        compare(&original, &replay, 3, 9)
            .unwrap()
            .outcome()
            .is_err()
    );
    assert!(
        compare(&original, &replay, 2, 10)
            .unwrap()
            .outcome()
            .is_err()
    );
}

#[test]
fn ship_name_witness_comparison_never_certifies_missing_census_or_empty_evidence() {
    let rows = vec![row(1, b"one")];
    let replay = recording(rows.clone(), true);
    let mut report = report(&rows, &replay);
    report.ships.clear();
    report.census = Census::Complete { rows: 0 };
    let result = compare(&report, &replay, 2, 9).unwrap();
    assert_eq!(result.missing_from_witness, 1);
    assert!(result.outcome().is_err());
    let mut malformed = serde_json::to_value(&report).unwrap();
    malformed["identity"]["exe_hash"] = serde_json::json!([0]);
    assert!(serde_json::from_value::<Report>(malformed).is_err());
}

#[test]
fn ship_name_witness_command_routes_offline_comparison_and_rejects_invalid_capture_options() {
    let root = crate::test_support::allocation_test_directory("ship-name-witness");
    let rows = vec![row(0x8765_4321, b"synthetic\0name\xff")];
    let replay = recording(rows.clone(), true);
    let mut report = report(&rows, &replay);
    let report_path = root.join("witness.json");
    let replay_path = root.join("names.battleplan");
    write_report(&report_path, &report).unwrap();
    assert!(write_report(&report_path, &report).is_err());
    fs::write(&replay_path, replay.bytes()).unwrap();
    let args = || {
        vec![
            "ship-name-witness".into(),
            "--compare".into(),
            report_path.clone().into_os_string(),
            replay_path.clone().into_os_string(),
            "--hour".into(),
            "2".into(),
            "--branch".into(),
            "9".into(),
        ]
    };
    crate::recording::command(args()).unwrap();
    report.ships[0].cache = Cache::Read {
        bytes: b"not the same".to_vec(),
    };
    fs::write(&report_path, serde_json::to_vec(&report).unwrap()).unwrap();
    assert!(
        crate::recording::command(args())
            .unwrap_err()
            .contains("1 mismatched")
    );
    for args in [
        vec![],
        vec!["--paused"],
        vec!["--paused", "--hour", "2", "--branch", "0"],
        vec!["--paused", "--hour", "2", "--branch", "9", "--branch", "9"],
    ] {
        assert!(options(args.into_iter().map(Into::into).collect()).is_err());
    }
    crate::recording::command(vec!["ship-name-witness".into(), "--help".into()]).unwrap();
}

#[test]
fn ship_name_witness_census_preserves_independent_ships_when_one_cache_refuses() {
    let (bytes, state, expected) = world(1);
    let memory = tracked(&bytes, Some(expected[0].0 + cship::NAME_MEMBER));
    let (census, ships) = ships(&memory, 0, state);
    assert!(matches!(census, Census::Complete { rows } if rows == expected.len()));
    assert!(matches!(ships[0].cache, Cache::Refused { .. }));
    assert_eq!(ships.len(), expected.len());
    assert!(
        ships[1..]
            .iter()
            .all(|ship| matches!(ship.cache, Cache::Read { .. }))
    );
    let (census, rows) = super::ships(
        &tracked(&bytes, Some(state + cgame_state::COUNTRIES)),
        0,
        state,
    );
    assert!(matches!(census, Census::Refused { .. }));
    assert!(rows.is_empty());
}

#[test]
fn ship_name_witness_heap_faults_and_first_nul_suffix_differences_remain_failures() {
    let text = b"a long retained cache\0with a suffix";
    let mut bytes = vec![0; 8];
    let (pointer, member) = native_ship(&mut bytes, reference(1), text, 0);
    let header = member + cname_group_member::CACHED_NAME;
    let heap = u64::from_le_bytes(bytes[header..header + 8].try_into().unwrap()) as usize;
    assert!(matches!(
        ship(
            &tracked(&bytes, Some(heap + 5)),
            0,
            pointer,
            reference(1),
            0
        )
        .cache,
        Cache::Refused { .. }
    ));
    bytes[heap + text.len()] = 1;
    assert!(matches!(
        ship(&tracked(&bytes, None), 0, pointer, reference(1), 0).cache,
        Cache::Refused { .. }
    ));
    let rows = vec![row(1, text)];
    let replay = recording(rows.clone(), true);
    let mut report = report(&rows, &replay);
    let Cache::Read { bytes } = &mut report.ships[0].cache else {
        unreachable!()
    };
    *bytes.last_mut().unwrap() ^= 1;
    let result = compare(&report, &replay, 2, 9).unwrap();
    assert_eq!(result.mismatched, 1);
    assert!(result.outcome().is_err());
}

#[test]
fn ship_name_witness_refuses_unavailable_replay_census_and_unverified_content() {
    let rows = vec![row(1, b"observed")];
    let replay = recording(rows.clone(), true);
    let report = report(&rows, &replay);
    for variant in 0..2 {
        let mut records = crate::test_support::allocation_records();
        for record in &mut records {
            match record {
                Record::SessionStart(session) => {
                    session.identity.verification = if variant == 1 {
                        Verification::Unchecked
                    } else {
                        Verification::Verified {
                            launcher: Roster::default(),
                            memory: Roster::default(),
                        }
                    };
                }
                Record::Frame(frame) => {
                    frame.metrics.table_ns[TableId::Ships.index()] = 1;
                    frame.tables[TableId::Ships.index()] = if variant == 0 {
                        TableUpdate::Unavailable {
                            reason: "synthetic census failure".into(),
                        }
                    } else {
                        TableUpdate::Snapshot(TableRows::Ships(rows.clone()))
                    };
                }
                _ => {}
            }
        }
        let replay = Replay::from_records(
            recorder_derive::writers(),
            crate::test_support::ALLOCATION_CAMPAIGN.to_vec(),
            &[records],
        )
        .unwrap();
        let result = compare(&report, &replay, 2, 9).unwrap();
        assert!(result.outcome().is_err(), "variant {variant}");
        assert!(!result.refusals.is_empty(), "variant {variant}");
        if variant == 0 {
            assert_eq!(result.replay_ships, None);
        }
    }
}

#[test]
fn ship_name_witness_counts_replay_refusals_even_without_a_matching_witness_ship() {
    let mut rows = vec![row(1, b"observed"), row(2, b"unavailable")];
    rows[1].name = Recorded::Value(None);
    let replay = recording(rows.clone(), true);
    let report = report(&rows[..1], &replay);
    let result = compare(&report, &replay, 2, 9).unwrap();
    assert_eq!(result.matched, 1);
    assert_eq!(result.replay_refused, 1);
    assert_eq!(result.missing_from_witness, 1);
    assert!(result.outcome().is_err());
    assert_eq!(result.ships[&rows[1].id].len(), 2);
}
