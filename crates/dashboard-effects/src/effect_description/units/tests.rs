use super::*;
use crate::{
    effect_description::tests::Fixture, scripted_text::EventScope, triggers::ContentFacts,
};
use EventObject::{Country as C, State as S, Unit as U};
use replay_data::rows::*;
use replay_frame::frame::TableView;
use std::collections::BTreeSet;
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
                ("AAA_ADJ", "Alphan"),
                ("BBB_ADJ", "Betan"),
                ("EFFECT_ADD_AIR_EXPERIENCE", "Air $VAL|2$"),
                ("EFFECT_ADD_NAVY_EXPERIENCE", "Navy $VAL|2$"),
                ("EFFECT_RAID_GIVE_UNIT_EXPERIENCE", "RaidXP $VAL|2$"),
                ("EFFECT_MANPOWER_ADD_COUNTRY", "Men $NAME$/$VAL|0$"),
                ("EFFECT_MANPOWER_ADD_STATE", "StateMen $NAME$/$VAL|0$"),
                ("EFFECT_DELETE_UNITS", "DeleteUnits $NAME$"),
                ("EFFECT_DISBAND_UNITS", "DisbandUnits $NAME$"),
                ("EFFECT_DISABLE_TEMPLATE_EDITTING", "Lock $NAME$"),
                ("EFFECT_ENABLE_TEMPLATE_EDITTING", "Unlock $NAME$"),
                ("EFFECT_CLEAR_TEMPLATE_DIVISION_CAP", "ClearCap $NAME$"),
                ("EFFECT_SET_TEMPLATE_DIVISION_CAP", "Cap $NAME$/$CAP|0$"),
                ("YES", "yes"),
                ("NO", "no"),
                (
                    "EFFECT_SET_TEMPLATE_FORCE_ALLOW_RECRUITING",
                    "Recruit $NAME$/$ALLOW$",
                ),
                ("EFFECT_DISABLE_ALL_TEMPLATE_EDITING", "LockAll"),
                ("EFFECT_ENABLE_ALL_TEMPLATE_EDITING", "UnlockAll"),
                ("EFFECT_TRANSFER_NAVY", "NavyTo $COUNTRY$"),
                ("EFFECT_TRANSFER_UNITS", "UnitsTo $RECIPIENT$"),
                (
                    "EFFECT_SEND_EQUIPMENT_FRACTION",
                    "Fraction $FRACTION|2$/$COUNTRY$",
                ),
                ("EFFECT_EQUIPMENT_FRACTION", "Adjust $VAL|2$"),
                (
                    "EFFECT_ADD_DESIGN_TEMPLATE_BONUS",
                    "Design $DISCOUNT|2$/$USES|0$/$NAME$ [This.GetName]",
                ),
                ("syn_bonus", "Bonus [This.GetName]"),
                (
                    "EFFECT_ADD_EQUIPMENT_PRODUCTION",
                    "Produce $EQUIPMENT$ [This.GetName]",
                ),
                (
                    "EFFECT_ADD_UNITS_TO_DIVISION_TEMPLATE",
                    "Add $COUNTRY$/$TEMPLATE_NAME$",
                ),
                ("EFFECT_SET_OOB", "OOB $NAME$ [This.GetName]"),
                (
                    "EFFECT_CREATE_SHIP",
                    "Ship $VARIANT$/$COUNTRY$ [This.GetName]",
                ),
                ("EFFECT_CREATE_UNIT", "Unit $OWNER$"),
                ("EFFECT_CREATE_UNIT_WITH_COUNT", "Units $OWNER$/$COUNT|0$"),
                ("EFFECT_CREATE_UNIT_LOCATION", "UnitAt $OWNER$/$STATE$"),
                (
                    "EFFECT_CREATE_UNIT_LOCATION_WITH_COUNT",
                    "UnitsAt $OWNER$/$COUNT|0$/$STATE$",
                ),
                (
                    "CHANGE_DIV_TEMPLATE_EFFECT",
                    "Template $TEMPLATE$/$DIV$ [Root.GetName]",
                ),
                ("EFFECT_UNIT_DESTROY_UNIT", "Destroy $NAME$ [Root.GetName]"),
                (
                    "EFFECT_SET_UNIT_ORGANIZATION",
                    "SetOrg $VAL|2$ [This.GetName]",
                ),
                ("EFFECT_RAID_DAMAGE_PLANE", "Plane $VAL|2$"),
                ("EFFECT_RAID_DAMAGE_PLANE_RATIO", "PlaneRatio $VAL|2$"),
                ("EFFECT_RAID_DAMAGE_UNIT_STRENGTH", "Strength $VAL|2$"),
                (
                    "EFFECT_RAID_DAMAGE_UNIT_STRENGTH_RATIO",
                    "StrengthRatio $VAL|2$",
                ),
                ("EFFECT_RAID_DAMAGE_UNIT_ORG", "Org $VAL|2$"),
                ("EFFECT_RAID_DAMAGE_UNIT_ORG_RATIO", "OrgRatio $VAL|2$"),
                (
                    "EFFECT_RAID_DAMAGE_UNIT_STRENGTH_ORG",
                    "Both $STR_VAL|2$/$ORG_VAL|2$",
                ),
                (
                    "EFFECT_RAID_DAMAGE_UNIT_STRENGTH_ORG_RATIO",
                    "BothRatio $STR_VAL|2$/$ORG_VAL|2$",
                ),
                ("syn_unit", "Base unit"),
                ("AAA_syn_unit", "Perspective unit"),
                ("UNLOCK_SUBUNIT_EFFECT", "UnlockType $NAME$ [This.GetName]"),
                ("syn_equipment", "Equipment [This.GetName]"),
                ("STAT_SYNTHETIC_COST", "Cost"),
                ("STAT_SYNTHETIC_SOFT_ATTACK", "Attack"),
                ("STAT_SYNTHETIC_AIR_ATTACK", "Range"),
                ("STAT_SYNTHETIC_AIR_ATTACK_MOD_DIFF", "Custom $VALUE|2$"),
                ("STAT_MODIFIER_DIFF_NEG", "Bad $VALUE|2$"),
                ("STAT_MODIFIER_DIFF", "Good $VALUE|2$"),
                ("Syn_Category", "Category"),
                ("AAA_Syn_Category", "Perspective category"),
                ("SUBUNIT_BONUS_HEADER", "$ICON$$NAME$"),
                ("STAT_ARMY_COMBAT_WIDTH", "Width"),
                ("STAT_ARMY_COMBAT_WIDTH_DIFF", " $VALUE|2$"),
                ("STAT_SYNTHETIC_DEFENSE", "Defense"),
                ("STAT_SYNTHETIC_SOFT_ATTACK_MOD_VALUE", " custom $VALUE|2$"),
                ("STAT_MODIFIER_VALUE", " value $VALUE|2$"),
                ("STAT_ADJUSTER_MOVEMENT", "Move"),
                ("STAT_ADJUSTER_MOVEMENT_DIFF", " $VALUE|2$"),
                ("STAT_ADJUSTER_ATTACK", "Attack"),
                ("STAT_ADJUSTER_ATTACK_DIFF", " $VALUE|2$"),
                ("STAT_ADJUSTER_DEFENCE", "Defence"),
                ("STAT_ADJUSTER_DEFENCE_DIFF", " $VALUE|2$"),
                ("night", "Night"),
                ("zeta", "Zeta"),
                ("alpha", "Alpha"),
                ("SUBUNIT_BONUS_CATEGORY_ADD", "Support $CATEGORY$"),
                ("SUBUNIT_CATEGORY_ADD_STAT", "Add $NAME$ $VALUE|2$"),
                ("SUBUNIT_CATEGORY_MULT_STAT", "Mult $NAME$ $VALUE|2$"),
                ("SUBUNIT_CATEGORY_NEG_ADD_STAT", "BadAdd $NAME$ $VALUE|2$"),
                ("SUBUNIT_CATEGORY_NEG_MULT_STAT", "BadMult $NAME$ $VALUE|2$"),
                ("STAT_SYNTHETIC_SUPPLY_CONSUMPTION", "Supply"),
                ("ZZZ", "Many"),
            ]
            .into_iter()
            .map(|(key, value)| (key.into(), value.into())),
        )
        .collect::<Localisation>()
        .into();
    Arc::make_mut(&mut fixture.catalogue.localisation).language = "english".into();
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            seed: replay_data::rows::Recorded::Value(None),
            scope_kind: 1,
            scope_id: 100,
            entries: vec![ScriptVariableEntry {
                name: b"syn_amount".to_vec(),
                value_raw: 249_999,
            }],
        }]));
    if let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *fixture.frame.tables[TableId::Provinces.index()]
    {
        for row in rows {
            row.controller = 100;
        }
    }
    if let TableView::Complete(TableRows::States(rows)) =
        &mut *fixture.frame.tables[TableId::States.index()]
    {
        rows[4].owner = 0;
    }
    fixture
}

fn render(
    fixture: &Fixture,
    body: &str,
    object: EventObject,
    content: Content<'_>,
    impassable: &[u32],
) -> Description {
    render_with_nato(fixture, body, object, content, impassable, false)
}

fn render_with_nato(
    fixture: &Fixture,
    body: &str,
    object: EventObject,
    content: Content<'_>,
    impassable: &[u32],
    nato: bool,
) -> Description {
    let dlcs = BTreeSet::new();
    let triggers = Default::default();
    let aliases = Arc::default();
    let mut provinces: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for province in fixture.frame.complete::<tables::Provinces>().unwrap() {
        provinces
            .entry(province.state)
            .or_default()
            .push(province.id);
    }
    let blocked = impassable.iter().copied().collect();
    let mut facts = ContentFacts::content_only(&dlcs, &triggers, &aliases);
    facts.state_provinces = &provinces;
    facts.state_content = Ok(());
    facts.impassable = &blocked;
    let scripted = fixture.catalogue.scripted_content().unwrap();
    let text = ScriptedText::new(&fixture.frame, &fixture.catalogue.localisation, scripted, 1)
        .with_nato_counters(nato);
    text.set_synthetic_world_with_template_names(World::read(&fixture.frame, facts));
    let context = Context::new(
        &text,
        &fixture.catalogue.localisation,
        1,
        "synthetic",
        content,
    );
    describe(
        &parser::parse(body.as_bytes()).unwrap().items,
        &context,
        &mut EventScopes::new(vec![EventScope::of(object)]),
    )
}

const UNIT: u64 = (7_u64 << 32) | 0x1269;

fn bonus_names() -> content_reader_script::key_names::KeyNames {
    let mut units = content_reader_script::sub_units::SubUnits::default();
    units.read(
        b"sub_units={syn_unit={}} sub_unit_categories={Syn_Category syn_category}",
        "synthetic",
    );
    let mut names = content_reader_script::key_names::KeyNames::default();
    names.sub_units = Some(units);
    names
}

#[test]
fn unit_bonus_preserves_rosters_stat_and_terrain_order_and_nato_frame() {
    game::enter();
    let mut fixture = fixture();
    let (mut catalogue, _folder) = crate::fixtures::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23},NSupply={MIN_TRAIN_REQUIREMENT=3}}",
        ),
        ("common/terrain/test.txt", "categories={zeta={} alpha={}}"),
    ]);
    catalogue.localisation = fixture.catalogue.localisation.clone();
    fixture.catalogue = catalogue;
    let names = bonus_names();
    let body = "fixture_unit_stat_bonus={name=ignored syn_category={soft_attack=8 combat_width=99 night={attack=99}} syn_unit={soft_attack=2 defense=3 soft_attack=bad combat_width=-0.5 night={defence=3 attack=2 movement=1 attack=bad} alpha={attack=5} zeta={attack=4}} syn_category={defense=9} missing_unit={soft_attack=99}}";
    for nato in [false, true] {
        let out = render_with_nato(
            &fixture,
            body,
            C(1),
            Content {
                key_names: Some(&names),
                ..Default::default()
            },
            &[],
            nato,
        );
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
        let frame = if nato { 2 } else { 1 };
        assert_eq!(
            out.text,
            format!(
                "\u{13}GFX_syn_unit_icon_small|{frame} Perspective unit\nWidth -0.50\n   Defense value 3.00\n   Attack custom 2.00\n\u{11}HNight\u{11}!\n   Move 1.00\n   Attack 2.00\n   Defence 3.00\n\u{11}HZeta\u{11}!\n   Attack 4.00\n\u{11}HAlpha\u{11}!\n   Attack 5.00\n\u{13}GFX_Syn_Category_icon_small Perspective category\n  Attack custom 8.00\n\u{13}GFX_Syn_Category_icon_small Perspective category\n  Defense value 9.00\n"
            )
        );
    }
}

#[test]
fn unit_bonus_battalion_mult_keeps_authored_stats_flags_and_adjacent_category_header() {
    game::enter();
    let fixture = fixture();
    let names = bonus_names();
    let out = render(
        &fixture,
        "fixture_unit_stat_bonus={syn_unit={battalion_mult={category=syn_category display_as_percentage=yes soft_attack=0.5 defense=1 soft_attack=0.25 soft_attack=0 soft_attack=bad supply_consumption=-0.2} battalion_mult={category=Syn_Category display_as_percentage=yes add=yes soft_attack=0.1 supply_consumption=0.2}}}",
        C(1),
        Content {
            key_names: Some(&names),
            ..Default::default()
        },
        &[],
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert_eq!(
        out.text,
        "\u{13}GFX_syn_unit_icon_small|1 Perspective unit\nSupport Perspective category\n    Add Attack 0.50\n    Add Defense 1.00\n    Add Attack 0.25\n    BadAdd Supply -0.20\n    Mult Attack 0.10\n    BadMult Supply 0.20\n"
    );
}

#[test]
fn unit_bonus_refused_adjuster_keeps_known_stats_and_later_selectors() {
    game::enter();
    let mut fixture = fixture();
    fixture.catalogue.localisation = fixture
        .catalogue
        .localisation
        .iter()
        .filter(|(key, _)| *key != "STAT_ADJUSTER_ATTACK_DIFF")
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect::<Localisation>()
        .into();
    let names = bonus_names();
    let out = render(
        &fixture,
        "fixture_unit_stat_bonus={syn_unit={soft_attack=2 night={attack=1}} syn_category={defense=3}}",
        C(1),
        Content {
            key_names: Some(&names),
            ..Default::default()
        },
        &[],
    );
    assert_eq!(
        out.text,
        "\u{13}GFX_syn_unit_icon_small|1 Perspective unit\n   Attack custom 2.00\n\u{13}GFX_Syn_Category_icon_small Perspective category\n  Defense value 3.00\n"
    );
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(
        out.refusals
            .keys()
            .all(|why| why.contains("STAT_ADJUSTER_ATTACK_DIFF"))
    );
}

fn unit_fixture() -> Fixture {
    let mut fixture = fixture();
    *fixture.frame.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(vec![DivisionRow {
            id: UNIT,
            roster_owner: 100,
            logical_owner: 200,
            expeditionary_owner: 300,
            organisation: 9_000_000,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(vec![DivisionStatusRow {
            division: UNIT,
            name: Recorded::NotRecorded,
            naming: Recorded::Value(Some(NameOperands {
                custom: b"Selected [This.GetName]".to_vec(),
                ..Default::default()
            })),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::DivisionTemplates.index()] =
        TableView::Complete(TableRows::DivisionTemplates(vec![
            DivisionTemplateRow {
                template: 10,
                owner: 100,
                name: b"Roster only".to_vec(),
                name_key: Recorded::NotRecorded,
                ..Default::default()
            },
            DivisionTemplateRow {
                template: 20,
                owner: 200,
                name: b"Raw [This.GetName]".to_vec(),
                name_key: Recorded::NotRecorded,
                ..Default::default()
            },
            DivisionTemplateRow {
                template: 30,
                owner: 200,
                name_key: Recorded::Value(b"DEF".to_vec()),
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::DivisionTemplateIcons.index()] =
        TableView::Complete(TableRows::DivisionTemplateIcons(vec![
            DivisionTemplateIconRow {
                template: 30,
                country: 100,
                ..Default::default()
            },
        ]));
    if let TableView::Complete(TableRows::ScriptVariables(rows)) =
        &mut *fixture.frame.tables[TableId::ScriptVariables.index()]
    {
        rows.insert(
            0,
            ScriptVariablesRow {
                scope_kind: 0,
                scope_id: 0,
                entries: vec![ScriptVariableEntry {
                    name: b"syn_amount".to_vec(),
                    value_raw: 25_000,
                }],
                ..Default::default()
            },
        );
    }
    fixture
}

#[test]
fn unit_carrier_empty_and_organization_table() {
    game::enter();
    let fixture = unit_fixture();
    for object in [
        C(1),
        S(1),
        U(0),
        EventObject::UnitLeader(UNIT),
        EventObject::Project { owner: 100, id: 7 },
        EventObject::Raid(UNIT),
    ] {
        for body in [
            "fixture_revise_unit_template={division_template=missing}",
            "fixture_dissolve_unit=no",
            "fixture_readiness_level=var:unrecorded^random",
        ] {
            let out = render(&fixture, body, object, Content::default(), &[]);
            assert!(out.text.is_empty(), "{body}/{object:?}: {}", out.text);
            assert!(
                out.refusals.is_empty(),
                "{body}/{object:?}: {:?}",
                out.refusals
            );
        }
    }
    for (body, expected) in [
        ("fixture_readiness_level=0", "SetOrg 0.00 [This.GetName]"),
        ("fixture_readiness_level=0.25", "SetOrg 0.25 [This.GetName]"),
        ("fixture_readiness_level=-0.5", "SetOrg -0.50 [This.GetName]"),
        (
            "fixture_readiness_level=2.49999",
            "SetOrg 2.49 [This.GetName]",
        ),
        (
            "fixture_readiness_level=syn_amount",
            "SetOrg 0.25 [This.GetName]",
        ),
        (
            "fixture_readiness_level=AAA.syn_amount",
            "SetOrg 2.49 [This.GetName]",
        ),
        ("fixture_readiness_level=THIS", "SetOrg 0.00 [This.GetName]"),
        (
            "fixture_readiness_level=event_target:absent",
            "SetOrg 0.00 [This.GetName]",
        ),
        ("fixture_revise_unit_template={division_template=missing}", ""),
        (
            "fixture_revise_unit_template={division_template=\"Roster only\"}",
            "",
        ),
        (
            "fixture_revise_unit_template={division_template=\"raw [This.GetName]\"}",
            "",
        ),
        ("fixture_revise_unit_template={division_template=DEF}", ""),
    ] {
        let out = render(&fixture, body, U(UNIT), Content::default(), &[]);
        assert_eq!(out.text, expected, "{body}: {:?}", out.refusals);
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
    }
}

#[test]
fn unit_carrier_shared_name_and_recording_gap_table() {
    game::enter();
    let mut fixture = unit_fixture();
    for (body, expected) in [
        (
            "fixture_revise_unit_template={division_template=\"Raw [This.GetName]\"}",
            "Template Raw [This.GetName]/Selected [This.GetName] [Root.GetName]",
        ),
        (
            "fixture_revise_unit_template={division_template=missing division_template=alpha}",
            "Template alpha/Selected [This.GetName] [Root.GetName]",
        ),
        ("fixture_dissolve_unit=yes", "Destroy Selected NULL NULL\n"),
        ("fixture_dissolve_unit=THIS", "Destroy Selected NULL NULL\n"),
        (
            "fixture_dissolve_unit=var:unrecorded^random",
            "Destroy Selected NULL NULL\n",
        ),
    ] {
        let out = render(&fixture, body, U(UNIT), Content::default(), &[]);
        assert_eq!(out.text, expected, "{body}: {:?}", out.refusals);
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
    }
    if let TableView::Complete(TableRows::DivisionStatus(rows)) =
        &mut *fixture.frame.tables[TableId::DivisionStatus.index()]
    {
        rows[0].naming = Recorded::NotRecorded;
    }
    for (body, class) in [
        (
            "fixture_revise_unit_template={division_template=\"Raw [This.GetName]\"}",
            "fixture_revise_unit_template",
        ),
        (
            "fixture_revise_unit_template={division_template=missing division_template=alpha}",
            "fixture_revise_unit_template",
        ),
        ("fixture_dissolve_unit=yes", "fixture_dissolve_unit"),
        ("fixture_dissolve_unit=THIS", "fixture_dissolve_unit"),
        ("fixture_dissolve_unit=var:unrecorded^random", "fixture_dissolve_unit"),
    ] {
        let out = render(
            &fixture,
            &format!("{body} {body}"),
            U(UNIT),
            Content::default(),
            &[],
        );
        assert!(out.text.is_empty(), "{body}: {}", out.text);
        let expected = format!(
            "synthetic effect {class}: key {}: naming of division status not recorded",
            if class == "fixture_dissolve_unit" {
                "fixture_dissolve_unit"
            } else {
                "fixture_revise_unit_template"
            },
        );
        assert_eq!(out.refusals, BTreeMap::from([(expected, 2)]), "{body}");
    }
    if let TableView::Complete(TableRows::DivisionTemplates(rows)) =
        &mut *fixture.frame.tables[TableId::DivisionTemplates.index()]
    {
        rows[2].name_key = Recorded::NotRecorded;
    }
    let out = render(
        &fixture,
        "fixture_revise_unit_template={division_template=missing}",
        U(UNIT),
        Content::default(),
        &[],
    );
    assert!(out.text.is_empty());
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(
        out.refusals
            .keys()
            .all(|why| why.contains("name key of division templates not recorded")),
        "{:?}",
        out.refusals
    );
    for (owner, reason) in [
        (0, "reserved country-zero division-template roster"),
        (
            999,
            "country links lacks change-division-template logical owner 999",
        ),
    ] {
        if let TableView::Complete(TableRows::Divisions(rows)) =
            &mut *fixture.frame.tables[TableId::Divisions.index()]
        {
            rows[0].logical_owner = owner;
        }
        let out = render(
            &fixture,
            "fixture_revise_unit_template={division_template=missing}",
            U(UNIT),
            Content::default(),
            &[],
        );
        assert!(out.text.is_empty());
        assert_eq!(out.refusals.values().sum::<usize>(), 1);
        assert!(
            out.refusals.keys().all(|why| why.contains(reason)),
            "{:?}",
            out.refusals
        );
    }
    for (body, class) in [
        (
            "fixture_revise_unit_template={division_template=missing}",
            "fixture_revise_unit_template",
        ),
        ("fixture_dissolve_unit=yes", "fixture_dissolve_unit"),
        (
            "fixture_readiness_level=var:unrecorded^random",
            "fixture_readiness_level",
        ),
    ] {
        let missing = UNIT + (1_u64 << 32);
        let out = render(&fixture, body, U(missing), Content::default(), &[]);
        assert!(out.text.is_empty());
        assert_eq!(out.refusals.values().sum::<usize>(), 1);
        assert!(
            out.refusals.keys().all(|why| why.contains(class)
                && why.contains(&format!("Divisions lacks UNIT {missing:#x}"))),
            "{body}: {:?}",
            out.refusals
        );
    }
    *fixture.frame.tables[TableId::Divisions.index()] = TableView::Unavailable {
        reason: "synthetic unit gap".into(),
    };
    for body in [
        "fixture_revise_unit_template={division_template=missing}",
        "fixture_dissolve_unit=yes",
        "fixture_readiness_level=1",
    ] {
        let out = render(&fixture, body, U(UNIT), Content::default(), &[]);
        assert!(out.text.is_empty());
        assert_eq!(out.refusals.values().sum::<usize>(), 1);
        assert!(
            out.refusals
                .keys()
                .all(|why| why.contains("synthetic unit gap")),
            "{body}: {:?}",
            out.refusals
        );
    }
}

#[test]
fn unit_carrier_graph_links_keep_operand_scope() {
    game::enter();
    let fixture = unit_fixture();
    let mut scopes = EventScopes::new(vec![
        EventScope {
            from: Link::Scope(1),
            ..EventScope::of(C(1))
        },
        EventScope {
            root: Link::Scope(0),
            ..EventScope::of(U(UNIT))
        },
    ]);
    let out = fixture.render(
        "FROM={fixture_readiness_level=ROOT.syn_amount}",
        &mut scopes,
        Content::default(),
    );
    assert_eq!(out.text, "SetOrg 2.49 [This.GetName]");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert_eq!(scopes.scopes[1].this, U(UNIT));
}

#[test]
fn unit_carrier_format98_naming_table() {
    game::enter();
    let names = content_reader_division_names::division_names::parse(
        br#"SYN_G={prefix="P " ordered={12={"%s/%d %s"}}
            unordered={a={"Literal %d"}} fallback_name="Fallback %d"}
            EMPTY={} SHIP={type=ship unique={"Ship"}}"#,
        &Default::default(),
    )
    .unwrap();
    let content = Content {
        division_names: Some(&names),
        ..Default::default()
    };
    let mut fixture = unit_fixture();
    for (group, order, ordered, programmatic, custom, kind, expected) in [
        ("missing", 0, 1, 0, "Player %d", 3, Ok("Player %d")),
        ("SyN_G", 12, 255, 0, "", 0, Ok("P XII/%d XII")),
        ("SYN_G", 0, 0, 0, "", 0, Ok("P Literal %d")),
        ("SYN_G", 5, 1, 0, "", 0, Ok("Fallback 5")),
        ("SYN_G", 2, 0, 0, "", 0, Ok("Fallback 3")),
        ("SYN_G", -1, 0, 0, "", 0, Ok("Fallback 1")),
        ("SYN_G", i32::MAX, 0, 0, "", 0, Ok("Fallback 1")),
        ("SYN_G", 12, 1, 255, "Ignored", 0, Ok("P XII/%d XII")),
        (
            "",
            7,
            1,
            1,
            "Stored %d [Root.GetTag]",
            0,
            Ok("Stored %d [Root.GetTag]"),
        ),
        (
            "SYN_G",
            0,
            1,
            0,
            "",
            0,
            Err("ordered name is not yet assigned"),
        ),
        (
            "missing",
            1,
            1,
            0,
            "",
            0,
            Err("no name file that was read declares"),
        ),
        ("SHIP", 0, 0, 0, "", 0, Err("not in the army table")),
        ("SYN_G", 1, 1, 0, "", 1, Err("name type 1")),
        (
            "",
            7,
            1,
            1,
            "",
            0,
            Err("legacy name allocation inputs are not recorded"),
        ),
        (
            "EMPTY",
            7,
            1,
            0,
            "",
            0,
            Err("legacy name allocation inputs are not recorded"),
        ),
    ] {
        if let TableView::Complete(TableRows::DivisionStatus(rows)) =
            &mut *fixture.frame.tables[TableId::DivisionStatus.index()]
        {
            rows[0].naming = Recorded::Value(Some(NameOperands {
                kind,
                order,
                ordered,
                programmatic,
                custom: custom.as_bytes().to_vec(),
                group: group.as_bytes().to_vec(),
            }));
        }
        let body = "fixture_revise_unit_template={division_template=\"Raw [This.GetName]\"}";
        let out = render(&fixture, body, U(UNIT), content, &[]);
        match expected {
            Ok(name) => {
                assert_eq!(
                    out.text,
                    format!("Template Raw [This.GetName]/{name} [Root.GetName]")
                );
                assert!(
                    out.refusals.is_empty(),
                    "{group}/{order}: {:?}",
                    out.refusals
                );
            }
            Err(reason) => {
                assert!(out.text.is_empty());
                assert_eq!(out.refusals.values().sum::<usize>(), 1);
                assert!(
                    out.refusals.keys().all(|why| why.contains(reason)),
                    "{group}/{order}: {:?}",
                    out.refusals
                );
            }
        }
    }
    for (naming, reason) in [
        (Recorded::NotRecorded, "naming of division status not recorded"),
        (Recorded::Value(None), "guarded read or arena shortage"),
        (
            Recorded::Value(Some(NameOperands {
                custom: vec![0xff],
                ..Default::default()
            })),
            "custom division name is not UTF-8",
        ),
    ] {
        if let TableView::Complete(TableRows::DivisionStatus(rows)) =
            &mut *fixture.frame.tables[TableId::DivisionStatus.index()]
        {
            rows[0].naming = naming;
        }
        let out = render(
            &fixture,
            "fixture_dissolve_unit=yes fixture_dissolve_unit=yes",
            U(UNIT),
            content,
            &[],
        );
        assert!(out.text.is_empty());
        assert_eq!(out.refusals.values().sum::<usize>(), 2);
        assert!(
            out.refusals.keys().all(|why| why.contains(reason)),
            "{:?}",
            out.refusals
        );
        let out = render(
            &fixture,
            "fixture_readiness_level=0.25",
            U(UNIT),
            content,
            &[],
        );
        assert_eq!(out.text, "SetOrg 0.25 [This.GetName]");
        assert!(out.refusals.is_empty());
    }
}

#[test]
fn unit_carrier_name_groups_use_recorded_layers_and_one_cache() {
    game::enter();
    use crate::fixtures::{folder_catalogue, layers_catalogue};
    let (_, base) = folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23}}",
        ),
        (
            "common/units/names_divisions/a.txt",
            "G={ordered={9={\"Base %d\"}}}",
        ),
        (
            "localisation/test_l_english.yml",
            "\u{feff}l_english:\n EFFECT_UNIT_DESTROY_UNIT:0 \"Destroy $NAME$ [Root.GetName]\"\n",
        ),
    ]);
    let (_, later) = folder_catalogue(&[(
        "common/units/names_divisions/a.txt",
        "G={ordered={9={\"Mod %d\"}}}",
    )]);
    let layers =
        content_reader_layers::ResolvedLayers::from_folders(&[(&base.0, &[][..]), (&later.0, &[][..])])
            .unwrap();
    let mut fixture = unit_fixture();
    fixture.catalogue = layers_catalogue(layers);
    if let TableView::Complete(TableRows::DivisionStatus(rows)) =
        &mut *fixture.frame.tables[TableId::DivisionStatus.index()]
    {
        rows[0].naming = Recorded::Value(Some(NameOperands {
            group: b"G".to_vec(),
            order: 9,
            ordered: 1,
            ..Default::default()
        }));
    }
    let cached = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    assert_eq!(
        cached
            .view(None)
            .division_names
            .unwrap()
            .resolve("G", 9, true)
            .as_deref(),
        Ok("Mod 9")
    );
    std::fs::write(later.0.join("common/units/names_divisions/a.txt"), "G={}").unwrap();
    let out = render(
        &fixture,
        "fixture_dissolve_unit=yes",
        U(UNIT),
        Content::default(),
        &[],
    );
    assert_eq!(out.text, "Destroy Mod 9 NULL\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn unit_carrier_naming_has_no_country_or_division_cap() {
    game::enter();
    let mut fixture = unit_fixture();
    let countries = crate::effect_description::tests::synthetic_countries(24);
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
    *fixture.frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries));
    let ids: Vec<_> = (1..=24_u64).map(|id| id << 32 | 55).collect();
    *fixture.frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(
        ids.iter()
            .enumerate()
            .map(|(at, &id)| DivisionRow {
                id,
                logical_owner: (at as u32 + 1) * 100,
                ..Default::default()
            })
            .collect(),
    ));
    *fixture.frame.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(
            ids.iter()
                .enumerate()
                .map(|(at, &division)| DivisionStatusRow {
                    division,
                    name: Recorded::NotRecorded,
                    naming: Recorded::Value(Some(NameOperands {
                        custom: format!("Unit {}", at + 1).into_bytes(),
                        ..Default::default()
                    })),
                    ..Default::default()
                })
                .collect(),
        ));
    let out = render(
        &fixture,
        "fixture_dissolve_unit=yes",
        U(*ids.last().unwrap()),
        Content::default(),
        &[],
    );
    assert_eq!(out.text, "Destroy Unit 24 NULL\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn units_output_branches() {
    game::enter();
    let fixture = fixture();
    let equipment = equipment();
    let mut sub_units = content_reader_script::sub_units::SubUnits::default();
    sub_units.read(b"sub_units={syn_unit={}}", "synthetic");
    let mut names = content_reader_script::key_names::KeyNames::default();
    names.sub_units = Some(sub_units);
    let content = Content {
        key_names: Some(&names),
        equipment: Some(&equipment),
        ..Default::default()
    };
    for (body, object, expected) in [
        ("fixture_air_points=1.25", C(1), "Air 1.25\n"),
        ("fixture_fleet_points=syn_amount", C(1), "Navy 2.49\n"),
        ("fixture_raid_unit_points=-0.5", C(1), "RaidXP -0.50\n"),
        ("fixture_recruit_pool=syn_amount", C(1), "Men Alpha/2\n"),
        ("fixture_recruit_pool=-1.5", S(1), "StateMen One/-2\n"),
        (
            "fixture_retire_formation={division_template=Raw}",
            C(1),
            "DeleteUnits Raw\n",
        ),
        (
            "fixture_retire_formation={division_template=Raw disband=yes}",
            C(1),
            "DisbandUnits Raw\n",
        ),
        (
            "fixture_retire_formation={division_template=Raw fixture_air_points=1}",
            C(1),
            "DeleteUnits Raw\nAir 1.00\n",
        ),
        (
            "fixture_dissolve_template={division_template=Raw}",
            C(1),
            "Delete Raw\n",
        ),
        (
            "fixture_dissolve_template={division_template=Raw disband=yes}",
            C(1),
            "Disband Raw\n",
        ),
        (
            "fixture_dissolve_template={division_template=Raw fixture_air_points=1}",
            C(1),
            "Delete Raw\n",
        ),
        (
            "fixture_lock_template={division_template=Raw is_locked=yes}",
            C(1),
            "Lock Raw\n",
        ),
        (
            "fixture_lock_template={division_template=Raw}",
            C(1),
            "Unlock Raw\n",
        ),
        (
            "fixture_clear_template_limit={division_template=Raw}",
            C(1),
            "ClearCap Raw\n",
        ),
        (
            "fixture_template_limit={division_template=Raw division_cap=syn_amount}",
            C(1),
            "Cap Raw/2\n",
        ),
        (
            "fixture_template_limit={division_template=Raw division_cap=-2.9}",
            C(1),
            "Cap Raw/-2\n",
        ),
        (
            "fixture_allow_recruitment={division_template=Raw force_allow_recruiting=yes}",
            C(1),
            "Recruit Raw/yes\n",
        ),
        (
            "fixture_allow_recruitment={division_template=Raw}",
            C(1),
            "Recruit Raw/no\n",
        ),
        ("fixture_lock_country_templates=yes", C(1), "LockAll\n"),
        (
            "fixture_lock_country_templates={is_locked=no}",
            C(1),
            "UnlockAll\n",
        ),
        ("fixture_move_fleet={target=BBB}", C(1), "NavyTo beta\n"),
        (
            "fixture_move_share={target=BBB fraction=0.25}",
            C(1),
            "UnitsTo beta",
        ),
        (
            "fixture_send_share={target=BBB fraction=1.5}",
            C(1),
            "Fraction 1.00/Beta\n",
        ),
        (
            "fixture_send_share={target=BBB fraction=-0.5}",
            C(1),
            "Fraction 0.00/Beta\n",
        ),
        ("fixture_equipment_share=2.5", C(1), "Adjust 1.50\n"),
        ("fixture_equipment_share=0", C(1), "Adjust -1.00\n"),
        (
            "fixture_design_bonus={name=syn_bonus cost_factor=-0.2 uses=3}",
            C(1),
            "Design -0.20/3/Bonus [This.GetName] [This.GetName]\n",
        ),
        (
            "fixture_design_bonus={name=syn_bonus cost_factor=syn_amount uses=oops}",
            C(1),
            "Design 0.00/0/Bonus [This.GetName] [This.GetName]\n",
        ),
        (
            "fixture_equipment_order={equipment={version_name=Raw type=missing} equipment={type=another}}",
            C(1),
            "Produce Raw Alpha",
        ),
        (
            "fixture_template_members={template_name=Raw}",
            C(1),
            "Add Alpha/Raw\n",
        ),
        ("fixture_naval_roster=Raw", C(1), "OOB Raw Alpha"),
        (
            "fixture_launch_ship={equipment_variant=Raw}",
            C(1),
            "Ship Raw/alpha Alpha",
        ),
        (
            "fixture_launch_ship={equipment_variant=Raw creator=BBB}",
            C(1),
            "Ship Raw/beta Alpha",
        ),
        ("fixture_launch_ship={equipment_variant=Raw creator=ZZZ}", C(1), ""),
        ("fixture_launch_ship={equipment_variant=Raw}", S(1), ""),
        ("fixture_raise_unit={owner=AAA}", C(1), "Unit Alphan"),
        ("fixture_raise_unit={owner=BBB count=2.5}", C(1), "Units Betan/3"),
        ("fixture_raise_unit={owner=AAA count=0}", S(1), ""),
        ("fixture_raise_unit={owner=AAA}", S(1), "UnitAt Alphan/One"),
        (
            "fixture_raise_unit={owner=AAA count=2}",
            S(1),
            "UnitsAt Alphan/2/One",
        ),
        (
            "fixture_raise_unit={owner=AAA prioritize_location=2}",
            S(1),
            "UnitAt Alphan/Two",
        ),
        (
            "fixture_raise_unit={owner=AAA prioritize_location=2}",
            C(1),
            "Unit Alphan",
        ),
        (
            "fixture_raise_unit={owner=AAA country_score={base=0}}",
            S(1),
            "Unit Alphan",
        ),
        (
            "fixture_raise_unit={owner=AAA country_score={base=1 modifier={factor=0 NOT={PREV={impassable=no}}}}}",
            S(1),
            "UnitAt Alphan/One",
        ),
        (
            "fixture_raid_damage={plane_loss=2 damage=3 str_damage=4 org_damage=5}",
            C(1),
            "Plane 2.00",
        ),
        (
            "fixture_raid_damage={plane_loss=2 ratio=yes}",
            C(1),
            "PlaneRatio 2.00",
        ),
        (
            "fixture_raid_damage={damage=3 ratio=yes org_damage=5}",
            C(1),
            "Strength 3.00",
        ),
        (
            "fixture_raid_damage={str_damage=4 org_damage=5}",
            C(1),
            "Both 4.00/5.00",
        ),
        (
            "fixture_raid_damage={str_damage=4 org_damage=5 ratio=yes}",
            C(1),
            "BothRatio 4.00/5.00",
        ),
        (
            "fixture_raid_damage={str_damage=syn_amount}",
            C(1),
            "Strength 2.49",
        ),
        (
            "fixture_raid_damage={str_damage=4 ratio=yes}",
            C(1),
            "StrengthRatio 4.00",
        ),
        ("fixture_raid_damage={org_damage=5}", C(1), "Org 5.00"),
        (
            "fixture_raid_damage={org_damage=5 ratio=yes}",
            C(1),
            "OrgRatio 5.00",
        ),
        ("fixture_raid_damage={ratio=yes}", C(1), ""),
        (
            "fixture_allow_subunit=syn_unit",
            C(2),
            "UnlockType Perspective unit [This.GetName]",
        ),
        ("fixture_unit_stat_bonus={name=ignored}", C(1), ""),
        ("fixture_warehouse_delivery={}", C(1), ""),
        ("fixture_equipment_model={}", C(1), ""),
        ("fixture_equipment_model={type=missing}", S(0), ""),
        ("fixture_equipment_model={type=missing}", S(5), ""),
        ("fixture_equipment_model={type=missing}", C(0), ""),
        ("fixture_scrap_ships={count=all}", C(1), ""),
        ("fixture_equipment_delivery={amount=1}", C(1), ""),
        ("fixture_empty_body={}", C(1), ""),
        (
            "fixture_equipment_change={name=ignored bonus={syn_equipment={build_cost_ic=-1 soft_attack=2 soft_attack=-2 air_attack=3}}}",
            C(1),
            "\u{11}YEquipment [This.GetName]\u{11}!: \n    CostBad -1.00\n    AttackGood 0.00\n    RangeCustom 3.00",
        ),
        (
            "BBB={fixture_move_share={target=AAA} fixture_equipment_share=2}",
            C(1),
            "\u{12}BBBBeta:\n   UnitsTo alpha   Adjust 1.00\n",
        ),
    ] {
        let out = render(&fixture, body, object, content, &[]);
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
        assert_eq!(out.text, expected, "{body}");
    }
    let mut graph = EventScopes::new(vec![EventScope {
        random_seed: 123,
        random_counter: 456,
        ..EventScope::of(C(1))
    }]);
    let before = graph.clone();
    let out = fixture.render("fixture_raise_unit={owner=AAA count=2}", &mut graph, content);
    assert_eq!(out.text, "Units Alphan/2");
    assert_eq!(graph, before);
    for state in [999, u32::MAX] {
        let out = render(
            &fixture,
            "fixture_equipment_model={type=syn_arch}",
            S(state),
            content,
            &[],
        );
        assert!(out.text.is_empty());
        assert_eq!(out.refusals.values().sum::<usize>(), 1);
        assert!(
            out.refusals
                .keys()
                .all(|why| why.contains("invalid nonzero state")),
            "{:?}",
            out.refusals,
        );
    }
}

#[test]
fn navy_unknown_target_uses_reserved_country_name() {
    game::enter();
    let mut fixture = fixture();
    fixture.catalogue.localisation = fixture
        .catalogue
        .localisation
        .iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .chain([("SYN_ZERO_DEF".into(), "reserved navy target".into())])
        .collect::<Localisation>()
        .into();
    Arc::make_mut(&mut fixture.catalogue.localisation).language = "english".into();
    for (built, expected) in [(2, "NavyTo reserved navy target\n"), (0, "NavyTo \n")] {
        *fixture.frame.tables[TableId::ReservedCountryNaming.index()] =
            TableView::Complete(TableRows::ReservedCountryNaming(vec![
                ReservedCountryNamingRow {
                    tag: b"SYN_ZERO".to_vec(),
                    built,
                    ..Default::default()
                },
            ]));
        let out = render(
            &fixture,
            "fixture_move_fleet={target=ZZZ}",
            C(1),
            Content::default(),
            &[],
        );
        assert_eq!(out.text, expected, "built={built}");
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    }
}

#[test]
fn units_admission_and_named_refusals() {
    game::enter();
    let mut fixture = fixture();
    if let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *fixture.frame.tables[TableId::Provinces.index()]
    {
        rows[0].controller = 200;
    }
    *fixture.frame.tables[TableId::ProvinceUnitOrder.index()] =
        TableView::Complete(TableRows::ProvinceUnitOrder(vec![ProvinceUnitOrderRow {
            province: 1,
            mixed: vec![11],
            naval: vec![],
        }]));
    *fixture.frame.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(vec![DivisionRow {
            id: 11,
            province: 1,
            logical_owner: 200,
            roster_owner: 200,
            ..Default::default()
        }]));
    for (access, faction, subject, war, enemy, blocked, score, expected) in [
        (
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            "Unit Alphan",
        ),
        (
            true,
            false,
            false,
            false,
            false,
            false,
            false,
            "UnitAt Alphan/One",
        ),
        (
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            "UnitAt Alphan/One",
        ),
        (
            false,
            false,
            true,
            false,
            false,
            false,
            false,
            "UnitAt Alphan/One",
        ),
        (true, true, true, true, false, false, false, "Unit Alphan"),
        (
            false,
            false,
            false,
            true,
            true,
            false,
            false,
            "UnitAt Alphan/One",
        ),
        (
            false,
            false,
            false,
            true,
            true,
            false,
            true,
            "UnitAt Alphan/One",
        ),
        (true, false, false, false, false, true, false, "Unit Alphan"),
    ] {
        *fixture.frame.tables[TableId::RelationLinks.index()] =
            TableView::Complete(TableRows::RelationLinks(if access {
                vec![RelationLinkRow {
                    country: 200,
                    target_index: 1,
                    links: RelationLinkRow::MILITARY_ACCESS,
                    transport_subject: Default::default(),
                }]
            } else {
                vec![]
            }));
        *fixture.frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(if faction {
                vec![
                    FactionMemberRow {
                        country: 100,
                        faction: 1,
                        ordinal: 0,
                    },
                    FactionMemberRow {
                        country: 200,
                        faction: 1,
                        ordinal: 1,
                    },
                ]
            } else {
                vec![]
            }));
        *fixture.frame.tables[TableId::Subjects.index()] =
            TableView::Complete(TableRows::Subjects(if subject {
                vec![SubjectRow {
                    overlord: 100,
                    subject: 200,
                    start_hour: 0,
                    end_hour: 1,
                    cancel: 1,
                    ..Default::default()
                }]
            } else {
                vec![]
            }));
        *fixture.frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(if war {
                vec![WarRelationRow {
                    first_country: 100,
                    second_country: 200,
                    start_hour: 0,
                    end_hour: 1,
                    cancel: 1,
                    ..Default::default()
                }]
            } else {
                vec![]
            }));
        let body = format!(
            "fixture_raise_unit={{owner=AAA allow_spawning_on_enemy_provs={} {}}}",
            if enemy { "yes" } else { "no" },
            if score { "country_score={base=1}" } else { "" },
        );
        let out = render(
            &fixture,
            &body,
            S(1),
            Content::default(),
            if blocked { &[1] } else { &[] },
        );
        assert_eq!(out.text, expected, "{body}: {:?}", out.refusals);
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
    }
    for (body, class, reason) in [
        (
            "fixture_equipment_order={equipment={type=missing}}",
            "fixture_equipment_order",
            "equipment content unavailable",
        ),
        (
            "fixture_warehouse_delivery={type=missing amount=1}",
            "fixture_warehouse_delivery",
            "equipment content unavailable",
        ),
        (
            "fixture_equipment_credit={equipment_type=missing}",
            "fixture_equipment_credit",
            "equipment content unavailable",
        ),
        (
            "fixture_equipment_model={type=missing}",
            "fixture_equipment_model",
            "equipment content unavailable",
        ),
        (
            "fixture_scrap_ships={type=missing count=all}",
            "fixture_scrap_ships",
            "equipment content unavailable",
        ),
        (
            "fixture_equipment_delivery={type=missing target=BBB}",
            "fixture_equipment_delivery",
            "equipment content unavailable",
        ),
        (
            "fixture_allow_subunit=undefined",
            "fixture_allow_subunit",
            "content unavailable",
        ),
        (
            "fixture_move_fleet={target=ZZZ}",
            "fixture_move_fleet",
            "reserved country naming has no singleton row",
        ),
        ("fixture_raise_unit={}", "fixture_raise_unit", "reserved country zero"),
        (
            "fixture_raise_unit=THIS",
            "fixture_raise_unit",
            "the scalar create_unit body is rejected and retains constructor defaults",
        ),
    ] {
        let out = render(
            &fixture,
            &format!("{body} {body}"),
            C(1),
            Content::default(),
            &[],
        );
        assert!(out.text.is_empty(), "{body}: {}", out.text);
        assert_eq!(
            out.refusals.values().sum::<usize>(),
            2,
            "{body}: {:?}",
            out.refusals,
        );
        assert!(
            out.refusals
                .keys()
                .all(|key| key.contains(class) && key.contains(reason)),
            "{body}: {:?}",
            out.refusals,
        );
    }
    if let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *fixture.frame.tables[TableId::Provinces.index()]
    {
        rows[0].controller = 100;
        rows[1].state = 1;
        rows[1].controller = 200;
    }
    *fixture.frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 100,
            ..Default::default()
        }]));
    let out = render(
        &fixture,
        "fixture_raise_unit={owner=AAA country_score={base=1 modifier={factor=1 tag=BBB has_cosmetic_tag=syn_costume}}}",
        S(1),
        Content::default(),
        &[],
    );
    assert!(out.text.is_empty());
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("country appearance")),
        "{:?}",
        out.refusals,
    );
}

#[test]
fn units_have_no_country_count_cap() {
    game::enter();
    let mut fixture = equipment_fixture();
    fixture.set_countries(crate::effect_description::tests::synthetic_countries(24));
    let out = render(
        &fixture,
        "fixture_recruit_pool=2.5",
        C(24),
        Content::default(),
        &[],
    );
    assert_eq!(out.text, "Men Many/3\n", "{:?}", out.refusals);
    assert!(out.refusals.is_empty());
    let equipment = equipment();
    let content = Content {
        equipment: Some(&equipment),
        ..Default::default()
    };
    for (body, expected) in [
        (
            "fixture_equipment_delivery={type=syn_arch target=AAA amount=1}",
            "SendOld 1/Alpha gear Many/Alpha Many\n",
        ),
        (
            "fixture_equipment_model={type=syn_arch allow_without_tech=yes}",
            "LegacyTechUnnamed Base short",
        ),
    ] {
        let out = render(&fixture, body, C(24), content, &[]);
        assert_eq!(out.text, expected, "{body}: {:?}", out.refusals);
        assert!(out.refusals.is_empty());
    }
}

fn equipment() -> content_reader_key_names::equipment::Database {
    content_reader_key_names::equipment::fixture::load(&[
        (
            "common/units/equipment/modules/synthetic.txt",
            "equipment_modules={syn_carrier={add_equipment_type=carrier} syn_sub={add_equipment_type=submarine}}",
        ),
        (
            "common/units/equipment/synthetic.txt",
            "equipments={
                syn_arch={is_archetype=yes type=infantry alias=syn_alias}
                syn_concrete={archetype=syn_arch family=syn_arch}
                syn_ship={is_archetype=yes type=capital_ship}
                syn_basic_hull={archetype=syn_ship family=syn_ship module_slots={main={}}}
                syn_hull={archetype=syn_ship family=syn_ship module_slots={main={}} default_modules={main=syn_carrier}}
            }",
        ),
    ])
    .0
}

fn equipment_fixture() -> Fixture {
    let mut fixture = fixture();
    fixture.catalogue.localisation = fixture
        .catalogue
        .localisation
        .iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .chain(
            [
                ("syn_arch", "Base gear [This.GetName]"),
                ("AAA_syn_arch", "Alpha gear [This.GetName]"),
                ("---_syn_arch", "Zero gear [This.GetName]"),
                ("syn_concrete", "Base model"),
                ("BBB_syn_concrete", "Beta model"),
                ("syn_arch_short", "Base short"),
                ("AAA_syn_arch_short", "Alpha short"),
                ("syn_concrete_short", "Model short"),
                ("syn_ship_short", "Ship short"),
                ("AAA_syn_ship_short", "Alpha ship short"),
                ("syn_hull_short", "Hull short"),
                ("syn_basic_hull", "Basic hull"),
                ("AAA_syn_basic_hull", "Alpha hull"),
                ("AAA_", "Null type"),
                ("ship_designer_capital_ship", "Capital"),
                ("ship_designer_capital_ship_syn_ship", "Specific capital"),
                ("ship_designer_carrier", "Carrier"),
                (
                    "EFFECT_ADD_EQUIPMENT_TO_STOCKPILE",
                    "Store $AMOUNT|0$/$EQUIPMENT$",
                ),
                (
                    "EFFECT_REMOVE_EQUIPMENT_FROM_STOCKPILE",
                    "Take $AMOUNT|0$/$EQUIPMENT$",
                ),
                (
                    "EFFECT_ADD_EQUIPMENT_TO_STOCKPILE_FOREIGN_PRODUCER",
                    "StoreForeign $AMOUNT|0$/$EQUIPMENT$/$PRODUCER$",
                ),
                (
                    "EFFECT_REMOVE_EQUIPMENT_FROM_STOCKPILE_FOREIGN_PRODUCER",
                    "TakeForeign $AMOUNT|0$/$EQUIPMENT$/$PRODUCER$",
                ),
                (
                    "EFFECT_SEND_EQUIPMENT_OLD",
                    "SendOld $AMOUNT|0$/$EQUIPMENT_NAME$/$COUNTRY$ [This.GetName]",
                ),
                (
                    "EFFECT_SEND_EQUIPMENT_NEW",
                    "SendNew $AMOUNT|0$/$EQUIPMENT_NAME$/$COUNTRY$ [This.GetName]",
                ),
                ("EFFECT_DESTROY_SHIP", "DestroyOne $COUNT$/$TYPE$"),
                ("EFFECT_DESTROY_SHIPS", "DestroyMany $COUNT$/$TYPE$"),
                ("EFFECT_DESTROY_SHIPS_ALL", "all [This.GetName]"),
                (
                    "EFFECT_CREATE_EQUIPMENT_VARIANT_LEGACY",
                    "LegacyTech $TYPE$/$NAME$",
                ),
                (
                    "EFFECT_CREATE_EQUIPMENT_VARIANT_LEGACY_NO_NAME",
                    "LegacyTechUnnamed $TYPE$",
                ),
                (
                    "EFFECT_CREATE_EQUIPMENT_VARIANT_LEGACY_NO_TECH",
                    "LegacyNoTech $TYPE$/$NAME$",
                ),
                (
                    "EFFECT_CREATE_EQUIPMENT_VARIANT_LEGACY_NO_TECH_NO_NAME",
                    "LegacyNoTechUnnamed $TYPE$",
                ),
                (
                    "EFFECT_CREATE_EQUIPMENT_VARIANT",
                    "ModularTech $TYPE$/$NAME$/$ROLE$",
                ),
                (
                    "EFFECT_CREATE_EQUIPMENT_VARIANT_NO_NAME",
                    "ModularTechUnnamed $TYPE$/$ROLE$",
                ),
                (
                    "EFFECT_CREATE_EQUIPMENT_VARIANT_NO_TECH",
                    "ModularNoTech $TYPE$/$NAME$/$ROLE$",
                ),
                (
                    "EFFECT_CREATE_EQUIPMENT_VARIANT_NO_TECH_NO_NAME",
                    "ModularNoTechUnnamed $TYPE$/$ROLE$",
                ),
                (
                    "EFFECT_MARKET_ADD_EQUIPMENT_SUBSIDY",
                    "Subsidy $COUNTRY$/$CIC|2$/$TYPE$/$SELLERS$",
                ),
                ("LIST_OR", "|"),
            ]
            .into_iter()
            .map(|(key, value)| (key.into(), value.into())),
        )
        .collect::<Localisation>()
        .into();
    Arc::make_mut(&mut fixture.catalogue.localisation).language = "english".into();
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(
            [
                (500, "syn_concrete"),
                (501, "syn_hull"),
                (502, "syn_arch"),
                (503, "syn_basic_hull"),
                (u32::MAX, "syn_arch"),
            ]
            .into_iter()
            .map(|(token, literal)| LexicalTokenRow {
                token,
                literal: literal.as_bytes().to_vec(),
            })
            .collect(),
        ));
    if let TableView::Complete(TableRows::ScriptVariables(rows)) =
        &mut *fixture.frame.tables[TableId::ScriptVariables.index()]
    {
        rows[0].entries.extend(
            [
                ("syn_type", 50_200_000),
                ("syn_fraction", 50_299_999),
                ("syn_negative", -100_001),
                ("syn_missing", 99_900_000),
            ]
            .into_iter()
            .map(|(name, value_raw)| ScriptVariableEntry {
                name: name.as_bytes().to_vec(),
                value_raw,
            }),
        );
    }
    *fixture.frame.tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            dlc_mask: 1 << 4,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![
            VariantDefinitionRow {
                id: 10,
                equipment_token: 500,
                creator_link: 100,
                display_name: b"First [This.GetName]".to_vec(),
                variant_name: b"Mark".to_vec(),
                append_variant_name: 1,
                ..Default::default()
            },
            VariantDefinitionRow {
                id: 20,
                equipment_token: 500,
                display_name: b"Later".to_vec(),
                ..Default::default()
            },
            VariantDefinitionRow {
                id: 30,
                equipment_token: 500,
                creator_link: 100,
                origin_link: 200,
                variant_name: b"Foreign".to_vec(),
                append_variant_name: 1,
                ..Default::default()
            },
            VariantDefinitionRow {
                id: 40,
                equipment_token: 501,
                display_name: b"Guarded".to_vec(),
                ..Default::default()
            },
            VariantDefinitionRow {
                id: 50,
                equipment_token: 501,
                display_name: b"Ship".to_vec(),
                classifier_flags: 1,
                ..Default::default()
            },
            VariantDefinitionRow {
                id: 60,
                equipment_token: 502,
                creator_link: 200,
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::AvailableEquipmentVariants.index()] =
        TableView::Complete(TableRows::AvailableEquipmentVariants(
            [
                (100, 0, 2, 20),
                (100, 0, 0, 10),
                (200, 0, 0, 30),
                (100, 0, 3, 40),
                (200, 0, 1, 50),
                (100, 0, 4, 60),
                (100, 1, 0, 50),
            ]
            .into_iter()
            .map(
                |(country, roster_kind, ordinal, variant)| AvailableEquipmentVariantRow {
                    country,
                    roster_kind,
                    ordinal,
                    variant,
                },
            )
            .collect(),
        ));
    fixture
}

#[test]
fn units_equipment_branches() {
    game::enter();
    let fixture = equipment_fixture();
    let equipment = equipment();
    let names = content_reader_script::key_names::KeyNames::default();
    let content = Content {
        equipment: Some(&equipment),
        key_names: Some(&names),
        ..Default::default()
    };
    for (body, object, expected) in [
        (
            "fixture_equipment_order={equipment={type=syn_arch}}",
            C(1),
            "Produce Zero gear Alpha Alpha",
        ),
        (
            "fixture_equipment_order={equipment={type=syn_arch creator=AAA}}",
            C(2),
            "Produce Alpha gear Beta Beta",
        ),
        (
            "fixture_equipment_order={equipment={type=syn_concrete creator=BBB} equipment={version_name=\"\"}}",
            C(1),
            "Produce Beta model Alpha",
        ),
        (
            "fixture_equipment_order={equipment={type=syn_arch creator=ROOT}}",
            C(1),
            "Produce Zero gear Alpha Alpha",
        ),
        (
            "fixture_equipment_order={equipment={type=syn_arch version_name=\"Raw [This.GetName]\"}}",
            C(2),
            "Produce Raw Beta Beta",
        ),
        (
            "fixture_warehouse_delivery={type=syn_alias amount=2.4}",
            C(2),
            "Store 2/Alpha gear Beta\n",
        ),
        (
            "fixture_warehouse_delivery={type=syn_arch amount=0}",
            C(1),
            "Take 0/Alpha gear Alpha\n",
        ),
        (
            "fixture_warehouse_delivery={type=syn_arch amount=1}",
            S(1),
            "Store 1/Alpha gear One\n",
        ),
        (
            "fixture_warehouse_delivery={type=syn_arch amount=-1.5 producer=BBB}",
            C(1),
            "TakeForeign -2/Alpha gear Alpha/Betan\n",
        ),
        (
            "fixture_warehouse_delivery={type=syn_arch amount=1 producer=BBB}",
            C(1),
            "StoreForeign 1/Alpha gear Alpha/Betan\n",
        ),
        (
            "fixture_warehouse_delivery={type=syn_concrete amount=1 variant_name=Ignored}",
            C(1),
            "Store 1/First Alpha Mark\n",
        ),
        (
            "fixture_warehouse_delivery={type=syn_concrete amount=1 producer=BBB}",
            C(1),
            "StoreForeign 1/Beta model Foreign/Betan\n",
        ),
        (
            "fixture_warehouse_delivery={type=syn_concrete amount=1 producer=BBB}",
            S(1),
            "StoreForeign 1/Beta model Foreign/Betan\n",
        ),
        (
            "fixture_warehouse_delivery={type=syn_hull amount=1}",
            C(1),
            "",
        ),
        (
            "fixture_warehouse_delivery={type=syn_hull amount=1 producer=BBB}",
            C(1),
            "StoreForeign 1/Ship/Betan\n",
        ),
        (
            "fixture_warehouse_delivery={type=syn_basic_hull amount=1}",
            C(1),
            "",
        ),
        (
            "fixture_warehouse_delivery={type=var:syn_type amount=1}",
            C(1),
            "Store 1/Alpha gear Alpha\n",
        ),
        (
            "fixture_warehouse_delivery={type=var:syn_fraction amount=1}",
            C(1),
            "Store 1/Alpha gear Alpha\n",
        ),
        (
            "fixture_warehouse_delivery={type=var:syn_negative amount=1}",
            C(1),
            "Store 1/Alpha gear Alpha\n",
        ),
        (
            "fixture_warehouse_delivery={type=var:syn_type type=syn_hull amount=1}",
            C(1),
            "Store 1/Alpha gear Alpha\n",
        ),
        (
            "fixture_warehouse_delivery={type=var:syn_negative type=syn_type amount=1}",
            C(1),
            "Store 1/Alpha gear Alpha\n",
        ),
        (
            "fixture_warehouse_delivery={type=syn_arch type=missing amount=1}",
            C(1),
            "",
        ),
        (
            "fixture_warehouse_delivery={type=syn_type amount=1}",
            C(1),
            "",
        ),
        (
            "fixture_warehouse_delivery={type=VAR:syn_type amount=1}",
            C(1),
            "",
        ),
        (
            "fixture_equipment_delivery={type=syn_arch target=BBB amount=syn_amount old_prioritised=yes}",
            C(1),
            "SendOld 2/Alpha gear Alpha/Beta Alpha\n",
        ),
        (
            "fixture_equipment_delivery={equipment=syn_arch target=BBB amount=-1.5 new_prioritised=yes}",
            C(1),
            "SendNew -2/Alpha gear Alpha/Beta Alpha\n",
        ),
        (
            "fixture_equipment_delivery={type=var:syn_type equipment=syn_hull target=BBB amount=1}",
            C(1),
            "SendOld 1/Alpha gear Alpha/Beta Alpha\n",
        ),
        (
            "fixture_equipment_delivery={type=missing target=ZZZ amount=not_recorded}",
            C(1),
            "",
        ),
        (
            "fixture_scrap_ships={type=syn_arch count=1.5}",
            C(1),
            "DestroyOne 1/Alpha gear Alpha\n",
        ),
        (
            "fixture_scrap_ships={type=syn_arch count=0x10}",
            C(1),
            "DestroyMany 0/Alpha gear Alpha\n",
        ),
        (
            "fixture_scrap_ships={type=syn_arch count=all}",
            C(1),
            "DestroyMany all Alpha/Alpha gear Alpha\n",
        ),
        (
            "fixture_scrap_ships={type=syn_arch count=\"all\"}",
            C(1),
            "DestroyMany 0/Alpha gear Alpha\n",
        ),
        (
            "fixture_scrap_ships={type=syn_arch count=-2}",
            C(1),
            "DestroyMany all Alpha/Alpha gear Alpha\n",
        ),
        (
            "fixture_scrap_ships={type=syn_arch count=syn_amount}",
            C(1),
            "DestroyMany 0/Alpha gear Alpha\n",
        ),
        (
            "fixture_equipment_model={type=syn_concrete name=Raw allow_without_tech=yes}",
            C(1),
            "LegacyTech Model short/Raw",
        ),
        (
            "fixture_equipment_model={type=syn_concrete allow_without_tech=yes}",
            C(1),
            "LegacyTechUnnamed Model short",
        ),
        (
            "fixture_equipment_model={type=syn_concrete name=Raw}",
            C(1),
            "LegacyNoTech Model short/Raw",
        ),
        (
            "fixture_equipment_model={type=syn_concrete}",
            C(1),
            "LegacyNoTechUnnamed Model short",
        ),
        (
            "fixture_equipment_model={type=syn_basic_hull name=Raw allow_without_tech=yes}",
            C(1),
            "ModularTech Alpha ship short/Raw/Specific capital",
        ),
        (
            "fixture_equipment_model={type=syn_basic_hull allow_without_tech=yes}",
            C(1),
            "ModularTechUnnamed Alpha ship short/Specific capital",
        ),
        (
            "fixture_equipment_model={type=syn_basic_hull name=Raw}",
            C(1),
            "ModularNoTech Alpha ship short/Raw/Specific capital",
        ),
        (
            "fixture_equipment_model={type=syn_basic_hull}",
            C(1),
            "ModularNoTechUnnamed Alpha ship short/Specific capital",
        ),
        (
            "fixture_equipment_model={type=syn_basic_hull allow_without_tech=yes}",
            S(1),
            "ModularTechUnnamed Alpha ship short/Specific capital",
        ),
        (
            "fixture_equipment_model={type=syn_basic_hull allow_without_tech=yes}",
            C(2),
            "ModularTechUnnamed Ship short/Specific capital",
        ),
        (
            "fixture_equipment_model={type=syn_arch allow_without_tech=yes}",
            C(1),
            "LegacyTechUnnamed Alpha short",
        ),
        (
            "fixture_equipment_model={type=syn_hull modules={main=empty} allow_without_tech=yes}",
            C(1),
            "ModularTechUnnamed Alpha ship short/Carrier",
        ),
        (
            "fixture_equipment_model={type=syn_hull modules={main=syn_sub} allow_without_tech=yes}",
            C(1),
            "ModularTechUnnamed Alpha ship short/Carrier",
        ),
        (
            "fixture_equipment_model={type=syn_basic_hull modules={main=syn_carrier MAIN=syn_sub} allow_without_tech=yes}",
            C(1),
            "ModularTechUnnamed Alpha ship short/Specific capital",
        ),
        (
            "fixture_equipment_model={type=syn_basic_hull modules={main=syn_carrier} modules={MAIN=missing} allow_without_tech=yes}",
            C(1),
            "ModularTechUnnamed Alpha ship short/Carrier",
        ),
        ("fixture_equipment_model={type=syn_arch}", S(0), ""),
        ("fixture_equipment_model={type=syn_arch}", S(5), ""),
        ("fixture_equipment_model={type=syn_arch}", C(0), ""),
        ("fixture_equipment_model={type=missing}", S(999), ""),
        (
            "fixture_equipment_credit={equipment_type=syn_arch cic=syn_amount}",
            C(1),
            "Subsidy Alpha/2.49/Alpha gear Alpha/\n",
        ),
        (
            "fixture_equipment_credit={equipment_type=syn_arch seller_tags={BBB AAA} seller_tags={BBB ZZZ} seller_trigger=syn_trigger}",
            C(1),
            "Subsidy Alpha/0.00/Alpha gear Alpha/Beta|Alpha|Beta\n",
        ),
        (
            "fixture_equipment_credit={equipment_type=syn_arch seller_tags={ZZZ} seller_trigger=\"\"}",
            C(1),
            "Subsidy Alpha/0.00/Alpha gear Alpha/\n",
        ),
        (
            "fixture_equipment_credit={equipment_type=missing}",
            C(1),
            "Subsidy Alpha/0.00/Null type/\n",
        ),
        (
            "BBB={fixture_equipment_delivery={type=syn_arch target=AAA amount=1} fixture_scrap_ships={type=syn_arch count=1}}",
            C(1),
            "\u{12}BBBBeta:\n   SendOld 1/Alpha gear Beta/Alpha Beta\n   DestroyOne 1/Alpha gear Beta\n",
        ),
    ] {
        let out = render(&fixture, body, object, content, &[]);
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
        assert_eq!(out.text, expected, "{body}");
    }
    let mut fixture = fixture;
    if let TableView::Complete(TableRows::SimulationFeatures(rows)) =
        &mut *fixture.frame.tables[TableId::SimulationFeatures.index()]
    {
        rows[0].dlc_mask = 0;
    }
    let out = render(
        &fixture,
        "fixture_equipment_model={type=syn_basic_hull allow_without_tech=yes}",
        C(1),
        content,
        &[],
    );
    assert_eq!(out.text, "LegacyTechUnnamed Alpha hull");
    assert!(out.refusals.is_empty());
}

#[test]
fn units_equipment_named_refusals() {
    game::enter();
    let mut fixture = equipment_fixture();
    let equipment = equipment();
    let content = Content {
        equipment: Some(&equipment),
        ..Default::default()
    };
    for (body, reason) in [
        (
            "fixture_warehouse_delivery={type={var=syn_type} amount=1}",
            "equipment-type block operand",
        ),
        (
            "fixture_warehouse_delivery={type=var:syn_missing amount=1}",
            "lexical tokens lacks equipment variable token 999",
        ),
        (
            "fixture_equipment_credit={equipment_type=syn_arch seller_tags={ZZZ} seller_trigger=syn_trigger}",
            "trigger-template description",
        ),
    ] {
        let out = render(&fixture, &format!("{body} {body}"), C(1), content, &[]);
        assert!(out.text.is_empty());
        assert_eq!(
            out.refusals.values().sum::<usize>(),
            2,
            "{body}: {:?}",
            out.refusals,
        );
        assert!(
            out.refusals.keys().all(|why| why.contains(reason)),
            "{body}: {:?}",
            out.refusals,
        );
    }
    for object in [C(0), S(1)] {
        let out = render(
            &fixture,
            "fixture_warehouse_delivery={type=syn_concrete amount=1}",
            object,
            content,
            &[],
        );
        assert!(out.text.is_empty());
        assert_eq!(out.refusals.values().sum::<usize>(), 1);
        assert!(
            out.refusals
                .keys()
                .all(|why| why.contains("direct country-zero production roster")),
            "{:?}",
            out.refusals,
        );
    }
    let out = render(
        &fixture,
        "fixture_warehouse_delivery={type=syn_concrete amount=1}",
        C(9999),
        content,
        &[],
    );
    assert!(out.text.is_empty());
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(
        out.refusals
            .keys()
            .all(|why| why.contains("equipment roster country slot 9999"))
    );
    if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
        &mut *fixture.frame.tables[TableId::VariantDefinitions.index()]
    {
        let variant = rows.iter_mut().find(|row| row.id == 60).unwrap();
        variant.creator_link = 100;
        variant.version = Recorded::NotRecorded;
    }
    let out = render(
        &fixture,
        "fixture_equipment_model={type=syn_concrete}",
        C(1),
        content,
        &[],
    );
    assert!(out.text.is_empty());
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(
        out.refusals
            .keys()
            .all(|why| why.contains("version of variant definitions")),
        "{:?}",
        out.refusals,
    );
    let out = render(
        &fixture,
        "fixture_equipment_model={type=syn_concrete allow_without_tech=yes}",
        C(1),
        content,
        &[],
    );
    assert_eq!(out.text, "LegacyTechUnnamed Model short");
    assert!(out.refusals.is_empty());
    if let TableView::Complete(TableRows::AvailableEquipmentVariants(rows)) =
        &mut *fixture.frame.tables[TableId::AvailableEquipmentVariants.index()]
    {
        rows.push(AvailableEquipmentVariantRow {
            country: 100,
            roster_kind: 0,
            ordinal: 99,
            variant: 9999,
        });
    }
    let out = render(
        &fixture,
        "fixture_warehouse_delivery={type=syn_concrete amount=1}",
        C(1),
        content,
        &[],
    );
    assert_eq!(out.text, "Store 1/First Alpha Mark\n");
    assert!(out.refusals.is_empty());
}

#[test]
fn units_recorded_variant_no_tech_probe() {
    game::enter();
    let equipment = equipment();
    let content = Content {
        equipment: Some(&equipment),
        ..Default::default()
    };
    let mut fixture = equipment_fixture();
    crate::test_country_links::extend(
        &mut fixture.frame,
        [CountryLinkRow {
            id: 101,
            canonical_slot: 1,
        }],
    );
    let definitions = fixture.frame.tables[TableId::VariantDefinitions.index()].clone();
    let available = fixture.frame.tables[TableId::AvailableEquipmentVariants.index()].clone();
    for (version, creator, origin, expected) in [
        (0, 100, 0, "LegacyTechUnnamed Model short"),
        (0, 101, 0, "LegacyTechUnnamed Model short"),
        (0, 100, u32::MAX, "LegacyTechUnnamed Model short"),
        (1, 100, 0, "LegacyNoTechUnnamed Model short"),
        (255, 100, 0, "LegacyNoTechUnnamed Model short"),
        (0, 200, 0, "LegacyNoTechUnnamed Model short"),
        (0, 0, 0, "LegacyNoTechUnnamed Model short"),
        (0, 100, 200, "LegacyNoTechUnnamed Model short"),
    ] {
        fixture.frame.tables[TableId::VariantDefinitions.index()] = definitions.clone();
        fixture.frame.tables[TableId::AvailableEquipmentVariants.index()] = available.clone();
        if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
            &mut *fixture.frame.tables[TableId::VariantDefinitions.index()]
        {
            let variant = rows.iter_mut().find(|row| row.id == 60).unwrap();
            variant.version = Recorded::Value(version);
            variant.creator_link = creator;
            variant.origin_link = origin;
            if expected.contains("LegacyTechUnnamed") {
                let mut later = variant.clone();
                later.id = 70;
                later.version = Recorded::NotRecorded;
                rows.push(later);
            }
        }
        if expected.contains("LegacyTechUnnamed")
            && let TableView::Complete(TableRows::AvailableEquipmentVariants(rows)) =
                &mut *fixture.frame.tables[TableId::AvailableEquipmentVariants.index()]
        {
            rows.push(AvailableEquipmentVariantRow {
                country: 100,
                roster_kind: 0,
                ordinal: 99,
                variant: 70,
            });
        }
        let out = render(
            &fixture,
            "fixture_equipment_model={type=syn_concrete}",
            C(1),
            content,
            &[],
        );
        assert!(
            out.refusals.is_empty(),
            "{version}/{creator}/{origin}: {:?}",
            out.refusals,
        );
        assert_eq!(out.text, expected, "{version}/{creator}/{origin}");
    }
    let mut fixture = equipment_fixture();
    if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
        &mut *fixture.frame.tables[TableId::VariantDefinitions.index()]
    {
        rows.iter_mut().find(|row| row.id == 60).unwrap().version = Recorded::NotRecorded;
    }
    let out = render(
        &fixture,
        "fixture_equipment_model={type=syn_concrete} fixture_equipment_model={type=syn_concrete}",
        C(1),
        content,
        &[],
    );
    assert!(out.text.is_empty());
    assert_eq!(out.refusals.values().sum::<usize>(), 2);
    assert!(
        out.refusals
            .keys()
            .all(|why| why.contains("version of variant definitions") && why.contains("not recorded"))
    );
    if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
        &mut *fixture.frame.tables[TableId::VariantDefinitions.index()]
    {
        rows.iter_mut()
            .find(|row| row.id == 60)
            .unwrap()
            .equipment_token = 500;
    }
    let out = render(
        &fixture,
        "fixture_equipment_model={type=syn_concrete}",
        C(1),
        content,
        &[],
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert_eq!(out.text, "LegacyNoTechUnnamed Model short");
}

#[test]
fn units_recorded_variant_pool_order_and_equality() {
    game::enter();
    let equipment = equipment();
    let content = Content {
        equipment: Some(&equipment),
        ..Default::default()
    };
    let mut fixture = equipment_fixture();
    fixture.catalogue.localisation = fixture
        .catalogue
        .localisation
        .iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .chain([
            (
                "INTERNATIONAL_MARKET_EQUIPMENT_AMOUNT".into(),
                "$AMOUNT$ $EQUIPMENT$".into(),
            ),
            (
                "EFFECT_MARKET_CREATE_PURCHASE_CONTRACT".into(),
                "$EQUIPMENT$".into(),
            ),
        ])
        .collect::<Localisation>()
        .into();
    Arc::make_mut(&mut fixture.catalogue.localisation).language = "english".into();
    crate::test_country_links::extend(
        &mut fixture.frame,
        [
            CountryLinkRow {
                id: 101,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 201,
                canonical_slot: 2,
            },
        ],
    );
    *fixture.frame.tables[TableId::AvailableEquipmentVariants.index()] =
        TableView::Complete(TableRows::AvailableEquipmentVariants(vec![
            AvailableEquipmentVariantRow {
                country: 100,
                variant: 10,
                ..Default::default()
            },
            AvailableEquipmentVariantRow {
                country: 200,
                variant: 20,
                ..Default::default()
            },
        ]));
    for (
        left_version,
        right_version,
        right_type,
        left_creator,
        right_creator,
        left_origin,
        right_origin,
        expected,
    ) in [
        (2, 1, "syn_concrete", 100, 100, 0, 0, "3 Right, 2 Left\n"),
        (0, 255, "syn_concrete", 100, 100, 0, 0, "2 Left, 3 Right\n"),
        (0, 0, "syn_arch", 0, 0, 0, 0, "3 Right, 2 Left\n"),
        (0, 0, "syn_concrete", 200, 100, 0, 0, "3 Right, 2 Left\n"),
        (0, 0, "syn_concrete", 100, 100, 200, 0, "3 Right, 2 Left\n"),
        (0, 0, "syn_concrete", 100, 101, 0, 0, "5 Left\n"),
        (0, 0, "syn_concrete", 100, 100, 200, 201, "5 Left\n"),
    ] {
        *fixture.frame.tables[TableId::VariantDefinitions.index()] =
            TableView::Complete(TableRows::VariantDefinitions(vec![
                VariantDefinitionRow {
                    id: 10,
                    equipment_token: 500,
                    creator_link: left_creator,
                    origin_link: left_origin,
                    display_name: b"Left".to_vec(),
                    version: Recorded::Value(left_version),
                    ..Default::default()
                },
                VariantDefinitionRow {
                    id: 20,
                    equipment_token: if right_type == "syn_arch" { 502 } else { 500 },
                    creator_link: right_creator,
                    origin_link: right_origin,
                    display_name: b"Right".to_vec(),
                    version: Recorded::Value(right_version),
                    ..Default::default()
                },
            ]));
        let body = format!(
            "fixture_purchase_order={{seller=AAA buyer=BBB equipment={{type=syn_concrete producer=AAA amount=2}} equipment={{type={right_type} producer=BBB amount=3}}}}"
        );
        let out = render(&fixture, &body, C(1), content, &[]);
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
        assert_eq!(
            out.text, expected,
            "{left_version}/{right_version}/{left_creator}/{right_creator}/{left_origin}/{right_origin}"
        );
        if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
            &mut *fixture.frame.tables[TableId::VariantDefinitions.index()]
        {
            rows[1].version = Recorded::NotRecorded;
        }
        let out = render(&fixture, &format!("{body} {body}"), C(1), content, &[]);
        assert!(out.text.is_empty());
        assert_eq!(out.refusals.values().sum::<usize>(), 2);
        assert!(
            out.refusals
                .keys()
                .all(|why| why.contains("version of variant definitions")
                    && why.contains("not recorded")),
            "{:?}",
            out.refusals,
        );
    }
}

#[test]
fn units_equipment_role_branches() {
    let cases = [
        ("fixture_capital_ship", role::CAPITAL_SHIP, "ship_designer_capital_ship"),
        ("fixture_submarine", role::SUBMARINE, "ship_designer_submarine"),
        ("fixture_screen", role::SCREEN, "ship_designer_screen"),
        ("fixture_carrier", role::CARRIER, "ship_designer_carrier"),
        ("fixture_capital_carrier", role::CAPITAL_CARRIER, "ship_designer_carrier"),
        ("fixture_tank", role::TANK, "tank_designer_"),
        ("fixture_fighter", role::FIGHTER, "plane_designer_fighter"),
        ("fixture_interceptor", role::INTERCEPTOR, "plane_designer_interceptor"),
        ("fixture_tactical_bomber", role::TACTICAL_BOMBER, "plane_designer_tactical_bomber"),
        ("fixture_strategic_bomber", role::STRATEGIC_BOMBER, "plane_designer_strategic_bomber"),
        ("fixture_close_air_support", role::CLOSE_AIR_SUPPORT, "plane_designer_cas"),
        ("fixture_naval_bomber", role::NAVAL_BOMBER, "plane_designer_naval_bomber"),
        ("fixture_suicide_aircraft", role::SUICIDE_AIRCRAFT, "plane_designer_suicide"),
        ("fixture_scout_plane", role::SCOUT_PLANE, "plane_designer_scout_plane"),
        ("fixture_maritime_patrol", role::MARITIME_PATROL, "plane_designer_maritime_patrol_plane"),
        ("fixture_flame_tank", role::FLAME_TANK, "tank_designer_flame"),
        ("fixture_amphibious_tank", role::AMPHIBIOUS_TANK, "tank_designer_amphibious"),
        ("fixture_anti_air_tank", role::ANTI_AIR_TANK, "tank_designer_anti_air"),
        ("fixture_artillery_tank", role::ARTILLERY_TANK, "tank_designer_artillery"),
        ("fixture_anti_tank", role::ANTI_TANK, "tank_designer_anti_tank"),
        ("fixture_rocket_tank", role::ROCKET_TANK, "tank_designer_rocket"),
        ("fixture_heavy_fighter", role::HEAVY_FIGHTER, "plane_designer_heavy_fighter"),
        ("fixture_mixed", role::FIGHTER | role::TANK, "unknown"),
    ];
    let game = game::executable().words(cases.iter().map(|&(name, _, _)| name));
    let categories: Vec<_> = cases.iter().map(|&(name, mask, _)| {
        (game.token(name).expect("registered equipment category"), mask, name)
    }).collect();
    game.auxiliary(|tables| {
        tables.category_names = Ok(categories.iter().map(|&(_, mask, name)| (name, mask)).collect());
        let families = tables.category_families.as_mut().unwrap();
        families.plane_designer = cases.iter()
            .filter(|(_, _, key)| key.starts_with("plane_designer_"))
            .fold(0, |mask, (_, role, _)| mask | role);
        families.ship_designer = role::SHIP_CATEGORIES;
        families.tank_designer = role::TANK;
        tables.equipment_categories = Ok(categories);
    }).enter();
    let mut source = String::from("equipments={");
    let mut localisation = Vec::new();
    let mut expected = Vec::new();
    for (index, (category, _, key)) in cases.into_iter().enumerate() {
        let name = format!("syn_role_{index}");
        source.push_str(&format!(
            "{name}={{is_archetype=yes type={{{category}}} module_slots={{main={{}}}}}}"
        ));
        let key = if key == "tank_designer_" {
            format!("{key}{name}")
        } else {
            key.to_owned()
        };
        localisation.push((format!("{name}_short"), "Role type".to_owned()));
        localisation.push((key.clone(), key.clone()));
        expected.push((name, key));
    }
    source.push('}');
    let equipment = content_reader_key_names::equipment::fixture::load(&[(
        "common/units/equipment/synthetic.txt",
        &source,
    )])
    .0;
    let mut fixture = equipment_fixture();
    fixture.catalogue.localisation = fixture
        .catalogue
        .localisation
        .iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .chain(localisation)
        .collect::<Localisation>()
        .into();
    Arc::make_mut(&mut fixture.catalogue.localisation).language = "english".into();
    if let TableView::Complete(TableRows::SimulationFeatures(rows)) =
        &mut *fixture.frame.tables[TableId::SimulationFeatures.index()]
    {
        rows[0].dlc_mask = (1 << 4) | (1 << 7) | (1 << 8);
    }
    let content = Content {
        equipment: Some(&equipment),
        ..Default::default()
    };
    for (name, key) in expected {
        let body = format!("fixture_equipment_model={{type={name} allow_without_tech=yes}}");
        let out = render(&fixture, &body, C(1), content, &[]);
        assert_eq!(
            out.text,
            format!("ModularTechUnnamed Role type/{key}"),
            "{body}: {:?}",
            out.refusals,
        );
        assert!(out.refusals.is_empty());
    }
}
