use super::super::super::test_support::{assets, frame, positive_frame};
use super::*;
use std::cell::OnceCell;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use viewer_native_gui::native_text::FLAG;
#[test]
fn controller_flag_uses_native_control_and_three_codepoints() {
    use dashboard_text::flag_choices::{Choices, FlagChoice};
    with_context(|context| {
        for tag in ["GER", "ÄØ中", "ABCD"] {
            let flags = OnceCell::from(Ok(Choices::Available {
                choices: vec![FlagChoice {
                    raw: 10,
                    tag: tag.into(),
                    candidates: vec!["GER_fascism".into()],
                    refusal: None,
                }],
                incomplete: None,
            }));
            let result = context.flag(2, &flags);
            if tag == "ABCD" {
                assert_eq!(
                    result,
                    Err("controller flag tag is not three codepoints".into())
                );
            } else {
                assert_eq!(result, Ok(format!("{}{tag}", FLAG)));
            }
        }
        let unavailable = OnceCell::from(Ok(Choices::Unavailable(
            "country links: not available (damaged block)".into(),
        )));
        assert_eq!(
            context.flag(2, &unavailable),
            Err("country links: not available (damaged block)".into())
        );
    });
}

#[test]
fn facility_threshold_is_strict_and_first_program_controls_project_text() {
    use super::super::super::test_support::put;
    use replay_data::rows::RaidFacilityRow;
    for (required, first_project) in [(1, Some(100)), (0, None), (0, Some(100))] {
        let mut assets = assets();
        assets
            .trigger_content
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
        let mut projects = content_reader_military::special_projects::Catalog::default();
        projects.projects = [("oilfield".into(), Default::default())].into();
        assets.tooltip.projects = Ok(projects);
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
            vec![replay_data::rows::CountryAppearanceRow {
                country: 11,
                ..Default::default()
            }],
        );
        put::<tables::Politics>(
            &mut frame,
            vec![replay_data::rows::PoliticsRow {
                country: 11,
                ..Default::default()
            }],
        );
        let loc: Localisation = [("oilfield".into(), "Viewer project".into())]
            .into_iter()
            .collect();
        let scripted_content = Err("no scripted content in this test".into());
        let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
        let context = Context {
            assets: &assets,
            world: &world,
            loc: &loc,
            recorded_language: &loc.language,
            scripted_content: &scripted_content,
            ours: 0,
            nato: false,
            countries: content_reader_country_names::country_names::CountryNames::read(&frame, &loc),
            flag_exists: &|_| true,
            scripted: Default::default(),
        };
        let scope = RaidScope {
            building_type: 200,
            building_flags: 1,
            building_location: 3,
            ..Default::default()
        };
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
    content_reader_script::test_game::enter();
    use super::super::super::test_support::put;
    for narrative in [None, Some("narrative"), Some("")] {
        for country_override in [true, false] {
            let mut assets = assets();
            assets
                .trigger_content
                .buildings
                .types
                .get_mut("dam")
                .unwrap()
                .specializations
                .push("test".into());
            assets.tooltip.intel = Ok(intel::Defines::default());
            let mut project = content_reader_military::special_projects::Project::default();
            project.name = narrative.map(Into::into);
            let mut projects = content_reader_military::special_projects::Catalog::default();
            projects.projects = [("oilfield".into(), project)].into();
            assets.tooltip.projects = Ok(projects);
            let mut frame = positive_frame();
            put::<tables::RaidFacilities>(
                &mut frame,
                vec![replay_data::rows::RaidFacilityRow {
                    country: 11,
                    owner: 10,
                    province: 3,
                    project_definition: Some(100),
                    ..Default::default()
                }],
            );
            put::<tables::CountryAppearance>(
                &mut frame,
                vec![replay_data::rows::CountryAppearanceRow {
                    country: 10,
                    name_tag: b"ENG".to_vec(),
                    ..Default::default()
                }],
            );
            put::<tables::Politics>(
                &mut frame,
                vec![replay_data::rows::PoliticsRow {
                    country: 10,
                    ..Default::default()
                }],
            );
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
            let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
            let context = Context {
                assets: &assets,
                world: &world,
                loc: &loc,
                recorded_language: "german",
                scripted_content: &scripted,
                ours: 1,
                nato: false,
                countries: content_reader_country_names::country_names::CountryNames::read(&frame, &loc),
                flag_exists: &|_| true,
                scripted: Default::default(),
            };
            let scope = RaidScope {
                building_type: 200,
                building_flags: 1,
                building_location: 3,
                ..Default::default()
            };
            let rules::Facility::Project(name) =
                context.facility(&scope, &OnceCell::from(Ok(0))).unwrap()
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
    use super::super::super::test_support::put;
    use replay_data::rows::UnitLeaderRow;
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
    let loc: Localisation = [("original_name".into(), "Viewer language name".into())]
        .into_iter()
        .collect();
    let scripted_content = Err("no scripted content in this test".into());
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    let context = Context {
        assets: &assets,
        world: &world,
        loc: &loc,
        recorded_language: &loc.language,
        scripted_content: &scripted_content,
        ours: 0,
        nato: false,
        flag_exists: &|_| true,
        scripted: Default::default(),
        countries: content_reader_country_names::country_names::CountryNames::read(&frame, &loc),
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
    assert_eq!(
        context.captive(&RaidScope {
            leader_reference: (8 << 32) | 84,
            ..Default::default()
        }),
        Err("captive leader is absent from UnitLeaders".into())
    );
}
fn scripted_content() -> dashboard_text::scripted_text::ScriptedContent {
    let (scripted, root) = scripted_content_with_files(&[]);
    std::fs::remove_dir_all(root).unwrap();
    scripted
}

fn scripted_content_with_files(
    files: &[(&str, &str)],
) -> (dashboard_text::scripted_text::ScriptedContent, PathBuf) {
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
        "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 23 } }",
    )
    .unwrap();
    for (relative, contents) in files {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let content = content_reader_layers::content::ResolvedContent {
        layers,
        stats: Default::default(),
        changed: Vec::new(),
        notes: Vec::new(),
    };
    let scripted =
        dashboard_text::scripted_text::ScriptedContent::read(Arc::new(content), String::new())
            .unwrap()
            .with_effect_describer(dashboard_effects::effect_describer);
    (scripted, root)
}

#[test]
fn raid_custom_trigger_nested_effect_icons_follow_the_viewer_nato_toggle() {
    content_reader_script::test_game::enter();
    use super::super::super::test_support::put;
    let (scripted, root) = scripted_content_with_files(&[
        ("common/units/test.txt", "sub_units={syn_unit={}}"),
        (
            "common/bop/test.txt",
            "balance={range={id=active on_activate={add_unit_bonus={syn_unit={}}}}}",
        ),
        (
            "common/scripted_localisation/test.txt",
            "entry={name=IconEntry text={localization_key=ICON_BODY}}",
        ),
    ]);
    let scripted = Ok(scripted);
    let assets = assets();
    let mut frame = positive_frame();
    put::<tables::PowerBalances>(
        &mut frame,
        vec![replay_data::rows::PowerBalanceRow {
            balance: b"balance".to_vec(),
            active_range: b"active".to_vec(),
            ..Default::default()
        }],
    );
    put::<tables::PowerBalanceMembers>(
        &mut frame,
        vec![replay_data::rows::PowerBalanceMemberRow {
            country: 10,
            ..Default::default()
        }],
    );
    let loc: Localisation = [
        ("ICON_TOOLTIP", "[IconEntry]"),
        ("ICON_BODY", "[ROOT.GetActiveRangeActivationEffect]"),
        ("PROGRESS_SECTION_ACTIVATION_EFFECT", "Activate:\n$DESC$"),
        ("SUBUNIT_BONUS_HEADER", "$ICON$$NAME$"),
        ("syn_unit", "Unit"),
        ("TRIGGER_FULLFILLED_PREFIX", "+"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect();
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    for nato in [false, true, false] {
        let context = Context {
            assets: &assets,
            world: &world,
            loc: &loc,
            recorded_language: &loc.language,
            scripted_content: &scripted,
            ours: 2,
            nato,
            countries: content_reader_country_names::country_names::CountryNames::read(&frame, &loc),
            flag_exists: &|_| true,
            scripted: Default::default(),
        };
        let lines = context.trigger_text(
            &triggers("custom_trigger_tooltip={tooltip=ICON_TOOLTIP always=yes}"),
            Scopes::of(Scope::Country(2)),
            "test",
        );
        assert_eq!(lines.len(), 1);
        let icon_frame = if nato { 2 } else { 1 };
        assert_eq!(
            lines[0].1.as_ref().unwrap(),
            &format!("+Activate:\n\u{13}GFX_syn_unit_icon_small|{icon_frame} Unit\n")
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
fn with_context(run: impl FnOnce(&Context<'_, '_>)) {
    use super::super::super::test_support::put;
    let assets = assets();
    let scripted = Ok(scripted_content());
    let mut frame = positive_frame();
    put::<tables::CountryAppearance>(
        &mut frame,
        [10, 11]
            .into_iter()
            .map(|country| replay_data::rows::CountryAppearanceRow {
                country,
                ..Default::default()
            })
            .collect(),
    );
    put::<tables::Politics>(
        &mut frame,
        [10, 11]
            .into_iter()
            .map(|country| replay_data::rows::PoliticsRow {
                country,
                ..Default::default()
            })
            .collect(),
    );
    let loc: Localisation = [
        ("GER", "Viewer country"),
        ("ENG", "Target country"),
        ("TRIGGER_ALWAYS_FALSE", "never"),
        ("TRIGGER_ALWAYS_TRUE", "always"),
        ("TRIGGER_FULLFILLED_PREFIX", "+"),
        ("TRIGGER_UNFULLFILLED_PREFIX", "-"),
        ("TRIGGER_OR_STARTS", "either"),
        ("TRIGGER_AND_STARTS", "both"),
        ("custom", "custom text"),
        ("custom_lf", "custom text\n"),
        ("custom_blank", "custom text\n\n"),
        (
            "scoped",
            "[ROOT.GetTag]/[FROM.GetID]/[THIS.GetTag]/[PREV.GetTag]",
        ),
        ("scoped_NOT", "negative [FROM.GetID]"),
        ("scoped_inner", "[ROOT.GetTag]/[THIS.GetTag]/[PREV.GetTag]/[PREV.FROM.GetTag]/[?target_province|0]"),
        ("chain", "[FROM.ROOT.GetTag]/[ROOT.FROM.PREV.GetTag]"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect();
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    let context = Context {
        assets: &assets,
        world: &world,
        loc: &loc,
        recorded_language: &loc.language,
        scripted_content: &scripted,
        ours: 2,
        nato: false,
        countries: content_reader_country_names::country_names::CountryNames::read(&frame, &loc),
        flag_exists: &|_| true,
        scripted: Default::default(),
    };
    run(&context);
}
fn triggers(text: &str) -> content_reader_script::triggers::Triggers {
    content_reader_parser::parser::parse(text.as_bytes())
        .unwrap()
        .items
}
#[test]
fn trigger_inversion_indentation_custom_override_and_named_unsupported_leaf() {
    content_reader_script::test_game::enter();
    with_context(|context| {
        let scope = Scopes::of(Scope::Country(2));
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
        let text = context.trigger_text(
            &triggers("custom_trigger_tooltip={tooltip=custom always=no} hidden_trigger={always=no} synthetic_unported_trigger=yes"),
            scope,
            "test",
        );
        assert_eq!(text.len(), 2);
        assert_eq!(
            text[0].1.as_deref(),
            Ok("-custom text
")
        );
        assert!(text[1].1.as_ref().unwrap_err().contains("synthetic_unported_trigger"));
        let terminated = context.trigger_text(
            &triggers("custom_trigger_tooltip={tooltip=custom_lf always=no} custom_trigger_tooltip={tooltip=custom_blank always=yes}"),
            scope,
            "test",
        );
        assert_eq!(terminated.len(), 2);
        assert_eq!(terminated[0].1.as_deref(), Ok("-custom text\n"));
        assert_eq!(terminated[1].1.as_deref(), Ok("+custom text\n\n"));
    });
}

#[test]
fn raid_tooltip_regression_special_project_requirement_uses_native_name_and_inversion() {
    use super::super::super::test_support::put;
    let mut assets = assets();
    let mut project = content_reader_military::special_projects::Project::default();
    project.name = Some("project_name".into());
    let mut projects = content_reader_military::special_projects::Catalog::default();
    projects.projects = [("oilfield".into(), project)].into();
    assets.tooltip.projects = Ok(projects);
    let mut frame = frame();
    put::<tables::CountryAppearance>(
        &mut frame,
        vec![replay_data::rows::CountryAppearanceRow {
            country: 10,
            ..Default::default()
        }],
    );
    put::<tables::Politics>(
        &mut frame,
        vec![replay_data::rows::PoliticsRow {
            country: 10,
            ..Default::default()
        }],
    );
    put::<tables::Projects>(
        &mut frame,
        vec![replay_data::rows::ProjectRow {
            owner: 10,
            definition_id: 100,
            started: 0,
            completed: 0,
            ..Default::default()
        }],
    );
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
    let scripted_content = Err("no scripted content in this test".into());
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    let context = Context {
        assets: &assets,
        world: &world,
        loc: &loc,
        recorded_language: &loc.language,
        scripted_content: &scripted_content,
        ours: 0,
        nato: false,
        countries: content_reader_country_names::country_names::CountryNames::read(&frame, &loc),
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
    content_reader_script::test_game::enter();
    with_context(|context| {
        let scopes = Scopes {
            root: Scope::Country(2),
            from: Some(Scope::State(10)),
            prev: Some(Scope::Country(2)),
            ..Scopes::of(Scope::Country(1))
        };
        let text = context.trigger_text(
            &triggers("custom_trigger_tooltip={tooltip=scoped always=yes}"),
            scopes,
            "test",
        );
        assert_eq!(text[0].1.as_deref(), Ok("+GER/10/ENG/GER\n"));
        let zero = context.trigger_text(
            &triggers("custom_trigger_tooltip={tooltip=scoped always=yes}"),
            Scopes {
                root: Scope::Country(0),
                ..scopes
            },
            "test",
        );
        assert!(
            zero[0]
                .1
                .as_ref()
                .unwrap_err()
                .contains("reserved country naming has no singleton row")
        );
        let text = context.trigger_text(
            &triggers("NOT={custom_trigger_tooltip={tooltip=scoped always=yes}}"),
            scopes,
            "test",
        );
        assert_eq!(text[0].1.as_deref(), Ok("-negative 10\n"));
    });
}

#[test]
fn custom_tooltip_chains_follow_the_raid_scopes_links() {
    content_reader_script::test_game::enter();
    with_context(|context| {
        let raid = Scopes {
            from: Some(Scope::Country(1)),
            ..Scopes::of(Scope::Country(2))
        };
        let tooltip = triggers("custom_trigger_tooltip={tooltip=chain always=yes}");
        let text = context.trigger_text(&tooltip, raid, "test");
        assert_eq!(text[0].1.as_deref(), Ok("+ENG/ENG\n"));
        let nested = Scopes {
            prev: Some(Scope::Country(2)),
            ..raid
        };
        let text = context.trigger_text(&tooltip, nested, "test");
        let refused = text[0].1.as_ref().unwrap_err();
        assert!(refused.contains("Root link is not known"), "{refused}");
        let scoped = context.trigger_text(
            &triggers("FROM={custom_trigger_tooltip={tooltip=scoped_inner always=no}}"),
            Scopes {
                target_province: Some(9),
                ..raid
            },
            "test",
        );
        assert_eq!(scoped.len(), 1);
        assert_eq!(
            scoped[0].1.as_deref(),
            Ok("-\u{11}YTarget country\u{11}!: GER/ENG/GER/ENG/9\n")
        );
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
    let organisation = 9 << 32 | 79;
    let nested = event_scopes(Scopes {
        this: Scope::IndustrialOrganisation(organisation),
        ..raid
    });
    assert_eq!(
        nested.scopes[0].this,
        EventObject::IndustrialOrganisation(organisation)
    );
    for (scope, object) in [
        (
            Scope::Project { owner: 100, id: 9 },
            EventObject::Project { owner: 100, id: 9 },
        ),
        (Scope::Raid(7 << 32 | 84), EventObject::Raid(7 << 32 | 84)),
        (Scope::Unit(8 << 32 | 55), EventObject::Unit(8 << 32 | 55)),
    ] {
        let nested = event_scopes(Scopes {
            this: scope,
            ..raid
        });
        assert_eq!(nested.scopes[0].this, object);
        assert_eq!(nested.scopes[0].root, Link::Scope(1));
    }
}

#[test]
fn state_name_uses_the_stored_owner_name_in_the_recording_language_only() {
    use super::super::super::test_support::put;
    let assets = assets();
    let mut frame = frame();
    put::<tables::States>(
        &mut frame,
        vec![replay_data::rows::StateRow {
            id: 10,
            owner_name: b"owner cache $literal$".to_vec(),
            ..Default::default()
        }],
    );
    let mut loc: Localisation = [("owner cache $literal$".into(), "must not localise".into())]
        .into_iter()
        .collect();
    loc.language = "english".into();
    let scripted_content = Err("no scripted content in this test".into());
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    let context = Context {
        assets: &assets,
        world: &world,
        loc: &loc,
        recorded_language: &loc.language,
        scripted_content: &scripted_content,
        ours: 0,
        nato: false,
        countries: content_reader_country_names::country_names::CountryNames::read(&frame, &loc),
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
            tooltip: content_reader_raids::raids::tooltips::Definition {
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
fn raid_target_names_refuse_unknown_owners_and_preserve_recorded_null_states() {
    use super::super::super::test_support::put;
    use replay_data::{rows::Recorded, rows::ReservedCountryNamingRow};
    let assets = assets();
    let loc: Localisation = [("VICTORY_POINTS_1".into(), "Wrong bare fallback".into())]
        .into_iter()
        .collect();
    for (owner, state) in [(999, 10), (0, 10), (10, 0)] {
        let mut frame = frame();
        put::<tables::ReservedCountryNaming>(
            &mut frame,
            vec![ReservedCountryNamingRow {
                tag: b"ZZ0".to_vec(),
                name_tag: b"unrelated_generator".to_vec(),
                original_tag_reference: Recorded::NotRecorded,
                original_tag: Recorded::Value(Some(b"unused_original".to_vec())),
                ..Default::default()
            }],
        );
        if state == 0 {
            let mut provinces = frame.complete::<tables::Provinces>().unwrap().to_vec();
            provinces[0].state = 0;
            put::<tables::Provinces>(&mut frame, provinces);
        }
        put::<tables::States>(
            &mut frame,
            vec![replay_data::rows::StateRow {
                id: 10,
                owner,
                custom_name: b"Recorded state".to_vec(),
                ..Default::default()
            }],
        );
        let scripted_content = Err("no scripted content in this test".into());
        let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
        let context = Context {
            assets: &assets,
            world: &world,
            loc: &loc,
            recorded_language: &loc.language,
            scripted_content: &scripted_content,
            ours: 0,
            nato: false,
            countries: content_reader_country_names::country_names::CountryNames::read(&frame, &loc),
            flag_exists: &|_| true,
            scripted: Default::default(),
        };
        let reason = context
            .location(&RaidScope {
                province: 1,
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(
            reason,
            if owner == 0 || state == 0 {
                "original tag reference of reserved country naming not recorded"
            } else {
                "raid target state's owner 999 is unresolved"
            },
        );
        if state == 0 {
            for scope in [
                RaidScope {
                    province: 1,
                    ..Default::default()
                },
                RaidScope {
                    leader_province: 1,
                    ..Default::default()
                },
                RaidScope {
                    building_type: 4,
                    building_flags: 1,
                    building_location: 1,
                    ..Default::default()
                },
            ] {
                assert_eq!(intel::target_state(&world, &scope), Ok(None));
                if scope.building_type != 0 {
                    assert_eq!(context.location(&scope).as_deref(), Ok("no_target_name"));
                }
            }
        }
    }
}

#[test]
fn details_put_launchable_types_first_and_name_the_only_one() {
    let order = |types: &[(usize, Result<bool>)]| detail_order(types);
    let (named, buttons) = order(&[(4, Ok(false)), (2, Ok(true)), (9, Ok(false))]);
    assert_eq!(named, Ok(Some(2)));
    assert_eq!(buttons, Ok(vec![(2, true), (4, false), (9, false)]));
    let (named, buttons) = order(&[(1, Ok(true)), (3, Ok(true)), (5, Err("unread".into()))]);
    assert_eq!(named, Ok(None));
    assert!(buttons.is_err());
    let (named, _) = order(&[(1, Ok(true)), (5, Err("unread".into()))]);
    assert!(named.is_err());
    let (named, buttons) = order(&[(1, Ok(false))]);
    assert_eq!((named, buttons), (Ok(None), Ok(vec![(1, false)])));
}
