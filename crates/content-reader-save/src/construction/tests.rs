use super::*;

#[test]
fn occupation_active_definitions_preserve_state_and_core_order_duplicates_and_empty_names() {
    let saved = pack(r#"countries={AAA={occupation_status={occupation={BBB={resistance_modifiers={"core_b" "" "core_a" "core_b"} compliance_modifiers={"core_c" "core_c"}}}}}} states={7={resistance={resistance_modifiers={"state_b" "" "state_a" "state_b"} compliance_modifiers={"state_c" "state_c"} resistance_target=-90071992547.40993}}} provinces={}"#).unwrap();
    let state = &saved.state_resistance[&7];
    assert!(!state.active);
    assert_eq!(state.target.raw, -9_007_199_254_740_993);
    assert_eq!(
        state.definitions.resistance,
        [
            b"state_b".to_vec(),
            Vec::new(),
            b"state_a".to_vec(),
            b"state_b".to_vec()
        ]
    );
    assert_eq!(
        state.definitions.compliance,
        [b"state_c".to_vec(), b"state_c".to_vec()]
    );
    let core = &saved.occupation_definitions[&(b"AAA".to_vec(), b"BBB".to_vec())];
    assert_eq!(
        core.resistance,
        [
            b"core_b".to_vec(),
            Vec::new(),
            b"core_a".to_vec(),
            b"core_b".to_vec()
        ]
    );
    assert_eq!(core.compliance, [b"core_c".to_vec(), b"core_c".to_vec()]);
}

#[test]
fn occupation_active_definition_omission_is_empty_and_target_omission_is_zero() {
    let saved = pack("countries={AAA={occupation_status={occupation={BBB={}}}}} states={7={resistance={}} 8={resistance={resistance_modifiers={} compliance_modifiers={} resistance_target=0}}} provinces={}").unwrap();
    assert_eq!(
        saved.state_resistance[&7].definitions,
        ActiveOccupationDefinitions::default()
    );
    assert_eq!(saved.state_resistance[&7].target.raw, 0);
    assert_eq!(
        saved.state_resistance[&8].definitions,
        ActiveOccupationDefinitions::default()
    );
    assert_eq!(saved.state_resistance[&8].target.raw, 0);
    assert_eq!(
        saved.occupation_definitions[&(b"AAA".to_vec(), b"BBB".to_vec())],
        ActiveOccupationDefinitions::default()
    );
    assert!(
        !saved
            .occupation_definitions
            .contains_key(&(b"AAA".to_vec(), b"CCC".to_vec()))
    );
}

#[test]
fn occupation_active_definitions_refuse_keyed_nested_scalar_and_duplicate_lists() {
    for fields in [
        "resistance_modifiers={name=value}",
        "compliance_modifiers={{}}",
        "resistance_modifiers=identity",
        "resistance_modifiers={a} resistance_modifiers={b}",
        "compliance_modifiers={a} compliance_modifiers={b}",
    ] {
        assert!(
            pack(&format!(
                "countries={{}} states={{7={{resistance={{{fields}}}}}}} provinces={{}}"
            ))
            .is_err(),
            "{fields}"
        );
        assert!(
            pack(&format!(
                "countries={{AAA={{occupation_status={{occupation={{BBB={{{fields}}}}}}}}}}} states={{}} provinces={{}}"
            ))
            .is_err(),
            "{fields}"
        );
    }
    assert!(pack("countries={} states={7={resistance={resistance_target=1 resistance_target=2}}} provinces={}").is_err());
}

#[test]
fn occupation_resistance_targets_keep_order_country_omissions_and_emitted_empty_tooltips() {
    let saved = pack("countries={} states={7={resistance={added_resistance_targets={{amount=-1.23456 controller=AAA tooltip=\"\"} {amount=0.00001 occupied=BBB tooltip=fixture}}}}} provinces={}").unwrap();
    let targets = &saved.state_resistance[&7].added;
    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0].amount.raw, -123_456);
    assert_eq!(targets[0].controller.as_deref(), Some(b"AAA".as_slice()));
    assert_eq!(targets[0].occupied, None);
    assert_eq!(targets[0].tooltip, Some(Vec::new()));
    assert_eq!(targets[1].amount.raw, 1);
    assert_eq!(targets[1].occupied.as_deref(), Some(b"BBB".as_slice()));
}

fn railway_fields(extra: &str) -> Vec<Item> {
    content_reader_parser::parser::parse(
        format!(
            "id={{id=4294967295 type=4713}} cost=-90071992547.40993 priority=-9 amount=-2147483648 created_date=1936.1.1.12 base=-3 target=7 current_province=-1 remaining_hours=0.00001 {extra}"
        )
        .as_bytes(),
    )
    .unwrap()
    .items
}

fn pack(input: &str) -> io::Result<Pack> {
    parse(&content_reader_parser::parser::parse(input.as_bytes())?.items)
}

#[test]
fn occupied_country_roster_preserves_saved_group_order() {
    let saved = pack("countries={AAA={occupation_status={occupation={CCC={} BBB={}}}} BBB={occupation_status={}}} states={} provinces={}").unwrap();
    assert_eq!(
        saved.occupation_settings[b"AAA".as_slice()].occupied,
        [b"CCC".to_vec(), b"BBB".to_vec()]
    );
    assert!(
        saved.occupation_settings[b"BBB".as_slice()]
            .occupied
            .is_empty()
    );
    assert!(pack("countries={AAA={occupation_status={occupation={BBB={} BBB={}}}}} states={} provinces={}").is_err());
}

#[test]
fn garrison_equipment_preserves_order_duplicates_extrema_and_missing_objects() {
    let saved = pack("countries={A={occupation_status={occupation={B={state_garrison_data={1={equipment={equipment={id={id=9 type=70} amount=-0.00001} allow_zero_entries=yes} equipment_need={mod_weapon=1 mod_weapon=-92233720368547.75808}} 2={}}}}}}} states={} provinces={}").unwrap();
    let first = &saved.state_garrisons[&(b"A".to_vec(), 1)];
    assert_eq!(
        first.equipment.as_ref().unwrap().equipment[0].amount.raw,
        -1
    );
    assert_eq!(
        first.equipment.as_ref().unwrap().allow_zero_entries,
        Some(true)
    );
    let requirements = first.equipment_need.as_ref().unwrap();
    assert_eq!(
        requirements
            .iter()
            .map(|(_, value)| value.raw)
            .collect::<Vec<_>>(),
        [100_000, i64::MIN]
    );
    let missing = &saved.state_garrisons[&(b"A".to_vec(), 2)];
    assert!(missing.equipment.is_none());
    assert!(missing.equipment_need.is_none());
}

#[test]
fn a_resource_origin_keeps_its_two_efficiencies_convoys_route_and_rights() {
    let saved = pack(
        r#"
        countries={GER={resources={
            origin={id={id=1 type=4713} country="GER" efficiency=0.9 efficiency_due_to_lost_convoys=0.5 request=6 state=119
                resources={steel=3} efficiency=0.45
                delivery_route={type=2 from_state=119 to_state=64 from_port=3379 to_port=6384 sender="GER" receiver="GER" convoys_owner="GER" naval_path={ 43 44 } dirty=no}
                given_resource_rights={ { steel "AFG" } { oil "GER" } }
                convoys_subscriber={ convoys=2 total=5 }
                destination=64}
            extra_resource_origin={origin={id={id=2 type=4713} country="GER" efficiency=1 efficiency_due_to_lost_convoys=1 state=267 efficiency=1
                delivery_route={type=1 from_state=267 to_state=64 land_path={ 267 415 64 } dirty=no}} giver="AFG"}
        }}}
        states={}
        provinces={}
    "#,
    )
    .unwrap();
    let germany = &saved.country_resources[b"GER".as_slice()];
    let [origin] = germany.origins.as_slice() else {
        panic!("{:?}", germany.origins);
    };
    assert_eq!(origin.state, Some(119));
    assert_eq!(
        origin
            .client_efficiency
            .each_ref()
            .map(|value| value.as_ref().map(|value| value.raw)),
        [Some(90_000), Some(50_000)]
    );
    assert_eq!(
        origin.efficiency.as_ref().map(|value| value.raw),
        Some(45_000)
    );
    assert_eq!(origin.request, Some(6));
    assert_eq!(
        origin.convoys,
        crate::economy::ConvoyCounts {
            assigned: 2,
            required: 5
        }
    );
    assert_eq!(
        origin.route,
        Some(SavedRoute {
            kind: 2,
            from_state: Some(119),
            to_state: Some(64),
            from_port: Some(3379),
            to_port: Some(6384),
            land_path: vec![],
            naval_path: vec![43, 44],
        })
    );
    assert_eq!(
        origin.rights,
        [
            (b"steel".to_vec(), b"AFG".to_vec()),
            (b"oil".to_vec(), b"GER".to_vec())
        ]
    );
    let [(held, giver)] = germany.rights_origins.as_slice() else {
        panic!("{:?}", germany.rights_origins);
    };
    assert_eq!(giver.as_slice(), b"AFG");
    assert_eq!(held.state, Some(267));
    assert_eq!(held.convoys, crate::economy::ConvoyCounts::default());
    assert_eq!(
        held.client_efficiency
            .each_ref()
            .map(|value| value.as_ref().map(|value| value.raw)),
        [Some(100_000), Some(100_000)]
    );
    assert_eq!(held.route.as_ref().unwrap().land_path, [267, 415, 64]);
    assert_eq!(held.request, None);
    assert_eq!(germany.origin_states, [119]);
}

#[test]
fn country_resources_keep_origin_states_and_override_entries_in_order() {
    let saved = pack(
        r#"
        countries={ITA={resources={
            origin={id={id=1 type=4713} country="ITA" state=731 destination=2}
            origin={id={id=2 type=4713} country="ITA"}
            origin={id={id=3 type=4713} country="ITA" state=5}
            modify_building_resources={synthetic_refinery={3=3 1=-0.5} dockyard={}}
        }} GER={}}
        states={}
        provinces={}
    "#,
    )
    .unwrap();
    assert_eq!(saved.country_resources.len(), 1);
    let italy = &saved.country_resources[b"ITA".as_slice()];
    assert_eq!(italy.origin_states, [731, 5]);
    let [(refinery, elements), (dockyard, empty)] = italy.building_overrides.as_slice() else {
        panic!("{:?}", italy.building_overrides);
    };
    assert_eq!(
        (refinery.as_slice(), dockyard.as_slice()),
        (b"synthetic_refinery".as_slice(), b"dockyard".as_slice())
    );
    assert_eq!(
        elements
            .iter()
            .map(|(index, amount)| (*index, amount.raw))
            .collect::<Vec<_>>(),
        [(3, 300_000), (1, -50_000)]
    );
    assert!(empty.is_empty());
}

#[test]
fn state_pools_and_dynamic_modifiers_keep_names_and_order() {
    let saved = pack(
        r#"
        countries={}
        states={2={resources={steel=8 oil=-0.5} dynamic_modifier={
            modifier={modifier="b" tag="SWI" enabled=yes}
            modifier={modifier="a" state=4 days=12 enabled=no}
        }} 3={}}
        provinces={}
    "#,
    )
    .unwrap();
    assert_eq!(saved.state_resources[&(2, b"steel".to_vec())].raw, 800_000);
    assert_eq!(saved.state_resources[&(2, b"oil".to_vec())].raw, -50_000);
    let modifiers = &saved.state_dynamic_modifiers[&2];
    assert_eq!(
        modifiers
            .iter()
            .map(|m| (
                m.token.as_slice(),
                m.scope_country.as_deref(),
                m.scope_state
            ))
            .collect::<Vec<_>>(),
        [
            (&b"b"[..], Some(&b"SWI"[..]), None),
            (&b"a"[..], None, Some(4))
        ]
    );
    assert!(!saved.state_dynamic_modifiers.contains_key(&3));
}

#[test]
fn construction_and_buildings_keep_shared_queue_order_and_signed_levels() {
    let saved = pack(r#"
        countries={GER={production={general_lines={
            building={id={id=4 type=59} cost=1 priority=-8 amount=1 created_date=1936.1.1.12 building={template=factory location=3}}
            railway_gun={id={id=1 type=76} priority=1 amount=0 cost=0 created_date="1944.8.26.17" railway_gun={id=1 type=74}}
            railway_gun={id={id=2 type=76} priority=2 amount=0 cost=0 created_date="1944.8.27.1" railway_gun={id=0 type=0}}
            rail_way={id={id=4 type=4713} cost=2 priority=-9 amount=1 created_date=1936.1.1.12 base=-1 target=3 current_province=-1 remaining_hours=-0.00001 rail_way={8 3 8}}
        }} buildings={factory={level=3 healthy_levels=2 partial_health=50}}} ITA={}}
        states={3={buildings={factory={level=-32768 healthy_levels=-2 partial_health=-90071992547.40993} port={level=1 healthy_levels=1 partial_health=0}} resistance={}}}
        provinces={3={buildings={factory={level=32767 healthy_levels=1 partial_health=0.00001}}}}
    "#).unwrap();
    let building = saved.building_construction.values().next().unwrap();
    assert_eq!(building.production.queue_ordinal, 0);
    assert_eq!(building.target_country, None);
    assert_eq!(building.repair_levels, None);
    assert_eq!(
        saved
            .railway_construction
            .values()
            .next()
            .unwrap()
            .production
            .queue_ordinal,
        3
    );
    assert_eq!(
        saved.railway_gun_repairs,
        vec![
            RailwayGunRepair {
                id: Some((1_u64 << 32) | 76),
                country: b"GER".to_vec(),
                queue_ordinal: 1,
                railway_gun: Some((1_u64 << 32) | 74),
            },
            RailwayGunRepair {
                id: Some((2_u64 << 32) | 76),
                country: b"GER".to_vec(),
                queue_ordinal: 2,
                railway_gun: Some(0),
            },
        ]
    );
    let state = &saved.state_buildings[&(3, b"factory".to_vec())];
    assert_eq!(state.level, i16::MIN);
    assert_eq!(state.partial_health.raw, -9_007_199_254_740_993);
    assert_eq!(saved.state_buildings[&(3, b"port".to_vec())].ordinal, 1);
    assert_eq!(
        saved.province_buildings[&(3, b"factory".to_vec())].level,
        i16::MAX
    );
    assert_eq!(saved.state_resistance[&3].level.raw, 0);
    let offsite = &saved.country_buildings[&(b"GER".to_vec(), b"factory".to_vec())];
    assert_eq!((offsite.level, offsite.healthy_level), (3, 2));
    assert_eq!(saved.country_buildings.len(), 1);
}

#[test]
fn occupation_retains_empty_present_law_full_scope_and_manpower_order() {
    let saved = pack(r#"
        countries={GER={occupation_status={default_law="" division_template_id={id=4294967295 type=52}
            foreign_manpower={value={tag=GER value=3} value={tag=FRA value=7}}
            occupation={FRA={occupation_law_list={0="law_a"} state_garrison_data={9={strength_ratio=90071992547.40993 garrison_required=-0.00001 suppression=yes army_manpower_need=-2147483648 manpower_pool={value={tag=GER value=4}}}}}
                ENG={occupation_law_list={0="law_b"}}}
        } collaboration={collaboration={FRA={value=-0.00001}}}}}
        states={9={resistance={resistance=-90071992547.40993 resistance_speed=-0.00001 compliance=3 compliance_speed=0.00001 operational_status=yes occupied_country_tag=FRA}}} provinces={}
    "#).unwrap();
    assert_eq!(
        saved.occupation_settings[b"GER".as_slice()].default_law,
        Some(Vec::new())
    );
    assert_eq!(
        saved.occupation_settings[b"GER".as_slice()].garrison_template,
        Some((u64::from(u32::MAX) << 32) | 52)
    );
    assert_eq!(saved.state_occupation_laws.len(), 2);
    assert_eq!(
        saved.state_occupation_laws[&(b"GER".to_vec(), b"ENG".to_vec(), 0)].law,
        b"law_b"
    );
    let garrison = &saved.state_garrisons[&(b"GER".to_vec(), 9)];
    assert_eq!(garrison.required_manpower, i32::MIN);
    assert_eq!(garrison.strength_ratio.raw, 9_007_199_254_740_993);
    assert_eq!(
        saved.country_reserve_manpower[b"GER".as_slice()][1].source_country,
        b"FRA"
    );
    assert_eq!(saved.state_resistance[&9].speed.raw, -1);
    assert_eq!(
        saved
            .country_collaboration
            .values()
            .next()
            .unwrap()
            .progress
            .raw,
        -1
    );
}

#[test]
fn malformed_or_duplicate_serialized_scope_is_never_silently_overwritten() {
    for input in [
        "countries={GER={occupation_status={occupation={FRA={occupation_law_list={0=a 0=b}}}}}} states={} provinces={}",
        "countries={GER={occupation_status={occupation={FRA={state_garrison_data={0={}}}}}}} states={} provinces={}",
        "countries={GER={occupation_status={foreign_manpower={value={tag=GER value=0}}}}} states={} provinces={}",
        "countries={} states={1={buildings={a={level=32768 healthy_levels=0 partial_health=0}}}} provinces={}",
        "countries={} states={1={buildings={a={level=1 healthy_levels=0}}}} provinces={}",
        "countries={GER={production={general_lines={unknown={}}}}} states={} provinces={}",
    ] {
        assert!(pack(input).is_err(), "{input}");
    }
}

#[test]
fn shared_manpower_reader_keeps_nonfaction_wrappers_unique() {
    let doc =
        content_reader_parser::parser::parse(b"manpower_pool={} manpower_pool={value={tag=MOD value=4}}")
            .unwrap();
    assert_eq!(
        manpower(&doc.items, b"manpower_pool")
            .unwrap_err()
            .to_string(),
        "duplicate save field manpower_pool",
    );
}

#[test]
fn base_resources_use_initial_declarations_and_preserve_resource_integer_scale() {
    let mut resources = BaseResources::default();
    resources
        .add_state_history(&super::super::history_fixture(b"state={id=1 resources={steel=-2147483648 oil=3 aluminium=10.000 chromium=-1.9 rubber=2147483648.0} history={resources={steel=999}}}"))
        .unwrap();
    assert_eq!(
        resources.rows[&(1, b"steel".to_vec())],
        (0, -214_748_364_800_000)
    );
    assert_eq!(resources.rows[&(1, b"oil".to_vec())], (1, 300_000));
    assert_eq!(resources.rows[&(1, b"aluminium".to_vec())], (2, 1_000_000));
    assert_eq!(resources.rows[&(1, b"chromium".to_vec())], (3, -100_000));
    assert_eq!(
        resources.rows[&(1, b"rubber".to_vec())],
        (4, 214_748_364_700_000)
    );
    assert!(
        resources
            .add_state_history(&super::super::history_fixture(b"state={id=1}"))
            .is_err()
    );
}

#[test]
fn repeated_resource_blocks_append_in_document_order() {
    let mut resources = BaseResources::default();
    resources
        .add_state_history(&super::super::history_fixture(b"state={id=949 resources={aluminium=1} history={resources={steel=999}} resources={} resources={oil=1 steel=-1.9}}"))
        .unwrap();
    assert_eq!(resources.rows.len(), 3);
    assert_eq!(resources.rows[&(949, b"aluminium".to_vec())], (0, 100_000));
    assert_eq!(resources.rows[&(949, b"oil".to_vec())], (1, 100_000));
    assert_eq!(resources.rows[&(949, b"steel".to_vec())], (2, -100_000));
}

#[test]
fn duplicate_resource_tokens_fail_without_replacing_an_earlier_declaration() {
    for input in [
        b"state={id=1 resources={steel=2 steel=7}}".as_slice(),
        b"state={id=1 resources={steel=2} resources={steel=7}}".as_slice(),
    ] {
        let mut resources = BaseResources::default();
        let error = resources
            .add_state_history(&super::super::history_fixture(input))
            .unwrap_err();
        assert!(error.to_string().contains("recorded (state, resource) key"));
        assert_eq!(resources.rows[&(1, b"steel".to_vec())], (0, 200_000));
    }
}

#[test]
fn railway_retains_signed_precision_full_identity_and_repeated_route_order() {
    let fields = railway_fields(
        "rail_way={7 4 7} cancel=yes speed=-0.00001 produced=90071992547.40993 active_factories=-8",
    );
    let row = railway(b"GER", 19, &fields).unwrap();
    assert_eq!(row.production.id, (u64::from(u32::MAX) << 32) | 4713);
    assert_eq!(row.production.cost.raw, -9_007_199_254_740_993);
    assert_eq!(row.production.produced.raw, 9_007_199_254_740_993);
    assert_eq!(row.production.speed.raw, -1);
    assert_eq!(row.production.active_factories, -8);
    assert_eq!(
        (row.production.queue_ordinal, row.production.priority),
        (19, -9)
    );
    assert_eq!(row.production.amount, i32::MIN);
    assert_eq!((row.base_level, row.current_segment), (-3, -1));
    assert_eq!(row.remaining_cost.raw, 1);
    assert!(row.cancelled);
    assert_eq!(row.route, [7, 4, 7]);
}

#[test]
fn railway_applies_only_serializer_proven_omissions_and_refuses_precision_loss() {
    let fields = railway_fields("");
    let row = railway(b"GER", 0, &fields).unwrap();
    assert_eq!(row.production.active_factories, 0);
    assert_eq!(
        (row.production.speed.raw, row.production.produced.raw),
        (0, 0)
    );
    assert!(!row.cancelled);
    assert!(row.route.is_empty());
    for extra in ["speed=0.000001", "rail_way={x=4}", "cancel=2", "amount=2"] {
        assert!(
            railway(b"GER", 0, &railway_fields(extra)).is_err(),
            "{extra}"
        );
    }
    let mut fields = fields;
    fields.retain(|item| item.key.as_deref() != Some(b"remaining_hours"));
    assert!(railway(b"GER", 0, &fields).is_err());
}

#[test]
fn shared_delivery_routes_keep_receiver_keys_and_exact_optional_state_endpoints() {
    let saved = pack(
        r#"countries={AA={resources={delivery_routes={
        BB={type=1 from_state=11 to_state=22 land_path={11 22}}
        CC={type=2 from_state=33 to_state=44 from_port=100 to_port=200 naval_path={7}}
        DD={from_state=0} EE={}
    }}} ZZ={resources={}}} states={} provinces={}"#,
    )
    .unwrap();
    let routes = &saved.country_resources[b"AA".as_slice()].delivery_routes;
    assert_eq!(
        routes[b"BB".as_slice()],
        SavedDeliveryRoute {
            from_state: Some(11),
            to_state: Some(22)
        }
    );
    assert_eq!(
        routes[b"CC".as_slice()],
        SavedDeliveryRoute {
            from_state: Some(33),
            to_state: Some(44)
        }
    );
    assert_eq!(
        routes[b"DD".as_slice()],
        SavedDeliveryRoute {
            from_state: Some(0),
            to_state: None
        }
    );
    assert_eq!(routes[b"EE".as_slice()], SavedDeliveryRoute::default());
    assert!(
        saved.country_resources[b"ZZ".as_slice()]
            .delivery_routes
            .is_empty()
    );
    for malformed in [
        "BB={} BB={}",
        "{}",
        "BB={from_state=bad}",
        "BB={to_state=1 to_state=2}",
    ] {
        let input = "countries={AA={resources={delivery_routes={ENTRIES}}}} states={} provinces={}"
            .replace("ENTRIES", malformed);
        assert!(pack(&input).is_err());
    }
}
