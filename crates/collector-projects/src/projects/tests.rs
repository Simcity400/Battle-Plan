use super::*;

#[test]
fn projects_job_capacity_accounting_tracks_all_vectors_through_growth_clear_and_drop() {
    use crate::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
    static LOCAL: Tracker = Tracker::new();
    let mut job = ProjectsJob::default();
    job.census.reserve_exact(3);
    job.census.resize(3, Default::default());
    job.statuses.reserve_exact(3);
    job.pointers.reserve_exact(3);
    job.capacity_charge = Charge::empty_on(&LOCAL);
    job.update_capacity_charge();
    let expected = |job: &ProjectsJob| {
        vector_bytes(&job.census)
            + vector_bytes(&job.statuses)
            + vector_bytes(&job.pointers)
            + vector_bytes(&job.assignment_references)
    };
    let initial = LOCAL.snapshot().current.auxiliary_vectors;
    assert_eq!(initial, expected(&job));
    job.grow();
    let grown = LOCAL.snapshot().current.auxiliary_vectors;
    assert!(grown > initial);
    assert_eq!(grown, expected(&job));
    job.clear();
    assert_eq!(LOCAL.snapshot().current.auxiliary_vectors, grown);
    drop(job);
    assert_eq!(LOCAL.snapshot().current, Capacity::default());
}

use replay_data::rows::TableSchema;

fn small() -> (Capacities, Captured) {
    let capacities = Capacities {
        rows: [2; N],
        program_names: 4,
        program_candidates: 2,
        raid_type_tokens: 2,
        raid_show_for: 2,
        raid_transports: 16,
        raid_equipment_items: 16,
        raid_equipment_archetypes: 16,
        counteroperatives: 16,
    };
    let captured = Captured::with_capacities(&capacities);
    (capacities, captured)
}

#[test]
fn refused_scientist_activity_keeps_membership_and_skills() {
    let (_, mut captured) = small();
    captured.scientists.rows[0] = scientist_roster::Row {
        country: 5_000,
        character: 7,
        skill_count: 1,
        active: None,
        ..Default::default()
    };
    captured.scientists.outcome = Outcome::Rows { len: 1 };
    captured.scientist_skills.rows[0] = scientist_roster::SkillRow {
        character: 7,
        token: u32::MAX,
        experience_raw: i64::MIN,
        ..Default::default()
    };
    captured.scientist_skills.outcome = Outcome::Rows { len: 1 };
    let tables = captured.tables();
    let TableView::Complete(rows::TableRows::Scientists(scientists)) = &tables[8] else {
        panic!("scientist membership lost");
    };
    assert_eq!(scientists[0].country, 5_000);
    assert_eq!(scientists[0].skill_count, 1);
    assert_eq!(scientists[0].active, rows::Recorded::Value(None));
    let TableView::Complete(rows::TableRows::ScientistSkills(skills)) = &tables[9] else {
        panic!("scientist specialization lost");
    };
    assert_eq!(skills[0].token, u32::MAX);
    assert_eq!(skills[0].experience_raw, i64::MIN);
}

#[test]
fn daily_project_siblings_do_no_inactive_native_work_after_shared_faults() {
    struct Unreadable;
    impl Memory for Unreadable {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            false
        }
    }
    let memory = engine_native_memory::memory::CountedMemory::new(Unreadable);
    let (_, mut captured) = small();
    for hour in 0..24 {
        let accounting = read_accounting::Capture::new();
        captured.capture(&memory, 0, 0x1000, hour, false);
        let counts = accounting.finish();
        for (index, outcome) in captured.outcomes().into_iter().enumerate() {
            if !replay_data::cadence::should_walk(TABLES[index], hour, false) {
                assert_eq!(
                    outcome,
                    Outcome::Unavailable {
                        reason: replay_data::cadence::SKIPPED_REASON
                    }
                );
                assert_eq!(counts[TABLES[index].index()].spans, 0);
            }
        }
    }
}

#[test]
fn daily_project_job_skips_empty_storage_and_preserves_raw_typed_identity() {
    use super::projects_test_kit as kit;
    let bytes = kit::fixture();
    let (capacities, _) = small();
    for hour in 0..24 {
        let active = replay_data::cadence::should_walk(TableId::ProjectPrograms, hour, false);
        assert_eq!(
            active,
            Some(hour as u32) == replay_data::cadence::refresh_slot(TableId::ProjectPrograms)
        );
        let memory = ProgramSource::new(&bytes);
        let mut job = if active {
            kit::job()
        } else {
            ProjectsJob::default()
        };
        let report = copy_projects_refresh(
            &memory,
            ProjectsCopyContext {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                active,
            },
            &mut job,
        );
        assert!(report.raw_ok);
        assert_eq!(job.active, active);
        let mut captured = Captured::with_capacities(&capacities);
        captured.nukes.outcome = Outcome::Rows { len: 1 };
        let mut ticks = [0; RAW.len()];
        captured.decode_projects_payload(kit::BASE, &job, &mut ticks);
        if active {
            let direct = kit::direct(&kit::memory(&bytes), &capacities);
            kit::assert_matches_direct(&captured, &direct);
            assert!(!memory.calls.borrow().is_empty());
        } else {
            assert!(memory.calls.borrow().is_empty());
            assert_eq!(report.copy_ticks, [0; RAW.len()]);
            assert_eq!(report.raw_bytes, [0; RAW.len()]);
            assert_eq!(ticks, [0; RAW.len()]);
            assert!([0, 3].iter().all(|&index| captured.outcomes()[index]
                == Outcome::Unavailable {
                    reason: replay_data::cadence::SKIPPED_REASON,
                }));
            captured.fail_projects_capture(false, "accepted worker stopped");
            assert_eq!(
                captured.programs.outcome,
                Outcome::Unavailable {
                    reason: replay_data::cadence::SKIPPED_REASON
                }
            );
        }
        assert_eq!(captured.nukes.outcome, Outcome::Rows { len: 1 });
    }
}

#[test]
fn failed_project_job_hides_prior_rows_and_preserves_independent_results() {
    let (_, mut captured) = small();
    captured.programs.outcome = Outcome::Rows { len: 1 };
    captured.projects.outcome = Outcome::Rows { len: 1 };
    captured.history.outcome = Outcome::Rows { len: 1 };
    captured.breakthroughs.outcome = Outcome::Rows { len: 1 };
    captured.nukes.outcome = Outcome::Rows { len: 1 };
    captured.raids.outcome = Outcome::Rows { len: 1 };
    let before = captured.outcomes();
    captured.raw_mode = true;
    captured.fail_projects_capture(true, "project acquisition worker failed");
    assert!(!captured.raw_mode);
    let after = captured.outcomes();
    let tables = captured.tables();
    for index in 0..N {
        if [0, 3].contains(&index) {
            assert!(
                matches!(after[index], Outcome::Unavailable { .. }),
                "{index}"
            );
            assert!(
                matches!(tables[index], TableView::Unavailable { .. }),
                "{index}"
            );
        } else {
            assert_eq!(after[index], before[index], "{index}");
        }
    }
}

fn complete<S: TableSchema>(view: &TableView) -> &[S::Row] {
    let TableView::Complete(rows) = view else {
        panic!("expected complete table")
    };
    rows.rows::<S>().unwrap()
}

#[test]
fn conversion_preserves_native_keys_fixed_values_presence_and_literal_building_bytes() {
    let (_, mut captured) = small();
    captured.programs.outcome = Outcome::Rows { len: 1 };
    captured.programs.rows[0] = nproject_cprogram::Row {
        id: u32::MAX,
        active_project: Some(0),
        scientist: u64::MAX,
        unread_rewards: i32::MIN,
        building_start: 1,
        building_len: 3,
        ..Default::default()
    };
    captured.names.first.copy_from_slice(&[99, 0xff, 0, b'a']);
    captured.names.len = 4;
    captured.names.required = 4;
    captured.projects.outcome = Outcome::Rows { len: 1 };
    captured.projects.rows[0] = nproject_cproject::Row {
        source_pointer: 0,
        owner: u32::MAX,
        local_id: u32::MAX,
        definition_id: u32::MAX,
        program_reference: u64::MAX,
        started: 1,
        completed: 0,
        state: engine_projects::nproject_cproject_state_machine::State {
            current_offset: u32::MAX,
            state_enums: [1, 2, 3, u32::MAX],
            not_completed: 1,
            phase_progress: i64::MIN,
            phase_target: i32::MAX,
            prototype_progress: i32::MIN,
            iterations: -17,
        },
    };
    captured.history.outcome = Outcome::Rows { len: 1 };
    captured.history.rows[0] = nproject_sproject_history::Row {
        owner: u32::MAX,
        project: u32::MAX,
        ordinal: u32::MAX,
        prototype_reward_handle: u32::MAX,
        option_handle: u32::MAX - 1,
        date: i32::MIN,
        context: engine_projects::nproject_sproject_context::Row {
            province: i32::MIN,
            progress: i32::MAX,
            character_reference: u64::MAX,
            country_present: 1,
            country_reference: Some(u32::MAX),
            scientist_present: 0,
            scientist_reference: None,
            state_present: 1,
            state_reference: Some(0),
        },
    };
    captured.breakthroughs.outcome = Outcome::Rows { len: 1 };
    captured.breakthroughs.rows[0] = nproject_cbreakthrough_progress::Row {
        owner: u32::MAX,
        specialization: u32::MAX,
        points: i32::MIN,
    };
    captured.nukes.outcome = Outcome::Rows { len: 1 };
    captured.nukes.rows[0] = cnuke::Row {
        country: u32::MAX,
        nuke_type: 1,
        amount_raw: i64::MAX,
        ready: i32::MIN,
    };
    captured.raid_countries.outcome = Outcome::Rows { len: 1 };
    captured.raid_countries.rows[0] = ccountry_raid_status::Row {
        canonical_slot: 0,
        country: -1,
        eligible_start: 0,
        eligible_len: 1,
        used_start: 1,
        used_len: 1,
    };
    captured.raid_type_tokens.first[..2].copy_from_slice(&[u32::MAX, 7]);
    captured.raid_type_tokens.len = 2;
    captured.raid_player.outcome = Outcome::Rows { len: 1 };
    captured.raid_player.rows[0] = engine_visibility::cplayer_status_cache::Row {
        country: u32::MAX,
        allowed_start: 1,
        allowed_len: 1,
    };
    captured.raid_show_for.first[0] = u32::MAX;
    captured.raid_show_for.len = 1;
    captured.raids.outcome = Outcome::Rows { len: 1 };
    captured.raids.rows[0] = craid_instance::Row {
        reference_type: u32::MAX - 1,
        id: u32::MAX,
        parent_slot: 0,
        ordinal: 7,
        phase: 3,
        outcome: -1,
        show_for_start: 0,
        show_for_len: 1,
        source: engine_native_air::craid_source::Row {
            kind: None,
            kind_present: 0,
            province: u32::MAX,
            ship_reference_type: u32::MAX,
            ship_reference_id: u32::MAX - 1,
            country: -1,
            building_type: u32::MAX,
            building_location: i32::MIN,
            building_country: i32::MAX,
        },
        ..Default::default()
    };
    captured.peace.outcome = Outcome::Rows { len: 1 };
    captured.peace.rows[0] = peace_conference::Row {
        id: u64::MAX,
        ordinal: u32::MAX,
        current_turn: i32::MIN,
        finalized: 1,
    };
    let views = captured.tables();
    let program = &complete::<tables::ProjectPrograms>(&views[0])[0];
    assert_eq!(
        (
            program.active_project,
            program.scientist,
            program.unread_rewards
        ),
        (Some(0), u64::MAX, i32::MIN)
    );
    assert_eq!(program.building, [0xff, 0, b'a']);
    let project = &complete::<tables::Projects>(&views[1])[0];
    assert_eq!(project.key(), u128::from(u64::MAX));
    assert_eq!(
        (project.definition_id, project.program_reference),
        (u32::MAX, u64::MAX)
    );
    assert_eq!(
        (
            project.state.phase_progress,
            project.state.phase_target,
            project.state.prototype_progress,
            project.state.iterations
        ),
        (i64::MIN, i32::MAX, i32::MIN, -17)
    );
    let history = &complete::<tables::ProjectHistory>(&views[2])[0];
    assert_eq!(history.key(), (1_u128 << 96) - 1);
    assert_eq!(
        (history.prototype_reward_handle, history.option_handle),
        (u32::MAX, u32::MAX - 1)
    );
    assert_eq!(
        (
            history.context.country_reference,
            history.context.scientist_reference,
            history.context.state_reference
        ),
        (Some(u32::MAX), None, Some(0))
    );
    let breakthrough = &complete::<tables::ProjectBreakthroughs>(&views[3])[0];
    assert_eq!(
        (breakthrough.key(), breakthrough.points),
        (u128::from(u64::MAX), i32::MIN)
    );
    let nuke = &complete::<tables::Nukes>(&views[4])[0];
    assert_eq!((nuke.amount_raw, nuke.ready), (i64::MAX, i32::MIN));
    let parent = &complete::<tables::RaidCountries>(&views[5])[0];
    assert_eq!(
        (
            parent.canonical_slot,
            parent.country,
            parent.eligible_types.as_slice(),
            parent.used_types.as_slice()
        ),
        (0, -1, &[u32::MAX][..], &[7][..])
    );
    let player = &complete::<tables::RaidPlayerAllowedTypes>(&views[13])[0];
    assert_eq!(
        (player.country, player.allowed.as_slice()),
        (u32::MAX, &[7][..])
    );
    let raid = &complete::<tables::Raids>(&views[6])[0];
    assert_eq!(
        raid.key(),
        (u128::from(u32::MAX) << 32) | u128::from(u32::MAX - 1)
    );
    assert_eq!(
        (
            raid.source.kind,
            raid.source.province,
            raid.source.ship_reference_id
        ),
        (None, u32::MAX, u32::MAX - 1)
    );
    assert_eq!(raid.show_for, [u32::MAX]);
    let peace = &complete::<tables::PeaceConferences>(&views[7])[0];
    assert_eq!(
        (peace.id, peace.ordinal, peace.current_turn, peace.finalized),
        (u64::MAX, u32::MAX, i32::MIN, 1)
    );
}

#[test]
fn invalid_building_ranges_and_impossible_row_counts_refuse_without_panicking() {
    let (_, mut captured) = small();
    captured.programs.outcome = Outcome::Rows { len: 1 };
    captured.names.len = 2;
    captured.names.required = 2;
    for (start, len) in [(1, 2), (usize::MAX, 1), (4, 0)] {
        captured.programs.rows[0].building_start = start;
        captured.programs.rows[0].building_len = len;
        assert!(matches!(
            captured.program_table(captured.programs.outcome),
            TableView::Unavailable { .. }
        ));
    }
    for outcome in [
        Outcome::Rows { len: 3 },
        Outcome::Truncated {
            len: 1,
            unread_rows: 0,
        },
    ] {
        assert!(matches!(bounded(outcome, 2), Outcome::Unavailable { .. }));
    }
    assert!(matches!(
        program_outcome(
            nproject_cprogram::NameWalk {
                outcome: Outcome::Rows { len: 0 },
                bytes_len: 0,
                required_bytes: 1,
                required_candidates: 1,
            },
            &[],
            &[],
        ),
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn program_capture_reads_once_and_preserves_primary_literal_and_truncation() {
    let (_, mut captured) = small();
    let mut calls = 0;
    captured.capture_programs(|_, rows, bytes| {
        calls += 1;
        rows[0] = nproject_cprogram::Row {
            id: 1,
            building_len: 1,
            ..Default::default()
        };
        bytes[0] = 0xff;
        nproject_cprogram::NameWalk {
            outcome: Outcome::Rows { len: 1 },
            bytes_len: 1,
            required_bytes: 1,
            required_candidates: 1,
        }
    });
    assert_eq!(calls, 1);
    assert_eq!(captured.programs.outcome, Outcome::Rows { len: 1 });
    assert_eq!(captured.names.first[0], 0xff);
    captured.reset();
    captured.capture_programs(|_, _, _| nproject_cprogram::NameWalk {
        outcome: Outcome::Truncated {
            len: 0,
            unread_rows: 1,
        },
        bytes_len: 0,
        required_bytes: 9,
        required_candidates: 1,
    });
    assert_eq!(
        captured.programs.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    assert_eq!(captured.names.required, 9);
}

#[test]
fn shared_raid_capture_reads_once_and_retains_independent_capacity_outcomes() {
    let (_, mut captured) = small();
    let mut calls = 0;
    capture_pair(
        &mut captured.raid_countries,
        &mut captured.raids,
        |parents, instances| {
            calls += 1;
            parents[0] = ccountry_raid_status::Row {
                country: 1,
                ..Default::default()
            };
            instances[0] = craid_instance::Row {
                id: 7,
                phase: 3,
                ..Default::default()
            };
            (Outcome::Rows { len: 1 }, Outcome::Rows { len: 1 })
        },
    );
    assert_eq!(calls, 1);
    assert_eq!(&captured.outcomes()[5..7], &[Outcome::Rows { len: 1 }; 2]);
    assert_eq!(captured.raids.rows[0].phase, 3);
    captured.raids.rows.clear();
    capture_pair(
        &mut captured.raid_countries,
        &mut captured.raids,
        |parents, instances| {
            calls += 1;
            assert_eq!(parents.len(), 2);
            assert!(instances.is_empty());
            (
                Outcome::Rows { len: 1 },
                Outcome::Truncated {
                    len: 0,
                    unread_rows: 1,
                },
            )
        },
    );
    assert_eq!(calls, 2);
    assert_eq!(captured.raid_countries.outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        captured.raids.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
}

#[test]
fn scratch_and_arena_growth_preserve_primary_requirements_after_failure() {
    let (mut capacities, mut captured) = small();
    captured.capture_programs(|_, _, _| nproject_cprogram::NameWalk {
        outcome: Outcome::Unavailable {
            reason: "program pointer scratch capacity insufficient",
        },
        bytes_len: 0,
        required_bytes: 17,
        required_candidates: 9,
    });
    captured.projects.outcome = Outcome::Truncated {
        len: 2,
        unread_rows: 63,
    };
    captured.grow(&mut capacities);
    assert!(captured.candidates.len() >= 9 && captured.names.first.len() >= 17);
    assert!(captured.projects.rows.len() >= 65);
    let before = captured.allocated_bytes();
    captured.reset();
    captured.grow(&mut capacities);
    assert_eq!(captured.allocated_bytes(), before);
}

fn split_capacities() -> Capacities {
    Capacities {
        rows: [256; N],
        program_names: 8192,
        program_candidates: 1024,
        raid_type_tokens: 256,
        raid_show_for: 256,
        raid_transports: 16,
        raid_equipment_items: 16,
        raid_equipment_archetypes: 16,
        counteroperatives: 16,
    }
}

struct ProgramSource<'a> {
    memory: engine_native_memory::memory::SliceMemory<'a>,
    pointer: usize,
    fail: Option<usize>,
    partial: usize,
    calls: std::cell::RefCell<Vec<(usize, usize)>>,
    scatters: std::cell::RefCell<Vec<Vec<(usize, usize)>>>,
}

impl<'a> ProgramSource<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        use super::projects_test_kit as kit;
        Self {
            memory: kit::memory(bytes),
            pointer: kit::GAME_STATE + kit::PROG1,
            fail: None,
            partial: 0,
            calls: Default::default(),
            scatters: Default::default(),
        }
    }
}

impl Memory for ProgramSource<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        use super::projects_test_kit as kit;
        let ordinal = self.calls.borrow().len();
        self.calls.borrow_mut().push((address, destination.len()));
        if self.fail == Some(ordinal) {
            let partial = self.partial.min(destination.len());
            destination[..partial].fill(0xe3);
            return false;
        }
        let source = address
            .checked_sub(self.pointer)
            .filter(|&offset| offset < nproject_cprogram::OWNER_COPY + 12)
            .map_or(address, |offset| kit::GAME_STATE + kit::PROG1 + offset);
        self.memory.copy(source, destination)
    }

    fn copy_scattered_prefetched<const M: usize>(
        &self,
        addresses: [usize; M],
        destinations: &mut [&mut [u8]; M],
    ) -> usize {
        self.scatters.borrow_mut().push(
            addresses
                .iter()
                .copied()
                .zip(destinations.iter().map(|field| field.len()))
                .collect(),
        );
        self.copy_scattered(addresses, destinations)
    }
}

fn program_links(bytes: &[u8]) -> Array {
    let start = cgame_state::COUNTRY_LINKS;
    Array::from_header(bytes[start..start + 16].try_into().unwrap(), 4).unwrap()
}

const COUNTRIES: Array = Array { data: 0, count: 3 };

fn copy_fixture_program(log: &mut RawLog<2>, memory: &ProgramSource<'_>) -> bool {
    use super::projects_test_kit as kit;
    let links = program_links(memory.memory.bytes);
    copy_program_row(
        log,
        memory,
        kit::BASE,
        ProjectCensus {
            game_state: kit::GAME_STATE,
            links: &links,
            countries: &COUNTRIES,
            vtable: 0,
            read_to: 3,
            census: &[CensusEntry {
                pointer: kit::GAME_STATE + kit::C1,
                id: 1,
                slot: 1,
                canonical: 1,
            }],
        },
        memory.pointer,
        &mut Vec::with_capacity(1),
    )
}

fn program_script(bytes: &[u8], pointer: usize) -> Vec<(Option<usize>, usize)> {
    use super::projects_test_kit as kit;
    let at = |offset| pointer.checked_add(offset);
    let heap = kit::GAME_STATE + kit::HEAP;
    let text_len = b"program-building-01".len();
    vec![
        (Some(pointer), 16),
        (at(nproject_cprogram::OWNER_PRIMARY), 8),
        (at(nproject_cprogram::OWNER_COPY), 12),
        (at(nproject_cprogram::SCIENTIST_REFERENCE_TYPE), 8),
        (at(nproject_cprogram::ACTIVE_FLAG), 1),
        (at(nproject_cprogram::BUILDING), 32),
        (at(nproject_cprogram::ACTIVE_PROJECT_HANDLE), 4),
        (Some(heap + text_len), 1),
        (Some(heap), text_len),
        (Some(program_links(bytes).data + 4), 4),
        (
            at(nproject_cprogram::SCIENTIST_REFERENCE_TYPE + nproject_cprogram::ASSIGNMENT_TIMER),
            8,
        ),
        (
            at(nproject_cprogram::SCIENTIST_REFERENCE_TYPE + nproject_cprogram::ASSIGNMENT_TIMED),
            1,
        ),
        (at(nproject_cprogram::SUPPORTING_SCIENTISTS), 16),
        (at(0x48), 8),
    ]
}

fn scalar_program_script(
    log: &mut RawLog<2>,
    memory: &ProgramSource<'_>,
    script: &[(Option<usize>, usize)],
) -> bool {
    let child = script
        .iter()
        .position(|&(address, _)| {
            address
                .and_then(|address| address.checked_sub(memory.pointer))
                .is_some_and(|offset| {
                    offset
                        == nproject_cprogram::SCIENTIST_REFERENCE_TYPE
                            + nproject_cprogram::ASSIGNMENT_TIMER
                        || offset == nproject_cprogram::SUPPORTING_SCIENTISTS
                })
        })
        .unwrap_or(script.len());
    if !script[..child]
        .iter()
        .all(|&(address, len)| span(log, memory, address, len, M_PROGRAMS).is_some())
    {
        return false;
    }
    let _ = script[child..]
        .iter()
        .all(|&(address, len)| span(log, memory, address, len, M_PROGRAMS).is_some());
    true
}

fn dirty_program_log(span_capacity: usize, byte_capacity: usize) -> RawLog<2> {
    let mut log = RawLog::new(span_capacity, byte_capacity);
    log.bytes.extend_from_slice(&vec![0xa7; byte_capacity]);
    log.clear();
    log
}

fn assert_program_logs_equal(actual: &RawLog<2>, expected: &RawLog<2>) {
    assert_eq!(actual.spans, expected.spans);
    assert_eq!(actual.bytes, expected.bytes);
    assert_eq!(actual.fault, expected.fault);
    assert_eq!(actual.raw_bytes, expected.raw_bytes);
    assert_eq!(actual.overflowed, expected.overflowed);
    assert_eq!(actual.has_bulk, expected.has_bulk);
    assert_eq!(actual.grow_for_next(), expected.grow_for_next());
}

#[test]
fn program_body_batch_matches_scalar_at_every_fault_and_partial_byte() {
    let bytes = projects_test_kit::fixture();
    let pointer = projects_test_kit::GAME_STATE + projects_test_kit::PROG1;
    let script = program_script(&bytes, pointer);
    for failed in 0..=script.len() {
        for partial in 0..=script.get(failed).map_or(0, |field| field.1) {
            let source = ProgramSource {
                fail: (failed < script.len()).then_some(failed),
                partial,
                ..ProgramSource::new(&bytes)
            };
            let reference = ProgramSource {
                fail: source.fail,
                partial,
                ..ProgramSource::new(&bytes)
            };
            let mut actual = dirty_program_log(16, 256);
            let mut expected = dirty_program_log(16, 256);
            assert_eq!(
                copy_fixture_program(&mut actual, &source),
                scalar_program_script(&mut expected, &reference, &script),
                "failed={failed}, partial={partial}"
            );
            assert_program_logs_equal(&actual, &expected);
            assert_eq!(*source.calls.borrow(), *reference.calls.borrow());
            assert_eq!(source.calls.borrow().len(), (failed + 1).min(script.len()));
            assert!(actual.bytes.get(actual.bytes.len()).is_none());
            assert_eq!(source.scatters.borrow().len(), usize::from(failed != 0));
            if failed != 0 {
                let body: Vec<_> = script[1..6]
                    .iter()
                    .map(|&(address, len)| (address.unwrap(), len))
                    .collect();
                assert_eq!(source.scatters.borrow()[0], body);
            }
            if failed == script.len() {
                assert_eq!(
                    actual.spans[1..6].iter().map(|span| span.len).sum::<u32>(),
                    61
                );
                assert!(actual.spans.iter().all(|span| span.mask == M_PROGRAMS));
                assert_eq!(actual.raw_bytes, [138, 0]);
            }
        }
    }
}

#[test]
fn program_body_batch_preserves_every_capacity_prefix_and_growth() {
    let bytes = projects_test_kit::fixture();
    let pointer = projects_test_kit::GAME_STATE + projects_test_kit::PROG1;
    let script = program_script(&bytes, pointer);
    for span_capacity in 0..=14 {
        for byte_capacity in 0..=131 {
            let source = ProgramSource::new(&bytes);
            let reference = ProgramSource::new(&bytes);
            let mut actual = dirty_program_log(span_capacity, byte_capacity);
            let mut expected = dirty_program_log(span_capacity, byte_capacity);
            assert_eq!(
                copy_fixture_program(&mut actual, &source),
                scalar_program_script(&mut expected, &reference, &script),
                "spans={span_capacity}, bytes={byte_capacity}"
            );
            assert_program_logs_equal(&actual, &expected);
            assert_eq!(*source.calls.borrow(), *reference.calls.borrow());
            assert_eq!(
                source.scatters.borrow().len(),
                usize::from(span_capacity >= 6 && byte_capacity >= 77)
            );
            actual.grow();
            expected.grow();
            assert_eq!(actual.spans.capacity(), expected.spans.capacity());
            assert_eq!(actual.bytes.capacity(), expected.bytes.capacity());
        }
    }
    let source = ProgramSource::new(&bytes);
    let mut log = dirty_program_log(16, 25);
    assert!(copy_fixture_program(&mut log, &source));
    assert!(source.scatters.borrow().is_empty());
}

#[test]
fn program_body_batch_preserves_checked_address_and_canonical_prefixes() {
    let bytes = projects_test_kit::fixture();
    for pointer in [
        0,
        usize::MAX - 15,
        usize::MAX - 16,
        usize::MAX - 32,
        usize::MAX - 0x100,
        usize::MAX - 0x170,
        usize::MAX - 0x178,
        usize::MAX - 0x17c,
        0x0000_7fff_ffff_ff00,
        0x0000_8000_0000_0000,
    ] {
        let source = ProgramSource {
            pointer,
            ..ProgramSource::new(&bytes)
        };
        let reference = ProgramSource {
            pointer,
            ..ProgramSource::new(&bytes)
        };
        let script = program_script(&bytes, pointer);
        let mut actual = dirty_program_log(16, 256);
        let mut expected = dirty_program_log(16, 256);
        assert_eq!(
            copy_fixture_program(&mut actual, &source),
            scalar_program_script(&mut expected, &reference, &script),
            "pointer={pointer:#x}"
        );
        assert_program_logs_equal(&actual, &expected);
        assert_eq!(*source.calls.borrow(), *reference.calls.borrow());
        if script[1..6].iter().any(|field| field.0.is_none()) {
            assert!(source.scatters.borrow().is_empty());
        }
    }
}

#[test]
fn program_body_batch_keeps_identity_owner_presence_and_string_gates() {
    use super::projects_test_kit as kit;
    let pointer = kit::GAME_STATE + kit::PROG1;
    for case in 0..=14 {
        let mut bytes = kit::fixture();
        let mut script = program_script(&bytes, pointer);
        let building = kit::PROG1 + nproject_cprogram::BUILDING;
        let success = match case {
            0 => true,
            1..=3 => {
                let (offset, width) = match case {
                    1 => (0, 8),
                    2 => (nproject_cprogram::TYPE, 4),
                    _ => (nproject_cprogram::ID, 4),
                };
                bytes[kit::PROG1 + offset..kit::PROG1 + offset + width].fill(0);
                script.truncate(1);
                false
            }
            4 => {
                bytes[kit::PROG1 + nproject_cprogram::OWNER_COPY] = 2;
                script.truncate(6);
                false
            }
            5 => {
                bytes[kit::PROG1 + nproject_cprogram::ACTIVE_FLAG] = 2;
                script.truncate(6);
                false
            }
            6 => {
                bytes[kit::PROG1 + nproject_cprogram::ACTIVE_FLAG] = 0;
                script.remove(6);
                true
            }
            7..=10 => {
                match case {
                    7 => bytes[building + 24..building + 32].copy_from_slice(&14_u64.to_le_bytes()),
                    8 => bytes[building + 16..building + 24].copy_from_slice(&20_u64.to_le_bytes()),
                    9 => bytes[building..building + 8].fill(0),
                    _ => bytes[building..building + 8]
                        .copy_from_slice(&((usize::MAX - 3) as u64).to_le_bytes()),
                }
                script.truncate(7);
                false
            }
            11 | 12 => {
                bytes[building..building + 32].fill(0);
                let len = if case == 11 { 0 } else { 8 };
                if len != 0 {
                    bytes[building..building + len].copy_from_slice(b"facility");
                }
                bytes[building + 16..building + 24].copy_from_slice(&(len as u64).to_le_bytes());
                bytes[building + 24..building + 32]
                    .copy_from_slice(&msvc_string::INLINE_CAPACITY.to_le_bytes());
                script[7] = (Some(pointer + nproject_cprogram::BUILDING + len), 1);
                if len == 0 {
                    script.remove(8);
                } else {
                    script[8] = (Some(pointer + nproject_cprogram::BUILDING), len);
                }
                true
            }
            13 => {
                for offset in [
                    nproject_cprogram::OWNER_PRIMARY,
                    nproject_cprogram::OWNER_COPY,
                ] {
                    bytes[kit::PROG1 + offset..kit::PROG1 + offset + 4].fill(0);
                }
                script.truncate(9);
                false
            }
            _ => {
                bytes[kit::HEAP + b"program-building-01".len()] = 7;
                true
            }
        };
        let source = ProgramSource::new(&bytes);
        let reference = ProgramSource::new(&bytes);
        let mut actual = dirty_program_log(16, 256);
        let mut expected = dirty_program_log(16, 256);
        assert_eq!(
            copy_fixture_program(&mut actual, &source),
            success,
            "case={case}"
        );
        assert!(scalar_program_script(&mut expected, &reference, &script));
        assert_program_logs_equal(&actual, &expected);
        assert_eq!(
            *source.calls.borrow(),
            *reference.calls.borrow(),
            "case={case}"
        );
        assert_eq!(
            source.scatters.borrow().len(),
            usize::from(!(1..=3).contains(&case))
        );
    }
    let bytes = kit::fixture();
    let source = ProgramSource::new(&bytes);
    let links = program_links(&bytes);
    let mut log = dirty_program_log(16, 256);
    assert!(!copy_program_row(
        &mut log,
        &source,
        usize::MAX,
        ProjectCensus {
            game_state: kit::GAME_STATE,
            links: &links,
            countries: &COUNTRIES,
            vtable: 0,
            read_to: 3,
            census: &[]
        },
        pointer,
        &mut Vec::with_capacity(1),
    ));
    assert!(source.calls.borrow().is_empty());
    assert!(source.scatters.borrow().is_empty());
    assert!(log.spans.is_empty());
}

#[test]
fn program_body_batch_preserves_duplicate_observations() {
    let bytes = projects_test_kit::fixture();
    let source = ProgramSource::new(&bytes);
    let reference = ProgramSource::new(&bytes);
    let script = program_script(&bytes, source.pointer);
    let mut actual = dirty_program_log(32, 512);
    let mut expected = dirty_program_log(32, 512);
    for _ in 0..2 {
        assert!(copy_fixture_program(&mut actual, &source));
        assert!(scalar_program_script(&mut expected, &reference, &script));
    }
    assert_program_logs_equal(&actual, &expected);
    assert_eq!(*source.calls.borrow(), *reference.calls.borrow());
    assert_eq!(source.scatters.borrow().len(), 2);
    assert_eq!(actual.spans.len(), 28);
    assert_eq!(actual.raw_bytes, [276, 0]);
}

#[test]
fn program_body_batch_keeps_plan_blocking_and_cold_retirement() {
    use super::projects_test_kit as kit;
    for failed in [false, true] {
        let bytes = kit::fixture();
        let source = ProgramSource::new(&bytes);
        let mut log = dirty_program_log(16, 256);
        log.initialize_read_plan(16, 16, 128);
        assert!(!log.begin_planned_capture(&source, [0; 4]));
        assert!(log.take(&source, source.pointer, 16, M_PROGRAMS).is_some());
        let owner = log
            .take(
                &source,
                source.pointer + nproject_cprogram::OWNER_PRIMARY,
                8,
                M_PROGRAMS,
            )
            .unwrap();
        assert_eq!(log.bytes[owner], 1);
        log.finish_planned_capture();
        log.prepare_read_plan();
        log.clear();

        let mut changed = bytes.clone();
        changed[kit::PROG1 + nproject_cprogram::OWNER_PRIMARY] = 2;
        let current = ProgramSource {
            fail: failed.then_some(1),
            ..ProgramSource::new(&changed)
        };
        assert!(!log.begin_planned_capture(&current, [0; 4]));
        assert_eq!(current.calls.borrow().len(), 2);
        assert!(!copy_fixture_program(&mut log, &current));
        assert!(log.planned_capture_failed());
        assert!(log.planned_capture_requires_no_retry());
        assert_eq!(log.fault, [true; 2]);
        assert_eq!(log.spans.len(), 1);
        assert_eq!(log.bytes.len(), 16);
        assert_eq!(current.calls.borrow().len(), 2);
        assert!(current.scatters.borrow().is_empty());
        log.finish_planned_capture();
        log.prepare_read_plan();
        log.clear();

        for _ in 0..2 {
            let fresh = ProgramSource::new(&bytes);
            assert!(!log.begin_planned_capture(&fresh, [0; 4]));
            assert!(fresh.calls.borrow().is_empty());
            assert!(copy_fixture_program(&mut log, &fresh));
            assert!(!log.planned_capture_failed());
            assert_eq!(fresh.scatters.borrow().len(), 1);
            assert_eq!(fresh.calls.borrow().len(), 14);
            log.finish_planned_capture();
            log.prepare_read_plan();
            log.clear();
        }
    }
}

#[test]
fn split_copy_matches_direct_walk() {
    use super::projects_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let capacities = split_capacities();
    let direct = kit::direct(&memory, &capacities);
    let memory = engine_native_memory::memory::CountedMemory::new(memory);
    let native = read_accounting::Capture::new();
    let (split, report, _) = kit::split(&memory, &capacities);
    let counts = native.finish();
    let mut expected_counts = read_accounting::ZERO;
    expected_counts[TABLES[0].index()] = counts[TABLES[0].index()];
    assert!(counts[TABLES[0].index()].spans > 0);
    assert!(counts[TABLES[0].index()].bytes > 0);
    assert_eq!(counts, expected_counts);
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(split.programs.outcome, Outcome::Rows { len: 1 });
    let program = &split.programs.rows[0];
    assert_eq!((program.id, program.owner, program.province), (5, 1, 13413));
    assert_eq!(program.active_project, Some(9));
    assert_eq!(
        &split.names.first[program.building_start..program.building_start + program.building_len],
        b"program-building-01"
    );
    assert_eq!(direct.projects.outcome, Outcome::Rows { len: 1 });
    assert!(matches!(
        split.projects.outcome,
        Outcome::Unavailable { .. }
    ));
    assert!(matches!(split.history.outcome, Outcome::Unavailable { .. }));
    assert_eq!(split.breakthroughs.outcome, Outcome::Rows { len: 1 });
    let breakthrough = &split.breakthroughs.rows[0];
    assert_eq!(
        (
            breakthrough.owner,
            breakthrough.specialization,
            breakthrough.points
        ),
        (1, 77, -8)
    );
}

#[test]
fn a_reserved_country_alias_keeps_the_split_census() {
    use super::projects_test_kit as kit;
    let mut bytes = kit::fixture();
    kit::add_reserved_alias(&mut bytes);
    let memory = kit::memory(&bytes);
    let capacities = split_capacities();
    let direct = kit::direct(&memory, &capacities);
    assert_eq!(direct.programs.outcome, Outcome::Rows { len: 1 });
    let (split, report, _) = kit::split(&memory, &capacities);
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(split.breakthroughs.outcome, Outcome::Rows { len: 1 });
}

#[test]
fn projects_truncation_matrix_matches_direct_with_same_caps() {
    use super::projects_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    for rows in [[0; N], [1; N], [256; N]] {
        let capacities = Capacities {
            rows,
            program_names: 8192,
            program_candidates: 1024,
            raid_type_tokens: 256,
            raid_show_for: 256,
            raid_transports: 16,
            raid_equipment_items: 16,
            raid_equipment_archetypes: 16,
            counteroperatives: 16,
        };
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert!(report.raw_ok, "rows={rows:?}");
        kit::assert_matches_direct(&split, &direct);
    }
    for (names, candidates) in [(0, 0), (4, 1), (8192, 1024)] {
        let capacities = Capacities {
            rows: [256; N],
            program_names: names,
            program_candidates: candidates,
            raid_type_tokens: 256,
            raid_show_for: 256,
            raid_transports: 16,
            raid_equipment_items: 16,
            raid_equipment_archetypes: 16,
            counteroperatives: 16,
        };
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert!(report.raw_ok);
        kit::assert_matches_direct(&split, &direct);
    }
    let capacities = Capacities {
        rows: [1; N],
        program_names: 8192,
        program_candidates: 1024,
        raid_type_tokens: 256,
        raid_show_for: 256,
        raid_transports: 16,
        raid_equipment_items: 16,
        raid_equipment_archetypes: 16,
        counteroperatives: 16,
    };
    let (split, _, _) = kit::split(&memory, &capacities);
    assert_eq!(split.programs.outcome, Outcome::Rows { len: 1 });
    assert_eq!(split.breakthroughs.outcome, Outcome::Rows { len: 1 });
    let bytes = kit::many_programs_fixture();
    let memory = kit::memory(&bytes);
    for cap in [0, 1, kit::PROGRAMS - 1, kit::PROGRAMS, kit::PROGRAMS + 1, 256] {
        let capacities = Capacities {
            rows: [cap; N],
            program_names: 8192,
            program_candidates: 1024,
            raid_type_tokens: 256,
            raid_show_for: 256,
            raid_transports: 16,
            raid_equipment_items: 16,
            raid_equipment_archetypes: 16,
            counteroperatives: 16,
        };
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert!(report.raw_ok, "cap={cap}");
        kit::assert_matches_direct(&split, &direct);
        let expected = Outcome::Rows { len: kit::PROGRAMS };
        assert_eq!(split.programs.outcome, expected, "every program is kept from an initial capacity of {cap}");
        let ids: Vec<u32> = split.programs.rows[..expected.len()].iter().map(|row| row.id).collect();
        assert_eq!(ids, [5, 6, 7], "programs are ordered by id, not by address");
    }
    let mut one = [nproject_cprogram::Row::default(); 1];
    let walked = nproject_cprogram::walk_buffer(
        &memory,
        kit::BASE,
        kit::GAME_STATE,
        engine_native_memory::walk::Buffer::Growing(&mut Vec::new()),
        engine_native_memory::walk::Buffer::Exact(&mut one),
        engine_native_memory::walk::Buffer::Growing(&mut Vec::new()),
    );
    assert_eq!(walked.outcome, Outcome::Truncated { len: 1, unread_rows: 2 }, "a one-row output keeps the first program and counts the two it could not hold");
    assert_eq!(one[0].id, kit::program_id(0));
}

#[test]
fn program_inline_building_name_is_copied_for_the_arena_read() {
    use super::projects_test_kit as kit;
    let mut bytes = kit::fixture();
    let at = kit::PROG1 + nproject_cprogram::BUILDING;
    bytes[at..at + 32].fill(0);
    bytes[at..at + 8].copy_from_slice(b"facility");
    bytes[at + 16..at + 24].copy_from_slice(&8_u64.to_le_bytes());
    bytes[at + 24..at + 32].copy_from_slice(&msvc_string::INLINE_CAPACITY.to_le_bytes());
    let memory = kit::memory(&bytes);
    let capacities = split_capacities();
    let direct = kit::direct(&memory, &capacities);
    let (split, report, _) = kit::split(&memory, &capacities);
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(split.programs.outcome, Outcome::Rows { len: 1 });
    assert_eq!(&split.names.first[..8], b"facility");
}

#[test]
fn split_copy_matches_direct_walk_on_fault_matrix() {
    use super::projects_test_kit as kit;
    use engine_native_memory::memory::SliceMemory;
    struct Fault<'a> {
        memory: SliceMemory<'a>,
        fault: std::ops::Range<usize>,
    }
    impl engine_native_memory::Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let Some(end) = address.checked_add(output.len()) else {
                return false;
            };
            if address < self.fault.end && end > self.fault.start {
                return false;
            }
            self.memory.copy(address, output)
        }
    }
    for (offset, fault_mask) in [
        (cgame_state::COUNTRIES, [true, true]),
        (kit::C1, [true, true]),
        (kit::S1, [true, true]),
        (kit::PROG1, [true, false]),
        (kit::HEAP, [true, false]),
        (kit::PROJ1, [false, false]),
        (kit::MACH1, [false, false]),
        (kit::HIST1, [false, false]),
        (
            kit::HIST1 + nproject_sproject_history::CONTEXT,
            [false, false],
        ),
        (kit::BT_NODE, [false, true]),
    ] {
        let bytes = kit::fixture();
        let fault = kit::GAME_STATE + offset;
        let memory = Fault {
            memory: SliceMemory {
                base: kit::GAME_STATE,
                bytes: &bytes,
            },
            fault: fault..fault + 1,
        };
        let capacities = split_capacities();
        let direct = kit::direct(&memory, &capacities);
        let (split, report, job) = kit::split(&memory, &capacities);
        assert!(report.raw_ok, "offset={offset:x}");
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(job.log.fault, fault_mask, "offset={offset:x}");
    }
}

fn project_hour(
    captured: &mut Captured,
    bytes: &[u8],
    hour: i32,
    store: &mut SliceFingerprints,
    changes: &crate::country_input_changes::Domain,
) -> Vec<u32> {
    use super::projects_test_kit as kit;
    captured.capture_selected(
        &kit::memory(bytes),
        &kit::memory(bytes),
        kit::BASE,
        kit::GAME_STATE,
        hour,
        false,
        store,
        changes,
    );
    assert!(
        matches!(captured.projects.outcome, Outcome::Rows { .. }),
        "hour {hour}"
    );
    assert!(
        matches!(captured.history.outcome, Outcome::Rows { .. }),
        "hour {hour}"
    );
    let mut owners: Vec<u32> = captured.project_read.iter().map(|&(_, raw)| raw).collect();
    owners.sort_unstable();
    owners
}

#[test]
fn projects_are_read_after_the_daily_tick_and_within_the_hour_of_a_marked_write() {
    use super::projects_test_kit as kit;
    use crate::country_input_changes::{Domain, Table};
    let mut bytes = kit::fixture();
    let changes = Domain::enabled(64);
    let mut captured = Captured::with_capacities(&split_capacities());
    let mut store = SliceFingerprints::new();
    assert_eq!(
        project_hour(&mut captured, &bytes, 5, &mut store, &changes),
        [1, 2]
    );
    assert_eq!(captured.projects.outcome, Outcome::Rows { len: 1 });
    assert!(project_hour(&mut captured, &bytes, 6, &mut store, &changes).is_empty());
    bytes[kit::PROJ1 + nproject_cproject::COMPLETED] = 1;
    changes.write(Table::Projects, 1);
    assert_eq!(
        project_hour(&mut captured, &bytes, 7, &mut store, &changes),
        [1]
    );
    assert_eq!(captured.projects.rows[0].completed, 1);
    assert!(project_hour(&mut captured, &bytes, 8, &mut store, &changes).is_empty());
    let links = cgame_state::COUNTRY_LINKS;
    bytes[links + 8..links + 12].copy_from_slice(&4_u32.to_le_bytes());
    bytes[links + 12..links + 16].copy_from_slice(&4_u32.to_le_bytes());
    let link =
        u64::from_le_bytes(bytes[links..links + 8].try_into().unwrap()) as usize - kit::GAME_STATE;
    bytes[link + 12..link + 16].copy_from_slice(&1_u32.to_le_bytes());
    bytes[kit::PROJ1 + nproject_cproject::OWNER..][..4].copy_from_slice(&3_u32.to_le_bytes());
    changes.write(Table::Projects, 1);
    assert_eq!(
        project_hour(&mut captured, &bytes, 9, &mut store, &changes),
        [1, 3]
    );
    assert_eq!(captured.projects.rows[0].owner, 3);
    assert_eq!(captured.history.rows[0].owner, 3);
    assert!(project_hour(&mut captured, &bytes, 10, &mut store, &changes).is_empty());
    bytes[kit::PROJ1 + nproject_cproject::STARTED] = 0;
    changes.write(Table::Projects, 1);
    assert_eq!(
        project_hour(&mut captured, &bytes, 11, &mut store, &changes),
        [1, 3]
    );
    assert_eq!(captured.projects.rows[0].started, 0);
    let date = kit::HIST1 + nproject_sproject_history::DATE;
    bytes[date..date + 4].copy_from_slice(&444_i32.to_le_bytes());
    assert_eq!(
        project_hour(&mut captured, &bytes, 24, &mut store, &changes),
        [1, 2, 3]
    );
    assert_eq!(captured.history.rows[0].date, 444);
    bytes[kit::PROJ1 + nproject_cproject::COMPLETED] = 0;
    assert!(project_hour(&mut captured, &bytes, 25, &mut store, &changes).is_empty());
    assert_eq!(
        project_hour(&mut captured, &bytes, 48, &mut store, &changes),
        [1, 2, 3]
    );
    assert_eq!(captured.projects.rows[0].completed, 0);
    assert!(project_hour(&mut captured, &bytes, 49, &mut store, &changes).is_empty());
}

#[test]
fn retained_scientist_origin_uses_its_daily_arena_and_is_not_published_as_an_hourly_read() {
    let (_, mut captured) = small();
    captured
        .scientist_names
        .extend_from_slice(b"events/replacement.txt");
    captured.scientists.rows[0] = scientist_roster::Row {
        character: 7,
        country: 5000,
        template_visibility: Some(1),
        template_origin: Some((0, 22, 17)),
        ..Default::default()
    };
    captured.scientists.outcome = Outcome::Rows { len: 1 };
    captured.scientist_injury_read = true;
    let tables = captured.tables();
    let TableView::Complete(rows::TableRows::ScientistDaily(scientists)) = &tables[19] else {
        panic!("roster unavailable")
    };
    assert_eq!(
        scientists[0].template_origin,
        rows::Recorded::Value(Some(rows::ScientistTemplateOrigin {
            file: b"events/replacement.txt".to_vec(),
            line: 17
        }))
    );
    rows::TableRows::ScientistDaily(scientists.clone())
        .encode_rows(&mut Vec::new())
        .unwrap();
    captured.scientist_injury_read = false;
    let tables = captured.tables();
    let TableView::Complete(rows::TableRows::Scientists(scientists)) = &tables[8] else {
        panic!("roster unavailable")
    };
    assert!(
        matches!(&tables[19], TableView::Unavailable { reason } if reason == replay_data::cadence::SKIPPED_REASON)
    );
    rows::TableRows::Scientists(scientists.clone())
        .encode_rows(&mut Vec::new())
        .unwrap();
}

#[test]
fn failed_scientist_daily_acquisition_keeps_its_own_read_boundary() {
    for (hour, force_full) in [(1, false), (4, false), (1, true)] {
        for previous_read in [false, true] {
            let mut captured = Captured::vacant();
            captured.scientist_injury_read = previous_read;
            captured.fail_gated_capture(hour, force_full, "fixture worker failure");
            assert_eq!(
                captured.outcomes()[19],
                super::super::failed_capture(
                    TableId::ScientistDaily,
                    hour,
                    force_full,
                    "fixture worker failure"
                )
            );
            assert_eq!(
                captured.outcomes()[8],
                Outcome::Unavailable {
                    reason: "fixture worker failure"
                }
            );
        }
    }
}

mod raid_memos {
    use super::*;
    use engine_native_memory::memory::SliceMemory;
    use replay_data::rows::RaidPathDispatcher;

    const GAME: usize = 0x100;
    const SYSTEM: usize = 0x800;
    const BUCKETS: usize = 0x08;
    const COUNT: usize = 0x10;
    const MASK: usize = 0x14;
    const MAX_PROBE: usize = 0x18;
    const LOCK: usize = 0x20;
    const BUCKET: usize = 0x20;
    const PROBE: usize = 0x04;
    const SOURCE: usize = 0x08;
    const TARGET: usize = 0x0c;
    const DISTANCE: usize = 0x10;
    const OK: usize = 0x18;
    const TABLES: [usize; 2] = [0x1000, 0x1400];
    const MEMOS: [usize; 2] = [raid_path_memo::LAND_MEMO, raid_path_memo::NAVAL_MEMO];

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x2000];
        let mut put = |at: usize, value: &[u8]| bytes[at..at + value.len()].copy_from_slice(value);
        put(
            GAME + cgame_state::RAID_SYSTEM,
            &(SYSTEM as u64).to_le_bytes(),
        );
        put(SYSTEM, &(craid_system::VTABLE_RVA.0 as u64).to_le_bytes());
        for ((memo, table), distance) in MEMOS.into_iter().zip(TABLES).zip([900_000i64, 500_000]) {
            put(SYSTEM + memo + BUCKETS, &(table as u64).to_le_bytes());
            put(SYSTEM + memo + COUNT, &1u32.to_le_bytes());
            put(SYSTEM + memo + MASK, &3u32.to_le_bytes());
            put(SYSTEM + memo + MAX_PROBE, &[1]);
            put(table + PROBE, &[1]);
            put(table + SOURCE, &7u32.to_le_bytes());
            put(table + TARGET, &9u32.to_le_bytes());
            put(table + DISTANCE, &distance.to_le_bytes());
            put(table + OK, &[1]);
            put(table + 5 * BUCKET + PROBE, &[u8::MAX]);
        }
        bytes
    }

    fn capture(captured: &mut Captured, bytes: &[u8]) {
        captured.capture(&SliceMemory { base: 0, bytes }, 0, GAME, 80, true);
    }

    #[test]
    fn both_memos_keep_the_same_pair_distinct_through_capture_and_codec() {
        let mut captured = Captured::vacant();
        capture(&mut captured, &fixture());
        assert_eq!(captured.outcomes()[12], Outcome::Rows { len: 2 });
        assert_eq!(captured.raid_reads().torn_memo_reads, 0);
        let TableView::Complete(rows::TableRows::RaidPathDistances(rows)) = &captured.tables()[12]
        else {
            panic!("both memos must be complete");
        };
        assert_eq!(
            rows.iter()
                .map(|row| (row.dispatcher, row.source, row.target, row.distance))
                .collect::<Vec<_>>(),
            [
                (RaidPathDispatcher::Land, 7, 9, 900_000),
                (RaidPathDispatcher::Naval, 7, 9, 500_000),
            ]
        );
        assert!(rows[0].key() < rows[1].key());
        let table = rows::TableRows::RaidPathDistances(rows.clone());
        let mut bytes = Vec::new();
        table.encode_rows(&mut bytes).unwrap();
        let mut decoder = replay_data::codec::Decoder::new(&bytes);
        assert_eq!(
            TableId::RaidPathDistances
                .decode_rows(2, &mut decoder)
                .unwrap(),
            table
        );
        decoder.finish().unwrap();
        let mut capacities = small().0;
        captured.grow(&mut capacities);
        assert!(
            captured
                .raid_paths
                .iter()
                .all(|memo| memo.rows.len() >= capacities.rows[12])
        );
    }

    #[test]
    fn either_locked_or_unreadable_memo_refuses_the_table_without_hiding_the_other_read() {
        for bad in 0..2 {
            for locked in [true, false] {
                let mut captured = Captured::vacant();
                let mut bytes = fixture();
                capture(&mut captured, &bytes);
                if locked {
                    bytes[SYSTEM + MEMOS[bad] + LOCK] = 1;
                } else {
                    bytes[SYSTEM + MEMOS[bad] + BUCKETS..][..8].fill(0);
                }
                captured.reset();
                capture(&mut captured, &bytes);
                assert!(matches!(
                    captured.outcomes()[12],
                    Outcome::Unavailable { .. }
                ));
                assert!(matches!(
                    &captured.tables()[12],
                    TableView::Unavailable { .. }
                ));
                assert!(matches!(
                    captured.raid_paths[bad].outcome,
                    Outcome::Unavailable { .. }
                ));
                assert_eq!(
                    captured.raid_paths[1 - bad].outcome,
                    Outcome::Rows { len: 1 }
                );
                assert_eq!(captured.raid_reads().torn_memo_reads, u32::from(locked));
            }
        }
    }

    #[test]
    fn memo_outcomes_keep_truncation_and_failure_separate() {
        let (_, mut captured) = small();
        captured.raid_paths[0].outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 2,
        };
        captured.raid_paths[1].outcome = Outcome::Truncated {
            len: 2,
            unread_rows: 3,
        };
        assert_eq!(
            captured.outcomes()[12],
            Outcome::Truncated {
                len: 3,
                unread_rows: 5
            }
        );
        captured.raid_paths[1].outcome = Outcome::Unavailable {
            reason: "naval memo unreadable",
        };
        assert_eq!(captured.outcomes()[12], captured.raid_paths[1].outcome);
    }
}

#[test]
fn the_rest_walked_beside_the_projects_equals_the_inline_walk() {
    use super::projects_test_kit as kit;
    use crate::country_input_changes::Domain;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let changes = Domain::enabled(64);
    let mut inline = Captured::with_capacities(&split_capacities());
    let mut beside = Captured::with_capacities(&split_capacities());
    let (mut inline_store, mut beside_store) = (SliceFingerprints::new(), SliceFingerprints::new());
    for (hour, force_full) in [(0, true), (5, false), (24, false)] {
        inline.capture_selected(
            &memory,
            &memory,
            kit::BASE,
            kit::GAME_STATE,
            hour,
            force_full,
            &mut inline_store,
            &changes,
        );
        beside.reset();
        let mut rest = beside.take_rest(kit::GAME_STATE, hour, force_full);
        beside.capture_selected(
            &memory,
            &memory,
            kit::BASE,
            kit::GAME_STATE,
            hour,
            force_full,
            &mut beside_store,
            &changes,
        );
        rest.capture(&memory, kit::BASE);
        beside.restore_rest(rest);
        assert_eq!(beside.outcomes(), inline.outcomes(), "hour {hour}");
        assert_eq!(beside.tables(), inline.tables(), "hour {hour}");
    }
}
