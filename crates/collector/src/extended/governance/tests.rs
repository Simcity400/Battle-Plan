use super::*;
use engine::cnational_focus;

#[test]
fn recorded_effect_inputs_project_exact_bytes_and_keep_decision_cadence() {
    let mut capacity = Capacities::default();
    capacity.raise_for_effect_inputs(
        cpolitical_status::IdeaDemand { rows: 1, bytes: 3 },
        cdecision_status::RandomItemDemand { rows: 1, bytes: 3 },
    );
    let mut captured = Captured::with_capacities(&capacity);
    captured.timed_ideas.rows[0] = cpolitical_status::TimedIdeaRow {
        country: 5_000,
        ordinal: 7,
        token_start: 0,
        token_len: 3,
        days: i32::MIN,
    };
    captured.timed_ideas.outcome = Outcome::Rows { len: 1 };
    captured.random_items.rows[0] = cdecision_status::RandomItemRow {
        country: 5_000,
        ordinal: 3,
        token_start: 0,
        token_len: 3,
        count: i32::MAX,
        target: -1,
    };
    captured.random_items.outcome = Outcome::Rows { len: 1 };
    for text in [
        &mut captured.timed_idea_text,
        &mut captured.random_item_text,
    ] {
        text.first[..3].copy_from_slice(&[b'x', 0, 255]);
        text.len = 3;
    }
    let tables = captured.tables();
    let TableView::Complete(rows::TableRows::TimedIdeas(timed)) = &tables[23] else {
        panic!("timed ideas unavailable");
    };
    assert_eq!(timed[0].token, [b'x', 0, 255]);
    assert_eq!(
        (timed[0].country, timed[0].ordinal, timed[0].days),
        (5_000, 7, i32::MIN)
    );
    let TableView::Complete(rows::TableRows::DecisionRandomItems(random)) = &tables[24] else {
        panic!("random items unavailable");
    };
    assert_eq!(random[0].token, [b'x', 0, 255]);
    assert_eq!((random[0].count, random[0].target), (i32::MAX, u32::MAX));
    for hour in 0..24 {
        assert!(replay::cadence::should_walk(
            TableId::TimedIdeas,
            hour,
            false
        ));
        assert_eq!(
            replay::cadence::should_walk(TableId::DecisionRandomItems, hour, false),
            replay::cadence::should_walk(TableId::Decisions, hour, false)
        );
    }
}

#[test]
fn breakdown_capture_keeps_the_original_push_capacity_trace() {
    use engine::memory::SliceMemory;
    const STATE: usize = 0x100;
    const MANAGER: usize = 0x800;
    const RECORDS: usize = 0x1000;
    let mut bytes = vec![0_u8; RECORDS + 17 * power_balance_record::STRIDE];
    bytes[STATE + engine::cgame_state::POWER_BALANCE_MANAGER
        ..STATE + engine::cgame_state::POWER_BALANCE_MANAGER + 8]
        .copy_from_slice(&(MANAGER as u64).to_le_bytes());
    bytes[MANAGER + power_balance_manager::RECORDS..MANAGER + power_balance_manager::RECORDS + 8]
        .copy_from_slice(&(RECORDS as u64).to_le_bytes());
    for record in 0..17 {
        // Empty cached accumulator at +0xC8, with an empty inline +0x48 label.
        let capacity = RECORDS + record * power_balance_record::STRIDE + 0xc8 + 0x48 + 24;
        bytes[capacity..capacity + 8].copy_from_slice(&15_u64.to_le_bytes());
    }
    let mut captured = Captured::vacant();
    let mut previous = Vec::<rows::PowerBalanceModifierBreakdownRow>::new();
    for count in [1_usize, 2, 5, 9, 2, 0, 17] {
        let header = ((count as u64) << 32) | count as u64;
        bytes[MANAGER + power_balance_manager::RECORDS + 8
            ..MANAGER + power_balance_manager::RECORDS + 16]
            .copy_from_slice(&header.to_le_bytes());
        captured.breakdowns.clear();
        captured.balances.outcome = Outcome::Rows { len: count };
        captured.capture_breakdowns(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            STATE,
        );
        previous.clear();
        for _ in 0..count {
            previous.push(rows::PowerBalanceModifierBreakdownRow::default());
        }
        assert_eq!(captured.breakdown_outcome, Outcome::Rows { len: count });
        assert_eq!(captured.breakdowns.capacity(), previous.capacity());
        for (record, row) in captured.breakdowns.iter().enumerate() {
            assert_eq!(row.record, record as u32);
            assert!(row.sources.is_empty());
            for nodes in [&row.daily, &row.weekly] {
                assert_eq!(nodes.len(), 1);
                assert_eq!((nodes[0].children, nodes[0].value), (0, 0));
                assert!(nodes[0].name.is_empty());
            }
        }
    }
}

#[test]
fn incomplete_breakdown_tree_cannot_publish_a_default_row() {
    let mut captured = Captured::vacant();
    captured.breakdowns = vec![rows::PowerBalanceModifierBreakdownRow {
        record: 7,
        sources: Vec::new(),
        daily: vec![rows::ModifierBreakdownNode {
            children: 1,
            value: 11,
            name: b"missing child".to_vec(),
        }],
        weekly: vec![rows::ModifierBreakdownNode {
            children: 0,
            value: 21,
            name: b"weekly root".to_vec(),
        }],
    }];
    captured.breakdown_outcome = Outcome::Rows { len: 1 };
    assert!(
        matches!(captured.breakdowns_table(), TableView::Unavailable { reason }
        if reason == "power balance modifier breakdown copy failed")
    );
}

#[test]
fn party_leader_role_identity_survives_capture_at_country_scale() {
    use engine::{ccharacter, ccountry, ccountry_leader, cideology_group, memory::SliceMemory};
    let mut bytes = vec![0_u8; 0x3000];
    let mut word = |at: usize, value: u64| bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    word(0x100, ccountry::VTABLE_RVA as u64);
    word(0x100 + ccountry::POLITICAL_STATUS, 0x1200);
    word(0x1200, cpolitical_status::VTABLE_RVA as u64);
    word(0x1200 + cpolitical_status::COUNTRY, 0x100);
    word(0x1200 + cpolitical_status::PARTIES, 0x1400);
    word(0x1200 + cpolitical_status::PARTIES + 8, (1 << 32) | 1);
    word(0x1400, 0x1500);
    word(0x1500, cpolitical_party::VTABLE_RVA as u64);
    word(0x1500 + cpolitical_party::ID, 7);
    word(0x1500 + cpolitical_party::IDEOLOGY_GROUP, 0x1600);
    word(0x1600, cideology_group::VTABLE_RVA as u64);
    word(0x1600 + cideology_group::ID, 7);
    word(0x1500 + cpolitical_party::COUNTRY_LEADERS, 0x1700);
    word(
        0x1500 + cpolitical_party::COUNTRY_LEADERS + 8,
        (1 << 32) | 1,
    );
    word(0x1700, 0x1800);
    word(0x1800, ccountry_leader::VTABLE_RVA as u64);
    word(0x1800 + ccountry_leader::CHARACTER, 0x1a00);
    word(0x1800 + ccountry_leader::LEADER_ID, 12);
    word(0x1800 + 0x140, 0x2500); // Changed ideology; the role key is still -7.
    word(0x1a00, ccharacter::VTABLE_RVA as u64);
    word(
        0x1a00 + ccharacter::TYPE,
        (91_u64 << 32) | u64::from(ccharacter::REFERENCE_TYPE),
    );
    word(0x1a00 + ccharacter::LEADER_MAP_HEAD, 0x1c00);
    word(0x1a00 + ccharacter::LEADER_MAP_SIZE, 1);
    for offset in [0, 8, 16] {
        word(0x1c00 + offset, 0x1c40);
        word(0x1c40 + offset, 0x1c00);
    }
    word(0x1c00 + 0x18, 1 << 8);
    word(0x1c40 + 0x20, u64::from((-7_i32) as u32));
    word(0x1c40 + 0x28, 0x1800);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let reader = cpolitical_status::Reader::new(&memory, 0, 0x100).unwrap();
    let mut captured = Captured::vacant();
    captured.party_leaders = CaptureRows::new(5_000);
    for (index, destination) in captured.party_leaders.rows.iter_mut().enumerate() {
        assert_eq!(
            reader.party_leaders(index as u32 + 1, std::slice::from_mut(destination)),
            Outcome::Rows { len: 1 }
        );
    }
    captured.party_leaders.outcome = Outcome::Rows { len: 5_000 };
    let tables = captured.tables();
    let TableView::Complete(TableRows::PartyLeaders(rows)) = &tables[21] else {
        panic!("leaders missing")
    };
    assert_eq!(rows.len(), 5_000);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(
            (row.country, row.character, row.role, row.leader_id),
            (index as u32 + 1, Some(91), Some(-7), 12)
        );
    }
}

#[test]
fn hourly_research_conversion_preserves_native_inputs_and_bonus_failures() {
    let mut captured = Captured::vacant();
    let mut input = engine::research_inputs::Row::default();
    input.slot.country = 11;
    input.slot.index = 2;
    input.slot.technology_present = 1;
    input.slot.technology_handle = 77;
    input.template_index = 3;
    input.slot.database_index = 3;
    let mut token = engine::msvc_string::Text::default();
    token.bytes[..3].copy_from_slice(b"abc");
    token.len = 3;
    input.token = engine::research_inputs::Text::Inline(token);
    input.progress_raw = i64::MAX;
    input.speed_bonus_raw = i64::MIN;
    captured.research_inputs.rows.push(input);
    captured.research_inputs.outcome = Outcome::Rows { len: 1 };
    captured.research_bonuses.outcome = Outcome::Unavailable {
        reason: "bonus read failed",
    };
    let tables = captured.tables();
    let TableView::Complete(TableRows::ResearchInputs(rows)) = &tables[18] else {
        panic!("inputs missing")
    };
    assert_eq!(rows[0].index, 2);
    assert_eq!(rows[0].database_index, replay::rows::Recorded::Value(3));
    assert_eq!(rows[0].token, b"abc");
    assert_eq!(rows[0].progress_raw, i64::MAX);
    assert_eq!(rows[0].speed_bonus_raw, i64::MIN);
    assert!(matches!(&tables[19], TableView::Unavailable { .. }));
    for table in [
        TableId::ResearchInputs,
        TableId::ResearchAppliedBonuses,
        TableId::ResearchModifiers,
    ] {
        for hour in 0..8 {
            assert!(replay::cadence::should_walk(table, hour, false));
        }
    }
}

#[test]
fn hourly_modifier_conversion_preserves_identity_signed_operands_and_source_text() {
    let mut captured = Captured::vacant();
    let mut row = engine::research_modifiers::Row {
        country: 11,
        index: u32::MAX,
        ordinal: 7,
        kind: engine::research_modifiers::Kind::CountrySource,
        raw: i64::MIN,
        auxiliary: i64::MAX,
        count: u32::MAX,
        source_id: 91,
        ..Default::default()
    };
    let mut name = engine::msvc_string::Text::default();
    name.bytes[..4].copy_from_slice(b"idea");
    name.len = 4;
    row.name = engine::research_inputs::Text::Inline(name);
    captured.research_modifiers.rows.push(row);
    captured.research_modifiers.outcome = Outcome::Rows { len: 1 };
    let tables = captured.tables();
    let TableView::Complete(TableRows::ResearchModifiers(rows)) = &tables[20] else {
        panic!("modifiers missing")
    };
    assert_eq!(
        (rows[0].country, rows[0].index, rows[0].ordinal),
        (11, u32::MAX, 7)
    );
    assert_eq!(
        (rows[0].raw, rows[0].auxiliary, rows[0].count),
        (i64::MIN, i64::MAX, u32::MAX)
    );
    assert_eq!(rows[0].kind, rows::ResearchModifierRow::COUNTRY_SOURCE);
    assert_eq!(rows[0].name, b"idea");
}

#[test]
fn owned_governance_vacancy_has_no_buffer_allocation() {
    fn assert_send<T: Send>() {}
    assert_send::<Captured>();
    let captured = Captured::vacant();
    assert_eq!(captured.allocated_bytes(), 0);
    assert!(captured.dictionary.is_empty());
    assert!(captured.technology_pointers.is_empty());
    assert_eq!(captured.technology_pointers.capacity(), 0);
    assert!(!captured.focus_raw_mode);
}

#[test]
fn owned_governance_failure_preserves_cadence_and_focus_delegation() {
    for force_full in [false, true] {
        for delegated_focus in [false, true] {
            let mut captured = Captured::vacant();
            captured.focus_raw_mode = delegated_focus;
            captured.definition_text.first = vec![1, 2, 3];
            captured.definition_text.len = 3;
            captured.definition_text.required = 7;
            captured.modifier_values.first = vec![11, 12];
            captured.modifier_values.len = 2;
            captured.modifier_values.required = 4;
            captured.dictionary_required = 9;
            let context = CaptureContext {
                base: 0,
                game_state: 0,
                hour: 1,
                force_full,
            };
            let report = captured.fail_owned_capture(context, "worker stopped");
            assert_eq!(report.ticks, [0; N]);
            assert_eq!(captured.focus_raw_mode, delegated_focus);
            assert_eq!(captured.definition_text.len, 0);
            assert_eq!(captured.definition_text.required, 0);
            assert_eq!(captured.modifier_values.len, 0);
            assert_eq!(captured.modifier_values.required, 0);
            assert_eq!(captured.dictionary_required, 0);
            for (index, outcome) in captured.outcomes().into_iter().enumerate() {
                let delegated = delegated_focus && index < 2;
                let skipped =
                    !replay::cadence::should_walk(TABLES[index], context.hour, force_full);
                assert_eq!(
                    outcome,
                    Outcome::Unavailable {
                        reason: if delegated {
                            "clock unavailable"
                        } else if skipped {
                            replay::cadence::SKIPPED_REASON
                        } else {
                            "worker stopped"
                        },
                    },
                    "{index}",
                );
            }
        }
    }
}

#[test]
fn focus_failure_preserves_other_governance_tables() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.focus_raw_mode = true;
    captured.focus.outcome = Outcome::Rows { len: 1 };
    captured.completed.outcome = Outcome::Rows { len: 1 };
    captured.technologies.outcome = Outcome::Rows { len: 1 };
    let reason = "accepted acquisition worker stopped";
    captured.fail_focus_capture(true, reason);
    assert_eq!(captured.focus.outcome, Outcome::Unavailable { reason });
    assert_eq!(captured.completed.outcome, Outcome::Unavailable { reason });
    assert!(!captured.focus_raw_mode);
    assert_eq!(captured.technologies.outcome, Outcome::Rows { len: 1 });
}

#[test]
fn focus_job_capacity_accounting_tracks_all_vectors_through_growth_clear_and_drop() {
    use crate::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
    static LOCAL: Tracker = Tracker::new();
    let mut job = FocusJob {
        rows: vec![Default::default(); 2],
        completed: vec![Default::default(); 3],
        bytes: vec![0; 10],
        provisioned: true,
        ..Default::default()
    };
    job.capacity_charge = Charge::empty_on(&LOCAL);
    job.update_capacity_charge();
    let expected = |job: &FocusJob| {
        vector_bytes(&job.rows) + vector_bytes(&job.completed) + vector_bytes(&job.bytes)
    };
    let initial = LOCAL.snapshot().current.payload_vectors;
    assert_eq!(initial, expected(&job));
    assert!(job.prepare([4, 6], 20));
    assert!(job.grow_for_next());
    job.grow();
    let grown = LOCAL.snapshot().current.payload_vectors;
    assert!(grown > initial);
    assert_eq!(grown, expected(&job));
    job.clear();
    assert_eq!(LOCAL.snapshot().current.payload_vectors, grown);
    drop(job);
    assert_eq!(LOCAL.snapshot().current, Capacity::default());
}

#[test]
fn large_census_growth_leaves_quarter_capacity_free_without_power_of_two_inflation() {
    let mut capacity = Capacities::default();
    capacity.rows[2] = 4;
    let mut captured = Captured::with_capacities(&capacity);
    assert_eq!(
        captured.technology_pointers.len(),
        captured.technologies.rows.len()
    );
    let research_bytes = |captured: &Captured| {
        captured.technologies.allocated_bytes()
            + captured.technology_pointers.capacity() * size_of::<usize>()
    };
    let other_bytes = captured.allocated_bytes() - research_bytes(&captured);
    captured.technology_pointers[0] = 0x1234;
    captured.technologies.outcome = Outcome::Truncated {
        len: 4,
        unread_rows: 246_996,
    };
    captured.grow(&mut capacity);
    assert_eq!(capacity.rows[2], 329_334);
    assert!(247_000 <= capacity.rows[2] - capacity.rows[2] / 4);
    assert_eq!(captured.technology_pointers.len(), capacity.rows[2]);
    assert_eq!(
        captured.technology_pointers.len(),
        captured.technologies.rows.len()
    );
    assert_eq!(captured.technology_pointers[0], 0x1234);
    assert_eq!(
        captured.allocated_bytes(),
        other_bytes + research_bytes(&captured)
    );
    let pointers = captured.technology_pointers.as_ptr();
    let allocated = captured.allocated_bytes();
    captured.technologies.outcome = Outcome::Rows { len: 247_000 };
    captured.grow(&mut capacity);
    assert_eq!(capacity.rows[2], 329_334);
    captured.reset();
    assert_eq!(captured.technology_pointers.as_ptr(), pointers);
    assert_eq!(captured.technology_pointers[0], 0x1234);
    assert_eq!(captured.allocated_bytes(), allocated);
}
#[test]
fn prior_failure_and_unread_count_survive_later_empty_country() {
    let mut outcome = Outcome::Truncated {
        len: 2,
        unread_rows: 3,
    };
    append(&mut outcome, Outcome::Rows { len: 0 });
    assert_eq!(
        outcome,
        Outcome::Truncated {
            len: 2,
            unread_rows: 3
        }
    );
    append(
        &mut outcome,
        Outcome::Unavailable {
            reason: "invalid country",
        },
    );
    append(&mut outcome, Outcome::Rows { len: 4 });
    assert_eq!(
        outcome,
        Outcome::Unavailable {
            reason: "invalid country"
        }
    );
}
#[test]
fn native_decision_source_keys_and_non_utf8_literals_survive_conversion() {
    let mut capacity = Capacities::default();
    capacity.rows[9] = 1;
    capacity.decision_text = 2;
    capacity.decision_countries = 1;
    let mut captured = Captured::with_capacities(&capacity);
    captured.decisions.rows[0] = cdecision_status::Row {
        container: cdecision_status::Container::Targeted,
        token_len: 2,
        days: i32::MIN,
        country_target: u32::MAX,
        ..Default::default()
    };
    captured.decisions.outcome = Outcome::Rows { len: 1 };
    captured.decision_text.first[..2].copy_from_slice(&[0xff, b'x']);
    captured.decision_text.len = 2;
    captured.decision_countries.first[0] = 17;
    captured.decision_countries.len = 1;
    let tables = captured.tables();
    let TableView::Complete(rows::TableRows::Decisions(rows)) = &tables[9] else {
        panic!()
    };
    assert_eq!(rows[0].country, 17);
    assert_eq!(rows[0].container, 3);
    assert_eq!(rows[0].token, [0xff, b'x']);
    assert_eq!(rows[0].days, i32::MIN);
    assert_eq!(rows[0].country_target, u32::MAX);
    captured.decisions.rows[0].token_len = 3;
    assert!(matches!(
        captured.tables()[9],
        TableView::Unavailable { .. }
    ));
}

/// Shared synthetic address space for the copy/decode split harness.
/// Two countries share one template object; rosters hold native holes and
/// unsorted acquisition order; ids cover heap and inline storage.
#[cfg(test)]
/// Shared synthetic address space for the scheduled focus copy/decode split
/// harness: two authenticated countries with inline, absent and heap focus
/// tokens plus one- and two-entry completed rosters.
pub(crate) mod focus_test_kit {
    use super::{Capacities, Captured, FocusJob};
    use crate::extended::governance::copy_focus_refresh;
    use engine::memory::SliceMemory;
    use engine::{
        ccontinuous_national_focus, ccountry, cgame_state, cnational_focus,
        cnational_focus_progress, cnational_focus_tree,
    };

    pub(crate) const BASE: usize = 0x1400_0000;
    const MEM: usize = 0x1_0000;
    const GS: usize = 0x1_0000;
    const CTAB: usize = 0x1_1000;
    const LTAB: usize = 0x1_1200;
    const C1: usize = 0x1_2000;
    const C2: usize = 0x1_4000;
    const R1: usize = 0x1_6000;
    const R2: usize = 0x1_7000;
    const TT1: usize = 0x1_8000;
    const FC1: usize = 0x1_8200;
    const FC2: usize = 0x1_8300;
    const CC1: usize = 0x1_8400;
    const FA: usize = 0x1_8500;
    const FB: usize = 0x1_8600;
    const FC: usize = 0x1_8700;
    const RS1: usize = 0x1_8800;
    const RS2: usize = 0x1_8900;
    const HB: usize = 0x1_8a00;
    const HEAP_TOKEN: &[u8] = b"completed_heap_token_longer_than_fifteen";

    pub(crate) struct Fixture {
        pub bytes: Vec<u8>,
    }

    impl Fixture {
        pub(crate) fn w64(&mut self, addr: usize, value: u64) {
            let at = addr - MEM;
            self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }

        pub(crate) fn w32(&mut self, addr: usize, value: u32) {
            let at = addr - MEM;
            self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }

        fn inline(&mut self, addr: usize, value: &[u8]) {
            assert!(value.len() <= 15);
            let at = addr - MEM;
            self.bytes[at..at + 32].fill(0);
            self.bytes[at..at + value.len()].copy_from_slice(value);
            self.w64(addr + 16, value.len() as u64);
            self.w64(addr + 24, 15);
        }

        fn heap_token(&mut self, addr: usize, value: &[u8]) {
            let at = addr - MEM;
            self.bytes[at..at + 32].fill(0);
            self.w64(addr, HB as u64);
            self.w64(addr + 16, value.len() as u64);
            self.w64(addr + 24, value.len() as u64);
            let heap = HB - MEM;
            self.bytes[heap..heap + value.len()].copy_from_slice(value);
            self.bytes[heap + value.len()] = 0;
        }

        fn country(&mut self, country: usize, id: u32, tree: usize, root: usize) {
            self.w64(country, (BASE + ccountry::VTABLE_RVA) as u64);
            self.w32(country + ccountry::ID, id);
            self.w64(country + ccountry::FOCUS_TREE, tree as u64);
            self.w64(country + ccountry::FOCUS_PROGRESS, root as u64);
        }

        #[allow(clippy::too_many_arguments)]
        fn root(
            &mut self,
            root: usize,
            current: usize,
            continuous: usize,
            progress: i64,
            paused: u8,
            count: i32,
            roster: usize,
            roster_count: u32,
        ) {
            self.w64(root, (BASE + cnational_focus_progress::VTABLE_RVA) as u64);
            self.w64(root + cnational_focus_progress::CURRENT, current as u64);
            self.w64(
                root + cnational_focus_progress::CURRENT + 8,
                continuous as u64,
            );
            self.w64(root + cnational_focus_progress::PROGRESS, progress as u64);
            self.bytes[root - MEM + cnational_focus_progress::PAUSED] = paused;
            self.w32(
                root + cnational_focus_progress::COMPLETED_COUNT,
                count as u32,
            );
            self.w64(
                root + cnational_focus_progress::COMPLETED_DATA,
                roster as u64,
            );
            self.w32(
                root + cnational_focus_progress::COMPLETED_DATA + 8,
                roster_count,
            );
            self.w32(
                root + cnational_focus_progress::COMPLETED_DATA + 12,
                roster_count,
            );
        }

        pub(crate) fn world() -> Self {
            let mut fixture = Self {
                bytes: vec![0; 0x1_0000],
            };
            fixture.w64(GS + cgame_state::COUNTRIES, CTAB as u64);
            fixture.w32(GS + cgame_state::COUNTRIES + 8, 4);
            fixture.w32(GS + cgame_state::COUNTRIES + 12, 3);
            fixture.w64(GS + cgame_state::COUNTRY_LINKS, LTAB as u64);
            fixture.w32(GS + cgame_state::COUNTRY_LINKS + 8, 4);
            fixture.w32(GS + cgame_state::COUNTRY_LINKS + 12, 3);
            fixture.w64(CTAB, 0);
            fixture.w64(CTAB + 8, C1 as u64);
            fixture.w64(CTAB + 16, C2 as u64);
            fixture.w32(LTAB, 0);
            fixture.w32(LTAB + 4, 1);
            fixture.w32(LTAB + 8, 2);
            fixture.country(C1, 1, TT1, R1);
            fixture.country(C2, 2, 0, R2);
            fixture.inline(TT1 + cnational_focus_tree::TOKEN, b"tree_one");
            fixture.root(R1, FC1, CC1, 12_345, 1, 1, RS1, 1);
            fixture.inline(FC1 + cnational_focus::TOKEN, b"active_one");
            fixture.inline(CC1 + ccontinuous_national_focus::TOKEN, b"cont_one");
            fixture.inline(FA + cnational_focus::TOKEN, b"done_a");
            fixture.w64(RS1, FA as u64);
            fixture.root(R2, FC2, 0, -7, 0, 2, RS2, 2);
            fixture.inline(FC2 + cnational_focus::TOKEN, b"active_two");
            fixture.heap_token(FB + cnational_focus::TOKEN, HEAP_TOKEN);
            fixture.inline(FC + cnational_focus::TOKEN, b"done_c");
            fixture.w64(RS2, FB as u64);
            fixture.w64(RS2 + 8, FC as u64);
            fixture
        }

        pub(crate) fn memory(&self) -> SliceMemory<'_> {
            SliceMemory {
                base: MEM,
                bytes: &self.bytes,
            }
        }

        pub(crate) fn captured() -> Captured {
            Captured::with_capacities(&Capacities::default())
        }

        pub(crate) fn copy_first(
            captured: &mut Captured,
            memory: &impl engine::Memory,
        ) -> (super::FocusCopyReport, FocusJob) {
            let mut job = FocusJob::new();
            let ctx = captured.focus_copy_context(BASE, GS, true);
            let report = copy_focus_refresh(memory, ctx, &mut job);
            (report, job)
        }

        /// Byte-identity assertion for the decode worker tests: typed
        /// rows, outcomes and arena bytes match exactly.
        pub(crate) fn assert_matches_typed(decoded: &Captured, typed: &Captured) {
            assert_eq!(decoded.focus.outcome, typed.focus.outcome);
            assert_eq!(decoded.completed.outcome, typed.completed.outcome);
            let focus_len = typed.focus.outcome.len();
            assert_eq!(
                &decoded.focus.rows[..focus_len],
                &typed.focus.rows[..focus_len]
            );
            let completed_len = typed.completed.outcome.len();
            assert_eq!(
                &decoded.completed.rows[..completed_len],
                &typed.completed.rows[..completed_len]
            );
            assert_eq!(decoded.focus_text.len, typed.focus_text.len);
            assert_eq!(decoded.focus_text.required, typed.focus_text.required);
            assert_eq!(
                &decoded.focus_text.first[..decoded.focus_text.len],
                &typed.focus_text.first[..typed.focus_text.len]
            );
        }
    }
}

#[test]
fn focus_copy_decode_is_byte_identical_from_cold_and_small_reusable_storage() {
    use focus_test_kit::Fixture;
    let fixture = Fixture::world();
    let memory = fixture.memory();
    let mut typed = Fixture::captured();
    typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
    assert_eq!(typed.focus.outcome, Outcome::Rows { len: 2 });
    assert_eq!(typed.completed.outcome, Outcome::Rows { len: 3 });
    assert_eq!(typed.focus_text.len, 88);
    assert_eq!(typed.focus_text.required, 88);
    // Heap token bytes survive exactly, including the long payload.
    let heap = b"completed_heap_token_longer_than_fifteen";
    assert!(
        typed.focus_text.first[..88]
            .windows(heap.len())
            .any(|window| window == heap)
    );

    let mut replayed = Fixture::captured();
    replayed.focus_raw_mode = true;
    let (report, first) = Fixture::copy_first(&mut replayed, &memory);
    assert!(report.raw_ok);
    replayed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
    let mut ticks = [0; 2];
    replayed.decode_focus_payload(focus_test_kit::BASE, &first, &mut ticks);
    assert!(!replayed.focus_raw_mode);
    assert_eq!(replayed.focus.outcome, typed.focus.outcome);
    assert_eq!(replayed.completed.outcome, typed.completed.outcome);
    let focus_len = typed.focus.outcome.len();
    assert_eq!(
        &typed.focus.rows[..focus_len],
        &replayed.focus.rows[..focus_len]
    );
    let completed_len = typed.completed.outcome.len();
    assert_eq!(
        &typed.completed.rows[..completed_len],
        &replayed.completed.rows[..completed_len]
    );
    assert_eq!(replayed.focus_text.len, typed.focus_text.len);
    assert_eq!(replayed.focus_text.required, typed.focus_text.required);
    assert_eq!(
        &replayed.focus_text.first[..replayed.focus_text.len],
        &typed.focus_text.first[..typed.focus_text.len]
    );

    // Small reusable buffers grow from this same original native demand.
    let mut small = Capacities::default();
    small.rows[0] = 1;
    small.rows[1] = 1;
    small.focus_text = 20;
    let mut typed_small = Captured::with_capacities(&small);
    typed_small.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
    let mut replayed_small = Captured::with_capacities(&small);
    replayed_small.focus_raw_mode = true;
    let (_, small_first) = Fixture::copy_first(&mut replayed_small, &memory);
    replayed_small.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
    let mut small_ticks = [0; 2];
    replayed_small.decode_focus_payload(focus_test_kit::BASE, &small_first, &mut small_ticks);
    assert_eq!(replayed_small.focus.outcome, typed_small.focus.outcome);
    assert_eq!(
        replayed_small.completed.outcome,
        typed_small.completed.outcome
    );
    assert_eq!(replayed_small.focus.outcome, Outcome::Rows { len: 2 });
    assert_eq!(replayed_small.completed.outcome, Outcome::Rows { len: 3 });
    assert_eq!(replayed_small.focus_text.len, 88);
    assert_eq!(
        replayed_small.focus_text.required,
        typed_small.focus_text.required
    );
    assert_eq!(replayed_small.focus_text.len, typed_small.focus_text.len);
}

#[test]
fn focus_storage_matrix_grows_before_publication_and_matches_direct() {
    // Each prior reusable extent, including empty storage, produces complete
    // first output from the original native headers. Fixed engine shortage
    // behavior remains covered by cnational_focus_progress's fixed walk tests.
    use focus_test_kit::Fixture;
    let fixture = Fixture::world();
    let memory = fixture.memory();
    for (focus_rows, completed_rows, text) in [
        (0, 0, 0),
        (1, 1, 20),
        (1, 2, 20),
        (2, 3, 88),
        (3, 4, 128),
        (64, 64, 8192),
    ] {
        let mut caps = Capacities::default();
        caps.rows[0] = focus_rows;
        caps.rows[1] = completed_rows;
        caps.focus_text = text;
        let mut typed = Captured::with_capacities(&caps);
        typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
        let mut replayed = Captured::with_capacities(&caps);
        replayed.focus_raw_mode = true;
        let (report, first) = Fixture::copy_first(&mut replayed, &memory);
        assert!(
            report.raw_ok,
            "focus={focus_rows} completed={completed_rows}"
        );
        replayed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
        let mut ticks = [0; 2];
        replayed.decode_focus_payload(focus_test_kit::BASE, &first, &mut ticks);
        Fixture::assert_matches_typed(&replayed, &typed);
        assert_eq!(replayed.focus.outcome, Outcome::Rows { len: 2 });
        assert_eq!(replayed.completed.outcome, Outcome::Rows { len: 3 });
        assert_eq!(replayed.focus_text.len, 88);
    }
    // Exact-fit arm pins Rows on both tables.
    let mut caps = Capacities::default();
    caps.rows[0] = 2;
    caps.rows[1] = 3;
    caps.focus_text = 88;
    let mut typed = Captured::with_capacities(&caps);
    typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
    assert_eq!(typed.focus.outcome, Outcome::Rows { len: 2 });
    assert_eq!(typed.completed.outcome, Outcome::Rows { len: 3 });
}

#[test]
fn focus_copy_decode_matches_faults_with_identical_reasons() {
    use focus_test_kit::Fixture;
    // Null status root fails both tables with the same reason.
    let mut broken = Fixture::world();
    broken.w64(0x1_2000 + ccountry::FOCUS_PROGRESS, 0);
    let memory = broken.memory();
    let mut typed = Fixture::captured();
    typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
    assert!(matches!(typed.focus.outcome, Outcome::Unavailable { .. }));
    let mut replayed = Fixture::captured();
    replayed.focus_raw_mode = true;
    let (report, first) = Fixture::copy_first(&mut replayed, &memory);
    assert!(report.raw_ok);
    replayed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
    let mut ticks = [0; 2];
    replayed.decode_focus_payload(focus_test_kit::BASE, &first, &mut ticks);
    assert_eq!(replayed.focus.outcome, typed.focus.outcome);
    assert_eq!(replayed.completed.outcome, typed.completed.outcome);

    // Vtable mismatch fails both tables together.
    let mut broken = Fixture::world();
    broken.w64(0x1_6000, 0);
    let memory = broken.memory();
    let mut typed = Fixture::captured();
    typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
    let mut replayed = Fixture::captured();
    replayed.focus_raw_mode = true;
    let (_, first) = Fixture::copy_first(&mut replayed, &memory);
    replayed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
    let mut ticks = [0; 2];
    replayed.decode_focus_payload(focus_test_kit::BASE, &first, &mut ticks);
    assert_eq!(replayed.focus.outcome, typed.focus.outcome);
    assert_eq!(replayed.completed.outcome, typed.completed.outcome);
    assert!(matches!(
        typed.focus.outcome,
        Outcome::Unavailable {
            reason: "focus status vtable mismatch"
        }
    ));

    // A bad summary token fails only the summary; completions survive.
    let mut broken = Fixture::world();
    broken.w64(0x1_8200 + cnational_focus::TOKEN + 16, 99);
    let memory = broken.memory();
    let mut typed = Fixture::captured();
    typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
    assert!(matches!(typed.focus.outcome, Outcome::Unavailable { .. }));
    let mut replayed = Fixture::captured();
    replayed.focus_raw_mode = true;
    let (_, first) = Fixture::copy_first(&mut replayed, &memory);
    replayed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
    let mut ticks = [0; 2];
    replayed.decode_focus_payload(focus_test_kit::BASE, &first, &mut ticks);
    assert_eq!(replayed.focus.outcome, typed.focus.outcome);
    assert_eq!(replayed.completed.outcome, typed.completed.outcome);
    assert_eq!(typed.completed.outcome, Outcome::Rows { len: 3 });

    // A null completed entry fails only the roster; the summary survives.
    let mut broken = Fixture::world();
    broken.w64(0x1_8800, 0);
    let memory = broken.memory();
    let mut typed = Fixture::captured();
    typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
    assert_eq!(typed.focus.outcome, Outcome::Rows { len: 2 });
    assert!(matches!(
        typed.completed.outcome,
        Outcome::Unavailable { .. }
    ));
    let mut replayed = Fixture::captured();
    replayed.focus_raw_mode = true;
    let (_, first) = Fixture::copy_first(&mut replayed, &memory);
    replayed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
    let mut ticks = [0; 2];
    replayed.decode_focus_payload(focus_test_kit::BASE, &first, &mut ticks);
    assert_eq!(replayed.focus.outcome, typed.focus.outcome);
    assert_eq!(replayed.completed.outcome, typed.completed.outcome);
}

struct FocusReads<'a> {
    memory: engine::memory::SliceMemory<'a>,
    reads: std::cell::Cell<usize>,
    fault: Option<usize>,
    failed: std::cell::Cell<usize>,
}

impl Memory for FocusReads<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.set(self.reads.get() + 1);
        if self.fault == Some(address) && self.failed.get() == 0 {
            self.failed.set(1);
            return false;
        }
        self.memory.copy(address, destination)
    }
}

#[test]
fn daily_governance_sources_keep_politics_and_research_siblings_independent() {
    use focus_test_kit::{BASE, Fixture};
    struct Sources<'a> {
        memory: engine::memory::SliceMemory<'a>,
        ideas: std::cell::Cell<usize>,
        fail_ideas: bool,
    }
    impl Memory for Sources<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let private = [
                0x1_9000 + cpolitical_status::IDEAS,
                0x1_a000 + cpolitical_status::IDEAS,
            ];
            if private
                .iter()
                .any(|&at| address <= at && at < address.saturating_add(output.len()))
            {
                self.ideas.set(self.ideas.get() + 1);
                if self.fail_ideas {
                    return false;
                }
            }
            self.memory.copy(address, output)
        }
    }
    let mut fixture = Fixture::world();
    for (country, politics) in [(0x1_2000, 0x1_9000), (0x1_4000, 0x1_a000)] {
        fixture.w64(country + ccountry::POLITICAL_STATUS, politics as u64);
        fixture.w64(politics, (BASE + cpolitical_status::VTABLE_RVA) as u64);
        fixture.w64(politics + cpolitical_status::COUNTRY, country as u64);
    }
    for fail_ideas in [false, true] {
        for hour in 0..24 {
            let memory = engine::memory::CountedMemory::new(Sources {
                memory: fixture.memory(),
                ideas: Default::default(),
                fail_ideas,
            });
            let mut captured = Fixture::captured();
            let accounting = read_accounting::Capture::new();
            captured.capture(&memory, BASE, 0x1_0000, hour, false);
            let counts = accounting.finish();
            assert_eq!(captured.politics.outcome, Outcome::Rows { len: 2 });
            assert_eq!(captured.parties.outcome, Outcome::Rows { len: 0 });
            // Ideas are hourly (replay::cadence).
            assert!(memory.0.ideas.get() > 0, "hour={hour}");
            for (index, slot) in [(0, 19), (3, 11)] {
                assert_eq!(
                    counts[TABLES[index].index()].spans > 0,
                    hour == slot,
                    "table={:?} hour={hour}",
                    TABLES[index]
                );
            }
            // Technologies are sliced; a new store reads every country at once.
            assert!(counts[TABLES[2].index()].spans > 0, "hour={hour}");
            for (index, outcome) in captured.outcomes().into_iter().enumerate() {
                if !replay::cadence::should_walk(TABLES[index], hour, false) {
                    assert_eq!(
                        outcome,
                        Outcome::Unavailable {
                            reason: replay::cadence::SKIPPED_REASON
                        }
                    );
                    assert_eq!(counts[TABLES[index].index()].spans, 0);
                }
            }
            if fail_ideas {
                assert!(matches!(
                    captured.ideas.outcome,
                    Outcome::Unavailable { .. }
                ));
            }
        }
    }
}

#[test]
fn failed_native_technology_name_does_not_request_deferred_arena_growth() {
    use focus_test_kit::{BASE, Fixture};
    const TEMPLATE: usize = 0x1_c000;
    const HEAP: usize = 0x9000_0000;
    const LENGTH: usize = 32768;
    let mut fixture = Fixture::world();
    for (owner, country) in [0x1_2000, 0x1_4000].into_iter().enumerate() {
        let status = 0x1_a000 + owner * 0x200;
        let roster = 0x1_a400 + owner * 0x20;
        let technology = 0x1_b000 + owner * 0x200;
        fixture.w64(country + ccountry::TECHNOLOGY_STATUS, status as u64);
        fixture.w64(status, (BASE + ctechnology_status::VTABLE_RVA) as u64);
        fixture.w64(status + ctechnology_status::COUNTRY, country as u64);
        fixture.w64(
            status + ctechnology_status::TECHNOLOGIES_DATA,
            roster as u64,
        );
        for offset in [8, 12] {
            fixture.w32(status + ctechnology_status::TECHNOLOGIES_DATA + offset, 1);
        }
        fixture.w64(roster, technology as u64);
        fixture.w64(technology, (BASE + ctechnology::VTABLE_RVA) as u64);
        fixture.w32(technology + ctechnology::DEFINITION_INDEX, 3);
        fixture.w64(technology + ctechnology::STATUS, status as u64);
        fixture.w64(technology + ctechnology::TEMPLATE, TEMPLATE as u64);
    }
    fixture.w64(TEMPLATE + ctechnology_template::ID, HEAP as u64);
    fixture.w64(TEMPLATE + ctechnology_template::ID + 16, LENGTH as u64);
    fixture.w64(TEMPLATE + ctechnology_template::ID + 24, LENGTH as u64);
    fixture.w32(TEMPLATE + ctechnology_template::DATABASE_INDEX, 1);
    fixture.w32(TEMPLATE + ctechnology_template::TECHNOLOGY_HANDLE, 3);
    struct Fault<'a>(engine::memory::SliceMemory<'a>);
    impl Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == HEAP + LENGTH && output.len() == 1 {
                output[0] = 0;
                return true;
            }
            self.0.copy(address, output)
        }
    }
    let mut capacity = Capacities {
        definition_text: 0,
        ..Default::default()
    };
    let mut captured = Captured::with_capacities(&capacity);
    captured.capture(&Fault(fixture.memory()), BASE, 0x1_0000, 0, true);
    assert_eq!(captured.technologies.outcome, Outcome::Rows { len: 2 });
    assert_eq!(
        captured.definitions.outcome,
        Outcome::Unavailable {
            reason: "technology id bytes unreadable",
        }
    );
    assert_eq!(captured.definition_text.required, 0);
    captured.grow(&mut capacity);
    assert_eq!(capacity.definition_text, 0);
    assert_eq!(captured.definition_text.first.capacity(), 0);
}

#[test]
fn first_research_capture_grows_all_storage_and_keeps_native_and_definition_order() {
    use focus_test_kit::{BASE, Fixture};
    let mut fixture = Fixture::world();
    for (owner, country) in [0x1_2000, 0x1_4000].into_iter().enumerate() {
        let status = 0x1_a000 + owner * 0x200;
        let roster = 0x1_a400 + owner * 0x20;
        fixture.w64(country + ccountry::TECHNOLOGY_STATUS, status as u64);
        fixture.w64(status, (BASE + ctechnology_status::VTABLE_RVA) as u64);
        fixture.w64(status + ctechnology_status::COUNTRY, country as u64);
        fixture.w64(
            status + ctechnology_status::TECHNOLOGIES_DATA,
            roster as u64,
        );
        for offset in [8, 12] {
            fixture.w32(status + ctechnology_status::TECHNOLOGIES_DATA + offset, 2);
        }
        for index in 0..2 {
            let technology = 0x1_b000 + (owner * 2 + index) * 0x200;
            let definition = index;
            let template = 0x1_c000 + definition * 0x600;
            fixture.w64(roster + index * 8, technology as u64);
            fixture.w64(technology, (BASE + ctechnology::VTABLE_RVA) as u64);
            fixture.w32(technology + ctechnology::DEFINITION_INDEX, 0x8000_0001);
            fixture.w64(technology + ctechnology::STATUS, status as u64);
            fixture.w64(technology + ctechnology::TEMPLATE, template as u64);
        }
    }
    for (index, handle) in [0x8000_0001, 0x8000_0001].into_iter().enumerate() {
        let template = 0x1_c000 + index * 0x600;
        fixture.w64(
            template + ctechnology_template::ID,
            u32::from_le_bytes(*b"tech") as u64,
        );
        fixture.w64(template + ctechnology_template::ID + 16, 4);
        fixture.w64(template + ctechnology_template::ID + 24, 15);
        fixture.w32(
            template + ctechnology_template::DATABASE_INDEX,
            index as u32 + 1,
        );
        fixture.w32(template + ctechnology_template::TECHNOLOGY_HANDLE, handle);
    }
    struct Reads<'a> {
        memory: engine::memory::SliceMemory<'a>,
        spans: std::cell::RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Reads<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.spans.borrow_mut().push((address, destination.len()));
            self.memory.copy(address, destination)
        }
    }
    let memory = Reads {
        memory: fixture.memory(),
        spans: Default::default(),
    };
    let mut captured = Captured::vacant();
    assert_eq!(captured.allocated_bytes(), 0);
    let mut enabled = [false; N];
    enabled[2] = true;
    enabled[4] = true;
    captured.technology_selection = SliceSelection::new(true);
    captured.technology_selection.reset(0, true);
    let mut store = SliceFingerprints::new();
    let result = acquire(
        &memory,
        BASE,
        0x1_0000,
        captured.buffers(),
        &enabled,
        &mut store,
    );
    assert_eq!(result.outcomes[2], Outcome::Rows { len: 4 });
    assert_eq!(result.outcomes[4], Outcome::Rows { len: 2 });
    assert_eq!(result.lens[1], 8);
    assert_eq!(
        captured.technologies.rows[..4]
            .iter()
            .map(|row| (row.country, row.database_index, row.technology_handle))
            .collect::<Vec<_>>(),
        [
            (1, 1, 0x8000_0001),
            (1, 2, 0x8000_0001),
            (2, 1, 0x8000_0001),
            (2, 2, 0x8000_0001)
        ]
    );
    assert_eq!(
        captured
            .definitions
            .rows
            .iter()
            .map(|row| (row.database_index, row.technology_handle))
            .collect::<Vec<_>>(),
        [(1, 0x8000_0001), (2, 0x8000_0001)]
    );
    assert_eq!(
        captured.allocated_bytes(),
        captured.technologies.rows.capacity() * size_of::<ctechnology::Row>()
            + captured.technology_pointers.capacity() * size_of::<usize>()
            + captured.definitions.rows.capacity() * size_of::<ctechnology_template::Row>()
            + captured.definition_text.first.capacity()
            + captured.dictionary.capacity() * size_of::<ctechnology_template::Entry>()
    );
    let spans = memory.spans.borrow();
    for owner in 0..2 {
        assert_eq!(
            spans
                .iter()
                .filter(|&&span| span
                    == (
                        0x1_a000 + owner * 0x200 + ctechnology_status::TECHNOLOGIES_DATA,
                        16
                    ))
                .count(),
            1
        );
        assert_eq!(
            spans
                .iter()
                .filter(|&&span| span == (0x1_c000 + owner * 0x600 + ctechnology_template::ID, 32))
                .count(),
            1
        );
    }
    drop(spans);
    // Later hours read only their slice, new owners and unpublished reads;
    // every roster read reaches both templates.
    fn read(
        captured: &mut Captured,
        memory: &impl Memory,
        enabled: &[bool; N],
        (hour, force_full): (i32, bool),
        store: &mut SliceFingerprints,
    ) -> (Vec<u32>, usize) {
        captured.technology_selection.reset(hour, force_full);
        let result = acquire(memory, BASE, 0x1_0000, captured.buffers(), enabled, store);
        let owners = captured.technologies.rows[..result.outcomes[2].len()]
            .iter()
            .map(|row| row.country)
            .collect();
        (owners, result.outcomes[4].len())
    }
    let mut at = |hour, force_full, store: &mut SliceFingerprints| {
        read(&mut captured, &memory, &enabled, (hour, force_full), store)
    };
    assert_eq!(at(1, false, &mut store), (vec![1, 1], 2));
    assert_eq!(at(3, false, &mut store), (vec![], 0));
    // An unpublished full hour is read again at the next hour.
    assert_eq!(at(4, true, &mut store), (vec![1, 1, 2, 2], 2));
    // The complete census saw only countries 1 and 2.
    let selection = &captured.technology_selection;
    assert!(selection.live(1) && selection.live(2) && !selection.live(3));
    selection.invalidate(&mut store);
    let mut at = |hour, force_full, store: &mut SliceFingerprints| {
        read(&mut captured, &memory, &enabled, (hour, force_full), store).0
    };
    assert_eq!(at(5, false, &mut store), [1, 1, 2, 2]);
    assert!(at(6, false, &mut store).is_empty());
    // A forgotten store makes every country new: read at once.
    store.forget();
    assert_eq!(at(7, false, &mut store), [1, 1, 2, 2]);
}

#[test]
fn inactive_focus_bypasses_owned_capacity_and_decode_preflights() {
    use focus_test_kit::{BASE, Fixture};
    let fixture = Fixture::world();
    let memory = FocusReads {
        memory: fixture.memory(),
        reads: Default::default(),
        fault: Some(0x1_6000),
        failed: Default::default(),
    };
    for hour in 0..24 {
        if replay::cadence::should_walk(TableId::Focus, hour, false) {
            continue;
        }
        let mut job = FocusJob::default();
        let report = copy_focus_refresh(
            &memory,
            FocusCopyContext {
                base: BASE,
                game_state: 0x1_0000,
                active: false,
                rows: [usize::MAX; 2],
                bytes: usize::MAX,
            },
            &mut job,
        );
        assert!(report.raw_ok);
        assert_eq!(report.copy_ticks, [0; 2]);
        assert_eq!(memory.reads.get(), 0);
        let mut captured = Fixture::captured();
        captured.politics.outcome = Outcome::Rows { len: 1 };
        let mut ticks = [0; 2];
        captured.decode_focus_payload(BASE, &job, &mut ticks);
        assert_eq!(ticks, [0; 2]);
        assert_eq!(
            captured.focus.outcome,
            Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON
            }
        );
        assert_eq!(captured.completed.outcome, captured.focus.outcome);
        assert_eq!(captured.politics.outcome, Outcome::Rows { len: 1 });
    }
}

#[test]
fn focus_cold_worker_storage_preserves_native_faults_without_deferred_retry() {
    use focus_test_kit::{BASE, Fixture};
    let fixture = Fixture::world();
    let memory = engine::memory::CountedMemory::new(FocusReads {
        memory: fixture.memory(),
        reads: Default::default(),
        fault: Some(0x1_6000),
        failed: Default::default(),
    });
    let mut captured = Fixture::captured();
    let ctx = captured.focus_copy_context(BASE, 0x1_0000, true);
    let mut job = FocusJob::new();
    assert_eq!(job.capacities(), ([0; 2], 0));
    let native = read_accounting::Capture::new();
    let report = copy_focus_refresh(&memory, ctx, &mut job);
    assert!(report.raw_ok);
    assert!(native.finish()[TABLES[0].index()].spans > 0);
    assert_eq!(memory.0.failed.get(), 1);
    let reads = memory.0.reads.get();
    captured.decode_focus_payload(BASE, &job, &mut [0; 2]);
    assert_eq!(memory.0.reads.get(), reads);
    assert_eq!(memory.0.failed.get(), 1);
    assert!(matches!(
        captured.focus.outcome,
        Outcome::Unavailable { .. }
    ));
    assert_eq!(captured.completed.outcome, captured.focus.outcome);
    assert!(!job.grow_for_next());
    job.clear();
    assert!(job.is_provisioned());
    assert_eq!(job.outcomes, [None; 2]);
}

#[test]
fn fixed_focus_prepare_refuses_unprovisioned_storage_before_any_source_read() {
    use focus_test_kit::{BASE, Fixture};
    let fixture = Fixture::world();
    let memory = FocusReads {
        memory: fixture.memory(),
        reads: Default::default(),
        fault: Some(0x1_6000),
        failed: Default::default(),
    };
    let captured = Fixture::captured();
    let ctx = captured.focus_copy_context(BASE, 0x1_0000, true);
    let native = read_accounting::Capture::new();
    let mut job = FocusJob::default();
    let prepared = job.prepare(ctx.rows, ctx.bytes);
    if prepared {
        copy_focus_pass(&mut job, &memory, ctx.base, ctx.game_state, &mut [0; 2]);
    }
    assert!(!prepared);
    assert_eq!(native.finish(), read_accounting::ZERO);
    assert_eq!(memory.reads.get(), 0);
    assert_eq!(memory.failed.get(), 0);
}

#[test]
fn focus_refresh_reads_one_pass_and_owned_decode_never_reads_live_memory() {
    use focus_test_kit::{BASE, Fixture};
    let fixture = Fixture::world();
    let source = engine::memory::CountedMemory::new(FocusReads {
        memory: fixture.memory(),
        reads: Default::default(),
        fault: None,
        failed: Default::default(),
    });
    let reference = engine::memory::CountedMemory::new(FocusReads {
        memory: fixture.memory(),
        reads: Default::default(),
        fault: None,
        failed: Default::default(),
    });
    let mut captured = Fixture::captured();
    let ctx = captured.focus_copy_context(BASE, 0x1_0000, true);
    let mut expected = FocusJob::new();
    assert!(expected.prepare(ctx.rows, ctx.bytes));
    let native = read_accounting::Capture::new();
    copy_focus_pass(&mut expected, &reference, BASE, 0x1_0000, &mut [0; 2]);
    let expected_counts = native.finish();
    let mut job = FocusJob::new();
    let native = read_accounting::Capture::new();
    let report = copy_focus_refresh(&source, ctx, &mut job);
    assert!(report.raw_ok);
    assert!(source.0.reads.get() > 0);
    assert_eq!(source.0.reads.get(), reference.0.reads.get());
    assert_eq!(read_accounting::snapshot(), expected_counts);
    let mut anchored = read_accounting::ZERO;
    anchored[TABLES[0].index()] = expected_counts[TABLES[0].index()];
    assert!(anchored[TABLES[0].index()].spans > 0);
    assert_eq!(expected_counts, anchored);
    assert_eq!(job.outcomes, expected.outcomes);
    assert_eq!(&job.rows[..2], &expected.rows[..2]);
    assert_eq!(&job.completed[..3], &expected.completed[..3]);
    assert_eq!(
        &job.bytes[..job.bytes_len],
        &expected.bytes[..expected.bytes_len]
    );
    let reads = source.0.reads.get();
    captured.decode_focus_payload(BASE, &job, &mut [0; 2]);
    assert_eq!(source.0.reads.get(), reads);
    assert_eq!(native.finish(), expected_counts);
    assert_eq!(captured.focus.outcome, Outcome::Rows { len: 2 });
    assert_eq!(captured.completed.outcome, Outcome::Rows { len: 3 });
    assert_eq!(&captured.focus.rows[..2], &job.rows[..2]);
    assert_eq!(&captured.completed.rows[..3], &job.completed[..3]);
}

#[test]
fn focus_native_faults_are_terminal_and_decode_cannot_retry_sources() {
    use focus_test_kit::{BASE, Fixture};
    let fixture = Fixture::world();
    for (fault, unavailable) in [
        (0x1_0000 + engine::cgame_state::COUNTRIES, [true, true]),
        (0x1_6000, [true, true]),
        (0x1_6000 + cnational_focus_progress::PROGRESS, [true, false]),
        (0x1_8800, [false, true]),
        (0x1_8a00, [false, true]),
    ] {
        let memory = FocusReads {
            memory: fixture.memory(),
            reads: Default::default(),
            fault: Some(fault),
            failed: Default::default(),
        };
        let mut captured = Fixture::captured();
        let (report, first) = Fixture::copy_first(&mut captured, &memory);
        assert!(report.raw_ok);
        assert_eq!(memory.failed.get(), 1);
        let reads = memory.reads.get();
        captured.focus_raw_mode = true;
        captured.decode_focus_payload(BASE, &first, &mut [0; 2]);
        assert_eq!(memory.reads.get(), reads);
        assert!(!captured.focus_raw_mode);
        for (slot, outcome) in [captured.focus.outcome, captured.completed.outcome]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                matches!(outcome, Outcome::Unavailable { .. }),
                unavailable[slot]
            );
        }
    }
}

#[test]
fn focus_install_rejects_short_owned_payloads_and_keeps_unaffected_table() {
    use focus_test_kit::{BASE, Fixture};
    let fixture = Fixture::world();
    for changed in 0..3 {
        let mut captured = Fixture::captured();
        let (_, mut first) = Fixture::copy_first(&mut captured, &fixture.memory());
        match changed {
            0 => {
                first.rows.truncate(first.outcomes[0].unwrap().len() - 1);
            }
            1 => {
                first
                    .completed
                    .truncate(first.outcomes[1].unwrap().len() - 1);
            }
            _ => {
                first.bytes.truncate(first.bytes_len - 1);
            }
        }
        captured.decode_focus_payload(BASE, &first, &mut [0; 2]);
        assert_eq!(
            matches!(captured.focus.outcome, Outcome::Unavailable { .. }),
            changed != 1
        );
        assert_eq!(
            matches!(captured.completed.outcome, Outcome::Unavailable { .. }),
            changed != 0
        );
    }
}

#[test]
fn focus_pair_uses_daily_slot_nineteen() {
    for hour in -24_i32..48 {
        assert_eq!(
            replay::cadence::should_walk(TableId::Focus, hour, false),
            hour.rem_euclid(24) == 19,
            "hour {hour}"
        );
        assert_eq!(
            replay::cadence::should_walk(TableId::CompletedFocuses, hour, false),
            hour.rem_euclid(24) == 19,
            "hour {hour}"
        );
        assert!(replay::cadence::should_walk(TableId::Focus, hour, true));
        assert!(replay::cadence::should_walk(
            TableId::CompletedFocuses,
            hour,
            true
        ));
    }
}
