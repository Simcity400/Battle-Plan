use super::*;

type OccupationMapFixture = (
    saved::Pack,
    BTreeMap<u32, content_reader_save::State>,
    [CountryRow; 3],
    replay_data::rows::StateResistanceRow,
    replay_data::rows::StateRow,
);

fn occupation_map_fixture() -> OccupationMapFixture {
    use replay_data::rows::{OccupationMap, Recorded, StateResistanceRow, StateRow};
    let saved = saved_pack(br#"countries={AAA={occupation_status={occupation={CCC={resistance_modifiers={"core_r" "core_r"} compliance_modifiers={"core_c"}}}}} BBB={} CCC={occupation_status={}}} states={7={owner=AAA resistance={occupied_country_tag=BBB resistance=1 resistance_target=2 resistance_modifiers={"state_r" "" "state_r"} compliance_modifiers={"state_c"}}}} provinces={}"#);
    let (_, mut ordinary, _, _) = recorder_oracle_support::fixtures::fixture();
    let mut state = ordinary.states.remove(&1).unwrap();
    state.id = 7;
    state.owner = Some(b"AAA".to_vec());
    state.controller = None;
    let countries = [(1, b"AAA"), (2, b"BBB"), (3, b"CCC")].map(|(id, tag)| CountryRow {
        id,
        tag: tag.to_vec(),
        ..Default::default()
    });
    let row = StateResistanceRow {
        state: 7,
        country: 2,
        level: 100_000,
        active: 0,
        map: Recorded::Value(Some(OccupationMap {
            resistance: vec![
                b"state_r\0trailing".to_vec(),
                Vec::new(),
                b"state_r".to_vec(),
                b"core_r".to_vec(),
                b"core_r".to_vec(),
            ],
            compliance: vec![b"state_c".to_vec(), b"core_c".to_vec()],
            core_country: Some(3),
            stored_target: None,
        })),
        ..Default::default()
    };
    let captured_state = StateRow {
        id: 7,
        owner: 1,
        controller: 1,
        ..Default::default()
    };
    (
        saved,
        BTreeMap::from([(7, state)]),
        countries,
        row,
        captured_state,
    )
}

#[test]
fn occupation_map_compares_inactive_state_then_core_occurrences_and_retained_country_key() {
    let (saved, states, countries, row, state) = occupation_map_fixture();
    let links = countries
        .iter()
        .map(|country| (country.id, country.tag.as_slice()))
        .collect();
    let mut report = Report::default();
    resistance_map(
        &[row],
        &saved,
        &states,
        &[state],
        &links,
        &links,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    for named in ["first NUL", "embedded lookup link", "other saved targets"] {
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains(named)),
            "{named}"
        );
    }
}

#[test]
fn occupation_map_saved_identity_order_duplicates_and_each_family_are_exact() {
    use replay_data::rows::Recorded;
    let (saved, states, countries, row, state) = occupation_map_fixture();
    let links = countries
        .iter()
        .map(|country| (country.id, country.tag.as_slice()))
        .collect();
    for mutation in 0..4 {
        let mut changed = row.clone();
        let Recorded::Value(Some(map)) = &mut changed.map else {
            unreachable!()
        };
        match mutation {
            0 => map.resistance.swap(0, 3),
            1 => {
                map.resistance.remove(3);
            }
            2 => map.resistance[3] = b"other_identity".to_vec(),
            _ => map.compliance.swap(0, 1),
        }
        let mut report = Report::default();
        resistance_map(
            &[changed],
            &saved,
            &states,
            std::slice::from_ref(&state),
            &links,
            &links,
            &mut report,
        )
        .unwrap();
        assert_eq!(
            report.differences.len(),
            1,
            "{mutation}: {:?}",
            report.differences
        );
    }
}

#[test]
fn occupation_map_demands_acquisition_including_inactive_and_nonpositive_states() {
    use replay_data::rows::Recorded;
    let (saved, states, countries, mut row, state) = occupation_map_fixture();
    let links = countries
        .iter()
        .map(|country| (country.id, country.tag.as_slice()))
        .collect();
    for missing in [Recorded::NotRecorded, Recorded::Value(None)] {
        row.map = missing;
        assert!(
            resistance_map(
                std::slice::from_ref(&row),
                &saved,
                &states,
                std::slice::from_ref(&state),
                &links,
                &links,
                &mut Report::default()
            )
            .is_err()
        );
    }
    row.country = u32::MAX;
    let mut report = Report::default();
    resistance_map(
        std::slice::from_ref(&row),
        &saved,
        &states,
        std::slice::from_ref(&state),
        &links,
        &links,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty());
    row.map = Recorded::NotRecorded;
    assert!(
        resistance_map(
            &[row],
            &saved,
            &states,
            &[state],
            &links,
            &links,
            &mut Report::default()
        )
        .is_err()
    );
}

#[test]
fn occupation_map_zero_level_target_compares_saved_exact_words_including_omitted_zero() {
    use replay_data::rows::Recorded;
    let (mut saved, states, countries, mut row, state) = occupation_map_fixture();
    let links = countries
        .iter()
        .map(|country| (country.id, country.tag.as_slice()))
        .collect();
    row.level = 0;
    for target in [0, -9_007_199_254_740_993] {
        saved.state_resistance.get_mut(&7).unwrap().target.raw = target;
        let Recorded::Value(Some(map)) = &mut row.map else {
            unreachable!()
        };
        map.stored_target = Some(target);
        let mut report = Report::default();
        resistance_map(
            std::slice::from_ref(&row),
            &saved,
            &states,
            std::slice::from_ref(&state),
            &links,
            &links,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        let Recorded::Value(Some(map)) = &mut row.map else {
            unreachable!()
        };
        map.stored_target = Some(target + 1);
        let mut report = Report::default();
        resistance_map(
            std::slice::from_ref(&row),
            &saved,
            &states,
            std::slice::from_ref(&state),
            &links,
            &links,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("stored_target"));
    }
    let Recorded::Value(Some(map)) = &mut row.map else {
        unreachable!()
    };
    map.stored_target = None;
    let mut report = Report::default();
    resistance_map(
        &[row],
        &saved,
        &states,
        &[state],
        &links,
        &links,
        &mut report,
    )
    .unwrap();
    assert!(!report.differences.is_empty());
}

#[test]
fn occupation_map_omitted_owner_uses_named_controller_dependency_and_refuses_missing_states() {
    let (saved, mut states, countries, row, mut state) = occupation_map_fixture();
    let links = countries
        .iter()
        .map(|country| (country.id, country.tag.as_slice()))
        .collect();
    states.get_mut(&7).unwrap().owner = None;
    let mut report = Report::default();
    resistance_map(
        std::slice::from_ref(&row),
        &saved,
        &states,
        std::slice::from_ref(&state),
        &links,
        &links,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("captured state controller"))
    );
    assert!(
        resistance_map(
            std::slice::from_ref(&row),
            &saved,
            &states,
            &[],
            &links,
            &links,
            &mut Report::default()
        )
        .is_err()
    );
    state.controller = 3;
    assert!(
        resistance_map(
            &[row],
            &saved,
            &states,
            &[state],
            &links,
            &links,
            &mut Report::default()
        )
        .is_err()
    );
}

#[test]
fn occupation_map_missing_saved_evidence_never_certifies_an_empty_core_list() {
    let (mut saved, mut states, countries, row, state) = occupation_map_fixture();
    let links = countries
        .iter()
        .map(|country| (country.id, country.tag.as_slice()))
        .collect();
    saved.occupation_definitions.clear();
    assert!(
        resistance_map(
            std::slice::from_ref(&row),
            &saved,
            &states,
            std::slice::from_ref(&state),
            &links,
            &links,
            &mut Report::default()
        )
        .is_err()
    );
    saved.occupation_settings.clear();
    assert!(
        resistance_map(
            std::slice::from_ref(&row),
            &saved,
            &states,
            std::slice::from_ref(&state),
            &links,
            &links,
            &mut Report::default()
        )
        .is_err()
    );
    states.clear();
    assert!(
        resistance_map(
            &[row],
            &saved,
            &states,
            &[state],
            &links,
            &links,
            &mut Report::default()
        )
        .is_err()
    );
}

#[test]
fn occupation_map_empty_group_retains_its_saved_key_and_absence_stays_separate() {
    use replay_data::rows::Recorded;
    let (mut saved, states, countries, mut row, state) = occupation_map_fixture();
    let links = countries
        .iter()
        .map(|country| (country.id, country.tag.as_slice()))
        .collect();
    *saved
        .occupation_definitions
        .get_mut(&(b"AAA".to_vec(), b"CCC".to_vec()))
        .unwrap() = saved::ActiveOccupationDefinitions::default();
    let Recorded::Value(Some(map)) = &mut row.map else {
        unreachable!()
    };
    map.resistance.truncate(3);
    map.compliance.truncate(1);
    let mut report = Report::default();
    resistance_map(
        std::slice::from_ref(&row),
        &saved,
        &states,
        std::slice::from_ref(&state),
        &links,
        &links,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty());
    let Recorded::Value(Some(map)) = &mut row.map else {
        unreachable!()
    };
    map.core_country = Some(2);
    assert!(
        resistance_map(
            std::slice::from_ref(&row),
            &saved,
            &states,
            std::slice::from_ref(&state),
            &links,
            &links,
            &mut Report::default()
        )
        .is_err()
    );
    let Recorded::Value(Some(map)) = &mut row.map else {
        unreachable!()
    };
    map.core_country = None;
    saved.occupation_definitions.clear();
    saved
        .occupation_settings
        .get_mut(b"AAA".as_slice())
        .unwrap()
        .occupied
        .clear();
    let mut report = Report::default();
    resistance_map(
        &[row],
        &saved,
        &states,
        &[state],
        &links,
        &links,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("native group absence"))
    );
}

#[test]
fn occupation_map_reserved_group_key_requires_its_recorded_tag_and_preserves_zero_identity() {
    use replay_data::rows::Recorded;
    let (mut saved, states, countries, mut row, state) = occupation_map_fixture();
    let mut tags: BTreeMap<_, _> = countries
        .iter()
        .map(|country| (country.id, country.tag.as_slice()))
        .collect();
    let definitions = saved
        .occupation_definitions
        .remove(&(b"AAA".to_vec(), b"CCC".to_vec()))
        .unwrap();
    saved
        .occupation_definitions
        .insert((b"AAA".to_vec(), b"reserved_fixture".to_vec()), definitions);
    saved
        .occupation_settings
        .get_mut(b"AAA".as_slice())
        .unwrap()
        .occupied = vec![b"reserved_fixture".to_vec()];
    let Recorded::Value(Some(map)) = &mut row.map else {
        unreachable!()
    };
    map.core_country = Some(0);
    assert!(
        resistance_map(
            std::slice::from_ref(&row),
            &saved,
            &states,
            std::slice::from_ref(&state),
            &tags,
            &tags,
            &mut Report::default()
        )
        .is_err()
    );
    tags.insert(0, b"reserved_fixture\0trailing");
    let mut report = Report::default();
    resistance_map(&[row], &saved, &states, &[state], &tags, &tags, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("first NUL"))
    );
}

#[test]
fn occupation_resistance_hover_compares_additions_and_exile_without_certifying_runtime_caches() {
    use replay_data::rows::{
        OccupationAddedTarget, OccupationHover, OccupationModifierSource, Recorded,
        StateResistanceRow,
    };
    let text = b"countries={AAA={} BBB={diplomacy={hosting_our_government_in_exile=AAA legitimacy=75}}} states={7={resistance={added_resistance_targets={{amount=-1.23456 controller=AAA tooltip=fixture}}}}} provinces={}";
    let saved = saved_pack(text);
    let intel =
        content_reader_save::intel::parse(&content_reader_parser::parser::parse(text).unwrap().items).unwrap();
    let a = CountryRow {
        id: 10,
        tag: b"AAA".to_vec(),
        ..Default::default()
    };
    let b = CountryRow {
        id: 11,
        tag: b"BBB".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(10, &a), (11, &b)]);
    let sources = [4, 4, 4, 4, 3, 3]
        .into_iter()
        .enumerate()
        .flat_map(|(section, count)| {
            (0..count).map(move |source| OccupationModifierSource {
                section: section as u8,
                source,
                key: 100 + source as u32,
                total: Some(if matches!(section, 1 | 2) { 123 } else { 0 }),
                nodes: if matches!(section, 1 | 2) {
                    vec![replay_data::rows::ModifierBreakdownNode {
                        children: 0,
                        value: 123,
                        name: b"rate source".to_vec(),
                    }]
                } else {
                    Vec::new()
                },
                ..Default::default()
            })
        })
        .collect();
    let mut row = StateResistanceRow {
        state: 7,
        country: 11,
        active: 1,
        hover: Recorded::Value(Some(OccupationHover {
            exile_host: 10,
            legitimacy: 7_500_000,
            added: vec![OccupationAddedTarget {
                value: -123_456,
                controller: 10,
                occupied: -1,
                localisation: b"fixture\0retained".to_vec(),
            }],
            ..Default::default()
        })),
        modifier_sources: Recorded::Value(sources),
        ..Default::default()
    };
    let mut report = Report::default();
    resistance_hover(&[row.clone()], &saved, &intel, &links, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("runtime-only"))
    );
    for section in [1, 2] {
        let mut refused = row.clone();
        let Recorded::Value(sources) = &mut refused.modifier_sources else {
            unreachable!()
        };
        let source = sources
            .iter_mut()
            .find(|source| source.section == section)
            .unwrap();
        source.nodes.clear();
        source.failure = b"rate tree: exact branch unavailable".to_vec();
        assert!(
            resistance_hover(&[refused], &saved, &intel, &links, &mut Report::default()).is_err()
        );
    }
    let mut missing_total = row.clone();
    let Recorded::Value(sources) = &mut missing_total.modifier_sources else {
        unreachable!()
    };
    sources
        .iter_mut()
        .find(|source| source.section == 2)
        .unwrap()
        .total = None;
    assert!(
        resistance_hover(
            &[missing_total],
            &saved,
            &intel,
            &links,
            &mut Report::default()
        )
        .is_err()
    );
    let Recorded::Value(Some(hover)) = &mut row.hover else {
        unreachable!()
    };
    hover.added[0].value += 1;
    hover.legitimacy -= 1;
    resistance_hover(&[row], &saved, &intel, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
}

fn history_fixture(input: &[u8]) -> content_reader_states::states::StateDefinition {
    let context = content_reader_states::states::Context {
        resource: &|_| content_reader_states::states::Resource::Found,
        max_building_levels: 20,
        province_count: 1000,
        names: &content_reader_script::key_names::KeyNames::default(),
    };
    content_reader_states::states::parse_states(input, &context)
        .unwrap()
        .0
        .remove(0)
}

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
                    content_reader_save::Country {
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
            .map(|&owner| replay_data::rows::SliceReadRow {
                table: replay_data::rows::TableId::StateCores.index() as u32,
                owner,
                sample_hour: 10,
            })
            .collect::<Vec<_>>()
    };
    let row = |state, cores: &[u32]| replay_data::rows::StateCoreRow {
        state,
        cores: cores.to_vec(),
        ..Default::default()
    };
    let run = |rows: &[replay_data::rows::StateCoreRow], countries, read: &[u32]| {
        let mut report = Report::default();
        state_cores(rows, countries, &links, &reads(read), 10, &mut report).unwrap();
        report
    };
    let countries = saved(&[(b"GER", &[1, 2]), (b"FRA", &[1])]);
    let report = run(
        &[row(1, &[7, 9]), row(2, &[5]), row(3, &[])],
        &countries,
        &[1, 2, 3],
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["StateCores"], 3);
    let report = run(&[row(1, &[5]), row(2, &[])], &countries, &[1]);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    let report = run(&[row(1, &[5, 7])], &countries, &[1, 2]);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
}

#[test]
fn state_claims_compare_canonical_membership_with_the_saved_country_lists() {
    use replay_data::{rows::Recorded, rows::StateCoreRow};
    let country = |tag: &[u8], id| CountryRow {
        id,
        tag: tag.to_vec(),
        ..Default::default()
    };
    let (ger, fra) = (country(b"GER", 5), country(b"FRA", 7));
    let links = BTreeMap::from([(5_u32, &ger), (7, &fra)]);
    let countries = [(b"GER", vec![1_u32, 2]), (b"FRA", vec![])]
        .into_iter()
        .map(|(tag, claims)| {
            (
                tag.to_vec(),
                content_reader_save::Country {
                    tag: tag.to_vec(),
                    claims,
                    ..Default::default()
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let reads = [1_u32, 2].map(|owner| replay_data::rows::SliceReadRow {
        table: replay_data::rows::TableId::StateCores.index() as u32,
        owner,
        sample_hour: 10,
    });
    let row = |state, claims: Recorded<Vec<u32>>| StateCoreRow {
        state,
        claims,
        ..Default::default()
    };
    let run = |rows: &[StateCoreRow]| {
        let mut report = Report::default();
        state_cores(rows, &countries, &links, &reads, 10, &mut report).unwrap();
        report
    };
    let report = run(&[row(1, vec![5].into()), row(2, vec![5].into())]);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let report = run(&[row(1, vec![5, 7].into()), row(2, vec![].into())]);
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert!(report.differences.iter().all(|d| d.contains("claims")));
    let report = run(&[row(1, Recorded::NotRecorded), row(2, Recorded::NotRecorded)]);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
}

#[test]
fn state_strategic_locations_compare_with_the_saved_state_and_province_lists() {
    use replay_data::{rows::Recorded, rows::StateCoreRow};
    let names = BTreeMap::from([
        (701_u32, &b"natural_harbor"[..]),
        (702, b"favorable_approach"),
    ]);
    let state = |locations: Vec<(&[u8], u32)>| content_reader_save::State {
        id: 4,
        owner: None,
        controller: None,
        demilitarized: None,
        is_border_conflict: None,
        manpower_available: None,
        manpower_locked: None,
        population: None,
        category: None,
        name: None,
        strategic_locations: locations
            .into_iter()
            .map(|(name, province)| (name.to_vec(), province))
            .collect(),
        contested_owners: Vec::new(),
    };
    let province = |id, tokens: &[&[u8]]| content_reader_save::ProvinceOverride {
        id,
        name: None,
        owner: None,
        controller: None,
        victory_points: None,
        strategic_locations: tokens.iter().map(|token| token.to_vec()).collect(),
    };
    let states = BTreeMap::from([(
        4,
        state(vec![(b"natural_harbor", 31), (b"favorable_approach", 32)]),
    )]);
    let provinces = BTreeMap::from([
        (31, province(31, &[b"favorable_approach"])),
        (32, province(32, &[])),
    ]);
    let state_of = BTreeMap::from([(31, 4), (32, 4)]);
    let reads = [replay_data::rows::SliceReadRow {
        table: replay_data::rows::TableId::StateCores.index() as u32,
        owner: 4,
        sample_hour: 10,
    }];
    let row = |pairs: Vec<u64>, entries: Vec<u64>| StateCoreRow {
        state: 4,
        strategic_locations: Recorded::Value(pairs),
        province_strategic_locations: Recorded::Value(entries),
        ..Default::default()
    };
    let run = |row: StateCoreRow| {
        let mut report = Report::default();
        state_strategic_locations(
            &[row],
            &states,
            &provinces,
            &state_of,
            &names,
            &reads,
            10,
            &mut report,
        )
        .unwrap();
        report.differences
    };
    let pair = |token: u64, province: u64| token << 32 | province;
    assert!(run(row(vec![pair(701, 31), pair(702, 32)], vec![pair(31, 702)])).is_empty());
    let differences = run(row(vec![pair(702, 32), pair(701, 31)], Vec::new()));
    assert_eq!(differences.len(), 2, "{differences:?}");
    let mut report = Report::default();
    assert!(
        state_strategic_locations(
            &[row(vec![pair(799, 31)], Vec::new())],
            &states,
            &provinces,
            &state_of,
            &names,
            &reads,
            10,
            &mut report
        )
        .is_err()
    );
}

fn saved_pack(text: &[u8]) -> saved::Pack {
    saved::parse(&content_reader_parser::parser::parse(text).unwrap().items).unwrap()
}

#[test]
fn occupation_law_names_compare_native_first_nul_prefixes() {
    use replay_data::{
        rows::OccupationGroupLaw, rows::OccupationSettingsRow, rows::Recorded,
        rows::StateOccupationLawRow,
    };
    let saved = saved_pack(b"countries={AAA={occupation_status={default_law=Law occupation={BBB={occupation_law=Law occupation_law_list={1=Law}}}}} BBB={}} states={} provinces={}");
    let countries = [b"AAA", b"BBB"].map(|tag| CountryRow {
        tag: tag.to_vec(),
        ..Default::default()
    });
    let links = BTreeMap::from([(1, &countries[0]), (2, &countries[1])]);
    let tags = BTreeMap::from([(1, b"AAA".as_slice()), (2, b"BBB".as_slice())]);
    let row = OccupationSettingsRow {
        country: 1,
        default_law_present: 1,
        law: b"Law\0tail".to_vec(),
        occupied: Recorded::Value(Some(vec![2])),
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
    occupation_settings(
        std::slice::from_ref(&row),
        &saved,
        &links,
        &tags,
        &mut report,
    )
    .unwrap();
    occupation_laws(
        std::slice::from_ref(&state),
        &saved,
        &links,
        Some(std::slice::from_ref(&row)),
        &tags,
        &mut report,
    )
    .unwrap();
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
    occupation_settings(
        std::slice::from_ref(&changed),
        &saved,
        &links,
        &tags,
        &mut report,
    )
    .unwrap();
    occupation_laws(
        &[state],
        &saved,
        &links,
        Some(&[changed]),
        &tags,
        &mut report,
    )
    .unwrap();
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
        occupation_settings(&[empty], &saved_empty, &links, &tags, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
    }
}

#[test]
fn garrison_equipment_oracle_compares_requirements_and_names_the_unsaved_allocation_cache() {
    use replay_data::{
        rows::GarrisonEquipment, rows::GarrisonEquipmentInputs, rows::Recorded,
        rows::StateGarrisonRow, rows::TableRows, rows::VariantDefinitionRow,
    };
    use replay_frame::frame::TableView;
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
        replay_data::rows::StateResourceRow {
            state: 2,
            resource: 7,
            pool: 800_000,
            ..Default::default()
        },
        replay_data::rows::StateResourceRow {
            state: 2,
            resource: 8,
            pool: -200_000,
            ..Default::default()
        },
        replay_data::rows::StateResourceRow {
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
fn an_unnamed_state_resource_is_counted_not_reported_missing() {
    use replay_data::rows::StateResourceRow;
    let saved = saved_pack(
        b"countries={} provinces={} states={2={resources={ore=8 coolant=-2}} 3={resources={coolant=5}}}",
    );
    let names = BTreeMap::from([(8, b"coolant".as_slice())]);
    let pool = |state, resource, pool| StateResourceRow {
        state,
        resource,
        pool,
        ..Default::default()
    };
    let run = |rows: &[StateResourceRow]| {
        let mut report = Report::default();
        state_pools(rows, &saved.state_resources, &names, &|_| true, &mut report).unwrap();
        report
    };
    let report = run(&[
        pool(2, 7, 800_000),
        pool(2, 8, -200_000),
        pool(3, 8, 500_000),
    ]);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["StateResources.pool"], 2);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("its pool cannot be matched by name"))
    );
    let report = run(&[pool(2, 7, 800_000)]);
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("state resources.2.row_count"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("state resources.(3,"))
    );
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
    let pool = |resource, pool| replay_data::rows::StateResourceRow {
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
    let modifier =
        |ordinal, token: &[u8], scope_country| replay_data::rows::StateDynamicModifierRow {
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
    assert_eq!(report.differences.len(), 3);
}

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
        engine_native::cbuilding_database::INFRASTRUCTURE_TOKEN,
        &b"infra"[..],
    )]);
    let states: Vec<_> = [2, 3, 4]
        .map(|id| replay_data::rows::StateRow {
            id,
            controller: 5,
            ..Default::default()
        })
        .into();
    let input = |state, origin, infrastructure_level| replay_data::rows::StateResourceInputRow {
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
    let earlier = [input(2, 0, None), input(3, 0, None), input(4, 1, None)];
    let report = run(&earlier, &|state| state != 2);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["StateResourceInputs"], 2);
    let report = run(&exact[..2], &|_| true);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("StateResourceInputs.4"));
}

#[test]
fn resource_inputs_compare_null_controller_without_a_country_join() {
    let saved = saved_pack(
        b"countries={A={resources={origin={country=A state=2}}}} states={2={}} provinces={}",
    );
    let states = [replay_data::rows::StateRow {
        id: 2,
        controller: 0,
        ..Default::default()
    }];
    let names = BTreeMap::from([(
        engine_native::cbuilding_database::INFRASTRUCTURE_TOKEN,
        b"infra".as_slice(),
    )]);
    let mut row = replay_data::rows::StateResourceInputRow {
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
        .map(|(id, token)| replay_data::rows::ResourceDefinitionRow {
            id,
            token,
            ..Default::default()
        })
        .into();
    let states: Vec<_> = [(2, 5), (3, 24)]
        .map(|(id, controller)| replay_data::rows::StateRow {
            id,
            controller,
            ..Default::default()
        })
        .into();
    let hour = 5;
    let row = |ordinal, entry, building, count, resource, amount| {
        replay_data::rows::CountryBuildingResourceRow {
            country: 5,
            ordinal,
            entry,
            building,
            count,
            resource,
            amount,
        }
    };
    let read = |owner, sample_hour| replay_data::rows::SliceReadRow {
        table: replay_data::rows::TableId::CountryBuildingResources.index() as u32,
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
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    let report = run(&[], &[read(5, hour), read(24, 0)], 0);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    let report = run(&[], &[read(5, hour)], 0);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("no SliceReads hour"))
    );
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
    let building = replay_data::rows::BuildingRow {
        location: 1,
        building_type: 31,
        level: i16::MIN,
        healthy_level: -2,
        partial_health: 1,
        ..Default::default()
    };
    let mut rail = replay_data::rows::RailwayConstructionRow {
        id: (2_u64 << 32) | 4713,
        country: 12,
        cost: 9_007_199_254_740_993,
        priority: -1,
        amount: 1,
        created_hour: content_reader_save::GameDate::parse(b"1936.1.1.12")
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
    let tags = BTreeMap::from([(12, b"GER".as_slice()), (22, b"FRA".as_slice())]);
    let mut rows = [
        replay_data::rows::OccupationSettingsRow {
            country: 12,
            garrison_template: Some((u64::from(u32::MAX) << 32) | 52),
            default_law_present: 1,
            law: Vec::new(),
            occupied: replay_data::rows::Recorded::Value(Some(Vec::new())),
            group_laws: replay_data::rows::Recorded::Value(Some(Vec::new())),
            ..Default::default()
        },
        replay_data::rows::OccupationSettingsRow {
            country: 22,
            occupied: replay_data::rows::Recorded::Value(Some(Vec::new())),
            group_laws: replay_data::rows::Recorded::Value(Some(Vec::new())),
            ..Default::default()
        },
    ];
    let mut report = Report::default();
    occupation_settings(&rows, &saved, &links, &tags, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);

    rows[0].garrison_template = Some((u64::from(u32::MAX - 1) << 32) | 52);
    occupation_settings(&rows, &saved, &links, &tags, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("garrison_template"));

    rows[0].garrison_template = Some((u64::from(u32::MAX) << 32) | 52);
    rows[1].garrison_template = Some((1_u64 << 32) | 52);
    let mut report = Report::default();
    occupation_settings(&rows, &saved, &links, &tags, &mut report).unwrap();
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
    let tags = BTreeMap::from([
        (1, b"AAA".as_slice()),
        (2, b"BBB".as_slice()),
        (3, b"CCC".as_slice()),
    ]);
    let mut row = replay_data::rows::OccupationSettingsRow {
        country: 1,
        occupied: replay_data::rows::Recorded::Value(Some(vec![3, 2])),
        group_laws: replay_data::rows::Recorded::Value(Some(
            [3, 2]
                .map(|occupied| replay_data::rows::OccupationGroupLaw {
                    occupied,
                    scope_country: Some(occupied),
                    law: None,
                })
                .to_vec(),
        )),
        ..Default::default()
    };
    let mut report = Report::default();
    occupation_settings(&[row.clone()], &saved, &links, &tags, &mut report).unwrap();
    assert!(report.differences.is_empty());
    row.occupied = replay_data::rows::Recorded::Value(Some(vec![2, 3]));
    occupation_settings(&[row.clone()], &saved, &links, &tags, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    row.occupied = replay_data::rows::Recorded::NotRecorded;
    let mut report = Report::default();
    occupation_settings(&[row], &saved, &links, &tags, &mut report).unwrap();
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
    let tags = BTreeMap::from([
        (12, b"GER".as_slice()),
        (22, b"FRA".as_slice()),
        (32, b"ENG".as_slice()),
    ]);
    let settings = [replay_data::rows::OccupationSettingsRow {
        country: 12,
        group_laws: replay_data::rows::Recorded::Value(Some(
            [22, 32]
                .map(|occupied| replay_data::rows::OccupationGroupLaw {
                    occupied,
                    scope_country: Some(occupied),
                    law: None,
                })
                .to_vec(),
        )),
        ..Default::default()
    }];
    let mut rows = [
        replay_data::rows::StateOccupationLawRow {
            controller: 12,
            occupied: 22,
            state: 0,
            name: b"a".to_vec(),
        },
        replay_data::rows::StateOccupationLawRow {
            controller: 12,
            occupied: 32,
            state: 0,
            name: b"b".to_vec(),
        },
    ];
    let mut report = Report::default();
    occupation_laws(&rows, &saved, &links, Some(&settings), &tags, &mut report).unwrap();
    assert!(report.differences.is_empty());
    rows[0].name = b"b".to_vec();
    occupation_laws(&rows, &saved, &links, Some(&settings), &tags, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
}

#[test]
fn resistance_country_omission_does_not_certify_raw_zero() {
    let saved = saved_pack(b"countries={} states={1={resistance={resistance=90071992547.40993 resistance_speed=-0.00001}}} provinces={}");
    let mut row = replay_data::rows::StateResistanceRow {
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
    content_reader_script::test_game::enter();
    let saved = saved_pack(b"countries={} states={} provinces={7={buildings={factory={level=1 healthy_levels=1 partial_health=0}}}}");
    let row = replay_data::rows::BuildingRow {
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
        replay_data::rows::StateBaseResourceRow {
            state: 1,
            resource: 91,
            ordinal: 77,
            amount: i64::MIN,
        },
        replay_data::rows::StateBaseResourceRow {
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
    content_reader_script::test_game::enter();
    let saved = saved_pack(b"countries={GER={production={general_lines={building={id={id=4 type=59} cost=1 priority=0 amount=1 created_date=1936.1.1.12 building={template=factory location=3}}}}}} states={} provinces={7={buildings={factory={level=1 healthy_levels=1 partial_health=0}}}}");
    let names = BTreeMap::from([
        (11, b"factory\0tail".as_slice()),
        (12, b"oil\0tail".as_slice()),
    ]);
    let mut report = Report::default();
    buildings(
        &[replay_data::rows::BuildingRow {
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
        &[replay_data::rows::StateBaseResourceRow {
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
        &[replay_data::rows::BuildingConstructionRow {
            id: (4_u64 << 32) | 59,
            country: 12,
            cost: 100_001,
            amount: 1,
            created_hour: content_reader_save::GameDate::parse(b"1936.1.1.12")
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
        replay_data::rows::BuildingRow {
            location: 7,
            building_type: 91,
            ..Default::default()
        },
        replay_data::rows::BuildingRow {
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

    let mut report = Report::default();
    buildings(
        &[replay_data::rows::BuildingRow {
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
    let country_bases = [content_reader_save::naval_launch::CountryNavalLaunchOccurrence {
        ordinal: 0,
        tag: b"TST".to_vec(),
        naval_bases: Some(
            vec![7]
                .into_iter()
                .map(content_reader_save::naval_launch::NavalBaseOccurrence::from)
                .collect(),
        ),
    }];
    let mut row = replay_data::rows::NavalBaseRow {
        province: 7,
        level: 5,
        healthy_level: 4,
        country_tags: replay_data::rows::Recorded::Value(Some(Vec::new())),
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
    let rosters = [content_reader_save::naval_launch::CountryNavalLaunchOccurrence {
        ordinal: 0,
        tag: b"TST".to_vec(),
        naval_bases: Some(
            vec![19]
                .into_iter()
                .map(content_reader_save::naval_launch::NavalBaseOccurrence::from)
                .collect(),
        ),
    }];
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
    use content_reader_save::naval_launch::parse_country_occurrences;
    use replay_data::{rows::NavalBaseRow, rows::Recorded};
    let bases = parse_country_occurrences(&content_reader_parser::parser::parse(
        b"A={strategic_navy={naval_base={7={disabled_for_repairs=\"A\" disabled_for_repairs=\"B\" disabled_for_repairs=\"A\" disabled_for_repairs=\"\"} 7={disabled_for_repairs=\"A\" disabled_for_repairs=\"B\" disabled_for_repairs=\"A\" disabled_for_repairs=\"\"}}}}",
    ).unwrap().items).unwrap();
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
    content_reader_script::test_game::enter();
    let mut content = saved::BaseResources::default();
    content.add_state_history(&history_fixture(b"state={id=949 resources={steel=0 oil=-2147483648.9} history={resources={steel=999 aluminium=888}} resources={} resources={aluminium=10.000}}"))
        .unwrap();
    let names = BTreeMap::from([
        (91, b"steel".as_slice()),
        (92, b"oil".as_slice()),
        (93, b"aluminium".as_slice()),
    ]);
    let rows = [
        replay_data::rows::StateBaseResourceRow {
            state: 949,
            resource: 91,
            ordinal: 0,
            amount: 0,
        },
        replay_data::rows::StateBaseResourceRow {
            state: 949,
            resource: 92,
            ordinal: 1,
            amount: -214_748_364_800_000,
        },
        replay_data::rows::StateBaseResourceRow {
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
    let row = replay_data::rows::BuildingConstructionRow {
        id: (u64::from(u32::MAX) << 32) | 59,
        country: 12,
        active_factories: -2,
        speed: -1,
        produced: -9_007_199_254_740_993,
        priority: -1,
        created_hour: content_reader_save::GameDate::parse(b"1936.1.1.12")
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
fn mixed_construction_queue_preserves_ordinals_and_names_unsupported_railway_gun_repairs() {
    let saved = saved_pack(
        br#"countries={GER={production={general_lines={
            building={id={id=4 type=59} cost=1 priority=0 amount=1 created_date=1936.1.1.12 building={template=factory location=3}}
            railway_gun={id={id=1 type=76} priority=1 amount=0 cost=0 created_date="1944.8.26.17" railway_gun={id=1 type=74}}
            railway_gun={id={id=2 type=76} priority=2 amount=0 cost=0 created_date="1944.8.27.1" railway_gun={id=0 type=0}}
            rail_way={id={id=2 type=4713} cost=2 priority=-1 amount=1 created_date=1936.1.1.12 base=-1 target=2 current_province=-1 remaining_hours=-0.00001 rail_way={7 4 7}}
            building={id={id=5 type=59} cost=1 priority=0 amount=1 created_date=1936.1.1.12 building={template=factory location=3} to_repair=2}
        }}}} states={} provinces={}"#,
    );
    let germany = CountryRow {
        id: 12,
        canonical_slot: 3,
        tag: b"GER".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(12, &germany)]);
    let names = BTreeMap::from([(91, b"factory".as_slice())]);
    let created_hour = content_reader_save::GameDate::parse(b"1936.1.1.12")
        .unwrap()
        .engine_hour()
        .unwrap();
    let buildings = vec![
        replay_data::rows::BuildingConstructionRow {
            id: (4_u64 << 32) | 59,
            country: 12,
            queue_ordinal: 0,
            cost: 100_000,
            amount: 1,
            created_hour,
            building_type: 91,
            target_flags: 2,
            target_state: 3,
            ..Default::default()
        },
        replay_data::rows::BuildingConstructionRow {
            id: (5_u64 << 32) | 59,
            country: 12,
            queue_ordinal: 4,
            cost: 100_000,
            amount: 1,
            created_hour,
            building_type: 91,
            target_flags: 2,
            target_state: 3,
            repair_levels: 2,
            ..Default::default()
        },
    ];
    let rails = vec![replay_data::rows::RailwayConstructionRow {
        id: (2_u64 << 32) | 4713,
        country: 12,
        queue_ordinal: 3,
        cost: 200_000,
        priority: -1,
        amount: 1,
        created_hour,
        base_level: -1,
        target_level: 2,
        current_segment: -1,
        remaining_cost: -1,
        route: vec![7, 4, 7],
        ..Default::default()
    }];
    let mut report = Report::default();
    building_construction(&buildings, &saved, &links, &names, &mut report).unwrap();
    railway_construction(&rails, &saved, &links, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["BuildingConstruction"], 2);
    assert_eq!(report.compared_tables["RailwayConstruction"], 1);
    let subjects = report
        .limitations
        .iter()
        .find(|(reason, _)| reason.contains("railway_gun"))
        .map(|(_, subjects)| subjects)
        .expect("named limitation for unsupported railway_gun repair rows");
    assert_eq!(subjects.len(), 2, "{subjects:?}");
    assert!(subjects[0].contains("GER.general_lines[1].railway_gun"), "{subjects:?}");
    assert!(subjects[0].contains(&format!("id={}", (1_u64 << 32) | 76)), "{subjects:?}");
    assert!(subjects[0].contains(&format!("target={}", (1_u64 << 32) | 74)), "{subjects:?}");
    assert!(subjects[1].contains("GER.general_lines[2].railway_gun"), "{subjects:?}");
    assert!(subjects[1].contains(&format!("id={}", (2_u64 << 32) | 76)), "{subjects:?}");
    assert!(subjects[1].contains("target=0"), "{subjects:?}");

    let mut rail_only = Report::default();
    railway_construction(&rails, &saved, &links, &mut rail_only).unwrap();
    assert!(rail_only.differences.is_empty(), "{:?}", rail_only.differences);
    assert!(
        rail_only
            .limitations
            .keys()
            .any(|reason| reason.contains("railway_gun")),
        "{:?}",
        rail_only.limitations
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
    let row = replay_data::rows::BuildingConstructionRow {
        id: (4_u64 << 32) | 59,
        country: 12,
        cost: 9_007_199_254_740_994,
        priority: -2,
        amount: 1,
        created_hour: content_reader_save::GameDate::parse(b"1936.1.1.12")
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
    use replay_data::{rows::OccupationGroupLaw, rows::OccupationSettingsRow, rows::Recorded};
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
    let tags = BTreeMap::from([
        (1, b"A".as_slice()),
        (2, b"B".as_slice()),
        (3, b"C".as_slice()),
    ]);
    let groups = vec![
        OccupationGroupLaw {
            occupied: 9002,
            scope_country: Some(2),
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
        occupied: Recorded::Value(Some(vec![2, 3])),
        group_laws: Recorded::Value(Some(groups)),
        ..Default::default()
    };
    let mut report = Report::default();
    occupation_settings(&[row.clone()], &saved, &links, &tags, &mut report).unwrap();
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
        occupation_settings(&[row.clone()], &saved, &links, &tags, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("group_laws"));
    }
}

type OccupationSavedKeyFixture = (
    saved::Pack,
    [CountryRow; 3],
    replay_data::rows::OccupationSettingsRow,
    replay_data::rows::StateOccupationLawRow,
    replay_data::rows::StateGarrisonRow,
    replay_data::rows::StateAssignedManpowerRow,
);

fn occupation_saved_key_fixture() -> OccupationSavedKeyFixture {
    use replay_data::rows::{OccupationGroupLaw, OccupationSettingsRow, Recorded};
    let saved = saved_pack(b"countries={AAA={occupation_status={default_law=default occupation={CCC={occupation_law=group occupation_law_list={7=override} state_garrison_data={7={army_manpower_need=4 strength_ratio=1 garrison_required=2 suppression=yes manpower_pool={value={tag=AAA value=5}}}}}}}} BBB={} CCC={}} states={} provinces={}");
    let countries = [(1, b"AAA"), (2, b"BBB"), (3, b"CCC")].map(|(id, tag)| CountryRow {
        id,
        tag: tag.to_vec(),
        ..Default::default()
    });
    let settings = OccupationSettingsRow {
        country: 1,
        default_law_present: 1,
        law: b"default".to_vec(),
        occupied: Recorded::Value(Some(vec![3])),
        group_laws: Recorded::Value(Some(vec![OccupationGroupLaw {
            occupied: 2,
            scope_country: Some(3),
            law: Some(b"group".to_vec()),
        }])),
        ..Default::default()
    };
    let law = replay_data::rows::StateOccupationLawRow {
        controller: 1,
        occupied: 2,
        state: 7,
        name: b"override".to_vec(),
    };
    let garrison = replay_data::rows::StateGarrisonRow {
        controller: 1,
        occupied: 2,
        state: 7,
        required_manpower: 4,
        strength_ratio: 100_000,
        garrison_required: 200_000,
        serialized_flag: 1,
        ..Default::default()
    };
    let manpower = replay_data::rows::StateAssignedManpowerRow {
        controller: 1,
        occupied: 2,
        state: 7,
        ordinal: 0,
        source_country: 1,
        amount: 5,
    };
    (saved, countries, settings, law, garrison, manpower)
}

#[test]
fn occupation_saved_keys_use_retained_country_and_keep_each_saved_value_exact() {
    use replay_data::rows::Recorded;
    let (saved, countries, mut settings, mut law, mut garrison, mut manpower) =
        occupation_saved_key_fixture();
    let links = countries
        .iter()
        .map(|country| (country.id, country))
        .collect();
    let tags = countries
        .iter()
        .map(|country| (country.id, country.tag.as_slice()))
        .collect();
    let mut report = Report {
        read_hours: Some(BTreeMap::from([
            (TableId::OccupationSettings, 3),
            (TableId::StateOccupationLaws, 3),
            (TableId::StateGarrisons, 3),
            (TableId::StateAssignedManpower, 3),
        ])),
        ..Default::default()
    };
    occupation_settings(
        std::slice::from_ref(&settings),
        &saved,
        &links,
        &tags,
        &mut report,
    )
    .unwrap();
    occupation_laws(
        std::slice::from_ref(&law),
        &saved,
        &links,
        Some(std::slice::from_ref(&settings)),
        &tags,
        &mut report,
    )
    .unwrap();
    garrisons(
        std::slice::from_ref(&garrison),
        &saved,
        &links,
        Some(std::slice::from_ref(&settings)),
        &tags,
        &mut report,
    )
    .unwrap();
    assigned_manpower(
        std::slice::from_ref(&manpower),
        &saved,
        &links,
        Some(std::slice::from_ref(&settings)),
        &tags,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        !report
            .limitations
            .keys()
            .any(|reason| reason.contains("retained occupied-country pointer is not serialized"))
    );

    let Recorded::Value(Some(groups)) = &mut settings.group_laws else {
        unreachable!()
    };
    groups[0].law = Some(b"wrong_group".to_vec());
    law.name = b"wrong_override".to_vec();
    garrison.required_manpower += 1;
    manpower.amount += 1;
    let mut report = Report::default();
    occupation_settings(
        std::slice::from_ref(&settings),
        &saved,
        &links,
        &tags,
        &mut report,
    )
    .unwrap();
    occupation_laws(
        &[law],
        &saved,
        &links,
        Some(std::slice::from_ref(&settings)),
        &tags,
        &mut report,
    )
    .unwrap();
    garrisons(
        &[garrison],
        &saved,
        &links,
        Some(std::slice::from_ref(&settings)),
        &tags,
        &mut report,
    )
    .unwrap();
    assigned_manpower(
        &[manpower],
        &saved,
        &links,
        Some(&[settings]),
        &tags,
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 4, "{:?}", report.differences);
}

#[test]
fn occupation_failed_roster_still_compares_group_and_default_laws() {
    use replay_data::rows::Recorded;
    let (saved, countries, mut settings, _, _, _) = occupation_saved_key_fixture();
    let links = countries
        .iter()
        .map(|country| (country.id, country))
        .collect();
    let tags = countries
        .iter()
        .map(|country| (country.id, country.tag.as_slice()))
        .collect();
    settings.occupied = Recorded::Value(None);
    settings.law = b"wrong_default".to_vec();
    let Recorded::Value(Some(groups)) = &mut settings.group_laws else {
        unreachable!()
    };
    groups[0].law = Some(b"wrong_group".to_vec());
    let mut report = Report::default();
    assert!(
        !report.attempt("construction OccupationSettings", |report| {
            occupation_settings(&[settings], &saved, &links, &tags, report)
        })
    );
    assert!(
        report.failed_comparisons["construction OccupationSettings"]
            .contains("roster capture failed")
    );
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert!(report.outcome().is_err());
}

#[test]
fn occupation_saved_keys_preserve_reserved_zero_and_require_its_exact_tag() {
    use replay_data::rows::Recorded;
    let (mut saved, countries, mut settings, law, garrison, manpower) =
        occupation_saved_key_fixture();
    let links = countries
        .iter()
        .map(|country| (country.id, country))
        .collect();
    let mut tags: BTreeMap<_, _> = countries
        .iter()
        .map(|country| (country.id, country.tag.as_slice()))
        .collect();
    let reserved = b"reserved_fixture".to_vec();
    let status = saved
        .occupation_settings
        .get_mut(b"AAA".as_slice())
        .unwrap();
    status.occupied = vec![reserved.clone()];
    let group_law = status.group_laws.remove(b"CCC".as_slice()).unwrap();
    status.group_laws.insert(reserved.clone(), group_law);
    let mut saved_law = saved
        .state_occupation_laws
        .remove(&(b"AAA".to_vec(), b"CCC".to_vec(), 7))
        .unwrap();
    saved_law.occupied.clone_from(&reserved);
    saved
        .state_occupation_laws
        .insert((b"AAA".to_vec(), reserved.clone(), 7), saved_law);
    saved
        .state_garrisons
        .get_mut(&(b"AAA".to_vec(), 7))
        .unwrap()
        .occupied = reserved;
    settings.occupied = Recorded::Value(Some(vec![0]));
    let Recorded::Value(Some(groups)) = &mut settings.group_laws else {
        unreachable!()
    };
    groups[0].scope_country = Some(0);
    let mut report = Report::default();
    assert!(
        !report.attempt("construction OccupationSettings", |report| {
            occupation_settings(
                std::slice::from_ref(&settings),
                &saved,
                &links,
                &tags,
                report,
            )
        })
    );
    assert!(report.outcome().is_err());
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    tags.insert(0, b"reserved_fixture\0tail");
    let mut report = Report::default();
    occupation_settings(
        std::slice::from_ref(&settings),
        &saved,
        &links,
        &tags,
        &mut report,
    )
    .unwrap();
    occupation_laws(
        &[law],
        &saved,
        &links,
        Some(std::slice::from_ref(&settings)),
        &tags,
        &mut report,
    )
    .unwrap();
    garrisons(
        &[garrison],
        &saved,
        &links,
        Some(std::slice::from_ref(&settings)),
        &tags,
        &mut report,
    )
    .unwrap();
    assigned_manpower(
        &[manpower],
        &saved,
        &links,
        Some(&[settings]),
        &tags,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("first NUL"))
    );
}

#[test]
fn occupation_saved_keys_refuse_missing_or_differently_supplied_group_witnesses() {
    use replay_data::rows::Recorded;
    let (saved, countries, settings, law, mut garrison, mut manpower) =
        occupation_saved_key_fixture();
    let links = countries
        .iter()
        .map(|country| (country.id, country))
        .collect();
    let tags = countries
        .iter()
        .map(|country| (country.id, country.tag.as_slice()))
        .collect();
    garrison.required_manpower += 1;
    manpower.amount += 1;
    for case in 0..9 {
        let mut settings = settings.clone();
        let mut report = Report::default();
        match case {
            0 => {
                let Recorded::Value(Some(groups)) = &mut settings.group_laws else {
                    unreachable!()
                };
                groups[0].scope_country = None;
            }
            1 => settings.group_laws = Recorded::NotRecorded,
            2 => settings.group_laws = Recorded::Value(None),
            3 => settings.group_laws = Recorded::Value(Some(Vec::new())),
            4 => settings.country = 99,
            5 => {}
            6 | 7 => {
                report.read_hours = Some(BTreeMap::from([
                    (TableId::OccupationSettings, 2),
                    (TableId::StateOccupationLaws, 3),
                    (TableId::StateGarrisons, 3),
                    (TableId::StateAssignedManpower, 3),
                ]));
                if case == 7 {
                    report
                        .read_hours
                        .as_mut()
                        .unwrap()
                        .remove(&TableId::OccupationSettings);
                }
            }
            8 => {
                let Recorded::Value(Some(groups)) = &mut settings.group_laws else {
                    unreachable!()
                };
                groups.push(groups[0].clone());
            }
            _ => unreachable!(),
        }
        let inventory = (case != 5).then_some(std::slice::from_ref(&settings));
        assert!(
            occupation_laws(
                std::slice::from_ref(&law),
                &saved,
                &links,
                inventory,
                &tags,
                &mut report
            )
            .is_err(),
            "case {case}"
        );
        assert!(
            garrisons(
                std::slice::from_ref(&garrison),
                &saved,
                &links,
                inventory,
                &tags,
                &mut report
            )
            .is_err(),
            "case {case}"
        );
        assert!(
            assigned_manpower(
                std::slice::from_ref(&manpower),
                &saved,
                &links,
                inventory,
                &tags,
                &mut report
            )
            .is_err(),
            "case {case}"
        );
        assert_eq!(
            report.differences.len(),
            2,
            "case {case}: {:?}",
            report.differences
        );
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("saved-key witness is unavailable"))
        );
    }
}

#[test]
fn occupation_missing_scope_still_compares_other_group_laws_default_and_state_overrides() {
    use replay_data::rows::{OccupationGroupLaw, Recorded};
    let (mut saved, countries, mut settings, missing_law, _, _) = occupation_saved_key_fixture();
    let links = countries
        .iter()
        .map(|country| (country.id, country))
        .collect();
    let tags = countries
        .iter()
        .map(|country| (country.id, country.tag.as_slice()))
        .collect();
    saved
        .occupation_settings
        .get_mut(b"AAA".as_slice())
        .unwrap()
        .group_laws
        .insert(b"BBB".to_vec(), Some(b"other".to_vec()));
    saved.state_occupation_laws.insert(
        (b"AAA".to_vec(), b"BBB".to_vec(), 8),
        saved::StateOccupationLaw {
            controller: b"AAA".to_vec(),
            occupied: b"BBB".to_vec(),
            state: 8,
            law: b"other_override".to_vec(),
        },
    );
    settings.law = b"wrong_default".to_vec();
    let Recorded::Value(Some(groups)) = &mut settings.group_laws else {
        unreachable!()
    };
    groups[0].scope_country = None;
    groups.push(OccupationGroupLaw {
        occupied: 3,
        scope_country: Some(2),
        law: Some(b"wrong_other".to_vec()),
    });
    let other_law = replay_data::rows::StateOccupationLawRow {
        controller: 1,
        occupied: 3,
        state: 8,
        name: b"wrong_override".to_vec(),
    };
    let mut report = Report::default();
    assert!(
        !report.attempt("construction OccupationSettings", |report| {
            occupation_settings(
                std::slice::from_ref(&settings),
                &saved,
                &links,
                &tags,
                report,
            )
        })
    );
    assert!(!report.attempt(
        "construction StateOccupationLaws",
        |report| occupation_laws(
            &[missing_law, other_law],
            &saved,
            &links,
            Some(&[settings]),
            &tags,
            report
        )
    ));
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    assert_eq!(report.failed_comparisons.len(), 2);
}

#[test]
fn occupation_resistance_uses_raw_saved_tag_spelling_and_keeps_independent_scalars() {
    let saved = saved_pack(b"countries={} states={7={resistance={occupied_country_tag=raw_alias resistance=1}}} provinces={}");
    let row = replay_data::rows::StateResistanceRow {
        state: 7,
        country: 9002,
        level: 100_000,
        ..Default::default()
    };
    let tags = BTreeMap::from([(9002, b"raw_alias\0tail".as_slice())]);
    let mut report = Report::default();
    resistance(std::slice::from_ref(&row), &saved, &tags, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let canonical_tags = BTreeMap::from([(9002, b"BBB".as_slice())]);
    let mut report = Report::default();
    resistance(
        std::slice::from_ref(&row),
        &saved,
        &canonical_tags,
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 1);
    let mut changed = row;
    changed.level += 1;
    let mut report = Report::default();
    assert!(
        !report.attempt("construction StateResistance", |report| resistance(
            &[changed],
            &saved,
            &BTreeMap::new(),
            report
        ))
    );
    assert_eq!(report.differences.len(), 1);
    assert!(report.outcome().is_err());
}

#[test]
fn occupation_map_separates_raw_occupied_spelling_from_resolved_controller_holder() {
    let (mut saved, mut states, countries, mut row, mut state) = occupation_map_fixture();
    states.get_mut(&7).unwrap().owner = None;
    state.controller = 9001;
    row.country = 9002;
    saved.state_resistance.get_mut(&7).unwrap().country = Some(b"raw_alias".to_vec());
    let controllers = BTreeMap::from([(9001, b"AAA".as_slice())]);
    let mut tags: BTreeMap<_, _> = countries
        .iter()
        .map(|country| (country.id, country.tag.as_slice()))
        .collect();
    tags.insert(9001, b"different_controller_spelling");
    tags.insert(9002, b"raw_alias\0tail");
    let mut report = Report::default();
    resistance_map(
        std::slice::from_ref(&row),
        &saved,
        &states,
        std::slice::from_ref(&state),
        &controllers,
        &tags,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    tags.remove(&9002);
    let error = resistance_map(
        &[row],
        &saved,
        &states,
        &[state],
        &controllers,
        &tags,
        &mut Report::default(),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("country tag 9002 was not acquired")
    );
}

#[test]
fn resource_map_compares_origins_rights_routes_and_import_colours_with_the_save() {
    use replay_data::{
        rows::ConvoyClientRow, rows::RegionalConvoyEfficiencyRow, rows::ResourceDefinitionRow,
        rows::ResourceOriginRow, rows::ResourceRightRow, rows::SliceReadRow, rows::TableId,
        rows::TradeRouteRow,
    };
    let saved = saved_pack(
        br#"countries={GER={resources={
            origin={country="GER" efficiency=1 efficiency_due_to_lost_convoys=0.7 request=3 state=119 efficiency=0.7
                delivery_route={type=2 from_state=119 to_state=64 from_port=3379 to_port=6384 naval_path={ 43 } dirty=no}
                given_resource_rights={ { steel "AFG" } }}}}
            AFG={}}
        states={} provinces={}"#,
    );
    let (ger, afg) = (
        CountryRow {
            id: 5,
            tag: b"GER".to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: 9,
            tag: b"AFG".to_vec(),
            ..Default::default()
        },
    );
    let links = BTreeMap::from([(5_u32, &ger), (9, &afg)]);
    let names = BTreeMap::from([(19_u32, &b"steel"[..])]);
    let resources = [ResourceDefinitionRow {
        id: 3,
        token: 19,
        ..Default::default()
    }];
    let origin = ResourceOriginRow {
        state: 119,
        country: 5,
        list: 0,
        efficiency: 70_000,
        bad: true,
        convoys_base: 3.into(),
        ..Default::default()
    };
    let rights = [ResourceRightRow {
        ordinal: 0,
        state: 119,
        resource: 3,
        holder: 9,
    }];
    let route = TradeRouteRow {
        country: 5,
        class: 0,
        other: 119,
        kind: 2,
        from: 3379,
        to: 6384,
        path: vec![43],
        ..Default::default()
    };
    let reads = [SliceReadRow {
        table: TableId::TradeRoutes.index() as u32,
        owner: 5,
        sample_hour: 10,
    }];
    let import = ConvoyClientRow {
        handle: 0x33,
        category: engine_resources::resource_map::IMPORT,
        owner: 9,
        other: 5,
        bad: false,
        efficiency_a: 100_000.into(),
        efficiency_b: 80_000.into(),
        convoys_base: 0.into(),
        ..Default::default()
    };
    use content_reader_save::economy::{self, ConvoyCounts};
    let decimal = |text: &[u8]| content_reader_save::Decimal::parse(text).unwrap();
    let economy = economy::Pack {
        countries: BTreeMap::from([(
            b"GER".to_vec(),
            economy::Country {
                market: None,
                fuel: None,
                experience: None,
                pools: None,
                logistics: None,
                lend_lease: Some(vec![economy::LendLease {
                    terms: None,
                    id: 0x34_u64 << 32,
                    receiver: b"AFG".to_vec(),
                    efficiency: decimal(b"0.5"),
                    efficiency_due_to_lost_convoys: decimal(b"1"),
                    request: Some(4),
                    convoys: ConvoyCounts {
                        assigned: 2,
                        required: 3,
                    },
                    delivered: content_reader_save::production::Pool {
                        equipment: Vec::new(),
                        allow_zero_entries: Some(false),
                    },
                    sunk: content_reader_save::production::Pool {
                        equipment: Vec::new(),
                        allow_zero_entries: Some(false),
                    },
                }]),
                exports: Some(vec![economy::ResourceExchange {
                    id: 0x33_u64 << 32,
                    receiver: Some(b"AFG".to_vec()),
                    resource: Some(b"steel".to_vec()),
                    efficiency: decimal(b"1"),
                    efficiency_due_to_lost_convoys: decimal(b"0.8"),
                    request: Some(0),
                    delivered: decimal(b"0"),
                    required_cic: 0,
                    lended_cic: 0,
                    convoys: ConvoyCounts::default(),
                }]),
            },
        )]),
        ..Default::default()
    };
    let lend = ConvoyClientRow {
        owner_ordinal: Some(0),
        other_ordinal: Some(0),
        handle: 0x34,
        category: engine_resources::resource_map::LEND_LEASE,
        owner: 5,
        other: 9,
        bad: true,
        convoys_assigned: 2,
        convoys_required: 3,
        efficiency_a: 50_000.into(),
        efficiency_b: 100_000.into(),
        convoys_base: 4.into(),
        ..Default::default()
    };
    let countries = BTreeMap::from([(
        b"GER".to_vec(),
        content_reader_save::Country {
            tag: b"GER".to_vec(),
            regional_convoy_efficiency: Some(BTreeMap::new()),
            ..Default::default()
        },
    )]);
    let run_with = |origins: &[ResourceOriginRow],
                    routes: &[TradeRouteRow],
                    clients: &[ConvoyClientRow],
                    regional: &[RegionalConvoyEfficiencyRow],
                    hour| {
        let mut report = Report::default();
        resource_map(
            &ResourceMapRows {
                origins,
                rights: &rights,
                clients,
                routes,
                regional,
                reads: &reads,
                resources: &resources,
            },
            &saved,
            &economy,
            &countries,
            &links,
            &names,
            80_000,
            hour,
            &mut report,
        )
        .unwrap();
        report
    };
    let run = |origins: &[ResourceOriginRow],
               routes: &[TradeRouteRow],
               clients: &[ConvoyClientRow],
               hour| {
        let clients: Vec<_> = clients.iter().cloned().chain([lend.clone()]).collect();
        run_with(
            origins,
            routes,
            &clients,
            &[RegionalConvoyEfficiencyRow {
                country: 5,
                region: 43,
                efficiency: 100_000,
            }],
            hour,
        )
    };
    let report = run(
        std::slice::from_ref(&origin),
        std::slice::from_ref(&route),
        std::slice::from_ref(&import),
        10,
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["ResourceOrigins"], 1);
    assert_eq!(report.compared_tables["ResourceRights"], 1);
    assert_eq!(report.compared_tables["TradeRoutes"], 1);
    assert_eq!(report.compared_tables["ConvoyClients"], 2);
    let mut reduced = origin.clone();
    reduced.convoys_base = 2.into();
    let mut slower = import.clone();
    slower.efficiency_b = 79_999.into();
    let mut requested = lend.clone();
    requested.convoys_base = 5.into();
    requested.efficiency_a = 100_000.into();
    let report = run_with(
        &[reduced],
        std::slice::from_ref(&route),
        &[slower, requested],
        &[RegionalConvoyEfficiencyRow {
            country: 5,
            region: 43,
            efficiency: 100_000,
        }],
        10,
    );
    let mut operands: Vec<_> = report
        .differences
        .iter()
        .filter_map(|difference| {
            [
                "ResourceOrigins.GER.own.119.convoys_base",
                "ConvoyClients.51.efficiency_b",
                "ConvoyClients.52.convoys_base",
                "ConvoyClients.52.efficiency_a",
            ]
            .into_iter()
            .find(|subject| difference.contains(subject))
        })
        .collect();
    operands.sort_unstable();
    assert_eq!(
        operands,
        [
            "ConvoyClients.51.efficiency_b",
            "ConvoyClients.52.convoys_base",
            "ConvoyClients.52.efficiency_a",
            "ResourceOrigins.GER.own.119.convoys_base",
        ],
        "{:?}",
        report.differences
    );
    let mut older_origin = origin.clone();
    older_origin.convoys_base = replay_data::rows::Recorded::NotRecorded;
    let older_clients: Vec<_> = [import.clone(), lend.clone()]
        .into_iter()
        .map(|mut client| {
            client.efficiency_a = replay_data::rows::Recorded::NotRecorded;
            client.efficiency_b = replay_data::rows::Recorded::NotRecorded;
            client.convoys_base = replay_data::rows::Recorded::NotRecorded;
            client
        })
        .collect();
    let report = run_with(
        &[older_origin],
        std::slice::from_ref(&route),
        &older_clients,
        &[RegionalConvoyEfficiencyRow {
            country: 5,
            region: 43,
            efficiency: 100_000,
        }],
        10,
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    for subject in [
        "ResourceOrigins.convoys_base: not recorded",
        "ConvoyClients.efficiency_a, efficiency_b, convoys_base: not recorded",
    ] {
        assert!(
            report
                .limitations
                .values()
                .flatten()
                .any(|limited| limited == subject),
            "{subject}: {:?}",
            report.limitations
        );
    }
    let omitted = saved_pack(
        br#"countries={GER={resources={
            origin={country="GER" efficiency=1 efficiency_due_to_lost_convoys=0.7 state=119 efficiency=0.7
                delivery_route={type=2 from_state=119 to_state=64 from_port=3379 to_port=6384 naval_path={ 43 } dirty=no}
                given_resource_rights={ { steel "AFG" } }}}}
            AFG={}}
        states={} provinces={}"#,
    );
    let mut report = Report::default();
    resource_map(
        &ResourceMapRows {
            origins: std::slice::from_ref(&origin),
            rights: &rights,
            clients: std::slice::from_ref(&lend),
            routes: std::slice::from_ref(&route),
            regional: &[RegionalConvoyEfficiencyRow {
                country: 5,
                region: 43,
                efficiency: 100_000,
            }],
            reads: &reads,
            resources: &resources,
        },
        &omitted,
        &economy,
        &countries,
        &links,
        &names,
        80_000,
        10,
        &mut report,
    )
    .unwrap();
    assert!(
        !report
            .differences
            .iter()
            .any(|difference| difference.contains("convoys_base")),
        "{:?}",
        report.differences
    );
    assert!(
        report
            .limitations
            .iter()
            .any(|(reason, subjects)| reason.contains("absent from the save")
                && subjects.iter().any(|subject| subject == "ResourceOrigins.GER.own.119.convoys_base")),
        "{:?}",
        report.limitations
    );
    let mut stray = origin.clone();
    stray.list = 2;
    let error = resource_map(
        &ResourceMapRows {
            origins: std::slice::from_ref(&stray),
            rights: &rights,
            clients: &[],
            routes: &[],
            regional: &[],
            reads: &reads,
            resources: &resources,
        },
        &saved,
        &economy,
        &countries,
        &links,
        &names,
        80_000,
        10,
        &mut Report::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("list 2"), "{error}");
    let mut reordered = origin.clone();
    reordered.ordinal = 1;
    let report = run(
        &[reordered],
        std::slice::from_ref(&route),
        std::slice::from_ref(&import),
        10,
    );
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("ResourceOrigins.GER.own.119.ordinal"))
    );
    let mut reordered_lend = lend.clone();
    reordered_lend.owner_ordinal = Some(1);
    let report = run_with(
        std::slice::from_ref(&origin),
        std::slice::from_ref(&route),
        &[import.clone(), reordered_lend],
        &[RegionalConvoyEfficiencyRow {
            country: 5,
            region: 43,
            efficiency: 100_000,
        }],
        10,
    );
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("owner_ordinal"))
    );
    let mut changed = origin.clone();
    changed.efficiency = 69_999;
    changed.bad = false;
    let mut moved = route.clone();
    moved.path = vec![44];
    let mut slow = import.clone();
    slow.bad = true;
    let report = run(&[changed], std::slice::from_ref(&moved), &[slow], 10);
    assert_eq!(report.differences.len(), 4, "{:?}", report.differences);
    let report = run(
        std::slice::from_ref(&origin),
        &[],
        std::slice::from_ref(&import),
        10,
    );
    assert!(
        report
            .differences
            .iter()
            .any(|d| d.contains("TradeRoutes.GER.origin.119"))
    );
    let report = run(
        std::slice::from_ref(&origin),
        std::slice::from_ref(&route),
        &[],
        10,
    );
    assert!(
        report
            .differences
            .iter()
            .any(|d| d.contains("ConvoyClients.51"))
    );
    let mut wrong_lend = lend.clone();
    wrong_lend.bad = false;
    wrong_lend.convoys_assigned = 0;
    wrong_lend.convoys_required = 0;
    let report = run_with(
        std::slice::from_ref(&origin),
        std::slice::from_ref(&route),
        &[import.clone(), wrong_lend],
        &[RegionalConvoyEfficiencyRow {
            country: 5,
            region: 43,
            efficiency: 100_000,
        }],
        10,
    );
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    let mut swapped_import = import.clone();
    (swapped_import.owner, swapped_import.other) = (5, 9);
    let mut redirected_lend = lend.clone();
    redirected_lend.other = 5;
    let report = run_with(
        std::slice::from_ref(&origin),
        std::slice::from_ref(&route),
        &[swapped_import, redirected_lend],
        &[RegionalConvoyEfficiencyRow {
            country: 5,
            region: 43,
            efficiency: 100_000,
        }],
        10,
    );
    let mut endpoints: Vec<_> = report
        .differences
        .iter()
        .filter_map(|difference| {
            ["ConvoyClients.51.owner", "ConvoyClients.51.other", "ConvoyClients.52.other"]
                .into_iter()
                .find(|subject| difference.contains(subject))
        })
        .collect();
    endpoints.sort_unstable();
    assert_eq!(
        endpoints,
        [
            "ConvoyClients.51.other",
            "ConvoyClients.51.owner",
            "ConvoyClients.52.other"
        ],
        "{:?}",
        report.differences
    );
    let report = run(&[origin], &[moved], &[import], 11);
    assert!(
        report
            .differences
            .iter()
            .any(|d| d.contains("TradeRoutes.GER"))
    );
    assert_eq!(report.compared_tables["TradeRoutes"], 0);
    let regional = [RegionalConvoyEfficiencyRow {
        country: 5,
        region: 43,
        efficiency: 50_000,
    }];
    let report = run_with(&[], &[], &[], &regional, 10);
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("RegionalConvoyEfficiency.GER.43")),
        "{:?}",
        report.differences
    );
}

#[test]
fn resource_right_order_is_runtime_only_but_membership_and_unique_positions_are_checked() {
    use replay_data::{rows::ResourceDefinitionRow, rows::ResourceRightRow};
    let saved = saved_pack(
        br#"countries={GER={resources={origin={state=119
            given_resource_rights={ { oil "AFG" } { steel "AFG" } }}}} AFG={}}
        states={} provinces={}"#,
    );
    let ger = CountryRow {
        id: 5,
        tag: b"GER".to_vec(),
        ..Default::default()
    };
    let afg = CountryRow {
        id: 9,
        tag: b"AFG".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(5, &ger), (9, &afg)]);
    let names = BTreeMap::from([(19, &b"steel"[..]), (20, &b"oil"[..])]);
    let resources = [
        ResourceDefinitionRow {
            id: 3,
            token: 19,
            ..Default::default()
        },
        ResourceDefinitionRow {
            id: 4,
            token: 20,
            ..Default::default()
        },
    ];
    let rights = [
        ResourceRightRow {
            state: 119,
            resource: 3,
            holder: 9,
            ordinal: 0,
        },
        ResourceRightRow {
            state: 119,
            resource: 4,
            holder: 9,
            ordinal: 1,
        },
    ];
    let run = |rights: &[ResourceRightRow]| {
        let mut report = Report::default();
        resource_map::rights(
            &ResourceMapRows {
                origins: &[],
                rights,
                clients: &[],
                routes: &[],
                regional: &[],
                reads: &[],
                resources: &resources,
            },
            &saved,
            &links,
            &names,
            &mut report,
        )
        .unwrap();
        report
    };
    assert!(run(&rights).differences.is_empty());
    let mut duplicate = rights.clone();
    duplicate[1].ordinal = 0;
    assert!(
        run(&duplicate)
            .differences
            .iter()
            .any(|d| d.contains("ordinal"))
    );
    assert!(
        run(&rights[..1])
            .differences
            .iter()
            .any(|d| d.contains("oil"))
    );
}
