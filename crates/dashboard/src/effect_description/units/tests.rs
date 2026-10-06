use super::*;
use crate::{effect_description::tests::Fixture, scripted_text::EventScope, triggers::ContentFacts};
use EventObject::{Country as C, State as S};
use replay::{frame::TableView, rows::*};
use std::collections::BTreeSet;

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
                ("EFFECT_SET_TEMPLATE_FORCE_ALLOW_RECRUITING", "Recruit $NAME$/$ALLOW$"),
                ("EFFECT_DISABLE_ALL_TEMPLATE_EDITING", "LockAll"),
                ("EFFECT_ENABLE_ALL_TEMPLATE_EDITING", "UnlockAll"),
                ("EFFECT_TRANSFER_NAVY", "NavyTo $COUNTRY$"),
                ("EFFECT_TRANSFER_UNITS", "UnitsTo $RECIPIENT$"),
                ("EFFECT_SEND_EQUIPMENT_FRACTION", "Fraction $FRACTION|2$/$COUNTRY$"),
                ("EFFECT_EQUIPMENT_FRACTION", "Adjust $VAL|2$"),
                (
                    "EFFECT_ADD_DESIGN_TEMPLATE_BONUS",
                    "Design $DISCOUNT|2$/$USES|0$/$NAME$ [This.GetName]",
                ),
                ("syn_bonus", "Bonus [This.GetName]"),
                ("EFFECT_ADD_EQUIPMENT_PRODUCTION", "Produce $EQUIPMENT$ [This.GetName]"),
                ("EFFECT_ADD_UNITS_TO_DIVISION_TEMPLATE", "Add $COUNTRY$/$TEMPLATE_NAME$"),
                ("EFFECT_SET_OOB", "OOB $NAME$ [This.GetName]"),
                ("EFFECT_CREATE_SHIP", "Ship $VARIANT$/$COUNTRY$ [This.GetName]"),
                ("EFFECT_CREATE_UNIT", "Unit $OWNER$"),
                ("EFFECT_CREATE_UNIT_WITH_COUNT", "Units $OWNER$/$COUNT|0$"),
                ("EFFECT_CREATE_UNIT_LOCATION", "UnitAt $OWNER$/$STATE$"),
                ("EFFECT_CREATE_UNIT_LOCATION_WITH_COUNT", "UnitsAt $OWNER$/$COUNT|0$/$STATE$"),
                ("EFFECT_RAID_DAMAGE_PLANE", "Plane $VAL|2$"),
                ("EFFECT_RAID_DAMAGE_PLANE_RATIO", "PlaneRatio $VAL|2$"),
                ("EFFECT_RAID_DAMAGE_UNIT_STRENGTH", "Strength $VAL|2$"),
                ("EFFECT_RAID_DAMAGE_UNIT_STRENGTH_RATIO", "StrengthRatio $VAL|2$"),
                ("EFFECT_RAID_DAMAGE_UNIT_ORG", "Org $VAL|2$"),
                ("EFFECT_RAID_DAMAGE_UNIT_ORG_RATIO", "OrgRatio $VAL|2$"),
                ("EFFECT_RAID_DAMAGE_UNIT_STRENGTH_ORG", "Both $STR_VAL|2$/$ORG_VAL|2$"),
                ("EFFECT_RAID_DAMAGE_UNIT_STRENGTH_ORG_RATIO", "BothRatio $STR_VAL|2$/$ORG_VAL|2$"),
                ("syn_unit", "Base unit"),
                ("AAA_syn_unit", "Perspective unit"),
                ("UNLOCK_SUBUNIT_EFFECT", "UnlockType $NAME$ [This.GetName]"),
                ("syn_equipment", "Equipment [This.GetName]"),
                ("STAT_COMMON_BUILD_COST_IC", "Cost"),
                ("STAT_ARMY_SOFT_ATTACK", "Attack"),
                ("STAT_AIR_RANGE", "Range"),
                ("STAT_AIR_RANGE_MOD_DIFF", "Custom $VALUE|2$"),
                ("STAT_MODIFIER_DIFF_NEG", "Bad $VALUE|2$"),
                ("STAT_MODIFIER_DIFF", "Good $VALUE|2$"),
                ("ZZZ", "Many"),
            ]
            .into_iter()
            .map(|(key, value)| (key.into(), value.into())),
        )
        .collect::<Localisation>()
        .into();
    Arc::make_mut(&mut fixture.catalogue.localisation).language = "english".into();
    *fixture.frame.tables[TableId::ScriptVariables.index()] = TableView::Complete(
        TableRows::ScriptVariables(vec![ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 100,
            entries: vec![ScriptVariableEntry {
                name: b"syn_amount".to_vec(),
                value_raw: 249_999,
            }],
        }]),
    );
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
    let dlcs = BTreeSet::new();
    let triggers = Default::default();
    let aliases = Arc::default();
    let mut provinces: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for province in fixture.frame.complete::<tables::Provinces>().unwrap() {
        provinces.entry(province.state).or_default().push(province.id);
    }
    let blocked = impassable.iter().copied().collect();
    let mut facts = ContentFacts::content_only(&dlcs, &triggers, &aliases);
    facts.state_provinces = &provinces;
    facts.impassable = &blocked;
    let text = ScriptedText::new(
        &fixture.frame,
        &fixture.catalogue.localisation,
        fixture.catalogue.scripted_content().unwrap(),
        1,
    );
    text.set_synthetic_world(World::read(&fixture.frame, facts));
    let context = Context::new(&text, &fixture.catalogue.localisation, 1, "synthetic", content);
    describe(
        &parser::parse(body.as_bytes()).unwrap().items,
        &context,
        &mut EventScopes::new(vec![EventScope::of(object)]),
    )
}

#[test]
fn native_units_output_branches() {
    let fixture = fixture();
    let equipment = equipment();
    let mut sub_units = installed_content::unit_leader::SubUnits::default();
    sub_units.read(b"sub_units={syn_unit={}}", "synthetic");
    let mut names = installed_content::key_names::KeyNames::default();
    names.sub_units = Some(sub_units);
    let content = Content {
        key_names: Some(&names),
        equipment: Some(&equipment),
        ..Default::default()
    };
    for (body, object, expected) in [
        ("air_experience=1.25", C(1), "Air 1.25\n"),
        ("navy_experience=syn_amount", C(1), "Navy 2.49\n"),
        ("raid_add_unit_experience=-0.5", C(1), "RaidXP -0.50\n"),
        ("add_manpower=syn_amount", C(1), "Men Alpha/2\n"),
        ("add_manpower=-1.5", S(1), "StateMen One/-2\n"),
        ("delete_units={division_template=Raw}", C(1), "DeleteUnits Raw\n"),
        ("delete_units={division_template=Raw disband=yes}", C(1), "DisbandUnits Raw\n"),
        (
            "delete_units={division_template=Raw air_experience=1}",
            C(1),
            "DeleteUnits Raw\nAir 1.00\n",
        ),
        ("delete_unit_template_and_units={division_template=Raw}", C(1), "Delete Raw\n"),
        (
            "delete_unit_template_and_units={division_template=Raw disband=yes}",
            C(1),
            "Disband Raw\n",
        ),
        (
            "delete_unit_template_and_units={division_template=Raw air_experience=1}",
            C(1),
            "Delete Raw\nAir 1.00\n",
        ),
        ("set_division_template_lock={division_template=Raw is_locked=yes}", C(1), "Lock Raw\n"),
        ("set_division_template_lock={division_template=Raw}", C(1), "Unlock Raw\n"),
        ("clear_division_template_cap={division_template=Raw}", C(1), "ClearCap Raw\n"),
        (
            "set_division_template_cap={division_template=Raw division_cap=syn_amount}",
            C(1),
            "Cap Raw/2\n",
        ),
        (
            "set_division_template_cap={division_template=Raw division_cap=-2.9}",
            C(1),
            "Cap Raw/-2\n",
        ),
        (
            "set_division_force_allow_recruiting={division_template=Raw force_allow_recruiting=yes}",
            C(1),
            "Recruit Raw/yes\n",
        ),
        ("set_division_force_allow_recruiting={division_template=Raw}", C(1), "Recruit Raw/no\n"),
        ("country_lock_all_division_template=yes", C(1), "LockAll\n"),
        ("country_lock_all_division_template={is_locked=no}", C(1), "UnlockAll\n"),
        ("transfer_navy={target=BBB}", C(1), "NavyTo beta\n"),
        ("transfer_units_fraction={target=BBB fraction=0.25}", C(1), "UnitsTo beta"),
        ("send_equipment_fraction={target=BBB fraction=1.5}", C(1), "Fraction 1.00/Beta\n"),
        ("send_equipment_fraction={target=BBB fraction=-0.5}", C(1), "Fraction 0.00/Beta\n"),
        ("set_equipment_fraction=2.5", C(1), "Adjust 1.50\n"),
        ("set_equipment_fraction=0", C(1), "Adjust -1.00\n"),
        (
            "add_design_template_bonus={name=syn_bonus cost_factor=-0.2 uses=3}",
            C(1),
            "Design -0.20/3/Bonus [This.GetName] [This.GetName]\n",
        ),
        (
            "add_design_template_bonus={name=syn_bonus cost_factor=syn_amount uses=oops}",
            C(1),
            "Design 0.00/0/Bonus [This.GetName] [This.GetName]\n",
        ),
        (
            "add_equipment_production={equipment={version_name=Raw type=missing} equipment={type=another}}",
            C(1),
            "Produce Raw Alpha",
        ),
        ("add_units_to_division_template={template_name=Raw}", C(1), "Add Alpha/Raw\n"),
        ("set_naval_oob=Raw", C(1), "OOB Raw Alpha"),
        ("create_ship={equipment_variant=Raw}", C(1), "Ship Raw/alpha Alpha"),
        ("create_ship={equipment_variant=Raw creator=BBB}", C(1), "Ship Raw/beta Alpha"),
        ("create_ship={equipment_variant=Raw creator=ZZZ}", C(1), ""),
        ("create_ship={equipment_variant=Raw}", S(1), ""),
        ("create_unit={owner=AAA}", C(1), "Unit Alphan"),
        ("create_unit={owner=BBB count=2.5}", C(1), "Units Betan/3"),
        ("create_unit={owner=AAA count=0}", S(1), ""),
        ("create_unit={owner=AAA}", S(1), "UnitAt Alphan/One"),
        ("create_unit={owner=AAA count=2}", S(1), "UnitsAt Alphan/2/One"),
        ("create_unit={owner=AAA prioritize_location=2}", S(1), "UnitAt Alphan/Two"),
        ("create_unit={owner=AAA prioritize_location=2}", C(1), "Unit Alphan"),
        ("create_unit={owner=AAA country_score={base=0}}", S(1), "Unit Alphan"),
        (
            "create_unit={owner=AAA country_score={base=1 modifier={factor=0 NOT={PREV={impassable=no}}}}}",
            S(1),
            "UnitAt Alphan/One",
        ),
        ("raid_damage_units={plane_loss=2 damage=3 str_damage=4 org_damage=5}", C(1), "Plane 2.00"),
        ("raid_damage_units={plane_loss=2 ratio=yes}", C(1), "PlaneRatio 2.00"),
        ("raid_damage_units={damage=3 ratio=yes org_damage=5}", C(1), "Strength 3.00"),
        ("raid_damage_units={str_damage=4 org_damage=5}", C(1), "Both 4.00/5.00"),
        ("raid_damage_units={str_damage=4 org_damage=5 ratio=yes}", C(1), "BothRatio 4.00/5.00"),
        ("raid_damage_units={str_damage=syn_amount}", C(1), "Strength 2.49"),
        ("raid_damage_units={str_damage=4 ratio=yes}", C(1), "StrengthRatio 4.00"),
        ("raid_damage_units={org_damage=5}", C(1), "Org 5.00"),
        ("raid_damage_units={org_damage=5 ratio=yes}", C(1), "OrgRatio 5.00"),
        ("raid_damage_units={ratio=yes}", C(1), ""),
        ("unlock_subunit=syn_unit", C(2), "UnlockType Perspective unit [This.GetName]"),
        ("add_unit_bonus={name=ignored}", C(1), ""),
        ("add_equipment_to_stockpile={}", C(1), ""),
        ("create_equipment_variant={}", C(1), ""),
        ("create_equipment_variant={type=missing}", S(0), ""),
        ("create_equipment_variant={type=missing}", S(5), ""),
        ("create_equipment_variant={type=missing}", C(0), ""),
        ("destroy_ships={count=all}", C(1), ""),
        ("send_equipment={amount=1}", C(1), ""),
        ("damage_units={}", C(1), ""),
        (
            "add_equipment_bonus={name=ignored bonus={syn_equipment={build_cost_ic=-1 soft_attack=2 soft_attack=-2 air_range=3}}}",
            C(1),
            "\u{11}YEquipment [This.GetName]\u{11}!: \n    CostBad -1.00\n    AttackGood 0.00\n    RangeCustom 3.00",
        ),
        (
            "BBB={transfer_units_fraction={target=AAA} set_equipment_fraction=2}",
            C(1),
            "@BBBBeta:\n   UnitsTo alpha\n   Adjust 1.00\n",
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
    let out = fixture.render("create_unit={owner=AAA count=2}", &mut graph, content);
    assert_eq!(out.text, "Units Alphan/2");
    assert_eq!(graph, before);
    for state in [999, u32::MAX] {
        let out = render(
            &fixture,
            "create_equipment_variant={type=syn_arch}",
            S(state),
            content,
            &[],
        );
        assert!(out.text.is_empty());
        assert_eq!(out.refusals.values().sum::<usize>(), 1);
        assert!(
            out.refusals.keys().all(|why| why.contains("invalid nonzero state")),
            "{:?}",
            out.refusals,
        );
    }
}

#[test]
fn native_units_admission_and_named_refusals() {
    let mut fixture = fixture();
    if let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *fixture.frame.tables[TableId::Provinces.index()]
    {
        rows[0].controller = 200;
    }
    *fixture.frame.tables[TableId::ProvinceUnitOrder.index()] = TableView::Complete(
        TableRows::ProvinceUnitOrder(vec![ProvinceUnitOrderRow {
            province: 1,
            mixed: vec![11],
            naval: vec![],
        }]),
    );
    *fixture.frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(
        vec![DivisionRow {
            id: 11,
            province: 1,
            logical_owner: 200,
            roster_owner: 200,
            ..Default::default()
        }],
    ));
    for (access, faction, subject, war, enemy, blocked, score, expected) in [
        (false, false, false, false, false, false, false, "Unit Alphan"),
        (true, false, false, false, false, false, false, "UnitAt Alphan/One"),
        (false, true, false, false, false, false, false, "UnitAt Alphan/One"),
        (false, false, true, false, false, false, false, "UnitAt Alphan/One"),
        (true, true, true, true, false, false, false, "Unit Alphan"),
        (false, false, false, true, true, false, false, "UnitAt Alphan/One"),
        (false, false, false, true, true, false, true, "UnitAt Alphan/One"),
        (true, false, false, false, false, true, false, "Unit Alphan"),
    ] {
        *fixture.frame.tables[TableId::RelationLinks.index()] =
            TableView::Complete(TableRows::RelationLinks(if access {
                vec![RelationLinkRow {
                    country: 200,
                    target_index: 1,
                    links: RelationLinkRow::MILITARY_ACCESS,
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
        *fixture.frame.tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(
            if subject {
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
            },
        ));
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
            "create_unit={{owner=AAA allow_spawning_on_enemy_provs={} {}}}",
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
            "add_equipment_production={equipment={type=missing}}",
            "CAddEquipmentProduction",
            "equipment content unavailable",
        ),
        (
            "add_equipment_to_stockpile={type=missing amount=1}",
            "CAddEquipmentToStockpileEffect",
            "equipment content unavailable",
        ),
        (
            "add_equipment_subsidy={equipment_type=missing}",
            "CAddEquipmentSubsidyEffect",
            "equipment content unavailable",
        ),
        (
            "create_equipment_variant={type=missing}",
            "CCreateEquipmentVariantEffect",
            "equipment content unavailable",
        ),
        (
            "destroy_ships={type=missing count=all}",
            "CDestroyShipsEffect",
            "equipment content unavailable",
        ),
        (
            "send_equipment={type=missing target=BBB}",
            "CSendEquipmentEffect",
            "equipment content unavailable",
        ),
        ("change_division_template=Raw", "CArmyChangeTemplateEffect", "UNIT_REFERENCE"),
        ("destroy_unit=yes", "CDestroyUnitEffect", "UNIT_REFERENCE"),
        ("set_unit_organization=1", "CSetUnitOrganization", "UNIT_REFERENCE"),
        ("add_unit_bonus={syn_unit={}}", "CAddUnitBonusEffect", "template_counter"),
        ("unlock_subunit=undefined", "CUnlockSubUnitEffect", "content unavailable"),
        ("transfer_navy={target=ZZZ}", "CTransferNavyEffect", "country"),
        ("create_unit={}", "CCreateUnitEffect", "reserved CCountry[0]"),
        ("create_unit=THIS", "CCreateUnitEffect", "rejects the scalar"),
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
            out.refusals.keys().all(|key| key.contains(class) && key.contains(reason)),
            "{body}: {:?}",
            out.refusals,
        );
    }
    // A successful earlier province must not hide a later unknown native score.
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
        "create_unit={owner=AAA country_score={base=1 modifier={factor=1 tag=BBB has_cosmetic_tag=syn_costume}}}",
        S(1),
        Content::default(),
        &[],
    );
    assert!(out.text.is_empty());
    assert!(
        out.refusals.keys().any(|why| why.contains("CountryAppearance")),
        "{:?}",
        out.refusals,
    );
}

#[test]
fn native_units_have_no_country_count_cap() {
    let mut fixture = equipment_fixture();
    let countries = crate::effect_description::tests::synthetic_countries(5000);
    *fixture.frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
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
                    ..Default::default()
                })
                .collect(),
        ));
    *fixture.frame.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(
        countries
            .iter()
            .map(|row| PoliticsRow {
                country: row.id,
                ..Default::default()
            })
            .collect(),
    ));
    *fixture.frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries));
    let out = render(&fixture, "add_manpower=2.5", C(5000), Content::default(), &[]);
    assert_eq!(out.text, "Men Many/3\n", "{:?}", out.refusals);
    assert!(out.refusals.is_empty());
    let equipment = equipment();
    let content = Content {
        equipment: Some(&equipment),
        ..Default::default()
    };
    for (body, expected) in [
        (
            "send_equipment={type=syn_arch target=AAA amount=1}",
            "SendOld 1/Alpha gear Many/Alpha Many\n",
        ),
        (
            "create_equipment_variant={type=syn_arch allow_without_tech=yes}",
            "LegacyTechUnnamed Base short",
        ),
    ] {
        let out = render(&fixture, body, C(5000), content, &[]);
        assert_eq!(out.text, expected, "{body}: {:?}", out.refusals);
        assert!(out.refusals.is_empty());
    }
}

fn equipment() -> installed_content::equipment::Database {
    installed_content::equipment::fixture::load(&[
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
                ("EFFECT_ADD_EQUIPMENT_TO_STOCKPILE", "Store $AMOUNT|0$/$EQUIPMENT$"),
                ("EFFECT_REMOVE_EQUIPMENT_FROM_STOCKPILE", "Take $AMOUNT|0$/$EQUIPMENT$"),
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
                ("EFFECT_CREATE_EQUIPMENT_VARIANT_LEGACY", "LegacyTech $TYPE$/$NAME$"),
                ("EFFECT_CREATE_EQUIPMENT_VARIANT_LEGACY_NO_NAME", "LegacyTechUnnamed $TYPE$"),
                ("EFFECT_CREATE_EQUIPMENT_VARIANT_LEGACY_NO_TECH", "LegacyNoTech $TYPE$/$NAME$"),
                (
                    "EFFECT_CREATE_EQUIPMENT_VARIANT_LEGACY_NO_TECH_NO_NAME",
                    "LegacyNoTechUnnamed $TYPE$",
                ),
                ("EFFECT_CREATE_EQUIPMENT_VARIANT", "ModularTech $TYPE$/$NAME$/$ROLE$"),
                ("EFFECT_CREATE_EQUIPMENT_VARIANT_NO_NAME", "ModularTechUnnamed $TYPE$/$ROLE$"),
                ("EFFECT_CREATE_EQUIPMENT_VARIANT_NO_TECH", "ModularNoTech $TYPE$/$NAME$/$ROLE$"),
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
    *fixture.frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(
        TableRows::LexicalTokens(
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
        ),
    );
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
    *fixture.frame.tables[TableId::SimulationFeatures.index()] = TableView::Complete(
        TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            dlc_mask: 1 << 4,
            ..Default::default()
        }]),
    );
    *fixture.frame.tables[TableId::VariantDefinitions.index()] = TableView::Complete(
        TableRows::VariantDefinitions(vec![
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
        ]),
    );
    *fixture.frame.tables[TableId::AvailableEquipmentVariants.index()] = TableView::Complete(
        TableRows::AvailableEquipmentVariants(
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
            .map(|(country, roster_kind, ordinal, variant)| AvailableEquipmentVariantRow {
                country,
                roster_kind,
                ordinal,
                variant,
            })
            .collect(),
        ),
    );
    fixture
}

#[test]
fn native_units_equipment_branches() {
    let fixture = equipment_fixture();
    let equipment = equipment();
    let names = installed_content::key_names::KeyNames::default();
    let content = Content {
        equipment: Some(&equipment),
        key_names: Some(&names),
        ..Default::default()
    };
    for (body, object, expected) in [
        (
            "add_equipment_production={equipment={type=syn_arch}}",
            C(1),
            "Produce Zero gear Alpha Alpha",
        ),
        (
            "add_equipment_production={equipment={type=syn_arch creator=AAA}}",
            C(2),
            "Produce Alpha gear Beta Beta",
        ),
        (
            "add_equipment_production={equipment={type=syn_concrete creator=BBB} equipment={version_name=\"\"}}",
            C(1),
            "Produce Beta model Alpha",
        ),
        (
            "add_equipment_production={equipment={type=syn_arch creator=ROOT}}",
            C(1),
            "Produce Zero gear Alpha Alpha",
        ),
        (
            "add_equipment_production={equipment={type=syn_arch version_name=\"Raw [This.GetName]\"}}",
            C(2),
            "Produce Raw Beta Beta",
        ),
        (
            "add_equipment_to_stockpile={type=syn_alias amount=2.4}",
            C(2),
            "Store 2/Alpha gear Beta\n",
        ),
        ("add_equipment_to_stockpile={type=syn_arch amount=0}", C(1), "Take 0/Alpha gear Alpha\n"),
        ("add_equipment_to_stockpile={type=syn_arch amount=1}", S(1), "Store 1/Alpha gear One\n"),
        (
            "add_equipment_to_stockpile={type=syn_arch amount=-1.5 producer=BBB}",
            C(1),
            "TakeForeign -2/Alpha gear Alpha/Betan\n",
        ),
        (
            "add_equipment_to_stockpile={type=syn_arch amount=1 producer=BBB}",
            C(1),
            "StoreForeign 1/Alpha gear Alpha/Betan\n",
        ),
        (
            "add_equipment_to_stockpile={type=syn_concrete amount=1 variant_name=Ignored}",
            C(1),
            "Store 1/First Alpha Mark\n",
        ),
        (
            "add_equipment_to_stockpile={type=syn_concrete amount=1 producer=BBB}",
            C(1),
            "StoreForeign 1/Beta model Foreign/Betan\n",
        ),
        (
            "add_equipment_to_stockpile={type=syn_concrete amount=1 producer=BBB}",
            S(1),
            "StoreForeign 1/Beta model Foreign/Betan\n",
        ),
        ("add_equipment_to_stockpile={type=syn_hull amount=1}", C(1), ""),
        (
            "add_equipment_to_stockpile={type=syn_hull amount=1 producer=BBB}",
            C(1),
            "StoreForeign 1/Ship/Betan\n",
        ),
        ("add_equipment_to_stockpile={type=syn_basic_hull amount=1}", C(1), ""),
        (
            "add_equipment_to_stockpile={type=var:syn_type amount=1}",
            C(1),
            "Store 1/Alpha gear Alpha\n",
        ),
        (
            "add_equipment_to_stockpile={type=var:syn_fraction amount=1}",
            C(1),
            "Store 1/Alpha gear Alpha\n",
        ),
        (
            "add_equipment_to_stockpile={type=var:syn_negative amount=1}",
            C(1),
            "Store 1/Alpha gear Alpha\n",
        ),
        (
            "add_equipment_to_stockpile={type=var:syn_type type=syn_hull amount=1}",
            C(1),
            "Store 1/Alpha gear Alpha\n",
        ),
        (
            "add_equipment_to_stockpile={type=var:syn_negative type=syn_type amount=1}",
            C(1),
            "Store 1/Alpha gear Alpha\n",
        ),
        ("add_equipment_to_stockpile={type=syn_arch type=missing amount=1}", C(1), ""),
        ("add_equipment_to_stockpile={type=syn_type amount=1}", C(1), ""),
        ("add_equipment_to_stockpile={type=VAR:syn_type amount=1}", C(1), ""),
        (
            "send_equipment={type=syn_arch target=BBB amount=syn_amount old_prioritised=yes}",
            C(1),
            "SendOld 2/Alpha gear Alpha/Beta Alpha\n",
        ),
        (
            "send_equipment={equipment=syn_arch target=BBB amount=-1.5 new_prioritised=yes}",
            C(1),
            "SendNew -2/Alpha gear Alpha/Beta Alpha\n",
        ),
        (
            "send_equipment={type=var:syn_type equipment=syn_hull target=BBB amount=1}",
            C(1),
            "SendOld 1/Alpha gear Alpha/Beta Alpha\n",
        ),
        ("send_equipment={type=missing target=ZZZ amount=not_recorded}", C(1), ""),
        ("destroy_ships={type=syn_arch count=1.5}", C(1), "DestroyOne 1/Alpha gear Alpha\n"),
        ("destroy_ships={type=syn_arch count=0x10}", C(1), "DestroyMany 0/Alpha gear Alpha\n"),
        (
            "destroy_ships={type=syn_arch count=all}",
            C(1),
            "DestroyMany all Alpha/Alpha gear Alpha\n",
        ),
        ("destroy_ships={type=syn_arch count=\"all\"}", C(1), "DestroyMany 0/Alpha gear Alpha\n"),
        (
            "destroy_ships={type=syn_arch count=-2}",
            C(1),
            "DestroyMany all Alpha/Alpha gear Alpha\n",
        ),
        (
            "destroy_ships={type=syn_arch count=syn_amount}",
            C(1),
            "DestroyMany 0/Alpha gear Alpha\n",
        ),
        (
            "create_equipment_variant={type=syn_concrete name=Raw allow_without_tech=yes}",
            C(1),
            "LegacyTech Model short/Raw",
        ),
        (
            "create_equipment_variant={type=syn_concrete allow_without_tech=yes}",
            C(1),
            "LegacyTechUnnamed Model short",
        ),
        (
            "create_equipment_variant={type=syn_concrete name=Raw}",
            C(1),
            "LegacyNoTech Model short/Raw",
        ),
        ("create_equipment_variant={type=syn_concrete}", C(1), "LegacyNoTechUnnamed Model short"),
        (
            "create_equipment_variant={type=syn_basic_hull name=Raw allow_without_tech=yes}",
            C(1),
            "ModularTech Alpha ship short/Raw/Specific capital",
        ),
        (
            "create_equipment_variant={type=syn_basic_hull allow_without_tech=yes}",
            C(1),
            "ModularTechUnnamed Alpha ship short/Specific capital",
        ),
        (
            "create_equipment_variant={type=syn_basic_hull name=Raw}",
            C(1),
            "ModularNoTech Alpha ship short/Raw/Specific capital",
        ),
        (
            "create_equipment_variant={type=syn_basic_hull}",
            C(1),
            "ModularNoTechUnnamed Alpha ship short/Specific capital",
        ),
        (
            "create_equipment_variant={type=syn_basic_hull allow_without_tech=yes}",
            S(1),
            "ModularTechUnnamed Alpha ship short/Specific capital",
        ),
        (
            "create_equipment_variant={type=syn_basic_hull allow_without_tech=yes}",
            C(2),
            "ModularTechUnnamed Ship short/Specific capital",
        ),
        (
            "create_equipment_variant={type=syn_arch allow_without_tech=yes}",
            C(1),
            "LegacyTechUnnamed Alpha short",
        ),
        (
            "create_equipment_variant={type=syn_hull modules={main=empty} allow_without_tech=yes}",
            C(1),
            "ModularTechUnnamed Alpha ship short/Carrier",
        ),
        (
            "create_equipment_variant={type=syn_hull modules={main=syn_sub} allow_without_tech=yes}",
            C(1),
            "ModularTechUnnamed Alpha ship short/Carrier",
        ),
        (
            "create_equipment_variant={type=syn_basic_hull modules={main=syn_carrier MAIN=syn_sub} allow_without_tech=yes}",
            C(1),
            "ModularTechUnnamed Alpha ship short/Specific capital",
        ),
        (
            "create_equipment_variant={type=syn_basic_hull modules={main=syn_carrier} modules={MAIN=missing} allow_without_tech=yes}",
            C(1),
            "ModularTechUnnamed Alpha ship short/Carrier",
        ),
        ("create_equipment_variant={type=syn_arch}", S(0), ""),
        ("create_equipment_variant={type=syn_arch}", S(5), ""),
        ("create_equipment_variant={type=syn_arch}", C(0), ""),
        ("create_equipment_variant={type=missing}", S(999), ""),
        (
            "add_equipment_subsidy={equipment_type=syn_arch cic=syn_amount}",
            C(1),
            "Subsidy Alpha/2.49/Alpha gear Alpha/\n",
        ),
        (
            "add_equipment_subsidy={equipment_type=syn_arch seller_tags={BBB AAA} seller_tags={BBB ZZZ} seller_trigger=syn_trigger}",
            C(1),
            "Subsidy Alpha/0.00/Alpha gear Alpha/Beta|Alpha|Beta\n",
        ),
        (
            "add_equipment_subsidy={equipment_type=syn_arch seller_tags={ZZZ} seller_trigger=\"\"}",
            C(1),
            "Subsidy Alpha/0.00/Alpha gear Alpha/\n",
        ),
        ("add_equipment_subsidy={equipment_type=missing}", C(1), "Subsidy Alpha/0.00/Null type/\n"),
        (
            "BBB={send_equipment={type=syn_arch target=AAA amount=1} destroy_ships={type=syn_arch count=1}}",
            C(1),
            "@BBBBeta:\n   SendOld 1/Alpha gear Beta/Alpha Beta\n   DestroyOne 1/Alpha gear Beta\n",
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
        "create_equipment_variant={type=syn_basic_hull allow_without_tech=yes}",
        C(1),
        content,
        &[],
    );
    assert_eq!(out.text, "LegacyTechUnnamed Alpha hull");
    assert!(out.refusals.is_empty());
}

#[test]
fn native_units_equipment_named_refusals() {
    let mut fixture = equipment_fixture();
    let equipment = equipment();
    let content = Content {
        equipment: Some(&equipment),
        ..Default::default()
    };
    for (body, reason) in [
        (
            "add_equipment_to_stockpile={type={var=syn_type} amount=1}",
            "equipment-type block operand",
        ),
        (
            "add_equipment_to_stockpile={type=var:syn_missing amount=1}",
            "LexicalTokens lacks equipment variable token 999",
        ),
        (
            "add_equipment_subsidy={equipment_type=syn_arch seller_tags={ZZZ} seller_trigger=syn_trigger}",
            "trigger-template vf12",
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
            "add_equipment_to_stockpile={type=syn_concrete amount=1}",
            object,
            content,
            &[],
        );
        assert!(out.text.is_empty());
        assert_eq!(out.refusals.values().sum::<usize>(), 1);
        assert!(
            out.refusals.keys().all(|why| why.contains("direct country0 production roster")),
            "{:?}",
            out.refusals,
        );
    }
    let out = render(
        &fixture,
        "add_equipment_to_stockpile={type=syn_concrete amount=1}",
        C(9999),
        content,
        &[],
    );
    assert!(out.text.is_empty());
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(out.refusals.keys().all(|why| why.contains("equipment roster country slot 9999")));
    if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
        &mut *fixture.frame.tables[TableId::VariantDefinitions.index()]
    {
        rows.iter_mut().find(|row| row.id == 60).unwrap().creator_link = 100;
    }
    let out = render(
        &fixture,
        "create_equipment_variant={type=syn_concrete}",
        C(1),
        content,
        &[],
    );
    assert!(out.text.is_empty());
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(
        out.refusals.keys().all(|why| why.contains("version u8 +0x41C")),
        "{:?}",
        out.refusals,
    );
    let out = render(
        &fixture,
        "create_equipment_variant={type=syn_concrete allow_without_tech=yes}",
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
    // A later unresolved roster row cannot override the first exact match.
    let out = render(
        &fixture,
        "add_equipment_to_stockpile={type=syn_concrete amount=1}",
        C(1),
        content,
        &[],
    );
    assert_eq!(out.text, "Store 1/First Alpha Mark\n");
    assert!(out.refusals.is_empty());
}

#[test]
fn native_units_equipment_role_branches() {
    let cases = [
        ("capital_ship", "ship_designer_capital_ship"),
        ("submarine", "ship_designer_submarine"),
        ("screen_ship", "ship_designer_screen"),
        ("carrier", "ship_designer_carrier"),
        ("capital_ship carrier", "ship_designer_carrier"),
        ("armor", "tank_designer_"),
        ("fighter", "plane_designer_fighter"),
        ("interceptor", "plane_designer_interceptor"),
        ("tactical_bomber", "plane_designer_tactical_bomber"),
        ("strategic_bomber", "plane_designer_strategic_bomber"),
        ("cas", "plane_designer_cas"),
        ("naval_bomber", "plane_designer_naval_bomber"),
        ("suicide", "plane_designer_suicide"),
        ("scout_plane", "plane_designer_scout_plane"),
        ("maritime_patrol_plane", "plane_designer_maritime_patrol_plane"),
        ("armor flame", "tank_designer_flame"),
        ("armor amphibious", "tank_designer_amphibious"),
        ("armor anti_air", "tank_designer_anti_air"),
        ("armor artillery", "tank_designer_artillery"),
        ("armor anti_tank", "tank_designer_anti_tank"),
        ("armor rocket", "tank_designer_rocket"),
        ("heavy_fighter", "plane_designer_heavy_fighter"),
        ("fighter armor", "unknown"),
    ];
    let mut source = String::from("equipments={");
    let mut localisation = Vec::new();
    let mut expected = Vec::new();
    for (index, (category, key)) in cases.into_iter().enumerate() {
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
    let equipment = installed_content::equipment::fixture::load(&[(
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
        let body = format!("create_equipment_variant={{type={name} allow_without_tech=yes}}");
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
