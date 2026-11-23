use super::*;

#[test]
fn faction_name_preserves_omitted_and_emitted_empty_keys() {
    for (field, expected) in [
        ("", None),
        ("name=\"\"", Some(b"".as_slice())),
        ("name=Custom", Some(b"Custom".as_slice())),
    ] {
        let doc = parser::parse(format!("id={{id=1 type=8}} {field}").as_bytes()).unwrap();
        let faction = parse_faction(&doc.items).unwrap();
        assert_eq!(faction.name.as_deref(), expected);
    }
}

#[test]
fn game_score_saved_rosters_keep_writer_order_and_empty_countries() {
    let saved = parse(br#"HOI4txt
date=1930.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
countries={AAA={units={
division={id={id=9 type=5} logical_country=AAA strength=0 organisation=0}
division={id={id=2 type=5} logical_country=AAA strength=0 organisation=0}
fleet={id={id=8 type=7}
task_force={id={id=9 type=6} logical_country=AAA mission={mission=0}}
task_force={id={id=2 type=6} logical_country=AAA mission={mission=0}}}
fleet={id={id=1 type=7}}}} BBB={}}
states={} provinces={}
"#).unwrap();
    let handle = |id: u64, kind| (id << 32) | kind;
    assert_eq!(
        saved.counter_inputs.division_orders[b"AAA".as_slice()],
        [handle(9, 5), handle(2, 5)]
    );
    assert_eq!(
        saved.counter_inputs.fleet_orders[b"AAA".as_slice()],
        [handle(8, 7), handle(1, 7)]
    );
    assert_eq!(
        saved.counter_inputs.fleets[&handle(8, 7)].task_forces,
        [handle(9, 6), handle(2, 6)]
    );
    assert!(
        saved.counter_inputs.fleets[&handle(1, 7)]
            .task_forces
            .is_empty()
    );
    assert!(saved.counter_inputs.division_orders[b"BBB".as_slice()].is_empty());
    assert!(saved.counter_inputs.fleet_orders[b"BBB".as_slice()].is_empty());
    assert_eq!(saved.divisions.len(), 2);
    assert_eq!(saved.task_forces.len(), 2);
}

#[test]
fn state_border_flag_preserves_absence_and_strict_boolean_witness() {
    let header = "HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 countries={} provinces={} ";
    for (token, expected) in [("yes", Some(true)), ("no", Some(false))] {
        let source = format!("{header}states={{1={{is_border_conflict={token}}}}}");
        assert_eq!(
            parse(source.as_bytes()).unwrap().states[&1].is_border_conflict,
            expected
        );
    }
    assert_eq!(
        parse(format!("{header}states={{1={{}}}}").as_bytes())
            .unwrap()
            .states[&1]
            .is_border_conflict,
        None
    );
    assert!(
        parse(format!("{header}states={{1={{is_border_conflict=maybe}}}}").as_bytes()).is_err()
    );
}

#[test]
fn faction_advisor_slots_keep_slot_order_and_empty_slots() {
    let faction = content_reader_parser::parser::parse(
        br#"id={ id=1 type=88 } upgrades={ slots={
            { modifier={ name="fixture_modifier" } }
            { country="ZZZ" advisor=fixture_advisor modifier={ } }
        } doctrine={ } }"#,
    )
    .unwrap();
    assert_eq!(
        parse_faction(&faction.items).unwrap().advisor_slots,
        [None, Some(b"fixture_advisor".to_vec())]
    );
    let bare = content_reader_parser::parser::parse(br#"id={ id=1 type=88 }"#).unwrap();
    assert!(parse_faction(&bare.items).unwrap().advisor_slots.is_empty());
    for broken in [
        br#"id={ id=1 type=88 } upgrades={ slots={ } }"#.as_slice(),
        br#"id={ id=1 type=88 } upgrades={ slots={ slot={ advisor=x } } }"#,
    ] {
        let broken = content_reader_parser::parser::parse(broken).unwrap();
        assert!(parse_faction(&broken.items).is_err());
    }
}

#[test]
fn civil_war_targets_are_every_written_tag_in_order() {
    let country = content_reader_parser::parser::parse(
        b"civil_war_target=\"ITA\" major=yes civil_war_target=\"D01\" cores={ 5 }",
    )
    .unwrap();
    assert_eq!(
        parse_country(b"GER", &country.items)
            .unwrap()
            .civil_war_targets,
        [b"ITA".to_vec(), b"D01".to_vec()]
    );
    assert!(
        parse_country(b"GER", &[])
            .unwrap()
            .civil_war_targets
            .is_empty()
    );
    let broken = content_reader_parser::parser::parse(b"civil_war_target={ ITA }").unwrap();
    assert!(parse_country(b"GER", &broken.items).is_err());
}

#[test]
fn country_cores_are_the_saved_state_ids_in_order() {
    let country =
        content_reader_parser::parser::parse(b"major=yes cores={ 5 42 50 978 } num_ships=66").unwrap();
    assert!(parse_country(b"GER", &country.items).unwrap().major);
    assert!(!parse_country(b"GER", &[]).unwrap().major);
    assert_eq!(
        parse_country(b"GER", &country.items).unwrap().cores,
        [5, 42, 50, 978]
    );
    assert!(parse_country(b"GER", &[]).unwrap().cores.is_empty());
}

#[test]
fn country_original_tag_preserves_conditional_presence_and_exact_scalar_bytes() {
    let parse = |text: &[u8]| {
        let doc = content_reader_parser::parser::parse(text).unwrap();
        parse_country(b"OWN", &doc.items)
    };
    assert_eq!(parse(b"").unwrap().original_tag, None);
    assert_eq!(
        parse(b"original_tag=OWN").unwrap().original_tag,
        Some(b"OWN".to_vec())
    );
    assert_eq!(
        parse(b"original_tag=OTHER").unwrap().original_tag,
        Some(b"OTHER".to_vec())
    );
    assert_eq!(
        parse(b"original_tag=\"\"").unwrap().original_tag,
        Some(Vec::new())
    );
    for broken in [
        b"original_tag=A original_tag=B".as_slice(),
        b"original_tag={}",
    ] {
        assert!(parse(broken).is_err());
    }
}

#[test]
fn map_input_fields_preserve_literal_bytes_and_writer_omissions() {
    let country = content_reader_parser::parser::parse(b"cosmetic_tag=BRM_british").unwrap();
    assert_eq!(
        parse_country(b"BRM", &country.items).unwrap().cosmetic_tag,
        Some(b"BRM_british".to_vec())
    );
    assert_eq!(parse_country(b"GER", &[]).unwrap().cosmetic_tag, None);
    let empty = content_reader_parser::parser::parse(b"cosmetic_tag=\"\"").unwrap();
    assert_eq!(
        parse_country(b"GER", &empty.items).unwrap().cosmetic_tag,
        Some(Vec::new())
    );
    let division = content_reader_parser::parser::parse(b"id={id=1 type=51} logical_country=GER strength=1 organisation=1 leader=yes fake_intel_division_template_id={id=4294967295 type=50}").unwrap();
    let row = parse_division(b"GER", &division.items).unwrap();
    assert!(row.separate_counter);
    assert_eq!(
        row.alternate_template,
        Some((u64::from(u32::MAX) << 32) | 50)
    );
    assert_eq!(row.template, None);
    let ship = content_reader_parser::parser::parse(b"id={id=1 type=51} strength=1 organisation=1 equipment={equipment={id={id=3 type=70} amount=-1} equipment={id={id=4 type=70} amount=0}}").unwrap();
    let row = parse_ship(9, &ship.items).unwrap();
    let pool = row.equipment.unwrap();
    assert_eq!(pool.equipment.len(), 2);
    assert_eq!(pool.equipment[0].amount.raw, -100_000);
    assert_eq!(pool.equipment[1].variant, (4_u64 << 32) | 70);
}

#[test]
fn regional_convoys_keep_only_written_efficiencies() {
    let read = |navy: &str| {
        let doc = content_reader_parser::parser::parse(navy.as_bytes()).unwrap();
        parse_country(b"GER", &doc.items).map(|country| country.regional_convoy_efficiency)
    };
    let efficiency = read(
        "strategic_navy={regional_convoys={ 305 { index=173 data={ required_convoys=5 } } { index=206 data={ required_convoys=1 efficiency=0.4 } } }}",
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        efficiency
            .iter()
            .map(|(region, value)| (*region, value.raw))
            .collect::<Vec<_>>(),
        [(206, 40_000)]
    );
    assert_eq!(read("").unwrap(), None);
    assert_eq!(read("strategic_navy={}").unwrap(), Some(Default::default()));
    assert!(read("strategic_navy={regional_convoys={ { index=1 data={} } }}").is_err());
    assert!(
        read("strategic_navy={regional_convoys={ 1 { index=1 data={} } { index=2 data={} } }}")
            .is_err()
    );
}

#[test]
fn naval_region_access_reads_flat_pairs_and_refuses_a_broken_list() {
    let parse = |text: &str| {
        let doc = content_reader_parser::parser::parse(text.as_bytes()).unwrap();
        parse_country(b"GER", &doc.items).map(|country| country.naval_region_access)
    };
    assert_eq!(parse("").unwrap(), None);
    assert_eq!(
        parse("strategic_navy={ per_region_access={ } }").unwrap(),
        Some(Vec::new())
    );
    assert_eq!(
        parse("strategic_navy={ per_region_access={\n18 1 43 2\n} }").unwrap(),
        Some(vec![(18, 1), (43, 2)])
    );
    for broken in [
        "strategic_navy={ per_region_access={ 18 } }",
        "strategic_navy={ per_region_access={ 18=1 } }",
        "strategic_navy={ per_region_access={ 18 300 } }",
    ] {
        assert!(parse(broken).is_err(), "{broken}");
    }
}

#[test]
fn map_input_fields_refuse_ambiguous_or_malformed_saved_operands() {
    for fields in ["cosmetic_tag=x cosmetic_tag=y", "cosmetic_tag={}"] {
        let doc = content_reader_parser::parser::parse(fields.as_bytes()).unwrap();
        assert!(parse_country(b"GER", &doc.items).is_err());
    }
    for field in [
        "leader=2",
        "leader=yes leader=no",
        "fake_intel_division_template_id={id=0 type=0}",
        "fake_intel_division_template_id={id=1}",
        "fake_intel_division_template_id={id=1 type=50} fake_intel_division_template_id={id=2 type=50}",
    ] {
        let source =
            format!("id={{id=1 type=51}} logical_country=GER strength=1 organisation=1 {field}");
        let doc = content_reader_parser::parser::parse(source.as_bytes()).unwrap();
        assert!(parse_division(b"GER", &doc.items).is_err(), "{field}");
    }
    for field in [
        "equipment={} equipment={}",
        "equipment={equipment={id={id=1 type=70}}}",
        "equipment={equipment={id={id=1 type=70} amount=0.000001}}",
    ] {
        let source = format!("id={{id=1 type=51}} strength=1 organisation=1 {field}");
        let doc = content_reader_parser::parser::parse(source.as_bytes()).unwrap();
        assert!(parse_ship(9, &doc.items).is_err(), "{field}");
    }
}

#[test]
fn engine_calendar_has_no_leap_day_and_saves_use_hour_one_through_twenty_four() {
    assert_eq!(
        GameDate::parse(b"1936.1.2.17")
            .unwrap()
            .engine_hour()
            .unwrap(),
        60_759_400
    );
    assert_eq!(
        GameDate::parse(b"1936.9.29.19")
            .unwrap()
            .engine_hour()
            .unwrap(),
        60_765_882
    );
    assert_eq!(
        GameDate::parse(b"1936.1.1.24")
            .unwrap()
            .engine_hour()
            .unwrap()
            + 1,
        GameDate::parse(b"1936.1.2.1")
            .unwrap()
            .engine_hour()
            .unwrap()
    );
    for bad in [
        "1936.2.29.1",
        "1936.1.1.0",
        "1936.1.1.25",
        "1936.13.1.1",
        "1936.1.0.1",
        "1936.1.1",
    ] {
        assert!(GameDate::parse(bad.as_bytes()).is_err(), "{bad}");
    }
}

#[test]
fn stability_terms_are_five_decimal_numbers() {
    let save = parse(br#"HOI4txt
date="1936.1.2.17" game_unique_id="campaign" version="build" game_unique_seed=-42 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
countries={ GER={ refresh=522 propaganda_stability_penalty=-0.10000 coastal_protection_ratio=0.025 being_bombed_support_penalty=-0.00003 heroes_dying_war_support_penalty=-1.00000 convoy_raiding_war_support_penalty=-0.00015 propaganda_war_support_penalty=-0.00021 } ITA={} }
states={ 1={owner="GER"} } provinces={ 2={controller="GER"} }
"#)
    .unwrap();
    let terms = &save.countries[b"GER".as_slice()].stability_terms;
    let raw = |value: &Option<Decimal>| value.as_ref().map(|value| value.raw);
    assert_eq!(raw(&terms.propaganda_stability_penalty), Some(-10_000));
    assert_eq!(raw(&terms.coastal_protection_ratio), Some(2_500));
    assert_eq!(raw(&terms.being_bombed_support_penalty), Some(-3));
    assert_eq!(raw(&terms.heroes_dying_war_support_penalty), Some(-100_000));
    assert_eq!(raw(&terms.convoy_raiding_war_support_penalty), Some(-15));
    assert_eq!(raw(&terms.propaganda_war_support_penalty), Some(-21));
    assert_eq!(
        save.countries[b"ITA".as_slice()].stability_terms,
        StabilityTerms::default()
    );
    assert_eq!(save.countries[b"GER".as_slice()].refresh, Some(522));
    assert_eq!(save.countries[b"ITA".as_slice()].refresh, None);
}

#[test]
fn stored_manpower_operands_preserve_signed_values_and_pool_omission() {
    let save = parse(br#"HOI4txt date="1936.1.1.1" game_unique_id=x version=b game_unique_seed=1 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
        countries={AAA={manpower={current=-2147483648}} BBB={}}
        states={1={owner=AAA manpower_pool={available=2147483647 locked=-17 total=1}} 2={owner=BBB}} provinces={}"#).unwrap();
    assert_eq!(
        save.countries[b"AAA".as_slice()].manpower_residual,
        i32::MIN
    );
    assert_eq!(save.countries[b"BBB".as_slice()].manpower_residual, 0);
    assert_eq!(save.states[&1].manpower_available, Some(i32::MAX));
    assert_eq!(save.states[&1].manpower_locked, Some(-17));
    assert_eq!(save.states[&2].manpower_available, None);
    assert_eq!(save.states[&1].population, Some(Some(1)));
    assert_eq!(save.states[&2].population, None);
}

#[test]
fn extraction_does_not_confuse_nested_similarly_named_fields_or_invent_missing_values() {
    let save = parse(br#"HOI4txt
date="1936.1.2.17" game_unique_id="campaign" version="build" game_unique_seed=-42 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
countries={ GER={ command_power=78.1411 politics={ political_power=582.128 } stability=0.7 war_support=0.3
flags={ stability={value=99 date=1936.1.2.17} } unrelated={stability=99}
convoys={ equipment={ equipment={ id={ id=1 type=70 } amount=200 } } } } D01={} }
states={ 1={owner="GER"} } provinces={ 2={controller="D01"} 3={buildings={}} }
"#).unwrap();
    assert_eq!(save.seed, -42);
    assert_eq!(
        save.countries[b"GER".as_slice()]
            .stability
            .as_ref()
            .unwrap()
            .raw,
        70_000
    );
    assert_eq!(
        save.countries[b"GER".as_slice()]
            .political_power
            .as_ref()
            .unwrap()
            .raw,
        58_212_800
    );
    assert_eq!(
        save.countries[b"GER".as_slice()]
            .convoy_equipment
            .as_ref()
            .unwrap()[0]
            .amount
            .raw,
        20_000_000
    );
    assert_eq!(save.countries[b"D01".as_slice()].command_power, None);
    assert_eq!(save.provinces[&3].controller, None);
    assert_eq!(
        save.intel.countries[b"GER".as_slice()]
            .flags
            .as_ref()
            .unwrap()[0]
            .value,
        99
    );
}

#[test]
fn strategic_locations_read_from_states_and_provinces() {
    let text = |state: &str, province: &str| {
        format!(
            "HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 countries={{}} states={{4={{{state}}} 5={{}}}} provinces={{1={{{province}}} 2={{}}}}"
        )
    };
    let saved = parse(
        text(
            "strategic_region_data={ { natural_harbor 1 } { favorable_approach 2 } }",
            "strategic_province_location={ defensive_chokepoint natural_harbor }",
        )
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(
        saved.states[&4].strategic_locations,
        [
            (b"natural_harbor".to_vec(), 1),
            (b"favorable_approach".to_vec(), 2)
        ]
    );
    assert!(saved.states[&5].strategic_locations.is_empty());
    assert_eq!(
        saved.provinces[&1].strategic_locations,
        [b"defensive_chokepoint".to_vec(), b"natural_harbor".to_vec()]
    );
    assert!(saved.provinces[&2].strategic_locations.is_empty());
    let saved = parse(text(r#"contested_owners={ "FIN" "SOV" }"#, "").as_bytes()).unwrap();
    assert_eq!(
        saved.states[&4].contested_owners,
        [b"FIN".to_vec(), b"SOV".to_vec()]
    );
    assert!(saved.states[&5].contested_owners.is_empty());
    for (state, province) in [
        ("strategic_region_data={ { a 1 b } }", ""),
        ("strategic_region_data={ { a=1 } }", ""),
        ("strategic_region_data={ x={ a 1 } }", ""),
        ("", "strategic_province_location={ a=b }"),
        ("contested_owners={ owner=FIN }", ""),
        ("contested_owners=FIN", ""),
    ] {
        assert!(
            parse(text(state, province).as_bytes()).is_err(),
            "{state}{province}"
        );
    }
}

#[test]
fn province_victory_points_keep_signed_integers_and_literal_omission() {
    let text = |value: &str| {
        format!(
            "HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 countries={{}} states={{}} provinces={{1={{victory_points={value}}} 2={{}}}}"
        )
    };
    for value in [i32::MIN, -1, 0, i32::MAX] {
        let saved = parse(text(&value.to_string()).as_bytes()).unwrap();
        assert_eq!(saved.provinces[&1].victory_points, Some(value));
        assert_eq!(saved.provinces[&2].victory_points, None);
    }
    for invalid in ["2147483648", "-2147483649", "1.0", "1.5"] {
        assert!(parse(text(invalid).as_bytes()).is_err());
    }
}

#[test]
fn province_override_names_are_literal_optional_bytes_and_duplicates_fail() {
    let text = |names: &str| {
        format!(
            "HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 countries={{}} states={{}} provinces={{{names}}}"
        )
    };
    let saved = parse(text(r#"1={name="Gdansk"} 2={} 3={name=""}"#).as_bytes()).unwrap();
    assert_eq!(
        saved.provinces[&1].name.as_deref(),
        Some(b"Gdansk".as_slice())
    );
    assert_eq!(saved.provinces[&2].name, None);
    assert_eq!(saved.provinces[&3].name.as_deref(), Some(b"".as_slice()));
    assert!(parse(text(r#"1={name="A" name="B"}"#).as_bytes()).is_err());
}

#[test]
fn division_template_parser_preserves_coordinates_and_rejects_duplicate_cells() {
    let parse = |cells: &str| {
        let document = parser::parse(format!("division_templates={{division_template={{id={{id=4294967295 type=52}} country=GER name=Infantry template_counter=-1 ingame_set_template_counter=yes regiments={{{cells}}}}}}}").as_bytes()).unwrap();
        parse_division_templates(&document.items)
    };
    let rows = parse("infantry={x=0 y=1} artillery={x=1 y=0}").unwrap();
    let row = &rows[&0xffff_ffff_0000_0034];
    assert_eq!(row.name, b"Infantry");
    assert_eq!(row.custom_icon, -1);
    assert!(row.custom_icon_nato);
    assert_eq!(row.composition[&(0, 0, 1)], b"infantry");
    assert_eq!(row.names_group, None);
    let document = parser::parse(br#"division_templates={division_template={id={id=4294967295 type=52} country=GER template_counter=-1 ingame_set_template_counter=no division_names_group="SOV_INF_01"}}"#).unwrap();
    let rows = parse_division_templates(&document.items).unwrap();
    assert_eq!(
        rows[&0xffff_ffff_0000_0034].names_group.as_deref(),
        Some(b"SOV_INF_01".as_slice())
    );
    assert!(parse("infantry={x=0 y=1} artillery={x=0 y=1}").is_err());
    assert!(parse("infantry={x=-1 y=1}").is_err());
}

#[test]
fn binary_malformed_and_ambiguous_saves_are_refused() {
    assert!(
        parse(b"HOI4bin\0")
            .unwrap_err()
            .to_string()
            .contains("save_as_binary=no")
    );
    assert!(parse(b"HOI4txtbad").is_err());
    let header = b"HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 countries={} states={} provinces={} ";
    let mut duplicate = header.to_vec();
    duplicate.extend_from_slice(b"date=1936.1.2.1");
    assert!(parse(&duplicate).is_err());
    assert_eq!(parse(header).unwrap().player, None);
    let mut player = b"HOI4txt\nplayer=\"GER\" ".to_vec();
    player.extend_from_slice(&header[8..]);
    assert_eq!(parse(&player).unwrap().player.as_deref(), Some(&b"GER"[..]));
    player.extend_from_slice(b"player=\"ITA\"");
    assert!(parse(&player).is_err());
}

#[test]
fn state_demilitarized_preserves_yes_no_and_missing_and_rejects_invalid_or_duplicate() {
    let document = |states: &str| {
        format!(
            "HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=13 countries={{}} provinces={{}} states={{{states}}}"
        )
    };
    let saved =
        parse(document("42={demilitarized=yes} 51={demilitarized=no} 978={}").as_bytes()).unwrap();
    assert_eq!(saved.states[&42].demilitarized, Some(true));
    assert_eq!(saved.states[&51].demilitarized, Some(false));
    assert_eq!(saved.states[&978].demilitarized, None);
    for value in ["1", "true", "{}", "yes demilitarized=no"] {
        assert!(parse(document(&format!("42={{demilitarized={value}}}")).as_bytes()).is_err());
    }
}

#[test]
fn state_control_has_its_own_explicit_and_owner_equivalent_save_rules() {
    let save = parse(
        br#"HOI4txt
date=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
countries={ GER={} D01={} }
states={1={owner=GER controller=D01 name="Renamed"} 2={owner=GER}}
provinces={3={controller=D01}}
"#,
    )
    .unwrap();
    assert_eq!(save.states[&1].name.as_deref(), Some(&b"Renamed"[..]));
    assert_eq!(save.states[&2].name, None);
    let map = MapMembership {
        provinces: BTreeMap::from([(0, None), (1, Some(1)), (2, Some(2)), (3, Some(2))]),
    };
    let map = PartialMapMembership::from(map);
    let states = save.known_state_control(&map).unwrap();
    assert_eq!(states.len(), 2);
    assert_eq!(states[0].owner, b"GER");
    assert_eq!(states[0].controller, b"D01");
    assert!(states[0].controller_explicit);
    assert_eq!(states[1].controller, b"GER");
    assert!(!states[1].controller_explicit);
    let provinces = save.province_control(&map).unwrap();
    assert_eq!(provinces[0].controller.as_deref(), Some(b"GER".as_slice()));
    assert_eq!(provinces[2].controller.as_deref(), Some(b"D01".as_slice()));
}

#[test]
fn state_comparison_refuses_missing_fields_unknown_tags_and_content_mismatch() {
    let save = parse(
        br#"HOI4txt
date=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
countries={ GER={} } states={1={owner=GER}} provinces={}
"#,
    )
    .unwrap();
    let map = MapMembership {
        provinces: BTreeMap::from([(1, Some(1))]),
    };
    let map = PartialMapMembership::from(map);
    let mut missing = save.clone();
    missing.states.clear();
    assert!(
        missing
            .known_state_control(&map)
            .unwrap_err()
            .to_string()
            .contains("lacks state 1")
    );
    let mut missing = save.clone();
    missing.states.get_mut(&1).unwrap().owner = None;
    assert!(
        missing
            .known_state_control(&map)
            .unwrap_err()
            .to_string()
            .contains("lacks its owner")
    );
    for owner in [false, true] {
        let mut unknown = save.clone();
        let state = unknown.states.get_mut(&1).unwrap();
        if owner {
            state.owner = Some(b"UNKNOWN".to_vec());
        } else {
            state.controller = Some(b"UNKNOWN".to_vec());
        }
        assert!(
            unknown
                .known_state_control(&map)
                .unwrap_err()
                .to_string()
                .contains("unknown saved country")
        );
    }
    let mut unexpected = save;
    unexpected.states.insert(
        2,
        State {
            is_border_conflict: None,
            id: 2,
            owner: Some(b"GER".to_vec()),
            controller: None,
            demilitarized: None,
            name: None,
            manpower_available: None,
            manpower_locked: None,
            population: None,
            category: None,
            strategic_locations: Vec::new(),
            contested_owners: Vec::new(),
        },
    );
    assert!(
        unexpected
            .known_state_control(&map)
            .unwrap_err()
            .to_string()
            .contains("no membership")
    );
}

#[test]
fn country_intel_triples_keep_saved_order_signs_and_nonpositive_levels() {
    let save = division_save(
        r#"
division={
 id={id=1 type=4919} logical_country=SOV strength=0 organisation=0
 country_intel={ 2 -2147483648 4 -1 0 -1 }
}
division={ id={id=2 type=4919} logical_country=SOV strength=0 organisation=0 }
"#,
    )
    .unwrap();
    let intel = &save.divisions[&((1_u64 << 32) | 4919)].country_intel;
    assert_eq!(
        intel,
        &[
            IntelEntry {
                viewer: 2,
                timer_low: i32::MIN,
                level: 4
            },
            IntelEntry {
                viewer: -1,
                timer_low: 0,
                level: -1
            },
        ]
    );
    assert!(
        save.divisions[&((2_u64 << 32) | 4919)]
            .country_intel
            .is_empty()
    );
}

#[test]
fn a_partial_or_keyed_country_intel_list_is_refused() {
    for list in ["{ 1 2 }", "{ }", "{ viewer=1 2 3 }"] {
        let save = division_save(&format!(
            "division={{ id={{id=1 type=4919}} logical_country=SOV strength=0 organisation=0 country_intel={list} }}"
        ));
        assert!(save.is_err(), "{list}");
    }
}

#[test]
fn volunteer_permission_blocks_keep_save_order_and_repeated_tags() {
    let save = parse(
        b"HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 countries={ SOV={ received_air_volunteer_permission={ tag=GER count=3 } received_air_volunteer_permission={ tag=GER count=-1 } } GER={} } states={} provinces={}",
    )
    .unwrap();
    let hosts = &save.countries[b"SOV".as_slice()].volunteer_permissions;
    assert_eq!(
        hosts,
        &[
            VolunteerPermission {
                tag: b"GER".to_vec(),
                count: 3
            },
            VolunteerPermission {
                tag: b"GER".to_vec(),
                count: -1
            },
        ]
    );
    assert!(
        save.countries[b"GER".as_slice()]
            .volunteer_permissions
            .is_empty()
    );
    assert!(
        parse(
            b"HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 countries={ SOV={ received_air_volunteer_permission={ tag=GER } } } states={} provinces={}",
        )
        .is_err()
    );
}

fn division_save(units: &str) -> io::Result<Save> {
    parse(format!(
        "HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 countries={{ SOV={{ units={{ {units} }} }} D01={{}} GER={{}} }} states={{}} provinces={{}}"
    ).as_bytes())
}

#[test]
fn unit_saved_target_and_full_path_preserve_order_and_omission() {
    for kind in ["division", "railway_gun"] {
        let parse_unit = |fields: &str| {
            division_save(&format!(
                "{kind}={{id={{id=1 type=51}} logical_country=SOV strength=0 organisation=0 {fields}}}"
            ))
        };
        for (fields, target, route) in [
            ("support_attack=7 full_path={9 8 9}", Some(7), vec![9, 8, 9]),
            ("", None, vec![]),
        ] {
            let save = parse_unit(fields).unwrap();
            let id = (1_u64 << 32) | 51;
            let (actual_target, actual_route) = if kind == "division" {
                let unit = &save.divisions[&id].presentation;
                (unit.support_attack, &unit.full_path)
            } else {
                let unit = &save.railway_guns[&id];
                (unit.support_attack, &unit.full_path)
            };
            assert_eq!(actual_target, target);
            assert_eq!(*actual_route, route);
        }
        for fields in [
            "support_attack=no",
            "support_attack={}",
            "support_attack=1 support_attack=2",
            "full_path={x=9}",
            "full_path=no",
            "commandlist={ {} }",
            "commandlist=yes",
        ] {
            assert!(parse_unit(fields).is_err(), "{kind}: {fields}");
        }
        for (fields, queued) in [
            ("commandlist={ move_order={ province=7 } attack_order={} }", 2),
            ("", 0),
        ] {
            let save = parse_unit(fields).unwrap();
            let id = (1_u64 << 32) | 51;
            let actual = if kind == "division" {
                save.divisions[&id].presentation.queued_actions
            } else {
                save.railway_guns[&id].queued_actions
            };
            assert_eq!(actual, queued, "{kind}: {fields}");
        }
    }
}

#[test]
fn railway_gun_extract_preserves_typed_identity_saved_route_and_combat_order() {
    let save = division_save(
        r#"
railway_gun={
 id={id=1 type=74} id={type=74 id=1}
 logical_country=D01 expeditionary_owner=GER location=3218
 path={4466 4689 4912 5127 5244 4119} previous=3425
 movement_progress=13.02667 strategic_redeployment=yes
 strength=90071992547.40993
 combat={{id=21029 type=62} {id=21037 type=62}}
 army={id=3378 type=53} definition=railway_gun
 country_intel={ 2 -2147483648 4 -1 0 -1 }
}
railway_gun={id={id=2 type=4713} logical_country=SOV strength=0}
"#,
    )
    .unwrap();
    let id = (1_u64 << 32) | 74;
    let gun = &save.railway_guns[&id];
    assert_eq!(save.railway_guns.len(), 2);
    assert_eq!(
        (gun.id, gun.roster_owner.as_slice()),
        (id, b"SOV".as_slice())
    );
    assert_eq!(gun.logical_owner, b"D01");
    assert_eq!(gun.expeditionary_owner.as_deref(), Some(b"GER".as_slice()));
    assert_eq!(gun.province, Some(3218));
    assert_eq!(gun.path, [4466, 4689, 4912, 5127, 5244, 4119]);
    assert!(gun.strategic_redeployment);
    assert_eq!(gun.strength.raw, 9_007_199_254_740_993);
    assert_eq!(
        gun.combats,
        [(21_029_u64 << 32) | 62, (21_037_u64 << 32) | 62]
    );
    assert_eq!(
        gun.country_intel,
        [
            IntelEntry {
                viewer: 2,
                timer_low: i32::MIN,
                level: 4,
            },
            IntelEntry {
                viewer: -1,
                timer_low: 0,
                level: -1,
            },
        ]
    );
    let sparse = &save.railway_guns[&((2_u64 << 32) | 4713)];
    assert_eq!(sparse.province, None);
    assert!(sparse.path.is_empty() && sparse.combats.is_empty());
}

#[test]
fn railway_gun_parser_refuses_invalid_ids_location_and_combat_shape() {
    let valid = "railway_gun={id={id=1 type=74} logical_country=SOV strength=0}";
    for bad in [
        "railway_gun={logical_country=SOV strength=0}",
        "railway_gun={id={id=0 type=0} logical_country=SOV strength=0}",
        "railway_gun={id={id=1 type=74} id={id=2 type=74} logical_country=SOV strength=0}",
        "railway_gun={id={id=1 type=74} logical_country=SOV location=0 strength=0}",
        "railway_gun={id={id=1 type=74} logical_country=SOV strength=0 combat={combat={id=1 type=62}}}",
        "railway_gun={id={id=1 type=74} logical_country=SOV strength=0 combat={{id=0 type=0}}}",
        "railway_gun={id={id=1 type=74} logical_country=SOV}",
    ] {
        assert!(division_save(bad).is_err(), "{bad}");
    }
    assert!(division_save(&format!("{valid} {valid}")).is_err());
}

#[test]
fn division_extract_preserves_full_handles_ownership_and_saved_fixed_values() {
    let save = division_save(
        r#"
division={
 id={id=2147483649 type=4919} id={type=4919 id=2147483649}
 logical_country=D01 expeditionary_owner=GER location=6906
 division_template_id={id=4294967295 type=6000}
 strength=90071992547.40993 organisation=-0.00001
 equipment={ equipment={id={id=1 type=70} amount=100} }
}
division={
 id={id=2147483649 type=4920} logical_country=SOV
 strength=0 organisation=0
}
"#,
    )
    .unwrap();
    let id = (2_147_483_649_u64 << 32) | 4919;
    let division = &save.divisions[&id];
    assert_eq!(save.divisions.len(), 2);
    assert_eq!(division.id, id);
    assert_eq!(division.roster_owner, b"SOV");
    assert_eq!(division.logical_owner, b"D01");
    assert_eq!(
        division.expeditionary_owner.as_deref(),
        Some(b"GER".as_slice())
    );
    assert_eq!(division.province, Some(6906));
    assert_eq!(division.template, Some((u64::from(u32::MAX) << 32) | 6000));
    assert_eq!(division.hit_points.raw, 9_007_199_254_740_993);
    assert_eq!(division.organisation.raw, -1);
    let without_optional = &save.divisions[&((2_147_483_649_u64 << 32) | 4920)];
    assert_eq!(without_optional.province, None);
    assert_eq!(without_optional.template, None);
    assert_eq!(without_optional.expeditionary_owner, None);
}

#[test]
fn division_missing_required_values_and_conflicting_duplicates_are_refused() {
    let valid = "division={id={id=1 type=51} logical_country=SOV strength=0 organisation=0}";
    for fragment in [
        valid.replace("id={id=1 type=51}", ""),
        valid.replace("type=51", ""),
        valid.replace("id=1", ""),
        valid.replace("logical_country=SOV", ""),
        valid.replace("strength=0", ""),
        valid.replace("organisation=0", ""),
        valid.replace("strength=0", "strength=0.000001"),
        valid.replace("type=51", "type=4294967296"),
        valid.replace(
            "logical_country=SOV",
            "logical_country=SOV logical_country=D01",
        ),
        valid.replace("organisation=0", "organisation=0 organisation=0"),
        valid.replace(
            "logical_country=SOV",
            "id={id=2 type=51} logical_country=SOV",
        ),
        format!("{valid} {valid}"),
    ] {
        assert!(division_save(&fragment).is_err(), "{fragment}");
    }
}

#[test]
fn division_presentation_parser_retains_route_order_precision_and_positive_omission() {
    let source = "division={id={id=1 type=51} logical_country=SOV strength=1 organisation=2 previous=7 path={9 8 9} movement_progress=90071992547.40993 dig_in=0.00001 dig_in_cap=8.8 strategic_redeployment=yes}";
    let saved = division_save(source).unwrap();
    let p = &saved.divisions[&((1_u64 << 32) | 51)].presentation;
    assert_eq!(p.path, [9, 8, 9]);
    assert_eq!(p.previous, Some(7));
    assert_eq!(
        p.movement_progress.as_ref().unwrap().raw,
        9_007_199_254_740_993
    );
    assert_eq!(p.dig_in.as_ref().unwrap().raw, 1);
    assert_eq!(p.dig_in_cap.as_ref().unwrap().raw, 880_000);
    assert!(p.strategic_redeployment);
    let acclimatized = division_save(
        "division={id={id=1 type=51} logical_country=SOV strength=1 organisation=2 acclimatization={hot_climate=1.5 actively_gaining=hot_climate cold_climate=-2 hot_climate=3 other_loss=0.1}}",
    )
    .unwrap();
    let acclimatization = acclimatized.divisions[&((1_u64 << 32) | 51)]
        .presentation
        .acclimatization
        .as_ref()
        .unwrap();
    assert_eq!(
        acclimatization
            .entries
            .iter()
            .map(|(name, weight)| (name.as_slice(), weight.raw))
            .collect::<Vec<_>>(),
        [
            (b"hot_climate".as_slice(), 150_000),
            (b"cold_climate", -200_000),
            (b"hot_climate", 300_000)
        ]
    );
    assert_eq!(acclimatization.maximum, None);
    let absent =
        division_save("division={id={id=1 type=51} logical_country=SOV strength=1 organisation=2}")
            .unwrap();
    assert_eq!(
        absent.divisions[&((1_u64 << 32) | 51)].presentation,
        DivisionPresentation::default()
    );
    for bad in [
        source.replace("path={9 8 9}", "path={x=9}"),
        source.replace("dig_in=0.00001", "dig_in=0.000001"),
        source.replace("strategic_redeployment=yes", "strategic_redeployment=maybe"),
        source.replace("previous=7", "previous=7 previous=7"),
    ] {
        assert!(division_save(&bad).is_err(), "{bad}");
    }
}

#[test]
fn division_name_block_keeps_every_conditional_key_distinct_from_absence() {
    let unit = |name: &str| {
        let source = format!(
            "division={{id={{id=1 type=51}} logical_country=SOV strength=1 organisation=2 {name}}}"
        );
        division_save(&source).map(|save| {
            save.divisions[&((1_u64 << 32) | 51)]
                .presentation
                .division_name
                .clone()
        })
    };
    assert_eq!(unit("").unwrap(), None);
    assert_eq!(
        unit("division_name={ type=0 }").unwrap(),
        Some(DivisionName::default())
    );
    assert_eq!(
        unit(r#"division_name={ type=0 name_order=-1 is_name_ordered=no override="Guards" override_set_programmatically=yes }"#)
            .unwrap(),
        Some(DivisionName {
            kind: 0,
            order: Some(-1),
            ordered: Some(false),
            custom: Some(b"Guards".to_vec()),
            programmatic: Some(true),
        })
    );
    for bad in [
        "division_name={ name_order=4 }",
        "division_name={ type=x }",
        "division_name={ type=0 is_name_ordered=maybe }",
        "division_name={ type=0 name_order=4 name_order=4 }",
    ] {
        assert!(unit(bad).is_err(), "{bad}");
    }
}

#[test]
fn division_supply_days_and_fuel_parse_exactly_and_absent_at_zero() {
    let unit = |fields: &str| {
        division_save(&format!(
            "division={{id={{id=1 type=51}} logical_country=SOV strength=1 organisation=2 {fields}}}"
        ))
    };
    let saved = unit("fuel=604.8 out_of_supply_days=30").unwrap();
    let presentation = &saved.divisions[&((1_u64 << 32) | 51)].presentation;
    assert_eq!(presentation.fuel.as_ref().unwrap().raw, 60_480_000);
    assert_eq!(presentation.out_of_supply_days, Some(30));
    let saved = unit("").unwrap();
    let presentation = &saved.divisions[&((1_u64 << 32) | 51)].presentation;
    assert!(presentation.fuel.is_none());
    assert_eq!(presentation.out_of_supply_days, None);
    for fields in ["out_of_supply_days=x", "fuel=1 fuel=1", "fuel=0.000001"] {
        assert!(unit(fields).is_err(), "{fields}");
    }
}

#[test]
fn division_supply_grace_parser_preserves_zero_signed_precision_and_absence() {
    let unit = |fields: &str| {
        division_save(&format!(
            "division={{id={{id=1 type=51}} logical_country=SOV strength=1 organisation=2 {fields}}}"
        ))
    };
    let saved = unit("max_supply=90071992547.40993 army_current_supply_ratio=-12.34567").unwrap();
    let presentation = &saved.divisions[&((1_u64 << 32) | 51)].presentation;
    assert_eq!(
        presentation.max_supply.as_ref().unwrap().raw,
        9_007_199_254_740_993
    );
    assert_eq!(
        presentation.army_current_supply_ratio.as_ref().unwrap().raw,
        -1_234_567
    );
    let saved = unit("max_supply=0 army_current_supply_ratio=0").unwrap();
    let presentation = &saved.divisions[&((1_u64 << 32) | 51)].presentation;
    assert_eq!(presentation.max_supply.as_ref().unwrap().raw, 0);
    assert_eq!(
        presentation.army_current_supply_ratio.as_ref().unwrap().raw,
        0
    );
    let saved = unit("").unwrap();
    let presentation = &saved.divisions[&((1_u64 << 32) | 51)].presentation;
    assert!(presentation.max_supply.is_none());
    assert!(presentation.army_current_supply_ratio.is_none());
    for fields in [
        "max_supply=x",
        "army_current_supply_ratio=0.000001",
        "max_supply=1 max_supply=1",
        "army_current_supply_ratio=0 army_current_supply_ratio=0",
    ] {
        assert!(unit(fields).is_err(), "{fields}");
    }
}

#[test]
fn division_disrupted_supply_parser_keeps_positive_precision_and_omission() {
    let unit = |fields: &str| {
        division_save(&format!(
            "division={{id={{id=1 type=51}} logical_country=SOV strength=1 organisation=2 {fields}}}"
        ))
    };
    for (text, raw) in [("0.00001", 1), ("90071992547.40993", 9_007_199_254_740_993)] {
        let saved = unit(&format!("disrupted_supply={text}")).unwrap();
        let value = saved.divisions[&((1_u64 << 32) | 51)]
            .presentation
            .disrupted_supply
            .as_ref()
            .unwrap();
        assert_eq!(value.raw, raw);
        assert_eq!(value.lexeme, text.as_bytes());
    }
    let saved = unit("").unwrap();
    assert!(
        saved.divisions[&((1_u64 << 32) | 51)]
            .presentation
            .disrupted_supply
            .is_none()
    );
    for fields in [
        "disrupted_supply=0",
        "disrupted_supply=-0.00001",
        "disrupted_supply=x",
        "disrupted_supply=0.000001",
        "disrupted_supply={}",
        "disrupted_supply=1 disrupted_supply=1",
    ] {
        assert!(unit(fields).is_err(), "{fields}");
    }
}

#[test]
fn division_explicit_invalid_optionals_are_not_treated_as_absent() {
    let prefix = "division={id={id=1 type=51} logical_country=SOV strength=0 organisation=0 ";
    for optional in [
        "location=0",
        "location=-1",
        "location={}",
        "location=1 location=1",
        "division_template_id={}",
        "division_template_id={id=2}",
        "division_template_id={type=52}",
        "division_template_id={id=0 type=0}",
        "division_template_id=no",
        "expeditionary_owner={}",
    ] {
        assert!(
            division_save(&format!("{prefix}{optional}}}")).is_err(),
            "{optional}"
        );
    }
    assert!(
        division_save("division={id={id=0 type=0} logical_country=SOV strength=0 organisation=0}")
            .is_err()
    );
    for handle in ["id=0 type=51", "id=1 type=0"] {
        let body = format!(
            "division={{id={{{handle}}} logical_country=SOV strength=0 organisation=0 division_template_id={{{handle}}}}}"
        );
        assert!(division_save(&body).is_ok(), "{body}");
    }
}

#[test]
fn duplicate_division_handle_in_another_country_roster_is_refused() {
    let body = "division={id={id=1 type=51} logical_country=D01 strength=1 organisation=1}";
    let text = format!(
        "HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 countries={{ SOV={{units={{{body}}}}} D01={{units={{{body}}}}} }} states={{}} provinces={{}}"
    );
    assert!(
        parse(text.as_bytes())
            .unwrap_err()
            .to_string()
            .contains("duplicate full division handle")
    );
}

const WAR_FIELDS: &str = "first=GER second=SOV start_date=1936.1.1.12 first_casualties=0 second_casualties=0 first_unknown_casualties=0 second_unknown_casualties=0 first_was_instigator=yes";

fn diplomacy_save(relations: &str, factions: &str) -> io::Result<Save> {
    parse(format!(
        "HOI4txt\ndate=1936.1.2.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 countries={{ GER={{diplomacy={{active_relations={{{relations}}}}}}} SOV={{}} D01={{}} }} states={{}} provinces={{}} {factions}"
    ).as_bytes())
}

#[test]
fn war_relations_preserve_envelopes_order_and_unscaled_signed_integers() {
    let body = WAR_FIELDS
        .replace("first=GER", "first=D01")
        .replace("second=SOV", "second=GER")
        .replace(
            "first_casualties=0",
            "first_casualties=-9223372036854775808",
        )
        .replace(
            "second_casualties=0",
            "second_casualties=9223372036854775807",
        )
        .replace(
            "first_unknown_casualties=0",
            "first_unknown_casualties=9007199254740993",
        )
        .replace(
            "second_unknown_casualties=0",
            "second_unknown_casualties=-1",
        )
        .replace("first_was_instigator=yes", "first_was_instigator=no");
    let save = diplomacy_save(&format!("SOV={{war_relation={{{body}}}}}"), "").unwrap();
    let war = &save.war_relations[0];
    assert_eq!(war.roster_owner, b"GER");
    assert_eq!(war.target, b"SOV");
    assert_eq!(war.first, b"D01");
    assert_eq!(war.second, b"GER");
    assert_eq!(war.first_casualties, i64::MIN);
    assert_eq!(war.second_casualties, i64::MAX);
    assert_eq!(war.first_unknown_casualties, 9_007_199_254_740_993);
    assert_eq!(war.second_unknown_casualties, -1);
    assert!(!war.first_was_instigator);
    assert!(!war.cancel);
    assert_eq!(
        war.start_date.unwrap(),
        GameDate::parse(b"1936.1.1.12").unwrap()
    );
    assert_eq!(war.end_date, None);
}

#[test]
fn war_dates_preserve_writer_omissions_without_filling_raw_hours() {
    let cancelled = WAR_FIELDS.replace("start_date=1936.1.1.12", "cancel=yes");
    for suffix in ["", " end_date=1936.1.2.1"] {
        let save =
            diplomacy_save(&format!("SOV={{war_relation={{{cancelled}{suffix}}}}}"), "").unwrap();
        let war = &save.war_relations[0];
        assert!(war.cancel);
        assert_eq!(war.start_date, None);
        assert_eq!(war.end_date.is_some(), !suffix.is_empty());
    }
    let save = diplomacy_save(
        &format!("SOV={{war_relation={{{WAR_FIELDS} end_date=1936.1.1.13}}}}"),
        "",
    )
    .unwrap();
    let war = &save.war_relations[0];
    assert_eq!(
        war.end_date.unwrap().engine_hour().unwrap(),
        war.start_date.unwrap().engine_hour().unwrap() + 1
    );
    let empty = diplomacy_save("SOV={opinion=10}", "").unwrap();
    assert!(empty.war_relations.is_empty());
    assert!(empty.factions.is_empty());
}

#[test]
fn war_missing_required_fields_and_malformed_values_are_refused() {
    for required in WAR_FIELDS.split(' ') {
        let body = WAR_FIELDS.replace(required, "");
        assert!(
            diplomacy_save(&format!("SOV={{war_relation={{{body}}}}}"), "").is_err(),
            "missing {required}"
        );
    }
    for body in [
        WAR_FIELDS.replace("first=GER", "first={}"),
        WAR_FIELDS.replace("second=SOV", "second=\"\""),
        WAR_FIELDS.replace("first_casualties=0", "first_casualties=0.0"),
        WAR_FIELDS.replace(
            "second_casualties=0",
            "second_casualties=9223372036854775808",
        ),
        WAR_FIELDS.replace(
            "first_unknown_casualties=0",
            "first_unknown_casualties=-9223372036854775809",
        ),
        WAR_FIELDS.replace("first_was_instigator=yes", "first_was_instigator=1"),
        WAR_FIELDS.replace("start_date=1936.1.1.12", "start_date=1936.2.29.12"),
        WAR_FIELDS.replace("start_date=1936.1.1.12", "cancel=no"),
        format!("{WAR_FIELDS} cancel=no"),
        format!("{WAR_FIELDS} cancel=yes"),
        format!("{WAR_FIELDS} cancel={{}}"),
        format!("{WAR_FIELDS} end_date=1936.1.1.12"),
        format!("{WAR_FIELDS} end_date=1936.1.1.11"),
        format!("{WAR_FIELDS} end_date={{}}"),
        format!("{WAR_FIELDS} first_casualties=0"),
        format!("{WAR_FIELDS} first_was_instigator=yes"),
    ] {
        assert!(
            diplomacy_save(&format!("SOV={{war_relation={{{body}}}}}"), "").is_err(),
            "{body}"
        );
    }
}

#[test]
fn war_duplicate_pairs_are_refused_across_target_envelopes() {
    let war = format!("war_relation={{{WAR_FIELDS}}}");
    for relations in [
        format!("SOV={{{war} {war}}}"),
        format!("SOV={{{war}}} D01={{{war}}}"),
    ] {
        assert!(diplomacy_save(&relations, "").is_err(), "{relations}");
    }
    let reversed = WAR_FIELDS
        .replace("first=GER", "first=SOV")
        .replace("second=SOV", "second=GER");
    let save = diplomacy_save(
        &format!("SOV={{{war}}} GER={{war_relation={{{reversed}}}}}"),
        "",
    )
    .unwrap();
    assert_eq!(save.war_relations.len(), 2);
    assert_eq!(save.war_relations[1].first, b"SOV");
}

#[test]
fn factions_preserve_full_handles_literal_name_and_leader_order() {
    let save = diplomacy_save(
        "",
        r#"faction={id={type=88 id=4294967295} members={SOV GER D01} template=faction_template_allies}
faction={id={id=1 type=0} name="Literal name"}
faction={id={id=0 type=88} members={} name="" color={ 81 78 74 } is_color_overridden=no}"#,
    )
    .unwrap();
    let full = (u64::from(u32::MAX) << 32) | 88;
    assert_eq!(save.factions.len(), 3);
    let faction = &save.factions[&full];
    assert_eq!(faction.id, full);
    assert!(faction.name.is_none());
    assert_eq!(
        faction.members,
        [b"SOV".to_vec(), b"GER".to_vec(), b"D01".to_vec()]
    );
    let no_members = &save.factions[&(1_u64 << 32)];
    let colored = &save.factions[&88];
    assert_eq!(colored.color.as_deref(), Some(&[81, 78, 74][..]));
    assert_eq!(colored.color_overridden, Some(false));
    assert_eq!(no_members.name.as_deref(), Some(b"Literal name".as_slice()));
    assert!(no_members.members.is_empty());
    assert_eq!(save.factions[&88].name.as_deref(), Some(b"".as_slice()));
    assert!(save.factions[&88].members.is_empty());
    assert!(faction.icon.is_none() && faction.theatres.is_empty());
    assert_eq!(
        faction.template.as_deref(),
        Some(&b"faction_template_allies"[..])
    );
    assert!(faction.color.is_none() && faction.color_overridden.is_none());
    assert!(no_members.template.is_none());
}

#[test]
fn faction_theatres_keep_saved_order_ids_countries_and_hidden_tags() {
    let save = diplomacy_save(
        "",
        r#"faction={id={id=2 type=88} icon="GFX_icon" pings={ regions={
{ name="A" tag="SOV" commander={ id=844 type=4713 } execution_type=balanced template=t countries={ "SIK"
 "AUS"
 } ids={ 1 5 -1 } hidden={ "GER"
 } }
{ name="B" tag="SOV" }
} } }
faction={id={id=3 type=88} icon="" pings={ }}"#,
    )
    .unwrap();
    let faction = &save.factions[&((2_u64 << 32) | 88)];
    assert_eq!(faction.icon.as_deref(), Some(&b"GFX_icon"[..]));
    assert_eq!(
        faction.theatres,
        [
            FactionTheatre {
                name: Some(b"A".to_vec()),
                tag: Some(b"SOV".to_vec()),
                commander: Some((844 << 32) | 4713),
                regions: vec![1, 5, u32::MAX],
                countries: vec![b"SIK".to_vec(), b"AUS".to_vec()],
                hidden: vec![b"GER".to_vec()],
            },
            FactionTheatre {
                name: Some(b"B".to_vec()),
                tag: Some(b"SOV".to_vec()),
                ..FactionTheatre::default()
            },
        ]
    );
    let empty = &save.factions[&((3_u64 << 32) | 88)];
    assert_eq!(empty.icon.as_deref(), Some(&b""[..]));
    assert!(empty.theatres.is_empty());
    for bad in [
        "faction={id={id=2 type=88} pings={ other={} }}",
        "faction={id={id=2 type=88} pings={ regions={ key={} } }}",
        "faction={id={id=2 type=88} pings={ regions={ { hidden={ a=b } } } }}",
        "faction={id={id=2 type=88} pings={ regions={ { commander={ id=0 type=0 } } } }}",
        "faction={id={id=2 type=88} pings={ regions={ { name=\"A\" name=\"B\" } } }}",
    ] {
        assert!(diplomacy_save("", bad).is_err(), "{bad}");
    }
}

#[test]
fn factions_refuse_duplicate_handles_members_and_malformed_fields() {
    for body in [
        "",
        "id={}",
        "id={id=1}",
        "id={type=88}",
        "id={id=0 type=0}",
        "id={id=4294967296 type=88}",
        "id={id=1 type=-1}",
        "id={id=1 type=88 type=88}",
        "id={id=1 type=88} id={id=1 type=88}",
        "id=no",
        "id={id=1 type=88} name={}",
        "id={id=1 type=88} name=x name=x",
        "id={id=1 type=88} members=no",
        "id={id=1 type=88} members={GER GER}",
        "id={id=1 type=88} members={GER=yes}",
        "id={id=1 type=88} members={{GER}}",
        "id={id=1 type=88} members={\"\"}",
        "id={id=1 type=88} members={} members={}",
    ] {
        assert!(
            diplomacy_save("", &format!("faction={{{body}}}")).is_err(),
            "{body}"
        );
    }
    let duplicate = "faction={id={type=88 id=1}} faction={id={id=1 type=88}}";
    assert!(
        diplomacy_save("", duplicate)
            .unwrap_err()
            .to_string()
            .contains("duplicate full faction handle")
    );
}

#[test]
fn land_combat_keeps_typed_participants_and_does_not_substitute_front_units() {
    let save = diplomacy_save(
        "",
        "combat={land_combat={
        id={type=62 id=3} location=42 day=10 duration=240
        attacker={unit={type=51 id=7} front={type=51 id=99} unit={type=52 id=7}}
        defender={unit={type=51 id=8}}
    }}",
    )
    .unwrap();
    let combat = &save.land_combats[&((3_u64 << 32) | 62)];
    assert_eq!(combat.province, 42);
    assert_eq!(combat.attackers, [(7_u64 << 32) | 51, (7_u64 << 32) | 52]);
    assert_eq!(combat.defenders, [(8_u64 << 32) | 51]);
    let duplicate =
        "land_combat={id={id=1 type=62} location=2 day=0 duration=0 attacker={} defender={}}";
    assert!(diplomacy_save("", &format!("combat={{{duplicate} {duplicate}}}")).is_err());
    assert!(
        diplomacy_save(
            "",
            "combat={land_combat={id={id=1 type=62} location=2 attacker={}}}"
        )
        .is_err()
    );
}

#[test]
fn border_war_class_comes_from_the_enclosing_key_and_gates_side_fields() {
    let body =
        "id={type=62 id=3} location=42 day=0 duration=0 attacker={state={flag=yes}} defender={}";
    let ordinary = diplomacy_save("", &format!("combat={{land_combat={{{body}}}}}")).unwrap();
    let details = &ordinary.land_combats[&((3_u64 << 32) | 62)].details;
    assert!(!details.border_war);
    assert!(details.sides[0].border_state.is_none());
    let body = body.replace("state={flag=yes}", "state=11");
    let border = diplomacy_save("", &format!("combat={{border_war_combat={{{body}}}}}")).unwrap();
    let details = &border.land_combats[&((3_u64 << 32) | 62)].details;
    assert!(details.border_war);
    assert!(details.transfer_state.is_none());
    assert_eq!(details.sides[0].border_state, Some(11));
}

#[test]
fn player_header_preserves_coop_entries_empty_users_and_signed_ids() {
    let save = diplomacy_save(
        "",
        r#"player_countries={
        GER={user="" country_leader=no}
        GER={user="same" country_leader=yes id=-2147483648}
        SOV={user="same" country_leader=no id=2147483647}
        ---={user="zero" country_leader=no id=0}
    }"#,
    )
    .unwrap();
    assert_eq!(save.players.len(), 4);
    assert_eq!(save.players[0].country, b"GER");
    assert!(save.players[0].user.is_empty());
    assert_eq!(save.players[0].id, -1);
    assert_eq!(save.players[1].id, i32::MIN);
    assert!(save.players[1].country_leader);
    assert_eq!(save.players[2].id, i32::MAX);
    assert_eq!(save.players[3].country, b"---");
    assert_eq!(save.players[3].id, 0);
    assert!(diplomacy_save("", "").unwrap().players.is_empty());
}

#[test]
fn player_header_requires_serialized_fields_and_preserves_ambiguity() {
    for entry in [
        "user=x",
        "country_leader=yes",
        "user=x country_leader=maybe",
        "user=x country_leader=yes id=2147483648",
        "user=x country_leader=yes id=-2147483649",
        "user=x user=y country_leader=yes",
        "user={} country_leader=yes",
        "user=x country_leader=yes id=1 id=1",
    ] {
        assert!(
            diplomacy_save("", &format!("player_countries={{GER={{{entry}}}}}")).is_err(),
            "{entry}"
        );
    }
    let saved = diplomacy_save("", "player_countries={GER={user=x country_leader=yes id=1} GER={user=x country_leader=yes id=1}}").unwrap();
    assert_eq!(saved.players.len(), 2);
    assert_eq!(saved.players[0], saved.players[1]);
}

#[test]
fn army_orders_preserve_all_lists_marshal_ownership_and_signed_saved_bits() {
    let saved = parse(br#"HOI4txt
date=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
states={} provinces={} countries={GER={theatres={theatre={id={id=1 type=67}
orders_group={id={type=53 id=1} name="Army 1" field_marshal_group=no motorization_level=0
 order_instance={instance_id=4294967295 type=4 can_execute=-128 invasion_source=-1 path={2 1 2 -1} states={3 3}}
 fallback={instance_id=2 type=2}
 virtual_fallback={instance_id=3 type=-1}}
field_marshal_group={id={type=53 id=2} name="Army Group 1" motorization_level=0 field_marshal_group=yes orders_group={id=1 type=53}
 order_instance={instance_id=2 type=3 invasion_source=4 convoys={convoys=0 total=0}}}
}}}}
"#).unwrap();
    assert_eq!(saved.army_orders.len(), 4);
    let group = (1_u64 << 32) | 53;
    let order = &saved.army_orders[&(group, u32::MAX)];
    assert_eq!(
        (order.order_type, order.active, order.paradrop_target),
        (4, 128, u32::MAX)
    );
    assert_eq!(order.path, [2, 1, 2, u32::MAX]);
    assert_eq!(order.target_states, [3, 3]);
    assert!(saved.army_orders[&(group, 2)].path.is_empty());
    assert_eq!(
        saved.army_orders[&((2_u64 << 32) | 53, 2)].paradrop_target,
        4
    );
}

#[test]
fn air_wings_preserve_pool_and_override_countries_and_required_mission_fields() {
    let saved = diplomacy_save("", "strategic_air={GER={air_wing_pool={
        id={id=1 type=70} definition=fixture_plane air_base={id=2 type=63}
        air_wings={id={id=3 type=54} count=-1 tag=--- experience=0 equipment={allow_zero_entries=no} mission={type=-2147483648}}
        air_wings={id={id=4 type=54} count=100 tag=SOV experience=0 equipment={allow_zero_entries=no} mission={type=2 effective_planes_count=57 strategic_region=5 region_change_penalty=0.90001 active=yes}}
    }}}").unwrap();
    assert_eq!(saved.air_wings.len(), 2);
    let wing = &saved.air_wings[&((3_u64 << 32) | 54)];
    assert_eq!(wing.owner, b"GER");
    assert_eq!(wing.country_override, b"---");
    assert_eq!((wing.planes, wing.mission_mask), (-1, 1 << 31));
    assert_eq!((wing.mission_region, wing.mission_active), (None, None));
    let assigned = &saved.air_wings[&((4_u64 << 32) | 54)];
    assert_eq!(
        (assigned.mission_active, assigned.region_change_penalty),
        (Some(true), Some(90_001))
    );
    assert_eq!(wing.region_change_penalty, None);
    assert_eq!((wing.pool_ordinal, assigned.pool_ordinal), (0, 1));
    let saved = diplomacy_save("", "strategic_air={
        GER={
            air_wing_pool={id={id=1 type=70} definition=fixture_plane air_base={id=2 type=63}
                air_wings={id={id=9 type=54} count=1 tag=GER experience=0 equipment={allow_zero_entries=no} mission={type=0}}}
            air_wing_pool={id={id=5 type=70} definition=fixture_plane air_base={id=6 type=63}
                air_wings={id={id=3 type=54} count=1 tag=GER experience=0 equipment={allow_zero_entries=no} mission={type=0}}}
        }
        SOV={air_wing_pool={id={id=7 type=70} definition=fixture_plane air_base={id=8 type=63}
            air_wings={id={id=4 type=54} count=1 tag=SOV experience=0 equipment={allow_zero_entries=no} mission={type=0}}}}
    }").unwrap();
    let ordinal = |id: u64| saved.air_wings[&((id << 32) | 54)].pool_ordinal;
    assert_eq!((ordinal(9), ordinal(3), ordinal(4)), (0, 1, 0));
    let body = "strategic_air={GER={air_wing_pool={id={id=1 type=70} definition=fixture_plane air_base={id=2 type=63} air_wings={id={id=3 type=54} count=1 tag=GER experience=0 equipment={allow_zero_entries=no} mission={type=0}}}}}";
    assert!(diplomacy_save("", body).is_ok());
    for fragment in [
        body.replace("count=1", ""),
        body.replace("type=0", ""),
        body.replace("tag=GER", ""),
        body.replace("air_base={id=2 type=63}", ""),
    ] {
        assert!(diplomacy_save("", &fragment).is_err(), "{fragment}");
    }
    let error = diplomacy_save("", &body.replace("allow_zero_entries=no", ""))
        .unwrap_err()
        .to_string();
    assert!(error.contains("air save tables: missing serialized air boolean"));
}

#[test]
fn stored_air_wing_save_names_preserve_empty_missing_and_literal_text() {
    for (field, expected) in [
        ("", None),
        ("name=\"\"", Some(&b""[..])),
        (
            "name=\"Wing %d [Root.GetTag]\"",
            Some(&b"Wing %d [Root.GetTag]"[..]),
        ),
    ] {
        let body = format!(
            "strategic_air={{GER={{air_wing_pool={{id={{id=1 type=70}} definition=fixture_plane air_base={{id=2 type=63}} air_wings={{id={{id=3 type=54}} count=1 tag=GER experience=0 equipment={{allow_zero_entries=no}} mission={{type=0}} {field}}}}}}}}}"
        );
        let saved = diplomacy_save("", &body).unwrap();
        assert_eq!(
            saved.air_wings[&((3_u64 << 32) | 54)].name.as_deref(),
            expected
        );
    }
}

#[test]
fn naval_combat_keeps_full_member_handles_signed_slots_and_required_side_scalars() {
    let body = "combat={naval_combat={id={type=61 id=4294967295} location=2
        attacker={unit={id=5 type=61} positioning=-0.00001 anti_air=90071992547.40993 group={
            member={unique_id=-1 state=-2147483648 ship={id=1 type=51} convoy={id=2 type=69} cached_info={tag=GER}}
            member={unique_id=0 state=32 cached_info={tag=---}}}}
        defender={unit={id=7 type=61} positioning=0 anti_air=0 unit={id=6 type=61}}}}";
    let saved = diplomacy_save("", body).unwrap();
    let id = (u64::from(u32::MAX) << 32) | 61;
    assert_eq!(saved.naval_combats[&id].attacker_positioning.raw, -1);
    assert_eq!(saved.naval_combats[&id].attackers, [(5_u64 << 32) | 61]);
    assert_eq!(
        saved.naval_combats[&id].defenders,
        [(7_u64 << 32) | 61, (6_u64 << 32) | 61]
    );
    let member = &saved.naval_combat_participants[&(id, 0, u32::MAX)];
    assert_eq!(member.state, 1 << 31);
    assert_eq!(member.ship, Some((1_u64 << 32) | 51));
    assert_eq!(member.convoy, Some((2_u64 << 32) | 69));
    assert!(saved.naval_combat_participants[&(id, 0, 0)].ship.is_none());
    assert!(saved.naval_combats[&id].transfers.is_empty());
    let transported = body.replace(
        "defender={",
        "naval_transport={ id=7 type=4713 } naval_transport={ id=0 type=0 } defender={",
    );
    assert_eq!(
        diplomacy_save("", &transported).unwrap().naval_combats[&id].transfers,
        [(7 << 32) | 4713, 0]
    );
    for fragment in [
        body.replace("unique_id=-1", ""),
        body.replace("anti_air=0", ""),
        body.replace("tag=GER", ""),
        body.replace("unique_id=0", "unique_id=-1"),
    ] {
        assert!(diplomacy_save("", &fragment).is_err(), "{fragment}");
    }
}

fn naval_save(units: &str) -> io::Result<Save> {
    parse(format!(
    "HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 countries={{ GER={{units={{{units}}}}} SOV={{}} D01={{}} }} states={{}} provinces={{}}"
).as_bytes())
}

#[test]
fn naval_save_keeps_nested_ownership_full_handles_and_exact_fixed5() {
    let saved = naval_save(
        r#"
  fleet={id={id=4294967295 type=61} home_base=2
    task_force={id={id=2147483648 type=4713} id={type=4713 id=2147483648}
      location=1 logical_country=SOV expeditionary_owner=D01 mission={mission=-1}
      support_attack=7 path={ 3 7 } full_path={ 4 7 }
      ship={id={id=4294967295 type=51} definition=battleship
        strength=90071992547.40993 organisation=78.72001 experience=34.32537 sunk_convoys=-1
        history={sunk_ship={id={id=9 type=51} strength=99}}}
      ship={id={id=0 type=51} definition=destroyer strength=0 organisation=0}
    }
  }
  fleet={id={id=1 type=61}}
"#,
    )
    .unwrap();
    let fleet_id = (u64::from(u32::MAX) << 32) | 61;
    let force_id = (2_147_483_648_u64 << 32) | 4713;
    let ship_id = (u64::from(u32::MAX) << 32) | 51;
    assert_eq!(
        (
            saved.fleets.len(),
            saved.task_forces.len(),
            saved.ships.len()
        ),
        (2, 1, 2)
    );
    assert_eq!(saved.fleets[&fleet_id].owner, b"GER");
    assert_eq!(saved.fleets[&fleet_id].home_base, Some(2));
    assert_eq!(saved.fleets[&((1_u64 << 32) | 61)].home_base, None);
    let force = &saved.task_forces[&force_id];
    assert_eq!(
        (force.fleet, force.province, force.mission),
        (fleet_id, Some(1), -1)
    );
    assert_eq!(force.roster_owner, b"GER");
    assert_eq!(force.logical_owner, b"SOV");
    assert_eq!(
        (
            force.target,
            force.path.as_slice(),
            force.alternate_path.as_slice()
        ),
        (Some(7), [3, 7].as_slice(), [4, 7].as_slice())
    );
    assert_eq!(
        force.expeditionary_owner.as_deref(),
        Some(b"D01".as_slice())
    );
    let ship = &saved.ships[&ship_id];
    assert_eq!(ship.task_force, force_id);
    assert_eq!(ship.strength.raw, 9_007_199_254_740_993);
    assert_eq!(ship.organisation.raw, 7_872_001);
    assert_eq!(ship.experience.raw, 3_432_537);
    assert_eq!(ship.sunk_convoys, -1);
    let omitted = &saved.ships[&51];
    assert_eq!((omitted.experience.raw, omitted.sunk_convoys), (0, 0));
}

#[test]
fn naval_save_refuses_ambiguous_census_and_missing_required_values() {
    let ship = "ship={id={id=1 type=51} strength=1 organisation=1}";
    let force = format!(
        "task_force={{id={{id=2 type=51}} logical_country=GER mission={{mission=0}} {ship}}}"
    );
    let fleet = format!("fleet={{id={{id=3 type=61}} {force}}}");
    let omitted = &naval_save(&fleet).unwrap().task_forces[&((2_u64 << 32) | 51)];
    assert_eq!(
        (
            omitted.target,
            omitted.path.len(),
            omitted.alternate_path.len()
        ),
        (None, 0, 0)
    );
    for invalid in [
        fleet.replace("strength=1", ""),
        fleet.replace("organisation=1", ""),
        fleet.replace("strength=1", "strength=1.000001"),
        fleet.replace("logical_country=GER", ""),
        fleet.replace("mission={mission=0}", "mission={}"),
        fleet.replace("mission={mission=0}", ""),
        fleet.replace("id={id=1 type=51}", "id={id=1 type=51} id={id=2 type=51}"),
        fleet.replace(ship, &format!("{ship} {ship}")),
        fleet.replace(&force, &format!("{force} {force}")),
        format!("{fleet} {fleet}"),
        format!("{fleet} fleet={{id={{id=4 type=61}} {force}}}"),
    ] {
        assert!(naval_save(&invalid).is_err(), "{invalid}");
    }
}
const HEADER: &str = concat!(
    "HOI4txt\ndate=1936.1.2.1 game_unique_id=c version=b game_unique_seed=0",
    " debug_current_ref_id=4294967295 unit=4012 order_index=7693 front_index=8230",
    " theatre_index=112 theater_group_index=924 military_deployment_line_index=7841",
    " military_deployment_conveyor_index=744 equipment_variant_index=6295",
    " country_leader_index=1 navy_id=8263 land_combat_id=15359",
    " multiplayer_random_count=-1333963091"
);

fn relations_save(relations: &str, intel: &str) -> io::Result<Save> {
    parse(
        format!(
            "{HEADER} countries={{ GER={{ diplomacy={{ active_relations={{ {relations} }} }} {intel} }} SOV={{}} D01={{}} }} states={{}} provinces={{}}"
        )
        .as_bytes(),
    )
}

#[test]
fn header_counters_keep_the_widths_their_writer_prints() {
    let save = relations_save("", "").unwrap();
    assert_eq!(
        save.counters,
        AllocationCounters {
            debug_current_ref_id: u32::MAX,
            unit: 4012,
            order_index: 7693,
            front_index: 8230,
            theatre_index: 112,
            theater_group_index: 924,
            military_deployment_line_index: 7841,
            military_deployment_conveyor_index: 744,
            equipment_variant_index: 6295,
            country_leader_index: 1,
            navy_id: 8263,
            land_combat_id: 15359,
            multiplayer_random_count: -1_333_963_091,
        }
    );
}

#[test]
fn a_header_without_every_counter_is_refused_rather_than_defaulted() {
    for key in [
        "debug_current_ref_id=4294967295",
        "navy_id=8263",
        "multiplayer_random_count=-1333963091",
    ] {
        let header = HEADER.replace(key, "");
        let error = parse(format!("{header} countries={{}} states={{}} provinces={{}}").as_bytes())
            .unwrap_err()
            .to_string();
        assert!(error.contains("save lacks"), "{error}");
    }
    let header = HEADER.replace("debug_current_ref_id=4294967295", "debug_current_ref_id=-1");
    assert!(
        parse(format!("{header} countries={{}} states={{}} provinces={{}}").as_bytes()).is_err()
    );
}

#[test]
fn relation_records_keep_cached_sums_saved_ordinals_and_class_spellings() {
    let save = relations_save(
        r#"SOV={
            opinion={ modifier=m date="1940.4.24.1" value=35 }
            cached_sum=-100
            last_send_diplomat="1940.6.14.1"
            war_relation={ first="GER" second="SOV" start_date="1939.9.1.1"
                first_casualties=1 second_casualties=2 first_unknown_casualties=3
                second_unknown_casualties=4 first_was_instigator=yes }
            market_access_rights={ first="GER" second="SOV" start_date="1940.6.13.1" }
            improve_relation={ first="GER" second="SOV" cancel=yes end_date="1941.1.1.1" }
            attitude="attitude_threatened"
            border_friction_claim=0.2647
        }
        D01={ attitude="attitude_hostile" }"#,
        "",
    )
    .unwrap();
    assert_eq!(save.relation_opinions.len(), 1);
    assert_eq!(save.relation_opinions[0].owner, b"GER");
    assert_eq!(save.relation_opinions[0].target, b"SOV");
    assert_eq!(save.relation_opinions[0].cached_sum, -100);
    let kinds: Vec<_> = save
        .relation_tuples
        .iter()
        .map(|tuple| (tuple.kind.clone(), tuple.ordinal, tuple.cancel))
        .collect();
    assert_eq!(
        kinds,
        vec![
            (b"market_access_rights".to_vec(), 1, false),
            (b"improve_relation".to_vec(), 2, true),
        ]
    );
    assert_eq!(save.relation_tuples[0].first, b"GER");
    assert_eq!(save.relation_tuples[0].second, b"SOV");
    assert!(save.relation_tuples[0].start_date.is_some());
    assert!(save.relation_tuples[1].start_date.is_none());
    assert!(save.relation_tuples[1].end_date.is_some());
    assert_eq!(save.war_relations.len(), 1);
}

#[test]
fn a_subject_relation_occupies_an_ordinal_without_becoming_a_tuple() {
    let save = relations_save(
        r#"SOV={
            puppet={ autonomy_state="a" value=0.2 first="GER" second="SOV" start_date="1936.1.1.1" }
            guarantee={ first="GER" second="SOV" start_date="1936.1.1.2" }
        }"#,
        "",
    )
    .unwrap();
    assert_eq!(save.relation_tuples.len(), 1);
    assert_eq!(save.relation_tuples[0].kind, b"guarantee");
    assert_eq!(save.relation_tuples[0].ordinal, 1);
}

#[test]
fn relation_blocks_without_endpoints_are_not_relations() {
    let save = relations_save(
        r#"SOV={
            rule_overrides={ override={ can_send_volunteers=yes desc="" } }
            lend_lease_to_allies_history={ ic_given=375.2 fuel_given=0 ic_received=0 fuel_received=0 }
            guarantee={ first="GER" second="SOV" start_date="1936.1.1.2" }
        }"#,
        "",
    )
    .unwrap();
    assert_eq!(save.relation_tuples.len(), 1);
    assert_eq!(save.relation_tuples[0].ordinal, 0);
    assert_eq!(save.war_delivery_histories.len(), 1);
    assert_eq!(save.war_delivery_histories[0].industry_sent, 37_520_000);
}

#[test]
fn relation_tuples_enforce_the_writers_conditional_date_rules() {
    for body in [
        r#"guarantee={ first="A" second="B" cancel=no start_date="1936.1.1.1" }"#,
        r#"guarantee={ first="A" second="B" cancel=yes start_date="1936.1.1.1" }"#,
        r#"guarantee={ first="A" second="B" }"#,
        r#"guarantee={ first="A" second="B" start_date="1936.1.2.1" end_date="1936.1.1.1" }"#,
    ] {
        let error = relations_save(&format!("SOV={{ {body} }}"), "")
            .unwrap_err()
            .to_string();
        assert!(error.contains("relation"), "{body}: {error}");
    }
}

#[test]
fn country_intel_records_are_positional_and_omit_only_exact_zero_lanes() {
    let save = relations_save(
        "",
        "intel={ intel={ { } { army=-15 navy=-10 airforce=-15 } { civilian=33.3102 army=6.79238 navy=27.27772 airforce=8.13062 } } }",
    )
    .unwrap();
    let records = &save.country_intel[b"GER".as_slice()];
    assert_eq!(records.len(), 3);
    assert_eq!(records[0], CountryIntelRecord::default());
    assert_eq!(
        records[1],
        CountryIntelRecord {
            civilian: 0,
            army: -1_500_000,
            navy: -1_000_000,
            air: -1_500_000,
        }
    );
    assert_eq!(
        records[2],
        CountryIntelRecord {
            civilian: 3_331_020,
            army: 679_238,
            navy: 2_727_772,
            air: 813_062,
        }
    );
}

#[test]
fn malformed_country_intel_lists_are_refused_rather_than_normalized() {
    for (body, needle) in [
        ("intel={ intel={ } }", "positive count"),
        ("intel={ intel={ { civilian=0 } } }", "zero lane"),
        (
            "intel={ intel={ { civilian=1 civilian=2 } } }",
            "repeats a lane",
        ),
        ("intel={ intel={ { land=1 } } }", "unexpected field"),
        ("intel={ intel={ x={ civilian=1 } } }", "positional"),
    ] {
        let error = relations_save("", body).unwrap_err().to_string();
        assert!(error.contains(needle), "{body}: {error}");
    }
}

#[test]
fn manpower_recruitment_exile_and_ship_crews_preserve_saved_signed_words() {
    let doc = content_reader_parser::parser::parse(
        b"manpower={ratio=-312500 exile_manpower=-9 max_exile_manpower=8123}",
    )
    .unwrap();
    let country = parse_country(b"MOD", &doc.items).unwrap();
    assert_eq!(country.recruitment_ratio.unwrap(), -312_500);
    assert_eq!(country.exile_manpower, Some(-9));
    assert_eq!(country.exile_manpower_cap, Some(8123));
    let doc = content_reader_parser::parser::parse(
        b"id={id=1 type=51} strength=1 organisation=1 manpower=-17 max_manpower=512",
    )
    .unwrap();
    let ship = parse_ship(9, &doc.items).unwrap();
    assert_eq!((ship.manpower, ship.max_manpower), (Some(-17), Some(512)));
}

#[test]
fn manpower_wing_exile_tag_keeps_saved_presence() {
    for (field, expected) in [
        ("", None),
        ("government_in_exile_tag=EXL", Some(b"EXL".as_slice())),
        ("government_in_exile_tag=OTHER", Some(b"OTHER".as_slice())),
    ] {
        let body = format!(
            "strategic_air={{HST={{air_wing_pool={{id={{id=1 type=70}} definition=fixture_plane air_base={{id=2 type=63}} air_wings={{id={{id=3 type=54}} count=1 tag=HST experience=0 equipment={{allow_zero_entries=no}} mission={{type=0}} {field}}}}}}}}}"
        );
        let saved = diplomacy_save("", &body).unwrap();
        assert_eq!(
            saved.air_wings[&((3_u64 << 32) | 54)]
                .government_in_exile_tag
                .as_deref(),
            expected
        );
    }
}
