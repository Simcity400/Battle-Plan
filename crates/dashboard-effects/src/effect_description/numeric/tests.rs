use super::*;
use crate::effect_description::tests::{Fixture, fresh};
use replay_data::rows::*;
use replay_frame::frame::TableView;
use std::sync::Arc;

fn fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.catalogue.localisation = fixture
        .catalogue
        .localisation
        .iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .chain(
            [
                ("POLITICS_ADD_FUEL", "Fuel $VAL|2$ [This.GetTag]"),
                ("EFFECT_ADD_COMMAND_POWER", "Command $VAL|2$ [This.GetTag]"),
                ("ADD_ATTACK_SKILL", "Attack $NUM|0$ [This.GetTag]"),
                ("ADD_DEFENSE_SKILL", "Defense $NUM|0$"),
                ("ADD_LOGISTICS_SKILL", "Logistics $NUM|0$"),
                ("ADD_PLANNING_SKILL", "Planning $NUM|0$"),
                ("ADD_COORDINATION_SKILL", "Coordination $NUM|0$"),
                ("ADD_MANEUVER_SKILL", "Maneuver $NUM|0$"),
                ("ADD_SKILL_LEVEL", "Skill $NUM|0$"),
                ("EFFECT_ADD_NUKE", "Nukes $VAL|0$"),
                ("EFFECT_ADD_CIC", "CIC $VAL|0$"),
                ("EFFECT_MIO_ADD_TASK_CAPACITY", "Tasks $VAL|0$"),
                ("EFFECT_MIO_ADD_SIZE_SINGULAR", "Size $VAL|0$"),
                ("EFFECT_MIO_ADD_SIZE_PLURAL", "Sizes $VAL|0$"),
                ("EFFECT_MIO_ADD_FUNDS", "Funds $VAL|2$"),
                ("EFFECT_MIO_SET_RESEARCH_ASSIGN_COST", "Cost $VAL|2$"),
                ("EFFECT_GAIN_XP", "XP $VALUE|0$ [This.GetTag]"),
                ("EFFECT_ADD_THREAT", "Threat $THREAT|0$"),
                ("EFFECT_REMOVE_THREAT", "Remove threat $THREAT|0$"),
                ("EFFECT_SET_GARRISON_STRENGTH", "Garrison $VALUE$"),
                ("EFFECT_ADD_COLLABORATION", "Collaborate $TARGET$/$VALUE|2$"),
                (
                    "EFFECT_REMOVE_COLLABORATION",
                    "Remove collaboration $TARGET$/$VALUE|2$",
                ),
                ("EFFECT_ADD_DECRYPTION", "Decrypt $COUNTRY$/$AMOUNT|0$"),
                ("EFFECT_ADD_DECRYPTION_RATIO", "Ratio $COUNTRY$/$RATIO|2$"),
                ("EFFECT_ADD_INTEL", "Intel $SOURCE$/$TARGET$:$INTEL$"),
                ("MODIFIER_CIVILIAN_INTEL_FACTOR", "Civilian"),
                ("MODIFIER_ARMY_INTEL_FACTOR", "Army"),
                ("MODIFIER_NAVY_INTEL_FACTOR", "Navy"),
                ("MODIFIER_AIRFORCE_INTEL_FACTOR", "Air"),
                ("EFFECT_SET_TRUCE", "Truce $COUNTRY$/$DATE$"),
                ("January", "January"),
                (
                    "EFFECT_CREATE_IMPORT",
                    "Import $RESOURCE$/$IMPORTER$/$EXPORTER$",
                ),
                (
                    "EFFECT_CREATE_IMPORT_HAS_DATA",
                    "Import $RESOURCE$/$IMPORTER$/$EXPORTER$/$DATE$",
                ),
                (
                    "EFFECT_CREATE_LICENSE_PRODUCTION",
                    "License $TARGET$/$NAME$",
                ),
            ]
            .into_iter()
            .map(|(key, value)| (key.into(), value.into())),
        )
        .collect::<Localisation>()
        .into();
    Arc::make_mut(&mut fixture.catalogue.localisation).language = "english".into();
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 100,
            entries: vec![ScriptVariableEntry {
                name: b"amount".to_vec(),
                value_raw: 250_000,
            }],
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            dlc_mask: 1 << 5,
            ..Default::default()
        }]));
    fixture
}

fn text(fixture: &Fixture, body: &str) -> String {
    let out = fixture.render(body, &mut fresh(), Content::default());
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    out.text
}

#[test]
fn fixed5_template_and_numeric_integer_forms() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = fixture();
    assert_eq!(
        text(
            &fixture,
            "add_command_power=amount add_fuel=-2.019 add_mio_funds=2.019 set_mio_design_team_assign_cost=0.25"
        ),
        "Command 2.50 AAA\nFuel -2.01 AAA\nFunds 2.01\nCost 0.25\n"
    );
    assert_eq!(
        text(
            &fixture,
            "add_attack=1.5 add_defense=-1.5 add_logistics=1.49 add_planning=-1.49 add_coordination=0.5 add_maneuver=-0.5 add_skill_level=0.49 add_nuclear_bombs=1.5"
        ),
        "Attack 2 [This.GetTag]\nDefense -2\nLogistics 1\nPlanning -1\nCoordination 1\nManeuver -1\nSkill 0\nNukes 2\n"
    );
    assert_eq!(
        text(
            &fixture,
            "add_cic=-1.9 add_mio_task_capacity=1.9 add_mio_size=1.9 add_mio_size=2.9 add_mio_size=-2.9"
        ),
        "CIC -1\nTasks 1\nSize 1\nSizes 2\nSize -2\n"
    );
    assert_eq!(
        text(&fixture, "add_command_power=ROOT"),
        "Command 0.00 AAA\n"
    );
}

#[test]
fn int_effect_literal_scoped_clamp_sign_and_no_line_feed() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = fixture();
    assert_eq!(text(&fixture, "gain_xp=010"), "XP 8 [This.GetTag]");
    assert_eq!(text(&fixture, "gain_xp=amount"), "XP 2 [This.GetTag]");
    assert_eq!(text(&fixture, "gain_xp=-2"), "XP 0 [This.GetTag]");
    assert_eq!(
        text(&fixture, "add_threat=-2 add_threat=0 add_threat=amount"),
        "Remove threat 2\nRemove threat 0\nThreat 2\n"
    );
}

#[test]
fn garrison_colour_percent_and_zero_plus_follow_the_helper() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = fixture();
    assert_eq!(
        text(
            &fixture,
            "set_garrison_strength=1.59 set_garrison_strength=-1.59 set_garrison_strength=0"
        ),
        "Garrison \u{11}G+1.5%\u{11}!\nGarrison \u{11}R-1.5%\u{11}!\nGarrison \u{11}Y+0.0%\u{11}!\n"
    );
}

#[test]
fn collaboration_gate_and_sign_do_not_add_a_line_feed() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = fixture();
    assert_eq!(
        text(&fixture, "add_collaboration={target=BBB value=-0.25}"),
        "Remove collaboration beta/0.25"
    );
    assert_eq!(text(&fixture, "add_collaboration={target=BBB value=0}"), "");
    *fixture.frame.tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![
            SimulationFeatureRow::default(),
        ]));
    *fixture.frame.tables[TableId::ScriptVariables.index()] = TableView::Unavailable {
        reason: "synthetic gap".into(),
    };
    assert_eq!(
        text(&fixture, "add_collaboration={target=BBB value=amount}"),
        ""
    );
}

#[test]
fn decryption_ratio_assignment_wins_and_marked_country_is_capitalized() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = fixture();
    assert_eq!(
        text(
            &fixture,
            "add_decryption={target=BBB amount=1.5} add_decryption={target=BBB ratio=0.25 amount=8} add_decryption={target=AAA ratio=amount}"
        ),
        "Decrypt \u{12}BBB Beta/2\nRatio \u{12}BBB Beta/0.25\n"
    );
}

#[test]
fn intel_uses_owned_state_existence_and_domain_order() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = fixture();
    assert_eq!(
        text(
            &fixture,
            "add_intel={target=BBB airforce_intel=0 navy_intel=-0.125 civilian_intel=12.34 army_intel=amount}"
        ),
        "Intel alpha/beta:\nCivilian: 12.34%, \nArmy: 2.5%, \nNavy: -0.125%"
    );
    let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *fixture.frame.tables[TableId::CountryCounterInputs.index()]
    else {
        unreachable!()
    };
    rows.iter_mut()
        .find(|row| row.country == 200)
        .unwrap()
        .owned_state_count = 0;
    *fixture.frame.tables[TableId::ScriptVariables.index()] = TableView::Unavailable {
        reason: "synthetic gap".into(),
    };
    *fixture.frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                original_tag_buckets: Recorded::Value(Some(vec![vec![], vec![], vec![], vec![]])),
                ..Default::default()
            },
        ]));
    assert_eq!(
        text(&fixture, "add_intel={target=BBB civilian_intel=amount}"),
        ""
    );
}

#[test]
fn intel_nonowning_target_requires_its_original_tag_bucket() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = fixture();
    let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *fixture.frame.tables[TableId::CountryCounterInputs.index()]
    else {
        unreachable!()
    };
    rows.iter_mut()
        .find(|row| row.country == 200)
        .unwrap()
        .owned_state_count = 0;
    for buckets in [Recorded::NotRecorded, Recorded::Value(None)] {
        *fixture.frame.tables[TableId::ReservedCountryNaming.index()] =
            TableView::Complete(TableRows::ReservedCountryNaming(vec![
                ReservedCountryNamingRow {
                    original_tag_buckets: buckets,
                    ..Default::default()
                },
            ]));
        let out = fixture.render(
            "add_intel={target=BBB civilian_intel=1}",
            &mut fresh(),
            Content::default(),
        );
        assert!(out.text.is_empty());
        assert_eq!(out.refusals.values().sum::<usize>(), 1);
        assert!(
            out.refusals
                .keys()
                .any(|why| why.contains("original tag buckets of reserved country naming")),
            "{:?}",
            out.refusals
        );
    }
}

#[test]
fn truce_rounds_days_from_current_midnight_and_uses_date_text() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = fixture();
    fixture.frame.hour = game_model::calendar::HOUR_EPOCH + 1936 * 8760 + 23;
    fixture.catalogue.localisation = std::sync::Arc::new(
        fixture
            .catalogue
            .localisation
            .iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .chain([("January".into(), "SyntheticJanuary".into())])
            .collect(),
    );
    assert_eq!(
        text(&fixture, "set_truce={target=BBB days=1.5}"),
        "Truce beta/3 SyntheticJanuary, 1936"
    );
}

#[test]
fn imports_and_licenses_use_content_and_ignore_execution_only_fields() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = fixture();
    let resources = [content_reader_key_names::resources::Resource {
        token: "syn_resource".into(),
        icon_frame: Some(7),
    }];
    let names = content_reader_script::key_names::KeyNames::default();
    for (body, expected) in [
        (
            "create_import={resource=syn_resource amount=4 exporter=BBB factories=100}",
            "Import 4\u{13}resources_strip|7 /Alpha/Beta",
        ),
        (
            "create_import={resource=syn_resource amount=4 exporter=BBB start_date=1940.2.3}",
            "Import 4\u{13}resources_strip|7 /Alpha/Beta/1940.2.3",
        ),
        (
            "create_production_license={target=BBB equipment={version_name=\"Raw [This.GetTag]\"} cost_factor=9 new_prioritised=yes}",
            "License beta/Raw AAA\n",
        ),
        (
            "create_production_license={equipment={version_name=Raw}}",
            "License alpha/Raw\n",
        ),
    ] {
        let out = fixture.render_native(
            body,
            &mut fresh(),
            Content {
                resources: Some(&resources),
                key_names: Some(&names),
                ..Default::default()
            },
        );
        assert_eq!(out.text, expected, "{body}");
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    }
}

#[test]
fn imports_and_licenses_resolve_reserved_country_names() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = fixture();
    *fixture.frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"BBB".to_vec(),
                built: 7,
                ..Default::default()
            },
        ]));
    let resources = [content_reader_key_names::resources::Resource {
        token: "syn_resource".into(),
        icon_frame: Some(7),
    }];
    let names = content_reader_script::key_names::KeyNames::default();
    for (body, expected) in [
        (
            "create_import={resource=syn_resource amount=4 exporter=0}",
            "Import 4\u{13}resources_strip|7 /Alpha/Beta",
        ),
        (
            "create_production_license={target=0 equipment={version_name=Raw}}",
            "License beta/Raw\n",
        ),
    ] {
        let out = fixture.render_native(
            body,
            &mut fresh(),
            Content {
                resources: Some(&resources),
                key_names: Some(&names),
                ..Default::default()
            },
        );
        assert_eq!(out.text, expected, "{body}: {:?}", out.refusals);
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
    }
}

#[test]
fn mines_missing_instance_names_are_counted_and_launch_nuke_returns_empty() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = fixture();
    let out = fixture.render(
        "add_mines={region=0 amount=amount} add_mines={region=7 amount=1} launch_nuke={state=1}",
        &mut fresh(),
        Content::default(),
    );
    assert!(out.text.is_empty());
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(
        out.refusals
            .keys()
            .any(|name| name.contains("add_mines") && name.contains("unrecorded instance name"))
    );
}
