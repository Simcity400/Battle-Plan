use super::*;
use crate::fixtures::{Folder, folder_catalogue};
use crate::effect_description::tests::{Fixture, fresh};
use replay_data::rows::*;
use replay_frame::frame::TableView;

fn fixture(output: &str) -> (Fixture, Folder) {
    let mut fixture = Fixture::new();
    let project = format!("mod_project={{specialization=mod_science project_output={{{output}}}}}");
    let (catalogue, folder) = folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}},NSupply={MAX_RAILWAY_LEVEL=5}}",
        ),
        ("common/country_tags/test.txt", "AAA=a BBB=b CCC=c"),
        (
            "common/characters/test.txt",
            "characters={mod_later={} mod_first={}}",
        ),
        ("common/special_projects/projects/test.txt", &project),
        (
            "common/special_projects/specialization/test.txt",
            "mod_science={}",
        ),
        (
            "common/buildings/test.txt",
            "buildings={mod_facility={specialization={mod_science} level_cap={province_max=1}}}",
        ),
        ("common/units/test.txt", "sub_units={mod_unit={}}"),
        (
            "common/units/equipment/test.txt",
            "equipments={mod_equipment={is_archetype=yes type=infantry}} equipment_modules={mod_module={allowed={always=yes}} mod_forbidden_module={allowed={always=no}}}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n CCC:0 \"Gamma\"\n",
                " mod_project:0 \"Project [Root.GetTag]\"\n EFFECT_COMPLETE_SPECIAL_PROJECT:0 \"Complete $PROJECT$\"\n SPECIAL_PROJECT_RESEARCH_OUTPUT:0 \"Output\"\n",
                " EFFECT_CONTEXT_SPECIAL_PROJECT:0 \"Project context $SPECIAL_PROJECT$\"\n EFFECT_ADD_SPECIAL_PROJECT_PROGRESS:0 \"Progress $PROJECT$ $PROGRESS|2$\"\n",
                " PROJECT_COUNTRY:0 \"Country [Root.GetTag]/[Prev.GetTag]\"\n PROJECT_SCIENTIST:0 \"Scientist [Root.GetName]\"\n PROJECT_FACILITY:0 \"Facility [Root.GetName]/[?facility_province_id|0]\"\n",
                " PROJECT_VARIABLE:0 \"Variable [?FROM.mod_value|2]\"\n",
                " SPECIAL_PROJECT_OUTPUT_SUBUNITS:0 \"Subunits:\"\n SPECIAL_PROJECT_OUTPUT_EQUIPMENT:0 \"Equipment:\"\n SPECIAL_PROJECT_OUTPUT_MODULE:0 \"Modules:\"\n",
                " mod_unit:0 \"Unit\"\n AAA_mod_unit:0 \"Alpha unit\"\n mod_equipment:0 \"Equipment\"\n AAA_mod_equipment:0 \"Alpha equipment\"\n mod_module:0 \"Module\"\n AAA_mod_module:0 \"Alpha module\"\n",
                " STAT_ARMY_SOFT_ATTACK:0 \"Soft:\"\n STAT_MODIFIER_DIFF:0 \"$VALUE|2$\"\n",
                " SUBUNIT_BONUS_HEADER:0 \"$ICON$$NAME$\"\n STAT_MODIFIER_VALUE:0 \"$VALUE|2$\"\n"
            ),
        ),
    ]);
    catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    fixture.catalogue = catalogue;
    *fixture.frame.tables[TableId::Projects.index()] =
        TableView::Complete(TableRows::Projects(vec![ProjectRow {
            owner: 100,
            local_id: 8,
            definition_id: 42,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 42,
                literal: b"mod_project".to_vec(),
            },
            LexicalTokenRow {
                token: 17,
                literal: b"mod_science".to_vec(),
            },
            LexicalTokenRow {
                token: 51,
                literal: b"mod_later".to_vec(),
            },
            LexicalTokenRow {
                token: 52,
                literal: b"mod_first".to_vec(),
            },
        ]));
    *fixture.frame.tables[TableId::CountryCharacters.index()] =
        TableView::Complete(TableRows::CountryCharacters(vec![
            CountryCharacterRow {
                country: 100,
                ordinal: 0,
                character: Some(7),
            },
            CountryCharacterRow {
                country: 100,
                ordinal: 1,
                character: Some(8),
            },
        ]));
    *fixture.frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![
            CharacterRow {
                id: 7,
                country: 100,
                name_token_id: 51,
                name: b"Later".to_vec(),
                ..Default::default()
            },
            CharacterRow {
                id: 8,
                country: 100,
                name_token_id: 52,
                name: b"First".to_vec(),
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::Scientists.index()] =
        TableView::Complete(TableRows::Scientists(vec![
            ScientistRow {
                country: 100,
                ordinal: 1,
                character: 7,
                ..Default::default()
            },
            ScientistRow {
                country: 100,
                ordinal: 0,
                character: 8,
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::ScientistSkills.index()] =
        TableView::Complete(TableRows::ScientistSkills(vec![
            ScientistSkillRow {
                character: 7,
                token: 17,
                level: 1,
                ..Default::default()
            },
            ScientistSkillRow {
                character: 8,
                token: 17,
                level: 1,
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::RaidFacilities.index()] =
        TableView::Complete(TableRows::RaidFacilities(vec![
            RaidFacilityRow {
                country: 100,
                owner: 100,
                ordinal: 1,
                id: 12,
                province: 1,
                ..Default::default()
            },
            RaidFacilityRow {
                country: 100,
                owner: 100,
                ordinal: 0,
                id: 11,
                province: 2,
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::ProjectPrograms.index()] =
        TableView::Complete(TableRows::ProjectPrograms(vec![
            ProjectProgramRow {
                id: 12,
                owner: 100,
                province: 1,
                building: b"mod_facility".to_vec(),
                ..Default::default()
            },
            ProjectProgramRow {
                id: 11,
                owner: 100,
                province: 2,
                building: b"mod_facility".to_vec(),
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 100,
            entries: Vec::new(),
            seed: Recorded::Value(None),
        }]));
    (fixture, folder)
}

fn render(fixture: &Fixture, body: &str) -> Description {
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    fixture.render_native(body, &mut fresh(), content.view(None))
}

#[test]
fn template_output_header_unlocks_limits_and_module_country_gate() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture(
        "enable_equipment_modules={mod_forbidden_module mod_module} enable_subunits={mod_unit limit={always=yes}} enable_equipments={mod_equipment} enable_subunits={mod_unit limit={always=no}}",
    );
    let output = render(&fixture, "complete_special_project=sp:mod_project");
    assert_eq!(
        output.text,
        "Complete Project AAA\n  Output\n  Subunits:\n  Alpha unit\n  Equipment:\n  Alpha equipment\n  Modules:\n  Alpha module\n"
    );
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn subunit_unlock_duplicate_keeps_its_first_limit_before_admission() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture(
        "enable_subunits={limit={always=no} mod_unit} enable_subunits={limit={always=yes} mod_unit}",
    );
    let output = render(&fixture, "complete_special_project=sp:mod_project");
    assert_eq!(output.text, "Complete Project AAA\n  Output\n");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn country_effects_use_fresh_owner_root_and_prev() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture("country_effects={custom_effect_tooltip=PROJECT_COUNTRY}");
    let output = render(
        &fixture,
        "complete_special_project={project=sp:mod_project show_modifiers=yes iteration_output={mod_reward=mod_option}}",
    );
    assert_eq!(
        output.text,
        "Complete Project AAA\n  Output\n  Country AAA/AAA\n\n"
    );
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn country_output_from_link_preserves_the_project_carrier() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture("country_effects={FROM={add_project_progress_ratio=0.37}}");
    let output = render(&fixture, "complete_special_project=sp:mod_project");
    assert_eq!(
        output.text,
        "Complete Project AAA\n  Output\n  Project context Project AAA Progress Project AAA 0.37\n\n"
    );
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn scientist_fallback_uses_roster_order_and_fresh_character_root() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture("scientist_effects={custom_effect_tooltip=PROJECT_SCIENTIST}");
    let output = render(&fixture, "complete_special_project=sp:mod_project");
    assert_eq!(
        output.text,
        "Complete Project AAA\n  Output\n  Scientist First\n\n"
    );
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
    let output = render(
        &fixture,
        "complete_special_project={project=sp:mod_project scientist=mod_first scientist=mod_later}",
    );
    assert_eq!(
        output.text,
        "Complete Project AAA\n  Output\n  Scientist Later\n\n"
    );
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn effects_keep_country_scientist_facility_order() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture(
        "country_effects={custom_effect_tooltip=PROJECT_COUNTRY} scientist_effects={custom_effect_tooltip=PROJECT_SCIENTIST} facility_state_effects={custom_effect_tooltip=PROJECT_FACILITY}",
    );
    let output = render(&fixture, "complete_special_project=sp:mod_project");
    assert_eq!(
        output.text,
        "Complete Project AAA\n  Output\n  Country AAA/AAA\n\n  Scientist First\n\n  Facility Two/2\n\n"
    );
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn unrostered_scientist_refusal_preserves_country_output() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture(
        "country_effects={custom_effect_tooltip=PROJECT_COUNTRY} scientist_effects={custom_effect_tooltip=PROJECT_SCIENTIST}",
    );
    *fixture.frame.tables[TableId::Scientists.index()] =
        TableView::Complete(TableRows::Scientists(vec![]));
    *fixture.frame.tables[TableId::CountryCharacters.index()] =
        TableView::Complete(TableRows::CountryCharacters(vec![]));
    *fixture.frame.tables[TableId::CountryRetiredCharacters.index()] =
        TableView::Complete(TableRows::CountryRetiredCharacters(vec![
            CountryCharacterRow {
                country: 100,
                ordinal: 0,
                character: Some(7),
            },
        ]));
    let output = render(
        &fixture,
        "complete_special_project={project=sp:mod_project scientist=mod_later}",
    );
    assert_eq!(
        output.text,
        "Complete Project AAA\n  Output\n  Country AAA/AAA\n\n"
    );
    assert_eq!(output.refusals, BTreeMap::from([("synthetic effect project completion: key complete_special_project: unrostered character 7 scientist holder is unrecorded".into(),1)]));
}

#[test]
fn facility_selector_precedes_specialization_and_injects_stored_province_id() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) =
        fixture("facility_state_effects={custom_effect_tooltip=PROJECT_FACILITY}");
    let output = render(
        &fixture,
        "complete_special_project={project=sp:mod_project state=2 state=1}",
    );
    assert_eq!(
        output.text,
        "Complete Project AAA\n  Output\n  Facility One/1\n\n"
    );
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
    let output = render(&fixture, "complete_special_project=sp:mod_project");
    assert_eq!(
        output.text,
        "Complete Project AAA\n  Output\n  Facility Two/2\n\n"
    );
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn project_output_reuses_equipment_and_subunit_bonus_formatters() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture(
        "equipment_bonus={mod_equipment={soft_attack=0.2}} sub_unit_bonus={mod_unit={soft_attack=0.1}} enable_equipments={mod_equipment}",
    );
    let output = render(&fixture, "complete_special_project=sp:mod_project");
    assert_eq!(
        output.text,
        "Complete Project AAA\n  Output\n  \u{13}GFX_mod_unit_icon_small|1 Alpha unit\n     Soft:0.10\n  \u{11}YEquipment\u{11}!: \n      Soft:0.20\n  Equipment:\n  Alpha equipment\n"
    );
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn gap_research_grants_pending_output_uses_exact_selected_assignment_and_never_falls_back() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation");
    let (mut fixture, _folder) = fixture(
        "country_effects={custom_effect_tooltip=PROJECT_VARIABLE} scientist_effects={custom_effect_tooltip=PROJECT_SCIENTIST}",
    );
    if let TableView::Complete(TableRows::Projects(rows)) =
        &mut *fixture.frame.tables[TableId::Projects.index()]
    {
        rows[0].runtime = Recorded::Value(ProjectRuntime {
            variables: Some(
                ProjectVariableStore {
                    seed: 1,
                    entries: vec![ProjectVariable {
                        name: b"mod_value".to_vec(),
                        value_raw: 125_000,
                    }],
                }
                .into(),
            ),
            ..Default::default()
        });
    }
    let scripted = fixture.catalogue.scripted_content().unwrap();
    let cache = scripted.effects().unwrap();
    let text = crate::scripted_text::ScriptedText::new(
        &fixture.frame,
        &fixture.catalogue.localisation,
        scripted,
        1,
    );
    let context = Context::new(
        &text,
        &fixture.catalogue.localisation,
        1,
        "pending output",
        cache.view(None),
    );
    let definitions =
        content_reader_military::special_projects::Catalog::read(&fixture.catalogue.content.layers).unwrap();
    let output = &definitions.projects["mod_project"].output;
    let absent = project_output_description(output, &context, 100, 8, Ok(None), Ok(None));
    assert!(!absent.text.contains("Scientist"));
    let selected = project_output_description(output, &context, 100, 8, Ok(Some(7)), Ok(None));
    assert!(selected.text.contains("Variable 1.25"), "{:?}", selected);
    assert!(selected.refusals.is_empty(), "{:?}", selected.refusals);
    assert!(selected.text.contains("Scientist Later"), "{:?}", selected);
    assert!(!selected.text.contains("Scientist First"));
}
