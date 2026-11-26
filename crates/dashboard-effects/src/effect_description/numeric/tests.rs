use super::*;
use crate::effect_description::tests::{Fixture, fresh};
use replay_data::rows::*;
use replay_frame::frame::TableView;
use std::sync::Arc;
use crate::fixtures::game;

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
    game::enter();
    let fixture = fixture();
    assert_eq!(
        text(
            &fixture,
            "fixture_command_points=amount fixture_fuel_gain=-2.019 fixture_organisation_funds=2.019 fixture_organisation_assignment_cost=0.25"
        ),
        "Command 2.50 AAA\nFuel -2.01 AAA\nFunds 2.01\nCost 0.25\n"
    );
    assert_eq!(
        text(
            &fixture,
            "fixture_attack_gain=1.5 fixture_defense_gain=-1.5 fixture_logistics_gain=1.49 fixture_planning_gain=-1.49 fixture_coordination_gain=0.5 fixture_maneuver_gain=-0.5 fixture_skill_gain=0.49 fixture_stockpile_gain=1.5"
        ),
        "Attack 2 [This.GetTag]\nDefense -2\nLogistics 1\nPlanning -1\nCoordination 1\nManeuver -1\nSkill 0\nNukes 2\n"
    );
    assert_eq!(
        text(
            &fixture,
            "fixture_industrial_gain=-1.9 fixture_task_slots=1.9 fixture_organisation_growth=1.9 fixture_organisation_growth=2.9 fixture_organisation_growth=-2.9"
        ),
        "CIC -1\nTasks 1\nSize 1\nSizes 2\nSize -2\n"
    );
    assert_eq!(
        text(&fixture, "fixture_command_points=ROOT"),
        "Command 0.00 AAA\n"
    );
}

#[test]
fn int_effect_literal_scoped_clamp_sign_and_no_line_feed() {
    game::enter();
    let fixture = fixture();
    assert_eq!(text(&fixture, "fixture_experience_gain=010"), "XP 8 [This.GetTag]");
    assert_eq!(text(&fixture, "fixture_experience_gain=amount"), "XP 2 [This.GetTag]");
    assert_eq!(text(&fixture, "fixture_experience_gain=-2"), "XP 0 [This.GetTag]");
    assert_eq!(
        text(&fixture, "fixture_threat_change=-2 fixture_threat_change=0 fixture_threat_change=amount"),
        "Remove threat 2\nRemove threat 0\nThreat 2\n"
    );
}

#[test]
fn garrison_colour_percent_and_zero_plus_follow_the_helper() {
    game::enter();
    let fixture = fixture();
    assert_eq!(
        text(
            &fixture,
            "fixture_garrison_readiness=1.59 fixture_garrison_readiness=-1.59 fixture_garrison_readiness=0"
        ),
        "Garrison \u{11}G+1.5%\u{11}!\nGarrison \u{11}R-1.5%\u{11}!\nGarrison \u{11}Y+0.0%\u{11}!\n"
    );
}

#[test]
fn collaboration_gate_and_sign_do_not_add_a_line_feed() {
    game::enter();
    let mut fixture = fixture();
    assert_eq!(
        text(&fixture, "fixture_collaboration_gain={target=BBB value=-0.25}"),
        "Remove collaboration beta/0.25"
    );
    assert_eq!(text(&fixture, "fixture_collaboration_gain={target=BBB value=0}"), "");
    *fixture.frame.tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![
            SimulationFeatureRow::default(),
        ]));
    *fixture.frame.tables[TableId::ScriptVariables.index()] = TableView::Unavailable {
        reason: "synthetic gap".into(),
    };
    assert_eq!(
        text(&fixture, "fixture_collaboration_gain={target=BBB value=amount}"),
        ""
    );
}

#[test]
fn decryption_ratio_assignment_wins_and_marked_country_is_capitalized() {
    game::enter();
    let fixture = fixture();
    assert_eq!(
        text(
            &fixture,
            "fixture_decryption_gain={target=BBB amount=1.5} fixture_decryption_gain={target=BBB ratio=0.25 amount=8} fixture_decryption_gain={target=AAA ratio=amount}"
        ),
        "Decrypt \u{12}BBB Beta/2\nRatio \u{12}BBB Beta/0.25\n"
    );
}

#[test]
fn intel_uses_owned_state_existence_and_domain_order() {
    game::enter();
    let mut fixture = fixture();
    assert_eq!(
        text(
            &fixture,
            "fixture_intel_gain={target=BBB airforce_intel=0 navy_intel=-0.125 civilian_intel=12.34 army_intel=amount}"
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
        text(&fixture, "fixture_intel_gain={target=BBB civilian_intel=amount}"),
        ""
    );
}

#[test]
fn intel_nonowning_target_requires_its_original_tag_bucket() {
    game::enter();
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
            "fixture_intel_gain={target=BBB civilian_intel=1}",
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
    game::enter();
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
        text(&fixture, "fixture_truce_length={target=BBB days=1.5}"),
        "Truce beta/3 SyntheticJanuary, 1936"
    );
}

#[test]
fn imports_and_licenses_use_content_and_ignore_execution_only_fields() {
    game::enter();
    let fixture = fixture();
    let resources = [content_reader_key_names::resources::Resource {
        token: "syn_resource".into(),
        icon_frame: Some(7),
    }];
    let names = content_reader_script::key_names::KeyNames::default();
    for (body, expected) in [
        (
            "fixture_import_order={resource=syn_resource amount=4 exporter=BBB factories=100}",
            "Import 4\u{13}resources_strip|7 /Alpha/Beta",
        ),
        (
            "fixture_import_order={resource=syn_resource amount=4 exporter=BBB start_date=1940.2.3}",
            "Import 4\u{13}resources_strip|7 /Alpha/Beta/1940.2.3",
        ),
        (
            "fixture_licensed_production={target=BBB equipment={version_name=\"Raw [This.GetTag]\"} cost_factor=9 new_prioritised=yes}",
            "License beta/Raw AAA\n",
        ),
        (
            "fixture_licensed_production={equipment={version_name=Raw}}",
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
    game::enter();
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
            "fixture_import_order={resource=syn_resource amount=4 exporter=0}",
            "Import 4\u{13}resources_strip|7 /Alpha/Beta",
        ),
        (
            "fixture_licensed_production={target=0 equipment={version_name=Raw}}",
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
    game::enter();
    let fixture = fixture();
    let out = fixture.render(
        "fixture_mine_total={region=0 amount=amount} fixture_mine_total={region=7 amount=1} fixture_launch_strike={state=1}",
        &mut fresh(),
        Content::default(),
    );
    assert!(out.text.is_empty());
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(
        out.refusals
            .keys()
            .any(|name| name.contains("fixture_mine_total") && name.contains("unrecorded instance name"))
    );
}
