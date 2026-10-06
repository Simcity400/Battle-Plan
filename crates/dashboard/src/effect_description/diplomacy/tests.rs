use super::*;
use crate::effect_description::tests::{Fixture, synthetic_countries};
use crate::scripted_text::{EventObject, EventScope};
use EventObject::{Country as C, State as S};
use replay::{frame::TableView, rows::*};

#[test]
fn diplomacy_native_output_branches() {
    let mut fixture = Fixture::new();
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n CCC:0 \"Gamma\"\n",
                " AAA_DEF:0 \"alpha\"\n BBB_DEF:0 \"beta\"\n CCC_DEF:0 \"éclair\"\n",
                " ADD_AUTONOMY_SCORE_DESC:0 \"score $VALUE|0$/$COUNTRY|0$\"\n",
                " ADD_AUTONOMY_SCORE_DETAILED_DESC:0 \"detailed $VALUE|0$\"\n",
                " EFFECT_BECOME_EXILED_IN:0 \"host $HOSTNAMEDEF$ [Root.GetTag]\"\n",
                " EFFECT_END_EXILE:0 \"return $COUNTRY$\"\n",
                " EFFECT_END_PUPPET:0 \"free $NAME$/$COUNTRY$\"\n",
                " EFFECT_MAKE_PUPPET:0 \"subject $COUNTRY$/$TARGET$\"\n",
                " EFFECT_RELEASE_PUPPET:0 \"release $COUNTRY$\"\n",
                " synthetic:0 \"Level [Root.GetTag]\"\n synthetic_other:0 \"Other\"\n",
                " autonomy_free:0 \"Defined free\"\n",
                " EFFECT_RELEASE_AUTONOMY:0 \"Autonomy $COUNTRY$/$NAME$\"\n",
                " EFFECT_SET_AUTONOMY:0 \"Set $COUNTRY$/$TARGET$/$AUTONOMY$\"\n",
                " EFFECT_AUTONOMY_FREE:0 \"Free $COUNTRY$\"\n",
                " EFFECT_REMOVE_EXILE_TAG:0 \"exile [This.GetName]\"\n",
                " SYN_NAME:0 \"Charter [This.GetTag]\"\n ROOT:0 \"Root literal\"\n",
                " EFFECT_CREATE_FACTION:0 \"Create $NAME$\"\n",
                " EFFECT_DISMANTLE_FACTION:0 \"Dismantle $NAME$\"\n",
                " EFFECT_REMOVE_FROM_FACTION:0 \"Remove $COUNTRY$\"\n",
                " EFFECT_REMOVE_FROM_FACTION_NAME:0 \"Remove $COUNTRY$/$FACTION$\"\n",
                " EFFECT_SET_FACTION_NAME:0 \"Rename $OLDNAME$/$NEWNAME$\"\n",
                " EFFECT_SET_FACTION_SPYMASTER:0 \"Spy $NAME$\"\n",
                " SHORT_TERM:0 \"Short\"\n MEDIUM_TERM:0 \"Medium\"\n LONG_TERM:0 \"Long\"\n",
                " FACTION_ADD_EXTRA_GOAL_SLOT:0 \"Slots $FACTION$/$CATEGORY$/$VAL$\"\n",
                " FACTION_ADD_EXTRA_GOAL_SLOT_NO_FACTION:0 \"SlotsOutside $CATEGORY$/$VAL$\"\n",
                " ADD_FACTION_INITIATIVE_EFFECT:0 \"Initiative $FACTION$/$AMOUNT|2$\"\n",
                " ADD_FACTION_INITIATIVE_EFFECT_NO_FACTION:0 \"InitiativeOutside $AMOUNT|2$\"\n",
                " FACTION_ACCESS_TAB_MILITARY:0 \"Military\"\n",
                " FACTION_ACCESS_TAB_UNLOCK:0 \"Unlock $NAME$\"\n FACTION_ACCESS_TAB_LOCK:0 \"Lock $NAME$\"\n",

                " EFFECT_ADD_CIVIL_WAR_TARGET:0 \"Civil $COUNTRY$\"\n",
                " EFFECT_REMOVE_CIVIL_WAR_TARGET:0 \"Uncivil $COUNTRY$\"\n",
                " EFFECT_GIVE_GUARANTEE:0 \"Guarantee $COUNTRY$\"\n",
                " EFFECT_GIVE_MILITARY_ACCESS:0 \"Access $COUNTRY$\"\n",
                " EFFECT_MARKET_GIVE_MARKET_ACCESS:0 \"Market $COUNTRY$\"\n",
                " EFFECT_REMOVE_WARGOAL:0 \"Goal $NAME$/$COUNTRY$\"\n",
                " EFFECT_ADD_TO_WAR:0 \"War $COUNTRY1$/$COUNTRY2$\"\n",
                " EFFECT_SET_WAR_SUPPORT:0 \"Support $VAL|0$\"\n",
                " EFFECT_ENACT_RELATION:0 \"Enact $NAME$/$WHO$\"\n",
                " EFFECT_CANCEL_RELATION:0 \"Cancel $NAME$/$WHO$\"\n",
                " DIPLOMACY_NAVAL_BLOCKADE_TITLE:0 \"Blockade\"\n",
                " DIPLOMACY_AIR_BASE_ACCESS:0 \"Air\"\n",
                " DIPLOMACY_GUARANTEE_TITLE:0 \"Guarantee\"\n",
                " DIPLOMACY_MILACC:0 \"Access\"\n",
                " DIPLOMACY_IMPROVERELATION:0 \"Improve\"\n",
                " DIPLOMACY_NONAGGRESSIONPACT:0 \"Pact\"\n",
                " DIPLOMACY_EMBARGO_TITLE:0 \"Embargo\"\n",
                " DIPLOMACY_INTERNATIONAL_MARKET_ACCESS_RIGHTS:0 \"Market\"\n",
                " DIPLOMACY_ATWAR:0 \"Atwar\"\n",
                " DIPLOMACY_SEND_ATTACHE:0 \"Attache\"\n",
                " DIPLOMACY_DOCKING_RIGHTS:0 \"Docking\"\n",
                " DIPLOMACY_LICACC:0 \"Licence\"\n",
                " EFFECT_BORDER_WAR:0 \"Border $STATE$ [This.GetName]\"\n",
                " EFFECT_END_BORDER_WAR:0 \"End $STATE$\"\n",
                " EFFECT_CANCEL_BORDER_WAR:0 \"Cancel $STATE1$/$STATE2$\"\n",
                " EFFECT_FINALIZE_BORDER_WAR:0 \"Final $STATE1$/$STATE2$\"\n",
                " EFFECT_UPDATE_BORDER_WAR:0 \"Update $STATE1$/$STATE2$\"\n",
                " EFFECT_UPDATE_BORDER_WAR_COMBAT_WIDTH:0 \"Width $NUM|2$\"\n",
                " EFFECT_UPDATE_BORDER_WAR_CHANGE_STATE:0 \"Change $VAL$\"\n",
                " EFFECT_UPDATE_BORDER_WAR_NUM_UNITS:0 \"Units $NUM|0$\"\n",
                " EFFECT_UPDATE_BORDER_WAR_MODIFIER:0 \"Modifier $NUM|2$\"\n",
                " YES:0 \"yes [Root.GetTag]\"\n NO:0 \"no\"\n",
            ),
        ),
    ]);
    fixture.catalogue = catalogue;
    fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    let mut autonomy_states = installed_content::autonomy::AutonomyStates {
        states: ["synthetic", "synthetic_other"]
            .into_iter()
            .map(|id| installed_content::autonomy::AutonomyState {
                id: id.into(),
                min_freedom_level: None,
            })
            .collect(),
    };
    *fixture.frame.tables[TableId::Autonomy.index()] =
        TableView::Complete(TableRows::Autonomy(vec![
            AutonomyRow {
                subject: 100,
                lower_score: 200_000,
                upper_score: 1_000_000,
                ..Default::default()
            },
            AutonomyRow {
                subject: 200,
                lower_score: -1,
                upper_score: i64::MAX,
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(
            (11..=15)
                .map(|id| CharacterRow {
                    id,
                    country: 100,
                    ..Default::default()
                })
                .collect(),
        ));
    let leader_type = engine::cunit_leader::REFERENCE_TYPE;
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(
            [(7, 11, 200), (8, 12, 0), (9, 13, -1)]
                .into_iter()
                .map(|(id, character_id, exile_country)| UnitLeaderRow {
                    id,
                    reference_type: leader_type,
                    character_id,
                    exile_country,
                    ..Default::default()
                })
                .collect(),
        ));
    *fixture.frame.tables[TableId::CharacterOperatives.index()] =
        TableView::Complete(TableRows::CharacterOperatives(vec![CharacterOperativeRow {
            character: 14,
            reference: (20_u64 << 32) | u64::from(leader_type),
            resolved: true,
            category: 3,
            ..Default::default()
        }]));
    let leader = EventObject::UnitLeader((7_u64 << 32) | u64::from(leader_type));
    *fixture.frame.tables[TableId::Factions.index()] =
        TableView::Complete(TableRows::Factions(vec![FactionRow {
            id: 11,
            name: b"Union".to_vec(),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![
            FactionMemberRow {
                country: 100,
                faction: 11,
                ordinal: 0,
            },
            FactionMemberRow {
                country: 200,
                faction: 11,
                ordinal: 1,
            },
        ]));
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 100,
            entries: vec![
                ScriptVariableEntry {
                    name: b"ratio".to_vec(),
                    value_raw: 50_000,
                },
                ScriptVariableEntry {
                    name: b"syn_slots".to_vec(),
                    value_raw: -150_000,
                },
                ScriptVariableEntry {
                    name: b"syn_initiative".to_vec(),
                    value_raw: 125_999,
                },
            ],
        }]));
    // Synthetic cases cover all 44 assigned vtables and native own-text branches.
    for (body, object, mask, expected, refusal) in [
        ("add_autonomy_score={value=9}", EventObject::Country(1), 0, "", None),
        ("add_autonomy_score={value=9}", EventObject::Country(1), 1, "", None),
        (
            "add_autonomy_score={value=-2 localization=unused}",
            EventObject::Country(1),
            2,
            "score -2/1\n",
            None,
        ),
        ("add_autonomy_score={value=3}", EventObject::Country(1), 4, "score 3/1\n", None),
        ("add_autonomy_score={}", EventObject::Country(1), 8, "score 0/1\n", None),
        ("add_autonomy_score={value=4}", EventObject::Country(0), 2, "detailed 4\n", None),
        ("add_autonomy_score={value=4}", EventObject::State(1), 2, "detailed 4\n", None),
        ("add_autonomy_score={value=4}", EventObject::Character(11), 2, "detailed 4\n", None),
        ("add_autonomy_score={value=4}", leader, 2, "detailed 4\n", None),
        ("add_autonomy_score={value=var:ratio}", EventObject::Country(1), 2, "score 0/1\n", None),
        ("add_autonomy_score={value=3 value=var:ratio}", C(1), 2, "score 3/1\n", None),
        ("add_autonomy_score={value=var:ratio value=-2}", C(1), 2, "score -2/1\n", None),
        ("add_autonomy_ratio={value=var:missing}", EventObject::Country(1), 0, "", None),
        ("add_autonomy_ratio={value=var:ratio}", EventObject::Country(1), 2, "score 4/1\n", None),
        ("add_autonomy_ratio={value=-0.5}", EventObject::Country(1), 2, "score -4/1\n", None),
        ("add_autonomy_ratio={}", EventObject::Country(1), 2, "score 0/1\n", None),
        ("add_autonomy_ratio={value=0.5}", EventObject::Country(2), 2, "score 0/1\n", None),
        ("add_autonomy_ratio={value=var:missing}", EventObject::Country(3), 2, "", None),
        ("add_autonomy_ratio={value=var:missing}", EventObject::State(1), 2, "", None),
        ("add_autonomy_ratio={value=var:missing}", EventObject::Character(11), 2, "", None),
        (
            "become_exiled_in={target=BBB legitimacy=var:missing}",
            EventObject::Country(1),
            2,
            "host beta AAA\n",
            None,
        ),
        ("become_exiled_in={target=event_target:absent}", EventObject::Country(1), 2, "", None),
        ("end_exile=BBB", EventObject::Country(1), 2, "return Beta", None),
        ("end_exile=CCC", EventObject::Country(1), 2, "return éclair", None),
        ("end_exile=event_target:absent", EventObject::Country(1), 2, "", None),
        ("end_puppet=BBB", EventObject::Country(1), 2, "free Alpha/Beta\n", None),
        ("end_puppet=event_target:absent", EventObject::Country(1), 2, "", None),
        ("puppet=BBB", EventObject::Country(1), 2, "subject Alpha/Beta\n", None),
        (
            "puppet={target=BBB end_wars=no end_civil_wars=yes}",
            EventObject::Country(1),
            2,
            "subject Alpha/Beta\n",
            None,
        ),
        ("puppet={target=event_target:absent}", EventObject::Country(1), 2, "", None),
        ("release_autonomy={target=BBB}", EventObject::Country(1), 2, "", None),
        (
            "release_autonomy={target=event_target:absent autonomy_state=synthetic}",
            EventObject::Country(1),
            2,
            "",
            None,
        ),
        (
            "release_autonomy={target=BBB autonomy_state=synthetic freedom_level=var:missing}",
            EventObject::Country(1),
            2,
            "Autonomy Beta/Level AAA\n",
            None,
        ),
        (
            "set_autonomy={target=BBB autonomy_state=synthetic freedom_level=var:missing}",
            EventObject::Country(1),
            2,
            "Set Alpha/Beta/Level AAA\n",
            None,
        ),
        (
            "set_autonomy={target=event_target:absent autonomy_state=synthetic}",
            EventObject::Country(1),
            2,
            "",
            None,
        ),
        (
            "release_autonomy={target=BBB autonomous_state=synthetic_other}",
            C(1),
            2,
            "Autonomy Beta/Other\n",
            None,
        ),
        (
            "release_autonomy={target=BBB autonomy_state=synthetic_other autonomous_state=synthetic}",
            C(1),
            2,
            "Autonomy Beta/Level AAA\n",
            None,
        ),
        (
            "release_autonomy={target=BBB autonomous_state=synthetic autonomy_state=synthetic_other}",
            C(1),
            2,
            "Autonomy Beta/Other\n",
            None,
        ),
        (
            "release_autonomy={target=BBB autonomy_state=synthetic autonomy_state=missing}",
            C(1),
            2,
            "",
            Some("not defined"),
        ),
        (
            "release_autonomy={target=event_target:absent autonomy_state=missing}",
            C(1),
            2,
            "",
            Some("not defined"),
        ),
        (
            "release_autonomy={target=BBB autonomy_state=Synthetic}",
            C(1),
            2,
            "",
            Some("not defined"),
        ),
        ("release_autonomy={target=BBB autonomy_state=\"\"}", C(1), 2, "", None),
        (
            "set_autonomy={target=BBB autonomous_state=synthetic_other autonomy_state=synthetic}",
            C(1),
            2,
            "Set Alpha/Beta/Level AAA\n",
            None,
        ),
        (
            "set_autonomy={target=BBB autonomy_state=synthetic autonomous_state=synthetic_other}",
            C(1),
            2,
            "Set Alpha/Beta/Other\n",
            None,
        ),
        ("set_autonomy={target=BBB autonomy_state=missing}", C(1), 2, "", Some("not defined")),
        ("set_autonomy={target=event_target:absent autonomy_state=missing}", C(1), 2, "", None),
        ("set_autonomy={target=BBB}", C(1), 2, "Set Alpha/Beta/\n", None),
        (
            "set_autonomy={target=BBB autonomy_state=autonomy_free freedom_level=var:missing}",
            C(1),
            2,
            "Free beta\n",
            None,
        ),
        ("release_puppet=BBB", EventObject::Country(1), 2, "release Beta\n", None),
        ("release_puppet=event_target:absent", EventObject::Country(1), 2, "", None),
        ("release_puppet_on_controlled=BBB", EventObject::Country(1), 2, "release Beta\n", None),
        ("release_puppet_on_controlled=event_target:absent", EventObject::Country(1), 2, "", None),
        ("remove_exile_tag=no", EventObject::Character(11), 2, "exile [This.GetName]", None),
        ("remove_exile_tag=yes", leader, 2, "exile [This.GetName]", None),
        ("remove_exile_tag=yes", EventObject::Character(12), 2, "", None),
        ("remove_exile_tag=yes", EventObject::Character(13), 2, "", None),
        ("remove_exile_tag=yes", EventObject::Character(15), 2, "", None),
        ("remove_exile_tag=yes", EventObject::Character(14), 2, "", Some("+0xED4")),
        ("create_faction=SYN_NAME", C(1), 2, "Create Charter AAA\n", None),
        ("create_faction=ROOT", C(1), 2, "Create Root literal\n", None), // No selector read by vf6.
        ("create_faction=\"\"", C(1), 2, "", None),
        ("create_faction=SYN_NAME", S(0), 2, "", None),
        ("dismantle_faction=no", C(1), 2, "Dismantle Union\n", None), // Boolean ignored by vf7.
        ("dismantle_faction=yes", C(2), 2, "", None),
        ("dismantle_faction=yes", C(3), 2, "", None),
        ("leave_faction=ROOT", C(1), 2, "Remove alpha/Union\n", None), // Definite current name.
        ("leave_faction=yes", C(3), 2, "", None),
        ("leave_faction=yes", S(1), 2, "", None),
        ("remove_from_faction=CCC", C(1), 2, "Remove Gamma/Union\n", None), // No target-membership gate.
        ("remove_from_faction=AAA", C(3), 2, "Remove Alpha\n", None),
        ("remove_from_faction=AAA", S(1), 2, "", Some("current scope has no country")),
        ("remove_from_faction=CCC", C(2), 2, "", None),
        ("remove_from_faction=0", C(1), 2, "", None),
        ("set_faction_name=SYN_NAME", C(1), 2, "Rename Union/Charter AAA\n", None),
        ("set_faction_name=ROOT", C(1), 2, "Rename Union/\n", None), // vf6 selector clears new name.
        ("set_faction_name=SYN_NAME", C(3), 2, "", None),
        ("set_faction_spymaster=no", C(2), 2, "Spy Union\n", None), // Nonleader is allowed; boolean ignored.
        ("set_faction_spymaster=yes", C(3), 2, "", None),
        (
            "add_faction_goal_slot={category=short_term value=1.5}",
            C(1),
            2,
            "Slots Union/Short/2\n",
            None,
        ),
        (
            "add_faction_goal_slot={category=medium_term value=syn_slots}",
            C(1),
            2,
            "Slots Union/Medium/-2\n",
            None,
        ),
        (
            "add_faction_goal_slot={category=long_term value=-1.49999}",
            C(1),
            2,
            "Slots Union/Long/-1\n",
            None,
        ),
        (
            "add_faction_goal_slot={category=long_term value=2}",
            C(3),
            2,
            "SlotsOutside Long/2\n",
            None,
        ),
        (
            "add_faction_goal_slot={category=short_term category=\"short_term\" value=3 value=4}",
            C(1),
            2,
            "Slots Union//4\n",
            None,
        ),
        ("add_faction_goal_slot={}", S(0), 2, "SlotsOutside /0\n", None),
        (
            "add_faction_goal_slot={category=short_term value=2 give_guarantee=BBB}",
            C(1),
            2,
            "Slots Union/Short/2\nGuarantee Beta\n",
            None,
        ),
        ("add_faction_initiative=syn_initiative", C(1), 2, "Initiative Union/1.25", None),
        ("add_faction_initiative=-2.5", C(3), 2, "InitiativeOutside -2.50", None),
        ("add_faction_initiative=ROOT", C(1), 2, "Initiative Union/0.00", None),
        ("set_faction_military_unlocked=yes", S(0), 2, "Unlock Military", None), // No country/faction gate or LF.
        ("set_faction_military_unlocked=no", C(1), 2, "Lock Military", None),
        ("set_faction_military_unlocked=YES", C(1), 2, "Lock Military", None),
        ("set_faction_military_unlocked=\"yes\"", C(1), 2, "Unlock Military", None),
        (
            "add_faction_initiative=2 set_faction_military_unlocked=yes",
            C(1),
            2,
            "Initiative Union/2.00\nUnlock Military",
            None,
        ),

        ("add_civil_war_target=BBB", C(1), 2, "Civil Beta\n", None),
        ("remove_civil_war_target=CCC", C(1), 2, "Uncivil Gamma\n", None),
        ("give_guarantee=BBB", C(1), 2, "Guarantee Beta\n", None),
        ("give_military_access=BBB", C(1), 2, "Access Beta\n", None),
        ("give_market_access=BBB", C(1), 2, "Market beta\n", None),
        ("remove_wargoal={target=BBB type=all}", C(1), 2, "Goal Alpha/Beta\n", None),
        ("remove_wargoal={target=event_target:absent type=all}", C(1), 2, "", None),
        ("remove_wargoal={target=ROOT target=BBB}", C(1), 2, "", Some("+0x24")),
        ("add_to_war={enemy=BBB targeted_alliance=CCC}", C(1), 2, "War Beta/Gamma\n", None),
        (
            "add_to_war={enemy=ROOT.FROM targeted_alliance=PREV.ROOT}",
            C(1),
            2,
            "War Alpha/Alpha\n",
            None,
        ),
        (
            "add_to_war={enemy=\"root.from\" targeted_alliance=PREV}",
            S(1),
            2,
            "War Alpha/Alpha\n",
            None,
        ),
        (
            "add_to_war={enemy=ROOT enemy=FROM targeted_alliance=CCC}",
            C(1),
            2,
            "",
            Some("1403AD690"),
        ),
        (
            "add_to_war={enemy=ROOT.owner targeted_alliance=CCC}",
            C(1),
            2,
            "",
            Some("invalid scope-link component"),
        ),
        (
            "add_to_war={enemy=var:ratio targeted_alliance=CCC}",
            C(1),
            2,
            "",
            Some("country specifier"),
        ),
        ("set_war_support=0.25", C(1), 2, "Support 0\n", None),
        ("set_border_war=yes", S(1), 2, "Border One [This.GetName]\n", None),
        ("set_border_war=ROOT", S(1), 2, "End One\n", None),
        ("set_border_war=yes", C(1), 2, "", None),
        (
            "cancel_border_war={attacker=1 defender=2}",
            C(1),
            2,
            "Cancel One(\u{12}AAA)/Two(\u{12}AAA)\n",
            None,
        ),
        ("cancel_border_war={attacker=0 defender=2}", C(1), 2, "", None),
        ("finalize_border_war={attacker=1 defender=2}", C(1), 2, "", None),
        (
            "finalize_border_war={attacker=1 defender=2 attacker_win=yes defender_win=yes}",
            C(1),
            2,
            "Final One(\u{12}AAA)/Two(\u{12}AAA)\n",
            None,
        ),
        (
            "finalize_border_war={attacker=1 defender=2 defender_win=yes}",
            C(1),
            2,
            "Final Two(\u{12}AAA)/One(\u{12}AAA)\n",
            None,
        ),
        (
            "finalize_border_war={attacker=0 defender=2 attacker_win=yes}",
            C(1),
            2,
            "",
            Some("invalid state 0"),
        ),
        (
            "set_border_war_data={attacker=1 defender=2 combat_width=2.5 change_state_after_war=yes max_attackers=3 attacker_modifier=-0.25 max_defenders=4 max_units=var:missing modifier=var:missing}",
            C(1),
            2,
            "Update One/Two\nWidth 2.50\nChange yes AAA\n\t\u{11}HOne\u{11}!:\nUnits 3\nModifier -0.25\n\t\u{11}HTwo\u{11}!:\nUnits 4\n",
            None,
        ),
        ("set_border_war_data={attacker=0 defender=2 combat_width=var:missing}", C(1), 2, "", None),
        (
            "diplomatic_relation={country=BBB relation=military_access active=no}",
            C(1),
            2,
            "Cancel Access/beta\n",
            None,
        ),
        (
            "diplomatic_relation={country=BBB relation=guarantee give_guarantee=CCC}",
            C(1),
            2,
            "Enact Guarantee/beta\nGuarantee Gamma\n",
            None,
        ),
        (
            "diplomatic_relation={country=BBB relation=guarantee give_guarantee=CCC}",
            S(1),
            2,
            "Guarantee Gamma\n",
            None,
        ),
        (
            "diplomatic_relation={country=\"BBB\" relation=guarantee relation=military_access active=no}",
            C(1),
            2,
            "Cancel Access/beta\n",
            None,
        ),
        ("add_to_war={enemy=\"BBB\" targeted_alliance=CCC}", C(1), 2, "War Beta/Gamma\n", None),
        ("diplomatic_relation={country=event_target:absent relation=guarantee}", C(1), 2, "", None),
        ("diplomatic_relation={country=BBB relation=guarantee}", S(1), 2, "", None),
        (
            "diplomatic_relation={country=BBB relation=\"guarantee\"}",
            C(1),
            2,
            "BROKEN DIPLOMATIC RELATION EFFECT",
            None,
        ),
        (
            "diplomatic_relation={country=BBB relation=synthetic_unknown}",
            C(1),
            2,
            "BROKEN DIPLOMATIC RELATION EFFECT",
            None,
        ),
        (
            "diplomatic_relation={country=ROOT country=BBB relation=guarantee}",
            C(1),
            2,
            "",
            Some("1403B2390"),
        ),
        (
            "diplomatic_relation={country=BBB relation=boost_party_popularity}",
            C(1),
            2,
            "",
            Some("140D0F6E0"),
        ),
        ("create_wargoal={target=BBB type=synthetic}", C(1), 2, "", Some("claimed-state set")),
        ("start_civil_war={ideology=synthetic size=0.5}", C(1), 2, "", Some("1403A0010")),
        (
            "create_purchase_contract={seller=AAA buyer=BBB civilian_factories=2}",
            C(1),
            2,
            "",
            Some("140BA59E0"),
        ),
        ("set_country_flag=synthetic_empty give_guarantee=BBB", C(1), 2, "Guarantee Beta\n", None),
        ("end_exile=BBB give_guarantee=CCC", C(1), 2, "return Beta\nGuarantee Gamma\n", None),
        (
            "diplomatic_relation={country=BBB relation=naval_blockade}",
            C(1),
            2,
            "Enact Blockade/beta\n",
            None,
        ),
        (
            "diplomatic_relation={country=BBB relation=air_base_access}",
            C(1),
            2,
            "Enact Air/beta\n",
            None,
        ),
        (
            "diplomatic_relation={country=BBB relation=offer_air_base_access}",
            C(1),
            2,
            "Enact Air/beta\n",
            None,
        ),
        (
            "diplomatic_relation={country=BBB relation=guarantee}",
            C(1),
            2,
            "Enact Guarantee/beta\n",
            None,
        ),
        (
            "diplomatic_relation={country=BBB relation=military_access}",
            C(1),
            2,
            "Enact Access/beta\n",
            None,
        ),
        (
            "diplomatic_relation={country=BBB relation=offer_military_access}",
            C(1),
            2,
            "Enact Access/beta\n",
            None,
        ),
        (
            "diplomatic_relation={country=BBB relation=improve_relation}",
            C(1),
            2,
            "Enact Improve/beta\n",
            None,
        ),
        (
            "diplomatic_relation={country=BBB relation=non_aggression_pact}",
            C(1),
            2,
            "Enact Pact/beta\n",
            None,
        ),
        (
            "diplomatic_relation={country=BBB relation=embargo}",
            C(1),
            2,
            "Enact Embargo/beta\n",
            None,
        ),
        (
            "diplomatic_relation={country=BBB relation=market_access_rights}",
            C(1),
            2,
            "Enact Market/beta\n",
            None,
        ),
        (
            "diplomatic_relation={country=BBB relation=war_relation}",
            C(1),
            2,
            "Enact Atwar/beta\n",
            None,
        ),
        (
            "diplomatic_relation={country=BBB relation=send_attache}",
            C(1),
            2,
            "Enact Attache/beta\n",
            None,
        ),
        (
            "diplomatic_relation={country=BBB relation=docking_rights}",
            C(1),
            2,
            "Enact Docking/beta\n",
            None,
        ),
        (
            "diplomatic_relation={country=BBB relation=offer_docking_rights}",
            C(1),
            2,
            "Enact Docking/beta\n",
            None,
        ),
        (
            "diplomatic_relation={country=BBB relation=licence_production_access}",
            C(1),
            2,
            "Enact Licence/beta\n",
            None,
        ),
        ("diplomatic_relation={country=BBB relation=puppet}", C(1), 2, "Enact /beta\n", None),
        ("diplomatic_relation={country=BBB relation=lend_lease}", C(1), 2, "Enact /beta\n", None),
        (
            "diplomatic_relation={country=BBB relation=equipment_purchase_contract_relation}",
            C(1),
            2,
            "Enact /beta\n",
            None,
        ),
        ("add_faction_goal=SYN_GOAL", C(1), 2, "", Some("faction goal content")),
        ("add_faction_goal=SYN_GOAL", C(3), 2, "", None),
        ("remove_faction_goal=SYN_GOAL", C(1), 2, "", Some("faction goal content")),
        ("remove_faction_goal=SYN_GOAL", C(3), 2, "", None),
        ("set_faction_manifest=SYN_GOAL", C(1), 2, "", Some("faction goal content")),
        ("set_faction_manifest=SYN_GOAL", C(3), 2, "", None),
        (
            "create_faction_from_template=SYN_TEMPLATE",
            C(1),
            2,
            "",
            Some("faction template content"),
        ),
        (
            "create_faction_from_template=SYN_TEMPLATE",
            C(3),
            2,
            "",
            Some("faction template content"),
        ),
        (
            "set_faction_member_upgrade_min={upgrade=SYN_UPGRADE}",
            C(1),
            2,
            "",
            Some("faction upgrade content"),
        ),
        ("set_faction_member_upgrade_min={upgrade=SYN_UPGRADE}", C(3), 2, "", None),
        ("set_faction_rule=SYN_RULE", C(1), 2, "", Some("faction rule content")),
        ("set_faction_rule=SYN_RULE", C(3), 2, "", None),
        ("add_faction_influence_ratio=0.1", C(1), 2, "", Some("1410CF4A0")),
        ("add_faction_influence_ratio=0.1", C(3), 2, "", Some("1410CF4A0")),
        ("add_faction_influence_score=1", C(1), 2, "", Some("1410CF4A0")),
        ("add_faction_influence_score=1", C(3), 2, "", Some("1410CF4A0")),
    ] {
        *fixture.frame.tables[TableId::SimulationFeatures.index()] =
            TableView::Complete(TableRows::SimulationFeatures(vec![SimulationFeatureRow {
                dlc_mask: mask,
                ..Default::default()
            }]));
        let mut scopes = EventScopes::new(vec![EventScope::of(object)]);
        let out = fixture.render(
            body,
            &mut scopes,
            Content {
                autonomy_states: Some(&autonomy_states),
                ..Content::default()
            },
        );
        assert_eq!(out.text, expected, "{body}/{object:?}: {:?}", out.refusals);
        match refusal {
            None => assert!(
                out.refusals.is_empty(),
                "{body}/{object:?}: {:?}",
                out.refusals,
            ),
            Some(reason) => {
                assert_eq!(
                    out.refusals.values().sum::<usize>(),
                    1,
                    "{body}: {:?}",
                    out.refusals,
                );
                assert!(
                    out.refusals.keys().next().unwrap().contains(reason),
                    "{body}: {:?}",
                    out.refusals,
                );
            }
        }
    }
    autonomy_states
        .states
        .push(installed_content::autonomy::AutonomyState {
            id: "autonomy_free".into(),
            min_freedom_level: None,
        });
    for (body, expected) in [
        ("set_autonomy={target=BBB autonomy_state=autonomy_free}", "Set Alpha/Beta/Defined free\n"),
        (
            "release_autonomy={target=BBB autonomy_state=autonomy_free}",
            "Autonomy Beta/Defined free\n",
        ),
    ] {
        let mut scopes = EventScopes::new(vec![EventScope::of(C(1))]);
        let out = fixture.render(
            body,
            &mut scopes,
            Content {
                autonomy_states: Some(&autonomy_states),
                ..Content::default()
            },
        );
        assert_eq!(out.text, expected, "{body}: {:?}", out.refusals);
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
    }
    for (body, refused) in [
        ("release_autonomy={target=BBB}", false),
        ("set_autonomy={target=event_target:absent autonomy_state=synthetic}", false),
        ("release_autonomy={target=BBB autonomy_state=synthetic}", true),
        ("set_autonomy={target=BBB autonomy_state=autonomy_free}", true),
    ] {
        let mut scopes = EventScopes::new(vec![EventScope::of(C(1))]);
        let out = fixture.render(body, &mut scopes, Content::default());
        assert!(out.text.is_empty(), "{body}: {:?}", out.refusals);
        assert_eq!(out.refusals.values().sum::<usize>(), usize::from(refused));
        assert!(
            out.refusals
                .keys()
                .all(|why| why.contains("autonomy states content unavailable"))
        );
    }
    for (body, expected, refusal) in [
        ("add_to_war={enemy=ROOT.FROM targeted_alliance=FROM.ROOT}", "War Alpha/Beta\n", None),
        ("add_to_war={enemy=ROOT.ROOT targeted_alliance=BBB}", "", Some("origin_country fallback")),
    ] {
        let mut graph = EventScopes::new(vec![
            EventScope {
                root: Link::Scope(1),
                from: Link::Scope(3),
                ..EventScope::of(C(1))
            },
            EventScope {
                root: Link::Scope(2),
                from: Link::Scope(3),
                ..EventScope::of(C(2))
            },
            EventScope::of(EventObject::Character(11)),
            EventScope {
                root: Link::Scope(1),
                ..EventScope::of(S(2))
            },
        ]);
        let out = fixture.render(body, &mut graph, Content::default());
        assert_eq!(out.text, expected, "{body}: {:?}", out.refusals);
        match refusal {
            None => assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals),
            Some(reason) => assert!(out.refusals.keys().any(|why| why.contains(reason))),
        }
    }
    for (template_token, expected, refused) in [(None, "Spy \n", false), (Some(77), "", true)] {
        if let TableView::Complete(TableRows::Factions(rows)) =
            &mut *fixture.frame.tables[TableId::Factions.index()]
        {
            rows[0].name.clear();
            rows[0].template_token = template_token;
        }
        let mut graph = EventScopes::new(vec![EventScope::of(C(1))]);
        let out = fixture.render("set_faction_spymaster=yes", &mut graph, Content::default());
        assert_eq!(out.text, expected, "{:?}", out.refusals);
        assert_eq!(out.refusals.values().sum::<usize>(), usize::from(refused));
    }
}

#[test]
fn diplomacy_ordered_subjects_at_5000_countries() {
    let mut fixture = Fixture::new();
    let countries = synthetic_countries(5_000);
    let mut localisation = "\u{feff}l_english:\n EFFECT_ADD_TO_FACTION:0 \"Join $COUNTRY$\"\n EFFECT_GIVE_GUARANTEE:0 \"Guarantee $COUNTRY$\"\n"
        .to_owned();
    for country in &countries {
        let tag = std::str::from_utf8(&country.tag).unwrap();
        localisation.push_str(&format!(" {tag}:0 \"{tag}\"\n"));
    }
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        ("localisation/test_l_english.yml", &localisation),
    ]);
    fixture.catalogue = catalogue;
    fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    *fixture.frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(
            countries
                .iter()
                .map(|row| CountryLinkRow {
                    id: row.id,
                    canonical_slot: row.canonical_slot,
                })
                .collect(),
        ));
    *fixture.frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(
            countries
                .iter()
                .map(|row| CountryAppearanceRow {
                    country: row.id,
                    subjects: if row.canonical_slot == 2 {
                        countries
                            .iter()
                            .skip(2)
                            .map(|subject| subject.id as i32)
                            .collect::<Vec<_>>()
                    } else {
                        vec![]
                    }
                    .into(),
                    ..Default::default()
                })
                .collect(),
        ));
    *fixture.frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(
            countries
                .iter()
                .map(|row| CountryCounterInputRow {
                    country: row.id,
                    owned_state_count: 1,
                    ..Default::default()
                })
                .collect(),
        ));
    *fixture.frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(
            countries
                .iter()
                .map(|row| PoliticsRow {
                    country: row.id,
                    ..Default::default()
                })
                .collect(),
        ));
    let target = String::from_utf8(countries[1].tag.clone()).unwrap();
    let last = String::from_utf8(countries.last().unwrap().tag.clone()).unwrap();
    *fixture.frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries));
    let mut graph = EventScopes::new(vec![EventScope::of(C(1))]);
    let out = fixture.render(
        &format!("add_to_faction={target} give_guarantee={last}"),
        &mut graph,
        Content::default(),
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert_eq!(out.text.lines().count(), 5_000);
    assert!(out.text.starts_with(&format!("Join {target}\n")));
    assert!(out.text.ends_with(&format!("Join {last}\nGuarantee {last}\n")));
}
