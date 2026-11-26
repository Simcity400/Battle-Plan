use super::*;
use crate::fixtures::{Folder, folder_catalogue};
use crate::effect_description::tests::{Fixture, fresh};
use crate::scripted_text::{EventObject, EventScope, Link};
use replay_data::rows::*;
use replay_frame::frame::TableView;
use crate::fixtures::game;

pub(super) fn fixture() -> (Fixture, Folder) {
    fixture_with(&[])
}

pub(super) fn fixture_with(extra: &[(&str, &str)]) -> (Fixture, Folder) {
    let mut fixture = Fixture::new();
    let mut files = vec![
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
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23}}",
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
                " FIXTURE_VOLUNTEERS:0 \"Volunteers\"\n FIXTURE_FACTIONS:0 \"Factions\"\n",
                " AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n CCC:0 \"Gamma\"\n",
                " AAA_DEF:0 \"alpha\"\n BBB_DEF:0 \"beta\"\n CCC_DEF:0 \"gamma\"\n",
                " EFFECT_ADD_CORE_OF:0 \"Core $COUNTRY$ [Root.GetTag]\"\n EFFECT_REMOVE_CORE_OF:0 \"Uncore $COUNTRY$ [Root.GetTag]\"\n",
                " EFFECT_ADD_CLAIM_BY:0 \"Claim @$TAG$$COUNTRY$ [Root.GetTag]\"\n EFFECT_REMOVE_CLAIM_BY:0 \"Unclaim @$TAG$$COUNTRY$ [Root.GetTag]\"\n",
                " EFFECT_LOAD_OOB:0 \"Load $NAME$ [Root.GetTag]\"\n EFFECT_SET_OOB:0 \"Set $NAME$ [Root.GetTag]\"\n",
                " SYN_TREE:0 \"Tree [Root.GetTag]\"\n EFFECT_LOAD_FOCUS_TREE:0 \"Use $TREE_NAME$\"\n",
                " EFFECT_RELEASE_NATION:0 \"Release $COUNTRY$ [Root.GetTag]\"\n CHANGE_TAG_EFFECT:0 \"Change $COUNTRY$ [Root.GetTag]\"\n",
                " EFFECT_ADD_THREAT:0 \"Add $THREAT|2$\"\n EFFECT_REMOVE_THREAT:0 \"Remove $THREAT|2$\"\n",
                " EFFECT_SEND_EMBARGO:0 \"Embargo $COUNTRY$\"\n EFFECT_BREAK_EMBARGO:0 \"Unembargo $COUNTRY$\"\n",
                " EFFECT_SCOPED_SOUND_DESC:0 \"Sound $SOUND_NAME$ for $COUNTRY$\"\n EFFECT_SOUND_DESC:0 \"Sound $SOUND_NAME$\"\n SYN_CHIME:0 \"Localised chime\"\n",
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
    ];
    files.extend_from_slice(extra);
    let (catalogue, folder) = folder_catalogue(&files);
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
            seed: replay_data::rows::Recorded::Value(None),
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

fn state_from_root() -> EventScopes {
    let mut state = EventScope::of(EventObject::State(2));
    state.root = Link::Scope(1);
    EventScopes::new(vec![state, EventScope::of(EventObject::Country(1))])
}

#[test]
fn politics_description_cases() {
    game::enter();
    let (fixture, _folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    for (case, body, expected, scopes) in [
        (
            "fixture_empty_scalar",
            "fixture_empty_scalar=\"synthetic log\"",
            "",
            fresh as fn() -> EventScopes,
        ),
        (
            "fixture_grant_core",
            "fixture_grant_core=ROOT",
            "Core Alpha [Root.GetTag]\n",
            state_from_root,
        ),
        (
            "fixture_grant_claim",
            "fixture_grant_claim=ROOT",
            "Claim \u{12}AAAAlpha [Root.GetTag]\n",
            state_from_root,
        ),
        (
            "fixture_load_roster",
            "fixture_load_roster=\"SYN_OOB\"",
            "Load SYN_OOB AAA",
            fresh,
        ),
        (
            "fixture_focus_tree",
            "fixture_focus_tree={keep_completed=no tree=SYN_TREE}",
            "Use Tree AAA\n",
            fresh,
        ),
        ("fixture_release_country", "fixture_release_country=BBB", "Release Beta AAA\n", fresh),
        (
            "fixture_revoke_core",
            "fixture_revoke_core=BBB",
            "Uncore Beta [Root.GetTag]\n",
            state_from_root,
        ),
        (
            "fixture_named_threat",
            "fixture_named_threat={name=SYN_REASON threat=4.35}",
            "Add 4.35\n",
            fresh,
        ),
        (
            "fixture_change_country_tag",
            "fixture_change_country_tag=BBB",
            "Change Alpha AAA\n",
            fresh,
        ),
        (
            "fixture_revoke_claim",
            "fixture_revoke_claim=ROOT",
            "Unclaim \u{12}AAAAlpha [Root.GetTag]\n",
            state_from_root,
        ),
        (
            "fixture_found_agency",
            "fixture_found_agency=yes",
            "Agency AAA\n",
            fresh,
        ),
        (
            "fixture_focus_cost",
            "fixture_focus_cost={cost=-11 focus={SYN_FOCUS_TWO SYN_FOCUS_ONE}}",
            "Extend 11 [Root.GetTag]\n Second\n First AAA\n",
            fresh,
        ),
        ("fixture_impose_embargo", "fixture_impose_embargo=BBB", "Embargo Beta\n", fresh),
        (
            "fixture_country_sound",
            "fixture_country_sound=SYN_CHIME",
            "Sound SYN_CHIME for Alpha",
            fresh,
        ),
        (
            "fixture_country_sound",
            "fixture_country_sound=\"SYN_CHIME\"",
            "Sound SYN_CHIME for Alpha",
            fresh,
        ),
        (
            "fixture_release_subject",
            "fixture_release_subject=BBB",
            "Release Beta AAA\n",
            fresh,
        ),
        (
            "fixture_sound_cue",
            "fixture_sound_cue=SYN_CHIME",
            "Sound SYN_CHIME",
            fresh,
        ),
        ("fixture_major_status", "fixture_major_status=yes", "Major Alpha\n", fresh),
        (
            "fixture_sound_cue",
            "fixture_sound_cue=\"SYN_CHIME\"",
            "Sound SYN_CHIME",
            fresh,
        ),
        (
            "fixture_withdraw_volunteers",
            "fixture_withdraw_volunteers=ROOT",
            "Volunteers alpha\n",
            fresh,
        ),
        (
            "fixture_call_election",
            "fixture_call_election=BBB",
            "Election Beta\n",
            fresh,
        ),
        (
            "fixture_political_points",
            "fixture_political_points=1.75",
            "Power 1.75\n",
            fresh,
        ),
        (
            "fixture_organisation_title",
            "fixture_organisation_title=SYN_MIO",
            "Rename Foundry AAA\n",
            fresh,
        ),
        (
            "fixture_lift_embargo",
            "fixture_lift_embargo=ROOT",
            "Unembargo Alpha\n",
            fresh,
        ),
        ("fixture_organisation_icon", "fixture_organisation_icon=SYN_ICON", "Icon AAA\n", fresh),
        (
            "fixture_free_leader",
            "fixture_free_leader=yes",
            "Free [Root.GetTag]",
            fresh,
        ),
        (
            "fixture_free_generals",
            "fixture_free_generals=ROOT",
            "Generals alpha [Root.GetTag]",
            fresh,
        ),
        (
            "fixture_withdraw_attache",
            "fixture_withdraw_attache=ROOT",
            "Attache alpha\n",
            fresh,
        ),
        ("fixture_replace_roster", "fixture_replace_roster=\"SYN:OOB\"", "Set SYN:OOB AAA", fresh),
        (
            "negative_and_zero_named_threat",
            "fixture_named_threat={threat=-1.25 name=SYN_REASON} fixture_named_threat={threat=0}",
            "Remove 1.25\nRemove 0.00\n",
            fresh,
        ),
        (
            "minor_uses_bool_not_country_selector",
            "fixture_major_status=no fixture_major_status=\"yes\" fixture_major_status=invalid",
            "Minor Alpha\nMajor Alpha\nMinor Alpha\n",
            fresh,
        ),
        (
            "single_focus_cost",
            "fixture_focus_cost={focus=SYN_FOCUS_ONE cost=0x7}",
            "Reduce First AAA 7\n",
            fresh,
        ),
        (
            "focus_cost_appends_repeated_lists",
            "fixture_focus_cost={focus=SYN_FOCUS_ONE focus={SYN_FOCUS_TWO} cost=9}",
            "Reduce 9 [Root.GetTag]\n First AAA\n Second\n",
            fresh,
        ),
        (
            "empty_focus_cost",
            "fixture_focus_cost={focus={} cost=7}",
            "",
            fresh,
        ),
        (
            "captured_generals_block",
            "fixture_free_generals={province=2 target=BBB}",
            "Generals beta [Root.GetTag]",
            fresh,
        ),
        (
            "captured_generals_ignores_captured_by_field",
            "fixture_free_generals={captured_by=BBB}",
            "",
            fresh,
        ),
        (
            "fixture_claim_occupation",
            "fixture_claim_occupation=1",
            "Add contested AAA/One/AAA/AAA/AAA",
            fresh,
        ),
        (
            "fixture_revoke_occupation",
            "fixture_revoke_occupation=CCC",
            "Remove contested CCC/Two/CCC/CCC/CCC",
            state_from_root,
        ),
        (
            "fixture_popularity_shift",
            "fixture_popularity_shift={ideology=ROOT popularity=-0.05}",
            "Popularity Noun AAA: -0.05\n",
            fresh,
        ),
        (
            "fixture_party_title",
            "fixture_party_title={ideology=SYN_IDEOLOGY long_name=SYN_LONG name=SYN_PARTY}",
            "Party name Ideology AAA => Long party AAA\n",
            fresh,
        ),
        (
            "set_party_name_short_name_does_not_fill_long_name",
            "fixture_party_title={ideology=SYN_IDEOLOGY name=SYN_PARTY}",
            "Party name Ideology AAA => \n",
            fresh,
        ),
        (
            "fixture_cosmetic_identity",
            "fixture_cosmetic_identity=SYN_COSMETIC",
            "Cosmetic alpha => Costume AAA AAA\n",
            fresh,
        ),
        (
            "fixture_focus_highlight",
            "fixture_focus_highlight=SYN_FOCUS",
            "",
            fresh,
        ),
        (
            "fixture_end_focus_highlight",
            "fixture_end_focus_highlight=SYN_FOCUS",
            "",
            fresh,
        ),
        (
            "fixture_country_rule",
            "fixture_country_rule={fixture_volunteers=yes}",
            "Rules AAA:[yes] Volunteers.\n",
            fresh,
        ),
        (
            "fixture_party_rule",
            "fixture_party_rule={ideology=SYN_IDEOLOGY desc=SYN_DESC fixture_factions=no}",
            "Rules Ideology AAA:[no] Factions.\n",
            fresh,
        ),
        (
            "fixture_revoke_override",
            "fixture_revoke_override={target=BBB fixture_volunteers=yes}",
            "Unrelation Alpha->Beta:[yes]Volunteers",
            fresh,
        ),
        (
            "explicit_relation_rule_override",
            "fixture_grant_override={target=BBB fixture_volunteers=no}",
            "Relation Alpha->Beta:[no]Volunteers",
            fresh,
        ),
    ] {
        let output = fixture.render(body, &mut scopes(), content.view(None));
        assert_eq!(output.text, expected, "{case}: {:?}", output.refusals);
        assert!(output.refusals.is_empty(), "{case}: {:?}", output.refusals);
    }
}

#[test]
fn change_tag_from_state_keeps_the_source_name_refusal_counted() {
    game::enter();
    let (fixture, _folder) = fixture();
    let output = fixture.render(
        "fixture_change_country_tag=THIS fixture_change_country_tag=THIS",
        &mut state_from_root(),
        Content::default(),
    );
    assert_eq!(output.text, "");
    assert_eq!(
        output.refusals,
        BTreeMap::from([(
            "synthetic effect fixture_change_country_tag: key fixture_change_country_tag: current scope has no country"
                .to_owned(),
            2,
        )]),
    );
}

#[test]
fn agency_feature_and_created_gates_limit_recorded_reads() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![
            SimulationFeatureRow::default(),
        ]));
    *fixture.frame.tables[TableId::IntelligenceAgencies.index()] = TableView::Unavailable {
        reason: "not needed".into(),
    };
    let output = fixture.render(
        "fixture_found_agency=yes",
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
        "fixture_found_agency=yes",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(output, Description::default());
}

#[test]
fn missing_agency_facts_are_named_and_counted() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::SimulationFeatures.index()] = TableView::Unavailable {
        reason: "missing mask".into(),
    };
    let output = fixture.render(
        "fixture_found_agency=yes fixture_found_agency=yes",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(output.text, "");
    assert_eq!(output.refusals.values().sum::<usize>(), 2);
    assert!(
        output
            .refusals
            .keys()
            .all(|why| why.contains("fixture_found_agency") && why.contains("missing mask"))
    );
}

#[test]
fn empty_agency_own_text_uses_core_projection_and_child_walk() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::IntelligenceAgencies.index()] =
        TableView::Complete(TableRows::IntelligenceAgencies(vec![
            IntelligenceAgencyRow {
                country: 100,
                created: 1,
                ..Default::default()
            },
        ]));
    let output = fixture.render(
        "fixture_found_agency={name=SYN_NAME icon=SYN_ICON fixture_points=2}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(output.text, "XP2\n");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn country_targets_resolve_a_late_synthetic_country() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    let countries = (1..=24)
        .map(|id| CountryRow {
            id,
            canonical_slot: id,
            tag: if id == 24 {
                b"BBB".to_vec()
            } else {
                format!("S{id}").into_bytes()
            },
            ..Default::default()
        })
        .collect();
    fixture.set_countries(countries);
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(24))]);
    let output = fixture.render("fixture_call_election=ROOT", &mut scopes, Content::default());
    assert_eq!(output.text, "Election Beta\n");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
    for (key, action) in [
        ("fixture_claim_occupation", "Add"),
        ("fixture_revoke_occupation", "Remove"),
    ] {
        let output = fixture.render(&format!("{key}=2"), &mut scopes, Content::default());
        assert_eq!(
            output.text,
            format!("{action} contested BBB/Two/BBB/BBB/BBB")
        );
        assert!(output.refusals.is_empty(), "{:?}", output.refusals);
    }
}

macro_rules! refused {
    ($name:ident, $body:expr, $why:expr) => {
        #[test]
        fn $name() {
            game::enter();
            let (fixture, _folder) = fixture();
            let body = $body;
            let key = body.split('=').next().unwrap();
            let output = fixture.render(body, &mut fresh(), Content::default());
            assert_eq!(output.text, "");
            assert_eq!(
                output.refusals,
                BTreeMap::from([(format!("synthetic effect {key}: key {key}: {}", $why), 1)])
            );
        }
    };
}

#[test]
fn contested_owner_uses_both_operand_branches_and_fresh_links() {
    game::enter();
    let (fixture, _folder) = fixture();
    for (body, object, expected) in [
        (
            "fixture_claim_occupation=BBB",
            EventObject::State(2),
            "Add contested BBB/Two/BBB/BBB/BBB",
        ),
        (
            "fixture_revoke_occupation=3",
            EventObject::Country(2),
            "Remove contested BBB/Three/BBB/BBB/BBB",
        ),
        (
            "fixture_revoke_occupation=BBB",
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
    game::enter();
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![
            ScriptVariablesRow {
                seed: replay_data::rows::Recorded::Value(None),
                scope_kind: 1,
                scope_id: 100,
                entries: vec![ScriptVariableEntry {
                    name: b"chosen_state".to_vec(),
                    value_raw: 300_000,
                }],
            },
            ScriptVariablesRow {
                seed: replay_data::rows::Recorded::Value(None),
                scope_kind: 2,
                scope_id: 2,
                entries: vec![ScriptVariableEntry {
                    name: b"chosen_country".to_vec(),
                    value_raw: i64::from(u32::from_le_bytes(*b"BBB\0") | 0x4000_0000),
                }],
            },
        ]));
    let output = fixture.render(
        "fixture_claim_occupation=var:chosen_state",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(output.text, "Add contested AAA/Three/AAA/AAA/AAA");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
    let output = fixture.render(
        "fixture_revoke_occupation=var:chosen_country",
        &mut state_from_root(),
        Content::default(),
    );
    assert_eq!(output.text, "Remove contested BBB/Two/BBB/BBB/BBB");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn absent_ideology_uses_null_object_without_evaluating_an_operand() {
    game::enter();
    let (mut fixture, _folder) = fixture();
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
    for (key, body) in [
        ("fixture_popularity_shift", "fixture_popularity_shift={popularity=0.03}"),
        ("fixture_party_title", "fixture_party_title={name=SYN_PARTY}"),
        ("fixture_party_rule", "fixture_party_rule={fixture_volunteers=yes}"),
    ] {
        let output = fixture.render(body, &mut fresh(), content.view(None));
        assert_eq!(output.text, "");
        assert_eq!(
            output.refusals,
            BTreeMap::from([(
                format!(
                    "synthetic effect {key}: key {key}: ideology database null-object identity is unrecorded"
                ),
                1,
            )])
        );
    }
}

refused!(
    remove_relation_rule_does_not_accept_add_only_usage_desc,
    "fixture_revoke_override={target=BBB usage_desc=SYN_DESC fixture_volunteers=yes}",
    "unknown relation rule usage_desc"
);

#[test]
fn named_party_effect_keeps_plain_text_without_unrelated_map_or_variable_inputs() {
    game::enter();
    let mut fixture = Fixture::new();
    let (catalogue, _folder) = folder_catalogue(&[
        (
            "common/ideologies/synthetic.txt",
            "ideologies={plain_group={}}",
        ),
        (
            "common/defines/synthetic.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23}}",
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
        "fixture_party_title={ideology=plain_group long_name=PLAIN_NAME}",
        &mut fresh(),
        Content {
            key_names: Some(&names),
            ..Default::default()
        },
    );
    assert_eq!(output.text, "Party Plain group => Plain name\n");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}
