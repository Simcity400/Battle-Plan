use super::*;
use crate::fixtures::{Folder, folder_catalogue};
use crate::effect_description::tests::{Fixture, fresh};
use crate::scripted_text::{EventObject, EventScope, EventScopes, Link};
use crate::triggers::ContentFacts;
use replay_data::rows::*;
use replay_frame::frame::TableView;

fn fixture() -> (Fixture, Folder) {
    let mut fixture = Fixture::new();
    let (catalogue, folder) = folder_catalogue(&[
        ("common/country_tags/synthetic.txt", "AAA=a BBB=b CCC=c"),
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}},NSupply={MAX_RAILWAY_LEVEL=5},NProject={DEFAULT_COMPLEXITY=3}}",
        ),
        (
            "common/technologies/synthetic.txt",
            concat!(
                "technology_folders={ignored_folder={}} technology_categories={mod_category mod_other} technologies={",
                "mod_tech={parent=mod_parent enable_equipments={mod_equipment}} mod_small={parent=mod_parent force_use_small_tech_layout=yes enable_equipments={mod_equipment}} ",
                "mod_callback={mod_unit={soft_attack=0.17} enable_equipments={missing_equipment mod_equipment} enable_equipment_modules={mod_module mod_module mod_second_module mod_denied_module} max_level=5 enable_building={building=mod_lab} enable_subunits={limit={always=yes} mod_unit} enable_tactic=mod_tactic enable_tactic=MOD_TACTIC on_research_complete_limit={always=yes} on_research_complete={limit={} custom_effect_tooltip=TECH_COMPLETE} modifier={custom_modifier_tooltip=TECH_MODIFIER}} ",
                "mod_conditional={on_research_complete_limit={always=no} on_research_complete={limit={} custom_effect_tooltip=TECH_COMPLETE} enable_subunits={limit={always=no} mod_unit} enable_subunits={limit={always=yes} mod_unit}} ",
                "mod_repeated={enable_building={building=mod_lab} enable_building={building=mod_lab level=2} enable_building={building=mod_store level=3} enable_building={building=MOD_LAB level=4} enable_tactic=mod_tactic enable_tactic=mod_tactic enable_tactic=mod_other_tactic enable_tactic=mod_tactic} ",
                "MOD_TECH={parent=mod_other}}"
            ),
        ),
        ("common/units/synthetic.txt", "sub_units={mod_unit={}}"),
        (
            "common/combat_tactics.txt",
            "mod_tactic={} mod_other_tactic={}",
        ),
        (
            "common/units/equipment/synthetic.txt",
            "equipments={mod_equipment={is_archetype=yes type=infantry}} equipment_modules={limit={always=yes} mod_module={allowed={tag=AAA}} mod_second_module={allowed={always=yes}}} equipment_modules={limit={always=no} mod_denied_module={allowed={always=yes}}}",
        ),
        (
            "common/doctrines/folders/synthetic.txt",
            "mod_folder={name=mod_folder_name}",
        ),
        (
            "common/doctrines/tracks/synthetic.txt",
            "mod_track={name=mod_track_name}",
        ),
        (
            "common/doctrines/grand_doctrines/synthetic.txt",
            "mod_grand={name=mod_grand_name}",
        ),
        (
            "common/doctrines/subdoctrines/land/synthetic.txt",
            "mod_sub={name=mod_sub_name}",
        ),
        (
            "common/special_projects/specialization/synthetic.txt",
            "mod_science={} mod_other_science={}",
        ),
        (
            "common/special_projects/projects/synthetic.txt",
            "mod_project={specialization=mod_science prototype_time=160 complexity={min=3 max=5}} mod_constant_project={specialization=mod_science prototype_time=mod_time.long complexity=mod_complexity.big}",
        ),
        (
            "common/buildings/synthetic.txt",
            "buildings={mod_lab={specialization={mod_science} level_cap={province_max=1}} mod_store={level_cap={province_max=1}}}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n CCC:0 \"Gamma\"\n",
                " EFFECT_ADD_RESEARCH_SLOT:0 \"Add $COUNT|0$ [Root.GetTag]\"\n EFFECT_REMOVE_RESEARCH_SLOT:0 \"Remove $COUNT|0$\"\n EFFECT_SET_RESEARCH_SLOTS:0 \"Slots $COUNT|0$\"\n",
                " EFFECT_INHERIT_TECHNOLOGY:0 \"Inherit $COUNTRY$\"\n",
                " EFFECT_MIO_ADD_RESEARCH_BONUS:0 \"MIO add $VAL|2$ [Root.GetTag]\"\n EFFECT_MIO_SET_RESEARCH_BONUS:0 \"MIO set $VAL|2$ [Root.GetTag]\"\n",
                " EFFECT_STEAL_RANDOM_TECH:0 \"Steal [Root.GetTag]\"\n EFFECT_STEAL_RANDOM_TECH_INSTANT:0 \"Steal instantly [Root.GetTag]\"\n",
                " FACTION_ACCESS_TAB_REASEARCH:0 \"Research [Root.GetTag]\"\n FACTION_ACCESS_TAB_UNLOCK:0 \"Unlock $NAME$\"\n FACTION_ACCESS_TAB_LOCK:0 \"Lock $NAME$\"\n",
                " mod_category:0 \"Category\"\n AAA_mod_category:0 \"Alpha category [Root.GetTag]\"\n mod_other:0 \"Other\"\n mod_parent:0 \"Parent\"\n AAA_mod_parent:0 \"Alpha parent\"\n mod_equipment:0 \"Equipment\"\n AAA_mod_equipment:0 \"Alpha equipment\"\n",
                " EFFECT_GIVE_TECHNOLOGY:0 \"Give $NAME$\"\n AND:0 \"plus \"\n EFFECT_TECH_BONUS_USES:0 \"$USES|0$ uses\"\n",
                " MOD_UNLOCK_TECH:0 \"Unlocks $TECH|H$.\"\n MOD_UNLOCK_LOWER:0 \"Unlocks $TECH|l$.\"\n MOD_UNLOCK_BOTH:0 \"Unlocks $TECH|H$; $OTHER$\"\n mod_tech:0 \"Mod technology\"\n",
                " EFFECT_ADD_TECH_BONUS_BONUS_ONLY:0 \"Bonus $BONUS|2$/$USES$:$LIST$\"\n EFFECT_ADD_TECH_BONUS_AHEAD_ONLY:0 \"Ahead $AHEAD|2$/$USES$:$LIST$\"\n EFFECT_ADD_TECH_BONUS:0 \"Both $AHEAD|2$/$BONUS|2$/$USES|0$:$LIST$\"\n",
                " mod_folder_name:0 \"Folder\"\n mod_track_name:0 \"Track\"\n mod_grand_name:0 \"Grand\"\n mod_sub_name:0 \"Sub\"\n",
                " SET_SUB_DOCTRINE_EFFECT:0 \"Set sub $NAME$\"\n SET_GRAND_DOCTRINE_EFFECT:0 \"Set grand $NAME$\"\n ADD_DAILY_MASTERY_EFFECT:0 \"Daily $AMOUNT|2$/$DAYS|0$\"\n ADD_MASTERY_BONUS_EFFECT:0 \"Mastery bonus $BONUS|2$/$DAYS|0$\"\n",
                " TRACK_EFFECT_ALL_TRACKS:0 \"$TRACK_EFFECT_DESC$ all\"\n TRACK_EFFECT_ONLY_FOLDER:0 \"$TRACK_EFFECT_DESC$ folder $NAME$\"\n TRACK_EFFECT_ONLY_GRAND_DOCTRINE:0 \"$TRACK_EFFECT_DESC$ grand $NAME$\"\n TRACK_EFFECT_ONLY_SUBDOCTRINE:0 \"$TRACK_EFFECT_DESC$ sub $NAME$\"\n TRACK_EFFECT_ONLY_TRACK:0 \"$TRACK_EFFECT_DESC$ track $NAME$\"\n",
                " TRACK_EFFECT_COMPLEX_HEADER:0 \"$TRACK_EFFECT_DESC$ conditions:\"\n TRACK_EFFECT_CONDITION_FOLDER:0 \"Folder $NAME$\"\n TRACK_EFFECT_CONDITION_GRAND_DOCTRINE:0 \"Grand $NAME$\"\n TRACK_EFFECT_CONDITION_SUBDOCTRINE:0 \"Sub $NAME$\"\n TRACK_EFFECT_CONDITION_TRACK_TYPE:0 \"Track $NAME$\"\n TRACK_EFFECT_CONDITION_TRACK_INDEX:0 \"Index $INDEX|0$\"\n",
                " mod_science:0 \"Science\"\n AAA_mod_science:0 \"Alpha science [Root.GetTag]\"\n mod_other_science:0 \"Other science\"\n",
                " ADD_BREAKTHROUGH_POINTS_EFFECT_DESC:0 \"Points:\"\n ADD_BREAKTHROUGH_PROGRESS_EFFECT_DESC:0 \"Progress:\"\n ADD_BREAKTHROUGH_POINTS_EFFECT_SPECIALIZATION:0 \"$SPECIALIZATION$:$AMOUNT|0$\"\n ADD_BREAKTHROUGH_PROGRESS_EFFECT_SPECIALIZATION:0 \"$SPECIALIZATION$:$AMOUNT|2$\"\n",
                " ADD_FACTION_POWER_PROJECTION_EFFECT_NO_FACTION:0 \"Projection $VAL|0$ [Root.GetTag]\"\n",
                " ADD_FACTION_POWER_PROJECTION_EFFECT:0 \"Projection $VAL|0$/$FACTION$ [Root.GetTag]\"\n",
                " mod_unit:0 \"Unit\"\n AAA_mod_unit:0 \"Alpha unit\"\n mod_module:0 \"Module\"\n AAA_mod_module:0 \"Alpha module\"\n mod_tactic:0 \"Tactic\"\n AAA_mod_tactic:0 \"Alpha tactic\"\n mod_lab:0 \"Lab\"\n",
                " ZERO_mod_module:0 \"Zero module\"\n mod_second_module:0 \"Second module\"\n ZERO_mod_second_module:0 \"Zero second\"\n mod_denied_module:0 \"Denied module\"\n",
                " mod_store:0 \"Store\"\n mod_other_tactic:0 \"Other tactic\"\n AAA_mod_other_tactic:0 \"Alpha other tactic\"\n",
                " SUBUNIT_BONUS_HEADER:0 \"$ICON$$NAME$\"\n STAT_ARMY_SOFT_ATTACK:0 \"Soft:\"\n STAT_MODIFIER_VALUE:0 \"$VALUE|2$\"\n BUILDING_LEVEL:0 \"Level $LEVEL|0$\"\n",
                " RESEARCH_UNLOCKS_EQUIPMENT_HEADER:0 \"Equipment:$EQUIPMENTS$\"\n RESEARCH_UNLOCKS_EQUIPMENT_MODULES_HEADER:0 \"Modules:$MODULES$\"\n RESEARCH_UNLOCKS_BUILDINGS_HEADER:0 \"Buildings:$BUILDINGS$\"\n RESEARCH_UNLOCKS_SUBUNITS_HEADER:0 \"Subunits:$SUBUNITS$\"\n RESEARCH_UNLOCKS_TACTICS_HEADER:0 \"Tactics:$TACTICS$\"\n",
                " TECH_COMPLETE:0 \"Complete [Root.GetTag]/[Prev.GetTag]\"\n TECH_MODIFIER:0 \"Modifier [Root.GetTag]\"\n",
                " mod_project:0 \"Project [Root.GetTag]\"\n AAA_mod_project:0 \"Alpha project [Root.GetTag]\"\n mod_constant_project:0 \"Project [Root.GetTag]\"\n",
                " EFFECT_ADD_SPECIAL_PROJECT_PROGRESS:0 \"Progress $PROJECT$ $PROGRESS|2$\"\n EFFECT_COMPLETE_SPECIAL_PROJECT:0 \"Complete $PROJECT$\"\n SPECIAL_PROJECT_RESEARCH_OUTPUT:0 \"Output\"\n",
                " EFFECT_REDUCE_SPECIAL_PROJECT_PROGRESS:0 \"Delay $PROJECT$ $PROGRESS|2$/$DAYS|0$ days\"\n"
            ),
        ),
    ]);
    catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    fixture.catalogue = catalogue;
    *fixture.frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(Vec::new()));
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 17,
                literal: b"mod_science".to_vec(),
            },
            LexicalTokenRow {
                token: 22,
                literal: b"mod_other_science".to_vec(),
            },
            LexicalTokenRow {
                token: 33,
                literal: b"mod_project".to_vec(),
            },
            LexicalTokenRow {
                token: 44,
                literal: b"mod_lab".to_vec(),
            },
        ]));
    *fixture.frame.tables[TableId::Projects.index()] =
        TableView::Complete(TableRows::Projects(vec![
            ProjectRow {
                owner: 100,
                local_id: 7,
                definition_id: 33,
                ..Default::default()
            },
            ProjectRow {
                owner: 200,
                local_id: 8,
                definition_id: 33,
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::TechnologyDefinitions.index()] =
        TableView::Complete(TableRows::TechnologyDefinitions(vec![
            TechnologyDefinitionRow {
                technology_handle: 42,
                database_index: 1,
                id: b"mod_tech".to_vec(),
                ..Default::default()
            },
            TechnologyDefinitionRow {
                technology_handle: 43,
                database_index: 6,
                id: b"MOD_TECH".to_vec(),
                ..Default::default()
            },
            TechnologyDefinitionRow {
                technology_handle: 53,
                database_index: 42,
                id: b"mod_callback".to_vec(),
                ..Default::default()
            },
            TechnologyDefinitionRow {
                technology_handle: 54,
                database_index: 53,
                id: b"mod_conditional".to_vec(),
                ..Default::default()
            },
            TechnologyDefinitionRow {
                technology_handle: 55,
                database_index: 54,
                id: b"mod_repeated".to_vec(),
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::ResearchTechnologies.index()] =
        TableView::Complete(TableRows::ResearchTechnologies(vec![
            ResearchTechnologyRow {
                database_index: 42,
                country: 100,
                technology_handle: 53,
                ..Default::default()
            },
            ResearchTechnologyRow {
                database_index: 53,
                country: 100,
                technology_handle: 54,
                ..Default::default()
            },
            ResearchTechnologyRow {
                database_index: 42,
                country: 200,
                technology_handle: 53,
                ..Default::default()
            },
            ResearchTechnologyRow {
                database_index: 54,
                country: 100,
                technology_handle: 55,
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"ZERO".to_vec(),
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 100,
            entries: vec![
                ScriptVariableEntry {
                    name: b"mod_bonus".to_vec(),
                    value_raw: 25_000,
                },
                ScriptVariableEntry {
                    name: b"mod_handle".to_vec(),
                    value_raw: 4_200_000,
                },
                ScriptVariableEntry {
                    name: b"mod_second_handle".to_vec(),
                    value_raw: 4_300_000,
                },
            ],
            ..Default::default()
        }]));
    (fixture, folder)
}

#[test]
fn technology_description_cases() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    for (case, body, expected) in [
        (
            "add_tech_bonus",
            "add_tech_bonus={category=mod_category uses=3 bonus=0.42 name=mod_name}",
            "Bonus 0.42/3 uses:Alpha category [Root.GetTag]\n",
        ),
        (
            "set_technology",
            "set_technology={mod_tech=1 popup=no}",
            "Give Alpha parent\n",
        ),
        (
            "add_research_slot",
            "add_research_slot=4",
            "Add 4 [Root.GetTag]\n",
        ),
        (
            "add_breakthrough_progress",
            "add_breakthrough_progress={value=0.28 specialization=mod_science}",
            "Progress:\n  Alpha science [Root.GetTag]:0.28\n",
        ),
        (
            "add_mastery_bonus",
            "add_mastery_bonus={track=mod_track bonus=0.32 name=mod_bonus days=41}",
            "Mastery bonus 0.32/41 track Track",
        ),
        (
            "inherit_technology",
            "inherit_technology=BBB",
            "Inherit Beta",
        ),
        (
            "add_mio_research_bonus",
            "add_mio_research_bonus=0.14",
            "MIO add 0.14 AAA\n",
        ),
        (
            "steal_random_tech_bonus",
            "steal_random_tech_bonus={instant=yes target=FROM dynamic=yes name=mod_name folder=mod_folder folder=sample_extra_folder bonus=1.8 uses=4}",
            "Steal instantly AAA",
        ),
        (
            "set_sub_doctrine",
            "set_sub_doctrine=mod_sub",
            "Set sub Sub",
        ),
        (
            "add_daily_mastery",
            "add_daily_mastery={days=73 track=mod_track name=mod_name amount=0.65}",
            "Daily 0.65/73 track Track",
        ),
        ("set_research_slots", "set_research_slots=6", "Slots 6"),
        (
            "add_breakthrough_points",
            "add_breakthrough_points={specialization=mod_science value=1}",
            "Points:\n  Alpha science [Root.GetTag]:1\n",
        ),
        (
            "set_grand_doctrine",
            "set_grand_doctrine=mod_grand",
            "Set grand Grand",
        ),
        (
            "add_faction_power_projection",
            "add_faction_power_projection=100",
            "Projection 100 AAA",
        ),
        (
            "set_mio_research_bonus",
            "set_mio_research_bonus=var:mod_bonus",
            "MIO set 0.25 AAA\n",
        ),
        (
            "set_faction_research_unlocked",
            "set_faction_research_unlocked=yes",
            "Unlock Research AAA",
        ),
        (
            "faction_research_quoted_yes_uses_raw_token_text",
            "set_faction_research_unlocked=\"yes\"",
            "Unlock Research AAA",
        ),
        (
            "faction_research_uppercase_yes_stays_locked",
            "set_faction_research_unlocked=YES",
            "Lock Research AAA",
        ),
        (
            "complete_special_project",
            "complete_special_project=sp:mod_project",
            "Complete Alpha project AAA\n  Output\n",
        ),
        (
            "complete_project_hidden_output",
            "complete_special_project={project=sp:mod_project show_modifiers=no}",
            "Complete Alpha project AAA\n",
        ),
        (
            "complete_project_null_selector_keeps_literal",
            "complete_special_project=sp:missing",
            "Complete sp:missing\n",
        ),
        (
            "research_slot_removal_zero",
            "add_research_slot=0",
            "Remove 0\n",
        ),
        (
            "technology_handle_uses_handle_not_database_index",
            "set_technology={var:mod_handle=0}",
            "Give Alpha parent\n",
        ),
        (
            "technology_handle_keeps_its_recorded_template",
            "set_technology={var:mod_second_handle=0}",
            "Give Other\n",
        ),
        (
            "technology_bonus_runtime_equipment_name",
            "add_tech_bonus={technology=mod_tech bonus=0.5 uses=2}",
            "Bonus 0.50/2 uses:Alpha equipment\n",
        ),
        (
            "technology_bonus_both_passes_integer_uses",
            "add_tech_bonus={technology=mod_small category=mod_category category=MOD_CATEGORY ahead_reduction=1 bonus=0.5 uses=2}",
            "Both 1.00/0.50/2:Alpha parent plus Alpha category [Root.GetTag]\n",
        ),
        (
            "mastery_complex_filter_order",
            "add_daily_mastery={amount=1 folder=mod_folder grand_doctrine=mod_grand sub_doctrine=mod_sub track=mod_track track_index=2}",
            "Daily 1.00/0 conditions:\nFolder Folder\nGrand Grand\nSub Sub\nTrack Track\nIndex 2",
        ),
        (
            "breakthrough_all_keeps_token_slot_order",
            "add_breakthrough_points={specialization=all specialization=mod_science value=1}",
            "Points:\n  Other science:1\n  Alpha science [Root.GetTag]:1\n",
        ),
    ] {
        let output = fixture.render_native(body, &mut fresh(), content.view(None));
        assert_eq!(output.text, expected, "{case}: {:?}", output.refusals);
        assert!(output.refusals.is_empty(), "{case}: {:?}", output.refusals);
    }
}

const RAID: u64 = 9 << 32 | 84;

fn raid_fixture() -> (Fixture, Folder) {
    let (mut fixture, folder) = fixture();
    *fixture.frame.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![
            ProvinceRow {
                id: 1,
                state: 1,
                owner: 200,
                ..Default::default()
            },
            ProvinceRow {
                id: 2,
                state: 1,
                owner: 200,
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::Raids.index()] =
        TableView::Complete(TableRows::Raids(vec![RaidRow {
            reference_type: 84,
            id: 9,
            parent_slot: 1,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
            first_country: 100,
            second_country: 200,
            start_hour: 0,
            end_hour: 0,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::ProvinceBuildings.index()] =
        TableView::Complete(TableRows::ProvinceBuildings(vec![
            BuildingRow {
                location: 1,
                building_type: 44,
                ordinal: 0,
                status_flags: 1,
                active: true,
                ..Default::default()
            },
            BuildingRow {
                location: 2,
                building_type: 44,
                ordinal: 0,
                status_flags: 1,
                active: true,
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::RaidFacilities.index()] =
        TableView::Complete(TableRows::RaidFacilities(vec![
            RaidFacilityRow {
                country: 200,
                owner: 200,
                ordinal: 1,
                province: 2,
                active_project: Some(8),
                ..Default::default()
            },
            RaidFacilityRow {
                country: 200,
                owner: 200,
                ordinal: 0,
                province: 1,
                active_project: Some(8),
                ..Default::default()
            },
        ]));
    (fixture, folder)
}

fn render_raid(fixture: &Fixture) -> Description {
    render_raid_with(fixture, &Default::default())
}

fn render_raid_with(fixture: &Fixture, constants: &content_reader_politics::script_constants::Constants) -> Description {
    let scripted = fixture.catalogue.scripted_content().unwrap();
    let content = scripted.effects().unwrap().view(None);
    let text = ScriptedText::new(&fixture.frame, &fixture.catalogue.localisation, scripted, 1);
    let dlcs = Default::default();
    let triggers = Default::default();
    let aliases = Default::default();
    let provinces = BTreeMap::from([(1, vec![2, 1])]);
    let mut facts = ContentFacts::content_only(&dlcs, &triggers, &aliases);
    facts.state_provinces = &provinces;
    facts.state_content = Ok(());
    facts.script_constants = Ok(constants);
    text.set_synthetic_world(World::read(&fixture.frame, facts));
    let context = Context::new(
        &text,
        &fixture.catalogue.localisation,
        1,
        "synthetic",
        content,
    );
    let mut scopes = EventScopes::new(vec![
        EventScope {
            root: Link::Scope(1),
            ..EventScope::of(EventObject::State(1))
        },
        EventScope::of(EventObject::Raid(RAID)),
    ]);
    let items = content_reader_script::effect_keys::read_body(
        b"raid_reduce_project_progress_ratio=0.03",
        content.key_names.unwrap(),
    )
    .unwrap();
    describe(&items, &context, &mut scopes)
}

#[test]
fn raid_reduce_project_progress_ratio() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = raid_fixture();
    let output = render_raid(&fixture);
    assert_eq!(output.text, "Delay Project BBB 0.03/120 days");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn raid_project_progress_reads_scoped_constants_for_both_operands() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = raid_fixture();
    if let TableView::Complete(TableRows::LexicalTokens(rows)) =
        &mut *fixture.frame.tables[TableId::LexicalTokens.index()]
    {
        rows.iter_mut()
            .filter(|row| row.token == 33)
            .for_each(|row| row.literal = b"mod_constant_project".to_vec());
    }
    let constants = content_reader_politics::script_constants::parse(&[(
        "constants",
        b"mod_time={schema={any_key=yes data=int} long=240} mod_complexity={schema={any_key=yes data={{key=min data=int}{key=max data=int}}} big={min=6 max=10}}",
    )]);
    let output = render_raid_with(&fixture, &constants);
    assert_eq!(output.text, "Delay Project BBB 0.03/90 days");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn raid_stops_at_first_enemy_facility_even_when_its_program_is_idle() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = raid_fixture();
    if let TableView::Complete(TableRows::RaidFacilities(rows)) =
        &mut *fixture.frame.tables[TableId::RaidFacilities.index()]
    {
        rows[0].active_project = None;
    }
    let output = render_raid(&fixture);
    assert_eq!(output.text, "");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn raid_skips_friendly_facilities() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = raid_fixture();
    if let TableView::Complete(TableRows::WarRelations(rows)) =
        &mut *fixture.frame.tables[TableId::WarRelations.index()]
    {
        rows.clear();
    }
    let output = render_raid(&fixture);
    assert_eq!(output.text, "");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn technology_catalogue_is_cached_once_per_stack() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, folder) = fixture();
    let scripted = fixture.catalogue.scripted_content().unwrap();
    let content = scripted.effects().unwrap();
    std::fs::write(
        folder.0.join("common/technologies/synthetic.txt"),
        "technologies={mod_tech={parent=wrong}}",
    )
    .unwrap();
    assert!(std::ptr::eq(content, scripted.effects().unwrap()));
    let output = fixture.render_native(
        "set_technology={mod_tech=1}",
        &mut fresh(),
        content.view(None),
    );
    assert_eq!(output.text, "Give Alpha parent\n");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn bound_technology_effect_keeps_body_order_and_fresh_completion_scope() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    let mut scopes = EventScopes::new(vec![
        EventScope {
            root: Link::Scope(1),
            prev: Link::Scope(1),
            ..EventScope::of(EventObject::Country(1))
        },
        EventScope::of(EventObject::Country(2)),
    ]);
    let output = fixture.render_native(
        "custom_effect_tooltip={localization_key=tech_effect|MOD_CALLBACK IGNORED=mod_parent}",
        &mut scopes,
        content.view(None),
    );
    assert_eq!(
        output.text,
        concat!(
            "\u{13}GFX_mod_unit_icon_small|1 Alpha unit\n   Soft:0.17\n",
            "Equipment:\u{11}HAlpha equipment\u{11}!\n",
            "Modules:\u{11}HZero module\u{11}!, \u{11}HZero second\u{11}!\n",
            "Buildings:\u{11}HLab\u{11}! (Level 5)\n",
            "Subunits:\u{11}HAlpha unit\u{11}!\n",
            "Complete AAA/AAA\n",
            "Tactics:\u{11}HAlpha tactic\u{11}!\n",
            "Modifier [Root.GetTag]\n"
        )
    );
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn bound_technology_entity_links_mark_every_unlock_only_when_asked() {
    use crate::effect_description::{EntityKind, Span, spans};
    use content_reader_localisation::localisation::Piece;
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    let render = |links| {
        fixture.with_context_linking(content.view(None), links, |context| {
            technology_description("mod_callback", None, context, 1)
        })
    };
    let (plain, linked) = (render(false), render(true));
    assert!(plain.refusals.is_empty(), "{:?}", plain.refusals);
    assert_eq!(plain.refusals, linked.refusals);
    assert!(!plain.text.contains(['\u{1}', '\u{2}', '\u{3}']), "{:?}", plain.text);
    let mut unmarked = String::new();
    let mut links = Vec::new();
    let mut open = None;
    for span in spans(&linked.text).unwrap() {
        match span {
            Span::Open { kind, token, .. } => open = Some((kind, token, String::new())),
            Span::Close => links.push(open.take().unwrap()),
            Span::Piece(piece) => {
                let text = match piece {
                    Piece::Text(text) => text.to_owned(),
                    Piece::Colour(code) => format!("\u{11}{code}"),
                    Piece::Icon(name) => format!("\u{13}{name} "),
                    other => panic!("unexpected {other:?}"),
                };
                if let Some((_, _, name)) = &mut open {
                    name.push_str(&text);
                }
                unmarked.push_str(&text);
            }
        }
    }
    assert_eq!(unmarked, plain.text);
    assert_eq!(
        links,
        [
            (EntityKind::Equipment, "mod_equipment", "Alpha equipment".to_owned()),
            (EntityKind::Module, "mod_module", "Zero module".to_owned()),
            (EntityKind::Module, "mod_second_module", "Zero second".to_owned()),
            (EntityKind::Building, "mod_lab", "Lab".to_owned()),
            (EntityKind::SubUnit, "mod_unit", "Alpha unit".to_owned()),
            (EntityKind::Tactic, "mod_tactic", "Alpha tactic".to_owned()),
        ]
    );
}

#[test]
fn technology_names_link_to_their_technology_only_when_asked() {
    use crate::effect_description::entity_links::unmark;
    use crate::effect_description::{EntityKind, Span, spans};
    use content_reader_localisation::localisation::Piece;
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    for (body, expected, token, index, name) in [
        ("set_technology={mod_tech=1}", "Give Alpha parent\n", "mod_tech", None, "Alpha parent"),
        ("set_technology={var:mod_second_handle=0}", "Give Other\n", "MOD_TECH", Some(6), "Other"),
        (
            "add_tech_bonus={technology=mod_tech bonus=0.5 uses=2}",
            "Bonus 0.50/2 uses:Alpha equipment\n",
            "mod_tech",
            None,
            "Alpha equipment",
        ),
        (
            "custom_effect_tooltip={localization_key=MOD_UNLOCK_TECH TECH=mod_tech}",
            "Unlocks \u{11}HMod technology\u{11}!.",
            "mod_tech",
            None,
            "Mod technology",
        ),
    ] {
        let items = content_reader_script::effect_keys::read_body(
            body.as_bytes(),
            content.view(None).key_names.unwrap(),
        )
        .unwrap();
        let render = |links| {
            fixture.with_context_linking(content.view(None), links, |context| {
                describe(&items, context, &mut fresh())
            })
        };
        let (plain, linked) = (render(false), render(true));
        assert_eq!(plain.text, expected, "{body}: {:?}", plain.refusals);
        assert!(plain.refusals.is_empty(), "{body}: {:?}", plain.refusals);
        assert_eq!(plain.refusals, linked.refusals);
        assert_eq!(unmark(&linked.text), plain.text, "{body}");
        let spans = spans(&linked.text).unwrap();
        let links: Vec<_> = spans
            .windows(3)
            .filter_map(|window| match window {
                [Span::Open { kind, token, index }, Span::Piece(Piece::Text(text)), Span::Close] => {
                    Some((*kind, *token, *index, *text))
                }
                _ => None,
            })
            .collect();
        assert_eq!(links, [(EntityKind::Technology, token, index, name)], "{body}");
    }
    let items = content_reader_script::effect_keys::read_body(
        b"custom_effect_tooltip={localization_key=MOD_UNLOCK_LOWER TECH=mod_tech}",
        content.view(None).key_names.unwrap(),
    )
    .unwrap();
    let render = |links| {
        fixture.with_context_linking(content.view(None), links, |context| {
            describe(&items, context, &mut fresh())
        })
    };
    let (plain, linked) = (render(false), render(true));
    assert_eq!(plain.text, "Unlocks mod technology.");
    assert!(plain.refusals.is_empty(), "{:?}", plain.refusals);
    assert!(!linked.text.contains("Mod technology"), "{linked:?}");
    assert!(
        linked
            .refusals
            .keys()
            .any(|why| why.contains("the game's formatting changes the linked technology names")),
        "a case directive on a linked name withholds the text by name instead of dropping the link: {linked:?}"
    );
    let items = content_reader_script::effect_keys::read_body(
        b"custom_effect_tooltip={localization_key=MOD_UNLOCK_BOTH TECH=mod_tech OTHER={localization_key=MOD_UNLOCK_TECH TECH=mod_tech}}",
        content.view(None).key_names.unwrap(),
    )
    .unwrap();
    let render = |links| {
        fixture.with_context_linking(content.view(None), links, |context| {
            describe(&items, context, &mut fresh())
        })
    };
    let (plain, linked) = (render(false), render(true));
    assert!(linked.refusals.is_empty(), "O-S5: a nested argument's own link does not make the outer comparison fail: {linked:?}");
    assert_eq!(unmark(&linked.text), plain.text);
}

#[test]
fn bound_technology_effect_combines_limits_and_accepts_unresearched_status() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    let output = fixture.render_native(
        "custom_effect_tooltip=tech_effect|mod_conditional",
        &mut fresh(),
        content.view(None),
    );
    assert_eq!(output.text, "");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn bound_technology_module_gate_uses_caller_country_and_names_use_reserved_zero_tag() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(2))]);
    let output = fixture.render_native(
        "custom_effect_tooltip=tech_effect|mod_callback",
        &mut scopes,
        content.view(None),
    );
    assert!(
        output.text.contains("Modules:\u{11}HZero second\u{11}!\n"),
        "{}",
        output.text
    );
    assert!(!output.text.contains("Zero module"));
    assert!(!output.text.contains("Denied module"));
    assert!(
        output
            .text
            .contains("Equipment:\u{11}HAlpha equipment\u{11}!\n")
    );
    assert!(output.text.contains("Complete BBB/BBB\n"));
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn bound_technology_effect_missing_identity_is_empty_but_missing_recording_refuses() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::ResearchTechnologies.index()] =
        TableView::Complete(TableRows::ResearchTechnologies(Vec::new()));
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    for scope in [
        Scopes::of(Scope::State(1)),
        Scopes::of(Scope::Character(7)),
        Scopes::of(Scope::Country(0)),
    ] {
        let text = fixture
            .with_renderer(&mut fresh(), content.view(None), |renderer| {
                renderer.bound_technology_effect("mod_callback", None, scope)
            })
            .unwrap();
        assert_eq!(text, "");
    }
    let output = fixture.render_native(
        "custom_effect_tooltip=tech_effect|missing_technology",
        &mut fresh(),
        content.view(None),
    );
    assert_eq!(output.text, "");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
    let output = fixture.render_native(
        "custom_effect_tooltip=tech_effect|mod_callback",
        &mut fresh(),
        content.view(None),
    );
    assert_eq!(output.text, "");
    assert!(output.refusals.keys().any(|why| why.contains("research technologies lacks bound country's technology status")), "{:?}", output.refusals);
}

#[test]
fn faction_power_projection_expands_with_recorded_membership() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::Factions.index()] =
        TableView::Complete(TableRows::Factions(vec![FactionRow {
            id: 5,
            name: b"Recorded faction".to_vec(),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![FactionMemberRow {
            country: 100,
            faction: 5,
            ordinal: 0,
        }]));
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    let output = fixture.render_native(
        "add_faction_power_projection=100",
        &mut fresh(),
        content.view(None),
    );
    assert_eq!(output.text, "Projection 100/Recorded faction AAA");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn bound_technology_modifier_recording_failure_preserves_other_fragments() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::ModifierDefinitions.index()] = TableView::Unavailable {
        reason: "synthetic fault".into(),
    };
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    let output = fixture.render_native(
        "custom_effect_tooltip=tech_effect|mod_callback",
        &mut fresh(),
        content.view(None),
    );
    assert!(
        output
            .text
            .starts_with("\u{13}GFX_mod_unit_icon_small|1 Alpha unit\n")
    );
    assert!(output.text.contains("Complete AAA/AAA\n"));
    assert!(
        output
            .text
            .ends_with("Tactics:\u{11}HAlpha tactic\u{11}!\n")
    );
    assert_eq!(output.refusals.values().sum::<usize>(), 1);
    assert!(
        output
            .refusals
            .keys()
            .any(|why| why.contains("technology modifiers")),
        "{:?}",
        output.refusals
    );
}

#[test]
fn bound_technology_unavailable_building_content_preserves_later_unlocks() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    let mut view = content.view(None);
    view.buildings = None;
    let output = fixture.render_native(
        "custom_effect_tooltip=tech_effect|mod_callback",
        &mut fresh(),
        view,
    );
    assert!(
        output
            .text
            .contains("Modules:\u{11}HZero module\u{11}!, \u{11}HZero second\u{11}!\n")
    );
    assert!(!output.text.contains("Buildings:"));
    assert!(output.text.contains("Subunits:\u{11}HAlpha unit\u{11}!\n"));
    assert!(output.text.ends_with("Modifier [Root.GetTag]\n"));
    assert_eq!(output.refusals.values().sum::<usize>(), 1);
    assert!(
        output
            .refusals
            .keys()
            .any(|why| why.contains("technology building unlocks")),
        "{:?}",
        output.refusals
    );
}

#[test]
fn bound_technology_repeated_building_and_tactic_compare_final_definition_pointer() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    let output = fixture.render_native(
        "custom_effect_tooltip=tech_effect|mod_repeated",
        &mut fresh(),
        content.view(None),
    );
    assert_eq!(
        output.text,
        concat!(
            "Buildings:\u{11}HLab\u{11}! (Level 1) plus \u{11}HLab\u{11}! (Level 2), \u{11}HStore\u{11}! (Level 3) plus \u{11}HLab\u{11}! (Level 4)\n",
            "Tactics:\u{11}HAlpha tactic\u{11}! plus \u{11}HAlpha tactic\u{11}!, \u{11}HAlpha other tactic\u{11}! plus \u{11}HAlpha tactic\u{11}!\n"
        )
    );
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}
