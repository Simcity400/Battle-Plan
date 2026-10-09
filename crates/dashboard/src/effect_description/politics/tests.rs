use super::*;
use crate::catalogue::tests::{Folder, folder_catalogue};
use crate::effect_description::tests::Fixture;
use crate::scripted_text::{EventObject, EventScope, Link};
use replay::{frame::TableView, rows::*};

pub(super) fn fixture() -> (Fixture, Folder) {
    let mut fixture = Fixture::new();
    let (catalogue, folder) = folder_catalogue(&[
        ("common/country_tags/synthetic.txt", "AAA=a BBB=b CCC=c"),
        (
            "common/national_focus/synthetic.txt",
            "focus_tree={id=SYN_TREE focus={id=SYN_FOCUS text=SYN_FOCUS_NAME}}",
        ),
        (
            "common/continuous_focus/synthetic.txt",
            "continuous_focus_palette={focus={id=SYN_FOCUS text=SYN_WRONG_NAME}}",
        ),
        (
            "common/technology_sharing/synthetic.txt",
            "technology_sharing_group={id=SYN_GROUP name=SYN_GROUP_NAME}",
        ),
        (
            "common/intelligence_agency_upgrades/synthetic.txt",
            "SYN_BRANCH={SYN_UPGRADE={}}",
        ),
        (
            "common/operation_tokens/synthetic.txt",
            "SYN_TOKEN={name=SYN_TOKEN_NAME text_icon=SYN_ICON icon=SYN_UNUSED_ICON}",
        ),
        ("common/combat_tactics.txt", "SYN_TACTIC={is_attacker=yes}"),
        (
            "common/scripted_triggers/synthetic.txt",
            "SYN_TRIGGER={custom_trigger_tooltip={tooltip=SYN_TARGET_TITLE always=yes}}",
        ),
        (
            "common/ideologies/synthetic.txt",
            "ideologies={SYN_IDEOLOGY={types={SYN_SUB={}}} SYN_OTHER={types={OTHER_SUB={}}}}",
        ),
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " syn_ideology:0 \"Ideology [Root.GetTag]\"\n syn_ideology_noun:0 \"Noun [Root.GetTag]\"\n syn_other:0 \"Other\"\n SYN_PARTY:0 \"Party [Root.GetTag]\"\n SYN_LONG:0 \"Long party [Root.GetTag]\"\n",
                " EFFECT_SET_POPULARITIES:0 \"Set popularities [Root.GetTag]:\"\n",
                " SYN_FOCUS_NAME:0 \"Focus [THIS.GetTag]/[ROOT.GetTag]/[FROM.GetTag]/[PREV.GetTag]\"\n",
                " SYN_FOCUS_TOOLTIP:0 \"for [FROM.GetTag]\"\n UNLOCK_COMPLETE_EFFECT:0 \"Complete $WHO$:$LIST$\"\n UNLOCK_COMPLETE_EFFECT_FOR:0 \"Complete $WHO$/$FOR$:$FOCUS$\"\n UNLOCK_FOCUS_EFFECT:0 \"Unlock $WHO$:$LIST$\"\n UNCOMPLETE_FOCUS_EFFECT:0 \"Uncomplete $WHO$:$LIST$\"\n",
                " SYN_GROUP_NAME:0 \"Group [FROM.GetTag]\"\n EFFECT_ADD_COUNTRY_TO_TECH_SHARING_GROUP:0 \"Join $TAG$/$COUNTRY$:$GROUPNAME$ [Root.GetTag]\"\n EFFECT_REMOVE_COUNTRY_FROM_TECH_SHARING_GROUP:0 \"Leave $TAG$/$COUNTRY$:$GROUPNAME$ [Root.GetTag]\"\n",
                " EFFECT_MODIFY_COUNTRY_TECH_SHARING_BONUS:0 \"Sharing $COUNTRY$/$GROUPNAME$:$BONUS|2%$ [Root.GetTag]\"\n",
                " SYN_UPGRADE:0 \"Upgrade [FROM.GetTag]\"\n EFFECT_UPGRADE_INTELLIGENCE_AGENCY:0 \"Agency $UPGRADE$\"\n",
                " SYN_TOKEN_NAME:0 \"Token [Root.GetTag]\"\n OPERATION_TOKEN_RECEIVE:0 \"Receive $TOKEN$/$COUNTRY$\"\n OPERATION_TOKEN_REMOVE:0 \"Remove $TOKEN$/$COUNTRY$\"\n SYN_TACTIC:0 \"Tactic [Root.GetTag]\"\n UNLOCK_TACTIC_EFFECT:0 \"Unlock tactic $NAME$\"\n SYN_TARGET_TITLE:0 \"Eligible [FROM.GetTag]\"\n",
                " POLITICS_IDEOLOGY_POPULARITY_CHANGE:0 \"Popularity $IDEOLOGY$: $VALUE|2$\"\n POLITICS_PARTY_NAME_CHANGE:0 \"Party name $IDEOLOGY$ => $NAME$\"\n",
                " SYN_COSMETIC_DEF:0 \"Costume [Root.GetTag]\"\n EFFECT_SET_COSMETIC_TAG:0 \"Cosmetic $COUNTRY$ => $COSMETIC_NAME$ [Root.GetTag]\"\n",
                " EFFECT_SET_RULE:0 \"Rules [Root.GetTag]:\"\n EFFECT_SET_PARTY_RULE:0 \"Rules $IDEOLOGY$:\"\n TRIGGER_FULLFILLED_PREFIX:0 \"[yes]\"\n TRIGGER_UNFULLFILLED_PREFIX:0 \"[no]\"\n",
                " EFFECT_ADD_RELATION_RULE_OVERRIDE:0 \"Relation $ORIGIN$->$TARGET$:\"\n EFFECT_REMOVE_RELATION_RULE_OVERRIDE:0 \"Unrelation $ORIGIN$->$TARGET$:\"\n",
                " CAN_SEND_VOLUNTEERS:0 \"Volunteers\"\n CAN_CREATE_FACTIONS:0 \"Factions\"\n",
                " AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n CCC:0 \"Gamma\"\n",
                " AAA_DEF:0 \"alpha\"\n BBB_DEF:0 \"beta\"\n CCC_DEF:0 \"gamma\"\n",
                " EFFECT_ADD_CORE_OF:0 \"Core $COUNTRY$ [Root.GetTag]\"\n EFFECT_REMOVE_CORE_OF:0 \"Uncore $COUNTRY$ [Root.GetTag]\"\n",
                " EFFECT_ADD_CLAIM_BY:0 \"Claim @$TAG$$COUNTRY$ [Root.GetTag]\"\n EFFECT_REMOVE_CLAIM_BY:0 \"Unclaim @$TAG$$COUNTRY$ [Root.GetTag]\"\n",
                " EFFECT_LOAD_OOB:0 \"Load $NAME$ [Root.GetTag]\"\n EFFECT_SET_OOB:0 \"Set $NAME$ [Root.GetTag]\"\n",
                " SYN_TREE:0 \"Tree [Root.GetTag]\"\n EFFECT_LOAD_FOCUS_TREE:0 \"Use $TREE_NAME$\"\n",
                " EFFECT_RELEASE_NATION:0 \"Release $COUNTRY$ [Root.GetTag]\"\n CHANGE_TAG_EFFECT:0 \"Change $COUNTRY$ [Root.GetTag]\"\n",
                " EFFECT_ADD_THREAT:0 \"Add $THREAT|2$\"\n EFFECT_REMOVE_THREAT:0 \"Remove $THREAT|2$\"\n",
                " EFFECT_SEND_EMBARGO:0 \"Embargo $COUNTRY$\"\n EFFECT_BREAK_EMBARGO:0 \"Unembargo $COUNTRY$\"\n",
                " EFFECT_SCOPED_SOUND_DESC:0 \"Sound $SOUND_NAME$ for $COUNTRY$\"\n EFFECT_SOUND_DESC:0 \"Sound $SOUND_NAME$\"\n SYN_SOUND:0 \"Localised sound\"\n",
                " EFFECT_SET_MAJOR_YES:0 \"Major $COUNTRY$\"\n EFFECT_SET_MAJOR_NO:0 \"Minor $COUNTRY$\"\n",
                " EFFECT_RECALL_VOLUNTEERS_FROM:0 \"Volunteers $COUNTRY$\"\n EFFECT_RECALL_ATTACHE:0 \"Attache $COUNTRY$\"\n POLITICS_HOLD_ELECTION:0 \"Election $COUNTRY$\"\n",
                " POLITICS_SET_POLITICAL_POWER:0 \"Power $VAL|2$\"\n",
                " SYN_MIO:0 \"Foundry [Root.GetTag]\"\n EFFECT_MIO_SET_NAME_KEY:0 \"Rename $NAME$\"\n EFFECT_MIO_SET_ICON:0 \"Icon [Root.GetTag]\"\n",
                " EFFECT_RELEASE_FROM_CAPTIVITY:0 \"Free [Root.GetTag]\"\n EFFECT_RELEASE_GENERALS:0 \"Generals $COUNTRY_NAME_DEF$ [Root.GetTag]\"\n",
                " EFFECT_CREATE_INTELLIGENCE_AGENCY:0 \"Agency [Root.GetTag]\"\n",
                " EFFECT_ADD_ARMY_EXPERIENCE:0 \"XP$VAL|0$\"\n",
                " EFFECT_ADD_CONTESTED_OWNER:0 \"Add contested [Country.GetTag]/[State.GetName]/[ROOT.GetTag]/[FROM.GetTag]/[PREV.GetTag]\"\n",
                " EFFECT_REMOVE_CONTESTED_OWNER:0 \"Remove contested [Country.GetTag]/[State.GetName]/[ROOT.GetTag]/[FROM.GetTag]/[PREV.GetTag]\"\n",
                " SYN_FOCUS_ONE:0 \"First [Root.GetTag]\"\n SYN_FOCUS_TWO:0 \"Second\"\n",
                " REDUCE_FOCUS_DAYS_EFFECT_NEGATIVE:0 \"Extend $FOCUS$ $DAYS|0$\"\n REDUCE_FOCUS_DAYS_EFFECT:0 \"Reduce $FOCUS$ $DAYS|0$\"\n",
                " REDUCE_FOCUS_DAYS_MULTIPLE_HEADER_NEGATIVE:0 \"Extend $DAYS|0$ [Root.GetTag]\"\n REDUCE_FOCUS_DAYS_MULTIPLE_HEADER:0 \"Reduce $DAYS|0$ [Root.GetTag]\"\n",
                " REDUCE_FOCUS_DAYS_MULTIPLE_FOCUS:0 \" $FOCUS$\"\n"
            ),
        ),
    ]);
    catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    fixture.catalogue = catalogue;
    *fixture.frame.tables[TableId::Clock.index()] =
        TableView::Complete(TableRows::Clock(vec![ClockRow {
            seed: 42,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 100,
            seed: replay::rows::Recorded::Value(None),
            entries: vec![ScriptVariableEntry {
                name: b"syn_support".to_vec(),
                value_raw: 3_000_000,
            }],
        }]));
    *fixture.frame.tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            dlc_mask: 1 << 5,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::IntelligenceAgencies.index()] =
        TableView::Complete(TableRows::IntelligenceAgencies(vec![
            IntelligenceAgencyRow {
                country: 100,
                ..Default::default()
            },
        ]));
    if let TableView::Complete(TableRows::Politics(rows)) =
        &mut *fixture.frame.tables[TableId::Politics.index()]
    {
        rows.iter_mut()
            .find(|row| row.country == 100)
            .unwrap()
            .ruling_party = Some(20);
    }
    *fixture.frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![
            PoliticalPartyRow {
                country: 100,
                id: 20,
                token: b"syn_ideology".to_vec(),
                ..Default::default()
            },
            PoliticalPartyRow {
                country: 100,
                id: 30,
                token: b"syn_other".to_vec(),
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 0x10010,
            literal: b"SYN_GROUP".to_vec(),
        }]));
    *fixture.frame.tables[TableId::TechSharingGroups.index()] =
        TableView::Complete(TableRows::TechSharingGroups(vec![TechSharingGroupRow {
            id: 0x10010,
            member_count: 1,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::TechSharingMembers.index()] =
        TableView::Complete(TableRows::TechSharingMembers(vec![TechSharingMemberRow {
            group: 0x10010,
            country: 100,
            ..Default::default()
        }]));
    (fixture, folder)
}

fn fresh() -> EventScopes {
    EventScopes::new(vec![EventScope::of(EventObject::Country(1))])
}

fn state_from_root() -> EventScopes {
    let mut state = EventScope::of(EventObject::State(2));
    state.root = Link::Scope(1);
    EventScopes::new(vec![state, EventScope::of(EventObject::Country(1))])
}

macro_rules! ported {
    ($name:ident, $body:expr, $expected:expr, $scopes:expr) => {
        #[test]
        fn $name() {
            let (fixture, _folder) = fixture();
            let content = fixture
                .catalogue
                .scripted_content()
                .unwrap()
                .effects()
                .unwrap();
            let output = fixture.render($body, &mut $scopes, content.view(None));
            assert_eq!(output.text, $expected);
            assert!(output.refusals.is_empty(), "{:?}", output.refusals);
        }
    };
}

// Authored witness forms; identifiers and all localisation are synthetic.
// common/bop/swi.txt:31, common/decisions/afg.txt:1094, chi_decisions.txt:5596.
ported!(log, "log=\"synthetic log\"", "", fresh());
ported!(
    add_core_of,
    "add_core_of=ROOT",
    "Core Alpha [Root.GetTag]\n",
    state_from_root()
);
ported!(
    add_claim_by,
    "add_claim_by=ROOT",
    "Claim \u{12}AAAAlpha [Root.GetTag]\n",
    state_from_root()
);
// common/decisions/cze.txt:177, bel.txt:865, ast.txt:3452.
ported!(
    load_oob,
    "load_oob=\"SYN_OOB\"",
    "Load SYN_OOB AAA",
    fresh()
);
ported!(
    load_focus_tree,
    "load_focus_tree={tree=SYN_TREE keep_completed=yes}",
    "Use Tree AAA\n",
    fresh()
);
ported!(release, "release=BBB", "Release Beta AAA\n", fresh());
// common/decisions/cze.txt:3609, chi_decisions.txt:9558, cze.txt:68, bol.txt:65.
ported!(
    remove_core_of,
    "remove_core_of=BBB",
    "Uncore Beta [Root.GetTag]\n",
    state_from_root()
);
ported!(
    add_named_threat,
    "add_named_threat={threat=2 name=SYN_REASON}",
    "Add 2.00\n",
    fresh()
);
ported!(
    change_tag_from,
    "change_tag_from=BBB",
    "Change Alpha AAA\n",
    fresh()
);
#[test]
fn change_tag_from_state_keeps_the_native_source_name_refusal_counted() {
    let (fixture, _folder) = fixture();
    // Target 14052B6A0 resolves THIS to the state's owner, but own 14037B2B0
    // names scope.country, which remains zero for a state scope.
    let output = fixture.render(
        "change_tag_from=THIS change_tag_from=THIS",
        &mut state_from_root(),
        Content::default(),
    );
    assert_eq!(output.text, "");
    assert_eq!(
        output.refusals,
        BTreeMap::from([(
            "synthetic effect CChangeTagEffect: key change_tag_from: current scope has no country (1405336C0)".to_owned(),
            2,
        )]),
    );
}
ported!(
    remove_claim_by,
    "remove_claim_by=ROOT",
    "Unclaim \u{12}AAAAlpha [Root.GetTag]\n",
    state_from_root()
);
// common/decisions/ita.txt:4860, ast.txt:808, common/national_focus/argentina.txt:844.
ported!(
    create_intelligence_agency,
    "create_intelligence_agency=yes",
    "Agency AAA\n",
    fresh()
);
ported!(
    reduce_focus_completion_cost,
    "reduce_focus_completion_cost={focus={SYN_FOCUS_ONE SYN_FOCUS_TWO} cost=-7}",
    "Extend 7 [Root.GetTag]\n First AAA\n Second\n",
    fresh()
);
ported!(send_embargo, "send_embargo=BBB", "Embargo Beta\n", fresh());
// events/pb_superevents.txt:36, mun_czechoslovakia.txt:927, bba_italy.txt:9334.
ported!(
    scoped_sound_effect,
    "scoped_sound_effect=SYN_SOUND",
    "Sound SYN_SOUND for Alpha",
    fresh()
);
ported!(
    scoped_play_song,
    "scoped_play_song=\"SYN_SOUND\"",
    "Sound SYN_SOUND for Alpha",
    fresh()
);
ported!(
    release_on_controlled,
    "release_on_controlled=BBB",
    "Release Beta AAA\n",
    fresh()
);
// common/decisions/ita.txt:1576; common/national_focus/australia_taog.txt:2274, italy.txt:18842.
ported!(
    sound_effect,
    "sound_effect=SYN_SOUND",
    "Sound SYN_SOUND",
    fresh()
);
ported!(set_major, "set_major=yes", "Major Alpha\n", fresh());
ported!(
    play_song,
    "play_song=\"SYN_SOUND\"",
    "Sound SYN_SOUND",
    fresh()
);
// common/decisions/bul.txt:4015; common/national_focus/hungary.txt:858, lep focus (league of eight provinces).txt:61.
ported!(
    recall_volunteers_from,
    "recall_volunteers_from=ROOT",
    "Volunteers alpha\n",
    fresh()
);
ported!(
    hold_election,
    "hold_election=BBB",
    "Election Beta\n",
    fresh()
);
ported!(
    set_political_power,
    "set_political_power=0",
    "Power 0.00\n",
    fresh()
);
// common/decisions/fra.txt:1360; common/on_actions/10_toa_on_actions.txt:20; fra.txt:1367.
ported!(
    set_mio_name_key,
    "set_mio_name_key=SYN_MIO",
    "Rename Foundry AAA\n",
    fresh()
);
ported!(
    break_embargo,
    "break_embargo=ROOT",
    "Unembargo Alpha\n",
    fresh()
);
ported!(set_mio_icon, "set_mio_icon=SYN_ICON", "Icon AAA\n", fresh());
// common/raids/land_infiltration_raids.txt:140; common/operations/00_operations.txt:155.
ported!(
    release_from_captivity,
    "release_from_captivity=yes",
    "Free [Root.GetTag]",
    fresh()
);
ported!(
    release_captured_generals_from,
    "release_captured_generals_from=ROOT",
    "Generals alpha [Root.GetTag]",
    fresh()
);
// events/generic.txt:1433, tno_ukraine.txt:8202.
ported!(
    recall_attache,
    "recall_attache=ROOT",
    "Attache alpha\n",
    fresh()
);
ported!(set_oob, "set_oob=\"SYN:OOB\"", "Set SYN:OOB AAA", fresh());

ported!(
    negative_and_zero_named_threat,
    "add_named_threat={threat=-1.25 name=SYN_REASON} add_named_threat={threat=0}",
    "Remove 1.25\nRemove 0.00\n",
    fresh()
);
ported!(
    minor_uses_bool_not_country_selector,
    "set_major=no set_major=\"yes\" set_major=invalid",
    "Minor Alpha\nMajor Alpha\nMinor Alpha\n",
    fresh()
);
ported!(
    single_focus_cost,
    "reduce_focus_completion_cost={focus=SYN_FOCUS_ONE cost=0x7}",
    "Reduce First AAA 7\n",
    fresh()
);
ported!(
    focus_cost_appends_repeated_lists,
    "reduce_focus_completion_cost={focus=SYN_FOCUS_ONE focus={SYN_FOCUS_TWO} cost=9}",
    "Reduce 9 [Root.GetTag]\n First AAA\n Second\n",
    fresh()
);
ported!(
    empty_focus_cost,
    "reduce_focus_completion_cost={focus={} cost=7}",
    "",
    fresh()
);
ported!(
    captured_generals_block,
    "release_captured_generals_from={province=2 target=BBB}",
    "Generals beta [Root.GetTag]",
    fresh()
);
ported!(
    captured_generals_ignores_non_native_captured_by,
    "release_captured_generals_from={captured_by=BBB}",
    "",
    fresh()
);

#[test]
fn agency_feature_and_created_gates_do_not_read_more_than_native() {
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![
            SimulationFeatureRow::default(),
        ]));
    *fixture.frame.tables[TableId::IntelligenceAgencies.index()] = TableView::Unavailable {
        reason: "not needed".into(),
    };
    let output = fixture.render(
        "create_intelligence_agency=yes",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(output, Description::default());
    *fixture.frame.tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            dlc_mask: 1 << 5,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::IntelligenceAgencies.index()] =
        TableView::Complete(TableRows::IntelligenceAgencies(vec![
            IntelligenceAgencyRow {
                country: 100,
                created: 1,
                ..Default::default()
            },
        ]));
    let output = fixture.render(
        "create_intelligence_agency=yes",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(output, Description::default());
}

#[test]
fn missing_agency_facts_are_named_and_counted() {
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::SimulationFeatures.index()] = TableView::Unavailable {
        reason: "missing mask".into(),
    };
    let output = fixture.render(
        "create_intelligence_agency=yes create_intelligence_agency=yes",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(output.text, "");
    assert_eq!(output.refusals.values().sum::<usize>(), 2);
    assert!(
        output
            .refusals
            .keys()
            .all(|why| why.contains("CCreateIntelligenceAgencyEffect")
                && why.contains("missing mask"))
    );
}

#[test]
fn empty_agency_own_text_uses_core_projection_and_child_walk() {
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::IntelligenceAgencies.index()] =
        TableView::Complete(TableRows::IntelligenceAgencies(vec![
            IntelligenceAgencyRow {
                country: 100,
                created: 1,
                ..Default::default()
            },
        ]));
    // 1403B1480 keeps name/icon as own fields and forwards the effect child.
    let output = fixture.render(
        "create_intelligence_agency={name=SYN_NAME icon=SYN_ICON army_experience=2}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(output.text, "XP2\n");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn country_targets_support_five_thousand_synthetic_countries() {
    let (mut fixture, _folder) = fixture();
    let countries = (1..=5_000)
        .map(|id| CountryRow {
            id,
            canonical_slot: id,
            tag: if id == 5_000 {
                b"BBB".to_vec()
            } else {
                format!("S{id}").into_bytes()
            },
            ..Default::default()
        })
        .collect();
    *fixture.frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries));
    *fixture.frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(
            (1..=5_000)
                .map(|id| CountryLinkRow {
                    id,
                    canonical_slot: id,
                })
                .collect(),
        ));
    *fixture.frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(
            (1..=5_000)
                .map(|country| CountryAppearanceRow {
                    country,
                    ..Default::default()
                })
                .collect(),
        ));
    *fixture.frame.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(
        (1..=5_000)
            .map(|country| PoliticsRow {
                country,
                ..Default::default()
            })
            .collect(),
    ));
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(5_000))]);
    let output = fixture.render("hold_election=ROOT", &mut scopes, Content::default());
    assert_eq!(output.text, "Election Beta\n");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
    for (key, action) in [
        ("add_contested_owner", "Add"),
        ("remove_contested_owner", "Remove"),
    ] {
        let output = fixture.render(&format!("{key}=2"), &mut scopes, Content::default());
        assert_eq!(
            output.text,
            format!("{action} contested BBB/Two/BBB/BBB/BBB")
        );
        assert!(output.refusals.is_empty(), "{:?}", output.refusals);
    }
}

// Each unavailable dependency remains a class-specific, counted refusal.
macro_rules! refused {
    ($name:ident, $body:expr, $why:expr) => {
        #[test]
        fn $name() {
            let (fixture, _folder) = fixture();
            let body = $body;
            let key = body.split('=').next().unwrap();
            let class = installed_content::effect_keys::native_class(key.as_bytes()).unwrap();
            let output = fixture.render(body, &mut fresh(), Content::default());
            assert_eq!(output.text, "");
            assert_eq!(
                output.refusals,
                BTreeMap::from([(
                    format!("synthetic effect {}: key {key}: {}", class.name, $why),
                    1
                )])
            );
        }
    };
}
// Authored witness: common/decisions/jap2.txt:172.
ported!(
    add_contested_owner,
    "add_contested_owner=2",
    "Add contested AAA/Two/AAA/AAA/AAA",
    fresh()
);
// Authored witness: common/decisions/ser.txt:2115.
ported!(
    remove_contested_owner,
    "remove_contested_owner=BBB",
    "Remove contested BBB/Two/BBB/BBB/BBB",
    state_from_root()
);

#[test]
fn contested_owner_uses_both_native_operand_branches_and_fresh_links() {
    let (fixture, _folder) = fixture();
    for (body, object, expected) in [
        (
            "add_contested_owner=BBB",
            EventObject::State(2),
            "Add contested BBB/Two/BBB/BBB/BBB",
        ),
        (
            "remove_contested_owner=3",
            EventObject::Country(2),
            "Remove contested BBB/Three/BBB/BBB/BBB",
        ),
        (
            "remove_contested_owner=BBB",
            EventObject::CountryState {
                country: 1,
                state: 3,
            },
            "Remove contested BBB/Three/BBB/BBB/BBB",
        ),
    ] {
        let mut graph = EventScopes::new(vec![EventScope {
            root: Link::Unknown,
            from: Link::Unknown,
            prev: Link::Unknown,
            random_seed: 99,
            random_counter: 100,
            ..EventScope::of(object)
        }]);
        let output = fixture.render(body, &mut graph, Content::default());
        assert_eq!(output.text, expected, "{body}");
        assert!(output.refusals.is_empty(), "{body}: {:?}", output.refusals);
        let selected = graph.scopes.last().unwrap();
        assert_eq!(
            (selected.root, selected.from, selected.prev),
            (Link::Itself, Link::Itself, Link::Itself)
        );
        assert_eq!(
            (selected.random_seed, selected.random_counter),
            (1, 0x5ea6_ba9e)
        );
    }
}

#[test]
fn contested_owner_evaluates_scoped_operands() {
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![
            ScriptVariablesRow {
                seed: replay::rows::Recorded::Value(None),
                scope_kind: 1,
                scope_id: 100,
                entries: vec![ScriptVariableEntry {
                    name: b"chosen_state".to_vec(),
                    value_raw: 300_000,
                }],
            },
            ScriptVariablesRow {
                seed: replay::rows::Recorded::Value(None),
                scope_kind: 2,
                scope_id: 2,
                entries: vec![ScriptVariableEntry {
                    name: b"chosen_country".to_vec(),
                    value_raw: i64::from(u32::from_le_bytes(*b"BBB\0") | 0x4000_0000),
                }],
            },
        ]));
    let output = fixture.render(
        "add_contested_owner=var:chosen_state",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(output.text, "Add contested AAA/Three/AAA/AAA/AAA");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
    let output = fixture.render(
        "remove_contested_owner=var:chosen_country",
        &mut state_from_root(),
        Content::default(),
    );
    assert_eq!(output.text, "Remove contested BBB/Two/BBB/BBB/BBB");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}
ported!(
    add_popularity,
    "add_popularity={ideology=ROOT popularity=-0.05}",
    "Popularity Noun AAA: -0.05\n",
    fresh()
);
ported!(
    set_party_name,
    "set_party_name={ideology=SYN_IDEOLOGY long_name=SYN_LONG name=SYN_PARTY}",
    "Party name Ideology AAA => Long party AAA\n",
    fresh()
);
ported!(
    set_party_name_short_name_does_not_fill_long_name,
    "set_party_name={ideology=SYN_IDEOLOGY name=SYN_PARTY}",
    "Party name Ideology AAA => \n",
    fresh()
);

#[test]
fn absent_ideology_uses_native_null_object_without_evaluating_an_operand() {
    let (mut fixture, _folder) = fixture();
    // An absent specifier is not numeric zero, even if that ID names a group.
    if let TableView::Complete(TableRows::PoliticalParties(parties)) =
        &mut *fixture.frame.tables[TableId::PoliticalParties.index()]
    {
        parties[0].id = 0;
    }
    *fixture.frame.tables[TableId::ScriptVariables.index()] = TableView::Unavailable {
        reason: "absent ideology has no variable object".into(),
    };
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    for (class, key, body) in [
        (
            "CAddPopularityEffect",
            "add_popularity",
            "add_popularity={popularity=0.03}",
        ),
        (
            "CSetPartyNameEffect",
            "set_party_name",
            "set_party_name={name=SYN_PARTY}",
        ),
        (
            "CSetPartyRuleEffect",
            "set_party_rule",
            "set_party_rule={can_send_volunteers=yes}",
        ),
    ] {
        let output = fixture.render(body, &mut fresh(), content.view(None));
        assert_eq!(output.text, "");
        assert_eq!(
            output.refusals,
            BTreeMap::from([(
                format!(
                    "synthetic effect {class}: key {key}: ideology database null-object identity (+0x78, 1402EE8E0) is unrecorded"
                ),
                1,
            )])
        );
    }
}
ported!(
    set_cosmetic_tag,
    "set_cosmetic_tag=SYN_COSMETIC",
    "Cosmetic alpha => Costume AAA AAA\n",
    fresh()
);
ported!(
    activate_shine_on_focus,
    "activate_shine_on_focus=SYN_FOCUS",
    "",
    fresh()
);
ported!(
    deactivate_shine_on_focus,
    "deactivate_shine_on_focus=SYN_FOCUS",
    "",
    fresh()
);
ported!(
    set_rule,
    "set_rule={can_send_volunteers=yes}",
    "Rules AAA:[yes] Volunteers.\n",
    fresh()
);
ported!(
    set_party_rule,
    "set_party_rule={ideology=SYN_IDEOLOGY desc=SYN_DESC can_create_factions=no}",
    "Rules Ideology AAA:[no] Factions.\n",
    fresh()
);

ported!(
    remove_relation_rule_override,
    "remove_relation_rule_override={target=BBB can_send_volunteers=yes}",
    "Unrelation Alpha->Beta:[yes]Volunteers",
    fresh()
);
ported!(
    explicit_relation_rule_override,
    "add_relation_rule_override={target=BBB can_send_volunteers=no}",
    "Relation Alpha->Beta:[no]Volunteers",
    fresh()
);
refused!(
    remove_relation_rule_does_not_accept_add_only_usage_desc,
    "remove_relation_rule_override={target=BBB usage_desc=SYN_DESC can_send_volunteers=yes}",
    "unknown relation rule usage_desc (1403AF060/1403B4150)"
);

#[test]
fn named_party_effect_keeps_plain_text_without_unrelated_map_or_variable_inputs() {
    let mut fixture = Fixture::new();
    let (catalogue, _folder) = folder_catalogue(&[
        (
            "common/ideologies/synthetic.txt",
            "ideologies={plain_group={}}",
        ),
        (
            "common/defines/synthetic.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "localisation/plain_l_english.yml",
            "\u{feff}l_english:\n plain_group:0 \"Plain group\"\n PLAIN_NAME:0 \"Plain name\"\n POLITICS_PARTY_NAME_CHANGE:0 \"Party $IDEOLOGY$ => $NAME$\"\n",
        ),
    ]);
    fixture.catalogue = catalogue;
    *fixture.frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 100,
            id: 20,
            token: b"plain_group".to_vec(),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "unrelated province read failed".into(),
    };
    *fixture.frame.tables[TableId::ScriptVariables.index()] = TableView::Unavailable {
        reason: "cached pointer opens no variable read".into(),
    };
    let names = fixture.catalogue.content.layers.key_names().unwrap();
    let output = fixture.render(
        "set_party_name={ideology=plain_group long_name=PLAIN_NAME}",
        &mut fresh(),
        Content {
            key_names: Some(&names),
            ..Default::default()
        },
    );
    assert_eq!(output.text, "Party Plain group => Plain name\n");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}
