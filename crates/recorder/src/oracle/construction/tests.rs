use super::*;

fn history_fixture(input: &[u8]) -> installed_content::states::StateDefinition {
    let context = installed_content::states::Context {
        resource: &|_| installed_content::states::Resource::Found,
        max_building_levels: 20,
        province_count: 1000,
        names: &installed_content::key_names::KeyNames::default(),
    };
    installed_content::states::parse_states(input, &context)
        .unwrap()
        .0
        .remove(0)
}

/// The save's country-side `cores`, inverted, against the recorded state
/// lists: order and alias spelling do not matter; membership does, and
/// only states read at the frame's hour are compared.
#[test]
fn state_cores_compare_canonical_membership_with_the_saved_country_lists() {
    let country = |tag: &[u8], id| CountryRow {
        id,
        tag: tag.to_vec(),
        ..Default::default()
    };
    let (ger, fra, alias) = (country(b"GER", 5), country(b"FRA", 7), country(b"GER", 9));
    let links = BTreeMap::from([(5_u32, &ger), (7, &fra), (9, &alias)]);
    let saved = |cores: &[(&[u8], &[u32])]| {
        cores
            .iter()
            .map(|(tag, states)| {
                (
                    tag.to_vec(),
                    installed_content::save::Country {
                        tag: tag.to_vec(),
                        cores: states.to_vec(),
                        ..Default::default()
                    },
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    let reads = |states: &[u32]| {
        states
            .iter()
            .map(|&owner| replay::rows::SliceReadRow {
                table: replay::rows::TableId::StateCores.index() as u32,
                owner,
                sample_hour: 10,
            })
            .collect::<Vec<_>>()
    };
    let row = |state, cores: &[u32]| replay::rows::StateCoreRow {
        state,
        cores: cores.to_vec(),
    };
    let run = |rows: &[replay::rows::StateCoreRow], countries, read: &[u32]| {
        let mut report = Report::default();
        state_cores(rows, countries, &links, &reads(read), 10, &mut report).unwrap();
        report
    };
    let countries = saved(&[(b"GER", &[1, 2]), (b"FRA", &[1])]);
    // State 1 lists FRA then GER through an alias; state 3 has none.
    let report = run(
        &[row(1, &[7, 9]), row(2, &[5]), row(3, &[])],
        &countries,
        &[1, 2, 3],
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["StateCores"], 3);
    // A missing core differs; a state not read this hour is not compared.
    let report = run(&[row(1, &[5]), row(2, &[])], &countries, &[1]);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    // A saved core of a read state with no row is a difference too.
    let report = run(&[row(1, &[5, 7])], &countries, &[1, 2]);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
}

fn saved_pack(text: &[u8]) -> saved::Pack {
    saved::parse(&installed_content::parser::parse(text).unwrap().items).unwrap()
}

#[test]
fn occupation_law_names_compare_native_first_nul_prefixes() {
    use replay::rows::{
        OccupationGroupLaw, OccupationSettingsRow, Recorded, StateOccupationLawRow,
    };
    let saved = saved_pack(b"countries={AAA={occupation_status={default_law=Law occupation={BBB={occupation_law=Law occupation_law_list={1=Law}}}}} BBB={}} states={} provinces={}");
    let countries = [b"AAA", b"BBB"].map(|tag| CountryRow {
        tag: tag.to_vec(),
        ..Default::default()
    });
    let links = BTreeMap::from([(1, &countries[0]), (2, &countries[1])]);
    let row = OccupationSettingsRow {
        country: 1,
        default_law_present: 1,
        law: b"Law\0tail".to_vec(),
        group_laws: Recorded::Value(Some(vec![OccupationGroupLaw {
            occupied: 2,
            scope_country: Some(2),
            law: Some(b"Law\0tail".to_vec()),
        }])),
        ..Default::default()
    };
    let state = StateOccupationLawRow {
        controller: 1,
        occupied: 2,
        state: 1,
        name: b"Law\0tail".to_vec(),
    };
    let mut report = Report::default();
    occupation_settings(std::slice::from_ref(&row), &saved, &links, &mut report).unwrap();
    occupation_laws(std::slice::from_ref(&state), &saved, &links, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        report
            .limitations
            .iter()
            .filter(|(reason, _)| reason.contains("first NUL"))
            .flat_map(|(_, fields)| fields)
            .count(),
        3
    );
    let mut changed = row.clone();
    changed.law[0] = b'X';
    let Recorded::Value(Some(groups)) = &mut changed.group_laws else {
        unreachable!()
    };
    groups[0].law.as_mut().unwrap()[0] = b'X';
    let mut state = state;
    state.name[0] = b'X';
    let mut report = Report::default();
    occupation_settings(&[changed], &saved, &links, &mut report).unwrap();
    occupation_laws(&[state], &saved, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 3);
    let saved_empty = saved_pack(b"countries={AAA={occupation_status={default_law=\"\" occupation={BBB={occupation_law=\"\"}}}} BBB={}} states={} provinces={}");
    for name in [b"\0tail".as_slice(), b"".as_slice()] {
        let mut empty = row.clone();
        empty.law = name.to_vec();
        let Recorded::Value(Some(groups)) = &mut empty.group_laws else {
            unreachable!()
        };
        groups[0].law = Some(name.to_vec());
        let mut report = Report::default();
        occupation_settings(&[empty], &saved_empty, &links, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
    }
}

#[test]
fn garrison_equipment_oracle_compares_requirements_and_names_the_unsaved_allocation_cache() {
    use replay::{
        frame::TableView,
        rows::{
            GarrisonEquipment, GarrisonEquipmentInputs, Recorded, StateGarrisonRow, TableRows,
            VariantDefinitionRow,
        },
    };
    let a = CountryRow {
        tag: b"A".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(1, &a)]);
    let names = BTreeMap::from([(700, b"mod_weapon".as_slice())]);
    let variant = (9_u64 << 32) | 70;
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 10,
        metrics: Default::default(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
    };
    *frame.tables[TableId::StateGarrisons.index()] =
        TableView::Complete(TableRows::StateGarrisons(vec![StateGarrisonRow {
            controller: 1,
            occupied: 1,
            state: 1,
            equipment: Recorded::Value(GarrisonEquipmentInputs {
                rows: vec![GarrisonEquipment {
                    equipment_token: 700,
                    archetype_token: 700,
                    required_raw: 100_000,
                    allocated_raw: 50_000,
                    ..Default::default()
                }],
                failure: None,
            }),
            ..Default::default()
        }]));
    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: variant,
            archetype_token: 700,
            ..Default::default()
        }]));
    for (requirements, allocation, missing, differences) in [
        ("mod_weapon=1", "0.5", false, 0),
        ("mod_weapon=2", "0.5", false, 1),
        ("mod_weapon=1", "0.6", false, 0),
        ("mod_weapon=1 extra_mod_weapon=1", "0.5", false, 1),
        ("mod_weapon=1", "0.5", true, 0),
    ] {
        let pool = if missing {
            String::new()
        } else {
            format!(
                "equipment={{equipment={{id={{id=9 type=70}} amount={allocation}}} allow_zero_entries=no}}"
            )
        };
        let saved = saved_pack(format!("countries={{A={{occupation_status={{occupation={{A={{state_garrison_data={{1={{{pool} equipment_need={{{requirements}}}}}}}}}}}}}}}}} states={{}} provinces={{}}").as_bytes());
        let mut report = Report::default();
        garrison_equipment(&frame, &saved, &links, &names, &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("the garrison save writer"))
        );
    }
}

#[test]
fn state_resource_block_gate_keeps_negative_siblings_but_omits_negative_only_pools() {
    let saved =
        saved_pack(b"countries={} provinces={} states={2={resources={ore=8 coolant=-2}} 3={}}");
    let names = BTreeMap::from([(7, b"ore".as_slice()), (8, b"coolant".as_slice())]);
    let mut rows = vec![
        replay::rows::StateResourceRow {
            state: 2,
            resource: 7,
            pool: 800_000,
            ..Default::default()
        },
        replay::rows::StateResourceRow {
            state: 2,
            resource: 8,
            pool: -200_000,
            ..Default::default()
        },
        replay::rows::StateResourceRow {
            state: 3,
            resource: 8,
            pool: -300_000,
            ..Default::default()
        },
    ];
    let mut report = Report::default();
    state_pools(
        &rows,
        &saved.state_resources,
        &names,
        &|_| true,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["StateResources.pool"], 2);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("negative pool values"))
    );
    rows[1].pool -= 1;
    let mut report = Report::default();
    state_pools(
        &rows,
        &saved.state_resources,
        &names,
        &|_| true,
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
}

#[test]
fn state_pools_and_modifiers_compare_names_order_and_scopes() {
    let saved = saved_pack(
        br#"countries={} provinces={} states={2={resources={steel=8} dynamic_modifier={
            modifier={modifier="b" tag="SWI" enabled=yes}
            modifier={modifier="a" enabled=yes}
        }}}"#,
    );
    let names = BTreeMap::from([(7_u32, &b"steel"[..]), (8, &b"oil"[..])]);
    let pool = |resource, pool| replay::rows::StateResourceRow {
        state: 2,
        resource,
        pool,
        ..Default::default()
    };
    let mut report = Report::default();
    state_pools(
        &[pool(7, 800_000), pool(8, 0)],
        &saved.state_resources,
        &names,
        &|_| true,
        &mut report,
    )
    .unwrap();
    assert_eq!(report.compared_tables["StateResources.pool"], 1);
    assert!(report.differences.is_empty());
    state_pools(
        &[pool(7, 800_001)],
        &saved.state_resources,
        &names,
        &|_| true,
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 1);
    // A state read at an earlier hour is reported, not compared.
    let mut report = Report::default();
    state_pools(
        &[pool(7, 800_001)],
        &saved.state_resources,
        &names,
        &|state| state != 2,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.compared_tables["StateResources.pool"], 0);
    assert_eq!(report.limitations.len(), 2);

    let swiss = CountryRow {
        id: 5,
        tag: b"SWI".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(5_u32, &swiss)]);
    let modifier = |ordinal, token: &[u8], scope_country| replay::rows::StateDynamicModifierRow {
        state: 2,
        ordinal,
        token: token.to_vec(),
        scope_country,
        scope_state: 0,
    };
    let mut report = Report::default();
    state_modifiers(
        &[modifier(0, b"b", 5), modifier(1, b"a", 0)],
        &saved.state_dynamic_modifiers,
        &links,
        &|_| true,
        &mut report,
    )
    .unwrap();
    assert_eq!(report.compared_tables["StateDynamicModifiers"], 2);
    assert!(report.differences.is_empty());
    state_modifiers(
        &[modifier(0, b"a", 0)],
        &saved.state_dynamic_modifiers,
        &links,
        &|_| true,
        &mut report,
    )
    .unwrap();
    // The swapped name and the missing second entry.
    assert_eq!(report.differences.len(), 3);
}

/// Origin presence comes from the controller's saved `origin` states and
/// the infrastructure level from the saved state building the engine's
/// fixed token names; a state without an origin has no level to compare.
#[test]
fn resource_inputs_compare_origins_and_infrastructure_levels() {
    let saved = saved_pack(
        br#"countries={ITA={resources={origin={country="ITA" state=2} origin={country="ITA" state=4}}}} provinces={}
        states={2={buildings={infra={level=5 healthy_levels=3 partial_health=0}}} 3={} 4={}}"#,
    );
    let italy = CountryRow {
        id: 5,
        tag: b"ITA".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(5_u32, &italy)]);
    let names = BTreeMap::from([(
        engine::cbuilding_database::INFRASTRUCTURE_TOKEN,
        &b"infra"[..],
    )]);
    let states: Vec<_> = [2, 3, 4]
        .map(|id| replay::rows::StateRow {
            id,
            controller: 5,
            ..Default::default()
        })
        .into();
    let input = |state, origin, infrastructure_level| replay::rows::StateResourceInputRow {
        state,
        origin,
        infrastructure_level,
        ..Default::default()
    };
    let run = |rows: &[_], sampled: &dyn Fn(u32) -> bool| {
        let mut report = Report::default();
        resource_inputs(rows, &states, &saved, &links, &names, sampled, &mut report).unwrap();
        report
    };
    let exact = [input(2, 1, Some(5)), input(3, 0, None), input(4, 1, None)];
    let report = run(&exact, &|_| true);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["StateResourceInputs"], 3);
    let report = run(
        &[input(2, 1, Some(4)), input(3, 1, None), input(4, 0, None)],
        &|_| true,
    );
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    // States read at an earlier hour are named, not compared.
    let report = run(&[input(2, 0, None)], &|state| state != 2);
    assert!(report.differences.is_empty());
    assert_eq!(report.compared_tables["StateResourceInputs"], 0);
}

#[test]
fn resource_inputs_compare_null_controller_without_a_country_join() {
    let saved = saved_pack(
        b"countries={A={resources={origin={country=A state=2}}}} states={2={}} provinces={}",
    );
    let states = [replay::rows::StateRow {
        id: 2,
        controller: 0,
        ..Default::default()
    }];
    let names = BTreeMap::from([(
        engine::cbuilding_database::INFRASTRUCTURE_TOKEN,
        b"infra".as_slice(),
    )]);
    let mut row = replay::rows::StateResourceInputRow {
        state: 2,
        ..Default::default()
    };
    let run = |row| {
        let mut report = Report::default();
        resource_inputs(
            &[row],
            &states,
            &saved,
            &BTreeMap::new(),
            &names,
            &|_| true,
            &mut report,
        )
        .unwrap();
        report
    };
    let report = run(row.clone());
    assert!(report.differences.is_empty());
    assert_eq!(report.compared_tables["StateResourceInputs"], 1);
    row.origin = 1;
    assert!(
        run(row)
            .differences
            .iter()
            .any(|row| row.contains(".origin"))
    );
}

/// Overrides compare entry by entry and element by element in native
/// order, the saved element key through the recorded resource database;
/// a controller without rows compares as empty only in its own slice.
#[test]
fn building_overrides_compare_entries_elements_and_empty_controllers() {
    let saved = saved_pack(
        br#"countries={ITA={resources={modify_building_resources={refinery={3=3 1=-0.5} dock={}}}}
        GER={resources={modify_building_resources={refinery={3=1}}}}} provinces={} states={}"#,
    );
    let italy = CountryRow {
        id: 5,
        tag: b"ITA".to_vec(),
        ..Default::default()
    };
    let germany = CountryRow {
        id: 24,
        canonical_slot: 1,
        tag: b"GER".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(5_u32, &italy), (24, &germany)]);
    let names = BTreeMap::from([(40_u32, &b"refinery"[..]), (41, &b"dock"[..])]);
    let resources: Vec<_> = [(1, 70), (3, 72)]
        .map(|(id, token)| replay::rows::ResourceDefinitionRow {
            id,
            token,
            ..Default::default()
        })
        .into();
    let states: Vec<_> = [(2, 5), (3, 24)]
        .map(|(id, controller)| replay::rows::StateRow {
            id,
            controller,
            ..Default::default()
        })
        .into();
    let hour = 5;
    let row = |ordinal, entry, building, count, resource, amount| {
        replay::rows::CountryBuildingResourceRow {
            country: 5,
            ordinal,
            entry,
            building,
            count,
            resource,
            amount,
        }
    };
    let read = |owner, sample_hour| replay::rows::SliceReadRow {
        table: replay::rows::TableId::CountryBuildingResources.index() as u32,
        owner,
        sample_hour,
    };
    let attempt = |rows: &[_], reads: &[_], hour| {
        let mut report = Report::default();
        building_overrides(
            rows,
            reads,
            &resources,
            &states,
            &saved,
            &links,
            &names,
            hour,
            &mut report,
        )
        .map(|()| report)
    };
    let run = |rows: &[_], reads: &[_], hour| attempt(rows, reads, hour).unwrap();
    let exact = [
        row(0, 0, 40, 2, 72, 300_000),
        row(1, 0, 40, 2, 70, -50_000),
        row(2, 1, 41, 0, 0, 0),
    ];
    // ITA was read at hour 5; GER, without rows, at hour 0.
    let reads = [read(5, hour), read(24, 0)];
    let report = run(&exact, &reads, hour);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["CountryBuildingResources"], 2);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("read before this hour"))
    );
    let changed = [row(0, 0, 40, 2, 70, 300_000), row(1, 0, 40, 2, 70, -50_001)];
    let report = run(&changed, &reads, hour);
    // The resource, the amount and the missing empty entry.
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    // At hour 0 GER's read found no rows against one saved entry.
    let report = run(&[], &[read(5, hour), read(24, 0)], 0);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    // A controller with neither rows nor a recorded read is named, not
    // compared, whatever its slice.
    let report = run(&[], &[read(5, hour)], 0);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("no SliceReads hour"))
    );
    // Rows without a read hour, and rows that skip an entry, are refused.
    assert!(attempt(&exact, &[], hour).is_err());
    assert!(attempt(&[row(0, 1, 40, 0, 0, 0)], &reads, hour).is_err());
}

#[test]
fn building_and_railway_comparisons_detect_one_native_unit_and_order_changes() {
    let saved = saved_pack(b"countries={GER={production={general_lines={rail_way={id={id=2 type=4713} cost=90071992547.40993 priority=-1 amount=1 created_date=1936.1.1.12 base=-1 target=2 current_province=-1 remaining_hours=-0.00001 rail_way={7 4 7}}}}}} states={1={buildings={factory={level=-32768 healthy_levels=-2 partial_health=0.00001}}}} provinces={}");
    let germany = CountryRow {
        id: 12,
        canonical_slot: 3,
        tag: b"GER".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(12, &germany)]);
    let names = BTreeMap::from([(31, b"factory".as_slice())]);
    let building = replay::rows::BuildingRow {
        location: 1,
        building_type: 31,
        level: i16::MIN,
        healthy_level: -2,
        partial_health: 1,
        ..Default::default()
    };
    let mut rail = replay::rows::RailwayConstructionRow {
        id: (2_u64 << 32) | 4713,
        country: 12,
        cost: 9_007_199_254_740_993,
        priority: -1,
        amount: 1,
        created_hour: installed_content::save::GameDate::parse(b"1936.1.1.12")
            .unwrap()
            .engine_hour()
            .unwrap(),
        base_level: -1,
        target_level: 2,
        current_segment: -1,
        remaining_cost: -1,
        route: vec![7, 4, 7],
        ..Default::default()
    };
    let mut report = Report::default();
    buildings(
        &[building],
        &saved.state_buildings,
        false,
        &names,
        &mut report,
    )
    .unwrap();
    railway_construction(&[rail.clone()], &saved, &links, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    rail.cost -= 1;
    rail.route.swap(0, 1);
    rail.created_hour += 1;
    railway_construction(&[rail], &saved, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 3);
}

#[test]
fn occupation_template_id_retains_full_handle_and_null_omission() {
    let saved = saved_pack(b"countries={GER={occupation_status={division_template_id={id=4294967295 type=52} default_law=\"\"}} FRA={occupation_status={}}} states={} provinces={}");
    let germany = CountryRow {
        tag: b"GER".to_vec(),
        ..Default::default()
    };
    let france = CountryRow {
        tag: b"FRA".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(12, &germany), (22, &france)]);
    let mut rows = [
        replay::rows::OccupationSettingsRow {
            country: 12,
            garrison_template: Some((u64::from(u32::MAX) << 32) | 52),
            default_law_present: 1,
            law: Vec::new(),
            group_laws: replay::rows::Recorded::Value(Some(Vec::new())),
            ..Default::default()
        },
        replay::rows::OccupationSettingsRow {
            country: 22,
            group_laws: replay::rows::Recorded::Value(Some(Vec::new())),
            ..Default::default()
        },
    ];
    let mut report = Report::default();
    occupation_settings(&rows, &saved, &links, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);

    rows[0].garrison_template = Some((u64::from(u32::MAX - 1) << 32) | 52);
    occupation_settings(&rows, &saved, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("garrison_template"));

    rows[0].garrison_template = Some((u64::from(u32::MAX) << 32) | 52);
    rows[1].garrison_template = Some((1_u64 << 32) | 52);
    let mut report = Report::default();
    occupation_settings(&rows, &saved, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("FRA.garrison_template"));
}

#[test]
fn occupied_country_roster_oracle_checks_order_and_appended_availability() {
    let saved = saved_pack(
        b"countries={AAA={occupation_status={occupation={CCC={} BBB={}}}}} states={} provinces={}",
    );
    let countries = [b"AAA", b"BBB", b"CCC"].map(|tag| CountryRow {
        tag: tag.to_vec(),
        ..Default::default()
    });
    let links = countries
        .iter()
        .enumerate()
        .map(|(ordinal, country)| (ordinal as u32 + 1, country))
        .collect();
    let mut row = replay::rows::OccupationSettingsRow {
        country: 1,
        occupied: replay::rows::Recorded::Value(Some(vec![3, 2])),
        group_laws: replay::rows::Recorded::Value(Some(
            [3, 2]
                .map(|occupied| replay::rows::OccupationGroupLaw {
                    occupied,
                    scope_country: Some(occupied),
                    law: None,
                })
                .to_vec(),
        )),
        ..Default::default()
    };
    let mut report = Report::default();
    occupation_settings(&[row.clone()], &saved, &links, &mut report).unwrap();
    assert!(report.differences.is_empty());
    row.occupied = replay::rows::Recorded::Value(Some(vec![2, 3]));
    occupation_settings(&[row.clone()], &saved, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    row.occupied = replay::rows::Recorded::NotRecorded;
    let mut report = Report::default();
    occupation_settings(&[row], &saved, &links, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.compared_tables["OccupationSettings"], 1);
    assert!(
        report
            .limitations
            .contains_key("OccupationSettings.occupied not recorded")
    );
}

#[test]
fn laws_with_zero_state_remain_distinct_in_different_occupied_scopes() {
    let saved = saved_pack(b"countries={GER={occupation_status={occupation={FRA={occupation_law_list={0=a}} ENG={occupation_law_list={0=b}}}}}} states={} provinces={}");
    let germany = CountryRow {
        tag: b"GER".to_vec(),
        ..Default::default()
    };
    let france = CountryRow {
        tag: b"FRA".to_vec(),
        ..Default::default()
    };
    let britain = CountryRow {
        tag: b"ENG".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(12, &germany), (22, &france), (32, &britain)]);
    let mut rows = [
        replay::rows::StateOccupationLawRow {
            controller: 12,
            occupied: 22,
            state: 0,
            name: b"a".to_vec(),
        },
        replay::rows::StateOccupationLawRow {
            controller: 12,
            occupied: 32,
            state: 0,
            name: b"b".to_vec(),
        },
    ];
    let mut report = Report::default();
    occupation_laws(&rows, &saved, &links, &mut report).unwrap();
    assert!(report.differences.is_empty());
    rows[0].name = b"b".to_vec();
    occupation_laws(&rows, &saved, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
}

#[test]
fn resistance_country_omission_does_not_certify_raw_zero() {
    let saved = saved_pack(b"countries={} states={1={resistance={resistance=90071992547.40993 resistance_speed=-0.00001}}} provinces={}");
    let mut row = replay::rows::StateResistanceRow {
        state: 1,
        country: u32::MAX,
        level: 9_007_199_254_740_993,
        speed: -1,
        ..Default::default()
    };
    let mut report = Report::default();
    resistance(&[row.clone()], &saved, &BTreeMap::new(), &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("signed-nonpositive"))
    );
    row.level -= 1;
    resistance(&[row], &saved, &BTreeMap::new(), &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
}

#[test]
fn repair_omission_is_an_inequality_not_a_zero_value() {
    let mut report = Report::default();
    repair_levels(&mut report, "negative", -9, None);
    repair_levels(&mut report, "zero", 0, None);
    assert!(report.differences.is_empty());
    assert_eq!(report.limitations.values().map(Vec::len).sum::<usize>(), 2);
    repair_levels(&mut report, "positive", 1, None);
    repair_levels(&mut report, "present", -1, Some(1));
    assert_eq!(report.differences.len(), 2);
}

#[test]
fn dictionary_missing_entry_is_explicit_and_literal_bytes_are_preserved() {
    let names = BTreeMap::from([(17, b"\xff".as_slice()), (19, b"factory\0tail".as_slice())]);
    assert_eq!(token(&names, 17).unwrap(), b"\xff");
    assert!(token(&names, 18).unwrap_err().contains("absent"));
    assert!(
        token(&names, 19)
            .unwrap_err()
            .contains("mapping to text is unproved")
    );
    assert_eq!(names[&19], b"factory\0tail");
}

#[test]
fn missing_dictionary_preserves_scope_checks_without_inventing_named_rows() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let saved = saved_pack(b"countries={} states={} provinces={7={buildings={factory={level=1 healthy_levels=1 partial_health=0}}}}");
    let row = replay::rows::BuildingRow {
        location: 7,
        building_type: 99,
        ordinal: 77,
        level: i16::MIN,
        healthy_level: i16::MIN,
        partial_health: i64::MIN,
        ..Default::default()
    };
    let mut report = Report::default();
    buildings(
        std::slice::from_ref(&row),
        &saved.province_buildings,
        true,
        &BTreeMap::new(),
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["ProvinceBuildings"], 0);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("levels and partial health cannot be matched"))
    );

    let mut extra = row.clone();
    extra.building_type += 1;
    let mut report = Report::default();
    buildings(
        &[row, extra],
        &saved.province_buildings,
        true,
        &BTreeMap::new(),
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("row_count"));

    let mut content = saved::BaseResources::default();
    content
        .add_state_history(&history_fixture(b"state={id=1 resources={steel=3 oil=7}}"))
        .unwrap();
    let resources = [
        replay::rows::StateBaseResourceRow {
            state: 1,
            resource: 91,
            ordinal: 77,
            amount: i64::MIN,
        },
        replay::rows::StateBaseResourceRow {
            state: 1,
            resource: 92,
            ordinal: 1,
            amount: 700_001,
        },
    ];
    let mut report = Report::default();
    base_resources(
        &resources,
        &content,
        &BTreeMap::from([(92, b"oil".as_slice())]),
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("oil.amount"));
    assert_eq!(
        report.compared_tables["StateBaseResources (verified content)"],
        1
    );
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("amount cannot be matched"))
    );
}

#[test]
fn nul_token_mapping_limits_keep_independent_scope_count_and_numeric_checks() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let saved = saved_pack(b"countries={GER={production={general_lines={building={id={id=4 type=59} cost=1 priority=0 amount=1 created_date=1936.1.1.12 building={template=factory location=3}}}}}} states={} provinces={7={buildings={factory={level=1 healthy_levels=1 partial_health=0}}}}");
    let names = BTreeMap::from([
        (11, b"factory\0tail".as_slice()),
        (12, b"oil\0tail".as_slice()),
    ]);
    let mut report = Report::default();
    buildings(
        &[replay::rows::BuildingRow {
            location: 7,
            building_type: 11,
            level: i16::MIN,
            partial_health: i64::MIN,
            ..Default::default()
        }],
        &saved.province_buildings,
        true,
        &names,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);

    let mut content = saved::BaseResources::default();
    content
        .add_state_history(&history_fixture(b"state={id=1 resources={oil=3 steel=0}}"))
        .unwrap();
    base_resources(
        &[replay::rows::StateBaseResourceRow {
            state: 1,
            resource: 12,
            ordinal: 77,
            amount: i64::MIN,
        }],
        &content,
        &names,
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("row_count"));

    let germany = CountryRow {
        tag: b"GER".to_vec(),
        ..Default::default()
    };
    building_construction(
        &[replay::rows::BuildingConstructionRow {
            id: (4_u64 << 32) | 59,
            country: 12,
            cost: 100_001,
            amount: 1,
            created_hour: installed_content::save::GameDate::parse(b"1936.1.1.12")
                .unwrap()
                .engine_hour()
                .unwrap(),
            building_type: 11,
            target_state: 4,
            ..Default::default()
        }],
        &saved,
        &BTreeMap::from([(12, &germany)]),
        &names,
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    assert!(
        report.differences[1..]
            .iter()
            .any(|difference| difference.contains("cost"))
    );
    assert!(
        report.differences[1..]
            .iter()
            .any(|difference| difference.contains("target_location"))
    );
    assert_eq!(report.compared_tables["ProvinceBuildings"], 0);
    assert_eq!(
        report.compared_tables["StateBaseResources (verified content)"],
        0
    );
    assert_eq!(report.compared_tables["BuildingConstruction"], 1);
    assert_eq!(
        report
            .limitations
            .keys()
            .filter(|reason| reason.contains("mapping to text is unproved"))
            .count(),
        3
    );
}

#[test]
fn resolved_province_buildings_compare_signed_zero_values_and_named_membership() {
    let saved = saved_pack(b"countries={} states={7={buildings={port={level=9 healthy_levels=9 partial_health=0}}}} provinces={7={buildings={factory={level=0 healthy_levels=0 partial_health=0} port={level=-32768 healthy_levels=-1 partial_health=-90071992547.40993}}}}");
    let names = BTreeMap::from([(91, b"factory".as_slice()), (92, b"port".as_slice())]);
    let rows = [
        replay::rows::BuildingRow {
            location: 7,
            building_type: 91,
            ..Default::default()
        },
        replay::rows::BuildingRow {
            location: 7,
            building_type: 92,
            ordinal: 1,
            level: i16::MIN,
            healthy_level: -1,
            partial_health: -9_007_199_254_740_993,
            ..Default::default()
        },
    ];
    let mut report = Report::default();
    buildings(&rows, &saved.province_buildings, true, &names, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["ProvinceBuildings"], 2);

    // A positive saved level must be in the active vector.
    let mut report = Report::default();
    buildings(
        &[replay::rows::BuildingRow {
            location: 7,
            building_type: 92,
            level: 9,
            healthy_level: 9,
            ..Default::default()
        }],
        &saved.state_buildings,
        false,
        &names,
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("active"));

    let mut changed = rows.clone();
    changed[0].partial_health = 1;
    changed[1].healthy_level -= 1;
    changed[1].ordinal = 0;
    let mut report = Report::default();
    buildings(
        &changed,
        &saved.province_buildings,
        true,
        &names,
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);

    let wrong_names = BTreeMap::from([(91, b"factory".as_slice()), (92, b"other".as_slice())]);
    let mut report = Report::default();
    buildings(
        &rows,
        &saved.province_buildings,
        true,
        &wrong_names,
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert_eq!(report.compared_tables["ProvinceBuildings"], 1);

    assert!(
        buildings(
            &[rows[0].clone(), rows[0].clone()],
            &saved.province_buildings,
            true,
            &names,
            &mut Report::default(),
        )
        .is_err()
    );
}

#[test]
fn naval_bases_sum_the_contents_port_buildings() {
    let saved = saved_pack(b"countries={} states={} provinces={7={buildings={dock={level=2 healthy_levels=1 partial_health=0} pier={level=3 healthy_levels=3 partial_health=0} factory={level=5 healthy_levels=5 partial_health=0}}}}");
    let ports = BTreeSet::from([b"dock".to_vec(), b"pier".to_vec()]);
    let country_bases = [
        installed_content::save::naval_launch::CountryNavalLaunchOccurrence {
            ordinal: 0,
            tag: b"TST".to_vec(),
            naval_bases: Some(
                vec![7]
                    .into_iter()
                    .map(installed_content::save::naval_launch::NavalBaseOccurrence::from)
                    .collect(),
            ),
        },
    ];
    let mut row = replay::rows::NavalBaseRow {
        province: 7,
        level: 5,
        healthy_level: 4,
        country_tags: replay::rows::Recorded::Value(Some(Vec::new())),
        ..Default::default()
    };
    let mut report = Report::default();
    naval_bases(
        std::slice::from_ref(&row),
        &saved.province_buildings,
        &ports,
        &country_bases,
        &BTreeMap::new(),
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    row.level = 2;
    let mut report = Report::default();
    naval_bases(
        &[row.clone()],
        &saved.province_buildings,
        &ports,
        &country_bases,
        &BTreeMap::new(),
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    // An unreadable country list is named on its own base, never waived.
    row.level = 5;
    row.countries_unreadable = true;
    let mut report = Report::default();
    naval_bases(
        &[row],
        &saved.province_buildings,
        &ports,
        &country_bases,
        &BTreeMap::new(),
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert_eq!(report.compared_tables["NavalBases"], 1);
}

#[test]
fn naval_base_membership_uses_saved_rosters_rather_than_all_port_buildings() {
    let saved = saved_pack(b"countries={} states={} provinces={19={buildings={synthetic_port={level=2 healthy_levels=2 partial_health=0}}}}");
    let ports = BTreeSet::from([b"synthetic_port".to_vec()]);
    let mut report = Report::default();
    naval_bases(
        &[],
        &saved.province_buildings,
        &ports,
        &[],
        &BTreeMap::new(),
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let rosters = [
        installed_content::save::naval_launch::CountryNavalLaunchOccurrence {
            ordinal: 0,
            tag: b"TST".to_vec(),
            naval_bases: Some(
                vec![19]
                    .into_iter()
                    .map(installed_content::save::naval_launch::NavalBaseOccurrence::from)
                    .collect(),
            ),
        },
    ];
    naval_bases(
        &[],
        &saved.province_buildings,
        &ports,
        &rosters,
        &BTreeMap::new(),
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("membership"));
}

#[test]
fn naval_base_saved_country_lists_compare_order_multiplicity_and_zero_level_bases() {
    use installed_content::save::naval_launch::parse_country_occurrences;
    use replay::rows::{NavalBaseRow, Recorded};
    let bases = parse_country_occurrences(&installed_content::parser::parse(
        b"A={strategic_navy={naval_base={7={disabled_for_repairs=\"A\" disabled_for_repairs=\"B\" disabled_for_repairs=\"A\" disabled_for_repairs=\"\"} 7={disabled_for_repairs=\"A\" disabled_for_repairs=\"B\" disabled_for_repairs=\"A\" disabled_for_repairs=\"\"}}}}",
    ).unwrap().items).unwrap();
    // Raw aliases cannot be projected through the canonical CountryLinks:
    // The game reads their own tag-table slots; slot zero uses the database.
    let tags = [
        b"A\0tail".to_vec(),
        b"B".to_vec(),
        b"A".to_vec(),
        Vec::new(),
    ];
    let mut row = NavalBaseRow {
        province: 7,
        countries: vec![5, 8, 5, 0],
        country_tags: Recorded::Value(Some(tags.to_vec())),
        ..Default::default()
    };
    let run = |row: &NavalBaseRow, bases: &[_]| {
        let mut report = Report::default();
        naval_bases(
            std::slice::from_ref(row),
            &BTreeMap::new(),
            &BTreeSet::new(),
            bases,
            &BTreeMap::from([
                (5, b"A".as_slice()),
                (8, b"B".as_slice()),
                (0, b"".as_slice()),
            ]),
            &mut report,
        )
        .unwrap();
        report
    };
    let report = run(&row, &bases);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    for wrong in [
        vec![b"X".to_vec(), b"B".to_vec(), b"A".to_vec(), Vec::new()],
        vec![b"B".to_vec(), b"A".to_vec(), b"A".to_vec(), Vec::new()],
        vec![b"A".to_vec(), b"B".to_vec(), b"A".to_vec()],
        vec![
            b"A".to_vec(),
            b"B".to_vec(),
            b"A".to_vec(),
            Vec::new(),
            Vec::new(),
        ],
    ] {
        row.country_tags = Recorded::Value(Some(wrong));
        let report = run(&row, &bases);
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        assert!(
            report
                .differences
                .iter()
                .all(|difference| difference.contains(".countries"))
        );
    }
    row.country_tags = Recorded::NotRecorded;
    assert!(run(&row, &bases).differences.is_empty());
    row.countries.swap(0, 1);
    assert_eq!(run(&row, &bases).differences.len(), 2);
    row.countries.swap(0, 1);
    row.country_tags = Recorded::Value(None);
    assert_eq!(run(&row, &bases).differences.len(), 2);
    row.countries_unreadable = true;
    assert_eq!(run(&row, &bases).differences.len(), 1);
    row.countries_unreadable = false;
    row.country_tags = Recorded::Value(Some(tags.to_vec()));
    let mut wrong_occurrence = bases.clone();
    wrong_occurrence[0].naval_bases.as_mut().unwrap()[1]
        .countries
        .reverse();
    assert_eq!(run(&row, &wrong_occurrence).differences.len(), 1);
}

#[test]
fn resolved_base_resources_compare_repeated_initial_blocks_and_native_ordinals() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut content = saved::BaseResources::default();
    content.add_state_history(&history_fixture(b"state={id=949 resources={steel=0 oil=-2147483648.9} history={resources={steel=999 aluminium=888}} resources={} resources={aluminium=10.000}}"))
        .unwrap();
    let names = BTreeMap::from([
        (91, b"steel".as_slice()),
        (92, b"oil".as_slice()),
        (93, b"aluminium".as_slice()),
    ]);
    let rows = [
        replay::rows::StateBaseResourceRow {
            state: 949,
            resource: 91,
            ordinal: 0,
            amount: 0,
        },
        replay::rows::StateBaseResourceRow {
            state: 949,
            resource: 92,
            ordinal: 1,
            amount: -214_748_364_800_000,
        },
        replay::rows::StateBaseResourceRow {
            state: 949,
            resource: 93,
            ordinal: 2,
            amount: 1_000_000,
        },
    ];
    let mut report = Report::default();
    base_resources(&rows, &content, &names, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        report.compared_tables["StateBaseResources (verified content)"],
        3
    );

    let mut changed = rows.clone();
    changed[0].amount = 1;
    changed[1].amount += 1;
    changed[2].ordinal = 0;
    let mut report = Report::default();
    base_resources(&changed, &content, &names, &mut report).unwrap();
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);

    let mut wrong_names = names.clone();
    wrong_names.insert(92, b"chromium".as_slice());
    let mut report = Report::default();
    base_resources(&rows, &content, &wrong_names, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert_eq!(
        report.compared_tables["StateBaseResources (verified content)"],
        2
    );
    assert!(
        base_resources(
            &[rows[0].clone(), rows[0].clone()],
            &content,
            &names,
            &mut Report::default(),
        )
        .is_err()
    );
}

#[test]
fn resolved_construction_names_retain_exact_numeric_and_queue_checks() {
    let saved = saved_pack(b"countries={GER={production={general_lines={building={id={id=4294967295 type=59} cost=0 speed=-0.00001 produced=-90071992547.40993 active_factories=-2 priority=-1 amount=0 created_date=1936.1.1.12 building={template=factory location=3} to_repair=1}}}}} states={} provinces={}");
    let germany = CountryRow {
        tag: b"GER".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(12, &germany)]);
    let names = BTreeMap::from([(91, b"factory".as_slice())]);
    let row = replay::rows::BuildingConstructionRow {
        id: (u64::from(u32::MAX) << 32) | 59,
        country: 12,
        active_factories: -2,
        speed: -1,
        produced: -9_007_199_254_740_993,
        priority: -1,
        created_hour: installed_content::save::GameDate::parse(b"1936.1.1.12")
            .unwrap()
            .engine_hour()
            .unwrap(),
        building_type: 91,
        target_flags: 2,
        target_location: u32::MAX,
        target_state: 3,
        target_country: u32::MAX,
        repair_levels: 1,
        ..Default::default()
    };
    let mut report = Report::default();
    building_construction(
        std::slice::from_ref(&row),
        &saved,
        &links,
        &names,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["BuildingConstruction"], 1);

    let mut changed = row.clone();
    changed.cost = 1;
    changed.produced += 1;
    changed.queue_ordinal = 1;
    let mut report = Report::default();
    building_construction(&[changed], &saved, &links, &names, &mut report).unwrap();
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);

    let mut report = Report::default();
    building_construction(
        std::slice::from_ref(&row),
        &saved,
        &links,
        &BTreeMap::from([(91, b"port".as_slice())]),
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("building_type"));
    assert!(
        building_construction(
            &[row.clone(), row],
            &saved,
            &links,
            &names,
            &mut Report::default(),
        )
        .is_err()
    );
}

#[test]
fn missing_construction_name_does_not_skip_numeric_or_target_comparison() {
    let saved = saved_pack(b"countries={GER={production={general_lines={building={id={id=4 type=59} cost=90071992547.40993 priority=-2 amount=1 created_date=1936.1.1.12 building={template=factory location=3}}}}}} states={} provinces={}");
    let germany = CountryRow {
        id: 12,
        canonical_slot: 3,
        tag: b"GER".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(12, &germany)]);
    let row = replay::rows::BuildingConstructionRow {
        id: (4_u64 << 32) | 59,
        country: 12,
        cost: 9_007_199_254_740_994,
        priority: -2,
        amount: 1,
        created_hour: installed_content::save::GameDate::parse(b"1936.1.1.12")
            .unwrap()
            .engine_hour()
            .unwrap(),
        building_type: 91,
        target_flags: 2,
        target_location: 3,
        target_state: 4,
        ..Default::default()
    };
    let mut report = Report::default();
    building_construction(&[row], &saved, &links, &BTreeMap::new(), &mut report).unwrap();
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("cost"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("target_location"))
    );
    assert_eq!(report.compared_tables["BuildingConstruction"], 1);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("construction building type is absent"))
    );
}

#[test]
fn manpower_filter_retains_relative_order_and_resolves_country_aliases() {
    let germany = CountryRow {
        id: 12,
        canonical_slot: 3,
        tag: b"GER".to_vec(),
        ..Default::default()
    };
    let france = CountryRow {
        id: 22,
        canonical_slot: 4,
        tag: b"FRA".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(12, &germany), (112, &germany), (22, &france)]);
    let saved = [
        saved::Manpower {
            source_country: b"GER".to_vec(),
            amount: 9,
        },
        saved::Manpower {
            source_country: b"FRA".to_vec(),
            amount: 7,
        },
    ];
    let mut report = Report::default();
    manpower(
        &mut report,
        "reserve",
        [(9, 22, 7), (1, u32::MAX, -2), (4, 112, 9)],
        &saved,
        &links,
    )
    .unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.limitations.values().map(Vec::len).sum::<usize>(), 1);
    manpower(
        &mut report,
        "reserve",
        [(0, 22, 7), (1, 12, 9)],
        &saved,
        &links,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(
        manpower(
            &mut report,
            "duplicate",
            [(0, 12, 7), (0, 12, 9)],
            &saved,
            &links
        )
        .is_err()
    );
    assert!(manpower(&mut report, "unresolved", [(0, 999, 7)], &saved, &links).is_err());
}

#[test]
fn occupation_group_laws_compare_null_membership_aliases_and_missing_or_extra_groups() {
    use replay::rows::{OccupationGroupLaw, OccupationSettingsRow, Recorded};
    let saved = saved_pack(b"countries={A={occupation_status={occupation={B={occupation_law=mod_law} C={}}}} B={} C={}} states={} provinces={}");
    let a = CountryRow {
        tag: b"A".to_vec(),
        ..Default::default()
    };
    let b = CountryRow {
        tag: b"B".to_vec(),
        ..Default::default()
    };
    let c = CountryRow {
        tag: b"C".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(1, &a), (2, &b), (3, &c), (9002, &b)]);
    let groups = vec![
        OccupationGroupLaw {
            occupied: 9002,
            scope_country: Some(9002),
            law: Some(b"mod_law".to_vec()),
        },
        OccupationGroupLaw {
            occupied: 3,
            scope_country: Some(3),
            law: None,
        },
    ];
    let mut row = OccupationSettingsRow {
        country: 1,
        group_laws: Recorded::Value(Some(groups)),
        ..Default::default()
    };
    let mut report = Report::default();
    occupation_settings(&[row.clone()], &saved, &links, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    for groups in [
        vec![
            OccupationGroupLaw {
                occupied: 2,
                scope_country: Some(2),
                law: Some(b"wrong".to_vec()),
            },
            OccupationGroupLaw {
                occupied: 3,
                scope_country: Some(3),
                law: None,
            },
        ],
        vec![OccupationGroupLaw {
            occupied: 2,
            scope_country: Some(2),
            law: Some(b"mod_law".to_vec()),
        }],
        vec![
            OccupationGroupLaw {
                occupied: 2,
                scope_country: Some(2),
                law: Some(b"mod_law".to_vec()),
            },
            OccupationGroupLaw {
                occupied: 3,
                scope_country: Some(3),
                law: None,
            },
            OccupationGroupLaw {
                occupied: 1,
                scope_country: Some(1),
                law: None,
            },
        ],
    ] {
        row.group_laws = Recorded::Value(Some(groups));
        let mut report = Report::default();
        occupation_settings(&[row.clone()], &saved, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("group_laws"));
    }
}
