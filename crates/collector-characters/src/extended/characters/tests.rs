use super::*;
use engine_native::{cgame_state, cunit_leader_trait};
#[cfg(test)]
use std::cell::Cell;
#[cfg(test)]
use engine_native::memory::SliceMemory;

#[cfg(test)]
pub(super) fn fixture_capacities() -> Capacities {
    Capacities {
        rows: [16_384, 4096, 4096, 2048, 4096, 0],
        bytes: [524_288, 0, 262_144, 131_072, 131_072, 0],
    }
}

pub(super) fn fixture_bindings() -> CharacterSourceBindings {
    let mut value = CharacterSourceBindings::new();
    value.sources = Vec::with_capacity(8192);
    value.update_charge();
    value
}

#[test]
fn tooltip_key_capacity_tracks_keys_instead_of_payload_bytes() {
    let _globals = crate::extended::characters::test_globals();
    let mut captured = Captured::with_capacities(&fixture_capacities());
    let key = vec![b'x'; 40_000];
    let len = key.len() + 16;
    captured.advisors.bytes.first[..8].copy_from_slice(&(key.len() as u64).to_le_bytes());
    captured.advisors.bytes.first[8..8 + key.len()].copy_from_slice(&key);
    captured.advisors.bytes.first[8 + key.len()..len].copy_from_slice(&0_u64.to_le_bytes());
    captured.advisors.bytes.len = len;
    captured.advisors.rows.rows[0] = cadvisor::Row {
        character: 7,
        tooltip_len: len,
        tooltip_recorded: true,
        ..Default::default()
    };
    captured.advisors.rows.outcome = Outcome::Rows { len: 1 };
    let TableView::Complete(replay_data::rows::TableRows::AdvisorRoles(rows)) =
        captured.advisors_table()
    else {
        panic!("advisor keys lost");
    };
    let replay_data::rows::Recorded::Value(Some(keys)) = &rows[0].tooltip_keys else {
        panic!("advisor tooltip keys refused");
    };
    assert_eq!(keys, &[key, Vec::new()]);
    assert_eq!(keys.capacity(), 2);
}

#[cfg(test)]
pub(super) const SECOND_ADVISOR: usize = 0x8c60;

#[cfg(test)]
pub(super) fn two_advisor_fixture(same_character: bool) -> Vec<u8> {
    use engine_native::{character_advisor_node as node, msvc_tree_node as tree};
    let mut bytes = characters_test_kit::mapped_fixture();
    let first_node = characters_test_kit::ADV_NODE;
    let second_node = 0x8c00;
    let character = CHARACTERS[usize::from(!same_character)];
    let head = if same_character { 0x2430 } else { 0x2490 };
    put(
        &mut bytes,
        character + ccharacter::ADVISOR_MAP_HEAD + 8,
        (if same_character { 2_u64 } else { 1_u64 }).to_le_bytes(),
    );
    if same_character {
        pointer(&mut bytes, first_node + tree::RIGHT, second_node);
        pointer(&mut bytes, second_node + tree::PARENT, first_node);
    } else {
        pointer(&mut bytes, head + tree::LEFT, second_node);
        pointer(&mut bytes, second_node + tree::PARENT, head);
    }
    pointer(&mut bytes, second_node + tree::LEFT, head);
    pointer(&mut bytes, second_node + tree::RIGHT, head);
    pointer(&mut bytes, second_node + node::ADVISOR, SECOND_ADVISOR);
    put(
        &mut bytes,
        second_node + node::KEY_HASH,
        2345_u32.to_le_bytes(),
    );
    literal(&mut bytes, first_node + node::KEY, b"a");
    literal(&mut bytes, second_node + node::KEY, b"b");
    put(
        &mut bytes,
        SECOND_ADVISOR,
        ((IMAGE + cadvisor::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    pointer(&mut bytes, SECOND_ADVISOR + cadvisor::CHARACTER, character);
    for (role, data, value, key) in [
        (characters_test_kit::ADV_ROLE, 0x8f00, -17_i64, &b""[..]),
        (SECOND_ADVISOR, 0x8f40, 19_i64, &b"z"[..]),
    ] {
        literal(&mut bytes, role + cadvisor::IDEA_TOKEN, b"i");
        literal(&mut bytes, role + cadvisor::SLOT, b"s");
        array(&mut bytes, role + cadvisor::MODIFIER_ENTRIES, data, 1);
        put(&mut bytes, data, 1_u32.to_le_bytes());
        put(&mut bytes, data + 8, value.to_le_bytes());
        let table = role + cadvisor::TOOLTIP_KEYS + 8;
        pointer(&mut bytes, table, data + 16);
        put(&mut bytes, table + 8, 1_u32.to_le_bytes());
        bytes[data + 20] = 1;
        literal(&mut bytes, data + 24, key);
    }
    bytes
}

#[test]
fn advisor_short_arena_grows_to_every_role_name_numeric_cache_and_tooltip() {
    let _globals = crate::extended::characters::test_globals();
    use super::characters_test_kit as kit;
    for same_character in [true, false] {
        let bytes = two_advisor_fixture(same_character);
        let memory = kit::memory(&bytes);
        let whole = kit::direct(
            &memory,
            &Capacities {
                rows: [64; N],
                bytes: [8192; N],
            },
        );
        for capacity in [1, 38, 46, 55] {
            let direct = kit::direct(
                &memory,
                &Capacities {
                    rows: [1; N],
                    bytes: [capacity; N],
                },
            );
            let advisors = &direct.advisors;
            assert_eq!(advisors.rows.outcome, Outcome::Rows { len: 2 });
            let arena = &advisors.bytes.first;
            for (index, row) in advisors.rows.rows[..2].iter().enumerate() {
                assert_eq!(
                    &arena[row.key_start..row.key_start + row.key_len],
                    if index == 0 { b"a" } else { b"b" }
                );
                assert!(row.modifier_recorded);
                assert_eq!(
                    &arena[row.modifier_start + 8..row.modifier_start + 16],
                    &if index == 0 { -17_i64 } else { 19_i64 }.to_le_bytes()
                );
                assert!(row.tooltip_recorded);
            }
            let last = &advisors.rows.rows[1];
            assert_eq!(
                &arena[last.tooltip_start..last.tooltip_start + 9],
                &[1, 0, 0, 0, 0, 0, 0, 0, b'z']
            );
            assert_eq!(advisors.rows.outcome, whole.advisors.rows.outcome);
            assert_eq!(&advisors.rows.rows[..2], &whole.advisors.rows.rows[..2]);
            assert_eq!(
                &arena[..advisors.bytes.len],
                &whole.advisors.bytes.first[..whole.advisors.bytes.len]
            );
        }
        let capacities = Capacities {
            rows: [64; N],
            bytes: [8192; N],
        };
        let (split, _, _) = kit::split(&kit::memory(&bytes), &capacities);
        let direct = kit::direct(&kit::memory(&bytes), &capacities);
        kit::assert_matches_direct(&split, &direct);
    }
}

#[test]
fn a_changed_advisor_suffix_grows_in_the_pass_without_repeating_its_prefix() {
    let _globals = crate::extended::characters::test_globals();
    use super::characters_test_kit as kit;
    let bytes = two_advisor_fixture(false);
    let context = CharactersCopyContext {
        force_full: false,
        base: IMAGE,
        game_state: ADDRESS,
        active: PART_MASKS[2],
    };
    let mut baseline = CharactersJob::with_storage(2, 2048, 32768, false);
    assert!(copy_characters(&kit::memory(&bytes), context, &mut baseline, Roles::Independent).raw_ok);
    let prefix_spans = baseline
        .log
        .spans
        .iter()
        .position(|span| span.addr == ADDRESS + kit::ADV_ROLE + cadvisor::TOOLTIP_KEYS + 8)
        .unwrap();
    let mut job = CharactersJob::with_storage(2, 2048, baseline.log.bytes.len() + 16, false);
    for warm in 0..3 {
        assert!(copy_characters(&kit::memory(&bytes), context, &mut job, Roles::Independent).raw_ok);
        if warm == 2 {
            assert!(job.log.planned_capture_succeeded());
        }
        job.prepare_read_plans();
        job.clear();
    }
    let mut changed = bytes.clone();
    changed.resize(0x9801, 0);
    let header = 0x8f18;
    pointer(&mut changed, header, 0x9000);
    put(&mut changed, header + 16, 2048_u64.to_le_bytes());
    put(&mut changed, header + 24, 2048_u64.to_le_bytes());
    changed[0x9000..0x9800].fill(b'x');
    let source = WaveMemory::new(&changed, None);
    let report = copy_characters(&source, context, &mut job, Roles::Independent);
    assert!(report.raw_ok && !report.plan_failed);
    assert!(!job.log.overflowed && !job.appended_overflowed && !job.grow_for_next());
    for span in &baseline.log.spans[..prefix_spans] {
        assert_eq!(
            source
                .reads
                .borrow()
                .iter()
                .filter(|&&(address, len)| address == span.addr && len == span.len as usize)
                .count(),
            1
        );
    }
    let mut recovered = Captured::with_capacities(&fixture_capacities());
    recovered.decode_characters_payload(IMAGE, &job, &mut [0; N]);
    let direct = direct_advisors(&kit::memory(&changed));
    assert_eq!(recovered.advisors_table(), direct.advisors_table());
    assert!(
        recovered.advisors.rows.rows[..2]
            .iter()
            .all(|row| row.modifier_recorded && row.tooltip_recorded)
    );
}

#[cfg(test)]
fn section_job() -> CharactersJob {
    let mut job = CharactersJob::with_storage(2, 2048, 32768, false);
    job.advisor_roots = collector_raw::raw::RawLog::new(4, 48);
    job.advisor_dynamic = Some(Box::new(CharactersJob::with_storage(2, 2048, 32768, false)));
    job
}

#[cfg(test)]
fn section_context() -> CharactersCopyContext {
    CharactersCopyContext {
        force_full: false,
        base: IMAGE,
        game_state: ADDRESS,
        active: PART_MASKS[2],
    }
}

#[cfg(test)]
pub(super) fn section_fixture() -> Vec<u8> {
    let mut bytes = characters_test_kit::mapped_fixture();
    array(
        &mut bytes,
        MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
        ROSTER,
        1,
    );
    array(
        &mut bytes,
        MANAGER + ccharacter_manager::DYNAMIC_CHARACTERS,
        0x8800,
        1,
    );
    pointer(&mut bytes, 0x8800, CHARACTERS[1]);
    bytes
}

#[cfg(test)]
fn direct_advisors(memory: &impl Memory) -> Captured {
    let mut captured = Captured::with_capacities(&split_capacities());
    let result = walk_family(
        memory,
        IMAGE,
        ADDRESS,
        captured.family().select(PART_MASKS[2]),
        None,
        None,
    );
    captured.advisors.primary(result[2]);
    captured
}

#[cfg(test)]
fn assert_advisors_match(job: &CharactersJob, memory: &impl Memory) {
    let mut captured = Captured::with_capacities(&split_capacities());
    captured.decode_characters_payload(IMAGE, job, &mut [0; N]);
    let direct = direct_advisors(memory);
    assert_eq!(captured.advisors.rows.outcome, direct.advisors.rows.outcome);
    let len = direct.advisors.rows.outcome.len();
    assert_eq!(
        captured.advisors.rows.rows[..len],
        direct.advisors.rows.rows[..len]
    );
    assert_eq!(
        captured.advisors.bytes.first[..captured.advisors.bytes.len],
        direct.advisors.bytes.first[..direct.advisors.bytes.len]
    );
    assert_eq!(
        captured.advisors.bytes.required,
        direct.advisors.bytes.required
    );
}

#[test]
fn advisor_raw_copy_keeps_fresh_ordered_modifier_entries_and_owned_bytes() {
    let _globals = crate::extended::characters::test_globals();
    let mut bytes = section_fixture();
    let data = 0x8a00;
    array(
        &mut bytes,
        characters_test_kit::ADV_ROLE + cadvisor::MODIFIER_ENTRIES,
        data,
        65,
    );
    bytes.resize(0x9000 + 65 * engine_native::modifier_entry::STRIDE, 0);
    for index in 0..65 {
        put(&mut bytes, data + index * 16, (index as u32).to_le_bytes());
        put(
            &mut bytes,
            data + index * 16 + 8,
            (-(index as i64)).to_le_bytes(),
        );
    }
    let mut job = section_job();
    let mut expected = direct_advisors(&characters_test_kit::memory(&bytes));
    for round in 0..2 {
        if round != 0 {
            put(&mut bytes, data + 8, i64::MIN.to_le_bytes());
            expected = direct_advisors(&characters_test_kit::memory(&bytes));
        }
        let report = copy_characters(
            &characters_test_kit::memory(&bytes),
            section_context(),
            &mut job,
            Roles::Independent,
        );
        assert!(report.raw_ok);
        assert_advisors_match(&job, &characters_test_kit::memory(&bytes));
        if round == 0 {
            job.prepare_read_plans();
        }
    }
    bytes.fill(0);
    let mut captured = Captured::with_capacities(&split_capacities());
    captured.decode_characters_payload(IMAGE, &job, &mut [0; N]);
    assert_eq!(captured.advisors_table(), expected.advisors_table());
    let TableView::Complete(TableRows::AdvisorRoles(rows)) = captured.advisors_table() else {
        panic!("complete advisor roles");
    };
    let entries = &rows[0]
        .modifier_cache
        .require("AdvisorRoles.modifier_cache")
        .unwrap()
        .as_ref()
        .unwrap()
        .entries;
    assert_eq!(entries.len(), 65);
    assert_eq!(entries[0].value, i64::MIN);
    assert_eq!(entries[64].definition, 64);
    assert_eq!(entries[64].value, -64);
}

#[test]
fn advisor_projection_refuses_partial_modifier_entries() {
    let _globals = crate::extended::characters::test_globals();
    let mut captured = Captured::with_capacities(&small_capacities());
    captured.advisors.rows.rows[0] = cadvisor::Row {
        character: 1,
        modifier_len: engine_native::modifier_entry::STRIDE,
        modifier_recorded: true,
        ..Default::default()
    };
    captured.advisors.rows.outcome = Outcome::Rows { len: 1 };
    captured.advisors.bytes.len = engine_native::modifier_entry::STRIDE;
    captured.advisors.bytes.first[..4].copy_from_slice(&77_u32.to_le_bytes());
    captured.advisors.bytes.first[8..16].copy_from_slice(&i64::MIN.to_le_bytes());
    let TableView::Complete(TableRows::AdvisorRoles(rows)) = captured.advisors_table() else {
        panic!("complete advisor roles");
    };
    assert_eq!(
        rows[0]
            .modifier_cache
            .require("AdvisorRoles.modifier_cache")
            .unwrap()
            .as_ref()
            .unwrap()
            .entries[0]
            .value,
        i64::MIN
    );
    captured.advisors.rows.rows[0].modifier_len -= 1;
    assert!(matches!(
        captured.advisors_table(),
        TableView::Unavailable { .. }
    ));
}

#[test]
fn advisor_roster_growth_rebuilds_only_its_section_and_reads_fresh_inactive_roles() {
    let _globals = crate::extended::characters::test_globals();
    let mut bytes = section_fixture();
    let mut job = section_job();
    for _ in 0..2 {
        assert!(
            copy_characters(
                &characters_test_kit::memory(&bytes),
                section_context(),
                &mut job,
                Roles::Independent,
            )
            .raw_ok
        );
        assert_advisors_match(&job, &characters_test_kit::memory(&bytes));
        job.prepare_read_plans();
        job.clear();
    }
    put(
        &mut bytes,
        CHARACTERS[0] + ccharacter::COUNTRY,
        0_u32.to_le_bytes(),
    );
    bytes[characters_test_kit::HEAP_AREA] = b'X';
    array(
        &mut bytes,
        MANAGER + ccharacter_manager::DYNAMIC_CHARACTERS,
        0x8800,
        2,
    );
    pointer(&mut bytes, 0x8808, 0x2300);
    assert!(
        copy_characters(
            &characters_test_kit::memory(&bytes),
            section_context(),
            &mut job,
            Roles::Independent,
        )
        .raw_ok
    );
    assert!(job.log.planned_capture_succeeded());
    assert!(
        !job.advisor_dynamic
            .as_ref()
            .unwrap()
            .log
            .planned_capture_succeeded()
    );
    assert_advisors_match(&job, &characters_test_kit::memory(&bytes));
    job.prepare_read_plans();
    job.clear();
    literal(
        &mut bytes,
        characters_test_kit::ADV_ROLE + cadvisor::IDEA_TOKEN,
        b"changed",
    );
    assert!(
        copy_characters(
            &characters_test_kit::memory(&bytes),
            section_context(),
            &mut job,
            Roles::Independent,
        )
        .raw_ok
    );
    assert!(!job.log.planned_capture_succeeded());
    assert!(
        job.advisor_dynamic
            .as_ref()
            .unwrap()
            .log
            .planned_capture_succeeded()
    );
    assert_advisors_match(&job, &characters_test_kit::memory(&bytes));
    job.prepare_read_plans();
    job.clear();
    for (data, count) in [(0x8900, 1), (0x8900, 0), (0x8800, 2)] {
        array(
            &mut bytes,
            MANAGER + ccharacter_manager::DYNAMIC_CHARACTERS,
            data,
            count,
        );
        pointer(&mut bytes, data, CHARACTERS[1]);
        assert!(
            copy_characters(
                &characters_test_kit::memory(&bytes),
                section_context(),
                &mut job,
                Roles::Independent,
            )
            .raw_ok
        );
        assert!(job.log.planned_capture_succeeded());
        assert_advisors_match(&job, &characters_test_kit::memory(&bytes));
        job.prepare_read_plans();
        job.clear();
    }
}

#[test]
fn advisor_sections_preserve_alias_occurrences_and_first_fault_without_live_retry() {
    let _globals = crate::extended::characters::test_globals();
    let mut bytes = section_fixture();
    pointer(&mut bytes, 0x8800, CHARACTERS[0]);
    let trace = WaveMemory::new(&bytes, None);
    let direct = direct_advisors(&trace);
    assert_eq!(direct.advisors.rows.outcome, Outcome::Rows { len: 2 });
    let mut faults = std::collections::BTreeSet::new();
    for &(address, width) in trace.reads.borrow().iter() {
        if width != 0 {
            faults.extend([address, address + width - 1]);
        }
    }
    for fault in faults.into_iter().map(Some).chain([None]) {
        for warm in [false, true] {
            let mut job = section_job();
            if warm {
                assert!(
                    copy_characters(
                        &characters_test_kit::memory(&bytes),
                        section_context(),
                        &mut job,
                        Roles::Independent,
                    )
                    .raw_ok
                );
                job.prepare_read_plans();
                job.clear();
            }
            let source = WaveMemory::new(&bytes, fault);
            assert!(copy_characters(&source, section_context(), &mut job, Roles::Independent).raw_ok);
            assert_advisors_match(&job, &WaveMemory::new(&bytes, fault));
            if let Some(fault) = fault {
                let reference = WaveMemory::new(&bytes, Some(fault));
                direct_advisors(&reference);
                assert_eq!(
                    source
                        .reads
                        .borrow()
                        .iter()
                        .filter(|&&(a, n)| (a..a + n).contains(&fault))
                        .count(),
                    reference
                        .reads
                        .borrow()
                        .iter()
                        .filter(|&&(a, n)| (a..a + n).contains(&fault))
                        .count(),
                    "failed occurrence retried at {fault:x}, warm={warm}"
                );
            }
        }
    }
}

#[test]
fn advisor_sections_keep_original_roster_ids_and_stop_at_semantic_failures() {
    let _globals = crate::extended::characters::test_globals();
    let original = section_fixture();
    let memory = characters_test_kit::memory(&original);
    let roster = ccharacter_manager::Roster::read(&memory, IMAGE, ADDRESS).unwrap();
    for (index, (_, section)) in roster.sections().into_iter().enumerate() {
        let entry = section
            .cursor()
            .next(&memory, [false, false, false, true])
            .unwrap()
            .unwrap()
            .entry()
            .unwrap();
        assert_eq!(
            (entry.roster, entry.ordinal, entry.pointer),
            (index as u8, 0, ADDRESS + CHARACTERS[index])
        );
    }
    for warm in [false, true] {
        for invalid_historical in [false, true] {
            let mut job = section_job();
            if warm {
                assert!(copy_characters(&memory, section_context(), &mut job, Roles::Independent).raw_ok);
                job.prepare_read_plans();
                job.clear();
            }
            let mut bytes = original.clone();
            pointer(&mut bytes, CHARACTERS[1], 0);
            if invalid_historical {
                pointer(&mut bytes, characters_test_kit::ADV_ROLE, 0);
            }
            let source = WaveMemory::new(&bytes, None);
            assert!(copy_characters(&source, section_context(), &mut job, Roles::Independent).raw_ok);
            assert_advisors_match(&job, &source);
            if invalid_historical {
                assert!(job.advisor_dynamic.as_ref().unwrap().log.spans.is_empty());
            }
        }
    }
}

#[test]
fn independent_parts_merge_fresh_rows_and_isolate_fallbacks_and_failures() {
    let _globals = crate::extended::characters::test_globals();
    use super::characters_test_kit as kit;
    let capacities = split_capacities();
    let baseline = kit::mapped_fixture();
    let mut jobs: [CharactersJob; PARTS] = std::array::from_fn(|part| {
        let mut log = collector_raw::raw::RawLog::new(8192, 65536);
        log.initialize_scattered_read_plan(8192, 8192, 65536);
        CharactersJob {
            part,
            appended_roles: Vec::with_capacity(log.spans.capacity() / 8),
            log,
            traits: TraitCapture::with_test_capacities(4096, 4096, 4096, 1 << 20),
            source_bindings: Some(fixture_bindings()),
            ..Default::default()
        }
    });
    let context = CharactersCopyContext {
        force_full: false,
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        active: [true; N],
    };
    let mut fresh = baseline.clone();
    put(
        &mut fresh,
        UNITS[0] + cunit_leader::EXPERIENCE,
        123456_i64.to_le_bytes(),
    );
    put(&mut fresh, EXPERIENCE_ARRAY + 8, 654321_i64.to_le_bytes());
    let mut changed_role = fresh.clone();
    put(
        &mut changed_role,
        kit::ADV_ROLE + cadvisor::IDEA_TOKEN,
        0_u64.to_le_bytes(),
    );
    let mut changed_roster = fresh.clone();
    put(&mut changed_roster, ROSTER, 0_u64.to_le_bytes());
    for bytes in [
        &baseline,
        &fresh,
        &changed_role,
        &fresh,
        &changed_roster,
        &baseline,
    ] {
        let memory = kit::memory(bytes);
        for (part, job) in jobs.iter_mut().enumerate() {
            let active = PART_MASKS[part];
            let report = copy_characters(
                &memory,
                CharactersCopyContext { active, ..context },
                job,
                Roles::Independent,
            );
            assert!(report.raw_ok && !report.plan_failed);
            for (slot, &ticks) in report.copy_ticks.iter().enumerate() {
                if slot != anchor(active) {
                    assert_eq!(ticks, 0);
                }
            }
        }
        for order in [[0, 1, 2, 3], [3, 2, 1, 0], [2, 0, 3, 1]] {
            let mut merged = Captured::with_capacities(&capacities);
            merged.set_character_activity([true; N]);
            merged.raw_active = [true; N];
            merged.capture(&memory, kit::BASE, kit::GAME_STATE, 0, false);
            for part in order {
                merged.decode_characters_payload(kit::BASE, &jobs[part], &mut [0; N]);
            }
            kit::assert_matches_direct(&merged, &kit::direct(&memory, &capacities));
            assert_eq!(merged.raw_active, [false; N]);
        }
        for job in &mut jobs {
            job.log.prepare_read_plan();
            job.clear();
        }
    }
    let memory = kit::memory(&fresh);
    for (part, job) in jobs.iter_mut().enumerate() {
        let report = copy_characters(
            &memory,
            CharactersCopyContext {
                active: PART_MASKS[part],
                ..context
            },
            job,
            Roles::Independent,
        );
        assert!(report.raw_ok && job.log.planned_capture_succeeded());
    }
    for (missing, missing_mask) in PART_MASKS.into_iter().enumerate() {
        for failed in [false, true] {
            let mut merged = Captured::with_capacities(&capacities);
            merged.set_character_activity([true; N]);
            merged.raw_active = if failed {
                [true; N]
            } else {
                missing_mask.map(|active| !active)
            };
            merged.capture(&memory, kit::BASE, kit::GAME_STATE, 0, false);
            let mut expected = kit::direct(&memory, &capacities);
            if failed {
                merged.fail_character_tables(missing_mask, "test failure");
                expected.fail_character_tables(missing_mask, "test failure");
            }
            for part in (0..PARTS).rev().filter(|&part| part != missing) {
                merged.decode_characters_payload(kit::BASE, &jobs[part], &mut [0; N]);
            }
            kit::assert_matches_direct(&merged, &expected);
        }
    }
}

#[test]
fn portrait_census_is_fresh_complete_and_never_reuses_a_failed_lease() {
    let _globals = crate::extended::characters::test_globals();
    use super::characters_test_kit as kit;
    let make = || {
        let mut log = collector_raw::raw::RawLog::new(8192, 65536);
        log.initialize_scattered_read_plan(8192, 8192, 65536);
        CharactersJob {
            log,
            portrait_census: Some(
                engine_allocation::shared(ccharacter_manager::PortraitCensus::try_new(8).unwrap())
                    .unwrap(),
            ),
            ..Default::default()
        }
    };
    let ctx = CharactersCopyContext {
        force_full: false,
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        active: [true, false, false, true, false, true],
    };
    let baseline = kit::mapped_fixture();
    let mut job = make();
    copy_characters(&kit::memory(&baseline), ctx, &mut job, Roles::Independent);
    let rows = job
        .portrait_census()
        .unwrap()
        .rows(ctx.base, ctx.game_state)
        .unwrap()
        .to_vec();
    assert_eq!(rows.len(), 3);
    job.log.prepare_read_plan();
    job.clear();
    assert!(job.portrait_census().is_none());
    let mut changed = baseline.clone();
    put(
        &mut changed,
        UNITS[0] + cunit_leader::EXPERIENCE,
        987_i64.to_le_bytes(),
    );
    copy_characters(&kit::memory(&changed), ctx, &mut job, Roles::Independent);
    assert!(job.log.planned_capture_succeeded());
    assert_eq!(
        job.portrait_census()
            .unwrap()
            .rows(ctx.base, ctx.game_state)
            .unwrap(),
        rows
    );
    pointer(&mut changed, ROSTER, CHARACTERS[1]);
    pointer(&mut changed, ROSTER + 8, CHARACTERS[0]);
    copy_characters(&kit::memory(&changed), ctx, &mut job, Roles::Independent);
    let reordered = job.portrait_census().unwrap();
    assert_eq!(
        reordered.rows(ctx.base, ctx.game_state).unwrap()[0],
        rows[1]
    );
    copy_characters(&kit::memory(&baseline), ctx, &mut job, Roles::Independent);
    assert!(job.portrait_census().is_none());
    job.log.prepare_read_plan();
    drop(reordered);
    copy_characters(&kit::memory(&baseline), ctx, &mut job, Roles::Independent);
    assert!(job.log.planned_capture_succeeded());
    assert!(job.portrait_census().is_none());
    let mut job = make();
    let daily = CharactersCopyContext {
        active: [true, true, false, true, false, true],
        ..ctx
    };
    copy_characters(&kit::memory(&baseline), ctx, &mut job, Roles::Independent);
    job.log.prepare_read_plan();
    assert!(copy_characters(&kit::memory(&baseline), daily, &mut job, Roles::Independent).raw_ok);
    job.log.prepare_read_plan();
    copy_characters(&kit::memory(&baseline), ctx, &mut job, Roles::Independent);
    assert!(
        job.log.planned_capture_succeeded(),
        "unchanged daily census should preserve the hourly plan"
    );
    assert!(copy_characters(&kit::memory(&changed), daily, &mut job, Roles::Independent).raw_ok);
    assert_eq!(
        job.portrait_census()
            .unwrap()
            .rows(ctx.base, ctx.game_state)
            .unwrap()[0],
        rows[1]
    );
    job.log.prepare_read_plan();
    copy_characters(&kit::memory(&baseline), ctx, &mut job, Roles::Independent);
    assert!(
        !job.log.planned_capture_succeeded(),
        "changed daily census must retire the old hourly association"
    );
    assert_eq!(
        job.portrait_census()
            .unwrap()
            .rows(ctx.base, ctx.game_state)
            .unwrap(),
        rows
    );
    job.log.prepare_read_plan();
    copy_characters(&kit::memory(&baseline), ctx, &mut job, Roles::Independent);
    assert!(job.log.planned_capture_succeeded());
    assert_eq!(
        job.portrait_census()
            .unwrap()
            .rows(ctx.base, ctx.game_state)
            .unwrap(),
        rows
    );
    for blocked in [
        ADDRESS + CHARACTERS[0],
        ADDRESS + CHARACTERS[1] + ccharacter::COUNTRY,
        ADDRESS + ROSTER,
    ] {
        let mut job = make();
        copy_characters(
            &WaveMemory::new(&baseline, Some(blocked)),
            ctx,
            &mut job,
            Roles::Independent,
        );
        assert!(job.portrait_census().is_none(), "fault {blocked:x}");
    }
    let mut job = make();
    job.portrait_census = Some(
        engine_allocation::shared(ccharacter_manager::PortraitCensus::try_new(2).unwrap()).unwrap(),
    );
    copy_characters(&kit::memory(&baseline), ctx, &mut job, Roles::Independent);
    assert!(!job.portrait_census.as_ref().unwrap().needs_growth());
    assert_eq!(
        job.portrait_census()
            .unwrap()
            .rows(ctx.base, ctx.game_state)
            .unwrap(),
        rows
    );
}

#[test]
fn hourly_plan_reads_fresh_values_and_preserves_every_fault_without_retry() {
    let _globals = crate::extended::characters::test_globals();
    use super::characters_test_kit as kit;
    use std::cell::RefCell;
    struct Trace<'a> {
        bytes: &'a [u8],
        calls: RefCell<Vec<(usize, usize)>>,
        fault: Option<(usize, usize)>,
    }
    impl Memory for Trace<'_> {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, out.len()));
            if self.fault == Some((address, out.len())) {
                out.fill(0xEE);
                return false;
            }
            kit::memory(self.bytes).copy(address, out)
        }
    }
    fn job() -> CharactersJob {
        let mut log = collector_raw::raw::RawLog::new(8192, 65536);
        log.initialize_scattered_read_plan(8192, 8192, 65536);
        CharactersJob {
            log,
            ..Default::default()
        }
    }
    let ctx = CharactersCopyContext {
        force_full: false,
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        active: [true, false, false, true, false, true],
    };
    let baseline = kit::mapped_fixture();
    let mut first = job();
    assert!(copy_characters(&kit::memory(&baseline), ctx, &mut first, Roles::Independent).raw_ok);
    first.log.prepare_read_plan();
    let mut fresh = baseline.clone();
    put(
        &mut fresh,
        UNITS[0] + cunit_leader::EXPERIENCE,
        123456789_i64.to_le_bytes(),
    );
    let report = copy_characters(&kit::memory(&fresh), ctx, &mut first, Roles::Independent);
    assert!(report.raw_ok && !report.plan_failed);
    assert!(
        first.log.planned_capture_succeeded(),
        "quiet structure must skip discovery"
    );
    let mut canonical = job();
    assert!(copy_characters(&kit::memory(&fresh), ctx, &mut canonical, Roles::Independent).raw_ok);
    assert!(first.log.logs_equal(&canonical.log));
    let captured = |payload: &CharactersJob| {
        let mut result = Captured::with_capacities(&split_capacities());
        result.decode_characters_payload(kit::BASE, payload, &mut [0; N]);
        result
    };
    kit::assert_matches_direct(&captured(&first), &captured(&canonical));
    let source = Trace {
        bytes: &baseline,
        calls: RefCell::new(vec![]),
        fault: None,
    };
    copy_characters(&source, ctx, &mut canonical, Roles::Independent);
    let reads = source.calls.into_inner();
    for rename in [false, true] {
        for fault in reads.iter().copied().map(Some).chain([None]) {
            let mut planned = job();
            copy_characters(
                &kit::memory(&baseline),
                ctx,
                &mut planned,
                Roles::Independent,
            );
            planned.log.prepare_read_plan();
            let mut changed = fresh.clone();
            if rename {
                changed[CHARACTERS[0] + ccharacter::NAME] ^= 1;
            }
            let actual = Trace {
                bytes: &changed,
                calls: RefCell::new(vec![]),
                fault,
            };
            let report = copy_characters(&actual, ctx, &mut planned, Roles::Independent);
            assert!(report.raw_ok && !report.plan_failed, "fault {fault:?}");
            let reference = Trace {
                bytes: &changed,
                calls: RefCell::new(vec![]),
                fault,
            };
            let mut expected = job();
            copy_characters(&reference, ctx, &mut expected, Roles::Independent);
            assert_eq!(
                *actual.calls.borrow(),
                *reference.calls.borrow(),
                "fault {fault:?}"
            );
            assert!(planned.log.logs_equal(&expected.log), "fault {fault:?}");
            kit::assert_matches_direct(&captured(&planned), &captured(&expected));
        }
    }
    for field in [
        ROSTER,
        CHARACTERS[0] + ccharacter::UNIT_LEADER,
        UNITS[0] + cunit_leader::SKILL,
    ] {
        let mut planned = job();
        copy_characters(
            &kit::memory(&baseline),
            ctx,
            &mut planned,
            Roles::Independent,
        );
        planned.log.prepare_read_plan();
        let mut changed = baseline.clone();
        put(&mut changed, field, 0_u64.to_le_bytes());
        let actual = Trace {
            bytes: &changed,
            calls: RefCell::new(vec![]),
            fault: None,
        };
        let report = copy_characters(&actual, ctx, &mut planned, Roles::Independent);
        assert!(report.raw_ok && !report.plan_failed);
        let reference = Trace {
            bytes: &changed,
            calls: RefCell::new(vec![]),
            fault: None,
        };
        let mut expected = job();
        copy_characters(&reference, ctx, &mut expected, Roles::Independent);
        assert_eq!(
            *actual.calls.borrow(),
            *reference.calls.borrow(),
            "changed field {field:x}"
        );
        assert!(planned.log.logs_equal(&expected.log));
        kit::assert_matches_direct(&captured(&planned), &captured(&expected));
    }
}

#[test]
fn template_cache_keeps_the_first_duplicate_occurrence() {
    let _globals = crate::extended::characters::test_globals();
    let mut rows = [ccharacter::Row::default(); 64];
    let bytes = std::array::from_fn::<_, 64, _>(|index| index as u8);
    for (index, row) in rows.iter_mut().enumerate() {
        row.id = ((index * 5) % 8) as u32;
        row.template = row.id as usize + 1;
        row.template_identity.line = index as i32;
        row.template_file_start = index;
        row.template_file_len = 1;
    }
    let mut cache = TemplateCache::default();
    cache.rebuild(Outcome::Rows { len: rows.len() }, &rows, &bytes);
    for id in 0..8 {
        let first = rows.iter().find(|row| row.id == id).unwrap();
        let known = cache.known(id, first.template).unwrap();
        assert_eq!(known.identity, first.template_identity);
        assert_eq!(
            known.file,
            &bytes[first.template_file_start..first.template_file_start + 1]
        );
    }
    rows[8].template += 1;
    cache.rebuild(Outcome::Rows { len: rows.len() }, &rows, &bytes);
    assert!(cache.known(rows[8].id, rows[8].id as usize + 1).is_none());
}

#[test]
fn template_identities_are_read_once_per_pointer_and_again_after_a_reload() {
    let _globals = crate::extended::characters::test_globals();
    use super::characters_test_kit as kit;
    use std::cell::RefCell;
    struct Trace<'a> {
        bytes: &'a [u8],
        calls: RefCell<Vec<(usize, usize)>>,
        fault: Option<(usize, usize)>,
    }
    impl Memory for Trace<'_> {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, out.len()));
            if self.fault == Some((address, out.len())) {
                out.fill(0xEE);
                return false;
            }
            kit::memory(self.bytes).copy(address, out)
        }
    }
    const TEMPLATES: [usize; 3] = [0x9000, 0x9400, 0x9c00];
    const FILES: [usize; 2] = [0x9800, 0x9900];
    fn template(
        bytes: &mut [u8],
        at: usize,
        tag: u32,
        file: Option<(usize, &[u8])>,
        line: i32,
        generated: u8,
    ) {
        put(
            bytes,
            at,
            ((IMAGE + ccharacter_template::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(bytes, at + ccharacter_template::TAG, tag.to_le_bytes());
        let header = at + ccharacter_template::SOURCE_FILE;
        match file {
            Some((payload, value)) => {
                pointer(bytes, header, payload);
                put(bytes, header + 16, (value.len() as u64).to_le_bytes());
                put(bytes, header + 24, (value.len() as u64).to_le_bytes());
                bytes[payload..payload + value.len()].copy_from_slice(value);
                bytes[payload + value.len()] = 0;
            }
            None => literal(bytes, header, b""),
        }
        put(
            bytes,
            at + ccharacter_template::SOURCE_LINE,
            line.to_le_bytes(),
        );
        bytes[at + ccharacter_template::GENERATED] = generated;
    }
    let mut world = kit::mapped_fixture();
    world.resize(0xa000, 0);
    template(
        &mut world,
        TEMPLATES[0],
        501,
        Some((FILES[0], b"common/characters/GER.txt")),
        3,
        0,
    );
    template(&mut world, TEMPLATES[1], 502, None, -1, 1);
    pointer(
        &mut world,
        CHARACTERS[0] + ccharacter::TEMPLATE,
        TEMPLATES[0],
    );
    pointer(
        &mut world,
        CHARACTERS[1] + ccharacter::TEMPLATE,
        TEMPLATES[1],
    );
    let active = PART_MASKS[0];
    let context = |force_full| CharactersCopyContext {
        force_full,
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        active,
    };
    let direct = |world: &[u8]| {
        let mut captured = Captured::with_capacities(&split_capacities());
        let results = walk_family(
            &kit::memory(world),
            kit::BASE,
            kit::GAME_STATE,
            captured.family().select(active),
            None,
            None,
        );
        captured.install_family(active, results);
        captured.fail_character_tables(
            active.map(|value| !value),
            replay_data::cadence::SKIPPED_REASON,
        );
        captured
    };
    let hour = |job: &mut CharactersJob, world: &[u8], force_full: bool, fault| {
        let source = Trace {
            bytes: world,
            calls: RefCell::new(vec![]),
            fault,
        };
        let report = copy_characters(&source, context(force_full), job, Roles::Independent);
        assert!(report.raw_ok && !report.plan_failed, "fault {fault:?}");
        let mut captured = Captured::with_capacities(&split_capacities());
        captured.decode_characters_payload(kit::BASE, job, &mut [0; N]);
        captured.remember_templates(job);
        let planned = job.log.planned_capture_succeeded();
        job.prepare_read_plans();
        job.clear();
        (captured, source.calls.into_inner(), planned)
    };
    let reads = |calls: &[(usize, usize)], at: usize| {
        calls
            .iter()
            .any(|&(address, _)| (ADDRESS + at..ADDRESS + at + 0x248).contains(&address))
    };
    let mut job = CharactersJob::new_part(0);

    let (first, calls, _) = hour(&mut job, &world, false, None);
    kit::assert_matches_direct(&first, &direct(&world));
    assert!(reads(&calls, TEMPLATES[0]) && reads(&calls, TEMPLATES[1]));
    assert!(calls.contains(&(ADDRESS + FILES[0], 25)));
    let TableView::Complete(rows) = first.characters_table() else {
        panic!("characters table incomplete");
    };
    let rows = rows.rows::<tables::Characters>().unwrap();
    let by_template = |tag: u32| rows.iter().find(|row| row.template_tag == tag).unwrap();
    let ger = by_template(501);
    assert_eq!(
        (
            ger.template_present,
            ger.template_file.as_slice(),
            ger.template_line,
            ger.template_generated
        ),
        (1, b"common/characters/GER.txt".as_slice(), 3, 0)
    );
    let never_read = by_template(502);
    assert_eq!(
        (
            never_read.template_present,
            never_read.template_file.len(),
            never_read.template_line,
            never_read.template_generated
        ),
        (1, 0, -1, 1)
    );
    assert!(
        rows.iter()
            .any(|row| row.template_present == 0 && row.template_tag == 0)
    );
    let demand = ccharacter::demand(&kit::memory(&world), kit::BASE, kit::GAME_STATE).unwrap();
    let mut sized = Capacities {
        rows: [1; N],
        bytes: [1; N],
    };
    sized.raise_for_characters(demand);
    let mut captured = Captured::with_capacities(&sized);
    let only = [true, false, false, false, false, false];
    let results = walk_family(
        &kit::memory(&world),
        kit::BASE,
        kit::GAME_STATE,
        captured.family().select(only),
        None,
        None,
    );
    assert_eq!(results[0].outcome, Outcome::Rows { len: demand.rows });
    assert_eq!(
        (demand.rows, results[0].bytes_len),
        (rows.len(), demand.bytes)
    );

    for planned_hour in [false, true] {
        let (captured, calls, planned) = hour(&mut job, &world, false, None);
        kit::assert_matches_direct(&captured, &direct(&world));
        assert!(!reads(&calls, TEMPLATES[0]) && !reads(&calls, TEMPLATES[1]));
        assert!(
            !calls
                .iter()
                .any(|&(address, _)| address == ADDRESS + FILES[0] + 25)
        );
        assert_eq!(planned, planned_hour);
    }

    let mut reloaded = world.clone();
    template(
        &mut reloaded,
        TEMPLATES[2],
        503,
        Some((FILES[1], b"common/characters/ENG.txt")),
        9,
        0,
    );
    pointer(
        &mut reloaded,
        CHARACTERS[0] + ccharacter::TEMPLATE,
        TEMPLATES[2],
    );
    let (captured, calls, _) = hour(&mut job, &reloaded, false, None);
    kit::assert_matches_direct(&captured, &direct(&reloaded));
    assert!(reads(&calls, TEMPLATES[2]) && !reads(&calls, TEMPLATES[1]));
    let (captured, calls, _) = hour(&mut job, &reloaded, false, None);
    kit::assert_matches_direct(&captured, &direct(&reloaded));
    assert!(!reads(&calls, TEMPLATES[2]));

    let (captured, calls, _) = hour(&mut job, &reloaded, true, None);
    kit::assert_matches_direct(&captured, &direct(&reloaded));
    assert!(reads(&calls, TEMPLATES[2]) && reads(&calls, TEMPLATES[1]));

    let mut fresh = CharactersJob::new_part(0);
    let (_, calls, _) = hour(&mut fresh, &world, false, None);
    let mut faults = 0;
    for fault in calls.into_iter().filter(|&(address, _)| {
        reads(&[(address, 0)], TEMPLATES[0])
            || reads(&[(address, 0)], TEMPLATES[1])
            || (ADDRESS + FILES[0]..ADDRESS + FILES[0] + 26).contains(&address)
    }) {
        let mut job = CharactersJob::new_part(0);
        let (captured, _, _) = hour(&mut job, &world, false, Some(fault));
        let reference = Trace {
            bytes: &world,
            calls: RefCell::new(vec![]),
            fault: Some(fault),
        };
        let mut expected = Captured::with_capacities(&split_capacities());
        let results = walk_family(
            &reference,
            kit::BASE,
            kit::GAME_STATE,
            expected.family().select(active),
            None,
            None,
        );
        expected.install_family(active, results);
        expected.fail_character_tables(
            active.map(|value| !value),
            replay_data::cadence::SKIPPED_REASON,
        );
        kit::assert_matches_direct(&captured, &expected);
        assert!(
            matches!(
                captured.characters.rows.outcome,
                Outcome::Unavailable { .. }
            ),
            "fault {fault:?}"
        );
        assert!(job.templates.entries.is_empty());
        faults += 1;
    }
    assert_eq!(faults, 6);
}

#[test]
fn complete_family_plan_refreshes_roles_and_traits_without_retry() {
    let _globals = crate::extended::characters::test_globals();
    use super::characters_test_kit as kit;
    use std::cell::RefCell;
    struct Trace<'a> {
        bytes: &'a [u8],
        calls: RefCell<Vec<(usize, usize)>>,
        fault: Option<(usize, usize)>,
    }
    impl Memory for Trace<'_> {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, out.len()));
            if self.fault == Some((address, out.len())) {
                return false;
            }
            kit::memory(self.bytes).copy(address, out)
        }
    }
    fn job() -> CharactersJob {
        let mut log = collector_raw::raw::RawLog::new(8192, 65536);
        log.initialize_scattered_read_plan(8192, 8192, 65536);
        CharactersJob {
            log,
            traits: TraitCapture::with_test_capacities(4096, 4096, 4096, 1 << 20),
            appended_roles: Vec::with_capacity(1024),
            source_bindings: Some(fixture_bindings()),
            ..Default::default()
        }
    }
    fn direct(memory: &impl Memory, capacities: &Capacities, active: [bool; N]) -> Captured {
        let mut captured = Captured::with_capacities(capacities);
        let results = walk_family(
            memory,
            kit::BASE,
            kit::GAME_STATE,
            captured.family().select(active),
            None,
            None,
        );
        captured.install_family([true; N], results);
        captured
    }
    for active in [[true; N], PART_MASKS[1], PART_MASKS[2], PART_MASKS[3]] {
        let capacities = split_capacities();
        let context = CharactersCopyContext {
            force_full: false,
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            active,
        };
        let baseline = kit::mapped_fixture();
        let decode = |payload: &CharactersJob| {
            let mut captured = Captured::with_capacities(&capacities);
            captured.decode_characters_payload(kit::BASE, payload, &mut [0; N]);
            captured
        };
        let mut prepared = job();
        assert!(copy_characters(&kit::memory(&baseline), context, &mut prepared, Roles::Independent).raw_ok);
        assert!(prepared.source_bindings.as_ref().unwrap().ready);
        prepared.log.prepare_read_plan();
        prepared.clear();
        let mut fresh = baseline.clone();
        put(
            &mut fresh,
            UNITS[0] + cunit_leader::EXPERIENCE,
            123456_i64.to_le_bytes(),
        );
        put(&mut fresh, EXPERIENCE_ARRAY + 8, 654321_i64.to_le_bytes());
        assert!(copy_characters(&kit::memory(&fresh), context, &mut prepared, Roles::Independent).raw_ok);
        assert!(prepared.log.planned_capture_succeeded());
        kit::assert_matches_direct(
            &decode(&prepared),
            &direct(&kit::memory(&fresh), &capacities, active),
        );

        let source = Trace {
            bytes: &baseline,
            calls: RefCell::new(vec![]),
            fault: None,
        };
        copy_characters(&source, context, &mut job(), Roles::Independent);
        let reads: std::collections::BTreeSet<_> = source.calls.into_inner().into_iter().collect();
        for fault in reads.into_iter().map(Some).chain([None]) {
            let mut prepared = job();
            copy_characters(
                &kit::memory(&baseline),
                context,
                &mut prepared,
                Roles::Independent,
            );
            prepared.log.prepare_read_plan();
            prepared.clear();
            let source = Trace {
                bytes: &fresh,
                calls: RefCell::new(vec![]),
                fault,
            };
            let report = copy_characters(&source, context, &mut prepared, Roles::Independent);
            assert!(report.raw_ok && !report.plan_failed, "fault {fault:?}");
            let reference = Trace {
                bytes: &fresh,
                calls: RefCell::new(vec![]),
                fault,
            };
            let mut ordinary = job();
            copy_characters(&reference, context, &mut ordinary, Roles::Independent);
            kit::assert_matches_direct(&decode(&prepared), &decode(&ordinary));
            if let Some(fault) = fault {
                assert!(
                    source
                        .calls
                        .borrow()
                        .iter()
                        .filter(|&&read| read == fault)
                        .count()
                        <= reference
                            .calls
                            .borrow()
                            .iter()
                            .filter(|&&read| read == fault)
                            .count(),
                    "prepared path retried a failed observation {fault:?}"
                );
            }
        }
        for field in [
            ROSTER,
            CHARACTERS[0] + ccharacter::UNIT_LEADER,
            UNITS[0] + cunit_leader::SKILL,
            TRAIT_ARRAY,
            EXPERIENCE_ARRAY,
            kit::ADV_ROLE + cadvisor::IDEA_TOKEN,
        ] {
            let mut prepared = job();
            copy_characters(
                &kit::memory(&baseline),
                context,
                &mut prepared,
                Roles::Independent,
            );
            prepared.log.prepare_read_plan();
            let mut changed = baseline.clone();
            put(&mut changed, field, 0_u64.to_le_bytes());
            for source in [&changed, &baseline] {
                assert!(
                    copy_characters(&kit::memory(source), context, &mut prepared, Roles::Independent).raw_ok
                );
                kit::assert_matches_direct(
                    &decode(&prepared),
                    &direct(&kit::memory(source), &capacities, active),
                );
                prepared.log.prepare_read_plan();
            }
        }
        for limit in [0, 1, 2] {
            let mut prepared = job();
            copy_characters(
                &kit::memory(&baseline),
                context,
                &mut prepared,
                Roles::Independent,
            );
            prepared.log.prepare_read_plan();
            for bytes_limit in [false, true] {
                let mut limited = split_capacities();
                if bytes_limit {
                    limited.bytes[4] = limit;
                } else {
                    limited.rows[4] = limit;
                }
                let limited_context = CharactersCopyContext {
                    force_full: false,
                    ..context
                };
                for (ctx, caps) in [(limited_context, &limited), (context, &capacities)] {
                    assert!(
                        copy_characters(
                            &kit::memory(&baseline),
                            ctx,
                            &mut prepared,
                            Roles::Independent,
                        )
                        .raw_ok
                    );
                    let mut actual = Captured::with_capacities(caps);
                    actual.decode_characters_payload(kit::BASE, &prepared, &mut [0; N]);
                    kit::assert_matches_direct(
                        &actual,
                        &direct(&kit::memory(&baseline), caps, active),
                    );
                    prepared.log.prepare_read_plan();
                }
            }
        }
    }
}

pub const ADDRESS: usize = 0x10000;
pub const IMAGE: usize = 0x140000000;
pub const MANAGER: usize = 0x1000;
pub const ROSTER: usize = 0x2000;
pub const CHARACTERS: [usize; 2] = [0x2100, 0x2200];
pub const UNITS: [usize; 2] = [0x3000, 0x5000];
pub const TRAIT_ARRAY: usize = 0x7000;
pub const EXPERIENCE_ARRAY: usize = 0x7040;
pub const DEFINITION: usize = 0x7800;

#[cfg(test)]
fn walk_units(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    units: GrowingBatch<'_, LeaderRow>,
    traits: Batch<'_, cunit_leader::TraitRow>,
) -> [ReadResult; 2] {
    let result = walk_family(
        memory,
        base,
        game_state,
        Family {
            characters: Batch::new(&mut Vec::new(), &mut Vec::new()),
            leaders: Batch::new(&mut Vec::new(), &mut Vec::new()),
            advisors: Batch::new(&mut Vec::new(), &mut Vec::new()),
            units,
            traits,
            training: true,
        },
        None,
        None,
    );
    [result[3], result[4]]
}

pub fn put<const N: usize>(bytes: &mut [u8], offset: usize, value: [u8; N]) {
    bytes[offset..offset + N].copy_from_slice(&value);
}

pub fn pointer(bytes: &mut [u8], offset: usize, target: usize) {
    put(bytes, offset, ((ADDRESS + target) as u64).to_le_bytes());
}

pub fn array(bytes: &mut [u8], offset: usize, data: usize, count: u32) {
    pointer(bytes, offset, data);
    put(bytes, offset + 8, count.to_le_bytes());
    put(bytes, offset + 12, count.to_le_bytes());
}

pub fn literal(bytes: &mut [u8], offset: usize, value: &[u8]) {
    assert!(value.len() < 16);
    bytes[offset..offset + 32].fill(0);
    bytes[offset..offset + value.len()].copy_from_slice(value);
    put(bytes, offset + 16, (value.len() as u64).to_le_bytes());
    put(bytes, offset + 24, 15_u64.to_le_bytes());
}

pub fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x9000];
    pointer(&mut bytes, cgame_state::CHARACTER_MANAGER, MANAGER);
    put(
        &mut bytes,
        MANAGER,
        ((IMAGE + ccharacter_manager::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    array(
        &mut bytes,
        MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
        ROSTER,
        2,
    );
    put(
        &mut bytes,
        DEFINITION,
        ((IMAGE + cunit_leader_trait::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        DEFINITION + cunit_leader_trait::TOKEN_ID,
        (-22_i32).to_le_bytes(),
    );
    put(
        &mut bytes,
        DEFINITION + cunit_leader_trait::ORDINAL,
        i32::MIN.to_le_bytes(),
    );
    literal(
        &mut bytes,
        DEFINITION + cunit_leader_trait::TOKEN,
        b"trait\xff",
    );
    for index in 0..2 {
        let character = CHARACTERS[index];
        let unit = UNITS[index];
        let id = u32::MAX - index as u32;
        pointer(&mut bytes, ROSTER + index * 8, character);
        put(
            &mut bytes,
            character,
            ((IMAGE + ccharacter::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            character + ccharacter::TYPE,
            ccharacter::REFERENCE_TYPE.to_le_bytes(),
        );
        put(&mut bytes, character + ccharacter::ID, id.to_le_bytes());
        put(
            &mut bytes,
            character + ccharacter::COUNTRY,
            (index as u32 + 1).to_le_bytes(),
        );
        pointer(&mut bytes, character + ccharacter::UNIT_LEADER, unit);
        literal(&mut bytes, character + ccharacter::NAME, b"literal");
        literal(&mut bytes, character + ccharacter::TOKEN, b"saved");
        for (role, offset) in [ccharacter::LEADER_MAP_HEAD, ccharacter::ADVISOR_MAP_HEAD]
            .into_iter()
            .enumerate()
        {
            let head = 0x2400 + (index * 2 + role) * 0x30;
            pointer(&mut bytes, character + offset, head);
            for link in [0, 8, 16] {
                pointer(&mut bytes, head + link, head);
            }
            bytes[head + 0x19] = 1;
        }
        put(
            &mut bytes,
            unit,
            ((IMAGE + cunit_leader::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            unit + cunit_leader::TYPE,
            cunit_leader::REFERENCE_TYPE.to_le_bytes(),
        );
        put(&mut bytes, unit + cunit_leader::ID, id.to_le_bytes());
        put(
            &mut bytes,
            unit + cunit_leader::CHARACTER_TYPE,
            ccharacter::REFERENCE_TYPE.to_le_bytes(),
        );
        put(
            &mut bytes,
            unit + cunit_leader::CHARACTER_ID,
            id.to_le_bytes(),
        );
        put(
            &mut bytes,
            unit + cunit_leader::EXPERIENCE,
            (i64::MIN + index as i64).to_le_bytes(),
        );
        literal(
            &mut bytes,
            unit + cunit_leader::TOKEN,
            if index == 0 { b"saved1" } else { b"saved2" },
        );
        literal(
            &mut bytes,
            unit + cunit_leader::NAME,
            if index == 0 { b"other1" } else { b"other2" },
        );
        array(
            &mut bytes,
            unit + cunit_leader::TRAIT_DATA,
            TRAIT_ARRAY + index * 8,
            1,
        );
        pointer(&mut bytes, TRAIT_ARRAY + index * 8, DEFINITION);
        array(
            &mut bytes,
            unit + cunit_leader::TRAIT_EXPERIENCE_DATA,
            EXPERIENCE_ARRAY + index * 16,
            1,
        );
        pointer(&mut bytes, EXPERIENCE_ARRAY + index * 16, DEFINITION);
        put(
            &mut bytes,
            EXPERIENCE_ARRAY + index * 16 + 8,
            (i64::MAX - index as i64).to_le_bytes(),
        );
    }
    bytes
}

#[cfg(test)]
struct CountedMemory<'a> {
    memory: SliceMemory<'a>,
    character_header_reads: Cell<usize>,
}

#[cfg(test)]
struct FaultMemory<'a> {
    memory: SliceMemory<'a>,
    fault: std::ops::Range<usize>,
    failures: Cell<usize>,
}

#[cfg(test)]
impl Memory for FaultMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let Some(end) = address.checked_add(output.len()) else {
            return false;
        };
        if address < self.fault.end && end > self.fault.start {
            self.failures.set(self.failures.get() + 1);
            let prefix = self.fault.start.saturating_sub(address).min(output.len());
            if prefix != 0 {
                self.memory.copy(address, &mut output[..prefix]);
            }
            return false;
        }
        self.memory.copy(address, output)
    }
}

#[cfg(test)]
impl Memory for CountedMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if output.len() == 16
            && CHARACTERS
                .iter()
                .any(|&character| address == ADDRESS + character)
        {
            self.character_header_reads
                .set(self.character_header_reads.get() + 1);
        }
        self.memory.copy(address, output)
    }
}

#[test]
fn shared_roster_keeps_traits_when_scalar_capacity_is_exhausted() {
    let _globals = crate::extended::characters::test_globals();
    let bytes = fixture();
    let memory = CountedMemory {
        memory: SliceMemory {
            base: ADDRESS,
            bytes: &bytes,
        },
        character_header_reads: Cell::new(0),
    };
    let mut units = vec![LeaderRow::default(); 1];
    let mut names = vec![0; 24];
    let mut traits = vec![cunit_leader::TraitRow::default(); 2];
    let mut tokens = vec![0; 12];
    let results = walk_units(
        &memory,
        IMAGE,
        ADDRESS,
        GrowingBatch::new(&mut units, &mut names),
        Batch::new(&mut traits, &mut tokens),
    );
    assert_eq!(memory.character_header_reads.get(), 2);
    assert_eq!(results[0].outcome, Outcome::Rows { len: 2 });
    assert_eq!(results[1].outcome, Outcome::Rows { len: 2 });
    assert_eq!(traits[1].token_start, 6);
    assert_eq!(&tokens[..12], b"trait\xfftrait\xff");
    assert_eq!(leader_row(&units[0], &names).experience, 0);
    assert_eq!(leader_row(&units[0], &names).inherited_text_a, b"saved1");
    assert_eq!(leader_row(&units[0], &names).inherited_text_b, b"other1");
    assert_eq!(leader_row(&units[1], &names).inherited_text_a, b"saved2");
    assert_eq!(leader_row(&units[1], &names).inherited_text_b, b"other2");
    assert_eq!(leader_reference(traits[0].leader_id), 0xffff_ffff_0000_1269);
}

#[test]
fn a_roster_larger_than_the_pool_is_recorded_whole_in_the_same_hour() {
    let _globals = crate::extended::characters::test_globals();
    let bytes = fixture();
    let memory = SliceMemory {
        base: ADDRESS,
        bytes: &bytes,
    };
    let mut ample = Captured::with_capacities(&fixture_capacities());
    ample.capture(&memory, IMAGE, ADDRESS, 0, true);
    let mut short = Captured::with_capacities(&Capacities {
        rows: [1; N],
        bytes: [1; N],
    });
    short.capture(&memory, IMAGE, ADDRESS, 0, true);
    assert_eq!(
        short.outcomes()[..5],
        [
            Outcome::Rows { len: 2 },
            Outcome::Rows { len: 0 },
            Outcome::Rows { len: 0 },
            Outcome::Rows { len: 2 },
            Outcome::Rows { len: 2 },
        ]
    );
    assert_eq!(short.outcomes(), ample.outcomes());
    assert_eq!(short.tables(), ample.tables());
}

#[test]
fn one_trait_vector_fault_does_not_invalidate_other_family_tables() {
    let _globals = crate::extended::characters::test_globals();
    let mut bytes = fixture();
    put(
        &mut bytes,
        UNITS[1] + cunit_leader::TRAIT_COUNT,
        (-1_i32).to_le_bytes(),
    );
    let memory = SliceMemory {
        base: ADDRESS,
        bytes: &bytes,
    };
    let mut units = vec![LeaderRow::default(); 2];
    let mut names = vec![0; 24];
    let mut traits = vec![cunit_leader::TraitRow::default(); 2];
    let mut tokens = vec![0; 12];
    let results = walk_units(
        &memory,
        IMAGE,
        ADDRESS,
        GrowingBatch::new(&mut units, &mut names),
        Batch::new(&mut traits, &mut tokens),
    );
    assert_eq!(results[0].outcome, Outcome::Rows { len: 2 });
    assert!(matches!(results[1].outcome, Outcome::Unavailable { .. }));
    assert_eq!(results[1].bytes_len, 0);
    assert_eq!(leader_row(&units[1], &names).inherited_text_a, b"saved2");
}

#[test]
fn character_parts_read_each_identity_once_per_owner_and_refresh_current_fields() {
    let _globals = crate::extended::characters::test_globals();
    let mut bytes = fixture();
    let capacity = Capacities {
        rows: [2; N],
        bytes: [32; N],
    };
    let mut captured = Captured::with_capacities(&capacity);
    for token in [0_i32, -7] {
        put(
            &mut bytes,
            CHARACTERS[0] + ccharacter::NAME_TOKEN_ID,
            token.to_le_bytes(),
        );
        let memory = CountedMemory {
            memory: SliceMemory {
                base: ADDRESS,
                bytes: &bytes,
            },
            character_header_reads: Cell::new(0),
        };
        captured.capture(&memory, IMAGE, ADDRESS, 0, true);
        assert_eq!(memory.character_header_reads.get(), 2 * PARTS);
        assert_eq!(
            captured.outcomes(),
            [
                Outcome::Rows { len: 2 },
                Outcome::Rows { len: 0 },
                Outcome::Rows { len: 0 },
                Outcome::Rows { len: 2 },
                Outcome::Rows { len: 2 },
                Outcome::Rows { len: 0 },
            ]
        );
        assert_eq!(captured.characters.rows.rows[0].name_token_id, token);
    }
}

#[test]
fn invalid_name_header_does_not_hide_roles_or_unit_state() {
    let _globals = crate::extended::characters::test_globals();
    let mut bytes = fixture();
    put(
        &mut bytes,
        CHARACTERS[0] + ccharacter::NAME + 16,
        16_u64.to_le_bytes(),
    );
    let memory = SliceMemory {
        base: ADDRESS,
        bytes: &bytes,
    };
    let capacity = Capacities {
        rows: [2; N],
        bytes: [32; N],
    };
    let mut captured = Captured::with_capacities(&capacity);
    let results = walk_family(&memory, IMAGE, ADDRESS, captured.family(), None, None);
    assert!(matches!(results[0].outcome, Outcome::Unavailable { .. }));
    assert_eq!(results[1].outcome, Outcome::Rows { len: 0 });
    assert_eq!(results[2].outcome, Outcome::Rows { len: 0 });
    for result in &results[3..] {
        assert_eq!(result.outcome, Outcome::Rows { len: 2 });
    }
}

#[test]
fn partial_character_span_faults_are_local_and_failed_spans_are_never_retried() {
    let _globals = crate::extended::characters::test_globals();
    let bytes = fixture();
    let capacity = Capacities {
        rows: [2; N],
        bytes: [32; N],
    };
    let expected = [2, 0, 0, 2, 2, 2];
    for (offset, affected) in [
        (
            ccharacter::NAME + 32,
            [true, false, false, false, false, false],
        ),
        (
            ccharacter::LEADER_MAP_HEAD + 8,
            [false, true, false, false, false, false],
        ),
        (
            ccharacter::ADVISOR_MAP_HEAD + 8,
            [false, false, true, false, false, false],
        ),
        (
            ccharacter::UNIT_LEADER + 4,
            [false, false, false, true, true, true],
        ),
        (
            ccharacter::COUNTRY + 2,
            [true, true, true, false, false, false],
        ),
    ] {
        let fault = ADDRESS + CHARACTERS[0] + offset;
        let memory = FaultMemory {
            memory: SliceMemory {
                base: ADDRESS,
                bytes: &bytes,
            },
            fault: fault..fault + 1,
            failures: Cell::new(0),
        };
        let mut captured = Captured::with_capacities(&capacity);
        let results = walk_family(&memory, IMAGE, ADDRESS, captured.family(), None, None);
        for (index, result) in results.into_iter().enumerate() {
            if affected[index] {
                assert!(
                    matches!(result.outcome, Outcome::Unavailable { .. }),
                    "offset={offset:x} table={index}"
                );
                assert_eq!(result.bytes_len, 0);
            } else {
                assert_eq!(
                    result.outcome,
                    Outcome::Rows {
                        len: expected[index]
                    },
                    "offset={offset:x} table={index}"
                );
            }
        }
        assert_eq!(
            memory.failures.get(),
            1,
            "failed span retried at {offset:x}"
        );
    }
}

#[test]
fn unrelated_portrait_bytes_are_not_read() {
    let _globals = crate::extended::characters::test_globals();
    let bytes = fixture();
    let capacity = Capacities {
        rows: [2; N],
        bytes: [32; N],
    };
    let portrait = ADDRESS + CHARACTERS[0] + ccharacter::PORTRAITS;
    let memory = FaultMemory {
        memory: SliceMemory {
            base: ADDRESS,
            bytes: &bytes,
        },
        fault: portrait..portrait + 1,
        failures: Cell::new(0),
    };
    let mut captured = Captured::with_capacities(&capacity);
    let results = walk_family(&memory, IMAGE, ADDRESS, captured.family(), None, None);
    for result in results {
        assert!(matches!(result.outcome, Outcome::Rows { .. }));
    }
    assert_eq!(memory.failures.get(), 0);
}

#[test]
fn active_abilities_grow_every_row_and_token_past_the_default_storage() {
    let _globals = crate::extended::characters::test_globals();
    use engine_land_inputs::cability;
    use engine_native::cgame_state;
    const COUNTRIES: u32 = 5000;
    const GAME_STATE: usize = 0x1000;
    const CTAB: usize = 0x10000;
    const LTAB: usize = 0x20000;
    const COUNTRY0: usize = 0x30000;
    const STRIDE: usize = 0x50;
    const RECORDS: usize = 0x100000;
    const DEFINITION: usize = 0x180000;
    let put = |bytes: &mut Vec<u8>, at: usize, value: &[u8]| {
        bytes[at..at + value.len()].copy_from_slice(value)
    };
    let header = |bytes: &mut Vec<u8>, at: usize, data: usize, count: u32| {
        bytes[at..at + 8].copy_from_slice(&(data as u64).to_le_bytes());
        bytes[at + 8..at + 12].copy_from_slice(&count.to_le_bytes());
        bytes[at + 12..at + 16].copy_from_slice(&count.to_le_bytes());
    };
    let mut bytes = vec![0; 0x190000];
    header(
        &mut bytes,
        GAME_STATE + cgame_state::COUNTRIES,
        CTAB,
        COUNTRIES + 1,
    );
    header(
        &mut bytes,
        GAME_STATE + cgame_state::COUNTRY_LINKS,
        LTAB,
        COUNTRIES + 1,
    );
    for slot in 1..=COUNTRIES {
        let country = COUNTRY0 + (slot as usize - 1) * STRIDE;
        let record = RECORDS + (slot as usize - 1) * active_ability_record::STRIDE;
        put(
            &mut bytes,
            CTAB + slot as usize * 8,
            &(country as u64).to_le_bytes(),
        );
        put(&mut bytes, LTAB + slot as usize * 4, &slot.to_le_bytes());
        put(
            &mut bytes,
            country,
            &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, country + ccountry::ID, &slot.to_le_bytes());
        header(&mut bytes, country + ccountry::ACTIVE_ABILITIES, record, 1);
        put(
            &mut bytes,
            record + active_ability_record::DEFINITION,
            &(DEFINITION as u64).to_le_bytes(),
        );
    }
    put(
        &mut bytes,
        DEFINITION,
        &(cability::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(&mut bytes, DEFINITION + cability::TOKEN, b"ability");
    put(
        &mut bytes,
        DEFINITION + cability::TOKEN_LENGTH,
        &7_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        DEFINITION + cability::TOKEN_CAPACITY,
        &15_u64.to_le_bytes(),
    );
    assert!(COUNTRY0 + COUNTRIES as usize * STRIDE + ccountry::ACTIVE_ABILITIES < RECORDS);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let defaults = super::super::active_abilities::Capacities::default();
    let (mut row_capacity, mut byte_capacity) = (defaults.rows, defaults.bytes);
    let total = COUNTRIES as usize;
    assert!(total > row_capacity && total * 7 > byte_capacity);
    let mut rows = vec![active_ability_record::Row::default(); row_capacity];
    let mut arena = vec![0; byte_capacity];
    let first = walk_abilities(&memory, 0, GAME_STATE, &mut rows, &mut arena);
    assert_eq!(first.outcome, Outcome::Rows { len: total });
    assert_eq!(first.required_bytes, total * 7);
    collector_primitives::table_capture::grow_capacity(&mut row_capacity, total);
    collector_primitives::table_capture::grow_capacity(&mut byte_capacity, first.required_bytes);
    let mut rows = vec![active_ability_record::Row::default(); row_capacity];
    let mut arena = vec![0; byte_capacity];
    engine_native::read_counts::ReadCounts::take_current();
    let second = walk_abilities(
        &engine_native::read_counts::CountedMemory::new(memory),
        0,
        GAME_STATE,
        &mut rows,
        &mut arena,
    );
    let with_records = engine_native::read_counts::ReadCounts::take_current();
    assert_eq!(second.outcome, Outcome::Rows { len: total });
    assert_eq!(
        (second.bytes_len, second.required_bytes),
        (total * 7, total * 7)
    );
    for (index, row) in rows[..total].iter().enumerate() {
        assert_eq!((row.country, row.ordinal), (index as u32 + 1, 0));
        assert_eq!((row.token_start, row.token_len), (index * 7, 7));
        assert_eq!(&arena[row.token_start..row.token_start + 7], b"ability");
    }
    for slot in 1..=COUNTRIES {
        let country = COUNTRY0 + (slot as usize - 1) * STRIDE;
        header(&mut bytes, country + ccountry::ACTIVE_ABILITIES, 0, 0);
    }
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let empty = walk_abilities(
        &engine_native::read_counts::CountedMemory::new(memory),
        0,
        GAME_STATE,
        &mut rows,
        &mut arena,
    );
    let countries_only = engine_native::read_counts::ReadCounts::take_current();
    assert_eq!(empty.outcome, Outcome::Rows { len: 0 });
    let calls =
        |counts: engine_native::read_counts::ReadCounts| counts.copy_calls + counts.scattered_calls;
    let countries = u64::from(COUNTRIES);
    assert_eq!(calls(countries_only), 2 + 7 * countries);
    assert_eq!(calls(with_records) - calls(countries_only), 3 * countries);
}

#[test]
fn byte_capacity_stops_the_prefix_and_retains_exact_omitted_count() {
    let _globals = crate::extended::characters::test_globals();
    let mut rows = vec![cunit_leader::TraitRow::default(); 3];
    let mut bytes = vec![0; 2];
    let mut output = Batch::new(&mut rows, &mut bytes);
    output.append(
        |mut rows, mut bytes| {
            rows[0].token_len = 2;
            bytes.copy_from_slice(b"ok");
            ReadResult {
                outcome: Outcome::Truncated {
                    len: 1,
                    unread_rows: 4,
                },
                bytes_len: 2,
                required_bytes: 9,
            }
        },
        |row, offset| relocate(&mut row.token_start, offset),
    );
    output.append(
        |rows, bytes| {
            assert!(rows.is_empty());
            assert!(bytes.is_empty());
            ReadResult::rows(Outcome::Truncated {
                len: 0,
                unread_rows: 7,
            })
        },
        |_, _| unreachable!(),
    );
    let result = output.result();
    assert_eq!(
        result.outcome,
        Outcome::Truncated {
            len: 1,
            unread_rows: 11
        }
    );
    assert_eq!((result.bytes_len, result.required_bytes), (2, 9));
}

#[cfg(test)]
fn small_capacities() -> Capacities {
    Capacities {
        rows: [2; N],
        bytes: [16; N],
    }
}

#[test]
fn ability_conversion_retains_presence_native_widths_and_literal_bytes() {
    let _globals = crate::extended::characters::test_globals();
    let native = active_ability_record::Row {
        country: u32::MAX,
        ordinal: u32::MAX,
        scope_country: 0x8000_0000,
        token_start: 1,
        token_len: 2,
        caster: Some(leader_reference(u32::MAX)),
        strategic_region: Some(u32::MAX),
        division_count: i32::MIN,
        activation_hour: i32::MIN,
        phase: 0xff,
        allocate_cost: 0xfe,
        allocated_command_power: i64::MIN,
        duration_hours: i32::MAX,
        cooldown_hours: i32::MIN,
        target_kind: i32::MIN,
    };
    let replay = ability_row(&native, b"x\xff\x80");
    assert_eq!(replay.token, b"\xff\x80");
    assert_eq!(replay.caster, Some(0xffff_ffff_0000_1269));
    assert_eq!(replay.strategic_region, Some(u32::MAX));
    assert_eq!(replay.division_count, i32::MIN);
    assert_eq!(replay.allocated_command_power, i64::MIN);
    assert_eq!(replay.phase, 0xff);
    assert_eq!(replay.allocate_cost, 0xfe);
    assert_eq!(replay.key(), u64::MAX as u128);
    let absent = ability_row(
        &active_ability_record::Row {
            caster: None,
            strategic_region: None,
            division_count: 1,
            allocated_command_power: -1,
            ..native
        },
        b"x\xff\x80",
    );
    assert_eq!(absent.caster, None);
    assert_eq!(absent.strategic_region, None);
    assert_eq!(
        (absent.division_count, absent.allocated_command_power),
        (1, -1)
    );
}

#[test]
fn conversion_rejects_bad_ranges_and_duplicate_keys() {
    let _globals = crate::extended::characters::test_globals();
    let mut captured = Captured::with_capacities(&small_capacities());
    captured.characters.rows.rows[0].id = u32::MAX;
    captured.characters.rows.rows[1] = captured.characters.rows.rows[0];
    captured.characters.rows.outcome = Outcome::Rows { len: 2 };
    captured.leaders.rows.rows[0].character = u32::MAX;
    captured.leaders.rows.rows[0].ideology = i32::MIN;
    captured.leaders.rows.rows[1] = captured.leaders.rows.rows[0];
    captured.leaders.rows.outcome = Outcome::Rows { len: 2 };
    captured.advisors.rows.rows[0].character = u32::MAX;
    captured.advisors.rows.rows[1] = captured.advisors.rows.rows[0];
    captured.advisors.rows.outcome = Outcome::Rows { len: 2 };
    let views = captured.tables();
    for view in &views[..3] {
        assert!(matches!(view, TableView::Unavailable { .. }));
    }
}

const COST_CHARACTERS: usize = 32;
const COST_LEADER_STEP: usize = 7;
const COST_ADVISORS_PER: usize = 2;
const COST_UNIT_STEP: usize = 11;
const COST_TRAITS: u32 = 2;
const COST_EXPERIENCE: u32 = 15;
const COST_LITERAL: &[u8] = b"seventeen_bytes__";

const COST_MANAGER: usize = 0x1000;
const COST_ROSTER: usize = 0x2000;
const COST_CHARACTER: usize = 0x4000;
const COST_LEADER_HEAD: usize = 0x8000;
const COST_ADVISOR_HEAD: usize = 0x9000;
const COST_LEADER_NODE: usize = 0xa000;
const COST_ADVISOR_NODE: usize = 0xb000;
const COST_LEADER_ROLE: usize = 0xd000;
const COST_LEADER_ROLE_STRIDE: usize = 0x200;
const COST_ADVISOR_ROLE: usize = 0x12000;
const COST_ADVISOR_ROLE_STRIDE: usize = 0x400;
const COST_UNIT: usize = 0x24000;
const COST_DEFINITION: usize = 0x28000;
const COST_TRAIT_ARRAY: usize = 0x2c000;
const COST_EXPERIENCE_ARRAY: usize = 0x2d000;
const COST_HEAP: usize = 0x2e000;

struct CostHeap {
    next: usize,
}

impl CostHeap {
    fn write(&mut self, bytes: &mut [u8], header: usize) {
        let body = self.next;
        self.next += 32;
        put(bytes, header, ((ADDRESS + body) as u64).to_le_bytes());
        put(
            bytes,
            header + 16,
            (COST_LITERAL.len() as u64).to_le_bytes(),
        );
        put(
            bytes,
            header + 24,
            (COST_LITERAL.len() as u64).to_le_bytes(),
        );
        bytes[body..body + COST_LITERAL.len()].copy_from_slice(COST_LITERAL);
        bytes[body + COST_LITERAL.len()] = 0;
    }
}

fn cost_tree(bytes: &mut [u8], field: usize, head: usize, node: usize, stride: usize, count: u32) {
    pointer(bytes, field, head);
    put(bytes, field + 8, u64::from(count).to_le_bytes());
    bytes[head + 0x19] = 1;
    if count == 0 {
        for link in [0, 8, 16] {
            pointer(bytes, head + link, head);
        }
        return;
    }
    let last = node + (count as usize - 1) * stride;
    pointer(bytes, head, node);
    pointer(bytes, head + 8, node);
    pointer(bytes, head + 16, last);
    for index in 0..count as usize {
        let current = node + index * stride;
        pointer(bytes, current, head);
        pointer(
            bytes,
            current + 8,
            if index == 0 { head } else { current - stride },
        );
        pointer(
            bytes,
            current + 16,
            if index + 1 == count as usize {
                head
            } else {
                current + stride
            },
        );
    }
}

pub fn cost_fixture() -> Vec<u8> {
    use engine_native::character_advisor_node;
    use engine_native_politics::character_leader_node;
    let mut bytes = vec![0; 0x32000];
    let mut heap = CostHeap { next: COST_HEAP };
    pointer(&mut bytes, cgame_state::CHARACTER_MANAGER, COST_MANAGER);
    put(
        &mut bytes,
        COST_MANAGER,
        ((IMAGE + ccharacter_manager::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    array(
        &mut bytes,
        COST_MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
        COST_ROSTER,
        COST_CHARACTERS as u32,
    );
    for definition in 0..4 {
        let at = COST_DEFINITION + definition * 0x1000;
        put(
            &mut bytes,
            at,
            ((IMAGE + cunit_leader_trait::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            at + cunit_leader_trait::TOKEN_ID,
            (definition as i32).to_le_bytes(),
        );
        put(
            &mut bytes,
            at + cunit_leader_trait::ORDINAL,
            (definition as i32).to_le_bytes(),
        );
        heap.write(&mut bytes, at + cunit_leader_trait::TOKEN);
    }
    for index in 0..COST_CHARACTERS {
        let character = COST_CHARACTER + index * 0x100;
        pointer(&mut bytes, COST_ROSTER + index * 8, character);
        put(
            &mut bytes,
            character,
            ((IMAGE + ccharacter::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            character + ccharacter::TYPE,
            ccharacter::REFERENCE_TYPE.to_le_bytes(),
        );
        put(
            &mut bytes,
            character + ccharacter::ID,
            (index as u32 + 1).to_le_bytes(),
        );
        put(
            &mut bytes,
            character + ccharacter::COUNTRY,
            (index as u32 % 7 + 1).to_le_bytes(),
        );
        heap.write(&mut bytes, character + ccharacter::NAME);
        heap.write(&mut bytes, character + ccharacter::TOKEN);
        let leaders = u32::from(index % COST_LEADER_STEP == 0);
        cost_tree(
            &mut bytes,
            character + ccharacter::LEADER_MAP_HEAD,
            COST_LEADER_HEAD + index * 0x40,
            COST_LEADER_NODE + index * 0x40,
            character_leader_node::SIZE,
            leaders,
        );
        if leaders != 0 {
            let node = COST_LEADER_NODE + index * 0x40;
            let role = COST_LEADER_ROLE + index * COST_LEADER_ROLE_STRIDE;
            put(
                &mut bytes,
                node + character_leader_node::IDEOLOGY,
                (index as i32).to_le_bytes(),
            );
            pointer(&mut bytes, node + character_leader_node::LEADER, role);
            put(
                &mut bytes,
                role,
                ((IMAGE + ccountry_leader::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            pointer(&mut bytes, role + ccountry_leader::CHARACTER, character);
        }
        let advisors = if index % 3 == 1 {
            COST_ADVISORS_PER as u32
        } else {
            0
        };
        cost_tree(
            &mut bytes,
            character + ccharacter::ADVISOR_MAP_HEAD,
            COST_ADVISOR_HEAD + index * 0x40,
            COST_ADVISOR_NODE + index * 0x100,
            character_advisor_node::SIZE,
            advisors,
        );
        for advisor in 0..advisors as usize {
            let node = COST_ADVISOR_NODE + index * 0x100 + advisor * character_advisor_node::SIZE;
            let role = COST_ADVISOR_ROLE
                + (index * COST_ADVISORS_PER + advisor) * COST_ADVISOR_ROLE_STRIDE;
            put(
                &mut bytes,
                node + character_advisor_node::KEY_HASH,
                (advisor as u32).to_le_bytes(),
            );
            heap.write(&mut bytes, node + character_advisor_node::KEY);
            pointer(&mut bytes, node + character_advisor_node::ADVISOR, role);
            put(
                &mut bytes,
                role,
                ((IMAGE + cadvisor::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            pointer(&mut bytes, role + cadvisor::CHARACTER, character);
            heap.write(&mut bytes, role + cadvisor::IDEA_TOKEN);
            heap.write(&mut bytes, role + cadvisor::SLOT);
        }
        if index % COST_UNIT_STEP != 2 {
            continue;
        }
        let slot = index / COST_UNIT_STEP;
        let unit = COST_UNIT + slot * 0x1000;
        pointer(&mut bytes, character + ccharacter::UNIT_LEADER, unit);
        put(
            &mut bytes,
            unit,
            ((IMAGE + cunit_leader::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            unit + cunit_leader::TYPE,
            cunit_leader::REFERENCE_TYPE.to_le_bytes(),
        );
        put(
            &mut bytes,
            unit + cunit_leader::ID,
            (index as u32 + 1).to_le_bytes(),
        );
        put(
            &mut bytes,
            unit + cunit_leader::CHARACTER_TYPE,
            ccharacter::REFERENCE_TYPE.to_le_bytes(),
        );
        put(
            &mut bytes,
            unit + cunit_leader::CHARACTER_ID,
            (index as u32 + 1).to_le_bytes(),
        );
        put(
            &mut bytes,
            unit + cunit_leader::EXPERIENCE,
            (index as i64).to_le_bytes(),
        );
        heap.write(&mut bytes, unit + cunit_leader::TOKEN);
        heap.write(&mut bytes, unit + cunit_leader::NAME);
        let traits = COST_TRAIT_ARRAY + slot * 0x100;
        array(
            &mut bytes,
            unit + cunit_leader::TRAIT_DATA,
            traits,
            COST_TRAITS,
        );
        for entry in 0..COST_TRAITS as usize {
            pointer(
                &mut bytes,
                traits + entry * 8,
                COST_DEFINITION + (entry % 4) * 0x1000,
            );
        }
        let experience = COST_EXPERIENCE_ARRAY + slot * 0x400;
        array(
            &mut bytes,
            unit + cunit_leader::TRAIT_EXPERIENCE_DATA,
            experience,
            COST_EXPERIENCE,
        );
        for entry in 0..COST_EXPERIENCE as usize {
            pointer(
                &mut bytes,
                experience + entry * 16,
                COST_DEFINITION + (entry % 4) * 0x1000,
            );
            put(
                &mut bytes,
                experience + entry * 16 + 8,
                (entry as i64).to_le_bytes(),
            );
        }
    }
    bytes
}

#[cfg(test)]
struct CostMemory<'a> {
    memory: SliceMemory<'a>,
    guards: Cell<usize>,
    spans: Cell<usize>,
    bytes: Cell<usize>,
    prefetched: std::cell::RefCell<Vec<Vec<(usize, usize)>>>,
}

#[cfg(test)]
impl<'a> CostMemory<'a> {
    #[cfg(test)]
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            memory: SliceMemory {
                base: ADDRESS,
                bytes,
            },
            guards: Cell::new(0),
            spans: Cell::new(0),
            bytes: Cell::new(0),
            prefetched: Default::default(),
        }
    }

    #[cfg(test)]
    fn span(&self, address: usize, output: &mut [u8]) -> bool {
        self.spans.set(self.spans.get() + 1);
        self.bytes.set(self.bytes.get() + output.len());
        self.memory.copy(address, output)
    }
}

#[cfg(test)]
impl Memory for CostMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.guards.set(self.guards.get() + 1);
        self.span(address, output)
    }

    fn copy_scattered<const M: usize>(
        &self,
        addresses: [usize; M],
        outputs: &mut [&mut [u8]; M],
    ) -> usize {
        if outputs.iter().any(|output| !output.is_empty()) {
            self.guards.set(self.guards.get() + 1);
        }
        for (index, (address, output)) in addresses.into_iter().zip(outputs).enumerate() {
            if output.is_empty() {
                continue;
            }
            if address == 0
                || address.checked_add(output.len()).is_none()
                || !self.span(address, output)
            {
                return index;
            }
        }
        M
    }

    fn copy_scattered_prefetched<const M: usize>(
        &self,
        addresses: [usize; M],
        outputs: &mut [&mut [u8]; M],
    ) -> usize {
        self.prefetched.borrow_mut().push(
            addresses
                .iter()
                .zip(outputs.iter())
                .map(|(&address, output)| (address, output.len()))
                .collect(),
        );
        self.copy_scattered(addresses, outputs)
    }
}

#[test]
fn character_nested_prefetch_selects_fixed_groups_without_rehinting_main_or_heap_scatter() {
    let _globals = crate::extended::characters::test_globals();
    let mut bytes = fixture();
    let heap = 0x8200;
    pointer(&mut bytes, DEFINITION + cunit_leader_trait::TOKEN, heap);
    put(
        &mut bytes,
        DEFINITION + cunit_leader_trait::TOKEN + msvc_string::LENGTH,
        17_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        DEFINITION + cunit_leader_trait::TOKEN + msvc_string::CAPACITY,
        31_u64.to_le_bytes(),
    );
    bytes[heap..heap + 17].fill(0xff);
    let source = CostMemory::new(&bytes);
    let mut job = small_wave_job();
    assert!(copy_view(&mut job, &source, IMAGE, ADDRESS + CHARACTERS[0]).is_some());
    assert!(source.prefetched.borrow().is_empty());
    let definition = job
        .traits
        .definitions
        .header(&source, IMAGE, ADDRESS + DEFINITION)
        .unwrap();
    job.traits.definitions.result(definition).unwrap();
    job.traits
        .definitions
        .consume(&source, definition)
        .unwrap();
    job.traits.definitions.result(definition).unwrap();
    assert_eq!(source.prefetched.borrow().len(), 1);

    let node = [0x1000, 0x1040, 0x1080].map(|offset| ADDRESS + offset);
    let role = [0x1100, 0x1140, 0x1180, 0x11c0].map(|offset| ADDRESS + offset);
    let unit = [
        cunit_leader::CHARACTER_TYPE,
        cunit_leader::CATEGORY,
        cunit_leader::EXPERIENCE,
        cunit_leader::SKILL,
    ]
    .map(|offset| ADDRESS + UNITS[0] + offset);
    let mut dead = false;
    assert!(
        take_advisor_fields(&mut job, &source, node.map(Some), [8, 32, 4], &mut dead).is_some()
    );
    assert!(
        take_advisor_fields(&mut job, &source, role.map(Some), [8, 8, 32, 32], &mut dead).is_some()
    );
    assert!(
        take_fixed_fields::<4, false>(
            &mut job,
            &source,
            unit.map(Some),
            [8, 4, 8, 8],
            MASK_UNITS,
            &mut dead,
        )
        .iter()
        .all(Option::is_some)
    );
    assert!(!dead);
    assert_eq!(
        *source.prefetched.borrow(),
        [
            vec![
                (ADDRESS + DEFINITION, 12),
                (ADDRESS + DEFINITION + cunit_leader_trait::ORDINAL, 4),
                (ADDRESS + DEFINITION + cunit_leader_trait::TOKEN, 32),
            ],
            node.into_iter().zip([8, 32, 4]).collect(),
            role.into_iter().zip([8, 8, 32, 32]).collect(),
            unit.into_iter().zip([8, 4, 8, 8]).collect(),
        ]
    );
    assert_eq!(
        (source.guards.get(), source.spans.get(), source.bytes.get()),
        (6, 24, 346 + 12 + 4)
    );
}

#[test]
fn character_family_read_cost_stays_at_its_measured_shape() {
    let _globals = crate::extended::characters::test_globals();
    let bytes = cost_fixture();
    let capacity = Capacities {
        rows: [4096; N],
        bytes: [131_072; N],
    };
    let mut captured = Captured::with_capacities(&capacity);
    let memory = CostMemory::new(&bytes);
    let results = walk_family(&memory, IMAGE, ADDRESS, captured.family(), None, None);
    assert_eq!(
        results.map(|result| result.outcome),
        [
            Outcome::Rows { len: 32 },
            Outcome::Rows { len: 5 },
            Outcome::Rows { len: 22 },
            Outcome::Rows { len: 3 },
            Outcome::Rows { len: 6 },
            Outcome::Rows { len: 3 },
        ]
    );
    assert!(captured.leaders.rows.rows[..5]
        .iter()
        .all(|row| row.current_ideology == Some(None) && row.current_ideology_failure.is_none()));
    assert!(captured.advisors.rows.rows[..22].iter().all(|row| {
        row.modifier_recorded
            && row.modifier_len == 0
            && row.tooltip_recorded
            && row.tooltip_len == 0
            && row.tooltip_failure.is_none()
    }));
    assert_eq!(
        (memory.guards.get(), memory.spans.get(), memory.bytes.get()),
        (
            447 + 3 + 22 + 5 + 22,
            928 + 9 + 32 + 3 + 22 + 5 + 22 - 3,
            13_086 + 39 + 32 * (12 + 4) + 3 * 4 + 22 * 16 + 5 * 8 + 22 * 17 - 3 * 8
        )
    );
}

#[test]
fn primary_growth_keeps_shared_outputs_independently_bounded() {
    let _globals = crate::extended::characters::test_globals();
    let mut capacities = small_capacities();
    let mut captured = Captured::with_capacities(&capacities);
    captured.traits.rows.outcome = Outcome::Truncated {
        len: 1,
        unread_rows: 19,
    };
    captured.traits.bytes.required = 80;
    captured.grow(&mut capacities);
    assert_eq!(captured.traits.rows.rows.len(), 32);
    assert_eq!(captured.traits.bytes.first.len(), 128);
    let before = captured.allocated_bytes();
    captured.reset();
    captured.grow(&mut capacities);
    assert_eq!(captured.allocated_bytes(), before);
    let idle = Captured::with_capacities(&capacities);
    assert_eq!(idle.allocated_bytes(), captured.allocated_bytes());
}

#[cfg(test)]
fn split_capacities() -> Capacities {
    Capacities {
        rows: [4096; N],
        bytes: [131_072; N],
    }
}

#[test]
fn a_manager_character_with_no_country_and_no_name_is_recorded_in_its_roster_position() {
    let _globals = crate::extended::characters::test_globals();
    use super::characters_test_kit as kit;
    let mut bytes = kit::mapped_fixture();
    put(
        &mut bytes,
        CHARACTERS[0] + ccharacter::COUNTRY,
        0_u32.to_le_bytes(),
    );
    literal(&mut bytes, CHARACTERS[0] + ccharacter::TOKEN, b"");
    let capacities = split_capacities();
    let direct = kit::direct(&kit::memory(&bytes), &capacities);
    assert_eq!(direct.characters.rows.outcome, Outcome::Rows { len: 3 });
    let rows = &direct.characters.rows.rows[..3];
    assert_eq!(
        (rows[0].id, rows[0].country, rows[0].token_len),
        (u32::MAX, 0, 0)
    );
    assert_eq!(
        rows.iter()
            .map(|row| (row.roster, row.ordinal))
            .collect::<Vec<_>>(),
        [(0, 0), (0, 1), (0, 2)]
    );
    assert_eq!(rows[1].country, 2);
    assert_eq!(direct.advisors.rows.outcome, Outcome::Rows { len: 1 });
    let advisor = &direct.advisors.rows.rows[0];
    assert_eq!((advisor.character, advisor.country), (u32::MAX, 0));
    let (split, _, _) = kit::split(&kit::memory(&bytes), &capacities);
    kit::assert_matches_direct(&split, &direct);
}

#[test]
fn operative_codenames_match_raw_and_typed_capture_with_heap_faults_and_growth() {
    let _globals = crate::extended::characters::test_globals();
    use super::characters_test_kit as kit;
    use engine_native::cname_group_member as naming;
    let mut bytes = kit::mapped_fixture();
    put(
        &mut bytes,
        0x2300 + ccharacter::UNIT_LEADER,
        0_u64.to_le_bytes(),
    );
    let group = b"long-operative-group-key";
    let custom = b"player-operative-rename";
    for (index, unit) in UNITS.into_iter().enumerate() {
        put(
            &mut bytes,
            unit,
            ((IMAGE + coperative_leader::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            unit + cunit_leader::CATEGORY,
            3_i32.to_le_bytes(),
        );
        let member = unit + coperative_leader::CODENAME;
        bytes[member..member + naming::INPUT_SPAN].fill(0);
        put(
            &mut bytes,
            member,
            ((IMAGE + naming::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(&mut bytes, member + naming::TYPE, 2_i32.to_le_bytes());
        put(
            &mut bytes,
            member + naming::ORDER,
            (index as i32 - 1).to_le_bytes(),
        );
        bytes[member + naming::FLAG_A] = 1;
        for (offset, payload, value) in [
            (
                naming::FIRST_NAME_TEXT,
                0x8a00 + index * 0x100,
                group.as_slice(),
            ),
            (
                naming::OVERRIDE_NAME,
                0x8a40 + index * 0x100,
                custom.as_slice(),
            ),
        ] {
            pointer(&mut bytes, member + offset, payload);
            put(
                &mut bytes,
                member + offset + 16,
                (value.len() as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                member + offset + 24,
                (value.len() as u64).to_le_bytes(),
            );
            bytes[payload..payload + value.len()].copy_from_slice(value);
        }
    }
    let capacities = split_capacities();
    let direct = kit::direct(&kit::memory(&bytes), &capacities);
    assert_eq!(direct.units.rows.outcome, Outcome::Rows { len: 2 });
    let (split, _, _) = kit::split(&kit::memory(&bytes), &capacities);
    kit::assert_matches_direct(&split, &direct);
    for (index, row) in direct.units.rows.rows[..2].iter().enumerate() {
        let captured = leader_row(row, &direct.units.bytes.first);
        let name = captured
            .codename
            .require("UnitLeaders.codename")
            .unwrap()
            .as_ref()
            .unwrap();
        assert_eq!(name.group.as_slice(), group.as_slice());
        assert_eq!(name.custom.as_slice(), custom.as_slice());
        assert_eq!(name.order, index as i32 - 1);
    }
    let mut small = capacities;
    small.bytes[3] = 24;
    let (mut split, _, _) = kit::split(&kit::memory(&bytes), &small);
    kit::assert_matches_direct(&split, &kit::direct(&kit::memory(&bytes), &small));
    assert_eq!(split.units.rows.outcome, Outcome::Rows { len: 2 });
    for (index, row) in split.units.rows.rows[..2].iter().enumerate() {
        assert_eq!(row.codename_reason, None);
        let captured = leader_row(row, &split.units.bytes.first);
        let name = captured
            .codename
            .require("UnitLeaders.codename")
            .unwrap()
            .as_ref()
            .unwrap();
        assert_eq!(name.group.as_slice(), group.as_slice());
        assert_eq!(name.custom.as_slice(), custom.as_slice());
        assert_eq!(name.order, index as i32 - 1);
    }
    assert_eq!(
        leader_row(&split.units.rows.rows[1], &split.units.bytes.first).inherited_text_a,
        b"saved2"
    );
    split.grow(&mut small);
    assert!(small.bytes[3] >= 12 + group.len() + custom.len());
    for fault in [
        UNITS[0] + coperative_leader::CODENAME,
        0x8a00,
        0x8a00 + group.len(),
        0x8a40 + custom.len(),
    ] {
        let source = WaveMemory::new(&bytes, Some(ADDRESS + fault));
        let (split, _, _) = kit::split(&source, &capacities);
        let source = WaveMemory::new(&bytes, Some(ADDRESS + fault));
        let direct = kit::direct(&source, &capacities);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(direct.units.rows.outcome, Outcome::Rows { len: 2 });
        assert!(direct.units.rows.rows[0].codename_reason.is_some());
        assert!(direct.units.rows.rows[1].codename.is_some());
        assert!(matches!(direct.traits.rows.outcome, Outcome::Rows { .. }));
    }
    put(
        &mut bytes,
        UNITS[0] + coperative_leader::CODENAME + naming::TYPE,
        1_i32.to_le_bytes(),
    );
    let (split, _, _) = kit::split(&kit::memory(&bytes), &capacities);
    let direct = kit::direct(&kit::memory(&bytes), &capacities);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(direct.units.rows.outcome, Outcome::Rows { len: 2 });
    assert_eq!(
        direct.units.rows.rows[0].codename_reason,
        Some("operative codename member is not type two")
    );
}

#[test]
fn split_copy_matches_direct_walk_on_cost_census() {
    let _globals = crate::extended::characters::test_globals();
    use super::characters_test_kit as kit;
    let bytes = cost_fixture();
    let memory = kit::memory(&bytes);
    let capacities = split_capacities();
    let direct = kit::direct(&memory, &capacities);
    let source = WaveMemory::new(&bytes, None);
    let (split, report, _) = kit::split(&source, &capacities);
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(split.characters.rows.outcome, Outcome::Rows { len: 32 });
    assert_eq!(split.leaders.rows.outcome, Outcome::Rows { len: 5 });
    assert_eq!(split.advisors.rows.outcome, Outcome::Rows { len: 22 });
    assert_eq!(split.units.rows.outcome, Outcome::Rows { len: 3 });
    assert_eq!(split.traits.rows.outcome, Outcome::Rows { len: 6 });
    let reads = source.reads.borrow().clone();
    for (rows, arena) in [(0, 0), (1, 1), (21, 64), (22, 22 * 3 * COST_LITERAL.len())] {
        let mut small = capacities;
        small.rows[2] = rows;
        small.bytes[2] = arena;
        let source = WaveMemory::new(&bytes, None);
        let (mut decoded, _, mut job) = kit::split(&source, &small);
        kit::assert_matches_direct(&decoded, &direct);
        assert_eq!(*source.reads.borrow(), reads);
        assert_eq!(job.advisor_demand.rows, 22);
        assert_eq!(job.advisor_demand.bytes, 22 * 3 * COST_LITERAL.len());
        assert!(!job.advisor_demand.overflowed);
        decoded.grow(&mut small);
        assert!(small.rows[2] >= job.advisor_demand.rows);
        assert!(small.bytes[2] >= job.advisor_demand.bytes);
        let retained = small;
        decoded.reset();
        decoded.grow(&mut small);
        assert_eq!(small, retained);
        let context = CharactersCopyContext {
            force_full: false,
            base: IMAGE,
            game_state: ADDRESS,
            active: [true; N],
        };
        assert!(copy_characters(&memory, context, &mut job, Roles::Independent).raw_ok);
        assert_eq!(job.advisor_demand.rows, 22);
        assert_eq!(job.advisor_demand.bytes, 22 * 3 * COST_LITERAL.len());
        decoded.decode_characters_payload(IMAGE, &job, &mut [0; N]);
        kit::assert_matches_direct(&decoded, &direct);
    }
}

#[test]
fn shared_trait_definitions_are_read_once_charged_and_fresh_after_recycling() {
    let _globals = crate::extended::characters::test_globals();
    let mut bytes = fixture();
    let capacities = split_capacities();
    let context = CharactersCopyContext {
        force_full: false,
        base: IMAGE,
        game_state: ADDRESS,
        active: [true; N],
    };
    let mut job = small_wave_job();
    for value in [b'x', b'y'] {
        bytes[DEFINITION + cunit_leader_trait::TOKEN] = value;
        let source = engine_native::memory::CountedMemory::new(WaveMemory::new(&bytes, None));
        let native = read_accounting::Capture::new();
        let report = copy_characters(&source, context, &mut job, Roles::Independent);
        assert!(report.raw_ok);
        let reads = source.0.reads.borrow();
        assert_eq!(
            reads
                .iter()
                .filter(|&&(address, len)| address == ADDRESS + DEFINITION && len == 12)
                .count(),
            1,
            "one current acquisition shared by four occurrences"
        );
        let counts = read_accounting::snapshot();
        let mut expected_counts = read_accounting::ZERO;
        expected_counts[TABLES[0].index()] = counts[TABLES[0].index()];
        assert_eq!(counts, expected_counts);
        let counted = counts[TABLES[0].index()];
        assert_eq!(
            counted.spans,
            reads.iter().filter(|&&(_, len)| len != 0).count() as u64
        );
        assert!(counted.copy_calls > 0 && counted.scattered_calls > 0);
        assert_eq!((counted.record_calls, counted.plan_calls), (0, 0));
        drop(reads);
        let mut captured = Captured::with_capacities(&capacities);
        captured.decode_characters_payload(IMAGE, &job, &mut [0; N]);
        let direct = characters_test_kit::direct(&characters_test_kit::memory(&bytes), &capacities);
        assert_wave_rows(&captured, &direct);
        assert_eq!(captured.traits.bytes.first[0], value);
        assert_eq!(native.finish(), counts);
    }
}

#[test]
fn shared_skills_keep_current_rows_native_costs_and_recycled_freshness() {
    let _globals = crate::extended::characters::test_globals();
    use super::characters_test_kit as kit;
    let mut bytes = kit::mapped_fixture();
    pointer(&mut bytes, UNITS[1] + cunit_leader::SKILL, kit::SKILL0);
    let capacities = split_capacities();
    let context = CharactersCopyContext {
        force_full: false,
        base: IMAGE,
        game_state: ADDRESS,
        active: [true; N],
    };
    let mut job = small_wave_job();
    for level in [-7_i32, 9] {
        put(
            &mut bytes,
            kit::SKILL0 + cunit_leader_skill::LEVEL,
            level.to_le_bytes(),
        );
        let source = engine_native::memory::CountedMemory::new(WaveMemory::new(&bytes, None));
        let report = copy_characters(&source, context, &mut job, Roles::Independent);
        assert!(report.raw_ok);
        let skill = ADDRESS + kit::SKILL0;
        for (address, len) in [
            (skill, 8),
            (skill + cunit_leader_skill::VALID, 1),
            (skill + cunit_leader_skill::LEVEL, 12),
        ] {
            assert_eq!(
                source
                    .0
                    .reads
                    .borrow()
                    .iter()
                    .filter(|&&key| key == (address, len))
                    .count(),
                1
            );
            assert_eq!(
                job.log
                    .spans
                    .iter()
                    .filter(|span| span.addr == address && span.len as usize == len)
                    .count(),
                2
            );
        }
        let mut decoded = Captured::with_capacities(&capacities);
        decoded.decode_characters_payload(IMAGE, &job, &mut [0; N]);
        kit::assert_matches_direct(&decoded, &kit::direct(&kit::memory(&bytes), &capacities));
        assert_eq!(decoded.units.rows.rows[0].native.skill_level, level);
        assert_eq!(decoded.units.rows.rows[1].native.skill_level, level);
    }
    for offset in [0, cunit_leader_skill::VALID, cunit_leader_skill::LEVEL] {
        let source = WaveMemory::new(&bytes, Some(ADDRESS + kit::SKILL0 + offset));
        let report = copy_characters(&source, context, &mut job, Roles::Independent);
        assert!(report.raw_ok);
        assert!(job.skills.iter().all(|skill| skill.pointer == 0));
        let mut decoded = Captured::with_capacities(&capacities);
        decoded.decode_characters_payload(IMAGE, &job, &mut [0; N]);
        kit::assert_matches_direct(&decoded, &kit::direct(&source, &capacities));
    }
}

#[test]
fn shared_skill_hits_keep_alias_failures_as_separate_raw_occurrences() {
    let _globals = crate::extended::characters::test_globals();
    use super::characters_test_kit as kit;
    let bytes = kit::mapped_fixture();
    let skill = ADDRESS + kit::SKILL0;
    let source = CostMemory::new(&bytes);
    let mut job = small_wave_job();
    let pointer_start = job
        .log
        .take(
            &source,
            ADDRESS + UNITS[0] + cunit_leader::SKILL,
            8,
            MASK_UNITS,
        )
        .unwrap();
    let mut dead = Dead::default();
    copy_unit_skill(&mut job, &source, IMAGE, pointer_start, &mut dead);
    assert!(!dead.units);
    assert_eq!(source.guards.get(), 2, "pointer plus first skill gather");
    let fault = WaveMemory::new(&bytes, Some(skill));
    assert!(job.log.take(&fault, skill, 8, MASK_CHARACTERS).is_none());
    let native_bytes = job.log.raw_bytes;
    copy_unit_skill(&mut job, &source, IMAGE, pointer_start, &mut dead);
    assert!(!dead.units);
    assert_eq!(source.guards.get(), 2, "cache hit makes no native calls");
    assert_eq!(job.log.raw_bytes, native_bytes);
    let owned = collector_raw::raw::Recorded::build(&job.log).unwrap();
    let mut vtable = [0; 8];
    assert!(owned.copy(skill, &mut vtable));
    assert!(!owned.copy(skill, &mut vtable));
    assert!(owned.copy(skill, &mut vtable));
    assert_eq!(
        u64::from_le_bytes(vtable),
        (IMAGE + cunit_leader_skill::VTABLE_RVA.0) as u64
    );
}

#[test]
fn empty_trait_definition_staging_grows_in_the_pass() {
    let _globals = crate::extended::characters::test_globals();
    let bytes = fixture();
    let capacities = split_capacities();
    let context = CharactersCopyContext {
        force_full: false,
        base: IMAGE,
        game_state: ADDRESS,
        active: [true; N],
    };
    let source = characters_test_kit::memory(&bytes);
    let mut job = small_wave_job();
    job.traits.definitions.entries = Vec::new();
    job.traits.definitions.update_charge();
    assert!(copy_characters(&source, context, &mut job, Roles::Independent).raw_ok);
    assert!(!job.grow_for_next());
    let mut captured = Captured::with_capacities(&capacities);
    captured.decode_characters_payload(IMAGE, &job, &mut [0; N]);
    assert_wave_rows(
        &captured,
        &characters_test_kit::direct(&source, &capacities),
    );
}

#[test]
fn split_copy_matches_direct_walk_with_skills_kinds_and_heap_names() {
    let _globals = crate::extended::characters::test_globals();
    use super::characters_test_kit as kit;
    let bytes = kit::mapped_fixture();
    let memory = kit::memory(&bytes);
    let capacities = split_capacities();
    let direct = kit::direct(&memory, &capacities);
    let (split, report, _) = kit::split(&memory, &capacities);
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(split.leaders.rows.outcome, Outcome::Rows { len: 1 });
    assert_eq!(split.leaders.rows.rows[0].ideology, 7);
    assert_eq!(split.advisors.rows.outcome, Outcome::Rows { len: 1 });
    let advisor = &split.advisors.rows.rows[0];
    assert_eq!(
        &split.advisors.bytes.first[advisor.key_start..advisor.key_start + advisor.key_len],
        b"twenty-byte-advisor!"
    );
    let units = &split.units.rows.rows;
    assert_eq!(units[0].native.skill_status, 2);
    assert_eq!(units[0].native.skill_level, 3);
    assert_eq!(units[0].native.skill_required_experience, 100);
    assert_eq!(units[0].native.skill_category, 5);
    assert_eq!(units[1].native.kind, cunit_leader::KIND_ARMY);
    assert_eq!(units[1].native.components, [11, 22, 33, 44]);
    assert_eq!(units[2].native.kind, cunit_leader::KIND_NAVY);
    assert_eq!(units[2].native.components, [51, 52, 53, 54]);
    let character = &split.characters.rows.rows[2];
    assert_eq!(
        &split.characters.bytes.first
            [character.name_start..character.name_start + character.name_len],
        b"heap-named-character!"
    );
}

#[test]
fn army_formation_suffix_is_fresh_in_prepared_copy_and_preserves_prefix_on_fault() {
    let _globals = crate::extended::characters::test_globals();
    use super::characters_test_kit as kit;
    let mut bytes = kit::mapped_fixture();
    let capacities = split_capacities();
    let active = [true, false, false, true, false, true];
    let context = CharactersCopyContext {
        force_full: false,
        base: IMAGE,
        game_state: ADDRESS,
        active,
    };
    let mut job = CharactersJob::new_part(0);
    let field = UNITS[1] + carmy_leader::FORMATION_REFERENCE;
    for (reference, blocked) in [
        (0, false),
        (0, false),
        ((7 << 32) | 53, false),
        (u64::MAX, false),
        (53, true),
        ((9 << 32) | 53, false),
    ] {
        put(&mut bytes, field, reference.to_le_bytes());
        let source = WaveMemory::new(&bytes, blocked.then_some(ADDRESS + field));
        let report = copy_characters(&source, context, &mut job, Roles::Independent);
        assert!(
            report.raw_ok,
            "a formation suffix cannot withhold the old prefix"
        );
        let mut decoded = Captured::with_capacities(&capacities);
        decoded.decode_characters_payload(IMAGE, &job, &mut [0; N]);
        let direct = kit::direct(
            &WaveMemory::new(&bytes, blocked.then_some(ADDRESS + field)),
            &capacities,
        );
        assert_eq!(decoded.units.rows.outcome, Outcome::Rows { len: 3 });
        assert_eq!(
            decoded.characters.rows.outcome,
            direct.characters.rows.outcome
        );
        assert_eq!(&decoded.units.rows.rows[..3], &direct.units.rows.rows[..3]);
        let row = &decoded.units.rows.rows[1];
        assert_eq!(row.formation_reference, (!blocked).then_some(reference));
        assert_eq!(row.formation_reason.is_some(), blocked);
        assert_eq!(row.native.components, [11, 22, 33, 44]);
        assert_eq!(
            leader_row(row, &decoded.units.bytes.first).inherited_text_a,
            b"saved2"
        );
        assert_eq!(
            leader_row(row, &decoded.units.bytes.first).formation_reference,
            replay_data::rows::Recorded::Value((!blocked).then_some(reference))
        );
        let reads = source.reads.borrow();
        assert_eq!(
            reads
                .iter()
                .filter(|&&(at, len)| at == ADDRESS + field && len == 8)
                .count(),
            1
        );
        let suffix = reads
            .iter()
            .position(|&(at, len)| at == ADDRESS + field && len == 8)
            .unwrap();
        let later_prefix = reads
            .iter()
            .position(|&(at, len)| {
                let header = ADDRESS + kit::UNIT2 + cunit_leader::NAME;
                at <= header && at + len >= header + 32
            })
            .unwrap();
        assert!(later_prefix < suffix);
        drop(reads);
        decoded.remember_templates(&mut job);
        job.prepare_read_plans();
        job.clear();
    }
    put(
        &mut bytes,
        UNITS[1] + cunit_leader::CATEGORY,
        (-1_i32).to_le_bytes(),
    );
    let source = WaveMemory::new(&bytes, Some(ADDRESS + field));
    assert!(copy_characters(&source, context, &mut job, Roles::Independent).raw_ok);
    let mut decoded = Captured::with_capacities(&capacities);
    decoded.decode_characters_payload(IMAGE, &job, &mut [0; N]);
    assert_eq!(decoded.units.rows.rows[1].formation_reference, None);
    assert_eq!(decoded.units.rows.rows[1].formation_reason, None);
    assert!(
        !source
            .reads
            .borrow()
            .iter()
            .any(|&(at, _)| at == ADDRESS + field)
    );
}

#[test]
fn split_keeps_character_and_unit_leader_registry_ids_distinct() {
    let _globals = crate::extended::characters::test_globals();
    use super::characters_test_kit as kit;
    let mut bytes = kit::mapped_fixture();
    for (index, unit) in UNITS.into_iter().enumerate() {
        put(
            &mut bytes,
            unit + cunit_leader::ID,
            (17 + index as u32).to_le_bytes(),
        );
    }
    let memory = kit::memory(&bytes);
    let capacities = split_capacities();
    let direct = kit::direct(&memory, &capacities);
    let (split, report, _) = kit::split(&memory, &capacities);
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(split.units.rows.outcome, Outcome::Rows { len: 3 });
    assert!(matches!(split.traits.rows.outcome, Outcome::Rows { len } if len > 0));
}

#[test]
fn characters_truncation_matrix_keeps_advisors_complete() {
    let _globals = crate::extended::characters::test_globals();
    use super::characters_test_kit as kit;
    let bytes = cost_fixture();
    let memory = kit::memory(&bytes);
    for (rows, byte) in [
        ([0; N], 0),
        ([1; N], 64),
        ([2; N], 64),
        ([32, 5, 22, 3, 6, 0], 131_072),
        ([33, 6, 23, 4, 7, 0], 131_072),
        ([4096; N], 131_072),
    ] {
        let capacities = Capacities {
            rows,
            bytes: [byte; N],
        };
        let mut complete = capacities;
        complete.rows[2] = complete.rows[2].max(22);
        complete.bytes[2] = complete.bytes[2].max(22 * 3 * COST_LITERAL.len());
        let direct = kit::direct(&memory, &complete);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert!(report.raw_ok, "rows={rows:?}");
        kit::assert_matches_direct(&split, &direct);
    }
    let capacities = Capacities {
        rows: [32, 5, 22, 3, 6, 0],
        bytes: [131_072; N],
    };
    let (split, _, _) = kit::split(&memory, &capacities);
    assert_eq!(split.characters.rows.outcome, Outcome::Rows { len: 32 });
    assert_eq!(split.leaders.rows.outcome, Outcome::Rows { len: 5 });
    assert_eq!(split.traits.rows.outcome, Outcome::Rows { len: 6 });
}

#[test]
fn advisor_decode_preserves_late_failure_with_small_buffers() {
    let _globals = crate::extended::characters::test_globals();
    use super::characters_test_kit as kit;
    let bytes = cost_fixture();
    let node = COST_ADVISOR_NODE + 0x100 + character_advisor_node::SIZE;
    let role = COST_ADVISOR_ROLE + (COST_ADVISORS_PER + 1) * COST_ADVISOR_ROLE_STRIDE;
    let slot = role + cadvisor::SLOT;
    let payload = u64::from_le_bytes(bytes[slot..slot + 8].try_into().unwrap()) as usize;
    for fault in [
        ADDRESS + node + msvc_tree_node::NIL,
        ADDRESS + role + cadvisor::IDEA_TOKEN,
        payload,
        payload + COST_LITERAL.len(),
    ] {
        let memory = WaveMemory::new(&bytes, Some(fault));
        let direct = kit::direct(&memory, &split_capacities());
        assert!(matches!(
            direct.advisors.rows.outcome,
            Outcome::Unavailable { .. }
        ));
        for (rows, arena) in [(0, 0), (1, 3 * COST_LITERAL.len()), (22, 1)] {
            let mut capacities = split_capacities();
            capacities.rows[2] = rows;
            capacities.bytes[2] = arena;
            let source = WaveMemory::new(&bytes, Some(fault));
            let (split, report, _) = kit::split(&source, &capacities);
            assert!(report.raw_ok);
            kit::assert_matches_direct(&split, &direct);
            assert_eq!(split.advisors.rows.rows[0], direct.advisors.rows.rows[0]);
            assert_eq!(
                source
                    .reads
                    .borrow()
                    .iter()
                    .filter(|&&(address, len)| {
                        address <= fault && fault < address.saturating_add(len)
                    })
                    .count(),
                1,
                "one original failed attempt, fault={fault:x}"
            );
        }
    }
}

#[cfg(test)]
struct FixedFieldMemory {
    failed_call: Option<usize>,
    calls: std::cell::RefCell<Vec<(usize, usize)>>,
    guards: Cell<usize>,
    prefetched: Cell<usize>,
}

#[cfg(test)]
impl FixedFieldMemory {
    #[cfg(test)]
    fn new(failed_call: Option<usize>) -> Self {
        Self {
            failed_call,
            calls: Default::default(),
            guards: Cell::new(0),
            prefetched: Cell::new(0),
        }
    }

    #[cfg(test)]
    fn observe(&self, address: usize, output: &mut [u8]) -> bool {
        let call = self.calls.borrow().len();
        self.calls.borrow_mut().push((address, output.len()));
        for (index, byte) in output.iter_mut().enumerate() {
            *byte = (call * 31 + index + address) as u8;
        }
        self.failed_call != Some(call)
    }
}

#[cfg(test)]
impl Memory for FixedFieldMemory {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.guards.set(self.guards.get() + 1);
        self.observe(address, output)
    }

    fn copy_scattered<const M: usize>(
        &self,
        addresses: [usize; M],
        outputs: &mut [&mut [u8]; M],
    ) -> usize {
        self.guards.set(self.guards.get() + 1);
        for (index, (address, output)) in addresses.into_iter().zip(outputs).enumerate() {
            if output.is_empty() {
                continue;
            }
            if address == 0
                || address.checked_add(output.len()).is_none()
                || !self.observe(address, output)
            {
                return index;
            }
        }
        M
    }

    fn copy_scattered_prefetched<const M: usize>(
        &self,
        addresses: [usize; M],
        outputs: &mut [&mut [u8]; M],
    ) -> usize {
        self.prefetched.set(self.prefetched.get() + 1);
        self.copy_scattered(addresses, outputs)
    }
}

#[cfg(test)]
fn scalar_fixed_fields<const M: usize, const CONTINUE: bool>(
    job: &mut CharactersJob,
    memory: &impl Memory,
    addresses: [Option<usize>; M],
    lengths: [usize; M],
    mask: u32,
    dead: &mut bool,
) -> [Option<usize>; M] {
    let mut starts = [None; M];
    for index in 0..M {
        starts[index] = addresses[index]
            .and_then(|address| job.log.take(memory, address, lengths[index], mask));
        if starts[index].is_none() {
            *dead = true;
            if !CONTINUE {
                break;
            }
        }
    }
    starts
}

#[test]
fn heap_literal_gather_keeps_scalar_observations_faults_and_growth() {
    let _globals = crate::extended::characters::test_globals();
    for len in [0, 1, 17, 64] {
        for span_room in 0..=4 {
            for byte_room in [0, 1, len, len + 1, 2 * (len + 1)] {
                for failed in [None, Some(0), Some(1), Some(2), Some(3)] {
                    let make_job = || CharactersJob {
                        log: collector_raw::raw::RawLog::new(span_room, byte_room),
                        ..CharactersJob::default()
                    };
                    let mut actual = make_job();
                    let mut scalar = make_job();
                    let actual_memory = FixedFieldMemory::new(failed);
                    let scalar_memory = FixedFieldMemory::new(failed);
                    for _ in 0..2 {
                        let result = copy_heap_literal_bytes(
                            &mut actual,
                            &actual_memory,
                            0x2000,
                            len,
                            MASK_CHARACTERS,
                        );
                        let expected = scalar
                            .log
                            .take(&scalar_memory, 0x2000 + len, 1, MASK_CHARACTERS)
                            .is_some()
                            && (len == 0
                                || scalar
                                    .log
                                    .take(&scalar_memory, 0x2000, len, MASK_CHARACTERS)
                                    .is_some());
                        assert_eq!(result, expected);
                        assert_eq!(actual.log.spans, scalar.log.spans);
                        assert_eq!(actual.log.bytes, scalar.log.bytes);
                        assert_eq!(actual.log.fault, scalar.log.fault);
                        assert_eq!(actual.log.raw_bytes, scalar.log.raw_bytes);
                        assert_eq!(actual.log.overflowed, scalar.log.overflowed);
                        assert_eq!(*actual_memory.calls.borrow(), *scalar_memory.calls.borrow());
                    }
                    if len != 0 && span_room == 4 && byte_room == 2 * (len + 1) && failed.is_none()
                    {
                        assert_eq!(actual_memory.guards.get(), 2);
                        assert_eq!(scalar_memory.guards.get(), 4);
                    }
                    actual.log.grow();
                    scalar.log.grow();
                    assert_eq!(actual.log.spans.capacity(), scalar.log.spans.capacity());
                    assert_eq!(actual.log.bytes.capacity(), scalar.log.bytes.capacity());
                }
            }
        }
    }
}

#[cfg(test)]
fn assert_fixed_fields_match_scalar<const M: usize, const CONTINUE: bool>(
    addresses: [Option<usize>; M],
    lengths: [usize; M],
    mask: u32,
    span_room: usize,
    byte_room: usize,
    failed_call: Option<usize>,
) -> (usize, usize) {
    let make_job = || {
        let mut job = CharactersJob {
            log: collector_raw::raw::RawLog::new(1 + span_room, 3 + byte_room),
            game_state: 0,
            ..CharactersJob::default()
        };
        let seed = SliceMemory {
            base: 0x900,
            bytes: &[17; 3],
        };
        assert_eq!(job.log.take(&seed, 0x900, 3, MASK_CHARACTERS), Some(0));
        job
    };
    let mut actual = make_job();
    let mut expected = make_job();
    let memory = FixedFieldMemory::new(failed_call);
    let scalar_memory = FixedFieldMemory::new(failed_call);
    let mut dead = false;
    let mut scalar_dead = false;
    let starts =
        take_fixed_fields::<M, CONTINUE>(&mut actual, &memory, addresses, lengths, mask, &mut dead);
    let scalar_starts = scalar_fixed_fields::<M, CONTINUE>(
        &mut expected,
        &scalar_memory,
        addresses,
        lengths,
        mask,
        &mut scalar_dead,
    );
    assert_eq!(starts, scalar_starts);
    assert_eq!(dead, scalar_dead);
    assert_eq!(actual.log.spans, expected.log.spans);
    assert_eq!(actual.log.bytes, expected.log.bytes);
    assert_eq!(actual.log.fault, expected.log.fault);
    assert_eq!(actual.log.raw_bytes, expected.log.raw_bytes);
    assert_eq!(actual.log.overflowed, expected.log.overflowed);
    assert_eq!(actual.log.has_bulk, expected.log.has_bulk);
    assert_eq!(actual.log.grow_for_next(), expected.log.grow_for_next());
    assert_eq!(*memory.calls.borrow(), *scalar_memory.calls.borrow());
    assert_eq!(
        memory.prefetched.get(),
        usize::from(
            addresses.iter().all(Option::is_some)
                && lengths.iter().filter(|&&len| len != 0).count() <= span_room
                && lengths.iter().sum::<usize>() <= byte_room
        ),
        "only a whole admitted group selects the prefetched provider"
    );
    assert_eq!(scalar_memory.prefetched.get(), 0);
    assert_eq!(actual.log.spans.capacity(), expected.log.spans.capacity());
    assert_eq!(actual.log.bytes.capacity(), expected.log.bytes.capacity());
    {
        let reader = collector_raw::raw::Recorded::build(&actual.log).unwrap();
        let independent = collector_raw::raw::Recorded::build(&actual.log).unwrap();
        let reference = collector_raw::raw::Recorded::build(&expected.log).unwrap();
        for _ in 0..2 {
            for index in (0..M).rev() {
                if let Some(address) = addresses[index] {
                    let mut copied = vec![0xa5; lengths[index]];
                    let mut other = copied.clone();
                    let mut scalar = copied.clone();
                    let result = reader.copy(address, &mut copied);
                    assert_eq!(result, independent.copy(address, &mut other));
                    assert_eq!(result, reference.copy(address, &mut scalar));
                    assert_eq!(copied, other);
                    assert_eq!(copied, scalar);
                }
            }
        }
    }
    actual.log.grow();
    expected.log.grow();
    assert_eq!(actual.log.spans.capacity(), expected.log.spans.capacity());
    assert_eq!(actual.log.bytes.capacity(), expected.log.bytes.capacity());
    (memory.guards.get(), scalar_memory.guards.get())
}

#[cfg(test)]
fn fixed_field_matrix<const M: usize, const CONTINUE: bool>(lengths: [usize; M], mask: u32) {
    let addresses = std::array::from_fn(|index| Some(0x1000 + index * 64));
    let bytes: usize = lengths.iter().sum();
    assert_eq!(
        assert_fixed_fields_match_scalar::<M, CONTINUE>(addresses, lengths, mask, M, bytes, None),
        (1, M),
    );
    for failed in std::iter::once(None).chain((0..M).map(Some)) {
        for span_room in 0..=M {
            assert_fixed_fields_match_scalar::<M, CONTINUE>(
                addresses, lengths, mask, span_room, bytes, failed,
            );
        }
        for byte_room in 0..=bytes {
            assert_fixed_fields_match_scalar::<M, CONTINUE>(
                addresses, lengths, mask, M, byte_room, failed,
            );
        }
    }
    for index in 0..M {
        for invalid in [None, Some(0), Some(0x0000_8000_0000_0000), Some(usize::MAX)] {
            let mut changed = addresses;
            changed[index] = invalid;
            for span_room in [0, index, M] {
                assert_fixed_fields_match_scalar::<M, CONTINUE>(
                    changed, lengths, mask, span_room, bytes, None,
                );
            }
        }
    }
}

#[test]
fn characters_fixed_gathers_match_scalar_at_every_field_fault_and_capacity() {
    let _globals = crate::extended::characters::test_globals();
    fixed_field_matrix::<3, false>([12, 4, 32], MASK_TRAITS);
    fixed_field_matrix::<4, false>([8, 4, 8, 8], MASK_UNITS);
    fixed_field_matrix::<4, false>([4; 4], MASK_UNITS);
    fixed_field_matrix::<2, false>([8; 2], MASK_TRAIT_FALLBACK);
    fixed_field_matrix::<3, true>([8, 1, 12], MASK_UNITS);
    fixed_field_matrix::<2, true>([32; 2], MASK_UNITS);
}

#[test]
fn characters_fixed_gathers_preserve_changing_repeated_observations_and_faults() {
    let _globals = crate::extended::characters::test_globals();
    for failed in [None, Some(0), Some(1), Some(2), Some(3)] {
        for span_room in 0..=4 {
            for byte_room in [0, 7, 8, 15, 16, 23, 24, 31, 32] {
                assert_fixed_fields_match_scalar::<4, false>(
                    [Some(0x1000); 4],
                    [8; 4],
                    MASK_UNITS,
                    span_room,
                    byte_room,
                    failed,
                );
                assert_fixed_fields_match_scalar::<4, true>(
                    [Some(0x1000); 4],
                    [8; 4],
                    MASK_UNITS,
                    span_room,
                    byte_room,
                    failed,
                );
            }
        }
    }
}

#[test]
fn characters_fixed_gathers_keep_family_rows_and_faults_exact() {
    let _globals = crate::extended::characters::test_globals();
    use super::characters_test_kit as kit;
    let bytes = kit::mapped_fixture();
    let capacities = Capacities {
        rows: [8; N],
        bytes: [256; N],
    };
    let fields = [
        (UNITS[0] + cunit_leader::CHARACTER_TYPE, 8),
        (UNITS[0] + cunit_leader::CATEGORY, 4),
        (UNITS[0] + cunit_leader::SKILL, 8),
        (UNITS[0] + cunit_leader::TYPE, 8),
        (kit::SKILL0, 8),
        (kit::SKILL0 + cunit_leader_skill::VALID, 1),
        (kit::SKILL0 + cunit_leader_skill::LEVEL, 12),
        (UNITS[1] + carmy_leader::ATTACK, 4),
        (UNITS[1] + carmy_leader::DEFENSE, 4),
        (UNITS[1] + carmy_leader::PLANNING, 4),
        (UNITS[1] + carmy_leader::LOGISTICS, 4),
        (0x5200 + cnavy_leader::ATTACK, 4),
        (0x5200 + cnavy_leader::DEFENSE, 4),
        (0x5200 + cnavy_leader::MANEUVERING, 4),
        (0x5200 + cnavy_leader::COORDINATION, 4),
        (UNITS[0] + cunit_leader::NAME, 32),
        (UNITS[0] + cunit_leader::TOKEN, 32),
        (DEFINITION, 12),
        (DEFINITION + cunit_leader_trait::ORDINAL, 4),
        (DEFINITION + cunit_leader_trait::TOKEN, 32),
    ];
    for (offset, len) in fields {
        for byte in [0, len - 1] {
            let fault = ADDRESS + offset + byte;
            let memory = FaultMemory {
                memory: kit::memory(&bytes),
                fault: fault..fault + 1,
                failures: Cell::new(0),
            };
            let direct = kit::direct(&memory, &capacities);
            let mut captured = Captured::with_capacities(&capacities);
            let make_job = || CharactersJob {
                log: collector_raw::raw::RawLog::new(4096, 65_536),
                appended_roles: Vec::with_capacity(512),
                game_state: 0,
                active: [true; N],
                traits: TraitCapture::with_test_capacities(4096, 4096, 4096, 1 << 20),
                advisor_demand: AdvisorDemand::default(),
                skills: [SkillCopy::default(); 32],
                ..Default::default()
            };
            let mut first = make_job();
            let report = copy_characters(
                &memory,
                CharactersCopyContext {
                    force_full: false,
                    base: IMAGE,
                    game_state: ADDRESS,
                    active: [true; N],
                },
                &mut first,
                Roles::Independent,
            );
            assert!(report.raw_ok, "field={offset:x}/{len} byte={byte}");
            assert!(
                memory.failures.get() > 0,
                "unexercised field={offset:x}/{len}"
            );
            assert!(
                first
                    .log
                    .fault
                    .iter()
                    .chain([&first.traits.fault])
                    .any(|&fault| fault)
            );
            captured.decode_characters_payload(IMAGE, &first, &mut [0; N]);
            kit::assert_matches_direct(&captured, &direct);
        }
    }
}

#[test]
fn advisor_gathers_preserve_every_partial_fault_and_exact_prefix() {
    let _globals = crate::extended::characters::test_globals();
    use super::characters_test_kit as kit;
    let bytes = kit::mapped_fixture();
    let role_at = kit::ADV_NODE + character_advisor_node::ADVISOR;
    let role = u64::from_le_bytes(bytes[role_at..role_at + 8].try_into().unwrap()) as usize;
    let key = kit::ADV_NODE + character_advisor_node::KEY;
    let payload = u64::from_le_bytes(bytes[key..key + 8].try_into().unwrap()) as usize;
    let key_len = u64::from_le_bytes(bytes[key + 16..key + 24].try_into().unwrap()) as usize;
    let fields = [
        (ADDRESS + role_at, 8),
        (ADDRESS + kit::ADV_NODE + character_advisor_node::KEY, 32),
        (
            ADDRESS + kit::ADV_NODE + character_advisor_node::KEY_HASH,
            4,
        ),
        (role, 8),
        (role + cadvisor::CHARACTER, 8),
        (role + cadvisor::IDEA_TOKEN, 32),
        (role + cadvisor::SLOT, 32),
        (payload + key_len, 1),
        (payload, key_len),
    ];
    for (failed, &(address, len)) in fields.iter().enumerate() {
        for byte in [0, len - 1] {
            let memory = FaultMemory {
                memory: kit::memory(&bytes),
                fault: address + byte..address + byte + 1,
                failures: Cell::new(0),
            };
            let mut capacities = split_capacities();
            let direct = kit::direct(&memory, &capacities);
            capacities.rows[2] = 0;
            capacities.bytes[2] = 0;
            let (split, report, job) = kit::split(&memory, &capacities);
            kit::assert_matches_direct(&split, &direct);
            assert!(report.raw_ok);
            assert_eq!(job.log.fault, [false, false, true, false, false, false]);
            assert_eq!(memory.failures.get(), 2, "field={failed} byte={byte}");
            let observed: Vec<_> = job
                .log
                .spans
                .iter()
                .filter(|span| fields.contains(&(span.addr, span.len as usize)))
                .map(|span| (span.addr, span.len as usize, span.ok))
                .collect();
            let expected: Vec<_> = fields[..=failed]
                .iter()
                .enumerate()
                .map(|(index, &(address, len))| (address, len, index < failed))
                .collect();
            assert_eq!(observed, expected, "field={failed} byte={byte}");
        }
    }
    for len in [1_u64 << 30, u64::MAX] {
        let mut invalid = bytes.clone();
        put(&mut invalid, key + 16, len.to_le_bytes());
        put(&mut invalid, key + 24, len.to_le_bytes());
        let memory = kit::memory(&invalid);
        let mut capacities = split_capacities();
        let direct = kit::direct(&memory, &capacities);
        capacities.rows[2] = 0;
        capacities.bytes[2] = 0;
        let (split, _, _) = kit::split(&memory, &capacities);
        kit::assert_matches_direct(&split, &direct);
        assert!(split.advisors.bytes.first.is_empty());
    }
}

#[test]
fn advisor_gather_address_overflow_keeps_the_scalar_prefix_without_a_fault_read() {
    let _globals = crate::extended::characters::test_globals();
    let bytes = [7; 24];
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut job = CharactersJob {
        log: collector_raw::raw::RawLog::new(8, 64),
        game_state: 0,
        ..CharactersJob::default()
    };
    let mut dead = false;
    assert!(
        take_advisor_fields(
            &mut job,
            &memory,
            [Some(0x1000), None, Some(0x1010)],
            [8; 3],
            &mut dead,
        )
        .is_none()
    );
    assert!(dead);
    assert_eq!(job.log.bytes, [7; 8]);
    assert_eq!(job.log.spans.len(), 1);
    assert_eq!(job.log.spans[0].addr, 0x1000);
    assert!(job.log.spans[0].ok);
    assert_eq!(job.log.fault, [false; N]);
    assert!(!job.log.overflowed);
}

#[test]
fn split_copy_matches_direct_walk_on_fault_matrix() {
    let _globals = crate::extended::characters::test_globals();
    use super::characters_test_kit as kit;
    for (offset, fault_mask) in [
        (
            cgame_state::CHARACTER_MANAGER,
            [true, true, true, true, true, true],
        ),
        (ROSTER, [true, true, true, true, true, true]),
        (CHARACTERS[0], [true, true, true, true, true, true]),
        (
            CHARACTERS[0] + ccharacter::COUNTRY,
            [true, true, true, false, false, false],
        ),
        (
            CHARACTERS[0] + ccharacter::NAME_TOKEN_ID,
            [true, false, false, false, false, false],
        ),
        (
            CHARACTERS[0] + ccharacter::NAME,
            [true, false, false, false, false, false],
        ),
        (
            CHARACTERS[0] + ccharacter::LEADER_MAP_HEAD,
            [false, true, false, false, false, false],
        ),
        (
            CHARACTERS[0] + ccharacter::ADVISOR_MAP_HEAD,
            [false, false, true, false, false, false],
        ),
        (
            CHARACTERS[0] + ccharacter::UNIT_LEADER,
            [false, false, false, true, true, true],
        ),
        (kit::HEAP_AREA, [false, false, true, false, false, false]),
        (
            kit::LEAD_NODE + character_leader_node::IDEOLOGY,
            [false, true, false, false, false, false],
        ),
        (UNITS[0], [false, false, false, true, true, true]),
        (
            UNITS[0] + cunit_leader::TRAIT_DATA,
            [false, false, false, false, true, false],
        ),
        (DEFINITION, [false, false, false, false, true, true]),
        (EXPERIENCE_ARRAY, [false, false, false, false, false, false]),
        (
            EXPERIENCE_ARRAY + 16,
            [false, false, false, false, false, true],
        ),
        (kit::SKILL0, [false, false, false, true, false, true]),
        (
            kit::HEAP_AREA + 64,
            [true, false, false, false, false, false],
        ),
    ] {
        let bytes = kit::mapped_fixture();
        let fault = ADDRESS + offset;
        let memory = FaultMemory {
            memory: SliceMemory {
                base: ADDRESS,
                bytes: &bytes,
            },
            fault: fault..fault + 1,
            failures: Cell::new(0),
        };
        let capacities = split_capacities();
        let direct = kit::direct(&memory, &capacities);
        let (split, report, job) = kit::split(&memory, &capacities);
        assert!(report.raw_ok, "offset={offset:x}");
        kit::assert_matches_direct(&split, &direct);
        let actual: [bool; N] =
            std::array::from_fn(|index| job.log.fault[index] || (index == 4 && job.traits.fault));
        assert_eq!(actual, fault_mask, "offset={offset:x}");
    }
}

#[test]
fn failed_character_refresh_preserves_capacity() {
    let _globals = crate::extended::characters::test_globals();
    let mut captured = Captured::with_capacities(&small_capacities());
    captured.raw_active = [true; N];
    captured.characters.rows.outcome = Outcome::Rows { len: 1 };
    captured.characters.bytes.len = 1;
    captured.characters.bytes.required = 7;
    let bytes = captured.allocated_bytes();
    captured.fail_characters_capture([true; N], "character worker failed");
    assert!(!captured.raw_active.iter().any(|&active| active));
    for outcome in &captured.outcomes() {
        assert_eq!(
            *outcome,
            Outcome::Unavailable {
                reason: "character worker failed"
            }
        );
    }
    assert_eq!(captured.characters.bytes.len, 0);
    assert_eq!(captured.characters.bytes.required, 7);
    assert_eq!(captured.allocated_bytes(), bytes);

    captured.leaders.bytes.required = 11;
    captured.traits.bytes.required = 13;
    captured.fail_characters_capture(
        [true, false, false, true, false, true],
        "next worker failed",
    );
    assert_eq!(captured.characters.bytes.required, 7);
    assert_eq!(captured.leaders.bytes.required, 0);
    assert_eq!(captured.traits.bytes.required, 0);
    assert_eq!(
        captured.leaders.rows.outcome,
        Outcome::Unavailable {
            reason: replay_data::cadence::SKIPPED_REASON,
        }
    );
    assert_eq!(captured.allocated_bytes(), bytes);
}

#[cfg(test)]
fn serial_family_reference(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    family: Family<'_>,
) -> [ReadResult; N] {
    let Family {
        mut characters,
        mut leaders,
        mut advisors,
        mut units,
        mut traits,
        training,
    } = family;
    let result = (|| {
        let roster = ccharacter_manager::Roster::read(memory, base, game_state)?;
        let mut cursor = roster.cursor();
        loop {
            if !characters.active()
                && !leaders.active()
                && !advisors.active()
                && !units.active()
                && !traits.active()
            {
                break;
            }
            let Some(view) = cursor.next(
                memory,
                [
                    characters.active(),
                    leaders.active(),
                    units.active() || traits.active(),
                    advisors.active(),
                ],
            )?
            else {
                break;
            };
            let (character_pointer, character_id) = view.reference();
            match view.entry() {
                Ok(character) => {
                    characters.append(
                        |rows, bytes| {
                            ccharacter::walk_entry(&view, base, character, |_| None, rows, bytes)
                                .into()
                        },
                        |row, offset| {
                            relocate(&mut row.name_start, offset)
                                && relocate(&mut row.token_start, offset)
                                && relocate(&mut row.template_file_start, offset)
                        },
                    );
                    leaders.append(
                        |rows, _| {
                            ReadResult::rows(ccountry_leader::walk_entry(
                                &view, base, character, rows,
                            ))
                        },
                        |_, _| true,
                    );
                    advisors.append(
                        |rows, bytes| {
                            cadvisor::walk_prefix_entry(&view, base, character, rows, bytes).into()
                        },
                        |row, offset| {
                            relocate(&mut row.key_start, offset)
                                && relocate(&mut row.idea_start, offset)
                                && relocate(&mut row.slot_start, offset)
                                && relocate(&mut row.modifier_start, offset)
                        },
                    );
                }
                Err(reason) => {
                    characters.fail(reason);
                    leaders.fail(reason);
                    advisors.fail(reason);
                }
            }
            if !units.active() && !traits.active() {
                continue;
            }
            let pointer = character_pointer
                .checked_add(ccharacter::UNIT_LEADER)
                .and_then(|address| read::<u64>(&view, address))
                .and_then(|pointer| usize::try_from(pointer).ok());
            let Some(pointer) = pointer else {
                units.fail("character unit leader pointer unreadable");
                traits.fail("character unit leader pointer unreadable");
                continue;
            };
            if pointer == 0 {
                continue;
            }
            let previous_len = units.len;
            units.append(
                |rows, at, bytes, from| {
                    read_unit_selected(memory, base, pointer, character_id, bytes, from, training)
                        .and_then(|read| read.store(rows, at))
                },
                |row, offset| {
                    relocate(&mut row.text_a_start, offset)
                        && relocate(&mut row.text_b_start, offset)
                        && relocate(&mut row.training.0, offset)
                        && relocate(&mut row.training_factors.0, offset)
                },
            );
            let identity = if traits.active() {
                Some(if units.len > previous_len {
                    Ok(units.rows[previous_len].native.id)
                } else {
                    unit_identity(memory, pointer, character_id)
                })
            } else {
                None
            };
            match identity {
                Some(Ok(id)) => {
                    traits.append(
                        |rows, bytes| {
                            cunit_leader::walk_traits(memory, base, pointer, id, rows, bytes).into()
                        },
                        |row, offset| relocate(&mut row.token_start, offset),
                    );
                }
                Some(Err(reason)) => traits.fail(reason),
                None => {}
            }
        }
        Ok::<_, &'static str>(())
    })();
    if let Err(reason) = result {
        characters.fail(reason);
        leaders.fail(reason);
        advisors.fail(reason);
        units.fail(reason);
        traits.fail(reason);
    }
    advisors.append_tooltips(memory, 0, advisors.len);
    [
        characters.result(),
        leaders.result(),
        advisors.result(),
        units.result(),
        traits.result(),
        units.result(),
    ]
}

#[cfg(test)]
pub(super) struct WaveMemory<'a> {
    bytes: &'a [u8],
    blocked: Option<usize>,
    changing: Option<(usize, usize)>,
    sequence: Cell<u8>,
    fail_sequence: Option<u8>,
    pub(super) reads: std::cell::RefCell<Vec<(usize, usize)>>,
}

#[cfg(test)]
impl<'a> WaveMemory<'a> {
    #[cfg(test)]
    pub(super) fn new(bytes: &'a [u8], blocked: Option<usize>) -> Self {
        Self {
            bytes,
            blocked,
            changing: None,
            sequence: Cell::new(0),
            fail_sequence: None,
            reads: std::cell::RefCell::new(Vec::new()),
        }
    }
}

#[cfg(test)]
impl Memory for WaveMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        if self
            .blocked
            .is_some_and(|at| address <= at && at - address < output.len())
        {
            let prefix = output.len().min(3);
            output[..prefix].fill(0xfe);
            return false;
        }
        if !(SliceMemory {
            base: ADDRESS,
            bytes: self.bytes,
        })
        .copy(address, output)
        {
            return false;
        }
        if self.changing == Some((address, output.len())) {
            self.sequence.set(self.sequence.get().wrapping_add(1));
            if self.fail_sequence == Some(self.sequence.get()) {
                output[0] = 0xfe;
                return false;
            }
            output[0] = b'A' + self.sequence.get() % 26;
        }
        true
    }
}

#[cfg(test)]
fn serial_family_capture(memory: &impl Memory, capacities: &Capacities) -> Captured {
    let mut captured = Captured::with_capacities(capacities);
    let results = serial_family_reference(memory, IMAGE, ADDRESS, captured.family());
    captured.characters.primary(results[0]);
    captured.leaders.primary(results[1]);
    captured.advisors.primary(results[2]);
    captured.units.primary(results[3]);
    captured.traits.primary(results[4]);
    captured
}

#[cfg(test)]
fn small_wave_job() -> CharactersJob {
    CharactersJob {
        log: collector_raw::raw::RawLog::new(8192, 262_144),
        appended_roles: Vec::with_capacity(1024),
        game_state: 0,
        active: [true; N],
        traits: TraitCapture::with_test_capacities(4096, 4096, 4096, 1 << 20),
        advisor_demand: AdvisorDemand::default(),
        skills: [SkillCopy::default(); 32],
        ..Default::default()
    }
}

#[test]
fn character_family_matches_raw_and_typed_at_every_hour() {
    let _globals = crate::extended::characters::test_globals();
    let bytes = fixture();
    let capacities = Capacities {
        rows: [4; N],
        bytes: [128; N],
    };
    let private = [
        ADDRESS + CHARACTERS[0] + ccharacter::LEADER_MAP_HEAD,
        ADDRESS + CHARACTERS[0] + ccharacter::ADVISOR_MAP_HEAD,
        ADDRESS + UNITS[0] + cunit_leader::TRAIT_DATA,
        ADDRESS + UNITS[0] + cunit_leader::TRAIT_EXPERIENCE_DATA,
    ];
    for blocked in [None, Some(private[0]), Some(private[2])] {
        for hour in 0..24 {
            let active = std::array::from_fn(|index| {
                replay_data::cadence::should_walk(TABLES[index], hour, false)
            });
            let typed_memory = WaveMemory::new(&bytes, blocked);
            let mut typed = Captured::with_capacities(&capacities);
            typed.capture(&typed_memory, IMAGE, ADDRESS, hour, false);
            let raw_memory = WaveMemory::new(&bytes, blocked);
            let mut job = small_wave_job();
            let report = copy_characters(
                &raw_memory,
                CharactersCopyContext {
                    force_full: false,
                    base: IMAGE,
                    game_state: ADDRESS,
                    active,
                },
                &mut job,
                Roles::Independent,
            );
            assert!(report.raw_ok, "hour={hour} blocked={blocked:?}");
            let mut decoded = Captured::with_capacities(&capacities);
            decoded.decode_characters_payload(IMAGE, &job, &mut [0; N]);
            characters_test_kit::assert_matches_direct(&decoded, &typed);
            assert_eq!(decoded.character_activity(), active);
            assert_eq!(&active[..5], &[true; 5]);
            assert_eq!(
                active[5],
                hour == replay_data::cadence::COMMANDER_PROGRESS_SLOT as i32
            );
        }
    }
}

#[cfg(test)]
fn wave_split_once(memory: &impl Memory, capacities: &Capacities) -> Captured {
    let mut job = small_wave_job();
    copy_pass(
        memory,
        CharactersCopyContext {
            force_full: false,
            base: IMAGE,
            game_state: ADDRESS,
            active: [true; N],
        },
        &mut job,
        None,
    );
    assert!(!job.overflowed());
    let mut captured = Captured::with_capacities(capacities);
    captured.decode_characters_payload(IMAGE, &job, &mut [0; N]);
    captured
}

#[cfg(test)]
fn assert_wave_rows(left: &Captured, right: &Captured) {
    characters_test_kit::assert_matches_direct(left, right);
    fn written<R: PartialEq + Default + std::fmt::Debug>(name: &str, left: &[R], right: &[R]) {
        let len = left.len().min(right.len());
        assert_eq!(left[..len], right[..len], "{name} rows");
        let blank = R::default();
        assert!(
            left[len..].iter().chain(&right[len..]).all(|row| *row == blank),
            "{name} rows past the shorter pool"
        );
    }
    written(
        "characters",
        &left.characters.rows.rows,
        &right.characters.rows.rows,
    );
    written("leaders", &left.leaders.rows.rows, &right.leaders.rows.rows);
    written(
        "advisors",
        &left.advisors.rows.rows,
        &right.advisors.rows.rows,
    );
    written("units", &left.units.rows.rows, &right.units.rows.rows);
    written("traits", &left.traits.rows.rows, &right.traits.rows.rows);
}

#[cfg(test)]
fn check_wave_static(bytes: &[u8], blocked: Option<usize>, capacities: &Capacities) {
    let serial_memory = WaveMemory::new(bytes, blocked);
    let current_memory = WaveMemory::new(bytes, blocked);
    let raw_memory = WaveMemory::new(bytes, blocked);
    let split = wave_split_once(&raw_memory, capacities);
    let mut complete = *capacities;
    complete.rows[2] = split.advisors.rows.rows.len();
    complete.bytes[2] = split.advisors.bytes.first.len();
    let serial = serial_family_capture(&serial_memory, &complete);
    let current = characters_test_kit::direct(&current_memory, &complete);
    assert_wave_rows(&current, &serial);
    assert_wave_rows(&split, &current);
}

#[test]
fn character_page_wave_matches_serial_static_rows_capacity_and_every_view_fault() {
    let _globals = crate::extended::characters::test_globals();
    let bytes = characters_test_kit::mapped_fixture();
    let full = Capacities {
        rows: [64; N],
        bytes: [8192; N],
    };
    check_wave_static(&bytes, None, &full);
    for count in [0, 1, 2, 3] {
        for arena in [0, 1, 16, 8192] {
            check_wave_static(
                &bytes,
                None,
                &Capacities {
                    rows: [count; N],
                    bytes: [arena; N],
                },
            );
        }
    }
    for slot in 0..N {
        let mut capacity = full;
        capacity.rows[slot] = 0;
        check_wave_static(&bytes, None, &capacity);
    }
    for character in [CHARACTERS[0], CHARACTERS[1], 0x2300] {
        for (offset, width) in VIEW_OFFSETS.into_iter().zip(VIEW_LENS) {
            for fault in [
                ADDRESS + character + offset,
                ADDRESS + character + offset + width - 1,
            ] {
                check_wave_static(&bytes, Some(fault), &full);
            }
        }
    }
    check_wave_static(
        &cost_fixture(),
        None,
        &Capacities {
            rows: [128; N],
            bytes: [131_072; N],
        },
    );
    for count in [0, 33, 65] {
        let mut bytes = fixture();
        array(
            &mut bytes,
            MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
            0x8400,
            count,
        );
        array(
            &mut bytes,
            MANAGER + ccharacter_manager::DYNAMIC_CHARACTERS,
            0x8800,
            1,
        );
        pointer(&mut bytes, 0x8800, CHARACTERS[0]);
        for index in 0..count {
            pointer(&mut bytes, 0x8400 + index as usize * 8, CHARACTERS[0]);
        }
        check_wave_static(&bytes, None, &full);
        if count > 32 {
            let page_count = (count - 32).min(32) as usize;
            for fault in [
                ADDRESS + 0x8400 + 32 * 8,
                ADDRESS + 0x8400 + (32 + page_count) * 8 - 1,
            ] {
                check_wave_static(&bytes, Some(fault), &full);
            }
            for index in [32, 32 + page_count - 1] {
                let mut null = bytes.clone();
                put(&mut null, 0x8400 + index * 8, 0_u64.to_le_bytes());
                check_wave_static(&null, None, &full);
                let mut invalid = bytes.clone();
                pointer(&mut invalid, 0x8400 + index * 8, CHARACTERS[1]);
                put(&mut invalid, CHARACTERS[1], 0_u64.to_le_bytes());
                check_wave_static(&invalid, None, &full);
            }
        }
    }
}

#[test]
fn character_page_wave_defers_global_errors_and_never_revives_failed_domains() {
    let _globals = crate::extended::characters::test_globals();
    let full = Capacities {
        rows: [64; N],
        bytes: [8192; N],
    };
    for later in [1, 2] {
        for null in [false, true] {
            let mut bytes = characters_test_kit::mapped_fixture();
            put(
                &mut bytes,
                CHARACTERS[0] + ccharacter::NAME + 24,
                14_u64.to_le_bytes(),
            );
            if null {
                put(&mut bytes, ROSTER + later * 8, 0_u64.to_le_bytes());
            } else {
                let target = [CHARACTERS[0], CHARACTERS[1], 0x2300][later];
                put(&mut bytes, target, 0_u64.to_le_bytes());
            }
            check_wave_static(&bytes, None, &full);
            let result = characters_test_kit::direct(&WaveMemory::new(&bytes, None), &full);
            assert_eq!(
                result.characters.rows.outcome,
                Outcome::Unavailable {
                    reason: "character string header invalid"
                }
            );
        }
    }
    let bytes = characters_test_kit::mapped_fixture();
    let trace = WaveMemory::new(&bytes, None);
    let _ = characters_test_kit::direct(&trace, &full);
    for &(address, width) in trace.reads.borrow().iter() {
        if address >= ADDRESS + 0x2400 {
            for fault in [address, address + width - 1] {
                check_wave_static(&bytes, Some(fault), &full);
            }
        }
    }
}

#[test]
fn character_page_wave_preserves_alias_cursors_and_current_values() {
    let _globals = crate::extended::characters::test_globals();
    let full = Capacities {
        rows: [64; N],
        bytes: [8192; N],
    };
    let mut bytes = fixture();
    let alias = ADDRESS + CHARACTERS[1] + ccharacter::NAME;
    pointer(
        &mut bytes,
        CHARACTERS[0] + ccharacter::NAME,
        CHARACTERS[1] + ccharacter::NAME,
    );
    put(
        &mut bytes,
        CHARACTERS[0] + ccharacter::NAME + 16,
        64_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        CHARACTERS[0] + ccharacter::NAME + 24,
        64_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        CHARACTERS[1] + ccharacter::COUNTRY,
        0_u32.to_le_bytes(),
    );
    let mut native = WaveMemory::new(&bytes, None);
    native.changing = Some((alias, 64));
    let mut raw = WaveMemory::new(&bytes, None);
    raw.changing = Some((alias, 64));
    let direct = characters_test_kit::direct(&native, &full);
    let split = wave_split_once(&raw, &full);
    assert_wave_rows(&split, &direct);
    assert_eq!(native.sequence.get(), 2);
    assert_eq!(raw.sequence.get(), 2);
    assert_eq!(direct.characters.rows.outcome, Outcome::Rows { len: 2 });
    let rows = &direct.characters.rows.rows;
    assert_eq!(direct.characters.bytes.first[rows[0].name_start], b'C');
    assert_eq!(direct.characters.bytes.first[rows[1].name_start], b'B');

    let mut serial_memory = WaveMemory::new(&bytes, None);
    serial_memory.changing = Some((alias, 64));
    let serial = serial_family_capture(&serial_memory, &full);
    assert_eq!(
        serial.characters.bytes.first[serial.characters.rows.rows[0].name_start],
        b'B'
    );
    assert_eq!(
        serial.characters.bytes.first[serial.characters.rows.rows[1].name_start],
        b'C'
    );
    for failure in [1, 2] {
        let mut native = WaveMemory::new(&bytes, None);
        native.changing = Some((alias, 64));
        native.fail_sequence = Some(failure);
        let mut raw = WaveMemory::new(&bytes, None);
        raw.changing = Some((alias, 64));
        raw.fail_sequence = Some(failure);
        let direct = characters_test_kit::direct(&native, &full);
        assert_wave_rows(&wave_split_once(&raw, &full), &direct);
        assert_eq!((native.sequence.get(), raw.sequence.get()), (2, 2));
    }

    let mut bytes = fixture();
    pointer(&mut bytes, ROSTER + 8, CHARACTERS[0]);
    array(
        &mut bytes,
        MANAGER + ccharacter_manager::DYNAMIC_CHARACTERS,
        0x2080,
        1,
    );
    pointer(&mut bytes, 0x2080, CHARACTERS[0]);
    let key = (ADDRESS + CHARACTERS[0] + ccharacter::NAME, 64);
    let mut native = WaveMemory::new(&bytes, None);
    native.changing = Some(key);
    let mut raw = WaveMemory::new(&bytes, None);
    raw.changing = Some(key);
    let direct = characters_test_kit::direct(&native, &full);
    assert_wave_rows(&wave_split_once(&raw, &full), &direct);
    assert_eq!((native.sequence.get(), raw.sequence.get()), (3, 3));
    assert_eq!(direct.characters.rows.rows[2].roster, 1);
    assert_eq!(direct.characters.rows.rows[2].ordinal, 0);

    let mut job = small_wave_job();
    let mut decoded = Captured::with_capacities(&full);
    for value in [b'x', b'y', b'z'] {
        bytes[CHARACTERS[0] + ccharacter::NAME] = value;
        copy_pass(
            &WaveMemory::new(&bytes, None),
            CharactersCopyContext {
                force_full: false,
                base: IMAGE,
                game_state: ADDRESS,
                active: [true; N],
            },
            &mut job,
            None,
        );
        assert!(!job.log.overflowed);
        decoded.decode_characters_payload(IMAGE, &job, &mut [0; N]);
        let current = characters_test_kit::direct(&WaveMemory::new(&bytes, None), &full);
        assert_wave_rows(&decoded, &current);
    }
}
