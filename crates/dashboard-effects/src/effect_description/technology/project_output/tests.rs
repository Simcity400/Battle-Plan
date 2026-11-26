use super::*;
use crate::fixtures::{Folder, folder_catalogue};
use crate::effect_description::tests::{Fixture, fresh};
use replay_data::rows::*;
use replay_frame::frame::TableView;
use crate::fixtures::game;

fn fixture(output: &str) -> (Fixture, Folder) {
    let mut fixture = Fixture::new();
    let project = format!("mod_project={{specialization=mod_science project_output={{{output}}}}}");
    let (catalogue, folder) = folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}},NSupply={MAX_RAILWAY_LEVEL=7}}",
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
        ("common/technology_tags/test.txt", "technology_categories={mod_armor}"),
        (
            "common/script_constants/test.txt",
            "sp_breakthrough_progress={schema={any_key=yes data=fixed_point} medium=0.25} sp_count={schema={any_key=yes data=int} one=1}",
        ),
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
                " STAT_SYNTHETIC_SOFT_ATTACK:0 \"Soft:\"\n STAT_MODIFIER_DIFF:0 \"$VALUE|2$\"\n",
                " SUBUNIT_BONUS_HEADER:0 \"$ICON$$NAME$\"\n STAT_MODIFIER_VALUE:0 \"$VALUE|2$\"\n",
                " SPECIAL_PROJECT_REWARDS_HEADER_TEXT:0 \"Outputs\"\n SPECIAL_PROJECT_REWARD_EQUIPMENT:0 \"Equipment got\"\n SPECIAL_PROJECT_REWARD_EQUIPMENT_MODULE:0 \"Modules got\"\n",
                " mod_armor:0 \"Armor technology\"\n AND:0 \"and \"\n EFFECT_TECH_BONUS_USES:0 \"$USES$x \"\n EFFECT_ADD_TECH_BONUS_BONUS_ONLY:0 \"$USES$$BONUS|%0+$ Research bonus for: $LIST|Y$.\"\n",
                " mod_science:0 \"Naval Engineering\"\n ADD_BREAKTHROUGH_PROGRESS_EFFECT_DESC:0 \"Gain breakthrough progress:\"\n ADD_BREAKTHROUGH_PROGRESS_EFFECT_SPECIALIZATION:0 \"$SPECIALIZATION|Y$: $AMOUNT|2%G$\"\n",
                " EFFECT_ADD_NAVY_EXPERIENCE:0 \"Navy Experience: $VAL|=+0$.\"\n SCIENTIST_ADD_XP_SPECIALIZATION_EFFECT:0 \"gains $XP|0Y$ experience in $SPECIALIZATION|Y$\"\n"
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
    game::enter();
    let (fixture, _folder) = fixture(
        "enable_equipment_modules={mod_forbidden_module mod_module} enable_subunits={mod_unit limit={always=yes}} enable_equipments={mod_equipment} enable_subunits={mod_unit limit={always=no}}",
    );
    let output = render(&fixture, "fixture_finish_project=sp:mod_project");
    assert_eq!(
        output.text,
        "Complete Project AAA\n  Output\n  Subunits:\n  Alpha unit\n  Equipment:\n  Alpha equipment\n  Modules:\n  Alpha module\n"
    );
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn subunit_unlock_duplicate_keeps_its_first_limit_before_admission() {
    game::enter();
    let (fixture, _folder) = fixture(
        "enable_subunits={limit={always=no} mod_unit} enable_subunits={limit={always=yes} mod_unit}",
    );
    let output = render(&fixture, "fixture_finish_project=sp:mod_project");
    assert_eq!(output.text, "Complete Project AAA\n  Output\n");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn country_effects_use_fresh_owner_root_and_prev() {
    game::enter();
    let (fixture, _folder) = fixture("country_effects={custom_effect_tooltip=PROJECT_COUNTRY}");
    let output = render(
        &fixture,
        "fixture_finish_project={project=sp:mod_project show_modifiers=yes iteration_output={mod_reward=mod_option}}",
    );
    assert_eq!(
        output.text,
        "Complete Project AAA\n  Output\n  Country AAA/AAA\n\n"
    );
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn country_output_from_link_preserves_the_project_carrier() {
    game::enter();
    let (fixture, _folder) = fixture("country_effects={FROM={add_project_progress_ratio=0.37}}");
    let output = render(&fixture, "fixture_finish_project=sp:mod_project");
    assert_eq!(
        output.text,
        "Complete Project AAA\n  Output\n  Project context Project AAA Progress Project AAA 0.37\n\n"
    );
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn scientist_fallback_uses_roster_order_and_fresh_character_root() {
    game::enter();
    let (fixture, _folder) = fixture("scientist_effects={custom_effect_tooltip=PROJECT_SCIENTIST}");
    let output = render(&fixture, "fixture_finish_project=sp:mod_project");
    assert_eq!(
        output.text,
        "Complete Project AAA\n  Output\n  Scientist First\n\n"
    );
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
    let output = render(
        &fixture,
        "fixture_finish_project={project=sp:mod_project scientist=mod_first scientist=mod_later}",
    );
    assert_eq!(
        output.text,
        "Complete Project AAA\n  Output\n  Scientist Later\n\n"
    );
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn effects_keep_country_scientist_facility_order() {
    game::enter();
    let (fixture, _folder) = fixture(
        "country_effects={custom_effect_tooltip=PROJECT_COUNTRY} scientist_effects={custom_effect_tooltip=PROJECT_SCIENTIST} facility_state_effects={custom_effect_tooltip=PROJECT_FACILITY}",
    );
    let output = render(&fixture, "fixture_finish_project=sp:mod_project");
    assert_eq!(
        output.text,
        "Complete Project AAA\n  Output\n  Country AAA/AAA\n\n  Scientist First\n\n  Facility Two/2\n\n"
    );
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn unrostered_scientist_refusal_preserves_country_output() {
    game::enter();
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
        "fixture_finish_project={project=sp:mod_project scientist=mod_later}",
    );
    assert_eq!(
        output.text,
        "Complete Project AAA\n  Output\n  Country AAA/AAA\n\n"
    );
    assert_eq!(output.refusals, BTreeMap::from([("synthetic effect project completion: key complete_special_project: unrostered character 7 scientist holder is unrecorded".into(),1)]));
}

#[test]
fn facility_selector_precedes_specialization_and_injects_stored_province_id() {
    game::enter();
    let (fixture, _folder) =
        fixture("facility_state_effects={custom_effect_tooltip=PROJECT_FACILITY}");
    let output = render(
        &fixture,
        "fixture_finish_project={project=sp:mod_project state=2 state=1}",
    );
    assert_eq!(
        output.text,
        "Complete Project AAA\n  Output\n  Facility One/1\n\n"
    );
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
    let output = render(&fixture, "fixture_finish_project=sp:mod_project");
    assert_eq!(
        output.text,
        "Complete Project AAA\n  Output\n  Facility Two/2\n\n"
    );
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn project_output_reuses_equipment_and_subunit_bonus_formatters() {
    game::enter();
    let (fixture, _folder) = fixture(
        "equipment_bonus={mod_equipment={soft_attack=0.2}} sub_unit_bonus={mod_unit={soft_attack=0.1}} enable_equipments={mod_equipment}",
    );
    let output = render(&fixture, "fixture_finish_project=sp:mod_project");
    assert_eq!(
        output.text,
        "Complete Project AAA\n  Output\n  \u{13}GFX_mod_unit_icon_small|1 Alpha unit\n     Soft:0.10\n  \u{11}YEquipment\u{11}!: \n      Soft:0.20\n  Equipment:\n  Alpha equipment\n"
    );
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn gap_research_grants_pending_output_uses_exact_selected_assignment_and_never_falls_back() {
    game::enter();
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

#[test]
fn iteration_output_follows_the_history_view_and_links_its_unlocks() {
    game::enter();
    let (fixture, _folder) = fixture(
        "country_effects={custom_effect_tooltip=PROJECT_COUNTRY} scientist_effects={custom_effect_tooltip=PROJECT_SCIENTIST} enable_equipments={mod_equipment} enable_equipment_modules={mod_module}",
    );
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
        "special-project iteration",
        cache.view(None),
    )
    .with_entity_links();
    let definitions =
        content_reader_military::special_projects::Catalog::read(&fixture.catalogue.content.layers).unwrap();
    let output = &definitions.projects["mod_project"].output;
    let scopes = |scientist| IterationScopes {
        country_seed: Some(5),
        scientist,
        scientist_seed: None,
        facility: Ok(None),
        state_seed: None,
    };
    let section = |heading: Option<&str>, text: &str| OutputSection {
        heading: heading.map(Into::into),
        text: text.into(),
    };
    let described = project_iteration_description(output, &context, 100, 8, scopes(Ok(Some(7))));
    assert_eq!(described.heading.as_deref(), Some("Outputs"));
    assert_eq!(
        described.sections,
        [
            section(None, "Country AAA/AAA"),
            section(None, "Scientist Later"),
            section(Some("Modules got"), "\u{1}mmod_module\u{2}Alpha module\u{3}"),
            section(Some("Equipment got"), "\u{1}emod_equipment\u{2}Alpha equipment\u{3}"),
        ],
        "country, scientist and state effects carry no heading; unlocks do, modules before equipment"
    );
    assert!(described.refusals.is_empty(), "{:?}", described.refusals);
    let refused = project_iteration_description(
        output,
        &context,
        100,
        8,
        scopes(Err("Unknown scientist reference type".into())),
    );
    assert_eq!(refused.sections.len(), 3, "a refused scientist keeps the rest: {refused:?}");
    assert_eq!(
        refused.refusals,
        BTreeMap::from([(
            "special-project iteration effect scientist effects: Unknown scientist reference type".into(),
            1
        )])
    );
    let empty = content_reader_military::special_projects::Output::default();
    let nothing = project_iteration_description(&empty, &context, 100, 8, scopes(Ok(None)));
    assert_eq!(
        (nothing.heading, nothing.sections.len(), nothing.no_changes),
        (None, 0, true),
        "no output, no heading: the option changes nothing"
    );
}

fn iteration(fixture: &Fixture) -> IterationDescription {
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
        "special-project iteration",
        cache.view(None),
    )
    .with_entity_links();
    let definitions =
        content_reader_military::special_projects::Catalog::read(&fixture.catalogue.content.layers).unwrap();
    project_iteration_description(
        &definitions.projects["mod_project"].output,
        &context,
        100,
        8,
        IterationScopes {
            country_seed: Some(5),
            scientist: Ok(Some(7)),
            scientist_seed: None,
            facility: Ok(None),
            state_seed: None,
        },
    )
}

#[test]
fn iteration_scientist_rewards_gated_on_the_scientists_level_render() {
    game::enter();
    for (gate, shown) in [("level < 5", true), ("level > 1", false)] {
        let (mut rewards, _folder) = fixture(&format!(
            "scientist_effects={{if={{limit={{has_scientist_level={{{gate} specialization=mod_science}}}} add_scientist_xp={{experience=15 specialization=mod_science}}}}}}"
        ));
        if let TableView::Complete(TableRows::Scientists(rows)) =
            &mut *rewards.frame.tables[TableId::Scientists.index()]
        {
            for row in rows {
                row.skill_count = 1;
            }
        }
        let described = iteration(&rewards);
        assert!(described.refusals.is_empty(), "{gate}: {:?}", described.refusals);
        assert_eq!(
            described.sections.iter().any(|section| section.text.contains("Naval Engineering experience")),
            shown,
            "{gate}: {described:?}"
        );
    }
}

#[test]
fn iteration_rewards_read_concisely_by_effect_type_in_the_games_value_format() {
    game::enter();
    let (rewards, _folder) = fixture(concat!(
        "country_effects={add_tech_bonus={name=mod_bonus bonus=0.2 uses=1 category=mod_armor} FROM={add_project_progress_ratio=0.15} ",
        "add_breakthrough_progress={specialization=mod_science value=sp_breakthrough_progress.medium} navy_experience=15 custom_effect_tooltip=PROJECT_COUNTRY} ",
        "scientist_effects={add_scientist_xp={experience=15 specialization=mod_science}}",
    ));
    let described = iteration(&rewards);
    assert!(!described.no_changes);
    assert_eq!(
        described.sections,
        [
            OutputSection {
                heading: None,
                text: concat!(
                    "Research bonus \u{11}G+20%\u{11}! for Armor technology (1 use)\n",
                    "Project progress +0.15\n",
                    "Naval Engineering breakthrough \u{11}G+25.00%\u{11}!\n",
                    "Navy experience \u{11}G+15\u{11}!\n",
                    "Country AAA/AAA",
                )
                .into(),
            },
            OutputSection {
                heading: None,
                text: "Later: Naval Engineering experience \u{11}Y+15\u{11}!".into(),
            },
        ],
        "covered effect types read concisely with the game's value format and names; the project's own header is dropped; a custom tooltip keeps the game text"
    );
    assert!(described.refusals.is_empty(), "{:?}", described.refusals);
    let output = render(
        &rewards,
        "fixture_finish_project=sp:mod_project add_breakthrough_progress={specialization=mod_science value=0.25}",
    );
    for game in [
        "1x \u{11}G20%\u{11}! Research bonus for: \u{11}YArmor technology\u{11}!.",
        "Project context Project AAA Progress Project AAA 0.15",
        "Navy Experience: \u{11}G+15\u{11}!.",
        "Gain breakthrough progress:\n  \u{11}YNaval Engineering\u{11}!: \u{11}G25.00%\u{11}!",
    ] {
        assert!(output.text.contains(game), "outside iterations the game text stays: {game:?} in {output:?}");
    }

    let (unread, _folder) = fixture(
        "country_effects={add_breakthrough_progress={specialization=mod_science value=sp_count.one}}",
    );
    let refused = iteration(&unread);
    assert!(refused.sections.is_empty(), "{refused:?}");
    assert_eq!(
        refused.refusals.keys().collect::<Vec<_>>(),
        ["special-project iteration effect add_breakthrough_progress: key add_breakthrough_progress: breakthrough progress sp_count.one is not a fixed-point script constant"],
        "an integer constant is not the fixed-point value the game reads, so it is withheld by name"
    );

    let (shown, _folder) = fixture(
        "country_effects={custom_effect_tooltip=PROJECT_COUNTRY if={limit={always=yes} effect_tooltip={navy_experience=15}}} scientist_effects={}",
    );
    let unchanged = iteration(&shown);
    assert_eq!(
        (unchanged.no_changes, unchanged.heading, unchanged.sections.len()),
        (true, None, 0),
        "an option whose effects only show tooltips changes nothing"
    );
    let (acting, _folder) = fixture("country_effects={custom_effect_tooltip=PROJECT_COUNTRY navy_experience=15}");
    let changed = iteration(&acting);
    assert!(!changed.no_changes);
    assert_eq!(changed.sections[0].text, "Country AAA/AAA\nNavy experience \u{11}G+15\u{11}!");
    let (selected, _folder) = fixture("country_effects={navy_experience=ROOT}");
    let selected = iteration(&selected);
    assert!(selected.refusals.is_empty(), "{:?}", selected.refusals);
    assert!(
        selected.sections[0].text.starts_with("Navy experience "),
        "a scope-selected amount reads concisely with the zero the game's own text shows: {selected:?}"
    );
}
