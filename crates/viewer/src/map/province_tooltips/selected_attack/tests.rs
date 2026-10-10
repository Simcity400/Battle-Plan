//! Selected-attack packet adapter and arithmetic fixtures.
use super::*;
use crate::map::{test_frame, test_put as put};
use arithmetic::{ONE, Projection, Row};
use replay::rows::{
    AttackTerrain, DivisionAttackInputs, DivisionRow, DivisionStatusRow, Recorded, TableRows,
};

fn empty() -> Projection {
    Projection {
        terrain: Ok(0),
        river: Ok(None),
        fort: Ok(None),
        snow: Ok(None),
        weather: Ok(None),
        night: Ok(None),
        enemy_air_positive: Ok(false),
    }
}

#[test]
fn every_native_slot_keeps_order_duplicate_weather_and_b_only_zero_attack() {
    let mut input = empty();
    input.terrain = Ok(1);
    input.river = Ok(Some([0, -2]));
    input.fort = Ok(Some([0, -3]));
    input.snow = Ok(Some(-4));
    input.weather = Ok(Some([0, -5]));
    input.night = Ok(Some(0));
    input.enemy_air_positive = Ok(true);
    let actual = arithmetic::rows(input);
    assert_eq!(
        actual.rows,
        vec![
            Row { kind: 27, value: 1 },
            Row { kind: 10, value: 0 },
            Row { kind: 9, value: 0 },
            Row {
                kind: 28,
                value: -4
            },
            Row { kind: 28, value: 0 },
            Row { kind: 29, value: 0 },
            Row { kind: 14, value: 0 }
        ]
    );
    assert!(actual.refusals.is_empty());
}

#[test]
fn refusal_does_not_change_zero_admission_or_remove_independent_later_rows() {
    let mut input = empty();
    input.terrain = Err("terrain refused".into());
    input.snow = Ok(Some(0));
    input.weather = Ok(Some([0, 0]));
    input.night = Ok(Some(0));
    input.enemy_air_positive = Ok(true);
    let actual = arithmetic::rows(input);
    assert_eq!(
        actual.rows,
        vec![Row { kind: 29, value: 0 }, Row { kind: 14, value: 0 }]
    );
    assert_eq!(actual.refusals, vec![("terrain", "terrain refused".into())]);
    assert!(arithmetic::rows(empty()).rows.is_empty());
}

#[test]
fn wrapped_one_products_cannot_be_cancelled_before_native_truncation() {
    assert_eq!(arithmetic::river([0, 0], 0, i64::MAX), [-1, -1]);
    assert_eq!(arithmetic::mul(i64::MAX, ONE), -1);
    assert_eq!(arithmetic::weather([-3, 5], 50_000), [-1, 2]);
    assert_eq!(arithmetic::night(i64::MAX, 1, 0, 0), i64::MIN);
}

#[test]
fn healthy_fort_is_signed_weighted_total_and_b_controls_visible_zero_row() {
    assert_eq!(arithmetic::fort([0, 0], 0, -1, -15_000), None);
    assert_eq!(
        arithmetic::fort([50_000, 0], 0, 3, -15_000),
        Some([0, -45_000])
    );
    assert_eq!(arithmetic::fort_base(Some(100_000), 0, -15_000), -30_000);
    assert_eq!(arithmetic::fort_base(None, i64::MAX, -15_000), -15_000);
}

#[test]
fn snow_uses_wrapping_sum_and_manager_ceiling() {
    assert!(arithmetic::has_snow(0, 2, 100));
    assert!(!arithmetic::has_snow(5, 2, 0));
    assert!(!arithmetic::has_snow(i64::MAX, 1, i64::MAX));
}

#[test]
fn one_shared_retained_route_is_independent_of_representative_and_eta_fallback() {
    let route = [5, 6, 7];
    assert_eq!(arithmetic::retained_edge(3, Ok(&route)), Ok(Some((6, 7))));
    assert_eq!(arithmetic::retained_edge(99, Ok(&route)), Ok(Some((6, 7))));
    assert_eq!(arithmetic::retained_edge(3, Ok(&[7])), Ok(Some((3, 7))));
    assert_eq!(arithmetic::retained_edge(3, Ok(&[])), Ok(None));
    assert_eq!(
        arithmetic::retained_edge(3, Err("route refused".into())),
        Err("route refused".into())
    );
}

#[test]
fn known_different_sources_skip_even_with_another_missing_location() {
    assert_eq!(
        arithmetic::same_current(&[Err("missing".into()), Ok(3), Ok(4)]),
        Ok(None)
    );
    assert_eq!(
        arithmetic::same_current(&[Err("missing".into()), Ok(3), Ok(3)]),
        Err("missing".into())
    );
    assert_eq!(arithmetic::same_current(&[Ok(3), Ok(3)]), Ok(Some(3)));
}

#[test]
fn raw_gates_do_not_demand_unknown_later_operands() {
    assert_eq!(
        outer_gate(0, || panic!(), |_| panic!(), |_| panic!()),
        Ok(false)
    );
    assert_eq!(
        outer_gate(u32::MAX, || panic!(), |_| panic!(), |_| panic!()),
        Ok(false)
    );
    assert_eq!(
        outer_gate(3, || Ok(0), |_| panic!(), |_| panic!()),
        Ok(false)
    );
    assert_eq!(
        outer_gate(3, || Ok(3), |_| panic!(), |_| panic!()),
        Ok(false)
    );
    assert_eq!(outer_gate(3, || Ok(4), |_| Ok(9), |_| panic!()), Ok(false));
}

#[test]
fn positive_raw_target_alias_to_actual_zero_still_demands_direct_matrix_zero() {
    let mut queried = false;
    assert_eq!(
        outer_gate(
            3,
            || Ok(99),
            |raw| Ok(if raw == 99 { 0 } else { 1 }),
            |target| {
                assert_eq!(target, 0);
                queried = true;
                Ok(true)
            }
        ),
        Ok(true)
    );
    assert!(queried);
    assert_eq!(
        outer_gate(
            3,
            || Ok(99),
            |raw| Ok(if raw == 99 { 0 } else { 1 }),
            |_| Err("uncertified directed target".into())
        ),
        Err("uncertified directed target".into())
    );
}

#[test]
fn heading_and_row_fixed5_controls_keep_positive_near_zero_and_red_zero() {
    assert_eq!(
        arithmetic::modifier_text("Terrain", 1),
        "Terrain: \u{11}G+0.00%\u{11}!"
    );
    assert_eq!(
        arithmetic::modifier_text("Weather", 0),
        "Weather: \u{11}R0.00%\u{11}!"
    );
    assert_eq!(
        arithmetic::modifier_text("River", -10_000),
        "River: \u{11}R-10.00%\u{11}!"
    );
}

fn certificate_frame(
    template: u64,
    attack_template: Option<u64>,
    sample_hour: i32,
) -> DecodedFrame {
    let mut frame = test_frame(42);
    put(
        &mut frame,
        TableRows::Divisions(vec![DivisionRow {
            id: 7,
            template,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::DivisionStatus(vec![DivisionStatusRow {
            division: 7,
            attack: Recorded::Value(DivisionAttackInputs {
                sample_hour,
                template: attack_template,
                ..Default::default()
            }),
            ..Default::default()
        }]),
    );
    frame
}

#[test]
fn template_identity_is_current_typed_certificate_and_matches_census() {
    let handle = (13_u64 << 32) | 52;
    assert_eq!(
        template(&certificate_frame(handle, Some(handle), 42), 7),
        Ok(handle)
    );
    assert_eq!(template(&certificate_frame(0, Some(0), 42), 7), Ok(0));
    assert!(template(&certificate_frame(handle, Some(handle), 41), 7).is_err());
    assert!(template(&certificate_frame(handle, None, 42), 7).is_err());
    assert!(template(&certificate_frame(handle, Some((14_u64 << 32) | 52), 42), 7).is_err());
    assert!(template(&certificate_frame(handle, Some((13_u64 << 32) | 51), 42), 7).is_err());
}

#[test]
fn historical_attack_gap_never_uses_legacy_template_as_current_certificate() {
    let mut frame = certificate_frame((13_u64 << 32) | 52, None, 42);
    put(
        &mut frame,
        TableRows::DivisionStatus(vec![DivisionStatusRow {
            division: 7,
            attack: Recorded::NotRecorded,
            ..Default::default()
        }]),
    );
    assert!(template(&frame, 7).unwrap_err().contains("not recorded"));
}

#[test]
fn terrain_domain_preserves_known_zero_refusal_and_rejects_ambiguous_identity() {
    let domain = Some(vec![
        AttackTerrain {
            terrain: 1,
            attack: Some(0),
        },
        AttackTerrain {
            terrain: 2,
            attack: None,
        },
    ]);
    assert_eq!(terrain(&domain, 1, "test"), Ok(0));
    assert!(terrain(&domain, 2, "test").is_err());
    assert!(terrain(&domain, 3, "test").is_err());
    assert!(terrain(&None, 1, "test").is_err());
    let duplicate = Some(vec![
        AttackTerrain {
            terrain: 1,
            attack: Some(0)
        };
        2
    ]);
    assert!(terrain(&duplicate, 1, "test").is_err());
}

#[test]
fn native_null_direct_commander_is_zero_without_group_packet_or_parent_chain() {
    assert!(commander(&test_frame(42), 0).unwrap().is_none());
    assert_eq!(leader_scalar(&Ok(None), |_| panic!(), "test"), Ok(0));
    assert_eq!(
        leader_scalar(&Err("selector refused".into()), |_| panic!(), "test"),
        Err("selector refused".into())
    );
}

#[test]
fn first_template_encounter_wins_and_known_null_wrappers_deduplicate() {
    let mut seen = BTreeSet::new();
    let mut unknown = false;
    assert_eq!(first_template(Ok(9), &mut seen, &mut unknown), Ok(Some(9)));
    assert_eq!(first_template(Ok(2), &mut seen, &mut unknown), Ok(Some(2)));
    assert_eq!(first_template(Ok(9), &mut seen, &mut unknown), Ok(None));
    assert_eq!(first_template(Ok(0), &mut seen, &mut unknown), Ok(Some(0)));
    assert_eq!(first_template(Ok(0), &mut seen, &mut unknown), Ok(None));
}

#[test]
fn unknown_earlier_wrapper_cannot_promote_a_later_representative() {
    let mut seen = BTreeSet::new();
    let mut unknown = false;
    assert_eq!(first_template(Ok(9), &mut seen, &mut unknown), Ok(Some(9)));
    assert_eq!(
        first_template(Err("missing wrapper".into()), &mut seen, &mut unknown),
        Err("missing wrapper".into())
    );
    assert_eq!(first_template(Ok(9), &mut seen, &mut unknown), Ok(None));
    assert!(first_template(Ok(2), &mut seen, &mut unknown).is_err());
    assert_eq!(first_template(Ok(2), &mut seen, &mut unknown), Ok(None));
}

#[test]
fn template_fallback_callback_converts_key_before_the_shared_country_localizer() {
    use replay::rows::TemplateDisplayNameParts as Parts;
    let localisation = installed_content::localisation::Localisation::default();
    let actual = dashboard::unit_names::template_name(
        Parts {
            literal: b"",
            fallback_key: Some(b"RAW_KEY"),
        },
        0,
        |raw, bytes| {
            assert_eq!(raw, 0);
            let key = fallback_key(bytes)?;
            crate::combat::text::country_localised(&localisation, Ok("ZZZ"), key)
        },
    );
    assert_eq!(actual, Ok("RAW_KEY".to_owned()));
    let refused = dashboard::unit_names::template_name(
        Parts {
            literal: b"",
            fallback_key: Some(&[255]),
        },
        0,
        |_, bytes| {
            let key = fallback_key(bytes)?;
            crate::combat::text::country_localised(&localisation, Ok("ZZZ"), key)
        },
    );
    assert_eq!(
        refused,
        Err("template fallback localisation key is not UTF-8".to_owned())
    );
}

#[test]
fn template_literal_or_known_empty_key_never_runs_the_fallback_conversion() {
    use replay::rows::TemplateDisplayNameParts as Parts;
    for (parts, expected) in [
        (
            Parts {
                literal: b"Authored",
                fallback_key: None,
            },
            "Authored",
        ),
        (
            Parts {
                literal: b"",
                fallback_key: Some(b""),
            },
            "",
        ),
    ] {
        assert_eq!(
            dashboard::unit_names::template_name(parts, -1, |_, bytes| {
                let _ = fallback_key(bytes)?;
                panic!("unexpected fallback callback")
            }),
            Ok(expected.to_owned())
        );
    }
}

// Attack-preview source fixtures.
fn fallback_localisation(entries: &[(&str, &str)]) -> installed_content::localisation::Localisation {
    entries
        .iter()
        .map(|&(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

#[test]
fn country_qualified_key_uses_only_the_native_c_string_before_utf8_and_lookup() {
    let loc = fallback_localisation(&[("TAG_KEY", "Qualified"), ("KEY", "Plain")]);
    for bytes in [&b"KEY\0TAIL"[..], &b"KEY\0\xFF"[..]] {
        assert_eq!(
            localise_template_fallback(&loc, "TAG", bytes),
            Ok("Qualified".to_owned())
        );
    }
    assert_eq!(fallback_key(b"KEY\0\xFF"), Ok("KEY"));
    assert!(
        fallback_key(b"\xFF\0KEY")
            .unwrap_err()
            .contains("not UTF-8")
    );
}

#[test]
fn originally_nonempty_leading_nul_key_still_demands_qualified_tag_underscore() {
    use replay::rows::TemplateDisplayNameParts as Parts;
    let loc = fallback_localisation(&[("TAG_", "Leading NUL qualified")]);
    let mut called = false;
    let name = dashboard::unit_names::template_name(
        Parts {
            literal: b"",
            fallback_key: Some(b"\0X"),
        },
        7,
        |raw, bytes| {
            called = true;
            assert_eq!(raw, 7);
            assert_eq!(bytes, b"\0X");
            localise_template_fallback(&loc, "TAG", bytes)
        },
    );
    assert!(called);
    assert_eq!(name, Ok("Leading NUL qualified".to_owned()));
}

#[test]
fn defined_empty_qualified_values_remain_authoritative_including_leading_nul() {
    use replay::rows::TemplateDisplayNameParts as Parts;
    let loc = fallback_localisation(&[("TAG_KEY", ""), ("KEY", "Plain"), ("TAG_", "")]);
    for bytes in [&b"KEY\0TAIL"[..], &b"\0X"[..]] {
        assert_eq!(
            dashboard::unit_names::template_name(
                Parts {
                    literal: b"",
                    fallback_key: Some(bytes)
                },
                0,
                |_, bytes| localise_template_fallback(&loc, "TAG", bytes)
            ),
            Ok(String::new())
        );
    }
}

#[test]
fn absent_qualified_key_uses_the_independent_plain_c_string() {
    let loc = fallback_localisation(&[("KEY", "Plain")]);
    assert_eq!(
        localise_template_fallback(&loc, "TAG", b"KEY\0\xFF"),
        Ok("Plain".to_owned())
    );
    assert_eq!(
        localise_template_fallback(&loc, "TAG", b"\0X"),
        Ok(String::new())
    );
}

#[test]
fn embedded_nul_tag_cuts_the_complete_candidate_for_positive_and_reserved_paths() {
    use replay::rows::TemplateDisplayNameParts as Parts;
    let loc = fallback_localisation(&[
        ("TA", "Native candidate"),
        ("TA_KEY", "Wrong cut"),
        ("KEY", "Wrong plain"),
    ]);
    for raw in [0, 7] {
        assert_eq!(
            dashboard::unit_names::template_name(
                Parts {
                    literal: b"",
                    fallback_key: Some(b"KEY")
                },
                raw,
                |received, bytes| {
                    assert_eq!(received, raw);
                    // Both actual tag providers feed the same adapter after their branch.
                    localise_template_fallback(&loc, "TA\0G", bytes)
                }
            ),
            Ok("Native candidate".to_owned())
        );
    }
}

#[test]
fn raw_mapping_requires_exact_zero_certificate_and_normal_typed_rows_have_priority() {
    use replay::rows::{
        CountryLinkRow, CountryRow, RawCountryMapping, RawCountryObservation,
        RawCountryObservations, RawCountryTag, ReservedCountryNamingRow,
    };
    let mut frame = test_frame(17);
    put(&mut frame, TableRows::CountryLinks(Vec::new()));
    let mut reserved = ReservedCountryNamingRow {
        raw_country_observations: Recorded::Value(Some(RawCountryObservations {
            sample_hour: 17,
            entries: vec![RawCountryObservation {
                raw: 7,
                mapping: RawCountryMapping::Observed {
                    canonical_slot: 0,
                    country_id: 0,
                },
                tag: RawCountryTag::Bytes(b"ALIAS\0\xff".to_vec()),
            }],
        })),
        ..Default::default()
    };
    put(
        &mut frame,
        TableRows::ReservedCountryNaming(vec![reserved.clone()]),
    );
    assert_eq!(slot_frame(&frame, 7), Ok(0));
    assert!(slot_frame(&frame, 8).is_err());
    put(
        &mut frame,
        TableRows::Countries(vec![CountryRow {
            id: 12,
            canonical_slot: 3,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::CountryLinks(vec![CountryLinkRow {
            id: 7,
            canonical_slot: 3,
        }]),
    );
    assert_eq!(slot_frame(&frame, 7), Ok(3));
    // Wrong type is unavailable normal input, never a missing row eligible for
    // sparse fallback.
    *frame.tables[rows::TableId::CountryLinks.index()] =
        replay::frame::TableView::Complete(TableRows::Countries(Vec::new()));
    assert!(slot_frame(&frame, 7).is_err());
    put(&mut frame, TableRows::CountryLinks(Vec::new()));
    reserved.raw_country_observations = Recorded::Value(Some(RawCountryObservations {
        sample_hour: 16,
        entries: vec![RawCountryObservation {
            raw: 7,
            mapping: RawCountryMapping::Observed {
                canonical_slot: 0,
                country_id: 0,
            },
            tag: RawCountryTag::Unreached,
        }],
    }));
    put(&mut frame, TableRows::ReservedCountryNaming(vec![reserved]));
    assert!(slot_frame(&frame, 7).is_err());
}

#[test]
fn tag_nul_precedes_utf8_and_unknown_exact_raw_tag_cannot_use_plain_fallback() {
    let loc = fallback_localisation(&[("TA", "Qualified"), ("KEY", "Plain")]);
    assert_eq!(
        localise_template_fallback(&loc, b"TA\0\xff", b"KEY\0\xff"),
        Ok("Qualified".into())
    );
    assert!(localise_template_fallback(&loc, b"\xff\0TA", b"KEY").is_err());
    let mut frame = test_frame(9);
    put(
        &mut frame,
        TableRows::ReservedCountryNaming(vec![rows::ReservedCountryNamingRow {
            raw_country_observations: Recorded::Value(Some(rows::RawCountryObservations {
                sample_hour: 9,
                entries: vec![rows::RawCountryObservation {
                    raw: 7,
                    mapping: rows::RawCountryMapping::Observed {
                        canonical_slot: 0,
                        country_id: 0,
                    },
                    tag: rows::RawCountryTag::Refused,
                }],
            })),
            ..Default::default()
        }]),
    );
    assert!(matches!(
        raw_observation(&frame, 7).unwrap().tag,
        rows::RawCountryTag::Refused
    ));
    assert!(raw_observation(&frame, 8).is_err());
}

fn recorded_layout_frame() -> DecodedFrame {
    engine::executable_tables::initialize_for_tests()
        .expect("modifier-name tests require BATTLE_PLAN_GAME_DIR with the supported installation");
    let mut frame = test_frame(42);
    let mut keys = engine::modifier_keys::all().collect::<Vec<_>>();
    keys.sort_unstable_by_key(|key| key.index);
    put(
        &mut frame,
        TableRows::ModifierDefinitions(
            keys.iter()
                .map(|key| rows::ModifierDefinitionRow {
                    index: key.index,
                    token: key.index + 1,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::LexicalTokens(
            keys.iter()
                .map(|key| rows::LexicalTokenRow {
                    token: key.index + 1,
                    literal: key.name.as_bytes().to_vec(),
                })
                .collect(),
        ),
    );
    frame
}

#[test]
fn selected_attack_fixed_scalar_requires_its_exact_recorded_definition_and_preserves_other_fields()
{
    use replay::fixed_layout::{
        SELECTED_ATTACK_FORT, SELECTED_ATTACK_NIGHT, SELECTED_ATTACK_WEATHER_ATTACK,
        SELECTED_ATTACK_WEATHER_BREAKTHROUGH,
    };
    let families = [
        SELECTED_ATTACK_WEATHER_ATTACK,
        SELECTED_ATTACK_WEATHER_BREAKTHROUGH,
        SELECTED_ATTACK_NIGHT,
        SELECTED_ATTACK_FORT,
    ];
    for family in families {
        assert!(family.tables.is_empty());
        assert_eq!(family.keys.len(), 1);
        let mut frame = recorded_layout_frame();
        assert_eq!(
            fixed_scalar(&frame, Some(0), family, "fixture scalar"),
            Ok(0)
        );
        let definitions = frame.complete::<tables::ModifierDefinitions>().unwrap();
        let broken: Vec<_> = definitions
            .iter()
            .filter(|row| row.index != family.keys[0])
            .cloned()
            .collect();
        put(&mut frame, TableRows::ModifierDefinitions(broken));
        assert!(
            fixed_scalar(&frame, Some(123), family, "fixture scalar")
                .unwrap_err()
                .contains("definition row missing")
        );
        for other in families {
            if other.keys != family.keys {
                assert_eq!(
                    fixed_scalar(&frame, Some(0), other, "independent scalar"),
                    Ok(0)
                );
            }
        }
    }
}

#[test]
fn selected_attack_fixed_scalar_refuses_wrong_spelling_and_unavailable_recorded_tables() {
    let family = replay::fixed_layout::SELECTED_ATTACK_FORT;
    let mut frame = recorded_layout_frame();
    let tokens = frame.complete::<tables::LexicalTokens>().unwrap();
    let mut changed = tokens.to_vec();
    changed
        .iter_mut()
        .find(|token| token.token == family.keys[0] + 1)
        .unwrap()
        .literal = b"different_mod_key".to_vec();
    put(&mut frame, TableRows::LexicalTokens(changed));
    assert!(
        fixed_scalar(&frame, Some(0), family, "fort")
            .unwrap_err()
            .contains("different script name")
    );
    assert!(fixed_scalar(&test_frame(42), Some(0), family, "fort").is_err());
    assert!(
        fixed_scalar(&recorded_layout_frame(), None, family, "fort")
            .unwrap_err()
            .contains("refused when recorded")
    );
}
