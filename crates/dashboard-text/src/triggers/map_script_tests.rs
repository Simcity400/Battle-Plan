use super::*;
use crate::catalogue::tests::script_game;

fn set<T: replay_data::rows::TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
    frame.tables[T::ID.index()] =
        replay_frame::frame::SharedView::from(TableView::Complete(TableRows::from_rows::<T>(rows)))
            .with_sampled_at(Some(frame.hour));
}

#[test]
fn map_scripts_use_calculated_stability_war_support_and_native_ai() {
    script_game::enter();
    let mut frame = frame(&[(100, 1, "GER", 1, "GER", "")], &[]);
    set::<tables::CountryStabilityInputs>(
        &mut frame,
        vec![replay_data::rows::CountryStabilityInputRow {
            country: 100,
            stability_raw: 20_000,
            war_support_raw: 10_000,
            stability_factor: Some(30_000),
            party_popularity_stability_factor: Some(0),
            min_stability: 0,
            max_stability: 100_000,
            world_tension_raw: 50_000,
            war_support_tension_impact: 40_000,
            war_support_factor: Some(0),
            min_war_support: 0,
            max_war_support: 100_000,
            ..Default::default()
        }],
    );
    set::<tables::SimulationFeatures>(&mut frame, vec![Default::default()]);
    set::<tables::HumanControllers>(
        &mut frame,
        vec![replay_data::rows::HumanControllerRow {
            country: 100,
            count: 0,
        }],
    );
    set::<tables::RaidCountryInputs>(
        &mut frame,
        vec![replay_data::rows::RaidCountryInputRow {
            canonical_slot: 1,
            autonomous: Some(0),
            ..Default::default()
        }],
    );
    with_world(&frame, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let scope = Scopes::of(Scope::Country(1));
        for (body, expected) in [
            ("has_stability > 0.49", true),
            ("has_stability = 0.5", false),
            ("has_war_support > 0.29", true),
            ("has_war_support < 0.3", false),
            ("is_ai = no", true),
            ("is_ai = yes", false),
        ] {
            assert_eq!(
                evaluator.evaluate(&items(body), scope).decided(),
                Ok(expected),
                "{body}"
            );
        }
        assert_eq!(evaluator.scoped_value(b"stability", scope), Ok(50_000));
    });
}

#[test]
fn map_scripts_ai_control_keeps_signed_counts_and_the_live_player_fallback() {
    script_game::enter();
    for (flag, count, player, override_ai, expected) in [
        (Some(1), 0, false, 0, true),
        (Some(7), 0, false, 0, true),
        (Some(1), -1, false, 0, false),
        (Some(1), 1, false, 0, false),
        (Some(0), 0, false, 0, false),
        (None, 0, true, 0, false),
        (None, 0, false, 0, true),
        (Some(0), 1, true, 1, true),
    ] {
        let mut f = frame(&[(100, 1, "GER", 1, "GER", "")], &[]);
        set::<tables::SimulationFeatures>(
            &mut f,
            vec![replay_data::rows::SimulationFeatureRow {
                human_ai: override_ai,
                ..Default::default()
            }],
        );
        set::<tables::RaidCountryInputs>(
            &mut f,
            vec![replay_data::rows::RaidCountryInputRow {
                canonical_slot: 1,
                autonomous: flag,
                ..Default::default()
            }],
        );
        set::<tables::HumanControllers>(
            &mut f,
            vec![replay_data::rows::HumanControllerRow {
                country: 100,
                count,
            }],
        );
        if player {
            set::<tables::Players>(
                &mut f,
                vec![replay_data::rows::PlayerRow {
                    country: 100,
                    ..Default::default()
                }],
            );
        }
        with_world(&f, "", Err("none"), |world| {
            assert_eq!(Evaluator::new(world, None).country_is_ai(1), Ok(expected));
        });
    }
}

#[test]
fn map_scripts_visit_rosters_in_order_and_keep_duplicate_cores() {
    script_game::enter();
    let mut frame = frame(
        &[(100, 1, "GER", 1, "GER", ""), (200, 2, "AAA", 1, "AAA", "")],
        &[],
    );
    set::<tables::CountryCounterInputs>(
        &mut frame,
        vec![
            CountryCounterInputRow {
                country: 100,
                owned_states: Some(vec![65, 64]).into(),
                neighbors: Some(vec![200]).into(),
                ..Default::default()
            },
            CountryCounterInputRow {
                country: 200,
                owned_state_count: 1,
                ..Default::default()
            },
        ],
    );
    set::<tables::StateCores>(
        &mut frame,
        vec![replay_data::rows::StateCoreRow {
            state: 64,
            cores: vec![200, 200, 100],
            ..Default::default()
        }],
    );
    set::<tables::States>(
        &mut frame,
        vec![
            replay_data::rows::StateRow {
                id: 64,
                ..Default::default()
            },
            replay_data::rows::StateRow {
                id: 65,
                ..Default::default()
            },
        ],
    );
    with_world(&frame, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let scope = Scopes::of(Scope::Country(1));
        for body in [
            "is_neighbor_of = AAA",
            "any_neighbor_country = { tag = AAA }",
            "all_owned_state = { always = yes }",
        ] {
            assert_eq!(
                evaluator.evaluate(&items(body), scope).decided(),
                Ok(true),
                "{body}"
            );
        }
        assert_eq!(
            evaluator
                .evaluate(
                    &items("any_country_with_core = { count = 2 tag = AAA }"),
                    Scopes::of(Scope::State(64))
                )
                .decided(),
            Ok(true)
        );
        assert_eq!(
            evaluator.scoped_value(b"owned_states^num", scope),
            Ok(200_000)
        );
        assert_eq!(
            evaluator.scoped_value(b"owned_states^0", scope),
            evaluator.encode(Scope::State(65))
        );
        assert_eq!(evaluator.scoped_value(b"owned_states^2", scope), Ok(0));
        assert_eq!(
            evaluator.scoped_value(b"global.countries^num", scope),
            Ok(200_000)
        );
        assert_eq!(
            evaluator
                .evaluate(
                    &items("any_of_scopes = { array = owned_states state = 64 }"),
                    scope
                )
                .decided(),
            Ok(true)
        );
    });
}

#[test]
fn map_scripts_clamp_expressions_keep_the_lower_arm_and_evaluate_both_bounds() {
    script_game::enter();
    with_world(&variables_frame(), "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let scope = Scopes::of(Scope::Country(0));
        for (value, min, max, expected) in [
            (1, 2, 9, 2),
            (12, 2, 9, 9),
            (5, 2, 9, 5),
            (1, 9, 2, 9),
            (12, 9, 2, 2),
        ] {
            let script = format!(
                "set_temp_variable = {{ var = result value = {{ value = {value} clamp = {{ min = {min} max = {max} }} }} }}"
            );
            assert_eq!(
                evaluator.evaluate(&items(&script), scope).decided(),
                Ok(true),
                "{script}"
            );
            assert_eq!(evaluator.scoped_value(b"result", scope), Ok(expected * ONE));
        }
    });
}

#[test]
fn map_scripts_state_callbacks_keep_holes_fractional_population_and_recorded_values() {
    script_game::enter();
    let mut f = frame(&[(100, 1, "GER", 1, "GER", "")], &[]);
    set::<tables::States>(
        &mut f,
        vec![replay_data::rows::StateRow {
            id: 64,
            owner: 100,
            controller: 0,
            population: 1234.into(),
            ..Default::default()
        }],
    );
    set::<tables::SimulationFeatures>(
        &mut f,
        vec![replay_data::rows::SimulationFeatureRow {
            state_array_len: Some(80).into(),
            ..Default::default()
        }],
    );
    set::<tables::StateResistance>(
        &mut f,
        vec![replay_data::rows::StateResistanceRow {
            state: 64,
            level: 1_234_567,
            compliance: 3_456_789,
            speed: -123,
            compliance_speed: 456,
            ..Default::default()
        }],
    );
    with_world(&f, "", Err("none"), |world| {
        let eval = Evaluator::new(world, None);
        let scope = Scopes::of(Scope::State(64));
        for (name, value) in [
            (b"state_population".as_slice(), 123_400_000),
            (b"state_population_k", 123_400),
            (b"resistance", 1_234_567),
            (b"compliance", 3_456_789),
            (b"resistance_speed", -123),
            (b"compliance_speed", 456),
            (b"global.states^num", 7_900_000),
        ] {
            assert_eq!(
                eval.scoped_value(name, scope),
                Ok(value),
                "{}",
                String::from_utf8_lossy(name)
            );
        }
        assert_eq!(
            eval.scoped_value(b"global.states^0", scope),
            eval.encode(Scope::State(1))
        );
        assert_eq!(
            eval.scoped_value(b"global.states^78", scope),
            eval.encode(Scope::State(79))
        );
        assert_eq!(eval.scoped_value(b"global.states^79", scope), Ok(0));
        let hole = eval.encode(Scope::State(79)).unwrap();
        let (states, failures) = crate::triggers::refusal::capture(|| {
            (
                eval.scoped_state(b"79", scope),
                eval.scoped_state(b"global.states^78", scope),
                eval.decoded(crate::triggers::variables::Value::Raw(hole), scope),
                eval.decoded(crate::triggers::variables::Value::Raw(79 * ONE), scope),
            )
        });
        assert_eq!(states, (Ok(NULL_STATE), Ok(NULL_STATE), Ok(None), Ok(None)));
        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(
            eval.scoped_value(b"owner", scope),
            eval.encode(Scope::Country(1))
        );
        assert_eq!(
            eval.scoped_value(b"controller", scope),
            Ok(i64::from(u32::from_le_bytes(*b"ZZ0\0") | 0x4000_0000))
        );
    });
}

#[test]
fn map_scripts_state_targets_filter_in_initiator_scope_and_deduplicate() {
    script_game::enter();
    let mut f = frame(&[(100, 1, "GER", 1, "GER", "")], &[]);
    set::<tables::States>(
        &mut f,
        [64, 65]
            .into_iter()
            .map(|id| replay_data::rows::StateRow {
                id,
                ..Default::default()
            })
            .collect(),
    );
    with_world(&f, "", Err("none"), |world| {
        let eval = Evaluator::new(world, None);
        let caller = Scopes::of(Scope::Country(1));
        assert_eq!(eval.evaluate(&items("tag = GER"), caller), Eval::True);
        assert_eq!(eval.scoped_state(b"64", caller), Ok(64));
        assert_eq!(
            eval.evaluate(&items("state = 64"), Scopes::of(Scope::State(64))),
            Eval::True
        );
        let fields = content_reader_parser::parser::parse(
            b"targets = { 65 64 65 } target_root_trigger = { tag = GER } target_trigger = { tag = GER FROM = { state = 64 } } targets_dynamic = yes state_target = yes",
        ).unwrap();
        let targets = content_reader_politics::country_scorers::script_targets(&fields.items);
        assert_eq!(
            targets.targets,
            [b"65".to_vec(), b"64".to_vec(), b"65".to_vec()]
        );
        assert_eq!(
            eval.script_state_target_candidates(&targets, caller),
            Ok(vec![64])
        );
    });
}

#[test]
fn map_scripts_target_selection_shares_temps_across_root_arrays_and_filters() {
    script_game::enter();
    let mut f = frame(&[(100, 1, "GER", 1, "GER", "")], &[]);
    set::<tables::States>(
        &mut f,
        [64, 65]
            .into_iter()
            .map(|id| replay_data::rows::StateRow {
                id,
                owner: 100,
                controller: 100,
                ..Default::default()
            })
            .collect(),
    );
    set::<tables::CountryCounterInputs>(
        &mut f,
        vec![replay_data::rows::CountryCounterInputRow {
            country: 100,
            owned_state_count: 2,
            owned_states: Some(vec![64, 65]).into(),
            ..Default::default()
        }],
    );
    set::<tables::ScriptVariables>(
        &mut f,
        vec![replay_data::rows::ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 100,
            seed: None.into(),
            entries: Vec::new(),
        }],
    );
    with_world(&f, "", Err("none"), |world| {
        let eval = Evaluator::new(world, None);
        let caller = Scopes::of(Scope::Country(1));
        let selection = |body: &str| {
            let parsed = content_reader_parser::parser::parse(body.as_bytes()).unwrap();
            content_reader_politics::country_scorers::script_targets(&parsed.items)
        };
        let states = selection(
            "target_root_trigger = { set_temp_variable = { t = 1 } } target_array = owned_states target_trigger = { check_variable = { t = 1 } FROM = { state = 64 } }",
        );
        assert_eq!(
            eval.script_state_target_candidates(&states, caller),
            Ok(vec![64])
        );
        let countries = selection(
            "target_root_trigger = { set_temp_variable = { t = 1 } } targets = { GER } target_trigger = { check_variable = { t = 1 } }",
        );
        assert_eq!(
            eval.script_target_candidates(&countries, caller),
            Ok(vec![1])
        );
        let accumulated = selection(
            "target_root_trigger = { set_temp_variable = { t = 0 } } target_array = owned_states target_trigger = { add_to_temp_variable = { t = 1 } check_variable = { t = 2 } }",
        );
        assert_eq!(
            eval.script_state_target_candidates(&accumulated, caller),
            Ok(vec![65])
        );
        let mut refused = selection("targets = { 64 }");
        refused.unported.push("unavailable selector".into());
        assert!(
            eval.script_state_target_candidates(&refused, caller)
                .is_err()
        );
        let fresh = selection("targets = { 64 } target_trigger = { check_variable = { t = 0 } }");
        assert_eq!(
            eval.script_state_target_candidates(&fresh, caller),
            Ok(vec![64])
        );
        let country_marker = selection("state_target = yes targets = { GER }");
        assert!(
            eval.script_target_candidates(&country_marker, caller)
                .unwrap_err()
                .contains("state_target selection mode")
        );
    });
}

#[test]
fn map_scripts_state_targets_skip_accepted_duplicates_but_retry_rejections() {
    script_game::enter();
    let mut f = frame(
        &[(100, 1, "GER", 1, "GER", ""), (200, 2, "SOV", 2, "SOV", "")],
        &[],
    );
    set::<tables::States>(
        &mut f,
        [64, 65, 66]
            .into_iter()
            .map(|id| {
                let country = if id == 66 { 200 } else { 100 };
                replay_data::rows::StateRow {
                    id,
                    owner: country,
                    controller: country,
                    ..Default::default()
                }
            })
            .collect(),
    );
    set::<tables::CountryCounterInputs>(
        &mut f,
        vec![
            replay_data::rows::CountryCounterInputRow {
                country: 100,
                owned_state_count: 2,
                owned_states: Some(vec![64, 65]).into(),
                ..Default::default()
            },
            replay_data::rows::CountryCounterInputRow {
                country: 200,
                owned_state_count: 1,
                owned_states: Some(vec![66]).into(),
                ..Default::default()
            },
        ],
    );
    with_world(&f, "", Err("none"), |world| {
        let eval = Evaluator::new(world, None);
        let caller = Scopes::of(Scope::Country(1));
        let selection = |candidates: &str, comparison: &str| {
            let body = format!(
                "target_root_trigger = {{ set_temp_variable = {{ t = 0 }} }} {candidates} target_trigger = {{ add_to_temp_variable = {{ t = 1 }} check_variable = {{ {comparison} }} }}"
            );
            let parsed = content_reader_parser::parser::parse(body.as_bytes()).unwrap();
            content_reader_politics::country_scorers::script_targets(&parsed.items)
        };
        for (candidates, comparison, visits) in [
            ("targets = { 64 64 65 }", "t < 3", 2),
            ("targets = { 64 64 65 }", "t > 1", 3),
            (
                "target_array = owned_states target_array = owned_states targets = { 64 65 }",
                "t < 3",
                2,
            ),
        ] {
            assert_eq!(
                eval.script_state_target_candidates(&selection(candidates, comparison), caller),
                Ok(vec![64, 65]),
                "{candidates}, {comparison}"
            );
            assert_eq!(
                eval.temp_value(b"t"),
                Some(Ok(visits * 100_000)),
                "{candidates}, {comparison}"
            );
        }
        let countries = selection("targets = { GER GER SOV }", "t < 3");
        assert_eq!(
            eval.script_target_candidates(&countries, caller),
            Ok(vec![1])
        );
        assert_eq!(eval.temp_value(b"t"), Some(Ok(300_000)));
    });
}

#[test]
fn map_scripts_daily_callbacks_refuse_stale_values_and_invalidate_reuse() {
    script_game::enter();
    let mut f = frame(&[(100, 1, "GER", 1, "GER", "")], &[]);
    set::<tables::States>(
        &mut f,
        vec![replay_data::rows::StateRow {
            id: 64,
            ..Default::default()
        }],
    );
    set::<tables::StateResistance>(
        &mut f,
        vec![replay_data::rows::StateResistanceRow {
            state: 64,
            level: 123,
            ..Default::default()
        }],
    );
    let read = |world: &World| {
        Evaluator::new(world, None).scoped_value(b"resistance", Scopes::of(Scope::State(64)))
    };
    let (value, mut dependencies) = with_world(&f, "", Err("none"), |world| {
        super::super::inputs::capture(&f, || read(world))
    });
    assert_eq!(value, Ok(123));
    let mut later = f.clone();
    later.hour += 1;
    with_world(&later, "", Err("none"), |world| {
        assert!(
            read(world)
                .unwrap_err()
                .contains("not read this hour, so the values carried from an earlier hour cannot answer this exactly")
        );
        assert!(!dependencies.confirm(&later, |key| match key {
            super::super::inputs::Key::World(query) => Some(world.reread(query)),
            _ => None,
        }));
    });
}

#[test]
fn map_scripts_tech_sharing_uses_the_certified_country_roster_not_global_members() {
    script_game::enter();
    let mut f = frame(&[(100, 1, "GER", 1, "GER", "")], &[]);
    set::<tables::LexicalTokens>(
        &mut f,
        vec![LexicalTokenRow {
            token: 9,
            literal: b"mod_group".to_vec(),
        }],
    );
    set::<tables::ResearchModifiers>(
        &mut f,
        vec![
            replay_data::rows::ResearchModifierRow {
                country: 100,
                index: u32::MAX,
                kind: replay_data::rows::ResearchModifierRow::SHARING_ROSTER,
                raw: -1,
                count: 2,
                ..Default::default()
            },
            replay_data::rows::ResearchModifierRow {
                country: 100,
                index: u32::MAX,
                ordinal: 1,
                kind: replay_data::rows::ResearchModifierRow::SHARING_ROSTER,
                raw: 9,
                ..Default::default()
            },
            replay_data::rows::ResearchModifierRow {
                country: 100,
                index: u32::MAX,
                ordinal: 2,
                kind: replay_data::rows::ResearchModifierRow::SHARING_ROSTER,
                raw: 9,
                ..Default::default()
            },
        ],
    );
    with_world(&f, "", Err("none"), |world| {
        assert_eq!(
            Evaluator::new(world, None).in_tech_sharing_group(1, b"mod_group"),
            Ok(true)
        );
    });
    set::<tables::ResearchModifiers>(
        &mut f,
        vec![replay_data::rows::ResearchModifierRow {
            country: 100,
            index: u32::MAX,
            kind: replay_data::rows::ResearchModifierRow::SHARING_ROSTER,
            raw: -1,
            count: 0,
            ..Default::default()
        }],
    );
    with_world(&f, "", Err("none"), |world| {
        assert_eq!(
            Evaluator::new(world, None).in_tech_sharing_group(1, b"mod_group"),
            Ok(false)
        );
    });
    with_world(&f, "", Err("none"), |world| {
        assert_eq!(
            Evaluator::new(world, None).in_tech_sharing_group(1, b"uninterned_group"),
            Ok(false)
        );
    });
    set::<tables::ResearchModifiers>(&mut f, Vec::new());
    with_world(&f, "", Err("none"), |world| {
        for name in [b"mod_group".as_slice(), b"uninterned_group"] {
            let why = Evaluator::new(world, None)
                .in_tech_sharing_group(1, name)
                .unwrap_err();
            assert!(
                why.contains("research modifiers sharing-roster witness unavailable"),
                "{why}"
            );
        }
    });
}

#[test]
fn map_scripts_idea_trait_reads_only_the_appointed_role_and_active_ideas() {
    script_game::enter();
    let (cat, _folder) = crate::catalogue::tests::folder_catalogue(&[(
        "common/country_leader/traits.txt",
        "leader_traits = { first = {} first = {} second = {} }",
    )]);
    let traits = content_reader_military::leader_traits::read(&cat.content.layers)
        .unwrap()
        .0;
    let units = content_reader_key_names::unit_leader::Traits::default();
    let ideas = content_reader_politics::ideas::IdeaContent::from_parsed(
        Vec::new(),
        vec![content_reader_politics::ideas::Idea {
            token: "spirit".into(),
            traits: vec!["second".into()],
            ..Default::default()
        }],
    );
    let mut f = frame(&[(100, 1, "GER", 1, "GER", "")], &[]);
    set::<tables::HiredAdvisors>(
        &mut f,
        vec![replay_data::rows::HiredAdvisorRow {
            country: 100,
            character: 7,
            idea: b"advisor".to_vec(),
            slot: b"seat".to_vec(),
            ..Default::default()
        }],
    );
    set::<tables::AdvisorRoles>(
        &mut f,
        vec![replay_data::rows::AdvisorRoleRow {
            character: 7,
            country: 100,
            idea: b"advisor".to_vec(),
            slot: b"seat".to_vec(),
            ordinal: 3,
            ..Default::default()
        }],
    );
    set::<tables::CharacterRoleTraits>(
        &mut f,
        vec![replay_data::rows::CharacterRoleTraitRow {
            character: 7,
            kind: 1,
            role: 3,
            definitions: vec![2],
            tokens: vec![9],
        }],
    );
    set::<tables::ActiveIdeas>(
        &mut f,
        vec![replay_data::rows::ActiveIdeaRow {
            country: 100,
            token: b"spirit".to_vec(),
            ..Default::default()
        }],
    );
    let (dlcs, names, aliases) = (BTreeSet::new(), TriggerNames::default(), Arc::default());
    let mut world = World::read(
        &f,
        ContentFacts {
            character_traits: Ok((&units, &traits)),
            ..ContentFacts::content_only(&dlcs, &names, &aliases)
        },
    );
    world.ideas = Ok(&ideas).into();
    let eval = Evaluator::new(&world, None);
    let scope = Scopes::of(Scope::Country(1));
    assert_eq!(
        eval.evaluate(&items("has_idea_with_trait = first"), scope),
        Eval::False
    );
    assert_eq!(
        eval.evaluate(&items("has_idea_with_trait = second"), scope),
        Eval::True
    );
}

#[test]
fn map_scripts_home_area_uses_global_area_size_and_the_states_first_province() {
    script_game::enter();
    let mut f = frame(&[(100, 1, "GER", 1, "GER", "")], &[]);
    set::<tables::States>(
        &mut f,
        [10, 20, 30]
            .into_iter()
            .map(|id| replay_data::rows::StateRow {
                id,
                owner: 100,
                ..Default::default()
            })
            .collect(),
    );
    set::<tables::Politics>(
        &mut f,
        vec![replay_data::rows::PoliticsRow {
            country: 100,
            capital: 10,
            ..Default::default()
        }],
    );
    set::<tables::Provinces>(
        &mut f,
        [(1, 10, 8), (2, 10, 7), (3, 20, 8), (4, 20, 7), (5, 30, 8)]
            .into_iter()
            .map(|(id, state, area)| replay_data::rows::ProvinceRow {
                id,
                state,
                controller: 100,
                controller_area: Some(area).into(),
                ..Default::default()
            })
            .collect(),
    );
    let order = BTreeMap::from([(10, vec![2, 1]), (20, vec![3, 4]), (30, vec![5])]);
    let (dlcs, names, aliases) = (BTreeSet::new(), TriggerNames::default(), Arc::default());
    let world = World::read(
        &f,
        ContentFacts {
            state_provinces: &order,
            state_content: Ok(()),
            ..ContentFacts::content_only(&dlcs, &names, &aliases)
        },
    );
    let eval = Evaluator::new(&world, None);
    assert_eq!(
        eval.evaluate(
            &items("is_in_home_area = yes"),
            Scopes::of(Scope::State(20))
        ),
        Eval::True
    );
}
