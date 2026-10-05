use super::super::super::tests::{assets, frame};
use super::*;
#[test]
fn controller_flag_uses_native_control_and_three_codepoints() {
    use dashboard::products::country_flags::FlagChoice;
    with_context(|context| {
        for tag in ["GER", "ÄØ中", "ABCD"] {
            let flags = std::cell::OnceCell::from(Ok(vec![FlagChoice {
                raw: 10,
                tag: tag.into(),
                candidates: vec!["GER_fascism".into()],
            }]));
            let result = context.flag(0, &flags);
            if tag == "ABCD" {
                assert_eq!(
                    result,
                );
            } else {
                assert_eq!(
                    result,
                    Ok(format!("{}{tag}", crate::map::native_text::FLAG))
                );
            }
        }
    });
}

#[test]
fn facility_threshold_is_strict_and_first_program_controls_project_text() {
    use super::super::super::tests::put;
    use replay::rows::RaidFacilityRow;
    for (required, first_project) in [(1, Some(100)), (0, None), (0, Some(100))] {
        let mut assets = assets();
        assets
            .buildings
            .types
            .get_mut("dam")
            .unwrap()
            .specializations
            .push("test".into());
        assets.tooltip.intel = Ok(intel::Defines {
            required,
            ..Default::default()
        });
        assets.tooltip.projects = Ok(installed_content::special_projects::Catalog {
            projects: [("oilfield".into(), Default::default())].into(),
            ..Default::default()
        });
        let mut frame = frame();
        put::<tables::RaidFacilities>(
            &mut frame,
            vec![
                RaidFacilityRow {
                    country: 11,
                    ordinal: 0,
                    province: 3,
                    owner: 11,
                    project_definition: first_project,
                    ..Default::default()
                },
                RaidFacilityRow {
                    country: 11,
                    ordinal: 1,
                    province: 3,
                    project_definition: Some(101),
                    ..Default::default()
                },
            ],
        );
        put::<tables::CountryAppearance>(
            &mut frame,
            vec![replay::rows::CountryAppearanceRow {
                country: 11,
                ..Default::default()
            }],
        );
        put::<tables::Politics>(
            &mut frame,
            vec![replay::rows::PoliticsRow {
                country: 11,
                ..Default::default()
            }],
        );
        let world = World::new(&frame, assets.facts()).unwrap();
        let loc: Localisation = [("oilfield".into(), "Viewer project".into())]
            .into_iter()
            .collect();
        let context = Context {
            assets: &assets,
            world: &world,
            loc: &loc,
            recorded_language: &loc.language,
            scripted_content: &Err("no scripted content in this test".into()),
            ours: 0,
            countries: installed_content::country_names::CountryNames::read(&frame, &loc),
            flag_exists: &|_| true,
            scripted: Default::default(),
        };
        let scope = RaidScope {
            building_type: 200,
            building_flags: 1,
            building_location: 3,
            ..Default::default()
        };
        // Country zero has CURRENT=0 in 140FD0EA0. Equality reveals the project.
        match (
            required,
            first_project,
            context.facility(&scope, &Default::default()).unwrap(),
        ) {
            (
                1,
                _,
                rules::Facility::Unknown {
                    current: 0,
                    required: 1,
                },
            )
            | (0, None, rules::Facility::Idle) => {}
            (0, Some(100), rules::Facility::Project(text)) => {
                assert_eq!(text, Ok("\u{11}YViewer project\u{11}!".into()))
            }
            _ => panic!("wrong facility threshold or program selection"),
        }
    }
}

#[test]
fn raid_tooltip_regression_facility_project_uses_country_narrative_and_scripted_name() {
    use super::super::super::tests::put;
    for narrative in [None, Some("narrative"), Some("")] {
        for country_override in [true, false] {
            let mut assets = assets();
            assets
                .buildings
                .types
                .get_mut("dam")
                .unwrap()
                .specializations
                .push("test".into());
            assets.tooltip.intel = Ok(intel::Defines::default());
            assets.tooltip.projects = Ok(installed_content::special_projects::Catalog {
                projects: [(
                    "oilfield".into(),
                    installed_content::special_projects::Project {
                        name: narrative.map(Into::into),
                        ..Default::default()
                    },
                )]
                .into(),
                ..Default::default()
            });
            let mut frame = frame();
            // The project owner's country supplies the naming scope, even when
            // its program occurs in the target controller's roster.
            put::<tables::RaidFacilities>(
                &mut frame,
                vec![replay::rows::RaidFacilityRow {
                    country: 11,
                    owner: 10,
                    province: 3,
                    project_definition: Some(100),
                    ..Default::default()
                }],
            );
            put::<tables::CountryAppearance>(
                &mut frame,
                vec![replay::rows::CountryAppearanceRow {
                    country: 10,
                    name_tag: b"ENG".to_vec(),
                    ..Default::default()
                }],
            );
            put::<tables::Politics>(
                &mut frame,
                vec![replay::rows::PoliticsRow {
                    country: 10,
                    ..Default::default()
                }],
            );
            let world = World::new(&frame, assets.facts()).unwrap();
            let loc: Localisation = [
                ("oilfield", "Default project"),
                ("GER_oilfield", "Country project [ROOT.GetTag]"),
                ("narrative", "Narrative project"),
                ("GER_narrative", "Country narrative [ROOT.GetTag]"),
            ]
            .into_iter()
            .filter(|(key, _)| country_override || !key.starts_with("GER_"))
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
            let scripted = Ok(scripted_content());
            let context = Context {
                assets: &assets,
                world: &world,
                loc: &loc,
                recorded_language: "german",
                scripted_content: &scripted,
                ours: 1,
                countries: installed_content::country_names::CountryNames::read(&frame, &loc),
                flag_exists: &|_| true,
                scripted: Default::default(),
            };
            let scope = RaidScope {
                building_type: 200,
                building_flags: 1,
                building_location: 3,
                ..Default::default()
            };
            let rules::Facility::Project(name) = context
                .facility(&scope, &std::cell::OnceCell::from(Ok(0)))
                .unwrap()
            else {
                panic!("ongoing project is absent");
            };
            assert_eq!(
                name.unwrap(),
                match (narrative, country_override) {
                    (Some("narrative"), true) =>
                        "\u{11}YCountry narrative GER\u{11}! (Default project)",
                    (Some("narrative"), false) =>
                        "\u{11}YNarrative project\u{11}! (Default project)",
                    (_, true) => "\u{11}YCountry project GER\u{11}! (Default project)",
                    (_, false) => "\u{11}YDefault project\u{11}!",
                }
            );
        }
    }
}

#[test]
fn captive_uses_original_leader_name_without_character_identity() {
    use super::super::super::tests::put;
    use replay::rows::UnitLeaderRow;
    let assets = assets();
    let mut frame = frame();
    put::<tables::UnitLeaders>(
        &mut frame,
        vec![UnitLeaderRow {
            id: 7,
            reference_type: 84,
            inherited_text_a: b"original_name".to_vec(),
            inherited_text_b: b"wrong client language".to_vec(),
            ..Default::default()
        }],
    );
    let world = World::new(&frame, assets.facts()).unwrap();
    let loc: Localisation = [("original_name".into(), "Viewer language name".into())]
        .into_iter()
        .collect();
    let context = Context {
        assets: &assets,
        world: &world,
        loc: &loc,
        recorded_language: &loc.language,
        scripted_content: &Err("no scripted content in this test".into()),
        ours: 0,
        flag_exists: &|_| true,
        scripted: Default::default(),
        countries: installed_content::country_names::CountryNames::read(&frame, &loc),
    };
    let scope = RaidScope {
        leader_reference: (7 << 32) | 84,
        ..Default::default()
    };
    assert_eq!(
        context.captive(&scope),
        Ok(Some("Viewer language name".into()))
    );
    assert_eq!(context.captive(&RaidScope::default()), Ok(None));
    // The game names every targeted leader; an unrecorded one is refused.
    assert_eq!(
        context.captive(&RaidScope {
            leader_reference: (8 << 32) | 84,
            ..Default::default()
        }),
        Err("captive leader is absent from UnitLeaders".into())
    );
}
/// Scripted text content with `MAX_SCRIPTED_LOC_RECURSION` alone.
fn scripted_content() -> dashboard::scripted_text::ScriptedContent {
    let root = std::env::temp_dir().join(format!(
        "hoi4_viewer_raid_scripted_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let path = root.join("common/defines/00_defines.lua");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 } }",
    )
    .unwrap();
    let layers = installed_content::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let content = installed_content::content::ResolvedContent {
        layers,
        stats: Default::default(),
    };
    let scripted = dashboard::scripted_text::ScriptedContent::read(
        std::sync::Arc::new(content),
        String::new(),
    )
    .unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    scripted
}
fn with_context(run: impl FnOnce(&Context<'_, '_>)) {
    let assets = assets();
    let scripted = Ok(scripted_content());
    let frame = frame();
    let world = World::new(&frame, assets.facts()).unwrap();
    let loc: Localisation = [
        ("TRIGGER_ALWAYS_FALSE", "never"),
        ("TRIGGER_ALWAYS_TRUE", "always"),
        ("TRIGGER_FULLFILLED_PREFIX", "+"),
        ("TRIGGER_UNFULLFILLED_PREFIX", "-"),
        ("TRIGGER_OR_STARTS", "either"),
        ("TRIGGER_AND_STARTS", "both"),
        ("custom", "custom text"),
        (
            "scoped",
            "[ROOT.GetTag]/[FROM.GetID]/[THIS.GetTag]/[PREV.GetTag]",
        ),
        ("scoped_NOT", "negative [FROM.GetID]"),
        ("chain", "[FROM.ROOT.GetTag]/[ROOT.FROM.PREV.GetTag]"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect();
    let context = Context {
        assets: &assets,
        world: &world,
        loc: &loc,
        recorded_language: &loc.language,
        scripted_content: &scripted,
        ours: 0,
        countries: installed_content::country_names::CountryNames::read(&frame, &loc),
        flag_exists: &|_| true,
        scripted: Default::default(),
    };
    run(&context);
}
fn triggers(text: &str) -> installed_content::triggers::Triggers {
    installed_content::parser::parse(text.as_bytes()).unwrap().items
}
#[test]
fn trigger_inversion_indentation_custom_override_and_named_unsupported_leaf() {
    with_context(|context| {
        let scope = Scopes::of(Scope::Country(0));
        let text = context.trigger_text(
            &triggers("always=no NOT={always=no} OR={always=yes always=no}"),
            scope,
            "test",
        );
        assert_eq!(
            text.into_iter()
                .map(|(_, s)| s.unwrap())
                .collect::<String>(),
            "-never
+always
+either:
   always
   never
"
        );
        let text=context.trigger_text(&triggers("custom_trigger_tooltip={tooltip=custom always=no} hidden_trigger={always=no} unsupported=yes"),scope,"test");
        assert_eq!(text.len(), 2);
        assert_eq!(
            text[0].1.as_deref(),
            Ok("-custom text
")
        );
        assert!(text[1].1.as_ref().unwrap_err().contains("unsupported"));
    });
}

#[test]
fn raid_tooltip_regression_special_project_requirement_uses_native_name_and_inversion() {
    use super::super::super::tests::put;
    let mut assets = assets();
    assets.tooltip.projects = Ok(installed_content::special_projects::Catalog {
        projects: [(
            "oilfield".into(),
            installed_content::special_projects::Project {
                name: Some("project_name".into()),
                ..Default::default()
            },
        )]
        .into_iter()
        .collect(),
        ..Default::default()
    });
    let mut frame = frame();
    put::<tables::CountryAppearance>(
        &mut frame,
        vec![replay::rows::CountryAppearanceRow {
            country: 10,
            ..Default::default()
        }],
    );
    put::<tables::Politics>(
        &mut frame,
        vec![replay::rows::PoliticsRow {
            country: 10,
            ..Default::default()
        }],
    );
    put::<tables::Projects>(
        &mut frame,
        vec![replay::rows::ProjectRow {
            owner: 10,
            definition_id: 100,
            started: 0,
            completed: 0,
            ..Default::default()
        }],
    );
    let world = World::new(&frame, assets.facts()).unwrap();
    let loc: Localisation = [
        ("TRIGGER_FULLFILLED_PREFIX", "+"),
        ("TRIGGER_UNFULLFILLED_PREFIX", "-"),
        ("TRIGGER_IS_SPECIAL_PROJECT_COMPLETED", "finished $PROJECT$"),
        (
            "TRIGGER_IS_NOT_SPECIAL_PROJECT_COMPLETED",
            "unfinished $PROJECT$",
        ),
        ("project_name", "synthetic project"),
        ("oilfield", "synthetic default name"),
        ("GER_project_name", "synthetic country name"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect();
    let context = Context {
        assets: &assets,
        world: &world,
        loc: &loc,
        recorded_language: &loc.language,
        scripted_content: &Err("no scripted content in this test".into()),
        ours: 0,
        countries: installed_content::country_names::CountryNames::read(&frame, &loc),
        flag_exists: &|_| true,
        scripted: Default::default(),
    };
    let scope = Scopes::of(Scope::Country(0));
    let lines = context.trigger_text(
        &triggers("is_special_project_completed=sp:oilfield NOT={is_special_project_completed=sp:oilfield}"),
        scope,
        "test",
    );
    assert_eq!(
        lines[0].1.as_deref(),
        Ok("-finished \u{11}Ysynthetic country name\u{11}! (synthetic default name)\n")
    );
    assert_eq!(
        lines[1].1.as_deref(),
        Ok("+unfinished \u{11}Ysynthetic country name\u{11}! (synthetic default name)\n")
    );
    let absent = context.trigger_text(
        &triggers("is_special_project_completed=sp:commando"),
        scope,
        "test",
    );
    assert_eq!(absent[0].1.as_deref(), Ok("-finished \n"));
    let lines = context.trigger_text(
        &triggers("is_special_project_completed=var:project"),
        scope,
        "test",
    );
    assert!(
        lines[0]
            .1
            .as_ref()
            .unwrap_err()
            .contains("sp: project operand")
    );
}

#[test]
fn custom_tooltip_uses_shared_scope_expansion_and_native_negative_key() {
    with_context(|context| {
        let scopes = Scopes {
            root: Scope::Country(0),
            from: Some(Scope::State(10)),
            prev: Some(Scope::Country(0)),
            ..Scopes::of(Scope::Country(1))
        };
        let text = context.trigger_text(
            &triggers("custom_trigger_tooltip={tooltip=scoped always=yes}"),
            scopes,
            "test",
        );
        assert_eq!(text[0].1.as_deref(), Ok("+GER/10/ENG/GER\n"));
        let text = context.trigger_text(
            &triggers("NOT={custom_trigger_tooltip={tooltip=scoped always=yes}}"),
            scopes,
            "test",
        );
        assert_eq!(text[0].1.as_deref(), Ok("-negative 10\n"));
    });
}

/// The raid condition chain's scope (ROOT GER) with FROM a default scope
/// of the target (ENG): chains follow the chosen scope's own links. A
/// scope change leaves the linked scopes' links unknown, so a chain
/// through them is refused rather than guessed.
#[test]
fn custom_tooltip_chains_follow_the_raid_scopes_links() {
    with_context(|context| {
        let raid = Scopes {
            from: Some(Scope::Country(1)),
            ..Scopes::of(Scope::Country(0))
        };
        let tooltip = triggers("custom_trigger_tooltip={tooltip=chain always=yes}");
        let text = context.trigger_text(&tooltip, raid, "test");
        assert_eq!(text[0].1.as_deref(), Ok("+ENG/ENG\n"));
        let nested = Scopes {
            prev: Some(Scope::Country(0)),
            ..raid
        };
        let text = context.trigger_text(&tooltip, nested, "test");
        let refused = text[0].1.as_ref().unwrap_err();
        assert!(refused.contains("Root link is not known"), "{refused}");
    });
}

#[test]
fn event_scopes_keep_the_raid_links_leaders_and_target_variables() {
    let raid = Scopes {
        from: Some(Scope::Country(1)),
        target_state: Some(10),
        target_leader: Some(9 << 32 | 3),
        ..Scopes::of(Scope::Country(0))
    };
    let scopes = event_scopes(raid);
    assert_eq!(
        scopes.scopes,
        [
            EventScope {
                from: Link::Scope(1),
                ..EventScope::of(EventObject::Country(0))
            },
            EventScope::of(EventObject::Country(1)),
        ]
    );
    assert_eq!(scopes.variables.target_state, Some(10));
    assert_eq!(scopes.variables.target_leader, Some(9 << 32 | 3));
    let nested = event_scopes(Scopes {
        this: Scope::Leader(9 << 32 | 3),
        prev: Some(Scope::Country(0)),
        ..raid
    });
    assert_eq!(nested.scopes[0].this, EventObject::UnitLeader(9 << 32 | 3));
    assert_eq!(nested.scopes[0].root, Link::Scope(1));
    assert_eq!(nested.scopes[1].root, Link::Unknown);
    assert_eq!(nested.variables.target_state, Some(10));
}

#[test]
fn state_name_uses_the_stored_owner_name_in_the_recording_language_only() {
    use super::super::super::tests::put;
    let assets = assets();
    let mut frame = frame();
    put::<tables::States>(
        &mut frame,
        vec![replay::rows::StateRow {
            id: 10,
            owner_name: b"owner cache $literal$".to_vec(),
            ..Default::default()
        }],
    );
    let world = World::new(&frame, assets.facts()).unwrap();
    let loc: Localisation = [("owner cache $literal$".into(), "must not localise".into())]
        .into_iter()
        .collect();
    let context = Context {
        assets: &assets,
        world: &world,
        loc: &loc,
        recorded_language: &loc.language,
        scripted_content: &Err("no scripted content in this test".into()),
        ours: 0,
        countries: installed_content::country_names::CountryNames::read(&frame, &loc),
        flag_exists: &|_| true,
        scripted: Default::default(),
    };
    assert_eq!(
        context.state_name(10).as_deref(),
        Ok("owner cache $literal$")
    );
    let other = Context {
        recorded_language: "german",
        ..context
    };
    assert!(
        other
            .state_name(10)
            .unwrap_err()
            .contains("stored in german without its key")
    );
}
#[test]
fn phase_progress_uses_preparation_override_and_refuses_missing_flying_inputs() {
    with_context(|context| {
        let kind = RaidType {
            tooltip: installed_content::raids::tooltips::Definition {
                days_to_prepare: 2,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut raid = RaidRow {
            phase: 2,
            preparation_hours: 12,
            ..Default::default()
        };
        assert_eq!(
            context.progress(&raid, &RaidTargetRow::default(), &kind),
            Ok(Some(25_000))
        );
        raid.phase = 1;
        assert_eq!(
            context.progress(&raid, &RaidTargetRow::default(), &kind),
            Ok(None)
        );
        raid.phase = 4;
        assert!(
            context
                .progress(&raid, &RaidTargetRow::default(), &kind)
                .is_err()
        );
    });
}
#[test]
#[ignore = "synthetic hover cost; release, affinity 0xF"]
fn hover_build_cost() {
    use super::super::super::tests::put;
    use replay::rows::{CountryLinkRow, CountryRow};
    let countries: u32 = std::env::var("BATTLE_PLAN_RAID_SCORE_COUNTRIES").or_else(|_| std::env::var("HOI4_RAID_SCORE_COUNTRIES"))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1500);
    let mut assets = assets();
    let mut frame = frame();
    assets
        .tooltip
        .state_names
        .insert(10, Ok("state_name".into()));
    put::<tables::Countries>(
        &mut frame,
        (0..countries)
            .map(|slot| CountryRow {
                id: slot + 10,
                canonical_slot: slot,
                tag: format!("C{slot}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    );
    put::<tables::CountryLinks>(
        &mut frame,
        (0..countries)
            .map(|slot| CountryLinkRow {
                id: slot + 10,
                canonical_slot: slot,
            })
            .collect(),
    );
    put::<tables::Raids>(
        &mut frame,
        vec![RaidRow {
            id: 1,
            reference_type: 84,
            phase: 5,
            ..Default::default()
        }],
    );
    put::<tables::RaidTargets>(
        &mut frame,
        vec![RaidTargetRow {
            id: 1,
            reference_type: 84,
            raid_type: 100,
            scope: RaidScope {
                state: 10,
                ..Default::default()
            },
            ..Default::default()
        }],
    );
    let loc: Localisation = [
        ("raid_inst_item_desc".into(), "$TYPE$ $TARGET$".into()),
        (
            "tooltip_raid_result".into(),
            "$RAID_DESC$ $OUTCOME_TYPE$".into(),
        ),
    ]
    .into_iter()
    .collect();
    let mut samples = Vec::new();
    for _ in 0..100 {
        let start = std::time::Instant::now();
        let world = World::new(&frame, assets.facts()).unwrap();
        let context = Context {
            assets: &assets,
            world: &world,
            loc: &loc,
            recorded_language: &loc.language,
            scripted_content: &Err("no scripted content in this test".into()),
            ours: 0,
            flag_exists: &|_| true,
            scripted: Default::default(),
            countries: installed_content::country_names::CountryNames::read(&frame, &loc),
        };
        let tooltip = context.raid(1, 84, false).unwrap();
        assert!(tooltip.refused.is_empty(), "{:?}", tooltip.refused);
        std::hint::black_box(tooltip);
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "raid hover ended countries={countries} median_ms={} max_ms={}",
        samples[50], samples[99]
    );
}
