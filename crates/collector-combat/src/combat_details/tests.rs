use super::*;

#[test]
fn border_width_projection_preserves_signed_zero_and_ordinary_absence() {
    let mut captured = Captured::with_capacities(&Capacities {
        details: 1,
        ..Capacities::default()
    });
    captured.details.outcome = Outcome::Rows { len: 1 };
    captured.details.rows[0].row.id = 62;
    for combat_width in [i64::MIN, -1, 0, 8_000_000, i64::MAX] {
        captured.details.rows[0].row.border_war = Some(engine_native_land::land_combat_details::BorderWarInputs {
            combat_width,
            ..Default::default()
        });
        let views = captured.tables();
        let TableView::Complete(replay_data::rows::TableRows::LandCombatDetails(rows)) = &views[0]
        else {
            panic!("border details withheld");
        };
        assert_eq!(
            rows[0]
                .border_war
                .require("LandCombatDetails.border_war")
                .unwrap()
                .as_ref()
                .unwrap()
                .combat_width
                .require("LandCombatDetails.border_war.combat_width")
                .unwrap(),
            &combat_width
        );
    }
    captured.details.rows[0].row.border_war = None;
    let views = captured.tables();
    let TableView::Complete(replay_data::rows::TableRows::LandCombatDetails(rows)) = &views[0]
    else {
        panic!("ordinary details withheld");
    };
    assert!(
        rows[0]
            .border_war
            .require("LandCombatDetails.border_war")
            .unwrap()
            .is_none()
    );
}

#[test]
fn side_inputs_project_the_copied_cas_and_commander_operands() {
    let mut captured = Captured::with_capacities(&Capacities {
        details: 1,
        detail_text: 32,
        timed_traits: 2,
        learning_traits: 3,
        ..Capacities::default()
    });
    captured.details.outcome = Outcome::Rows { len: 1 };
    captured.detail_text.first[..18].copy_from_slice(b"DESCtrait_atrait_b");
    captured.detail_text.len = 18;
    captured.timed_traits.first[..2].copy_from_slice(&[
        engine_native_land::land_combat_details::CommanderTimedTrait {
            token_id: 5,
            definition_ordinal: 2,
            remaining_days: -1,
        },
        engine_native_land::land_combat_details::CommanderTimedTrait {
            token_id: 6,
            definition_ordinal: 3,
            remaining_days: 0,
        },
    ]);
    captured.timed_traits.len = 2;
    let learning =
        |token_id, experience, required, token_start| engine_native::cunit_leader::CommanderLearningTrait {
            token_id,
            definition_ordinal: token_id + 10,
            required_experience: required,
            experience,
            token_start,
            token_len: 7,
        };
    captured.learning_traits.first[..3].copy_from_slice(&[
        learning(1, 5_000 * 400, 400, 4),
        learning(2, 5_001 * 400, 400, 11),
        learning(3, 1, 0, 4),
    ]);
    captured.learning_traits.len = 3;
    let side = &mut captured.details.rows[0].row.sides[0];
    side.commander = 0x1234_0000_0021;
    side.anti_air_attack = 7_654_321;
    side.damage = [1, 2, 3, 4, 1_234_567, 89];
    side.commander_experience = 345_678_900;
    side.next_skill = Some(engine_native_naval::naval_commander_inputs::SkillDefinition {
        present: 1,
        valid: 1,
        required_experience: 800,
        ..Default::default()
    });
    side.learning_threshold = Some(5_000);
    side.skill_modifiers = Some([-12_345, 67_891]);
    side.commander_description_token = engine_native_land::land_combat_details::Span { start: 0, len: 4 };
    side.timed_traits_start = 0;
    side.timed_traits_len = 2;
    side.learning_traits = engine_native_land::land_combat_details::Span { start: 0, len: 3 };
    let views = captured.tables();
    let TableView::Complete(replay_data::rows::TableRows::LandCombatDetails(rows)) = &views[0]
    else {
        panic!("details withheld");
    };
    let [inputs, other] = rows[0]
        .side_inputs
        .require("LandCombatDetails.side_inputs")
        .unwrap();
    assert_eq!(inputs.anti_air_attack, 7_654_321);
    assert_eq!(inputs.prevented_damage, [1_234_567, 89]);
    assert_eq!(inputs.skill_progress, 432_098);
    assert_eq!(inputs.skill_modifiers, Some([-12_345, 67_891]));
    assert_eq!(inputs.description_token, b"DESC");
    assert_eq!(
        inputs
            .timed_traits
            .iter()
            .map(|t| (t.token_id, t.definition_ordinal, t.remaining_days))
            .collect::<Vec<_>>(),
        [(5, 2, -1), (6, 3, 0)]
    );
    assert_eq!(
        inputs
            .learning_traits
            .as_ref()
            .unwrap()
            .iter()
            .map(|t| (
                t.token_id,
                t.definition_ordinal,
                t.progress,
                t.token.as_slice()
            ))
            .collect::<Vec<_>>(),
        [
            (2, 12, 5_001, b"trait_b".as_slice()),
            (3, 13, 0xffff_ffff, b"trait_a".as_slice())
        ]
    );
    assert_eq!(
        other.skill_progress,
        replay_data::rows::LandCombatSideInputs::NO_PROGRESS
    );
    assert_eq!(other.learning_traits, Some(Vec::new()));
    assert_eq!(other.skill_modifiers, Some([0; 2]));
    assert!(other.description_token.is_empty());
    let side = &mut captured.details.rows[0].row.sides[0];
    side.next_skill.as_mut().unwrap().valid = 0;
    let views = captured.tables();
    let TableView::Complete(replay_data::rows::TableRows::LandCombatDetails(rows)) = &views[0]
    else {
        panic!("details withheld");
    };
    assert_eq!(
        rows[0].side_inputs.require("side_inputs").unwrap()[0].skill_progress,
        replay_data::rows::LandCombatSideInputs::NO_PROGRESS
    );
    let side = &mut captured.details.rows[0].row.sides[0];
    side.next_skill = None;
    side.learning_threshold = None;
    side.skill_modifiers = None;
    side.side_input_refusals = [Some("next"), Some("floor"), Some("block")];
    let views = captured.tables();
    let TableView::Complete(replay_data::rows::TableRows::LandCombatDetails(rows)) = &views[0]
    else {
        panic!("details withheld");
    };
    let inputs = &rows[0].side_inputs.require("side_inputs").unwrap()[0];
    assert_eq!(
        inputs.skill_progress,
        replay_data::rows::LandCombatSideInputs::UNREAD_PROGRESS
    );
    assert_eq!(
        (&inputs.skill_modifiers, &inputs.learning_traits),
        (&None, &None)
    );
    assert_eq!(inputs.anti_air_attack, 7_654_321);
    assert_eq!(inputs.description_token, b"DESC");
}

#[test]
fn experience_ratio_keeps_the_native_sentinel_and_zero_requirement_words() {
    for (experience, required, ratio) in [
        (0, 100, 0),
        (-250_000, 100, -2_500),
        (i64::MAX, 100, 0xffff_ffff),
        (i64::MIN, 100, 0xffff_ffff),
        (i64::MIN + 1, 100, 0xffff_ffff),
        (
            i64::MIN + 2,
            100,
            (i64::MIN + 2).wrapping_mul(100_000) / 10_000_000,
        ),
        (1_000, 0, 0xffff_ffff),
        (1_000, -1, -1_000),
    ] {
        assert_eq!(
            super::experience_ratio(experience, required),
            ratio,
            "{experience} / {required}"
        );
    }
}

#[test]
fn modifier_capacity_and_division_growth_keep_independent_failures() {
    let mut image = vec![0_u8; 0x4000];
    for (address, value) in [
        (0x1000, engine_native::carmy::VTABLE_RVA.0 as u64),
        (0x1018, 0x1234_0000_0033),
        (0x1170, 0x3000),
        (0x1178, 2_u64 << 32 | 2),
        (0x3000, 27),
        (0x3008, (-900_i64) as u64),
        (0x3010, 800),
        (0x3018, 2),
    ] {
        image[address..address + 8].copy_from_slice(&value.to_le_bytes());
    }
    let memory = engine_native_memory::memory::SliceMemory {
        base: 0,
        bytes: &image,
    };
    let mut sides = vec![
        ModifierSide {
            combat: 62,
            ..Default::default()
        },
        ModifierSide {
            combat: 62,
            side: 1,
            ..Default::default()
        },
    ];
    let mut members = vec![ModifierMember::default(); 2];
    let mut values = vec![engine_native_land::division_combat_inputs::Modifier::default(); 1];
    let mut test_sources = Sources::new();
    let mut images = Images::default();
    let mut scratch = Scratch::default();
    let mut destination = Destination {
        sources: &mut test_sources,
        commanders: Default::default(),
        leaders: &mut engine_native_land::land_combat_details::Leaders::default(),
        images: &mut images,
        scratch: &mut scratch,
        name_groups: &mut engine_native::cname_group_member::GroupKeys::default(),
        names: engine_native::cname_group_member::Names {
            bytes: engine_native_memory::walk::Buffer::Growing(&mut Vec::new()),
            len: 0,
            required: 0,
        },
        reference: false,
        details: &mut Vec::new(),
        commander_traits: &mut Vec::new(),
        detail_text: &mut Vec::new(),
        timed_traits: &mut Vec::new(),
        air_counts: &mut Vec::new(),
        learning_traits: &mut Vec::new(),
        unit_scratch: &mut Vec::new(),
        detail_used: engine_native_land::land_combat_details::DetailCounts::default(),
        detail_required: engine_native_land::land_combat_details::DetailCounts::default(),
        divisions: &mut Vec::new(),
        sides: &mut sides,
        combat_sides: &mut Vec::new(),
        country_tags: &mut Vec::new(),
        country_tags_progress: Progress::default(),
        country_set_limit: Ok(1),
        rosters: &mut Vec::new(),
        members: &mut members,
        modifiers: &mut values,
        width_terms: &mut Vec::new(),
        width_progress: Progress::default(),
        progress: [Progress::default(); N],
        arenas: [Progress::default(); N],
        exile_tags: &mut Vec::new(),
        exile_progress: Progress::default(),
        ticks: [0; N],
    };
    destination.progress[2] = Progress {
        len: 2,
        required: 2,
        failure: None,
    };
    destination.modifier_member(&memory, 0, 0x1000, 0x1234_0000_0033, 0, Some(0));
    assert_eq!(destination.arenas[1].required, 1);
    assert_eq!(destination.arenas[2].required, 2);
    assert_eq!(destination.arenas[2].len, 2);
    destination.finish();
    assert_eq!(
        destination.progress[2].outcome(Outcome::Rows { len: 1 }),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(destination.sides[0].side, 0);
    assert_eq!(destination.sides[0].len, 1);
    assert_eq!(
        destination.modifiers[..2]
            .iter()
            .map(|m| m.id)
            .collect::<Vec<_>>(),
        [27, 2]
    );

    destination.progress = [Progress::default(); N];
    destination.visit(
        &memory,
        0,
        0,
        cland_combat::Source {
            pointer: 0,
            id: 62,
            sides: [0; 2],
            attackers: engine_native_memory::cpdx_array::Array { data: 0, count: 1 },
            defenders: engine_native_memory::cpdx_array::Array { data: 0, count: 0 },
            provinces: engine_native_memory::cpdx_array::Array { data: 0, count: 0 },
        },
        cland_combat::Members::none(),
    );
    assert_eq!(destination.divisions.len(), 1);
    for progress in &destination.progress[..2] {
        assert!(matches!(
            progress.outcome(Outcome::Rows { len: 1 }),
            Outcome::Unavailable { .. }
        ));
    }
    assert_eq!(destination.progress[2].required, 2);
}

#[test]
fn roster_spans_project_committed_and_reserve_lists_in_arena_order() {
    let c = Capacities {
        combats: 2,
        census_participants: 1,
        details: 2,
        divisions: 1,
        sides: 1,
        combat_sides: 1,
        country_tags: 1,
        rosters: 6,
        modifier_members: 1,
        modifiers: 1,
        width_terms: 1,
        exile_tags: 1,
        division_names: 1,
        name_templates: 1,
        name_groups: 1,
        commander_traits: 2,
        detail_text: 2,
        timed_traits: 2,
        air_counts: 2,
        learning_traits: 2,
    };
    let mut captured = Captured::with_capacities(&c);
    captured.details.outcome = Outcome::Rows { len: 2 };
    captured.rosters.len = 6;
    captured
        .rosters
        .first
        .copy_from_slice(&[13, 6, 11, 4, 1, 9]);
    for (row, id, spans) in [
        (0, 0x1000_0000_003e, [(0, 2, 2, 1), (3, 0, 3, 0)]),
        (1, 0x2000_0000_003e, [(3, 0, 3, 0), (3, 1, 4, 2)]),
    ] {
        let detail = &mut captured.details.rows[row].row;
        detail.id = id;
        for (side, (committed_start, committed_len, reserve_start, reserve_len)) in
            detail.sides.iter_mut().zip(spans)
        {
            side.committed_start = committed_start;
            side.committed_len = committed_len;
            side.reserve_start = reserve_start;
            side.reserve_len = reserve_len;
        }
    }
    let [
        TableView::Complete(replay_data::rows::TableRows::LandCombatDetails(details)),
        _,
        _,
        _,
        ..,
    ] = captured.tables()
    else {
        panic!("complete land details were not projected")
    };
    let lists: Vec<_> = details
        .iter()
        .map(|row| {
            row.sides
                .clone()
                .map(|side| [side.committed, side.reserves])
        })
        .collect();
    assert_eq!(
        lists,
        [
            [[vec![13, 6], vec![11]], [vec![], vec![]]],
            [[vec![], vec![]], [vec![4], vec![1, 9]]],
        ]
    );
}

fn destination(c: &mut Captured) -> Destination<'_> {
    Destination {
        sources: &mut c.sources,
        commanders: Default::default(),
        leaders: &mut c.leaders,
        images: &mut c.images,
        scratch: &mut c.scratch,
        reference: false,
        name_groups: &mut c.name_cache,
        names: engine_native::cname_group_member::Names {
            bytes: engine_native_memory::walk::Buffer::Growing(&mut c.division_names.first),
            len: 0,
            required: 0,
        },
        details: &mut c.details.rows,
        divisions: &mut c.divisions.rows,
        sides: &mut c.sides.rows,
        combat_sides: &mut c.combat_sides.rows,
        country_tags: &mut c.country_tags.first,
        country_tags_progress: Progress::default(),
        country_set_limit: Ok(1),
        rosters: &mut c.rosters.first,
        commander_traits: &mut c.commander_traits.first,
        detail_text: &mut c.detail_text.first,
        timed_traits: &mut c.timed_traits.first,
        air_counts: &mut c.air_counts.first,
        learning_traits: &mut c.learning_traits.first,
        unit_scratch: &mut c.unit_scratch,
        detail_used: engine_native_land::land_combat_details::DetailCounts::default(),
        detail_required: engine_native_land::land_combat_details::DetailCounts::default(),
        members: &mut c.members.first,
        modifiers: &mut c.modifiers.first,
        width_terms: &mut c.width_terms.first,
        width_progress: Progress::default(),
        exile_tags: &mut c.exile_tags.first,
        exile_progress: Progress::default(),
        progress: [Progress::default(); N],
        arenas: [Progress::default(); N],
        ticks: [0; N],
    }
}

#[test]
fn an_overflowing_exile_division_is_counted_out_then_kept_after_growth() {
    let mut c = Capacities {
        exile_tags: 3,
        ..Capacities::default()
    };
    let mut captured = Captured::with_capacities(&c);
    let combat = 0x1000_0000_003e;
    let division = |exile| Acquired {
        stats: engine_native_land::division_combat_inputs::Row {
            naming: Some(Default::default()),
            ..Default::default()
        },
        exile,
        context: Default::default(),
        reserve_speed: None,
    };
    let place = |ordinal, exile_start| Placement {
        pointer: 0,
        combat,
        side: 0,
        ordinal,
        province: 7,
        role: 1,
        exile_start,
    };
    let row = |cores, occupiers| {
        engine_native_land::division_combat_inputs::Exile::Row(engine_native_land::division_combat_inputs::ExileRow {
            origin_type: 2,
            country: 4,
            cores,
            occupiers,
            ..Default::default()
        })
    };
    let pass = |captured: &mut Captured, second: engine_native_land::division_combat_inputs::Exile| {
        let mut d = destination(captured);
        grow_vec(d.divisions, 2);
        d.exile_tags[..2].copy_from_slice(&[11, 12]);
        for (ordinal, exile) in [row((0, 2), (2, 0)), second].into_iter().enumerate() {
            d.progress[1].required += 1;
            let start = d.exile_progress.len;
            if let engine_native_land::division_combat_inputs::Exile::Row(_) = exile
                && ordinal == 1
            {
                d.exile_tags[start..start + 3].copy_from_slice(&[21, 22, 23]);
            }
            d.admit(place(ordinal as u32, start), division(exile));
        }
        (d.progress[1], d.exile_progress)
    };
    let (divisions, exile) = pass(
        &mut captured,
        engine_native_land::division_combat_inputs::Exile::DoesNotFit { required: 3 },
    );
    assert_eq!(
        divisions.outcome(Outcome::Rows { len: 1 }),
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
    assert_eq!((exile.len, exile.required), (2, 5));
    let first = captured.divisions.rows[0].exile.unwrap();
    assert_eq!((first.cores, first.occupiers), ((0, 2), (2, 0)));
    captured.exile_tags.len = exile.len;
    captured.exile_tags.required = exile.required;
    captured.grow(&mut c);
    assert!(captured.exile_tags.first.len() >= 5);
    let (divisions, exile) = pass(&mut captured, row((0, 1), (1, 2)));
    assert_eq!(
        divisions.outcome(Outcome::Rows { len: 1 }),
        Outcome::Rows { len: 2 }
    );
    assert_eq!((exile.len, exile.required), (5, 5));
    let second = captured.divisions.rows[1].exile.unwrap();
    assert_eq!((second.cores, second.occupiers), ((2, 1), (3, 2)));
    captured.exile_tags.len = exile.len;
    captured.divisions.outcome = Outcome::Rows { len: 2 };
    let [
        _,
        TableView::Complete(replay_data::rows::TableRows::LandCombatDivisions(rows)),
        _,
        _,
        ..,
    ] = captured.tables()
    else {
        panic!("complete land divisions were not projected")
    };
    assert_eq!(rows[0].exile.as_ref().unwrap().location_cores, [11, 12]);
    assert_eq!(rows[1].exile.as_ref().unwrap().location_cores, [21]);
    assert_eq!(rows[1].exile.as_ref().unwrap().occupiers, [22, 23]);
}

#[test]
fn cas_operands_project_onto_the_side_inputs_alone() {
    let mut captured = Captured::with_capacities(&Capacities {
        details: 1,
        ..Capacities::default()
    });
    captured.details.outcome = Outcome::Rows { len: 1 };
    captured.details.rows[0].row.id = 62;
    let side = &mut captured.details.rows[0].row.sides[1];
    side.anti_air_attack = 1_234_567;
    side.damage = [1, 2, 3, 4, 250_000, i64::MAX];
    let views = captured.tables();
    let TableView::Complete(replay_data::rows::TableRows::LandCombatDetails(rows)) = &views[0]
    else {
        panic!("details withheld");
    };
    let defender = &rows[0].sides[1];
    assert_eq!(defender.damage, [1, 2]);
    assert_eq!(defender.anti_air_attack, RetiredKept::default());
    assert_eq!(defender.ground_damage, RetiredNested::default());
    assert_eq!(defender.prevented_damage, RetiredKept::default());
    let mut row = rows[0].clone();
    replay_data::rows::RowCodec::round_to_display(&mut row);
    let [attacker, defender] = row
        .side_inputs
        .require("LandCombatDetails.side_inputs")
        .unwrap();
    assert_eq!(attacker.prevented_damage, [0, 0]);
    assert_eq!(defender.prevented_damage, [200_000, i64::MAX]);
    assert_eq!(defender.anti_air_attack, 1_234_567);
    assert_eq!(row.cas_inputs(1), Some((1_234_567, [200_000, i64::MAX])));
}

#[test]
fn exile_arena_grows_off_the_pass_from_its_counted_demand() {
    let mut c = Capacities {
        exile_tags: 3,
        ..Capacities::default()
    };
    let mut captured = Captured::with_capacities(&c);
    let before = captured.exile_tags.first.len();
    captured.exile_tags.len = 3;
    captured.exile_tags.required = before * 5 + 3;
    captured.grow(&mut c);
    assert!(captured.exile_tags.first.len() >= before * 5 + 3);
    assert!(c.exile_tags >= before * 5 + 3);
}

#[test]
fn naming_direction_and_exile_operands_project_onto_division_rows() {
    let c = Capacities {
        combats: 1,
        census_participants: 1,
        details: 1,
        divisions: 2,
        sides: 1,
        combat_sides: 1,
        country_tags: 1,
        rosters: 1,
        modifier_members: 1,
        modifiers: 1,
        width_terms: 1,
        exile_tags: 1,
        division_names: 1,
        name_templates: 1,
        name_groups: 1,
        commander_traits: 2,
        detail_text: 2,
        timed_traits: 2,
        air_counts: 2,
        learning_traits: 2,
    };
    let mut captured = Captured::with_capacities(&c);
    captured.divisions.outcome = Outcome::Rows { len: 2 };
    captured.exile_tags.first = vec![99, 7, 8, 11];
    captured.exile_tags.len = 4;
    captured.division_names.first = b"xGER_INF_01Guards".to_vec();
    captured.division_names.len = 17;
    let stats = engine_native_land::division_combat_inputs::Row {
        direction_group: -3,
        categories: 0x4000_0000_0000_0001,
        initiative: 2_500,
        commander: 0x1234_0000_0021,
        naming: Some(engine_native::cname_group_member::Inputs {
            kind: 2,
            order: 14,
            ordered: 1,
            custom: (11, 6),
            group: (1, 10),
            ..Default::default()
        }),
        ..Default::default()
    };
    captured.divisions.rows[0] = Division {
        combat: 0x1000_0000_003e,
        province: 362,
        role: 1,
        stats,
        exile: Some(engine_native_land::division_combat_inputs::ExileRow {
            origin_type: 2,
            country: 0,
            country_fallback: 12,
            cores: (1, 2),
            occupiers: (3, 1),
        }),
        ..Default::default()
    };
    captured.divisions.rows[1] = Division {
        combat: 0x1000_0000_003e,
        ordinal: 1,
        role: 2,
        exile: None,
        reserve_speed: Some(Ok(35_000)),
        ..Default::default()
    };
    let [
        _,
        TableView::Complete(replay_data::rows::TableRows::LandCombatDivisions(divisions)),
        _,
        _,
        ..,
    ] = captured.tables()
    else {
        panic!("complete land divisions were not projected")
    };
    assert_eq!(divisions[0].province, 362);
    assert_eq!(divisions[0].direction_group, -3);
    assert_eq!(divisions[0].categories, 0x4000_0000_0000_0001);
    let own = divisions[0].own.as_ref().unwrap();
    assert_eq!(
        (
            own.naming.kind,
            own.naming.order,
            own.naming.ordered,
            own.naming.group.as_slice(),
            own.naming.custom.as_slice()
        ),
        (2, 14, 1, b"GER_INF_01".as_slice(), b"Guards".as_slice())
    );
    assert_eq!(
        divisions[0].exile,
        Some(LandCombatExileInputs {
            origin_type: 2,
            country: 0,
            country_fallback: 12,
            location_cores: vec![7, 8],
            occupiers: vec![11],
        })
    );
    assert_eq!(divisions[1].exile, None);
    assert_eq!(divisions[0].commander, 0x1234_0000_0021);
    assert_eq!(own.initiative, 2_500);
    assert_eq!(divisions[0].reinforcement_speed, None);
    assert_eq!(divisions[1].reinforcement_speed, Some(35_000));
    captured.divisions.rows[1].reserve_speed = Some(Err("refused"));
    let [
        _,
        TableView::Complete(replay_data::rows::TableRows::LandCombatDivisions(refused)),
        _,
        _,
        ..,
    ] = captured.tables()
    else {
        panic!("complete land divisions were not projected")
    };
    assert_eq!(refused[1].reinforcement_speed, None);
    captured.divisions.rows[0].joined = true;
    let [
        _,
        TableView::Complete(replay_data::rows::TableRows::LandCombatDivisions(joined)),
        _,
        _,
        ..,
    ] = captured.tables()
    else {
        panic!("complete land divisions were not projected")
    };
    assert_eq!((joined[0].own.is_none(), joined[1].own.is_some()), (true, true));
}

#[test]
fn source_failures_and_primary_arena_capacity_are_explicit() {
    let mut c = Capacities {
        combats: 1,
        census_participants: 2,
        details: 1,
        divisions: 2,
        sides: 2,
        combat_sides: 2,
        country_tags: 2,
        rosters: 2,
        modifier_members: 2,
        modifiers: 2,
        width_terms: 2,
        exile_tags: 2,
        division_names: 1,
        name_templates: 1,
        name_groups: 1,
        commander_traits: 1,
        detail_text: 1,
        timed_traits: 1,
        air_counts: 1,
        learning_traits: 1,
    };
    let mut captured = Captured::with_capacities(&c);
    let memory = engine_native_memory::memory::SliceMemory {
        base: 0,
        bytes: &[],
    };
    captured.capture(&memory, 0, 0);
    assert!(
        captured
            .tables()
            .iter()
            .all(|t| matches!(t, TableView::Unavailable { .. }))
    );
    assert!(matches!(
        Progress {
            len: 1,
            required: 1,
            failure: None
        }
        .outcome(Outcome::Truncated {
            len: 1,
            unread_rows: 2
        }),
        Outcome::Unavailable { .. }
    ));
    captured.reset();
    captured.sides.outcome = Outcome::Truncated {
        len: 1,
        unread_rows: 3,
    };
    captured.members.required = 7;
    captured.modifiers.required = 19;
    captured.width_terms.required = 9;
    captured.grow(&mut c);
    assert!(c.sides >= 4 && c.modifier_members >= 7 && c.modifiers >= 19 && c.width_terms >= 9);
    captured.details.outcome = Outcome::Rows { len: 1 };
    captured.divisions.outcome = Outcome::Unavailable {
        reason: "native division read failed",
    };
    captured.sides.outcome = Outcome::Rows { len: 1 };
    assert_eq!(captured.outcomes()[0], Outcome::Rows { len: 1 });
    assert!(matches!(
        captured.outcomes()[1],
        Outcome::Unavailable { .. }
    ));
    assert_eq!(captured.outcomes()[2], Outcome::Rows { len: 1 });
    let allocated = captured.allocated_bytes();
    captured.fail("ordinary worker unavailable");
    assert_eq!(
        captured.outcomes(),
        [Outcome::Unavailable {
            reason: "ordinary worker unavailable"
        }; N],
    );
    assert_eq!(captured.allocated_bytes(), allocated);
}

#[test]
fn combat_sides_project_independently_of_the_larger_tables() {
    let mut captured = Captured::with_capacities(&Capacities {
        country_tags: 5,
        combat_sides: 1,
        ..Capacities::default()
    });
    captured.details.outcome = Outcome::Unavailable {
        reason: "tactic context failed",
    };
    captured.country_tags.first[..5].copy_from_slice(&[7, 0, 9, 4, 2]);
    captured.combat_sides.rows[0] = CombatSides {
        sets: cland_combat::CountrySetsRow {
            combat: 62,
            indicator: 1,
            flags: [0x4000, 0x800],
            start: 0,
            lens: [2, 1, 2, 0],
        },
    };
    captured.combat_sides.outcome = Outcome::Rows { len: 1 };
    let [
        TableView::Unavailable { .. },
        _,
        _,
        TableView::Complete(replay_data::rows::TableRows::LandCombatSides(sides)),
        ..,
    ] = captured.tables()
    else {
        panic!("the side table was not projected independently")
    };
    assert_eq!(
        sides,
        [LandCombatSideRow {
            combat: 62,
            indicator: 1,
            flags: [0x4000, 0x800],
            attacker_primary: vec![7, 0],
            attacker_secondary: vec![9],
            defender_primary: vec![4, 2],
            defender_secondary: vec![],
        }]
    );
}

fn modifier_capture() -> Captured {
    let mut captured = Captured::with_capacities(&Capacities {
        sides: 4,
        modifier_members: 2,
        modifiers: 3,
        ..Capacities::default()
    });
    let sides = [
        (7, 0, None, 0, 2),
        (7, 1, None, 2, 0),
        (9, 0, Some([1, -2, 300_000]), 2, 0),
        (9, 1, Some([-4, 5, i64::MAX]), 2, 0),
    ];
    for (at, (combat, side, border_values, start, len)) in sides.into_iter().enumerate() {
        captured.sides.rows[at] = ModifierSide {
            combat,
            side,
            border_values,
            start,
            len,
        };
    }
    for (at, (division, ordinal, start, len)) in
        [(70, 0, 0, 2), (71, 1, 2, 1)].into_iter().enumerate()
    {
        captured.members.first[at] = ModifierMember {
            division,
            ordinal,
            start,
            len,
        };
    }
    captured.members.len = 2;
    captured.members.required = 2;
    for (at, (id, value)) in [(4, 10), (4, 10), (6, 7_000)].into_iter().enumerate() {
        captured.modifiers.first[at] = engine_native_land::division_combat_inputs::Modifier {
            id,
            value_a: value,
            value_b: -value,
        };
    }
    captured.modifiers.len = 3;
    captured.sides.outcome = Outcome::Rows { len: 4 };
    captured
}

fn modifier_entry(id: u32, value: i64) -> CombatModifierValue {
    CombatModifierValue {
        id,
        value_a: value,
        value_b: -value,
    }
}

#[test]
fn land_combat_division_modifiers_hold_a_row_a_member_in_key_order() {
    let mut captured = modifier_capture();
    let tables = captured.tables();
    let TableView::Complete(replay_data::rows::TableRows::LandCombatDivisionModifiers(rows)) =
        &tables[4]
    else {
        panic!("member rows were not projected")
    };
    let expected = [
        LandCombatDivisionModifierRow {
            combat: 7,
            side: 0,
            ordinal: 0,
            entries: vec![modifier_entry(4, 10), modifier_entry(4, 10)],
            division: Some(70),
        },
        LandCombatDivisionModifierRow {
            combat: 7,
            side: 0,
            ordinal: 1,
            entries: vec![modifier_entry(6, 7_000)],
            division: Some(71),
        },
    ];
    assert_eq!(rows, &expected);
    let TableView::Complete(replay_data::rows::TableRows::LandCombatModifiers(sides)) = &tables[2]
    else {
        panic!("side rows were not projected")
    };
    assert_eq!(sides.len(), 4);
    assert!(sides.iter().all(|side| side.divisions == Retired));
    captured.members.required = 5;
    captured.sides.outcome = Outcome::Truncated {
        len: 2,
        unread_rows: 2,
    };
    assert_eq!(
        captured.outcomes()[4],
        Outcome::Truncated {
            len: 2,
            unread_rows: 3
        }
    );
    let tables = captured.tables();
    let TableView::Truncated {
        rows: replay_data::rows::TableRows::LandCombatDivisionModifiers(truncated),
        missing,
    } = &tables[4]
    else {
        panic!("truncated sides did not publish their members' rows")
    };
    assert_eq!((truncated.as_slice(), *missing), (&expected[..], 3));
    captured.members.required = 2;
    assert_eq!(captured.outcomes()[4], Outcome::Rows { len: 2 });
    captured.sides.outcome = Outcome::Unavailable {
        reason: "land modifiers source changed",
    };
    assert_eq!(
        captured.outcomes()[4],
        Outcome::Unavailable {
            reason: "land modifiers source changed"
        }
    );
    assert!(matches!(
        captured.tables()[4],
        TableView::Unavailable { .. }
    ));
}

#[test]
fn border_values_survive_an_independent_country_set_failure() {
    let mut captured = modifier_capture();
    captured.combat_sides.outcome = Outcome::Unavailable {
        reason: "land combat country list node unreadable",
    };
    let [
        _,
        _,
        TableView::Complete(replay_data::rows::TableRows::LandCombatModifiers(sides)),
        TableView::Unavailable { .. },
        TableView::Complete(replay_data::rows::TableRows::LandCombatDivisionModifiers(_)),
        _,
    ] = captured.tables()
    else {
        panic!("the modifier tables were not projected independently")
    };
    assert_eq!(
        sides
            .iter()
            .map(|side| (side.combat, side.side, side.border_values))
            .collect::<Vec<_>>(),
        [
            (7, 0, None),
            (7, 1, None),
            (9, 0, Some([1, -2, 300_000])),
            (9, 1, Some([-4, 5, i64::MAX])),
        ]
    );
}

#[test]
fn detail_arenas_grow_from_live_demand() {
    let mut c = Capacities::default();
    let mut captured = Captured::with_capacities(&c);
    captured.commander_traits.required = 1_000;
    captured.detail_text.required = 100_000;
    captured.timed_traits.required = 500;
    captured.air_counts.required = 3_000;
    captured.learning_traits.required = 7_000;
    captured.rosters.required = 9_000;
    captured.grow(&mut c);
    assert!(captured.air_counts.first.len() >= 3_000);
    assert!(captured.learning_traits.first.len() >= 7_000);
    assert!(captured.rosters.first.len() >= 9_000);
    assert!(c.commander_traits >= 1_000);
    assert!(c.detail_text >= 100_000);
    assert!(c.timed_traits >= 500);
    assert!(captured.commander_traits.first.len() >= 1_000);
    assert!(captured.detail_text.first.len() >= 100_000);
    assert!(captured.timed_traits.first.len() >= 500);
}

struct ReserveWorld(Vec<u8>);

impl Memory for ReserveWorld {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address == engine_native_parts::modifier_definition::TABLE_RVA.0 && output.len() == 8 {
            output.copy_from_slice(&0x3000_0000_u64.to_le_bytes());
            return true;
        }
        if (0x3000_0000..0x3004_0000).contains(&address) {
            output.fill(0);
            return true;
        }
        engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &self.0,
        }
        .copy(address, output)
    }
}

impl ReserveWorld {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.0[at..at + bytes.len()].copy_from_slice(bytes);
    }
    fn leaf(&mut self, at: usize, value: i64) {
        let entries = at + 0x200;
        self.put(at, &(entries as u64).to_le_bytes());
        self.put(at + 8, &0x1_0000_0001_u64.to_le_bytes());
        self.put(
            entries,
            &engine_executable::modifier_keys::LAND_REINFORCE_RATE.to_le_bytes(),
        );
        self.put(entries + 8, &value.to_le_bytes());
        self.put(at + 0x48, b"Home");
        self.put(at + 0x48 + 16, &4_u64.to_le_bytes());
        self.put(at + 0x48 + 24, &15_u64.to_le_bytes());
    }
}

#[test]
fn reserve_trees_refuse_per_row_and_carry_certified_trees() {
    struct Fixed(std::cell::Cell<(u64, u64)>);
    impl naval_tree_changes::Changes for Fixed {
        fn sequence(&self, _receiver: usize) -> Option<(u64, u64)> {
            Some(self.0.get())
        }
    }
    let mut world = ReserveWorld(vec![0; 0x1000]);
    world.leaf(0x100, 7);
    let mut captured = Captured::vacant();
    for (owner, accumulator) in [(4, 0x100), (5, 0x9000)] {
        let mut row = Division {
            role: 2,
            ..Default::default()
        };
        row.context.owner = owner;
        row.context.country_accumulator = accumulator;
        captured.divisions.rows.push(row);
    }
    captured.divisions.outcome = Outcome::Rows { len: 2 };
    let changes = Fixed(std::cell::Cell::new((1, 1)));
    let rows = |captured: &mut Captured, world: &ReserveWorld| {
        captured.capture_reserve_trees(world, 0, &changes);
        assert!(matches!(captured.reserve_outcome, Outcome::Rows { len: 2 }));
        captured
            .reserve_trees
            .iter()
            .map(|row| (row.owner, row.nodes.as_ref().map(|nodes| nodes[0].value)))
            .collect::<Vec<_>>()
    };
    assert_eq!(rows(&mut captured, &world), [(4, Some(7)), (5, None)]);
    world.put(0x308, &9_i64.to_le_bytes());
    assert_eq!(rows(&mut captured, &world), [(4, Some(7)), (5, None)]);
    assert_eq!(rows(&mut captured, &world), [(4, Some(7)), (5, None)]);
    world.put(0x308, &11_i64.to_le_bytes());
    changes.0.set((1, 2));
    assert_eq!(rows(&mut captured, &world), [(4, Some(11)), (5, None)]);
    captured.divisions.outcome = Outcome::Truncated {
        len: 1,
        unread_rows: 3,
    };
    captured.capture_reserve_trees(&world, 0, &changes);
    assert_eq!(
        captured.reserve_outcome,
        Outcome::Truncated {
            len: 1,
            unread_rows: 3
        }
    );
}

#[test]
fn reserve_tree_carries_stay_linear() {
    const SOURCES: usize = 16;
    const FIRST: usize = 0x1000;
    const STRIDE: usize = 0x300;
    struct Certificates(std::cell::Cell<bool>);
    impl naval_tree_changes::Changes for Certificates {
        fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
            let accumulator = receiver.wrapping_add(engine_native_memory::country_identity::MODIFIER_ACCUMULATOR);
            let index = (accumulator - FIRST) / STRIDE;
            Some((1, if self.0.get() && index % 2 == 1 { 2 } else { 1 }))
        }
    }
    let at = |index: usize| FIRST + index * STRIDE;
    let mut world = ReserveWorld(vec![0; FIRST + SOURCES * STRIDE]);
    for index in 0..SOURCES {
        world.leaf(at(index), index as i64 + 1);
    }
    let mut captured = Captured::vacant();
    for index in (0..SOURCES).rev() {
        let mut row = Division {
            role: 2,
            ..Default::default()
        };
        row.context.owner = index as u32 + 1;
        row.context.country_accumulator = at(index);
        captured.divisions.rows.push(row);
    }
    captured.divisions.outcome = Outcome::Rows { len: SOURCES };
    let changes = Certificates(std::cell::Cell::new(false));
    let pass = |captured: &mut Captured, world: &ReserveWorld| {
        captured.capture_reserve_trees(world, 0, &changes);
        assert_eq!(captured.reserve_outcome, Outcome::Rows { len: SOURCES });
        assert!(
            captured.reserve_carry_probes <= 2 * SOURCES,
            "{} held trees examined for {SOURCES} sources",
            captured.reserve_carry_probes
        );
        captured
            .reserve_trees
            .iter()
            .map(|row| (row.owner, row.nodes.as_ref().map(|nodes| nodes[0].value)))
            .collect::<Vec<_>>()
    };
    let expected = |value: &dyn Fn(usize) -> i64| {
        (0..SOURCES)
            .map(|index| (index as u64 + 1, Some(value(index))))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        pass(&mut captured, &world),
        expected(&|index| index as i64 + 1)
    );
    let unseen = |index: usize| 100_000 + index as i64;
    for index in 0..SOURCES {
        world.put(at(index) + 0x208, &unseen(index).to_le_bytes());
    }
    assert_eq!(
        pass(&mut captured, &world),
        expected(&|index| index as i64 + 1)
    );
    changes.0.set(true);
    assert_eq!(
        pass(&mut captured, &world),
        expected(&|index| if index % 2 == 1 {
            unseen(index)
        } else {
            index as i64 + 1
        })
    );
}
