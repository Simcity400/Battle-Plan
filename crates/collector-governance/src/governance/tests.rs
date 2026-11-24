use super::*;

#[test]
fn nested_heap_measurement_is_lazy_tracks_loans_and_survives_outer_growth() {
    use engine_research::research_inputs::{Row, Text};
    let mut captured = Captured::vacant();
    captured.research_inputs.rows.push(Row {
        token: Text::Owned(Vec::with_capacity(64)),
        ..Default::default()
    });
    captured
        .breakdowns
        .push(rows::PowerBalanceModifierBreakdownRow {
            record: 7,
            sources: vec![Vec::with_capacity(23)],
            daily: Vec::new(),
            weekly: Vec::new(),
        });
    assert!(captured.nested_bytes.get().is_none());
    let before = captured.allocated_bytes();
    assert_eq!(
        captured.nested_bytes.get().copied(),
        Some(captured.nested_allocated_bytes())
    );
    assert_eq!(captured.allocated_bytes(), before);
    let mut part = captured.take_research();
    assert!(captured.nested_bytes.get().is_none());
    let loaned = captured.allocated_bytes();
    assert!(loaned < before);
    part.inputs.rows[0].token = Text::Owned(Vec::with_capacity(128));
    captured.restore_research(part);
    assert!(captured.nested_bytes.get().is_none());
    assert!(captured.allocated_bytes() > before);
    assert_eq!(
        captured.nested_bytes.get().copied(),
        Some(captured.nested_allocated_bytes())
    );
    captured.reset();
    assert!(captured.nested_bytes.get().is_none());
    captured.allocated_bytes();
    assert_eq!(
        captured.nested_bytes.get().copied(),
        Some(captured.nested_allocated_bytes())
    );
    let nested = captured.nested_bytes.get().copied();
    let mut capacity = Capacities::default();
    capacity.rows[18] = 3;
    capacity.rows[19] = 3;
    capacity.rows[20] = 3;
    captured.grow(&mut capacity);
    assert_eq!(captured.nested_bytes.get().copied(), nested);
    captured.allocated_bytes();
    assert_eq!(
        captured.nested_bytes.get().copied(),
        Some(captured.nested_allocated_bytes())
    );
}

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
        assert!(replay_data::cadence::should_walk(
            TableId::TimedIdeas,
            hour,
            false
        ));
        assert_eq!(
            replay_data::cadence::should_walk(TableId::DecisionRandomItems, hour, false),
            replay_data::cadence::should_walk(TableId::Decisions, hour, false)
        );
    }
}

#[test]
fn breakdown_capture_reads_sources_without_the_cached_trees() {
    use engine_native_memory::memory::SliceMemory;
    const STATE: usize = 0x100;
    const MANAGER: usize = 0x800;
    const RECORDS: usize = 0x1000;
    const SOURCES: usize = 0x1400;
    const STATICS: usize = 0x2000;
    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }
    fn name(bytes: &mut [u8], at: usize, text: &[u8]) {
        put(bytes, at, text);
        put(bytes, at + 0x10, &(text.len() as u64).to_le_bytes());
        put(bytes, at + 0x18, &15_u64.to_le_bytes());
    }
    let mut bytes = vec![0_u8; STATICS + 0x400];
    put(
        &mut bytes,
        STATE + engine_native::cgame_state::POWER_BALANCE_MANAGER,
        &(MANAGER as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        MANAGER + power_balance_manager::RECORDS,
        &(RECORDS as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        MANAGER + power_balance_manager::RECORDS + 8,
        &((1_u64 << 32) | 1).to_le_bytes(),
    );
    put(&mut bytes, RECORDS + 0x50, &(SOURCES as u64).to_le_bytes());
    put(
        &mut bytes,
        RECORDS + 0x58,
        &((3_u64 << 32) | 3).to_le_bytes(),
    );
    for (ordinal, source) in [STATICS, STATICS + 0x200, STATICS].into_iter().enumerate() {
        put(
            &mut bytes,
            SOURCES + ordinal * 8,
            &(source as u64).to_le_bytes(),
        );
    }
    name(&mut bytes, STATICS + 0x1a8, b"first");
    name(&mut bytes, STATICS + 0x200 + 0x1a8, b"second");
    put(
        &mut bytes,
        RECORDS + 0xc8 + 0x18 + 8,
        &((1_u64 << 32) | 1).to_le_bytes(),
    );
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert!(engine_native_politics::modifier_breakdown::balance(&memory, 0, RECORDS).is_err());
    let mut captured = Captured::vacant();
    captured.balances.outcome = Outcome::Rows { len: 1 };
    captured.capture_breakdowns(&memory, 0, STATE, 1);
    assert_eq!(captured.breakdown_outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        captured.breakdowns,
        [rows::PowerBalanceModifierBreakdownRow {
            record: 0,
            sources: vec![b"first".to_vec(), b"second".to_vec(), b"first".to_vec()],
            daily: Vec::new(),
            weekly: Vec::new(),
        }]
    );
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
    use engine_native::{ccharacter, ccountry};
    use engine_native_parts::cideology_group;
    use engine_native_memory::memory::SliceMemory;
    use engine_native_politics::ccountry_leader;
    let mut bytes = vec![0_u8; 0x3000];
    let mut word = |at: usize, value: u64| bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    word(0x100, ccountry::VTABLE_RVA.0 as u64);
    word(0x100 + ccountry::POLITICAL_STATUS, 0x1200);
    word(0x1200, engine_native_parts::cpolitical_status_layout::VTABLE_RVA.0 as u64);
    word(0x1200 + engine_native_parts::cpolitical_status_layout::COUNTRY, 0x100);
    word(0x1200 + cpolitical_status::PARTIES, 0x1400);
    word(0x1200 + cpolitical_status::PARTIES + 8, (1 << 32) | 1);
    word(0x1400, 0x1500);
    word(0x1500, engine_native_parts::cpolitical_party_layout::VTABLE_RVA.0 as u64);
    word(0x1500 + engine_native_parts::cpolitical_party_layout::ID, 7);
    word(0x1500 + engine_native_parts::cpolitical_party_layout::IDEOLOGY_GROUP, 0x1600);
    word(0x1600, cideology_group::VTABLE_RVA.0 as u64);
    word(0x1600 + cideology_group::ID, 7);
    word(0x1500 + engine_native_parts::cpolitical_party_layout::COUNTRY_LEADERS, 0x1700);
    word(
        0x1500 + engine_native_parts::cpolitical_party_layout::COUNTRY_LEADERS + 8,
        (1 << 32) | 1,
    );
    word(0x1700, 0x1800);
    word(0x1800, engine_native_parts::ccountry_leader_layout::VTABLE_RVA.0 as u64);
    word(0x1800 + engine_native_parts::ccountry_leader_layout::CHARACTER, 0x1a00);
    word(0x1800 + ccountry_leader::LEADER_ID, 12);
    word(0x1800 + 0x140, 0x2500);
    word(0x1a00, ccharacter::VTABLE_RVA.0 as u64);
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
        let mut one = Vec::new();
        assert_eq!(
            reader.party_leaders(index as u32 + 1, &mut one),
            Outcome::Rows { len: 1 }
        );
        *destination = one.swap_remove(0);
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
    let mut input = engine_research::research_inputs::Row::default();
    input.slot.country = 11;
    input.slot.index = 2;
    input.slot.technology_present = 1;
    input.slot.technology_handle = 77;
    input.template_index = 3;
    input.slot.database_index = 3;
    let mut token = engine_native_memory::msvc_string::Text::default();
    token.bytes[..3].copy_from_slice(b"abc");
    token.len = 3;
    input.token = engine_research::research_inputs::Text::Inline(token);
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
    assert_eq!(
        rows[0].database_index,
        replay_data::rows::Recorded::Value(3)
    );
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
            assert!(replay_data::cadence::should_walk(table, hour, false));
        }
    }
}

#[test]
fn hourly_modifier_conversion_preserves_identity_signed_operands_and_source_text() {
    let mut captured = Captured::vacant();
    let mut row = engine_research::research_modifiers::Row {
        country: 11,
        index: u32::MAX,
        ordinal: 7,
        kind: engine_research::research_modifiers::Kind::CountrySource,
        raw: i64::MIN,
        auxiliary: i64::MAX,
        count: u32::MAX,
        source_id: 91,
        ..Default::default()
    };
    let mut name = engine_native_memory::msvc_string::Text::default();
    name.bytes[..4].copy_from_slice(b"idea");
    name.len = 4;
    row.name = engine_research::research_inputs::Text::Inline(name);
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
fn owned_governance_failure_preserves_cadence() {
    for force_full in [false, true] {
        let mut captured = Captured::vacant();
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
        assert_eq!(captured.definition_text.len, 0);
        assert_eq!(captured.definition_text.required, 0);
        assert_eq!(captured.modifier_values.len, 0);
        assert_eq!(captured.modifier_values.required, 0);
        assert_eq!(captured.dictionary_required, 0);
        for (index, outcome) in captured.outcomes().into_iter().enumerate() {
            let skipped =
                !replay_data::cadence::should_walk(TABLES[index], context.hour, force_full);
            assert_eq!(
                outcome,
                Outcome::Unavailable {
                    reason: if skipped {
                        replay_data::cadence::SKIPPED_REASON
                    } else {
                        "worker stopped"
                    },
                },
                "{index}",
            );
        }
    }
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

    let mut partial_cap = Capacities::default();
    partial_cap.rows[2] = 4;
    let mut partial = Captured::with_capacities(&partial_cap);
    partial.technologies.rows.reserve_exact(12);
    partial.technologies.outcome = Outcome::Truncated {
        len: 4,
        unread_rows: 8,
    };
    collector_test_support::with_allocations_denied(|| {
        engine_allocation::advisory(|| partial.grow(&mut partial_cap));
    });
    assert_eq!(partial.technologies.rows.len(), 4);
    assert_eq!(partial.technology_pointers.len(), 4);
    partial.grow(&mut partial_cap);
    assert_eq!(partial.technologies.rows.len(), 16);
    assert_eq!(partial.technology_pointers.len(), 16);
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

#[cfg(test)]
pub mod focus_test_kit {
    use super::{Capacities, Captured};
    use engine_focus::{
        ccontinuous_national_focus, cnational_focus, cnational_focus_progress, cnational_focus_tree,
    };
    use engine_native_memory::memory::SliceMemory;
    use engine_native::{ccountry, cgame_state};

    pub const BASE: usize = 0x1400_0000;
    const MEM: usize = 0x1_0000;
    const GS: usize = 0x1_0000;
    const CTAB: usize = 0x1_1000;
    const LTAB: usize = 0x1_1200;
    const C1: usize = 0x1_2000;
    pub const C2: usize = 0x1_4000;
    pub const R1: usize = 0x1_6000;
    pub const R2: usize = 0x1_7000;
    const TT1: usize = 0x1_8000;
    const FC1: usize = 0x1_8200;
    pub const FC2: usize = 0x1_8300;
    const CC1: usize = 0x1_8400;
    const FA: usize = 0x1_8500;
    const FB: usize = 0x1_8600;
    const FC: usize = 0x1_8700;
    const RS1: usize = 0x1_8800;
    const RS2: usize = 0x1_8900;
    const HB: usize = 0x1_8a00;
    const HEAP_TOKEN: &[u8] = b"completed_heap_token_longer_than_fifteen";

    pub struct Fixture {
        pub bytes: Vec<u8>,
    }

    impl Fixture {
        pub fn w64(&mut self, addr: usize, value: u64) {
            let at = addr - MEM;
            self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }

        pub fn w32(&mut self, addr: usize, value: u32) {
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
            self.w64(country, (BASE + ccountry::VTABLE_RVA.0) as u64);
            self.w32(country + ccountry::ID, id);
            self.w32(country + ccountry::OWNED_STATE_COUNT, 1);
            self.w64(country + ccountry::FOCUS_TREE, tree as u64);
            self.w64(country + ccountry::FOCUS_PROGRESS, root as u64);
        }

        #[expect(
            clippy::too_many_arguments,
            reason = "writes every field of one fake research root"
        )]
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
            self.w64(root, (BASE + cnational_focus_progress::VTABLE_RVA.0) as u64);
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

        pub fn world() -> Self {
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

        pub fn memory(&self) -> SliceMemory<'_> {
            SliceMemory {
                base: MEM,
                bytes: &self.bytes,
            }
        }

        pub fn captured() -> Captured {
            Captured::with_capacities(&Capacities::default())
        }
    }
}

#[test]
fn daily_governance_sources_keep_politics_and_research_siblings_independent() {
    use focus_test_kit::{BASE, Fixture};
    struct Sources<'a> {
        memory: engine_native_memory::memory::SliceMemory<'a>,
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
        fixture.w64(politics, (BASE + engine_native_parts::cpolitical_status_layout::VTABLE_RVA.0) as u64);
        fixture.w64(politics + engine_native_parts::cpolitical_status_layout::COUNTRY, country as u64);
    }
    for fail_ideas in [false, true] {
        for hour in 0..24 {
            let memory = engine_native_memory::memory::CountedMemory::new(Sources {
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
            assert!(memory.0.ideas.get() > 0, "hour={hour}");
            let (index, slot) = (3, 11);
            assert_eq!(
                counts[TABLES[index].index()].spans > 0,
                hour == slot,
                "table={:?} hour={hour}",
                TABLES[index]
            );
            for index in [0, 2] {
                assert!(counts[TABLES[index].index()].spans > 0, "hour={hour}");
            }
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
        fixture.w64(status, (BASE + ctechnology_status::VTABLE_RVA.0) as u64);
        fixture.w64(status + ctechnology_status::COUNTRY, country as u64);
        fixture.w64(
            status + ctechnology_status::TECHNOLOGIES_DATA,
            roster as u64,
        );
        for offset in [8, 12] {
            fixture.w32(status + ctechnology_status::TECHNOLOGIES_DATA + offset, 1);
        }
        fixture.w64(roster, technology as u64);
        fixture.w64(technology, (BASE + ctechnology::VTABLE_RVA.0) as u64);
        fixture.w32(technology + ctechnology::DEFINITION_INDEX, 3);
        fixture.w64(technology + ctechnology::STATUS, status as u64);
        fixture.w64(technology + ctechnology::TEMPLATE, TEMPLATE as u64);
    }
    fixture.w64(TEMPLATE + ctechnology_template::ID, HEAP as u64);
    fixture.w64(TEMPLATE + ctechnology_template::ID + 16, LENGTH as u64);
    fixture.w64(TEMPLATE + ctechnology_template::ID + 24, LENGTH as u64);
    fixture.w32(TEMPLATE + ctechnology_template::DATABASE_INDEX, 1);
    fixture.w32(TEMPLATE + ctechnology_template::TECHNOLOGY_HANDLE, 3);
    struct Fault<'a>(engine_native_memory::memory::SliceMemory<'a>);
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

fn technology_fixture() -> focus_test_kit::Fixture {
    use focus_test_kit::{BASE, Fixture};
    let mut fixture = Fixture::world();
    for (owner, country) in [0x1_2000, 0x1_4000].into_iter().enumerate() {
        let status = 0x1_a000 + owner * 0x200;
        let roster = 0x1_a400 + owner * 0x20;
        fixture.w64(country + ccountry::TECHNOLOGY_STATUS, status as u64);
        fixture.w64(status, (BASE + ctechnology_status::VTABLE_RVA.0) as u64);
        fixture.w64(status + ctechnology_status::COUNTRY, country as u64);
        fixture.w64(
            status + ctechnology_status::TECHNOLOGIES_DATA,
            roster as u64,
        );
        for offset in [8, 12] {
            fixture.w32(status + ctechnology_status::TECHNOLOGIES_DATA + offset, 2);
        }
        for index in 0..2 {
            let technology = fixture_technology(owner, index);
            let definition = index;
            let template = 0x1_c000 + definition * 0x600;
            fixture.w64(roster + index * 8, technology as u64);
            fixture.w64(technology, (BASE + ctechnology::VTABLE_RVA.0) as u64);
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
    fixture
}

fn fixture_technology(owner: usize, index: usize) -> usize {
    0x1_b000 + (owner * 2 + index) * 0x200
}

fn assign_slots(fixture: &mut focus_test_kit::Fixture, owner: usize, assigned: &[usize]) {
    use focus_test_kit::BASE;
    let status = 0x1_a000 + owner * 0x200;
    let slots = 0x1_d000 + owner * 0x400;
    let roster = status + ctechnology_status::RESEARCH_SLOTS_DATA;
    fixture.w64(roster, slots as u64);
    fixture.w32(roster + 8, assigned.len() as u32);
    fixture.w32(roster + 12, assigned.len() as u32);
    for (position, &technology) in assigned.iter().enumerate() {
        let slot = slots + 0x100 + position * 0x80;
        fixture.w64(slots + position * 8, slot as u64);
        fixture.w64(slot, (BASE + cresearch_slot::VTABLE_RVA.0) as u64);
        fixture.w32(slot + cresearch_slot::TECHNOLOGY_HANDLE, 0x8000_0001);
        fixture.w64(slot + cresearch_slot::STATUS, status as u64);
        fixture.w64(slot + cresearch_slot::TECHNOLOGY, technology as u64);
        fixture.w64(slot + cresearch_slot::RESEARCH_SPEED, 100_000);
        fixture.w32(slot + cresearch_slot::INDEX, position as u32);
    }
}

#[test]
fn first_research_capture_grows_all_storage_and_keeps_native_and_definition_order() {
    use focus_test_kit::BASE;
    let fixture = technology_fixture();
    struct Reads<'a> {
        memory: engine_native_memory::memory::SliceMemory<'a>,
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
    captured.technology_selection = SliceSelection::new();
    captured.technology_selection.reset(0, true);
    let mut store = SliceFingerprints::new();
    let changes = crate::country_input_changes::Domain::enabled(64);
    let result = acquire(
        &memory,
        BASE,
        0x1_0000,
        0,
        captured.buffers(),
        &enabled,
        Slices {
            technologies: &mut store,
            focus: &mut SliceFingerprints::new(),
            changes: &changes,
        },
        None,
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
    fn read(
        captured: &mut Captured,
        memory: &impl Memory,
        enabled: &[bool; N],
        (hour, force_full): (i32, bool),
        store: &mut SliceFingerprints,
        changes: &crate::country_input_changes::Domain,
    ) -> (Vec<u32>, usize) {
        captured.technology_selection.reset(hour, force_full);
        let result = acquire(
            memory,
            BASE,
            0x1_0000,
            hour,
            captured.buffers(),
            enabled,
            Slices {
                technologies: store,
                focus: &mut SliceFingerprints::new(),
                changes,
            },
            None,
        );
        let owners = captured.technologies.rows[..result.outcomes[2].len()]
            .iter()
            .map(|row| row.country)
            .collect();
        (owners, result.outcomes[4].len())
    }
    enabled[3] = true;
    let mut at = |hour, force_full, store: &mut SliceFingerprints| {
        read(
            &mut captured,
            &memory,
            &enabled,
            (hour, force_full),
            store,
            &changes,
        )
    };
    assert_eq!(at(1, true, &mut store), (vec![1, 1, 2, 2], 2));
    assert_eq!(at(25, false, &mut store), (vec![1, 1], 0));
    assert_eq!(at(3, false, &mut store), (vec![], 0));
    assert_eq!(at(4, true, &mut store), (vec![1, 1, 2, 2], 2));
    let selection = &captured.technology_selection;
    assert!(selection.live(1) && selection.live(2) && !selection.live(3));
    selection.invalidate(&mut store);
    let mut at = |hour, force_full, store: &mut SliceFingerprints| {
        read(
            &mut captured,
            &memory,
            &enabled,
            (hour, force_full),
            store,
            &changes,
        )
        .0
    };
    assert_eq!(at(5, false, &mut store), [1, 1, 2, 2]);
    assert!(at(6, false, &mut store).is_empty());
    store.forget();
    assert_eq!(at(7, false, &mut store), [1, 1, 2, 2]);
}

#[test]
fn a_marked_technology_roster_is_read_within_the_hour_and_an_unmarked_change_is_carried() {
    use crate::country_input_changes::{Domain, Table};
    let mut fixture = technology_fixture();
    assign_slots(&mut fixture, 0, &[fixture_technology(0, 0)]);
    let changes = Domain::enabled(64);
    let mut captured = Captured::vacant();
    let mut enabled = [false; N];
    enabled[2] = true;
    enabled[3] = true;
    enabled[4] = true;
    let mut store = SliceFingerprints::new();
    let mut at = |fixture: &focus_test_kit::Fixture, hour: i32| {
        captured.technology_selection.reset(hour, false);
        let result = acquire(
            &fixture.memory(),
            focus_test_kit::BASE,
            0x1_0000,
            hour,
            captured.buffers(),
            &enabled,
            Slices {
                technologies: &mut store,
                focus: &mut SliceFingerprints::new(),
                changes: &changes,
            },
            None,
        );
        let rows = captured.technologies.rows[..result.outcomes[2].len()].to_vec();
        let mut owners: Vec<u32> = rows.iter().map(|row| row.country).collect();
        owners.dedup();
        (owners, rows)
    };
    assert_eq!(at(&fixture, 5).0, [1, 2]);
    assert!(at(&fixture, 6).0.is_empty());
    changes.write(Table::Technologies, 2);
    assert_eq!(at(&fixture, 7).0, [2]);
    assert!(at(&fixture, 8).0.is_empty());
    changes.write(Table::Research, 1);
    changes.write_every(Table::Research);
    assert!(at(&fixture, 9).0.is_empty());
    fixture.w64(
        fixture_technology(0, 0) + ctechnology::LIVE_PROGRESS,
        260_000,
    );
    let (owners, rows) = at(&fixture, 25);
    assert_eq!(owners, [1]);
    assert_eq!(rows[0].progress_raw, 260_000);
    fixture.w32(fixture_technology(1, 1) + ctechnology::LEVEL, 3);
    assert!(at(&fixture, 27).0.is_empty());
    let (owners, rows) = at(&fixture, 50);
    assert_eq!(owners, [2]);
    assert_eq!(rows[1].level, 0);
}

fn focus_hour(
    captured: &mut Captured,
    fixture: &focus_test_kit::Fixture,
    hour: i32,
    store: &mut SliceFingerprints,
    changes: &crate::country_input_changes::Domain,
) -> Vec<u32> {
    let mut enabled = [false; N];
    enabled[0] = true;
    enabled[1] = true;
    captured.focus_selection.reset_daily(hour, false);
    let result = acquire(
        &fixture.memory(),
        focus_test_kit::BASE,
        0x1_0000,
        hour,
        captured.buffers(),
        &enabled,
        Slices {
            technologies: &mut SliceFingerprints::new(),
            focus: store,
            changes,
        },
        None,
    );
    assert!(matches!(result.outcomes[1], Outcome::Rows { .. }));
    captured.focus.rows[..result.outcomes[0].len()]
        .iter()
        .map(|row| row.country)
        .collect()
}

#[test]
fn the_focus_boundary_reads_only_countries_owning_a_state() {
    use crate::country_input_changes::Domain;
    use focus_test_kit::C2;
    let mut fixture = focus_test_kit::Fixture::world();
    let changes = Domain::enabled(64);
    let mut captured = focus_test_kit::Fixture::captured();
    let mut store = SliceFingerprints::new();
    assert_eq!(
        focus_hour(&mut captured, &fixture, 5, &mut store, &changes),
        [1, 2]
    );
    fixture.w32(C2 + ccountry::OWNED_STATE_COUNT, 0);
    assert!(focus_hour(&mut captured, &fixture, 6, &mut store, &changes).is_empty());
    assert_eq!(
        focus_hour(&mut captured, &fixture, 24, &mut store, &changes),
        [1, 2]
    );
    assert!(focus_hour(&mut captured, &fixture, 25, &mut store, &changes).is_empty());
    assert_eq!(
        focus_hour(&mut captured, &fixture, 48, &mut store, &changes),
        [1]
    );
    fixture.w32(C2 + ccountry::OWNED_STATE_COUNT, 3);
    assert!(focus_hour(&mut captured, &fixture, 50, &mut store, &changes).is_empty());
    assert_eq!(
        focus_hour(&mut captured, &fixture, 72, &mut store, &changes),
        [1, 2]
    );
}

#[test]
fn focus_is_read_after_the_daily_tick_and_within_the_hour_of_a_marked_write() {
    use crate::country_input_changes::{Domain, Table};
    use engine_focus::cnational_focus_progress as progress;
    use focus_test_kit::{FC2, R1, R2};
    let mut fixture = focus_test_kit::Fixture::world();
    let changes = Domain::enabled(64);
    let mut captured = focus_test_kit::Fixture::captured();
    let mut store = SliceFingerprints::new();
    assert_eq!(
        focus_hour(&mut captured, &fixture, 5, &mut store, &changes),
        [1, 2]
    );
    assert!(focus_hour(&mut captured, &fixture, 1, &mut store, &changes).is_empty());
    assert!(focus_hour(&mut captured, &fixture, 6, &mut store, &changes).is_empty());
    fixture.w64(R2 + progress::PROGRESS, 4_000);
    changes.write(Table::Focus, 2);
    assert_eq!(
        focus_hour(&mut captured, &fixture, 7, &mut store, &changes),
        [2]
    );
    assert_eq!(captured.focus.rows[0].progress, 4_000);
    assert!(focus_hour(&mut captured, &fixture, 8, &mut store, &changes).is_empty());
    fixture.w64(R1 + progress::PROGRESS, 99_000);
    fixture.w64(R2 + progress::PROGRESS, 9_000);
    assert_eq!(
        focus_hour(&mut captured, &fixture, 24, &mut store, &changes),
        [1, 2]
    );
    assert_eq!(captured.focus.rows[0].progress, 99_000);
    fixture.w64(R1 + progress::CURRENT, FC2 as u64);
    assert!(focus_hour(&mut captured, &fixture, 25, &mut store, &changes).is_empty());
    assert_eq!(
        focus_hour(&mut captured, &fixture, 48, &mut store, &changes),
        [1, 2]
    );
    assert!(focus_hour(&mut captured, &fixture, 49, &mut store, &changes).is_empty());
}

#[test]
fn an_unmarked_technology_slice_reads_only_progress() {
    use crate::country_input_changes::{Domain, Table};
    let mut fixture = technology_fixture();
    let changes = Domain::enabled(64);
    let mut captured = Captured::vacant();
    let mut enabled = [false; N];
    enabled[2] = true;
    enabled[3] = true;
    enabled[4] = true;
    let mut store = SliceFingerprints::new();
    struct Reads<'a> {
        memory: engine_native_memory::memory::SliceMemory<'a>,
        addresses: std::cell::RefCell<Vec<usize>>,
    }
    impl Memory for Reads<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.addresses.borrow_mut().push(address);
            self.memory.copy(address, destination)
        }
    }
    let mut at = |fixture: &focus_test_kit::Fixture, hour: i32| {
        captured.technology_selection.reset(hour, false);
        let memory = Reads {
            memory: fixture.memory(),
            addresses: Default::default(),
        };
        let result = acquire(
            &memory,
            focus_test_kit::BASE,
            0x1_0000,
            hour,
            captured.buffers(),
            &enabled,
            Slices {
                technologies: &mut store,
                focus: &mut SliceFingerprints::new(),
                changes: &changes,
            },
            None,
        );
        let rows = captured.technologies.rows[..result.outcomes[2].len()].to_vec();
        (rows, memory.addresses.into_inner())
    };
    assign_slots(&mut fixture, 0, &[fixture_technology(0, 1)]);
    at(&fixture, 5);
    fixture.w64(
        fixture_technology(0, 1) + ctechnology::LIVE_PROGRESS,
        310_000,
    );
    let (carried, addresses) = at(&fixture, 25);
    let unassigned = fixture_technology(0, 0);
    assert!(
        addresses.iter().all(|&address| {
            !(unassigned..unassigned + 0x200).contains(&address)
                && (0..2).all(|template| {
                    address != 0x1_c000 + template * 0x600 + ctechnology_template::ID
                })
        }),
        "a carried slice reads no unassigned technology and no definition"
    );
    assert_eq!(carried.len(), 2);
    assert_eq!(carried[1].progress_raw, 310_000);
    changes.write(Table::Technologies, 1);
    let (full, _) = at(&fixture, 27);
    assert_eq!(carried, full);
}

struct Faulty<'a> {
    memory: engine_native_memory::memory::SliceMemory<'a>,
    address: usize,
    remaining: std::cell::Cell<u32>,
    progress: std::cell::RefCell<Vec<usize>>,
}

impl Memory for Faulty<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if (address & 0xff) == ctechnology::LIVE_PROGRESS & 0xff
            && (0x1_b000..0x1_b800).contains(&address)
        {
            self.progress.borrow_mut().push(address);
        }
        if address == self.address && self.remaining.get() != 0 {
            self.remaining.set(self.remaining.get() - 1);
            return false;
        }
        self.memory.copy(address, destination)
    }
}

#[test]
fn a_carried_roster_reads_progress_in_roster_order_and_keeps_a_persistent_failure() {
    use crate::country_input_changes::Domain;
    let mut fixture = technology_fixture();
    assign_slots(
        &mut fixture,
        0,
        &[fixture_technology(0, 1), fixture_technology(0, 0)],
    );
    let changes = Domain::enabled(64);
    let mut captured = Captured::vacant();
    let mut enabled = [false; N];
    enabled[2] = true;
    enabled[3] = true;
    enabled[4] = true;
    let mut store = SliceFingerprints::new();
    let mut at = |fixture: &focus_test_kit::Fixture, hour: i32, address: usize, faults: u32| {
        captured.technology_selection.reset(hour, false);
        let memory = Faulty {
            memory: fixture.memory(),
            address,
            remaining: std::cell::Cell::new(faults),
            progress: Default::default(),
        };
        let result = acquire(
            &memory,
            focus_test_kit::BASE,
            0x1_0000,
            hour,
            captured.buffers(),
            &enabled,
            Slices {
                technologies: &mut store,
                focus: &mut SliceFingerprints::new(),
                changes: &changes,
            },
            None,
        );
        (result.outcomes[2], memory.progress.into_inner())
    };
    assert!(matches!(at(&fixture, 5, 0, 0).0, Outcome::Rows { .. }));
    let first = fixture_technology(0, 0) + ctechnology::LIVE_PROGRESS;
    let second = fixture_technology(0, 1) + ctechnology::LIVE_PROGRESS;
    let (outcome, progress) = at(&fixture, 25, 0, 0);
    assert_eq!(outcome, Outcome::Rows { len: 2 });
    assert_eq!(progress[..2], [first, second]);
    let (outcome, progress) = at(&fixture, 49, first, 1);
    assert_eq!(outcome, Outcome::Rows { len: 2 });
    assert_eq!(progress[..3], [first, first, second]);
    let (outcome, _) = at(&fixture, 73, first, u32::MAX);
    assert!(
        matches!(outcome, Outcome::Unavailable { .. }),
        "{outcome:?}"
    );
}

#[test]
fn a_carried_roster_fails_as_the_full_walk_at_each_fault() {
    use crate::country_input_changes::Domain;
    let mut fixture = technology_fixture();
    assign_slots(&mut fixture, 0, &[fixture_technology(0, 1)]);
    let status = 0x1_a000;
    let faults = [
        0x1_2000 + ccountry::TECHNOLOGY_STATUS,
        status,
        status + ctechnology_status::COUNTRY,
        status + ctechnology_status::TECHNOLOGIES_DATA,
        fixture_technology(0, 1) + ctechnology::LIVE_PROGRESS,
    ];
    let capture = |store: &mut SliceFingerprints,
                   changes: &Domain,
                   (hour, force_full): (i32, bool),
                   (address, remaining): (usize, u32)| {
        let mut captured = Captured::vacant();
        let mut enabled = [false; N];
        enabled[2] = true;
        enabled[3] = true;
        enabled[4] = true;
        captured.technology_selection.reset(hour, force_full);
        let memory = Faulty {
            memory: fixture.memory(),
            address,
            remaining: std::cell::Cell::new(remaining),
            progress: Default::default(),
        };
        acquire(
            &memory,
            focus_test_kit::BASE,
            0x1_0000,
            hour,
            captured.buffers(),
            &enabled,
            Slices {
                technologies: store,
                focus: &mut SliceFingerprints::new(),
                changes,
            },
            None,
        )
        .outcomes[2]
    };
    for address in faults {
        let full = capture(
            &mut SliceFingerprints::new(),
            &Domain::enabled(64),
            (0, true),
            (address, u32::MAX),
        );
        assert!(
            matches!(full, Outcome::Unavailable { .. }),
            "{address:#x}: {full:?}"
        );
        let changes = Domain::enabled(64);
        let mut store = SliceFingerprints::new();
        assert!(matches!(
            capture(&mut store, &changes, (5, false), (0, 0)),
            Outcome::Rows { .. }
        ));
        let carried = capture(&mut store, &changes, (25, false), (address, u32::MAX));
        assert_eq!(carried, full, "{address:#x}");
    }
}

struct NthFault<'a> {
    memory: engine_native_memory::memory::SliceMemory<'a>,
    address: usize,
    nth: u32,
    persistent: bool,
    reads: std::cell::Cell<u32>,
}

impl Memory for NthFault<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if address == self.address {
            let read = self.reads.get() + 1;
            self.reads.set(read);
            if read == self.nth || (self.persistent && read > self.nth) {
                return false;
            }
        }
        self.memory.copy(address, destination)
    }
}

#[test]
fn two_slots_on_one_technology_read_its_progress_once_in_the_carried_roster() {
    use crate::country_input_changes::Domain;
    let mut fixture = technology_fixture();
    let shared = fixture_technology(0, 1);
    assign_slots(&mut fixture, 0, &[shared, shared]);
    let progress = shared + ctechnology::LIVE_PROGRESS;
    let capture = |store: &mut SliceFingerprints,
                   changes: &Domain,
                   (hour, force_full): (i32, bool),
                   (nth, persistent): (u32, bool)| {
        let mut captured = Captured::vacant();
        let mut enabled = [false; N];
        enabled[2] = true;
        enabled[3] = true;
        enabled[4] = true;
        captured.technology_selection.reset(hour, force_full);
        let memory = NthFault {
            memory: fixture.memory(),
            address: progress,
            nth,
            persistent,
            reads: std::cell::Cell::new(0),
        };
        let outcomes = acquire(
            &memory,
            focus_test_kit::BASE,
            0x1_0000,
            hour,
            captured.buffers(),
            &enabled,
            Slices {
                technologies: store,
                focus: &mut SliceFingerprints::new(),
                changes,
            },
            None,
        )
        .outcomes;
        (outcomes[2..4].to_vec(), memory.reads.get())
    };
    for (nth, persistent) in [(0, false), (2, false), (2, true)] {
        let (full, full_reads) = capture(
            &mut SliceFingerprints::new(),
            &Domain::enabled(64),
            (0, true),
            (nth, persistent),
        );
        let changes = Domain::enabled(64);
        let mut store = SliceFingerprints::new();
        let (held, _) = capture(&mut store, &changes, (5, false), (0, false));
        assert!(
            held.iter()
                .all(|outcome| matches!(outcome, Outcome::Rows { .. }))
        );
        let (carried, carried_reads) =
            capture(&mut store, &changes, (25, false), (nth, persistent));
        assert_eq!(carried_reads, full_reads, "progress reads, fault {nth}");
        match (full[0], carried[0]) {
            (Outcome::Rows { .. }, Outcome::Rows { .. }) => {}
            (full, carried) => assert_eq!(carried, full, "fault {nth}"),
        }
        assert!(matches!(carried[0], Outcome::Rows { .. }), "{carried:?}");
        assert_eq!(carried[1], full[1], "fault {nth}");
        if persistent {
            assert!(
                matches!(carried[1], Outcome::Unavailable { .. }),
                "{carried:?}"
            );
        } else {
            assert!(matches!(carried[1], Outcome::Rows { .. }), "{carried:?}");
        }
    }
}
