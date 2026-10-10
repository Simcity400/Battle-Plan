//! UNRUN private oracle fixtures. They certify no native acquisition.
use super::*;
use installed_content::{
    key_names::KeyNames,
    save::{self, Decimal},
};
use replay::frame::TableView;

fn set(frame: &mut DecodedFrame, table: TableId, rows: TableRows) {
    *frame.tables[table.index()] = TableView::Complete(rows);
}

fn fort_dictionary(frame: &mut DecodedFrame) {
    set(
        frame,
        TableId::LexicalTokens,
        TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 0x310A,
            literal: b"LAND_FORT".to_vec(),
        }]),
    );
}

fn division(saved: &mut Save, id: u64, template: Option<u64>) {
    saved.divisions.insert(
        id,
        save::Division {
            id,
            roster_owner: b"SYNTHETIC".to_vec(),
            logical_owner: b"SYNTHETIC".to_vec(),
            expeditionary_owner: None,
            province: None,
            template,
            alternate_template: None,
            separate_counter: false,
            hit_points: Decimal::parse(b"0").unwrap(),
            organisation: Decimal::parse(b"0").unwrap(),
            country_intel: Vec::new(),
            presentation: Default::default(),
        },
    );
}

fn known_commander(leader: u64) -> CommanderAttackInputs {
    CommanderAttackInputs {
        leader,
        fort: Some(0),
        river: Some(0),
        snow: Some(0),
        night: Some(0),
        terrain: Some(Vec::new()),
    }
}

#[test]
fn null_standard_template_does_not_substitute_an_alternate_or_demand_cache_leaves() {
    let (mut frame, mut saved, _, _) = super::super::tests::fixture();
    let id = (7_u64 << 32) | 51;
    let alternate = (9_u64 << 32) | engine::creferenced_division_template::REFERENCE_TYPE as u64;
    saved.divisions.clear();
    division(&mut saved, id, None);
    saved.divisions.get_mut(&id).unwrap().alternate_template = Some(alternate);
    let row = DivisionStatusRow {
        division: id,
        attack: Recorded::Value(DivisionAttackInputs {
            sample_hour: frame.hour,
            template: Some(0),
            ..Default::default()
        }),
        ..Default::default()
    };
    set(
        &mut frame,
        TableId::DivisionStatus,
        TableRows::DivisionStatus(vec![row]),
    );
    let mut report = Report::default();
    divisions(&frame, &saved, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    saved.divisions.get_mut(&id).unwrap().template = Some(alternate);
    let mut report = Report::default();
    divisions(&frame, &saved, &mut report).unwrap();
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("saved standard wrapper"))
    );
}

#[test]
fn historical_attack_absence_keeps_a_saved_identity_difference() {
    let (mut frame, mut saved, _, _) = super::super::tests::fixture();
    saved.divisions.clear();
    let row = DivisionStatusRow {
        division: (7_u64 << 32) | 51,
        attack: Recorded::NotRecorded,
        ..Default::default()
    };
    set(
        &mut frame,
        TableId::DivisionStatus,
        TableRows::DivisionStatus(vec![row]),
    );
    let mut report = Report::default();
    divisions(&frame, &saved, &mut report).unwrap();
    assert!(
        report
            .limitations
            .contains_key("DivisionStatus.attack not recorded")
    );
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("saved division"))
    );
    assert!(report.outcome().is_err());
}

#[test]
fn stale_attack_packet_cannot_certify_the_current_save() {
    let mut report = Report::default();
    assert!(!fresh(&mut report, "synthetic packet", 6, 7));
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("sample_hour"))
    );
    assert!(report.outcome().is_err());
}

#[test]
fn group_commander_null_and_refusal_are_distinct_saved_outcomes() {
    let leader = (4_u64 << 32) | engine::cunit_leader::REFERENCE_TYPE as u64;
    let mut packet = OrdersGroupAttackInputs {
        sample_hour: 7,
        commander: Some(None),
    };
    let mut absent = Report::default();
    group_packet(7, "group", &packet, Some(None), &mut absent);
    assert!(absent.differences.is_empty());
    let mut changed = Report::default();
    group_packet(7, "group", &packet, Some(Some(leader)), &mut changed);
    assert!(
        changed
            .differences
            .iter()
            .any(|line| line.contains("saved direct +88"))
    );
    packet.commander = None;
    let mut refused = Report::default();
    group_packet(7, "group", &packet, Some(None), &mut refused);
    assert!(
        refused
            .differences
            .iter()
            .any(|line| line.contains("capture refused"))
    );
}

#[test]
fn parent_commander_and_failed_numeric_leaf_cannot_hide_direct_leader_difference() {
    let direct = (4_u64 << 32) | engine::cunit_leader::REFERENCE_TYPE as u64;
    let parent = (5_u64 << 32) | engine::cunit_leader::REFERENCE_TYPE as u64;
    let mut commander = known_commander(parent);
    commander.river = None;
    let packet = OrdersGroupAttackInputs {
        sample_hour: 7,
        commander: Some(Some(commander)),
    };
    let mut report = Report::default();
    group_packet(7, "group", &packet, Some(Some(direct)), &mut report);
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("saved direct +88"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("commander.river"))
    );
    assert!(report.outcome().is_err());
}

fn building(saved: &mut Save, province: u32, name: &[u8], healthy: i16) {
    let row = save::construction::Building {
        location: province,
        building_type: name.to_vec(),
        ordinal: 0,
        level: healthy,
        healthy_level: healthy,
        partial_health: Decimal::parse(b"0").unwrap(),
    };
    saved
        .construction
        .province_buildings
        .insert((province, name.to_vec()), row);
}

#[test]
fn fort_sum_uses_exact_signed_content_weights_and_healthy_not_total_levels() {
    let (_, mut saved, _, _) = super::super::tests::fixture();
    let content = installed_content::buildings::parse(
        b"buildings={cosmic={land_fort=2147483647} other={land_fort=-3} maritime={naval_fort=8}}",
        &KeyNames::default(),
    )
    .unwrap();
    building(&mut saved, 17, b"COSMIC", 2);
    building(&mut saved, 17, b"other", -2);
    building(&mut saved, 17, b"maritime", 4);
    saved
        .construction
        .province_buildings
        .get_mut(&(17, b"COSMIC".to_vec()))
        .unwrap()
        .level = 21;
    assert_eq!(saved_fort_sum(17, &saved, &content).unwrap(), 4);
    assert_eq!(saved_fort_sum(18, &saved, &content).unwrap(), 0);
    // The native wrap is -2 + 6, not widened/checked 4_294_967_300.
}

#[test]
fn unknown_or_ambiguous_fort_content_is_refused_instead_of_classified_by_name() {
    let (_, mut saved, _, _) = super::super::tests::fixture();
    building(&mut saved, 17, b"unmapped", 1);
    let content =
        installed_content::buildings::parse(b"buildings={other={land_fort=0}}", &KeyNames::default())
            .unwrap();
    assert!(saved_fort_sum(17, &saved, &content).is_err());
    let content = installed_content::buildings::parse(
        b"buildings={unmapped={land_fort=1} unmapped={land_fort=2}}",
        &KeyNames::default(),
    )
    .unwrap();
    assert!(saved_fort_sum(17, &saved, &content).is_err());
}

#[test]
fn fort_saved_difference_survives_an_independent_state_modifier_refusal() {
    let (mut frame, mut saved, _, _) = super::super::tests::fixture();
    fort_dictionary(&mut frame);
    let content =
        installed_content::buildings::parse(b"buildings={odd={land_fort=3}}", &KeyNames::default())
            .unwrap();
    building(&mut saved, 17, b"odd", 2);
    let row = ProvinceRow {
        id: 17,
        controller: 1,
        fort_positive: 1,
        attack: Recorded::Value(ProvinceAttackInputs {
            sample_hour: frame.hour,
            fort_sum: Some(5),
            land_bunker_effectiveness_factor: None,
        }),
        ..Default::default()
    };
    set(
        &mut frame,
        TableId::Provinces,
        TableRows::Provinces(vec![row]),
    );
    let mut report = Report::default();
    provinces(&frame, &saved, &Ok(&content), &mut report).unwrap();
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("saved healthy buildings"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("land_bunker_effectiveness_factor"))
    );
    assert!(report.outcome().is_err());
}

#[test]
fn nonpositive_fort_sum_does_not_demand_a_state_factor() {
    let (mut frame, saved, _, _) = super::super::tests::fixture();
    fort_dictionary(&mut frame);
    let content = installed_content::buildings::Buildings::default();
    let row = ProvinceRow {
        id: 17,
        fort_positive: 0,
        attack: Recorded::Value(ProvinceAttackInputs {
            sample_hour: frame.hour,
            fort_sum: Some(0),
            land_bunker_effectiveness_factor: None,
        }),
        ..Default::default()
    };
    set(
        &mut frame,
        TableId::Provinces,
        TableRows::Provinces(vec![row]),
    );
    let mut report = Report::default();
    provinces(&frame, &saved, &Ok(&content), &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
}

#[test]
fn missing_or_different_native_keyword_refuses_even_an_empty_saved_fort_sum() {
    let (mut frame, saved, _, _) = super::super::tests::fixture();
    let content = installed_content::buildings::Buildings::default();
    let row = ProvinceRow {
        id: 17,
        fort_positive: 0,
        attack: Recorded::Value(ProvinceAttackInputs {
            sample_hour: frame.hour,
            fort_sum: Some(0),
            land_bunker_effectiveness_factor: None,
        }),
        ..Default::default()
    };
    set(
        &mut frame,
        TableId::Provinces,
        TableRows::Provinces(vec![row]),
    );
    for literal in [None, Some(b"wrong_field".as_slice())] {
        set(
            &mut frame,
            TableId::LexicalTokens,
            TableRows::LexicalTokens(
                literal
                    .into_iter()
                    .map(|literal| LexicalTokenRow {
                        token: 0x310A,
                        literal: literal.to_vec(),
                    })
                    .collect(),
            ),
        );
        let mut report = Report::default();
        provinces(&frame, &saved, &Ok(&content), &mut report).unwrap();
        assert!(
            report
                .failed_comparisons
                .values()
                .any(|reason| reason.contains("token310A"))
        );
        assert!(report.outcome().is_err());
    }
}

#[test]
fn country_capture_demand_preserves_undemanded_absence_and_genuine_zero() {
    let mut packet = CountryAttackInputs {
        sample_hour: 7,
        ..Default::default()
    };
    let mut undemanded = Report::default();
    country_packet(7, "country", &packet, false, false, &mut undemanded);
    assert!(undemanded.differences.is_empty());
    let mut refused = Report::default();
    country_packet(7, "country", &packet, true, false, &mut refused);
    assert!(
        refused
            .differences
            .iter()
            .any(|line| line.contains("land_night_attack"))
    );
    packet.land_night_attack = Some(0);
    let mut zero = Report::default();
    country_packet(7, "country", &packet, true, false, &mut zero);
    assert!(zero.differences.is_empty());
}

#[test]
fn actual_owner_zero_and_raw_aliases_require_recorded_resolution() {
    let links = BTreeMap::from([(42, 9), (77, 0)]);
    let countries = BTreeMap::from([(9, 100)]);
    assert_eq!(demand_country(42, &links, &countries).unwrap(), Some(100));
    assert_eq!(demand_country(77, &links, &countries).unwrap(), None);
    assert_eq!(demand_country(0, &links, &countries).unwrap(), None);
    assert!(demand_country(i32::MIN as u32, &links, &countries).is_err());
    assert!(demand_country(43, &links, &countries).is_err());
}

#[test]
fn country_frame_dispatch_refuses_negative_roster_owner_without_losing_actual_zero_aliases() {
    let (mut frame, _, _, _) = super::super::tests::fixture();
    set(
        &mut frame,
        TableId::CountryLinks,
        TableRows::CountryLinks(Vec::new()),
    );
    set(
        &mut frame,
        TableId::CountryCounterInputs,
        TableRows::CountryCounterInputs(Vec::new()),
    );
    let reserved = ReservedCountryNamingRow {
        raw_country_observations: Recorded::Value(Some(RawCountryObservations {
            sample_hour: frame.hour,
            entries: [0, 77]
                .map(|raw| RawCountryObservation {
                    raw,
                    mapping: RawCountryMapping::Observed {
                        canonical_slot: 0,
                        country_id: 0,
                    },
                    tag: RawCountryTag::Unreached,
                })
                .to_vec(),
        })),
        attack: Recorded::Value(Some(CountryAttackInputs {
            sample_hour: frame.hour,
            land_night_attack: Some(0),
            land_bunker_effectiveness_factor: None,
        })),
        ..Default::default()
    };
    set(
        &mut frame,
        TableId::ReservedCountryNaming,
        TableRows::ReservedCountryNaming(vec![reserved]),
    );
    for raw in [0, 77, i32::MIN as u32] {
        set(
            &mut frame,
            TableId::Divisions,
            TableRows::Divisions(vec![DivisionRow {
                id: (3_u64 << 32) | 51,
                roster_owner: raw,
                expeditionary_owner: 0,
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        let passed = report.attempt("CountryCounterInputs selected attack", |r| {
            countries(&frame, r)
        });
        if raw == i32::MIN as u32 {
            assert!(!passed);
            assert!(country_demand(&frame).is_err());
            assert!(
                report
                    .failed_comparisons
                    .values()
                    .any(|reason| reason.contains("signed-negative"))
            );
            assert!(report.outcome().is_err());
        } else {
            assert!(passed, "{:?}", report.failed_comparisons);
            assert!(country_demand(&frame).unwrap().zero_night);
            assert!(report.differences.is_empty(), "{:?}", report.differences);
            assert!(report.outcome().is_ok());
        }
    }
}

#[test]
fn shared_fort_keyword_keeps_two_provinces_content_and_difference_failures_independent() {
    let (mut frame, mut saved, _, _) = super::super::tests::fixture();
    fort_dictionary(&mut frame);
    let content =
        installed_content::buildings::parse(b"buildings={good={land_fort=3}}", &KeyNames::default())
            .unwrap();
    building(&mut saved, 17, b"unmapped", 1);
    building(&mut saved, 18, b"good", 2);
    let rows = [17, 18].map(|id| ProvinceRow {
        id,
        fort_positive: 1,
        attack: Recorded::Value(ProvinceAttackInputs {
            sample_hour: frame.hour,
            fort_sum: Some(5),
            land_bunker_effectiveness_factor: None,
        }),
        ..Default::default()
    });
    set(
        &mut frame,
        TableId::Provinces,
        TableRows::Provinces(rows.to_vec()),
    );
    let mut report = Report::default();
    provinces(&frame, &saved, &Ok(&content), &mut report).unwrap();
    assert!(
        report
            .failed_comparisons
            .contains_key("Provinces selected attack saved fort 17")
    );
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("Provinces 18.attack.fort_sum saved healthy buildings"))
    );
    assert!(report.outcome().is_err());

    // A globally unauthenticated keyword must refuse both saved numerical
    // comparisons. It still cannot suppress the second row's prefix check.
    set(
        &mut frame,
        TableId::LexicalTokens,
        TableRows::LexicalTokens(Vec::new()),
    );
    let mut rows = rows;
    let Recorded::Value(packet) = &mut rows[1].attack else {
        unreachable!()
    };
    packet.fort_sum = Some(0);
    set(
        &mut frame,
        TableId::Provinces,
        TableRows::Provinces(rows.to_vec()),
    );
    let mut report = Report::default();
    provinces(&frame, &saved, &Ok(&content), &mut report).unwrap();
    assert!(
        report
            .failed_comparisons
            .contains_key("Provinces selected attack saved fort 17")
    );
    assert!(
        report
            .failed_comparisons
            .contains_key("Provinces selected attack saved fort 18")
    );
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("Provinces 18.attack.fort_positive packet consistency"))
    );
    assert!(
        !report
            .differences
            .iter()
            .any(|line| line.contains("saved healthy buildings"))
    );
    assert!(report.outcome().is_err());
}

#[test]
fn six_native_scalars_count_one_refusal_without_fabricating_zero_or_hiding_siblings() {
    let (mut frame, _, _, _) = super::super::tests::fixture();
    let mut packet = PoliticalRuntimeScalars {
        sample_hour: frame.hour,
        river_small: Some(i64::MIN),
        river_large: None,
        night: Some(0),
        fort: Some(i64::MAX),
        move_strength_threshold: Some(-1),
        move_organisation_threshold: Some(1),
    };
    set(
        &mut frame,
        TableId::SimulationFeatures,
        TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            runtime_scalars: Recorded::Value(packet),
            ..Default::default()
        }]),
    );
    let mut report = Report::default();
    scalars(&frame, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("river_large"));
    packet.river_large = Some(0);
    set(
        &mut frame,
        TableId::SimulationFeatures,
        TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            runtime_scalars: Recorded::Value(packet),
            ..Default::default()
        }]),
    );
    let mut report = Report::default();
    scalars(&frame, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("not independent acceptance"))
    );
}

#[test]
fn terrain_failure_is_local_and_empty_live_domain_is_distinct() {
    let mut report = Report::default();
    terrain(
        &mut report,
        "domain",
        &Some(vec![
            AttackTerrain {
                terrain: 1,
                attack: None,
            },
            AttackTerrain {
                terrain: 2,
                attack: Some(i64::MIN),
            },
        ]),
    );
    assert_eq!(report.differences.len(), 1);
    let mut empty = Report::default();
    terrain(&mut empty, "domain", &Some(Vec::new()));
    assert!(empty.differences.is_empty());
    let mut refused = Report::default();
    terrain(&mut refused, "domain", &None);
    assert_eq!(refused.differences.len(), 1);
}

#[test]
fn saved_weather_identity_difference_remains_when_an_effective_modifier_is_refused() {
    let (mut frame, mut saved, _, _) = super::super::tests::fixture();
    saved.weather.regions = Some(vec![save::weather::Region {
        slot: 0,
        region: Some(3),
        rain_light: None,
        rain_heavy: None,
        snow: None,
        blizzard: None,
        sandstorm: None,
        arctic_water: None,
    }]);
    let row = WeatherRegionRow {
        id: 0,
        attack: Recorded::Value(WeatherRegionAttackInputs {
            sample_hour: frame.hour,
            attack: None,
            breakthrough: Some(0),
        }),
        ..Default::default()
    };
    set(
        &mut frame,
        TableId::WeatherRegions,
        TableRows::WeatherRegions(vec![row]),
    );
    let mut report = Report::default();
    regions(&frame, &saved, &mut report).unwrap();
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("saved region identity"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains(".attack.attack"))
    );
    assert!(report.outcome().is_err());
}

fn install_raw(frame: &mut DecodedFrame, field: Recorded<Option<RawCountryObservations>>) {
    set(
        frame,
        TableId::ReservedCountryNaming,
        TableRows::ReservedCountryNaming(vec![ReservedCountryNamingRow {
            raw_country_observations: field,
            // Raw-certificate fixtures do not install a current attack packet.
            attack: Recorded::NotRecorded,
            ..Default::default()
        }]),
    );
}
fn raw_packet(hour: i32, mapping: RawCountryMapping, tag: RawCountryTag) -> RawCountryObservations {
    RawCountryObservations {
        sample_hour: hour,
        entries: vec![RawCountryObservation {
            raw: 77,
            mapping,
            tag,
        }],
    }
}

#[test]
fn omitted_alias_requires_current_exact_zero_certificate_and_normal_row_wins() {
    let (mut frame, _, _, _) = super::super::tests::fixture();
    let hour = frame.hour;
    install_raw(
        &mut frame,
        Recorded::Value(Some(raw_packet(
            hour,
            RawCountryMapping::Observed {
                canonical_slot: 0,
                country_id: 0,
            },
            RawCountryTag::Unreached,
        ))),
    );
    let countries = BTreeMap::from([(9, 100)]);
    assert_eq!(
        demand_country_observed(&frame, 77, &BTreeMap::new(), &countries).unwrap(),
        None
    );
    assert_eq!(
        demand_country_observed(&frame, 77, &BTreeMap::from([(77, 9)]), &countries).unwrap(),
        Some(100)
    );
    assert!(demand_country_observed(&frame, 78, &BTreeMap::new(), &countries).is_err());
    assert!(
        demand_country_observed(&frame, i32::MIN as u32, &BTreeMap::new(), &countries).is_err()
    );
    for mapping in [
        RawCountryMapping::Refused,
        RawCountryMapping::Unreached,
        RawCountryMapping::Observed {
            canonical_slot: 9,
            country_id: 100,
        },
    ] {
        install_raw(
            &mut frame,
            Recorded::Value(Some(raw_packet(hour, mapping, RawCountryTag::Refused))),
        );
        assert!(demand_country_observed(&frame, 77, &BTreeMap::new(), &countries).is_err());
    }
    install_raw(
        &mut frame,
        Recorded::Value(Some(raw_packet(
            hour - 1,
            RawCountryMapping::Observed {
                canonical_slot: 0,
                country_id: 0,
            },
            RawCountryTag::Unreached,
        ))),
    );
    assert!(demand_country_observed(&frame, 77, &BTreeMap::new(), &countries).is_err());
}

#[test]
fn raw_current_refusal_and_staleness_fail_while_historical_absence_is_named() {
    let (mut frame, _, _, _) = super::super::tests::fixture();
    install_raw(&mut frame, Recorded::Value(None));
    let mut refused = Report::default();
    raw_observations(&frame, &mut refused).unwrap();
    assert!(refused.outcome().is_err());
    assert!(
        refused
            .differences
            .iter()
            .any(|line| line.contains("capture refused"))
    );
    install_raw(&mut frame, Recorded::NotRecorded);
    let mut historical = Report::default();
    raw_observations(&frame, &mut historical).unwrap();
    assert!(historical.outcome().is_ok());
    assert!(!historical.limitations.is_empty());
    let hour = frame.hour;
    install_raw(
        &mut frame,
        Recorded::Value(Some(raw_packet(
            hour - 1,
            RawCountryMapping::Unreached,
            RawCountryTag::Bytes(Vec::new()),
        ))),
    );
    let mut stale = Report::default();
    raw_observations(&frame, &mut stale).unwrap();
    assert!(stale.outcome().is_err());
    assert!(!stale.differences.is_empty());
}

#[test]
fn historical_attack_absence_does_not_demand_an_unrecorded_raw_certificate() {
    let (mut frame, _, _, _) = super::super::tests::fixture();
    set(
        &mut frame,
        TableId::CountryCounterInputs,
        TableRows::CountryCounterInputs(Vec::new()),
    );
    set(
        &mut frame,
        TableId::Divisions,
        TableRows::Divisions(vec![DivisionRow {
            id: (3_u64 << 32) | 51,
            roster_owner: 0,
            ..Default::default()
        }]),
    );
    install_raw(&mut frame, Recorded::NotRecorded);
    let mut report = Report::default();
    countries(&frame, &mut report).unwrap();
    assert!(report.outcome().is_ok());
    assert_eq!(
        report.limitations["ReservedCountryNaming.attack not recorded"],
        vec!["actual owner0 attack".to_string()],
    );
}

#[test]
fn raw_identity_disagreement_remains_a_difference() {
    let (mut frame, _, _, _) = super::super::tests::fixture();
    let hour = frame.hour;
    set(
        &mut frame,
        TableId::CountryLinks,
        TableRows::CountryLinks(vec![CountryLinkRow {
            id: 77,
            canonical_slot: 9,
        }]),
    );
    set(
        &mut frame,
        TableId::Countries,
        TableRows::Countries(vec![CountryRow {
            id: 100,
            canonical_slot: 9,
            ..Default::default()
        }]),
    );
    for mapping in [
        RawCountryMapping::Observed {
            canonical_slot: 0,
            country_id: 0,
        },
        RawCountryMapping::Observed {
            canonical_slot: 9,
            country_id: 101,
        },
    ] {
        install_raw(
            &mut frame,
            Recorded::Value(Some(raw_packet(hour, mapping, RawCountryTag::Unreached))),
        );
        let mut report = Report::default();
        raw_observations(&frame, &mut report).unwrap();
        assert!(report.outcome().is_err());
        assert!(
            report
                .differences
                .iter()
                .any(|line| line.contains("raw mapping 77"))
        );
    }
}

#[test]
fn demanded_template_tag_keeps_empty_bytes_distinct_from_refused_or_unreached() {
    let (mut frame, _, _, _) = super::super::tests::fixture();
    let hour = frame.hour;
    let handle = (7000_u64 << 32) | 52;
    set(
        &mut frame,
        TableId::DivisionTemplateIcons,
        TableRows::DivisionTemplateIcons(vec![DivisionTemplateIconRow {
            template: handle,
            country: 77,
            display_name: Recorded::Value(Some(TemplateDisplayName {
                sample_hour: hour,
                value: TemplateNameValue::Changed {
                    literal: Vec::new(),
                    fallback_key: Some(b"KEY".to_vec()),
                },
            })),
            ..Default::default()
        }]),
    );
    for tag in [
        RawCountryTag::Unreached,
        RawCountryTag::Refused,
        RawCountryTag::Bytes(Vec::new()),
    ] {
        let empty = matches!(tag, RawCountryTag::Bytes(_));
        install_raw(
            &mut frame,
            Recorded::Value(Some(raw_packet(hour, RawCountryMapping::Refused, tag))),
        );
        let mut report = Report::default();
        raw_observations(&frame, &mut report).unwrap();
        assert_eq!(report.differences.is_empty(), empty);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("unsaved runtime witnesses"))
        );
    }
}
