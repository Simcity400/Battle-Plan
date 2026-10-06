use super::*;
use crate::catalogue::tests::{Folder, folder_catalogue};
use crate::effect_description::tests::{Fixture, fresh};
use replay::{frame::TableView, rows::*};

fn fixture() -> (Fixture, Folder) {
    let mut fixture = Fixture::new();
    let (catalogue, folder) = folder_catalogue(&[
        ("common/country_tags/synthetic.txt", "AAA=a BBB=b CCC=c"),
        ("common/defines/test.lua", "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}},NSupply={MAX_RAILWAY_LEVEL=5}}"),
        ("common/technologies/synthetic.txt", "technology_categories={mod_category mod_other} technologies={mod_tech={parent=mod_parent enable_equipments={mod_equipment}} mod_small={parent=mod_parent force_use_small_tech_layout=yes enable_equipments={mod_equipment}}}"),
        ("common/units/equipment/synthetic.txt", "equipments={mod_equipment={is_archetype=yes type=infantry}}"),
        ("common/doctrines/folders/synthetic.txt", "mod_folder={name=mod_folder_name}"),
        ("common/doctrines/tracks/synthetic.txt", "mod_track={name=mod_track_name}"),
        ("common/doctrines/grand_doctrines/synthetic.txt", "mod_grand={name=mod_grand_name}"),
        ("common/doctrines/subdoctrines/land/synthetic.txt", "mod_sub={name=mod_sub_name}"),
        ("common/special_projects/specialization/synthetic.txt", "mod_science={} mod_other_science={}"),
        ("localisation/test_l_english.yml", concat!(
            "\u{feff}l_english:\n",
            " AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n CCC:0 \"Gamma\"\n",
            " EFFECT_ADD_RESEARCH_SLOT:0 \"Add $COUNT|0$ [Root.GetTag]\"\n EFFECT_REMOVE_RESEARCH_SLOT:0 \"Remove $COUNT|0$\"\n EFFECT_SET_RESEARCH_SLOTS:0 \"Slots $COUNT|0$\"\n",
            " EFFECT_INHERIT_TECHNOLOGY:0 \"Inherit $COUNTRY$\"\n",
            " EFFECT_MIO_ADD_RESEARCH_BONUS:0 \"MIO add $VAL|2$ [Root.GetTag]\"\n EFFECT_MIO_SET_RESEARCH_BONUS:0 \"MIO set $VAL|2$ [Root.GetTag]\"\n",
            " EFFECT_STEAL_RANDOM_TECH:0 \"Steal [Root.GetTag]\"\n EFFECT_STEAL_RANDOM_TECH_INSTANT:0 \"Steal instantly [Root.GetTag]\"\n",
            " FACTION_ACCESS_TAB_REASEARCH:0 \"Research [Root.GetTag]\"\n FACTION_ACCESS_TAB_UNLOCK:0 \"Unlock $NAME$\"\n FACTION_ACCESS_TAB_LOCK:0 \"Lock $NAME$\"\n",
            " mod_category:0 \"Category\"\n AAA_mod_category:0 \"Alpha category [Root.GetTag]\"\n mod_other:0 \"Other\"\n mod_parent:0 \"Parent\"\n AAA_mod_parent:0 \"Alpha parent\"\n mod_equipment:0 \"Equipment\"\n AAA_mod_equipment:0 \"Alpha equipment\"\n",
            " EFFECT_GIVE_TECHNOLOGY:0 \"Give $NAME$\"\n AND:0 \"and \"\n EFFECT_TECH_BONUS_USES:0 \"$USES|0$ uses\"\n",
            " EFFECT_ADD_TECH_BONUS_BONUS_ONLY:0 \"Bonus $BONUS|2$/$USES$:$LIST$\"\n EFFECT_ADD_TECH_BONUS_AHEAD_ONLY:0 \"Ahead $AHEAD|2$/$USES$:$LIST$\"\n EFFECT_ADD_TECH_BONUS:0 \"Both $AHEAD|2$/$BONUS|2$/$USES|0$:$LIST$\"\n",
            " mod_folder_name:0 \"Folder\"\n mod_track_name:0 \"Track\"\n mod_grand_name:0 \"Grand\"\n mod_sub_name:0 \"Sub\"\n",
            " SET_SUB_DOCTRINE_EFFECT:0 \"Set sub $NAME$\"\n SET_GRAND_DOCTRINE_EFFECT:0 \"Set grand $NAME$\"\n ADD_DAILY_MASTERY_EFFECT:0 \"Daily $AMOUNT|2$/$DAYS|0$\"\n ADD_MASTERY_BONUS_EFFECT:0 \"Mastery bonus $BONUS|2$/$DAYS|0$\"\n",
            " TRACK_EFFECT_ALL_TRACKS:0 \"$TRACK_EFFECT_DESC$ all\"\n TRACK_EFFECT_ONLY_FOLDER:0 \"$TRACK_EFFECT_DESC$ folder $NAME$\"\n TRACK_EFFECT_ONLY_GRAND_DOCTRINE:0 \"$TRACK_EFFECT_DESC$ grand $NAME$\"\n TRACK_EFFECT_ONLY_SUBDOCTRINE:0 \"$TRACK_EFFECT_DESC$ sub $NAME$\"\n TRACK_EFFECT_ONLY_TRACK:0 \"$TRACK_EFFECT_DESC$ track $NAME$\"\n",
            " TRACK_EFFECT_COMPLEX_HEADER:0 \"$TRACK_EFFECT_DESC$ conditions:\"\n TRACK_EFFECT_CONDITION_FOLDER:0 \"Folder $NAME$\"\n TRACK_EFFECT_CONDITION_GRAND_DOCTRINE:0 \"Grand $NAME$\"\n TRACK_EFFECT_CONDITION_SUBDOCTRINE:0 \"Sub $NAME$\"\n TRACK_EFFECT_CONDITION_TRACK_TYPE:0 \"Track $NAME$\"\n TRACK_EFFECT_CONDITION_TRACK_INDEX:0 \"Index $INDEX|0$\"\n",
            " mod_science:0 \"Science\"\n AAA_mod_science:0 \"Alpha science [Root.GetTag]\"\n mod_other_science:0 \"Other science\"\n",
            " ADD_BREAKTHROUGH_POINTS_EFFECT_DESC:0 \"Points:\"\n ADD_BREAKTHROUGH_PROGRESS_EFFECT_DESC:0 \"Progress:\"\n ADD_BREAKTHROUGH_POINTS_EFFECT_SPECIALIZATION:0 \"$SPECIALIZATION$:$AMOUNT|0$\"\n ADD_BREAKTHROUGH_PROGRESS_EFFECT_SPECIALIZATION:0 \"$SPECIALIZATION$:$AMOUNT|2$\"\n",
            " ADD_FACTION_POWER_PROJECTION_EFFECT_NO_FACTION:0 \"Projection $VAL|0$ [Root.GetTag]\"\n"
        )),
    ]);
    catalogue.scripted_content().unwrap().set_synthetic_environment(Default::default());
    fixture.catalogue = catalogue;
    *fixture.frame.tables[TableId::CountryAppearance.index()] = TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow { country: 100, ..Default::default() }]));
    *fixture.frame.tables[TableId::FactionMembers.index()] = TableView::Complete(TableRows::FactionMembers(Vec::new()));
    *fixture.frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![
        LexicalTokenRow { token: 17, literal: b"mod_science".to_vec() },
        LexicalTokenRow { token: 22, literal: b"mod_other_science".to_vec() },
    ]));
    *fixture.frame.tables[TableId::TechnologyDefinitions.index()] = TableView::Complete(TableRows::TechnologyDefinitions(vec![TechnologyDefinitionRow { technology_handle: 42, database_index: 7, id: b"mod_tech".to_vec(), ..Default::default() }]));
    *fixture.frame.tables[TableId::ScriptVariables.index()] = TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow { scope_kind: 1, scope_id: 100, entries: vec![ScriptVariableEntry { name: b"mod_bonus".to_vec(), value_raw: 25_000 }, ScriptVariableEntry { name: b"mod_handle".to_vec(), value_raw: 4_200_000 }], ..Default::default() }]));
    (fixture, folder)
}

macro_rules! ported {
    ($name:ident, $body:expr, $expected:expr) => {
        #[test]
        fn $name() {
            let (fixture, _folder) = fixture();
            let content = fixture.catalogue.scripted_content().unwrap().effects().unwrap();
            let output = fixture.render_native($body, &mut fresh(), content.view(None));
            assert_eq!(output.text, $expected);
            assert!(output.refusals.is_empty(), "{:?}", output.refusals);
        }
    };
}

// Inventory witness forms, with synthetic content identifiers/localisation.
// common/decisions/afg.txt:445; _debug_decisions.txt:170; raj_goe.txt:1833.
ported!(add_tech_bonus, "add_tech_bonus={name=mod_name bonus=0.75 uses=1 category=mod_category}", "Bonus 0.75/1 uses:Alpha category [Root.GetTag]\n");
ported!(set_technology, "set_technology={mod_tech=1 popup=no}", "Give Alpha parent\n");
ported!(add_research_slot, "add_research_slot=1", "Add 1 [Root.GetTag]\n");
// common/decisions/cog.txt:1275; jap.txt:6912.
ported!(add_breakthrough_progress, "add_breakthrough_progress={specialization=mod_science value=0.1}", "Progress:\n  Alpha science [Root.GetTag]:0.10\n");
ported!(add_mastery_bonus, "add_mastery_bonus={bonus=0.15 days=180 name=mod_bonus track=mod_track}", "Mastery bonus 0.15/180 track Track");
// common/decisions/cze.txt:66; national_focus/australia_taog.txt:9468.
ported!(inherit_technology, "inherit_technology=BBB", "Inherit Beta");
ported!(add_mio_research_bonus, "add_mio_research_bonus=0.05", "MIO add 0.05 AAA\n");
// common/operations/00_operations.txt:1955.
ported!(steal_random_tech_bonus, "steal_random_tech_bonus={folder=electronics_folder folder=industry_folder bonus=3.0 name=mod_name target=FROM dynamic=yes instant=yes uses=1}", "Steal instantly AAA");
// national_focus/china_nationalist_sea.txt:10341/:10009; afghanistan.txt:10561.
ported!(set_sub_doctrine, "set_sub_doctrine=mod_sub", "Set sub Sub");
ported!(add_daily_mastery, "add_daily_mastery={amount=0.5 days=100 name=mod_name track=mod_track}", "Daily 0.50/100 track Track");
ported!(set_research_slots, "set_research_slots=2", "Slots 2");
// national_focus/argentina.txt:1111; italy.txt:4219; russia.txt:6499.
ported!(add_breakthrough_points, "add_breakthrough_points={specialization=mod_science value=1}", "Points:\n  Alpha science [Root.GetTag]:1\n");
ported!(set_grand_doctrine, "set_grand_doctrine=mod_grand", "Set grand Grand");
ported!(add_faction_power_projection, "add_faction_power_projection=100", "Projection 100 AAA");
// events/ire events (ireland).txt:16173; scripted_effects/00_faction_scripted_effects.txt:125.
ported!(set_mio_research_bonus, "set_mio_research_bonus=var:mod_bonus", "MIO set 0.25 AAA\n");
ported!(set_faction_research_unlocked, "set_faction_research_unlocked=yes", "Unlock Research AAA");

macro_rules! core_gap {
    ($name:ident, $body:expr, $class:expr, $reason:expr) => {
        #[test]
        fn $name() {
            let (fixture, _folder) = fixture();
            let content = fixture.catalogue.scripted_content().unwrap().effects().unwrap();
            let output = fixture.render_native($body, &mut fresh(), content.view(None));
            assert_eq!(output.text, "");
            assert_eq!(output.refusals, BTreeMap::from([(format!("synthetic effect {}: {}", $class, $reason), 1)]));
        }
    };
}

// national_focus/italy.txt:4407; decisions/sia.txt:1736; raids/air_raids.txt:188.
core_gap!(add_project_progress_ratio, "add_project_progress_ratio=0.5", "CAddProjectProgressRatioEffect@?A0x9898cfbe@NProject", "project scope carrier +0x90/+0x94 is pending lead I's core (140529480, own 140493450)");
core_gap!(complete_special_project, "complete_special_project=sp:mod_project", "CCompleteProjectEffect@?A0x9898cfbe@NProject", "project selector sp: and scope carrier are pending lead I's core (1405382B0/1405381D0, own 140494060)");
core_gap!(raid_reduce_project_progress_ratio, "raid_reduce_project_progress_ratio=0.05", "CRaidReduceProjectProgressRatioEffect@?A0x8a0aef5d@NRaids", "raid scope carrier +0x88/+0x8C is pending lead I's core (1404A43D0, own 1404A3F20)");

ported!(research_slot_removal_zero, "add_research_slot=0", "Remove 0\n");
ported!(technology_handle_uses_handle_not_database_index, "set_technology={var:mod_handle=0}", "Give Alpha parent\n");
ported!(technology_bonus_runtime_equipment_name, "add_tech_bonus={technology=mod_tech bonus=0.5 uses=2}", "Bonus 0.50/2 uses:Alpha equipment\n");
ported!(technology_bonus_both_passes_integer_uses, "add_tech_bonus={technology=mod_small category=mod_category category=MOD_CATEGORY ahead_reduction=1 bonus=0.5 uses=2}", "Both 1.00/0.50/2:Alpha parent and Alpha category [Root.GetTag]\n");
ported!(mastery_complex_filter_native_order, "add_daily_mastery={amount=1 folder=mod_folder grand_doctrine=mod_grand sub_doctrine=mod_sub track=mod_track track_index=2}", "Daily 1.00/0 conditions:\nFolder Folder\nGrand Grand\nSub Sub\nTrack Track\nIndex 2");
ported!(breakthrough_all_keeps_token_slot_order, "add_breakthrough_points={specialization=all specialization=mod_science value=1}", "Points:\n  Other science:1\n  Alpha science [Root.GetTag]:1\n");

#[test]
fn technology_catalogue_is_cached_once_per_stack() {
    let (fixture, folder) = fixture();
    let scripted = fixture.catalogue.scripted_content().unwrap();
    let content = scripted.effects().unwrap();
    std::fs::write(folder.0.join("common/technologies/synthetic.txt"), "technologies={mod_tech={parent=wrong}}").unwrap();
    assert!(std::ptr::eq(content, scripted.effects().unwrap()));
    let output = fixture.render_native("set_technology={mod_tech=1}", &mut fresh(), content.view(None));
    assert_eq!(output.text, "Give Alpha parent\n");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}
