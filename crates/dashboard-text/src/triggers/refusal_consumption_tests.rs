use super::*;

fn set_invasion_table<T: replay_data::rows::TableSchema>(f: &mut DecodedFrame, rows: Vec<T::Row>) {
    *f.tables[T::ID.index()] = TableView::Complete(TableRows::from_rows::<T>(rows));
}

fn add_known_invasion(f: &mut DecodedFrame, state: u32) {
    use replay_data::rows::*;
    let TableView::Complete(TableRows::LandTheatres(theatres)) =
        &mut *f.tables[TableId::LandTheatres.index()]
    else {
        panic!("complete theatres");
    };
    theatres.push(LandTheatreRow {
        id: 2,
        owner: 100,
        ..Default::default()
    });
    set_invasion_table::<tables::Provinces>(
        f,
        vec![ProvinceRow {
            id: 1,
            state,
            ..Default::default()
        }],
    );
    set_invasion_table::<tables::OrdersGroups>(
        f,
        vec![OrdersGroupRow {
            handle: 10,
            theatre: 2,
            theatre_ordinal: Some(0),
            ..Default::default()
        }],
    );
    set_invasion_table::<tables::OrdersGroupPlans>(
        f,
        vec![OrdersGroupPlanRow {
            group: 10,
            primary_orders: vec![1],
            ..Default::default()
        }],
    );
    set_invasion_table::<tables::ArmyOrders>(
        f,
        vec![ArmyOrderRow {
            owner_group: 10,
            instance: 1,
            order_type: 3,
            path: vec![1],
            ..Default::default()
        }],
    );
    set_invasion_table::<tables::ArmyOrderPlans>(
        f,
        vec![ArmyOrderPlanRow {
            owner_group: 10,
            instance: 1,
            ..Default::default()
        }],
    );
    set_invasion_table::<tables::InvasionPreparations>(
        f,
        vec![InvasionPreparationRow {
            owner_group: 10,
            instance: 1,
            ..Default::default()
        }],
    );
}

fn invasion_frame() -> DecodedFrame {
    let mut f = positive_variables_frame();
    *f.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![replay_data::rows::StateRow {
            id: 64,
            owner: 101,
            ..Default::default()
        }]));
    *f.tables[TableId::LandTheatres.index()] = TableView::Complete(TableRows::LandTheatres(vec![
        replay_data::rows::LandTheatreRow {
            id: 1,
            owner: 999,
            ..Default::default()
        },
    ]));
    f
}

#[test]
fn an_unlinked_invasion_owner_refuses_masked_colours_on_cold_and_warm_caches() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    for (known_state, unresolved) in [
        (None, true),
        (Some(65), true),
        (Some(64), true),
        (Some(64), false),
    ] {
        let mut f = invasion_frame();
        if let Some(state) = known_state {
            add_known_invasion(&mut f, state);
        }
        if !unresolved {
            let TableView::Complete(TableRows::LandTheatres(rows)) =
                &mut *f.tables[TableId::LandTheatres.index()]
            else {
                panic!("theatres");
            };
            rows.retain(|row| row.owner != 999);
        }
        let dlcs = BTreeSet::new();
        let names = TriggerNames::default();
        let aliases = Arc::default();
        let mut facts = ContentFacts::content_only(&dlcs, &names, &aliases);
        facts.invasion_prepare_days = Ok(1);
        let world = World::read(&f, facts);
        let eval = Evaluator::new(&world, None);
        let scopes = Scopes::of(Scope::Country(1));
        let body = "OR = { AND = { has_naval_invasion_against_state = 64 set_temp_variable = { red = 1 } } always = yes } set_temp_variable = { alpha = 1 }";
        for _ in 0..2 {
            if !unresolved {
                assert_eq!(eval.evaluate_colour(&items(body), scopes, &[]), Eval::True);
                assert_eq!(eval.temp_value(b"red").unwrap().unwrap(), ONE);
            } else {
                colour_refusals(&eval, scopes, body, &["CountryLinks"]);
            }
        }
        assert_eq!(eval.evaluate(&items(body), scopes), Eval::True);
    }
}

#[test]
fn alias_bucket_failures_are_reached_only_when_the_candidate_is_consumed() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let mut f = positive_variables_frame();
    if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *f.tables[TableId::ReservedCountryNaming.index()]
    {
        rows[0].original_tag_buckets = Recorded::Value(None);
    } else {
        panic!("reserved fixture");
    }
    for passes in [true, false] {
        let alias = format!(
            "ALI = {{ original_tag = GER always = {} }}",
            if passes { "yes" } else { "no" }
        );
        with_world(&f, &alias, Err("none"), |world| {
            let eval = Evaluator::new(world, None);
            let scopes = Scopes::of(Scope::Country(1));
            for _ in 0..2 {
                if passes {
                    assert_eq!(
                        eval.evaluate_colour(
                            &items("tag = ALI set_temp_variable = { alpha = 1 }"),
                            scopes,
                            &[]
                        ),
                        Eval::True
                    );
                } else {
                    colour_refusals(
                        &eval,
                        scopes,
                        "tag = ALI",
                        &["ReservedCountryNaming.original_tag_buckets"],
                    );
                }
            }
        });
    }
}

#[test]
fn capital_and_overlord_indexes_preserve_failed_country_joins() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let mut f = positive_variables_frame();
    *f.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![replay_data::rows::PoliticsRow {
            country: 101,
            capital: 64,
            ..Default::default()
        }]));
    *f.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![]));
    let scopes = crate::scope::RecordedScopes::read(&f, Arc::default());
    for _ in 0..2 {
        let (_, failures) = super::super::refusal::capture(|| {
            for name in ["capital_scope", "overlord"] {
                let error = scopes
                    .resolve_text(name, false, &crate::scope::Context::country(1))
                    .unwrap_err();
                let crate::scope::Unresolved::Missing(failure) = error else {
                    panic!("missing join");
                };
                super::super::refusal::assert_reached(&failure);
            }
        });
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(failures.first().unwrap().names("CountryLinks"));
    }
}

#[test]
fn unavailable_faction_members_has_one_global_identity_across_scope_readers() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let mut f = positive_variables_frame();
    *f.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
        reason: "synthetic missing membership".into(),
    };
    with_world(&f, "", Err("none"), |world| {
        for _ in 0..2 {
            let (_, failures) = super::super::refusal::capture(|| {
                for slot in [1, 2] {
                    let failed = world.recorded().faction(slot).unwrap_err();
                    super::super::refusal::assert_reached(&failed);
                    assert!(world.faction(slot).is_err());
                }
                assert!(world.recorded().faction_first_raw(1).is_err());
            });
            assert_eq!(failures.len(), 1, "{failures:?}");
            let why = failures.first().unwrap();
            assert_eq!(
                why.identities(),
                &BTreeSet::from([super::super::refusal::Identity::Input {
                    table: TableId::FactionMembers,
                    field: String::new(),
                    scope: None,
                }]),
                "{why}"
            );
        }
    });
}

#[test]
fn consuming_a_lazy_seed_through_the_output_reader_replays_its_failure() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let f = positive_variables_frame();
    with_world(&f, "", Err("none"), |world| {
        let eval = Evaluator::new(world, None);
        eval.initialize_variable_random(Scopes::of(Scope::Country(1)));
        for _ in 0..2 {
            let (_, failures) = super::super::refusal::capture(|| {
                let failure = eval.temp_value(b"random_seed").unwrap().unwrap_err();
                super::super::refusal::assert_reached(&failure);
            });
            assert_eq!(failures.len(), 1, "{failures:?}");
            assert!(failures.first().unwrap().names("ScriptVariables"));
        }
    });
}

#[test]
fn filtered_raw_joins_refuse_each_source_once_and_resolved_colours_stay_exact() {
    use replay_data::rows::*;
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    for broken in [true, false] {
        let raw = if broken { 999 } else { 100 };
        let mut f = positive_variables_frame();
        set_invasion_table::<tables::States>(
            &mut f,
            vec![StateRow {
                id: 64,
                owner: 100,
                controller: raw,
                ..Default::default()
            }],
        );
        set_invasion_table::<tables::CountryCounterInputs>(
            &mut f,
            vec![
                CountryCounterInputRow {
                    country: 100,
                    owned_state_count: 1,
                    owned_states: Some(vec![64]).into(),
                    ..Default::default()
                },
                CountryCounterInputRow {
                    country: 103,
                    owned_state_count: 1,
                    ..Default::default()
                },
            ],
        );
        set_invasion_table::<tables::CountryWars>(
            &mut f,
            vec![CountryWarRow {
                country: 100,
                enemies_read: 1,
                enemies: vec![103],
                ..Default::default()
            }],
        );
        set_invasion_table::<tables::CountryCivilWars>(
            &mut f,
            vec![CountryCivilWarRow {
                country: 103,
                target: raw,
                ..Default::default()
            }],
        );
        set_invasion_table::<tables::AdvisorTokens>(
            &mut f,
            vec![AdvisorTokenRow {
                token: b"synthetic_advisor".to_vec(),
                ..Default::default()
            }],
        );
        set_invasion_table::<tables::HiredAdvisors>(
            &mut f,
            vec![HiredAdvisorRow {
                country: raw,
                idea: b"synthetic_advisor".to_vec(),
                ..Default::default()
            }],
        );
        set_invasion_table::<tables::FactionMembers>(
            &mut f,
            vec![FactionMemberRow {
                country: 100,
                faction: 1,
                ..Default::default()
            }],
        );
        set_invasion_table::<tables::LexicalTokens>(
            &mut f,
            vec![
                LexicalTokenRow {
                    token: 10,
                    literal: b"synthetic_project".to_vec(),
                },
                LexicalTokenRow {
                    token: 11,
                    literal: b"synthetic_mio".to_vec(),
                },
                LexicalTokenRow {
                    token: 12,
                    literal: b"synthetic_actor".to_vec(),
                },
            ],
        );
        set_invasion_table::<tables::Projects>(
            &mut f,
            vec![ProjectRow {
                owner: raw,
                local_id: 9,
                definition_id: 10,
                ..Default::default()
            }],
        );
        set_invasion_table::<tables::IndustrialOrganisations>(
            &mut f,
            vec![
                IndustrialOrganisationRow {
                    country: raw,
                    organisation: 6 << 32 | 79,
                    token: 11,
                    ordinal: 0,
                    ..Default::default()
                },
                IndustrialOrganisationRow {
                    country: 100,
                    organisation: 7 << 32 | 79,
                    token: 11,
                    ordinal: 1,
                    ..Default::default()
                },
            ],
        );
        set_invasion_table::<tables::Characters>(
            &mut f,
            vec![CharacterRow {
                id: 7,
                country: raw,
                name_token_id: 12,
                ..Default::default()
            }],
        );
        set_invasion_table::<tables::ResearchModifiers>(
            &mut f,
            vec![ResearchModifierRow {
                country: raw,
                kind: ResearchModifierRow::SHARING_ROSTER,
                index: u32::MAX,
                raw: -1,
                count: 0,
                ..Default::default()
            }],
        );
        let roles = LeaderRoles::new(vec![content_reader_key_names::characters::LeaderTemplate {
            path: "common/characters/synthetic.txt".into(),
            tag: "synthetic_actor".into(),
            line: None,
            role: Ok(None),
        }]);
        with_world(&f, "", Ok(&roles), |world| {
            let eval = Evaluator::new(world, None);
            let scopes = Scopes::of(Scope::Country(1));
            let checks = [
                "check_variable = { owned_controlled_states^num = 1 }",
                "all_of_scopes = { array = owned_controlled_states always = yes }",
                "NOT = { any_of_scopes = { array = owned_controlled_states always = no } }",
                "has_civil_war = no",
                "has_idea = synthetic_advisor",
                "mio:synthetic_mio = { always = yes }",
                "sp:synthetic_project = { always = yes }",
                "synthetic_actor = { always = yes }",
                "is_in_tech_sharing_group = synthetic_project",
            ];
            for _ in 0..2 {
                for check in checks {
                    let body = format!(
                        "OR = {{ {check} always = yes }} set_temp_variable = {{ red = 1 }} set_temp_variable = {{ alpha = 1 }}"
                    );
                    if broken {
                        colour_refusals(&eval, scopes, &body, &["CountryLinks"]);
                    } else {
                        assert_eq!(
                            eval.evaluate_colour(&items(&body), scopes, &[]),
                            Eval::True,
                            "{check}"
                        );
                        assert_eq!(eval.temp_value(b"red").unwrap().unwrap(), ONE, "{check}");
                        assert_eq!(eval.temp_value(b"alpha").unwrap().unwrap(), ONE, "{check}");
                    }
                }
            }
        });
    }
}

#[test]
fn filtered_resolution_certainty_is_part_of_a_querys_reuse_digest() {
    use replay_data::rows::*;
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let mut before = positive_variables_frame();
    set_invasion_table::<tables::AdvisorTokens>(
        &mut before,
        vec![AdvisorTokenRow {
            token: b"synthetic_advisor".to_vec(),
            ..Default::default()
        }],
    );
    set_invasion_table::<tables::HiredAdvisors>(
        &mut before,
        vec![HiredAdvisorRow {
            country: 103,
            idea: b"synthetic_advisor".to_vec(),
            ..Default::default()
        }],
    );
    set_invasion_table::<tables::FactionMembers>(
        &mut before,
        vec![FactionMemberRow {
            country: 100,
            faction: 1,
            ..Default::default()
        }],
    );
    let mut after = before.clone();
    after.hour += 1;
    let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *after.tables[TableId::CountryLinks.index()]
    else {
        panic!("links");
    };
    rows.retain(|row| row.id != 103);
    with_world(&before, "", Err("none"), |first| {
        with_world(&after, "", Err("none"), |later| {
            let (answer, mut inputs) =
                super::super::inputs::capture(&before, || first.has_idea(1, "synthetic_advisor"));
            assert_eq!(answer, Ok(false));
            assert_eq!(later.has_idea(1, "synthetic_advisor"), Ok(false));
            assert!(!inputs.confirm(&after, |key| match key {
                super::super::inputs::Key::World(query) => Some(later.reread(query)),
                super::super::inputs::Key::Scopes(query) => Some(later.recorded().reread(query)),
                _ => None,
            }));
        });
    });
}

#[test]
fn missing_positive_character_tokens_refuse_on_cold_and_warm_indexes() {
    use replay_data::rows::*;
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let roles = LeaderRoles::new(vec![content_reader_key_names::characters::LeaderTemplate {
        path: "common/characters/synthetic.txt".into(),
        tag: "synthetic_actor".into(),
        line: None,
        role: Ok(None),
    }]);
    for token in [-1, 0, 12] {
        let mut f = positive_variables_frame();
        set_invasion_table::<tables::Characters>(
            &mut f,
            vec![CharacterRow {
                id: 7,
                country: 100,
                name_token_id: token,
                ..Default::default()
            }],
        );
        set_invasion_table::<tables::LexicalTokens>(&mut f, vec![]);
        with_world(&f, "", Ok(&roles), |world| {
            let eval = Evaluator::new(world, None);
            let scopes = Scopes::of(Scope::Country(1));
            let body = "NOT = { synthetic_actor = { always = yes } } set_temp_variable = { red = 1 } set_temp_variable = { alpha = 1 }";
            for _ in 0..2 {
                if token > 0 {
                    colour_refusals(&eval, scopes, body, &["LexicalTokens"]);
                } else {
                    assert_eq!(eval.evaluate_colour(&items(body), scopes, &[]), Eval::True);
                }
            }
        });
    }
}

#[test]
fn missing_state_references_replay_acquisition_in_encoded_and_numeric_scopes() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    for raw in [(0xffff_ffff_c000_0000_u64 | 64) as i64, 64 * ONE] {
        let mut f = with_variables(
            frame(&original_countries(), &[]),
            &[(1, 100, &[("selected_state", raw)])],
        );
        *f.tables[TableId::States.index()] = TableView::Unavailable {
            reason: "synthetic missing states".into(),
        };
        with_world(&f, "", Err("none"), |world| {
            let eval = Evaluator::new(world, None);
            let scopes = Scopes::of(Scope::Country(1));
            let body =
                "NOT = { var:selected_state = { always = yes } } set_temp_variable = { alpha = 1 }";
            for _ in 0..2 {
                colour_refusals(&eval, scopes, body, &["States"]);
            }
            let (_, refusals) = super::super::refusal::capture(|| world.state_row(0));
            assert!(
                refusals.is_empty(),
                "literal null must not read States: {refusals:?}"
            );
        });
    }
}

#[test]
fn missing_province_references_refuse_control_and_invasion_nonmatches() {
    use replay_data::rows::*;
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let mut f = invasion_frame();
    add_known_invasion(&mut f, 64);
    set_invasion_table::<tables::LandTheatres>(
        &mut f,
        vec![LandTheatreRow {
            id: 2,
            owner: 100,
            ..Default::default()
        }],
    );
    set_invasion_table::<tables::States>(
        &mut f,
        vec![StateRow {
            id: 64,
            owner: 100,
            controller: 100,
            ..Default::default()
        }],
    );
    set_invasion_table::<tables::Provinces>(&mut f, vec![]);
    let dlcs = BTreeSet::new();
    let names = TriggerNames::default();
    let aliases = Arc::default();
    let authored = [(64, vec![1])].into_iter().collect();
    let mut facts = ContentFacts::content_only(&dlcs, &names, &aliases);
    facts.state_content = Ok(());
    facts.state_provinces = &authored;
    facts.invasion_prepare_days = Ok(1);
    let world = World::read(&f, facts);
    let eval = Evaluator::new(&world, None);
    for _ in 0..2 {
        colour_refusals(
            &eval,
            Scopes::of(Scope::State(64)),
            "NOT = { is_fully_controlled_by = GER } set_temp_variable = { alpha = 1 }",
            &["Provinces"],
        );
        colour_refusals(
            &eval,
            Scopes::of(Scope::Country(1)),
            "NOT = { has_naval_invasion_against_state = 64 } set_temp_variable = { alpha = 1 }",
            &["Provinces"],
        );
    }
}

#[test]
fn positive_id_sources_record_but_authenticated_nulls_and_membership_absence_do_not() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let f = positive_variables_frame();
    with_world(&f, "", Err("none"), |world| {
        let (_, missing) = super::super::refusal::capture(|| {
            assert!(world.token(9000).unwrap().is_none());
            assert!(world.character(9000).unwrap().is_none());
            assert!(world.unit_leader(9000).unwrap().is_none());
            let eval = Evaluator::new(world, None);
            for kind in [
                engine_projects::nproject_cproject::REFERENCE_TYPE,
                79,
                engine_native::craid_instance::REFERENCE_TYPE_VALUE,
            ] {
                assert!(eval.decoded_carrier(kind, 9000).unwrap().is_none());
            }
        });
        for table in [
            "LexicalTokens",
            "Characters",
            "UnitLeaders",
            "Projects",
            "IndustrialOrganisations",
            "Raids",
        ] {
            assert_eq!(
                missing.iter().filter(|why| why.names(table)).count(),
                1,
                "{missing:?}"
            );
        }
        let (_, absent) = super::super::refusal::capture(|| {
            assert!(world.state_row(0).is_none());
            assert!(world.province_row(0).is_none());
            assert!(world.token_of(b"absent_literal").unwrap().is_none());
        });
        assert!(absent.is_empty(), "{absent:?}");
    });
}

#[test]
fn state_builtin_country_miss_has_only_its_country_links_identity() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let mut f = positive_variables_frame();
    set_invasion_table::<tables::States>(
        &mut f,
        vec![replay_data::rows::StateRow {
            id: 64,
            owner: 100,
            controller: 999,
            ..Default::default()
        }],
    );
    with_world(&f, "", Err("none"), |world| {
        let eval = Evaluator::new(world, None);
        for _ in 0..2 {
            colour_refusals(
                &eval,
                Scopes::of(Scope::State(64)),
                "set_temp_variable = { red = controller } set_temp_variable = { alpha = 1 }",
                &["CountryLinks"],
            );
        }
    });
}

#[test]
fn country_token_absence_requires_complete_countries_on_cold_and_warm_reads() {
    use content_reader_script::scope::Names;
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    for status in ["complete", "truncated", "unavailable"] {
        let mut f = with_variables(
            frame(&[(100, 1, "GER", 1, "GER", "")], &[]),
            &[(1, 100, &[]), (2, 64, &[])],
        );
        let rows = f.complete::<tables::Countries>().unwrap().to_vec();
        if status == "truncated" {
            *f.tables[TableId::Countries.index()] = TableView::Truncated {
                rows: TableRows::Countries(rows),
                missing: 1,
            };
        } else if status == "unavailable" {
            *f.tables[TableId::Countries.index()] = TableView::Unavailable {
                reason: "synthetic missing country tags".into(),
            };
        }
        with_world(&f, "", Err("none"), |world| {
            let eval = Evaluator::new(world, None);
            let scopes = Scopes::of(if status == "unavailable" {
                Scope::State(64)
            } else {
                Scope::Country(1)
            });
            for _ in 0..2 {
                let (_, missing) = super::super::refusal::capture(|| {
                    assert!(!Names::is_country(world.recorded(), "AAA"))
                });
                if status == "complete" {
                    assert!(missing.is_empty(), "{missing:?}");
                } else {
                    assert_eq!(missing.len(), 1, "{missing:?}");
                    assert!(missing.first().unwrap().names("Countries"));
                }
                let body = "set_temp_variable = { red = AAA } set_temp_variable = { alpha = 1 }";
                if status == "complete" {
                    assert_eq!(eval.evaluate_colour(&items(body), scopes, &[]), Eval::True);
                    assert_eq!(eval.temp_value(b"red").unwrap().unwrap(), 0);
                } else {
                    colour_refusals(&eval, scopes, body, &["Countries"]);
                }
            }
            if status == "truncated" {
                let (_, known) = super::super::refusal::capture(|| {
                    assert!(Names::is_country(world.recorded(), "GER"))
                });
                assert!(
                    known.is_empty(),
                    "a known member proves presence: {known:?}"
                );
            }
        });
    }
}

#[test]
fn character_token_membership_absence_requires_both_complete_tables() {
    use content_reader_script::scope::Names;
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    for unavailable in [TableId::Characters, TableId::LexicalTokens] {
        let mut f = positive_variables_frame();
        *f.tables[unavailable.index()] = TableView::Truncated {
            rows: TableRows::empty(unavailable),
            missing: 1,
        };
        with_world(&f, "", Err("none"), |world| {
            for _ in 0..2 {
                let (_, missing) = super::super::refusal::capture(|| {
                    assert!(!Names::is_character(world.recorded(), "synthetic_actor"))
                });
                assert_eq!(missing.len(), 1, "{missing:?}");
                assert!(
                    missing
                        .first()
                        .unwrap()
                        .names(&format!("{unavailable:?}"))
                );
            }
        });
    }
}

#[test]
fn missing_fresh_resistance_row_preserves_its_state_input_identity() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let mut f = positive_variables_frame();
    f.tables[TableId::StateResistance.index()] = replay_frame::frame::SharedView::from(
        TableView::Complete(TableRows::StateResistance(vec![])),
    )
    .with_sampled_at(Some(f.hour));
    with_world(&f, "", Err("none"), |world| {
        let eval = Evaluator::new(world, None);
        for _ in 0..2 {
            colour_refusals(
                &eval,
                Scopes::of(Scope::State(64)),
                "set_temp_variable = { red = resistance } set_temp_variable = { alpha = 1 }",
                &["StateResistance (State(64))"],
            );
        }
    });
}

#[test]
fn nested_scope_parse_replays_country_classification_failure_on_cold_and_warm_reads() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let roles = LeaderRoles::new(Vec::new());
    for incomplete in [false, true] {
        let mut f = positive_variables_frame();
        if incomplete {
            let rows = f.complete::<tables::Countries>().unwrap().to_vec();
            *f.tables[TableId::Countries.index()] = TableView::Truncated {
                rows: TableRows::Countries(rows),
                missing: 1,
            };
        }
        with_world(&f, "", Ok(&roles), |world| {
            let eval = Evaluator::new(world, None);
            let scopes = Scopes::of(Scope::Country(1));
            let callback = items(
                "if = { limit = { ROOT.AAA = { always = yes } } set_temp_variable = { green = 1 } } set_temp_variable = { alpha = 1 }",
            );
            for _ in 0..2 {
                let result = eval.evaluate_colour(&callback, scopes, &[]);
                if incomplete {
                    let Eval::Unknown(reasons) = result else {
                        panic!("nested classification must refuse: {result:?}");
                    };
                    assert_eq!(reasons.len(), 1, "{reasons:?}");
                    assert!(reasons.first().unwrap().names("Countries"));
                } else {
                    assert_eq!(result, Eval::True);
                    assert!(eval.temp_value(b"green").is_none());
                    assert_eq!(eval.temp_value(b"alpha").unwrap().unwrap(), 100_000);
                }
            }
        });
    }
}

#[test]
fn keyword_scope_parse_replays_character_classification_failure_on_cold_and_warm_reads() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let f = positive_variables_frame();
    let roles = LeaderRoles::new(Vec::new());
    for templates in [Ok(&roles), Err("synthetic missing character templates")] {
        with_world(&f, "", templates, |world| {
            let eval = Evaluator::new(world, None);
            let scopes = Scopes::of(Scope::Country(1));
            let callback = items(
                "if = { limit = { ROOT = { always = yes } } set_temp_variable = { green = 1 } } set_temp_variable = { alpha = 1 }",
            );
            for _ in 0..2 {
                let result = eval.evaluate_colour(&callback, scopes, &[]);
                if templates.is_err() {
                    let Eval::Unknown(reasons) = result else {
                        panic!("keyword fall-through must refuse: {result:?}");
                    };
                    assert_eq!(reasons.len(), 1, "{reasons:?}");
                    assert!(reasons.first().unwrap().contains("character templates"));
                } else {
                    assert_eq!(result, Eval::True);
                    assert_eq!(eval.temp_value(b"green").unwrap().unwrap(), 100_000);
                    assert_eq!(eval.temp_value(b"alpha").unwrap().unwrap(), 100_000);
                }
            }
        });
    }
}
