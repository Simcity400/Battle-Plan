use super::*;

fn railway_fields(extra: &str) -> Vec<Item> {
    crate::parser::parse(
        format!(
            "id={{id=4294967295 type=4713}} cost=-90071992547.40993 priority=-9 amount=-2147483648 created_date=1936.1.1.12 base=-3 target=7 current_province=-1 remaining_hours=0.00001 {extra}"
        )
        .as_bytes(),
    )
    .unwrap()
    .items
}

fn pack(input: &str) -> io::Result<Pack> {
    parse(&crate::parser::parse(input.as_bytes())?.items)
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
        2
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
    // A country's offsite buildings keep their tag; a country without a
    // block has none.
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
fn base_resources_use_initial_declarations_and_preserve_native_integer_scale() {
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
