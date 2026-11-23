use super::*;
use crate::effect_description::tests::Fixture;
use crate::scripted_text::{EventObject, EventScope, Link};
use EventObject::{Country as C, State as S};
use replay_data::rows::*;
use replay_frame::frame::TableView;

#[test]
fn state_category_description_uses_cached_validity_and_operand_spelling() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = Fixture::new();
    let (catalogue, folder) = crate::fixtures::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        ),
        (
            "common/state_category/test.txt",
            "state_categories={Syn_Category={local_building_slots=7} missing_loc={} Syn_Category={local_building_slots=9}}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " Syn_Category:0 \"Declared [This.GetName]\"\n",
                " SYN_CATEGORY:0 \"Operand\"\n",
                " EFFECT_SET_STATE_CATEGORY:0 \"Category $CATEGORY$ [This.GetName]\"\n",
            ),
        ),
    ]);
    fixture.catalogue = catalogue;
    let scripted = fixture.catalogue.scripted_content().unwrap();
    scripted.set_synthetic_environment(Default::default());
    let cached = scripted.effects().unwrap();
    std::fs::write(
        folder.0.join("common/state_category/test.txt"),
        "state_categories={replacement={}}",
    )
    .unwrap();
    assert!(std::ptr::eq(cached, scripted.effects().unwrap()));
    for (body, object, expected, refusal) in [
        (
            "set_state_category=Syn_Category",
            S(1),
            "Category Declared [This.GetName] [This.GetName]\n",
            None,
        ),
        (
            "set_state_category=SYN_CATEGORY",
            S(1),
            "Category Operand [This.GetName]\n",
            None,
        ),
        (
            "set_state_category=syn_category",
            C(1),
            "Category syn_category [This.GetName]\n",
            None,
        ),
        (
            "set_state_category=missing_loc",
            S(999),
            "Category missing_loc [This.GetName]\n",
            None,
        ),
        (
            "set_state_category=undefined",
            S(1),
            "",
            Some("invalid state category undefined"),
        ),
        (
            "set_state_category=replacement",
            S(1),
            "",
            Some("invalid state category replacement"),
        ),
        (
            "set_state_category={}",
            S(1),
            "",
            Some("requires a scalar operand"),
        ),
    ] {
        let mut scopes = EventScopes::new(vec![EventScope::of(object)]);
        let out = fixture.render(body, &mut scopes, cached.view(None));
        assert_eq!(out.text, expected, "{body}: {:?}", out.refusals);
        match refusal {
            Some(why) => assert!(
                out.refusals.keys().any(|key| key.contains(why)),
                "{body}: {:?}",
                out.refusals
            ),
            None => assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals),
        }
    }
}

#[test]
fn state_content_descriptions_use_cached_names() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = Fixture::new();
    let (catalogue, _folder) = crate::fixtures::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}},NSupply={MAX_RAILWAY_LEVEL=4}}",
        ),
        (
            "common/buildings/test.txt",
            "buildings={Syn_Mill={} Syn_Tower={level_cap={province_max=5}} Syn_Spire={level_cap={province_max=4}}}",
        ),
        (
            "common/resources/test.txt",
            "resources={syn_ore={icon_frame=7}}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " AAA:0 \"Alpha\"\n AAA_ADJ:0 \"Alphan\"\n",
                " Syn_Mill:0 \"Mill [This.GetName]\"\n",
                " Syn_Mill_plural:0 \"Mills\"\n Syn_Tower:0 \"Tower\"\n",
                " Syn_Spire:0 \"Spire\"\n",
                " EFFECT_ADD_BUILDING_CONSTRUCTION_INSTANT:0 \"Instant $NUMBER|0$/$BUILDING$\"\n",
                " EFFECT_ADD_BUILDING_CONSTRUCTION:0 \"Build $NUMBER|0$/$BUILDING$\"\n",
                " EFFECT_ADD_BUILDING_CONSTRUCTION_PROVINCE:0 \"Province $NUMBER|0$/$BUILDING$\"\n",
                " EFFECT_ADD_BUILDING_CONSTRUCTION_INSTANT_PROVINCE:0 \"InstantProvince $NUMBER|0$/$BUILDING$\"\n",
                " EFFECT_SET_BUILDING_CONSTRUCTION:0 \"Set $NUMBER$/$BUILDING$\"\n",
                " EFFECT_ADD_OFFSITE_BUILDING:0 \"Offsite $NUMBER$/$BUILDING$\"\n",
                " CONSTRUCT_BUILDING_IN_STATE:0 \"Random $BUILDING$/$STATE$\"\n",
                " EFFECT_DAMAGE_BUILDING:0 \"Damage $BUILDING$/$AMOUNT|3$ [This.GetName]\"\n",
                " EFFECT_TEMPORARY_REPAIR_SPEED_MODIFIER:0 \"Repair $BUILDING$/$FACTOR|1$\\n\\nMore\"\n",
                " EFFECT_REMOVE_BUILDING:0 \"Remove $NUMBER$/$BUILDING$/$LOCATION$\"\n",
                " EFFECT_MODIFY_BUILDING_RESOURCES:0 \"Modify $BUILDING$/$AMOUNT|0$/$FRAME|0$\"\n",
                " EFFECT_ADD_RESOURCE:0 \"Add $RESOURCE$/$STATE$/$DAYS|0$\"\n",
                " EFFECT_REMOVE_RESOURCE:0 \"Minus $RESOURCE$/$STATE$/$DAYS|0$\"\n",
                " EFFECT_ADD_TEMPORARY_RESOURCE:0 \"TempAdd $RESOURCE$/$STATE$/$DAYS|0$\"\n",
                " EFFECT_REMOVE_TEMPORARY_RESOURCE:0 \"TempMinus $RESOURCE$/$STATE$/$DAYS|0$\"\n",
                " EFFECT_ADD_RESOURCE_NO_STATE:0 \"AddHidden $RESOURCE$\"\n",
                " EFFECT_REMOVE_RESOURCE_NO_STATE:0 \"MinusHidden $RESOURCE$\"\n",
                " EFFECT_ADD_TEMPORARY_RESOURCE_NO_STATE:0 \"TempAddHidden $RESOURCE$/$DAYS|0$\"\n",
                " EFFECT_REMOVE_TEMPORARY_RESOURCE_NO_STATE:0 \"TempMinusHidden $RESOURCE$/$DAYS|0$\"\n",
                " EFFECT_CREATE_RAILWAY_GUN:0 \"Gun $OWNER$ [This.GetName]\"\n",
                " EFFECT_CREATE_RAILWAY_GUN_LOCATION:0 \"GunAt $OWNER$/$STATE$ [This.GetName]\"\n",
                " EFFECT_BUILD_RAILWAY_IN_STATE:0 \"Rail $LEVEL|0$/$STATE$ [This.GetName]\"\n",
                " EFFECT_BUILD_RAILWAY:0 \"Rail $LEVEL|0$/$FROM$/$TO$ [This.GetName]\"\n",
            ),
        ),
    ]);
    fixture.catalogue = catalogue;
    let scripted = fixture.catalogue.scripted_content().unwrap();
    scripted.set_synthetic_environment(Default::default());
    let cache = scripted.effects().unwrap();
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 600,
                literal: b"syn_mill".to_vec(),
            },
            LexicalTokenRow {
                token: 700,
                literal: b"syn_ore".to_vec(),
            },
        ]));
    *fixture.frame.tables[TableId::StateBuildings.index()] =
        TableView::Complete(TableRows::StateBuildings(vec![BuildingRow {
            location: 1,
            building_type: 600,
            level: 0,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::ResourceDefinitions.index()] =
        TableView::Complete(TableRows::ResourceDefinitions(vec![
            ResourceDefinitionRow {
                id: 9,
                token: 700,
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 100,
            seed: replay_data::rows::Recorded::Value(None),
            entries: vec![
                ScriptVariableEntry {
                    name: b"syn_amount".to_vec(),
                    value_raw: 249_999,
                },
                ScriptVariableEntry {
                    name: b"syn_days".to_vec(),
                    value_raw: -250_000,
                },
            ],
        }]));
    if let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *fixture.frame.tables[TableId::Provinces.index()]
    {
        rows[4].state = 0;
    }
    for (body, object, expected, refusal) in [
        (
            "add_building_construction={type=syn_mill level=2}",
            C(1),
            "Instant 2/Mill [This.GetName]\n",
            None,
        ),
        (
            "add_building_construction={type=syn_mill level=syn_amount}",
            C(1),
            "Instant 2/Mill [This.GetName]\n",
            None,
        ),
        (
            "add_building_construction={type=SYN_MILL level=-1.5 instant_build=no}",
            C(1),
            "Build -2/Mill [This.GetName]\n",
            None,
        ),
        (
            "add_building_construction={type=syn_mill level=1.5 province=1}",
            C(1),
            "InstantProvince 2/Mill [This.GetName]\n",
            None,
        ),
        (
            "add_building_construction={type=syn_mill level=1 province=1 instant_build=no}",
            C(1),
            "Province 1/Mill [This.GetName]\n",
            None,
        ),
        (
            "add_building_construction={type=syn_mill province={id=1}}",
            C(1),
            "Instant 0/Mill [This.GetName]\n",
            None,
        ),
        (
            "add_building_construction={type=syn_mill province=1 province={id=2}}",
            C(1),
            "InstantProvince 0/Mill [This.GetName]\n",
            None,
        ),
        (
            "set_building_level={type=syn_mill level=-2}",
            C(1),
            "Set -2/Mill [This.GetName]\n",
            None,
        ),
        (
            "set_building_level={type=syn_mill level=2 province=1}",
            C(1),
            "Set 2/Mill [This.GetName]\n",
            None,
        ),
        (
            "add_offsite_building={type=syn_mill level=2}",
            C(1),
            "Offsite 2/Mill Alpha\n",
            None,
        ),
        (
            "construct_building_in_random_province={syn_tower=3 syn_mill=4}",
            S(1),
            "Random Tower/One",
            None,
        ),
        (
            "construct_building_in_random_province={\"syn_tower\"=3}",
            S(1),
            "Random Tower/One",
            None,
        ),
        (
            "construct_building_in_random_province={syn_tower=3 syn_spire=2 syn_mill=1}",
            S(1),
            "Random Spire/One",
            None,
        ),
        (
            "damage_building={type=syn_mill damage=.125 province=unavailable}",
            C(1),
            "Damage Mills/0.000 [This.GetName]\n",
            None,
        ),
        (
            "damage_building={type=syn_mill damage=0.125 repair_speed_modifier=-0.1}",
            C(1),
            "Damage Mills/0.125 [This.GetName]\n   Repair Mills/-0.1\n\n   More\n",
            None,
        ),
        (
            "remove_building={type=syn_mill state=1 level=1.5}",
            C(1),
            "Remove 2/Mill [This.GetName]/One\n",
            None,
        ),
        (
            "remove_building={type=syn_mill state=999 level=1}",
            S(1),
            "Remove 1/Mill [This.GetName]/One\n",
            None,
        ),
        (
            "remove_building={type=syn_mill state=2 level=unavailable}",
            C(1),
            "",
            None,
        ),
        ("remove_building={type=syn_tower level=1}", S(1), "", None),
        (
            "modify_building_resources={building=syn_mill amount=3 resource=syn_ore}",
            C(1),
            "Modify Mill Alpha/3/9\n",
            None,
        ),
        (
            "modify_building_resources={building=missing amount=3 resource=syn_ore}",
            C(1),
            "",
            None,
        ),
        (
            "modify_building_resources={building=\"syn_mill\" amount=3 resource=syn_ore}",
            C(1),
            "",
            None,
        ),
        (
            "add_resource={type=syn_ore amount=1.5 state=1}",
            C(1),
            "Add 2\u{13}resources_strip|7 /One/0\n",
            None,
        ),
        (
            "add_resource={type=syn_ore amount=0 state=1}",
            C(1),
            "Minus 0\u{13}resources_strip|7 /One/0\n",
            None,
        ),
        (
            "add_resource={type=syn_ore amount=1 state=1 days=2}",
            C(1),
            "TempAdd 1\u{13}resources_strip|7 /One/2\n",
            None,
        ),
        (
            "add_resource={type=syn_ore amount=syn_amount state=1 days=syn_days}",
            C(1),
            "TempAdd 2\u{13}resources_strip|7 /One/-3\n",
            None,
        ),
        (
            "add_resource={type=syn_ore amount=-1.5 days=-2}",
            S(1),
            "TempMinus 2\u{13}resources_strip|7 /One/-2\n",
            None,
        ),
        (
            "add_resource={type=syn_ore amount=1 state=1 show_state_in_tooltip=no}",
            C(1),
            "AddHidden 1\u{13}resources_strip|7 \n",
            None,
        ),
        (
            "add_resource={type=syn_ore amount=-1 state=1 show_state_in_tooltip=no}",
            C(1),
            "MinusHidden 1\u{13}resources_strip|7 \n",
            None,
        ),
        (
            "add_resource={type=syn_ore amount=1 state=1 days=2 show_state_in_tooltip=no}",
            C(1),
            "TempAddHidden 1\u{13}resources_strip|7 /2\n",
            None,
        ),
        (
            "add_resource={type=syn_ore amount=-1 state=1 days=2 show_state_in_tooltip=no}",
            C(1),
            "TempMinusHidden 1\u{13}resources_strip|7 /2\n",
            None,
        ),
        (
            "add_resource={type=syn_ore amount=1 state=0 days=unavailable}",
            S(1),
            "",
            None,
        ),
        (
            "create_railway_gun={location=999}",
            C(1),
            "Gun Alphan Alpha",
            None,
        ),
        ("create_railway_gun={}", C(1), "Gun Alphan Alpha", None),
        (
            "create_railway_gun={location=1}",
            C(1),
            "GunAt Alphan/One Alpha",
            None,
        ),
        ("build_railway={level=5 path={999}}", C(1), "", None),
        ("build_railway={path={1}}", C(1), "Rail 1/One Alpha", None),
        (
            "build_railway={level=.5 path={1 2}}",
            C(1),
            "Rail 1/One/Two Alpha",
            None,
        ),
        (
            "build_railway={path={1 999 2}}",
            C(1),
            "Rail 1/One/Two Alpha",
            None,
        ),
        (
            "build_railway={path={1} build_only_on_allied=yes}",
            S(1),
            "Rail 1/One One",
            None,
        ),
        ("build_railway={path={5}}", C(1), "", None),
        ("build_railway={}", C(1), "", None),
        (
            "build_railway={path={1} controller_priority={factor=2}}",
            C(1),
            "",
            Some("controller"),
        ),
        (
            "build_railway={path={1} build_only_on_allied=yes}",
            C(1),
            "",
            Some("country/controller eligibility"),
        ),
        (
            "remove_building={type=syn_tower province=1}",
            S(1),
            "",
            Some("MovementMap::terrain"),
        ),
        (
            "remove_building={type=syn_mill state=1 tags={syn_tag}}",
            S(1),
            "",
            Some("tag-selected"),
        ),
        (
            "add_resource={type=syn_missing amount=1 state=1}",
            C(1),
            "",
            Some("null resource"),
        ),
        (
            "build_railway={start_state=1 target_state=2}",
            C(1),
            "",
            Some("endpoint selection/pathfinding"),
        ),
        ("add_building_construction={}", C(1), "", Some("template")),
        (
            "add_building_construction={type=syn_missing}",
            C(1),
            "",
            Some("null-template"),
        ),
        (
            "modify_building_resources={building=syn_mill resource=syn_ore}",
            C(1),
            "",
            Some("missing or failed amount assignment leaves the amount uninitialised"),
        ),
        (
            "modify_building_resources={building=syn_mill amount=1}",
            C(1),
            "",
            Some("missing resource leaves the resource index uninitialised"),
        ),
    ] {
        let mut scopes = EventScopes::new(vec![EventScope::of(object)]);
        let out = fixture.render(body, &mut scopes, cache.view(None));
        assert_eq!(out.text, expected, "{body}: {:?}", out.refusals);
        if let Some(reason) = refusal {
            assert_eq!(
                out.refusals.values().sum::<usize>(),
                1,
                "{body}: {:?}",
                out.refusals
            );
            assert!(
                out.refusals.keys().any(|why| why.contains(reason)),
                "{body}: {:?}",
                out.refusals
            );
        } else {
            assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
        }
    }
}

#[test]
fn state_content_province_display_uses_fallbacks() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = Fixture::new();
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *fixture.frame.tables[TableId::CountryAppearance.index()]
    {
        rows.iter_mut()
            .find(|row| row.country == 100)
            .unwrap()
            .original_tag_reference = Recorded::Value(Some(0));
    }
    fixture.catalogue.localisation = fixture
        .catalogue
        .localisation
        .iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .chain(
            [
                ("EFFECT_SET_VP", "SetVP $PROVINCE$/$VALUE|0$"),
                ("EFFECT_ADD_VP", "AddVP $PROVINCE$/$VALUE|0$"),
                ("EFFECT_SUBTRACT_VP", "SubVP $PROVINCE$/$VALUE|0$"),
                ("EFFECT_SET_PROVINCE_NAME", "Rename $OLDNAME$/$NAME$"),
                ("EFFECT_RESET_PROVINCE_NAME", "ResetProvince $NAME$"),
                ("SYN_NAME", "New [This.GetName]"),
                ("EMPTY", ""),
                ("AAA_VICTORY_POINTS_2", "Qualified"),
                ("VICTORY_POINTS_2", "Generic"),
                ("AAA_VICTORY_POINTS_3", ""),
                ("VICTORY_POINTS_3", "GenericThree"),
            ]
            .map(|(key, value)| (key.into(), value.into())),
        )
        .collect::<Localisation>()
        .into();
    Arc::make_mut(&mut fixture.catalogue.localisation).language = "english".into();
    if let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *fixture.frame.tables[TableId::Provinces.index()]
    {
        rows[0].victory_points = 1;
    }
    for (body, expected) in [
        ("set_victory_points={province=1 value=-2.5}", "SetVP One/-3"),
        ("add_victory_points={province=1 value=2.49}", "AddVP One/2"),
        ("add_victory_points={province=1 value=-2.5}", "SubVP One/3"),
        ("add_victory_points={province=1 value=0}", "SubVP One/0"),
        (
            "set_province_name={id=1 name=SYN_NAME}",
            "Rename One/New Alpha\n",
        ),
        ("reset_province_name=1", "ResetProvince One"),
    ] {
        let mut scopes = EventScopes::new(vec![EventScope::of(C(1))]);
        let out = fixture.render(body, &mut scopes, Content::default());
        assert_eq!(out.text, expected, "{body}: {:?}", out.refusals);
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
    }
    for (id, raw, display) in [
        (1, b"".as_slice(), "One"),
        (2, b"".as_slice(), "Qualified"),
        (2, b"EMPTY".as_slice(), ""),
        (2, b"\0hidden".as_slice(), ""),
        (3, b"".as_slice(), "Three"),
    ] {
        *fixture.frame.tables[TableId::ProvinceNames.index()] =
            TableView::Complete(TableRows::ProvinceNames(vec![ProvinceNameRow {
                province: id,
                name: raw.to_vec(),
            }]));
        let text = ScriptedText::new(
            &fixture.frame,
            &fixture.catalogue.localisation,
            fixture.catalogue.scripted_content().unwrap(),
            1,
        );
        assert_eq!(
            text.province_display_name(id).unwrap(),
            display,
            "{id}/{raw:?}"
        );
    }
    *fixture.frame.tables[TableId::ProvinceNames.index()] =
        TableView::Complete(TableRows::ProvinceNames(vec![ProvinceNameRow {
            province: 2,
            name: b"EMPTY".to_vec(),
        }]));
    let mut scopes = EventScopes::new(vec![EventScope::of(C(1))]);
    let out = fixture.render(
        "set_victory_points={province=2 value=2}",
        &mut scopes,
        Content::default(),
    );
    assert_eq!(out.text, "SetVP EMPTY/2");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    for (reference, reason) in [
        (
            Recorded::NotRecorded,
            "original tag reference of country appearance not recorded",
        ),
        (
            Recorded::Value(None),
            "original tag reference of country appearance capture refused",
        ),
    ] {
        if let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *fixture.frame.tables[TableId::CountryAppearance.index()]
        {
            rows.iter_mut()
                .find(|row| row.country == 100)
                .unwrap()
                .original_tag_reference = reference;
        }
        let out = fixture.render(
            "set_victory_points={province=1 value=-2.5}",
            &mut EventScopes::new(vec![EventScope::of(C(1))]),
            Content::default(),
        );
        assert!(out.text.is_empty(), "{:?}", out.refusals);
        assert_eq!(
            out.refusals.values().sum::<usize>(),
            1,
            "{:?}",
            out.refusals
        );
        assert!(
            out.refusals.keys().any(|why| why.contains(reason)),
            "{:?}",
            out.refusals
        );
    }
}

#[test]
fn state_effect_zero_keeps_target() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    for (body, expected) in [
        ("add_state_claim=0", "Claim \n"),
        ("transfer_state=0", "Transfer \n"),
        ("add_state_claim=THIS", "Claim One\n"),
        ("transfer_state=THIS", "Transfer One\n"),
    ] {
        let mut scopes = EventScopes::new(vec![EventScope::of(S(1))]);
        let out = fixture.render(body, &mut scopes, Content::default());
        assert_eq!(out.text, expected, "{body}: {:?}", out.refusals);
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
    }
}

#[test]
fn reserved_country_names_describe_omitted_country_operands() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = Fixture::new();
    fixture.catalogue.localisation = fixture
        .catalogue
        .localisation
        .iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .chain(
            [
                ("ZERO", "Reserved"),
                ("ZERO_DEF", "the reserved"),
                ("ZERO_ADJ", "reservedish"),
                (
                    "EFFECT_ADD_RESISTANCE_TARGET",
                    "Target $VALUE$/$OCCUPIER$/$OCCUPIED$",
                ),
                ("EFFECT_GIVE_RESOURCE_RIGHTS", "Rights $RECEIVER$/$STATE$"),
                ("EFFECT_SET_STATE_OWNER_TO", "Owner $COUNTRY$"),
                ("EFFECT_CREATE_RAILWAY_GUN", "Gun $OWNER$"),
                ("EFFECT_ADD_TO_WAR", "War $COUNTRY1$/$COUNTRY2$"),
            ]
            .map(|(key, value)| (key.into(), value.into())),
        )
        .collect::<Localisation>()
        .into();
    Arc::make_mut(&mut fixture.catalogue.localisation).language = "english".into();
    *fixture.frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"ZERO".to_vec(),
                built: 7,
                ..Default::default()
            },
        ]));
    for (body, object, expected) in [
        (
            "add_resistance_target=5",
            S(1),
            "Target \u{11}R+5.0%\u{11}!/\u{12}ZERO The reserved/\u{12}ZERO The reserved\n",
        ),
        (
            "give_resource_rights={state=1}",
            C(1),
            "Rights the reserved/One\n",
        ),
        ("set_state_owner_to=0", S(1), "Owner Reserved\n"),
        ("create_railway_gun={location=999}", C(0), "Gun reservedish"),
        ("add_to_war={}", C(1), "War Reserved/Reserved\n"),
    ] {
        let mut scopes = EventScopes::new(vec![EventScope::of(object)]);
        let out = fixture.render(body, &mut scopes, Content::default());
        assert_eq!(out.text, expected, "{body}: {:?}", out.refusals);
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
    }
    if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *fixture.frame.tables[TableId::ReservedCountryNaming.index()]
    {
        rows[0].built = 0;
    }
    let mut scopes = EventScopes::new(vec![EventScope::of(S(1))]);
    let out = fixture.render("add_resistance_target=5", &mut scopes, Content::default());
    assert_eq!(
        out.text,
        "Target \u{11}R+5.0%\u{11}!/\u{12}ZERO /\u{12}ZERO \n"
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    *fixture.frame.tables[TableId::ReservedCountryNaming.index()] = TableView::Unavailable {
        reason: "format 98".into(),
    };
    let out = fixture.render("add_resistance_target=5", &mut scopes, Content::default());
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("ReservedCountryNaming"))
    );
}

#[test]
fn country_effect_state_links_use_recorded_owner() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = Fixture::new();
    fixture.catalogue.localisation = fixture
        .catalogue
        .localisation
        .iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .chain([
            (
                "EFFECT_SET_STATE_OWNER_TO".into(),
                "OwnerTo $COUNTRY$".into(),
            ),
            (
                "EFFECT_SET_STATE_CONTROLLER_TO".into(),
                "ControllerTo $COUNTRY$".into(),
            ),
            (
                "EFFECT_TRANSFER_STATE_TO".into(),
                "TransferTo $COUNTRY$".into(),
            ),
            (
                "EFFECT_INIT_RESISTANCE_BY_COUNTRY".into(),
                "Init $COUNTRY$".into(),
            ),
        ])
        .collect::<Localisation>()
        .into();
    Arc::make_mut(&mut fixture.catalogue.localisation).language = "english".into();
    if let TableView::Complete(TableRows::States(rows)) =
        &mut *fixture.frame.tables[TableId::States.index()]
    {
        rows[1].owner = 200;
        rows[1].controller = 300;
        rows[2].owner = 300;
    }
    for (body, expected) in [
        ("set_state_owner_to=ROOT", "OwnerTo Beta\n"),
        ("set_state_controller_to=PREV", "ControllerTo Gamma\n"),
        ("transfer_state_to=FROM", "TransferTo Alpha\n"),
        ("start_resistance=ROOT", "Init Beta"),
        ("transfer_state_to=THIS", "TransferTo Alpha\n"),
    ] {
        let mut scopes = EventScopes::new(vec![
            EventScope {
                root: Link::Scope(1),
                prev: Link::Scope(2),
                from: Link::Scope(3),
                ..EventScope::of(S(1))
            },
            EventScope::of(S(2)),
            EventScope::of(S(3)),
            EventScope::of(S(4)),
        ]);
        let out = fixture.render(body, &mut scopes, Content::default());
        assert_eq!(out.text, expected, "{body}: {:?}", out.refusals);
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
    }
}

#[test]
fn states_branches() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = Fixture::new();
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *fixture.frame.tables[TableId::CountryAppearance.index()]
    {
        rows.iter_mut()
            .find(|row| row.country == 100)
            .unwrap()
            .original_tag_reference = Recorded::Value(Some(0));
    }
    fixture.catalogue.localisation = fixture
        .catalogue
        .localisation
        .iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .chain(
            [
                ("EFFECT_ADD_STATE_CORE", "Core $STATE$"),
                ("EFFECT_REMOVE_STATE_CLAIM", "Unclaim $STATE$"),
                ("EFFECT_REMOVE_STATE_CORE", "Uncore $STATE$"),
                ("EFFECT_SET_CAPITAL", "Capital $STATE$"),
                ("EFFECT_SET_STATE_OWNER", "Owner $STATE$"),
                ("EFFECT_SET_STATE_CONTROLLER", "Controller $STATE$"),
                ("EFFECT_SET_STATE_OWNER_TO", "OwnerTo $COUNTRY$"),
                ("EFFECT_SET_STATE_CONTROLLER_TO", "ControllerTo $COUNTRY$"),
                ("EFFECT_TRANSFER_STATE_TO", "TransferTo $COUNTRY$"),
                ("EFFECT_RESET_STATE_NAME", "Reset"),
                ("EFFECT_SET_STATE_NAME", "Name $NAME$"),
                ("SYN_NAME", "New [This.GetName]"),
                ("EFFECT_ADD_COMPLIANCE", "Compliance $VALUE$"),
                ("EFFECT_ADD_RESISTANCE", "Resistance $VALUE$"),
                ("EFFECT_SET_COMPLIANCE", "SetCompliance $VALUE$"),
                ("EFFECT_SET_RESISTANCE", "SetResistance $VALUE$"),
                ("EFFECT_DEMILITARIZED_ZONE", "Demilitarized"),
                ("EFFECT_NOT_DEMILITARIZED_ZONE", "Militarized"),
                (
                    "EFFECT_ADD_EXTRA_STATE_SHARED_BUILDING_SLOTS",
                    "Slots $COUNT|0$",
                ),
                (
                    "EFFECT_REMOVE_EXTRA_STATE_SHARED_BUILDING_SLOTS",
                    "Unslots $COUNT|0$",
                ),
                ("EFFECT_CANCEL_RESISTANCE", "Cancel [This.GetName]"),
                ("EFFECT_FORCE_ADD_RESISTANCE", "Enable $STATE$"),
                ("EFFECT_FORCE_ADD_RESISTANCE_CLEAR", "Clear $STATE$"),
                ("EFFECT_INIT_RESISTANCE", "Init"),
                ("EFFECT_INIT_RESISTANCE_BY_COUNTRY", "Init $COUNTRY$"),
                ("EFFECT_SUPPLY_UNITS", "Supply $NUM|2$"),
                (
                    "EFFECT_ADD_RESISTANCE_TARGET_WITH_FROM_AND_TARGET",
                    "Target $VALUE$/$OCCUPIER$/$OCCUPIED$/$NUM_DAYS$ [This.GetName]",
                ),
                ("DURATION_DAYS", "$NUM$ days [This.GetName]"),
                ("EFFECT_GO_TO_PROVINCE", "Go $STATE$"),
                ("EFFECT_SET_PROVINCE_CONTROLLER", "Control $STATE$"),
                ("EFFECT_GIVE_RESOURCE_RIGHTS", "Rights $RECEIVER$/$STATE$"),
                ("EFFECT_REMOVE_RESOURCE_RIGHTS", "End $STATE$"),
                ("EFFECT_SET_VP", "SetVP $PROVINCE$/$VALUE|0$"),
                ("EFFECT_ADD_VP", "AddVP $PROVINCE$/$VALUE|0$"),
                ("EFFECT_SET_PROVINCE_NAME", "Rename $OLDNAME$/$NAME$"),
                ("EFFECT_RESET_PROVINCE_NAME", "ResetProvince $NAME$"),
                ("EFFECT_ADD_RESOURCE", "Resource $RESOURCE$/$STATE$"),
                ("AAA_ADJ", "Alphan"),
                ("EFFECT_CREATE_RAILWAY_GUN", "Gun $OWNER$"),
                (
                    "EFFECT_CREATE_RAILWAY_GUN_LOCATION",
                    "GunAt $OWNER$/$STATE$",
                ),
            ]
            .map(|(key, value)| (key.into(), value.into())),
        )
        .collect::<Localisation>()
        .into();
    Arc::make_mut(&mut fixture.catalogue.localisation).language = "english".into();
    if let TableView::Complete(TableRows::Politics(rows)) =
        &mut *fixture.frame.tables[TableId::Politics.index()]
    {
        rows[0].capital = 4;
    }
    if let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *fixture.frame.tables[TableId::Provinces.index()]
    {
        rows[0].victory_points = 1;
        rows[4].state = 0;
        rows.push(ProvinceRow {
            id: 8,
            state: 3,
            ..Default::default()
        });
    }
    if let TableView::Complete(TableRows::States(rows)) =
        &mut *fixture.frame.tables[TableId::States.index()]
    {
        rows.push(StateRow {
            id: 6,
            ..Default::default()
        });
    }
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![
            ScriptVariablesRow {
                scope_kind: 1,
                scope_id: 100,
                seed: replay_data::rows::Recorded::Value(None),
                entries: vec![ScriptVariableEntry {
                    name: b"syn".to_vec(),
                    value_raw: 249_999,
                }],
            },
            ScriptVariablesRow {
                scope_kind: 2,
                scope_id: 1,
                seed: replay_data::rows::Recorded::Value(None),
                entries: vec![ScriptVariableEntry {
                    name: b"syn".to_vec(),
                    value_raw: 249_999,
                }],
            },
        ]));
    *fixture.frame.tables[TableId::StateResistance.index()] =
        TableView::Complete(TableRows::StateResistance(vec![StateResistanceRow {
            state: 1,
            country: 200,
            active: 0,
            ..Default::default()
        }]));
    let mut names = content_reader_script::key_names::KeyNames::default();
    names
        .countries
        .extend([b"AAA".to_vec(), b"BBB".to_vec(), b"CCC".to_vec()]);
    names.counts.buildings.insert(b"syn_mill".to_vec());
    let resources = [content_reader_key_names::resources::Resource {
        token: "syn_ore".into(),
        icon_frame: Some(7),
    }];
    let content = Content {
        key_names: Some(&names),
        resources: Some(&resources),
        ..Default::default()
    };
    for (body, object, expected, refusal) in [
        ("add_state_core=2.5", C(1), "Core Three\n", None),
        ("add_state_core=1", C(1), "Core One\n", None),
        ("add_state_core=0", S(1), "Core \n", None),
        ("add_state_core=-1", C(1), "Core \n", None),
        ("add_state_core=syn", C(1), "Core Two\n", None),
        ("add_state_core=THIS", S(1), "Core One\n", None),
        ("add_state_core=ROOT", C(1), "Core \n", None),
        ("add_state_core=PREV", S(1), "Core One\n", None),
        ("add_state_core=FROM", S(1), "Core One\n", None),
        (
            "add_state_core=event_target:syn_state",
            C(1),
            "Core Three\n",
            None,
        ),
        ("add_state_core=owner", S(1), "Core \n", None),
        ("add_state_core=capital", S(0), "Core \n", None),
        (
            "add_state_core=capital",
            S(u32::MAX),
            "",
            Some("invalid state"),
        ),
        ("add_state_core=capital", C(1), "Core \n", None),
        ("add_state_core=capital", S(1), "Core Four\n", None),
        (
            "add_state_core=capital",
            S(6),
            "",
            Some("reserved country zero's capital state"),
        ),
        (
            "goto_state=capital",
            S(999),
            "",
            Some("missing state owner"),
        ),
        ("remove_state_claim=99", C(1), "Unclaim \n", None),
        ("remove_state_core=2", C(1), "Uncore Two\n", None),
        ("set_capital={state=3}", C(1), "Capital Three\n", None),
        ("set_capital={}", S(1), "Capital \n", None),
        (
            "set_capital={state=syn remember_old_capital=no}",
            C(1),
            "Capital Two\n",
            None,
        ),
        (
            "set_capital={state=THIS remember_old_capital=yes}",
            S(1),
            "Capital One\n",
            None,
        ),
        ("set_state_owner=4", C(1), "Owner Four\n", None),
        ("set_state_controller=5", C(1), "Controller Five\n", None),
        ("set_state_owner_to=BBB", S(1), "OwnerTo Beta\n", None),
        (
            "set_state_controller_to=CCC",
            S(1),
            "ControllerTo Gamma\n",
            None,
        ),
        ("transfer_state_to=AAA", S(1), "TransferTo Alpha\n", None),
        ("transfer_state_to=THIS", C(1), "TransferTo Alpha\n", None),
        ("set_state_owner_to=owner", S(1), "OwnerTo Alpha\n", None),
        (
            "set_state_controller_to=controller",
            S(1),
            "ControllerTo Alpha\n",
            None,
        ),
        (
            "transfer_state_to=event_target:syn_country",
            S(1),
            "TransferTo Beta\n",
            None,
        ),
        ("set_state_owner_to=ROOT", S(1), "OwnerTo Alpha\n", None),
        (
            "set_state_owner_to=ROOT",
            S(999),
            "",
            Some("invalid nonzero state"),
        ),
        (
            "start_resistance=ROOT",
            S(u32::MAX),
            "",
            Some("invalid nonzero state"),
        ),
        (
            "set_state_controller_to=event_target:syn_state",
            S(1),
            "",
            Some("country zero"),
        ),
        (
            "transfer_state_to=occupied",
            S(1),
            "TransferTo Beta\n",
            None,
        ),
        ("transfer_state_to=0", S(1), "", Some("country zero")),
        (
            "set_state_name=SYN_NAME",
            S(1),
            "Name New [This.GetName]\n",
            None,
        ),
        ("set_state_name=\"\"", S(1), "Reset\n", None),
        ("reset_state_name=yes", S(1), "Reset\n", None),
        (
            "add_compliance=1.25",
            S(1),
            "Compliance \u{11}G+1.2%\u{11}!\n",
            None,
        ),
        (
            "add_compliance=-1.25",
            S(1),
            "Compliance \u{11}R-1.2%\u{11}!\n",
            None,
        ),
        (
            "add_resistance=1.25",
            S(1),
            "Resistance \u{11}R+1.2%\u{11}!\n",
            None,
        ),
        (
            "add_resistance=-1.25",
            S(1),
            "Resistance \u{11}G-1.2%\u{11}!\n",
            None,
        ),
        (
            "add_compliance=0",
            S(1),
            "Compliance \u{11}Y+0.0%\u{11}!\n",
            None,
        ),
        (
            "add_resistance=0",
            S(1),
            "Resistance \u{11}Y+0.0%\u{11}!\n",
            None,
        ),
        (
            "add_resistance=ROOT",
            S(1),
            "Resistance \u{11}Y+0.0%\u{11}!\n",
            None,
        ),
        ("set_compliance=1.25", S(1), "SetCompliance 1.2%\n", None),
        ("set_resistance=-1.25", S(1), "SetResistance -1.2%\n", None),
        ("set_compliance=ROOT", S(1), "SetCompliance 0.0%\n", None),
        ("set_demilitarized_zone=yes", S(1), "Demilitarized\n", None),
        ("set_demilitarized_zone=no", S(1), "Militarized\n", None),
        (
            "set_demilitarized_zone=invalid",
            S(1),
            "Militarized\n",
            None,
        ),
        ("set_demilitarized_zone=ROOT", S(1), "Militarized\n", None),
        ("set_demilitarized_zone=yes", S(999), "", None),
        (
            "add_extra_state_shared_building_slots=0.5",
            S(1),
            "Slots 1\n",
            None,
        ),
        (
            "add_extra_state_shared_building_slots=-0.5",
            S(1),
            "Unslots 1\n",
            None,
        ),
        (
            "add_extra_state_shared_building_slots=0",
            S(1),
            "Unslots 0\n",
            None,
        ),
        (
            "add_extra_state_shared_building_slots=ROOT",
            S(1),
            "Unslots 0\n",
            None,
        ),
        ("cancel_resistance=no", C(1), "Cancel [This.GetName]", None),
        (
            "force_enable_resistance={clear=yes}",
            S(1),
            "Clear One",
            None,
        ),
        (
            "force_enable_resistance={clear=yes clear=invalid}",
            S(1),
            "Clear One",
            None,
        ),
        (
            "force_enable_resistance={clear=no}",
            S(1),
            "Enable One",
            None,
        ),
        ("force_enable_resistance=BBB", S(1), "Enable One", None),
        ("force_enable_resistance={}", S(999), "", None),
        (
            "force_enable_resistance={add_political_power=1}",
            S(1),
            "Enable One\nPP1\n",
            None,
        ),
        (
            "start_resistance={}",
            S(1),
            "",
            Some("requires a scalar operand"),
        ),
        (
            "add_resistance_target={amount={value=0}}",
            S(1),
            "",
            Some("amount requires a scalar operand"),
        ),
        (
            "set_victory_points={province={value=1}}",
            S(1),
            "",
            Some("province requires a scalar operand"),
        ),
        ("start_resistance=BBB", S(1), "Init Beta", None),
        ("start_resistance=owner", S(1), "Init Alpha", None),
        ("start_resistance=controller", S(1), "Init Alpha", None),
        ("start_resistance=occupied", S(1), "Init Beta", None),
        (
            "start_resistance=event_target:syn_country",
            S(1),
            "Init Beta",
            None,
        ),
        ("start_resistance=capital", S(1), "Init", None),
        (
            "start_resistance=event_target:syn_state",
            S(1),
            "Init",
            None,
        ),
        ("start_resistance=owner", C(1), "Init", None),
        (
            "transfer_state_to=controller",
            C(1),
            "",
            Some("country zero"),
        ),
        (
            "transfer_state_to=owner",
            EventObject::Province(1),
            "",
            Some("event scope's state ID"),
        ),
        ("start_resistance=ROOT", S(0), "Init", None),
        ("start_resistance=yes", S(1), "Init", None),
        ("start_resistance=0", S(1), "Init", None),
        ("start_resistance=ROOT", C(1), "Init Alpha", None),
        ("start_resistance=ROOT", S(1), "Init Alpha", None),
        (
            "set_state_category=SYN_CATEGORY",
            S(1),
            "",
            Some("state categories content unavailable"),
        ),
        ("supply_units=3.25", S(1), "Supply 3.25", None),
        ("supply_units=ROOT", S(1), "Supply 0.00", None),
        ("add_resistance_target=0", S(1), "", None),
        (
            "add_resistance_target={amount=0 days=random occupied=unrecorded}",
            S(1),
            "",
            None,
        ),
        (
            "add_resistance_target={}",
            S(1),
            "",
            Some("reserved country naming has no singleton row"),
        ),
        (
            "add_resistance_target=5",
            S(1),
            "",
            Some("reserved country naming has no singleton row"),
        ),
        (
            "add_resistance_target={amount=1 occupied=AAA}",
            S(1),
            "",
            Some("reserved country naming has no singleton row"),
        ),
        (
            "add_resistance_target={amount=1 occupier=BBB}",
            S(1),
            "",
            Some("reserved country naming has no singleton row"),
        ),
        (
            "add_resistance_target={amount=1 occupied=AAA occupier=BBB}",
            S(1),
            "Target \u{11}R+1.0%\u{11}!/\u{12}BBB Beta/\u{12}AAA Alpha/0 [This.GetName]\n",
            None,
        ),
        (
            "add_resistance_target={occupied=AAA occupier=BBB days=2.5}",
            S(1),
            "Target \u{11}G-1.0%\u{11}!/\u{12}BBB Beta/\u{12}AAA Alpha/3 [This.GetName] (3 days [This.GetName])\n",
            None,
        ),
        (
            "add_resistance_target={amount=-1.25 occupied=BBB occupier=AAA days=-2.5}",
            C(1),
            "Target \u{11}G-1.2%\u{11}!/\u{12}AAA Alpha/\u{12}BBB Beta/-3 [This.GetName]\n",
            None,
        ),
        (
            "add_resistance_target={amount=var:syn occupied=event_target:syn_country occupier=AAA days=var:syn}",
            S(1),
            "Target \u{11}R+2.4%\u{11}!/\u{12}AAA Alpha/\u{12}BBB Beta/2 [This.GetName] (2 days [This.GetName])\n",
            None,
        ),
        (
            "add_resistance_target={amount=var:syn occupied=event_target:syn_country occupier=controller days=var:syn}",
            S(1),
            "Target \u{11}R+2.4%\u{11}!/\u{12}AAA Alpha/\u{12}BBB Beta/2 [This.GetName] (2 days [This.GetName])\n",
            None,
        ),
        (
            "add_resistance_target={amount=-92233720368547.75808 occupied=AAA occupier=BBB}",
            S(1),
            "Target \u{11}Y+0.0%\u{11}!/\u{12}BBB Beta/\u{12}AAA Alpha/0 [This.GetName]\n",
            None,
        ),
        (
            "add_resistance_target={amount=1 amount=0 occupied=AAA occupier=BBB}",
            S(1),
            "",
            Some("repeated resistance-target operand amount"),
        ),
        (
            "add_resistance_target={amount=1 days=random occupied=AAA occupier=BBB}",
            S(1),
            "",
            Some("seed of script variables"),
        ),
        (
            "add_resistance_target={amount=0 add_political_power=1}",
            S(1),
            "PP1\n",
            None,
        ),
        ("goto_state=1", C(1), "Go One\n", None),
        ("goto_state=999", S(1), "", None),
        ("goto_state=0", S(1), "", None),
        ("goto_state=THIS", S(1), "Go One\n", None),
        ("goto_state=capital", S(0), "", None),
        ("goto_state=capital", C(1), "", None),
        ("goto_state=capital", S(1), "Go Four\n", None),
        ("goto_province=010", C(1), "Go Three\n", None),
        ("goto_province=2", C(1), "Go Two\n", None),
        ("goto_province=\"2.9\"", C(1), "Go Two\n", None),
        ("goto_province=03", C(1), "Go Three\n", None),
        ("goto_province=999", C(1), "", Some("invalid province")),
        ("goto_province=5", C(1), "", Some("province has no state")),
        ("set_province_controller=2", C(1), "Control Two", None),
        ("set_province_controller=999", C(1), "", None),
        ("set_province_controller=THIS", S(1), "", None),
        ("set_victory_points={province=999 value=1}", C(1), "", None),
        ("set_victory_points={province=5.5 value=1}", C(1), "", None),
        ("add_victory_points={value=1}", C(1), "", None),
        (
            "set_victory_points={province=1.49 value=-2.5}",
            C(1),
            "SetVP One/-3",
            None,
        ),
        (
            "add_victory_points={province=1 value=2}",
            C(1),
            "AddVP One/2",
            None,
        ),
        (
            "set_province_name={}",
            C(1),
            "ERROR: Invalid location or missing victory point for rename",
            None,
        ),
        (
            "set_province_name={id=2 name=SYN_NAME}",
            C(1),
            "ERROR: Invalid location or missing victory point for rename",
            None,
        ),
        (
            "set_province_name={id=1 id=invalid name=SYN_NAME}",
            C(1),
            "Rename One/New Alpha\n",
            None,
        ),
        ("reset_province_name=999", C(1), "", None),
        (
            "add_building_construction={type=syn_mill level=1}",
            C(1),
            "",
            Some("buildings content unavailable"),
        ),
        (
            "set_building_level={type=syn_mill level=2 province=1}",
            S(1),
            "",
            Some("buildings content unavailable"),
        ),
        (
            "add_offsite_building={type=syn_mill level=-1}",
            C(1),
            "",
            Some("buildings content unavailable"),
        ),
        (
            "construct_building_in_random_province={syn_mill=2}",
            S(1),
            "",
            Some("buildings content unavailable"),
        ),
        ("damage_building={}", S(1), "", None),
        (
            "damage_building={state=999 province=999 damage=2}",
            S(1),
            "",
            None,
        ),
        (
            "damage_building={type=syn_mill damage=.5}",
            S(1),
            "",
            Some("buildings content unavailable"),
        ),
        (
            "damage_building={tags={syn_tag}}",
            S(1),
            "",
            Some("tag-selected building helper"),
        ),
        ("remove_building={}", S(1), "", None),
        (
            "remove_building={type=syn_mill level=2}",
            S(1),
            "",
            Some("buildings content unavailable"),
        ),
        (
            "modify_building_resources={building=syn_unknown amount=3 resource=syn_ore}",
            C(1),
            "",
            None,
        ),
        ("modify_building_resources={building={}}", C(1), "", None),
        (
            "modify_building_resources={building=syn_mill amount=3 resource=syn_ore}",
            C(1),
            "",
            Some("buildings content unavailable"),
        ),
        (
            "add_resource={type=syn_ore amount=2 state=999}",
            C(1),
            "",
            None,
        ),
        ("add_resource={type=syn_ore amount=2}", C(1), "", None),
        (
            "add_resource={type=syn_ore amount=2}",
            S(1),
            "Resource 2\u{13}resources_strip|7 /One\n",
            None,
        ),
        (
            "add_resource={type=syn_ore amount=-2 state=0}",
            S(1),
            "",
            None,
        ),
        (
            "give_resource_rights={receiver=BBB state=1}",
            C(1),
            "Rights beta/One\n",
            None,
        ),
        (
            "give_resource_rights={receiver=BBB state=1 resources={syn_ore}}",
            C(1),
            "Rights beta/One\n",
            None,
        ),
        ("give_resource_rights={receiver=BBB}", S(1), "", None),
        (
            "give_resource_rights={receiver=BBB state=999}",
            C(1),
            "",
            None,
        ),
        (
            "give_resource_rights={state=1}",
            C(1),
            "",
            Some("reserved country naming has no singleton row"),
        ),
        ("remove_resource_rights=0", S(1), "", None),
        ("remove_resource_rights=THIS", S(1), "End One\n", None),
        ("remove_resource_rights=2", C(1), "End Two\n", None),
        ("remove_resource_rights=999", C(1), "", None),
        ("build_railway={level=0}", C(1), "", None),
        ("build_railway={level=.49999}", C(1), "", None),
        (
            "build_railway={level=.5}",
            C(1),
            "",
            Some("MAX_RAILWAY_LEVEL content unavailable"),
        ),
        (
            "build_railway={}",
            C(1),
            "",
            Some("MAX_RAILWAY_LEVEL content unavailable"),
        ),
        (
            "build_railway={level=0 add_stability=.1}",
            C(1),
            "S0\n",
            None,
        ),
        (
            "create_railway_gun={location=1}",
            C(1),
            "GunAt Alphan/One",
            None,
        ),
        (
            "create_railway_gun={location=999}",
            C(1),
            "Gun Alphan",
            None,
        ),
        ("strategic_state_location={syn_radar=42}", C(1), "", None),
        ("strategic_state_location={}", S(1), "", None),
        (
            "strategic_state_location={syn_radar=42}",
            S(1),
            "",
            Some("strategic-location templates"),
        ),
        (
            "BBB={add_state_core=1 set_capital={state=2}}",
            C(1),
            "\u{12}BBBBeta:\n   Core One\n   Capital Two\n",
            None,
        ),
        (
            "cancel_resistance=no supply_units=2",
            C(1),
            "Cancel [This.GetName]\nSupply 2.00",
            None,
        ),
    ] {
        let mut scopes = EventScopes::new(vec![EventScope::of(object)]);
        scopes.saved_targets.insert(
            0,
            vec![
                (b"syn_country".to_vec(), C(2)),
                (b"syn_state".to_vec(), S(3)),
            ],
        );
        let out = fixture.render(body, &mut scopes, content);
        assert_eq!(out.text, expected, "{body}: {:?}", out.refusals);
        assert_eq!(
            out.refusals.values().sum::<usize>(),
            usize::from(refusal.is_some()),
            "{body}: {:?}",
            out.refusals
        );
        if let Some(reason) = refusal {
            let parsed = content_reader_parser::parser::parse(body.as_bytes()).unwrap();
            let effect_key = key(&parsed.items[0]);
            assert!(
                out.refusals
                    .keys()
                    .any(|why| why.contains(effect_key.as_str()) && why.contains(reason)),
                "{body}: {:?}",
                out.refusals
            );
        }
    }
    let mut scopes = EventScopes::new(vec![EventScope::of(S(1))]);
    let out = fixture.render(
        "add_resistance_target={amount=0 ignored_field=yes}",
        &mut scopes,
        content,
    );
    assert!(out.text.is_empty());
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(
        out.refusals
            .keys()
            .all(|why| why.contains("unknown effect key ignored_field"))
    );
    let mut scopes = EventScopes::new(vec![
        EventScope {
            root: Link::Scope(1),
            prev: Link::Scope(2),
            from: Link::Scope(3),
            ..EventScope::of(S(1))
        },
        EventScope::of(S(2)),
        EventScope::of(S(3)),
        EventScope::of(S(4)),
    ]);
    let out = fixture.render(
        "add_state_core=ROOT remove_state_core=PREV set_state_owner=FROM",
        &mut scopes,
        content,
    );
    assert_eq!(out.text, "Core Two\nUncore Three\nOwner Four\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn state_tables_refuse_gaps_and_name_a_late_country() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = Fixture::new();
    for (table, rows, body) in [
        (
            TableId::States,
            TableRows::States(vec![]),
            "add_state_core=999",
        ),
        (
            TableId::Provinces,
            TableRows::Provinces(vec![]),
            "set_province_controller=999",
        ),
        (
            TableId::StateResistance,
            TableRows::StateResistance(vec![]),
            "transfer_state_to=occupied",
        ),
    ] {
        let previous = fixture.frame.tables[table.index()].clone();
        let words = table.words();
        for (view, gap) in [
            (
                TableView::Unavailable {
                    reason: "synthetic gap".into(),
                },
                [
                    format!("{words}: not available: synthetic gap"),
                    format!("hour 1 {table:?} is unavailable: synthetic gap"),
                ],
            ),
            (
                TableView::Truncated {
                    rows: rows.clone(),
                    missing: 1,
                },
                [
                    format!("{words}: read only in part (1 rows missing)"),
                    format!("hour 1 {table:?} is truncated (1 missing rows)"),
                ],
            ),
        ] {
            *fixture.frame.tables[table.index()] = view;
            let mut scopes = EventScopes::new(vec![EventScope::of(S(1))]);
            let out = fixture.render(body, &mut scopes, Content::default());
            assert!(out.text.is_empty());
            assert_eq!(
                out.refusals.values().sum::<usize>(),
                1,
                "{body}: {:?}",
                out.refusals
            );
            assert!(
                out.refusals
                    .keys()
                    .any(|why| gap.iter().any(|gap| why.contains(gap.as_str()))),
                "{body}: {:?}",
                out.refusals
            );
        }
        fixture.frame.tables[table.index()] = previous;
    }
    fixture.set_countries(crate::effect_description::tests::synthetic_countries(24));
    fixture.catalogue.localisation = fixture
        .catalogue
        .localisation
        .iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .chain([
            (
                "EFFECT_TRANSFER_STATE_TO".into(),
                "TransferTo $COUNTRY$".into(),
            ),
            ("ZZZ".into(), "Many".into()),
        ])
        .collect::<Localisation>()
        .into();
    Arc::make_mut(&mut fixture.catalogue.localisation).language = "english".into();
    let mut scopes = EventScopes::new(vec![EventScope::of(C(24))]);
    let out = fixture.render("transfer_state_to=THIS", &mut scopes, Content::default());
    assert_eq!(out.text, "TransferTo Many\n", "{:?}", out.refusals);
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}
