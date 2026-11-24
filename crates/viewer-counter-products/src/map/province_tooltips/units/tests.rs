use super::*;
use replay_data::{
    rows::ArmyIconInputRow, rows::ClockRow, rows::DivisionRow, rows::LexicalTokenRow,
    rows::ProvinceUnitOrderRow, rows::TableRows, rows::TemplateSubunit,
};
use viewer_runtime::count_math::count_range_seeded;
use viewer_foundation::test_support::unit_types::subunits as synthetic_subunits;
use viewer_foundation::test_support::{test_frame, test_put as put};

fn unit(owner: u32, rank: u8, template_id: u32, intel: u16) -> Unit {
    Unit {
        owner,
        rank,
        template: u64::from(template_id) << 32 | 7,
        intel,
    }
}

fn content() -> Content {
    let subunits = synthetic_subunits();
    Content {
        template_reveal: Ok(
            viewer_foundation::visibility::template_reveal::Defines {
                army_maximum: 100_000,
                basic: 30_000,
                standard: 70_000,
                count: [25_000, 75_000, 150_000],
            },
        ),
        subunits,
        defines: CountDefines {
            denominator: 100_000,
            minimum: 25_000,
            maximum: 75_000,
            range: 150_000,
        },
    }
}

fn loc() -> Localisation {
    [
        ("SUB_UNIT_ACCURAT_VALUE", "$VALUE$"),
        ("SUB_UNIT_RANGED_VALUE", "$MIN$-$MAX$"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect()
}

#[test]
fn first_input_fake_template_applies_to_every_known_unit_and_249_is_omitted() {
    let units = vec![unit(2, 0, 1, 249), unit(1, 3, 9, 255), unit(3, 1, 7, 250)];
    let fallback = 44 << 32 | 7;
    let prepared = group_templates(units, false, fallback);
    assert_eq!(prepared.groups, [(fallback, 2)]);
    assert!(
        group_templates(vec![unit(2, 0, 1, 249)], false, fallback)
            .groups
            .is_empty()
    );
}

#[test]
fn groups_use_native_rank_country_and_numeric_template_id_descending() {
    let groups = group_templates(
        vec![
            unit(5, 1, 2, 250),
            unit(2, 2, 3, 250),
            unit(2, 2, 9, 250),
            unit(1, 3, 1, 250),
            unit(5, 1, 2, 250),
            unit(6, 1, 4, 250),
            unit(8, 0, 5, 250),
        ],
        true,
        0,
    );
    assert_eq!(
        groups.groups,
        [(1, 1), (9, 1), (3, 1), (4, 1), (2, 2), (5, 1)].map(|(id, count)| ((id << 32) | 7, count))
    );
}

fn frame() -> DecodedFrame {
    let mut frame = test_frame(60_000_000);
    put(
        &mut frame,
        TableRows::Divisions(
            [(1, 11, 1), (2, 22, 9), (3, 22, 1)]
                .into_iter()
                .map(|(id, roster_owner, template)| DivisionRow {
                    id,
                    roster_owner,
                    template: template << 32 | 7,
                    province: 100,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::ArmyIconInputs(vec![ArmyIconInputRow {
            division: 3,
            alternate_template: 44 << 32 | 7,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::ProvinceUnitOrder(vec![ProvinceUnitOrderRow {
            province: 100,
            mixed: vec![3, 900, 2, 1],
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::DivisionTemplateIcons(
            [(1, 1, 2), (9, 2, 3)]
                .into_iter()
                .map(|(id, token, count)| DivisionTemplateIconRow {
                    template: id << 32 | 7,
                    country: 11,
                    subunits: vec![TemplateSubunit {
                        token,
                        local: 0,
                        count,
                    }],
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 1,
                literal: b"Foot".to_vec(),
            },
            LexicalTokenRow {
                token: 2,
                literal: b"Horse".to_vec(),
            },
        ]),
    );
    put(&mut frame, TableRows::Clock(vec![ClockRow::default()]));
    frame
}

#[test]
fn counter_members_use_the_shared_summary_without_other_province_units() {
    content_reader_script::test_game::enter();
    let frame = frame();
    let content = content();
    let links = BTreeMap::from([(11, 1), (22, 2)]);
    let canonical = |raw| links.get(&raw).copied();
    let vision = Vision::sharing(1, [2]);
    let loc = loc();
    let prepared = prepare_members(&frame, &[2], &canonical, &vision).unwrap();
    assert_eq!(
        summary_prepared(&content, &frame, &prepared, &canonical, &vision, &loc, true).unwrap(),
        "3\u{13}unit_Horse_icon_small|2 "
    );
    let prepared = prepare_members(&frame, &[1, 3], &canonical, &vision).unwrap();
    assert_eq!(
        summary_prepared(
            &content, &frame, &prepared, &canonical, &vision, &loc, false,
        )
        .unwrap(),
        "4\u{13}unit_Foot_icon_small|1 "
    );
    let province = summary(&content, &frame, 100, &links, &vision, &loc, false).unwrap();
    let prepared = prepare_members(&frame, &[3, 2, 1], &canonical, &vision).unwrap();
    assert_eq!(
        province,
        summary_prepared(
            &content, &frame, &prepared, &canonical, &vision, &loc, false,
        )
        .unwrap()
    );
    assert_eq!(
        province,
        "4\u{13}unit_Foot_icon_small|1    3\u{13}unit_Horse_icon_small|1 "
    );
}

#[test]
fn shared_preparation_keeps_hidden_first_input_disclosure_before_sorting() {
    let frame = frame();
    let links = BTreeMap::from([(11, 1), (22, 2)]);
    let canonical = |raw| links.get(&raw).copied();
    let vision = Vision::sharing(1, []).with_truncated_unit_intel([(2, 2), (3, 1)]);
    let prepared = prepare_members(&frame, &[3, 1, 2], &canonical, &vision).unwrap();
    assert!(!prepared.disclosed);
    assert_eq!(prepared.groups, [(44 << 32 | 7, 2)]);
    let prepared = prepare_members(&frame, &[1, 3, 2], &canonical, &vision).unwrap();
    assert!(prepared.disclosed);
    assert_eq!(prepared.groups, [(1 << 32 | 7, 1), (9 << 32 | 7, 1)]);
}

#[test]
fn exact_counter_members_refuse_missing_inputs_without_a_province_label() {
    let frame = frame();
    let links = BTreeMap::from([(11, 1), (22, 2)]);
    let canonical = |raw| links.get(&raw).copied();
    let vision = Vision::sharing(1, []).with_truncated_unit_intel([(2, 2)]);
    assert_eq!(
        prepare_members(&frame, &[1, 900], &canonical, &vision).unwrap_err(),
        "Divisions member 900 is absent"
    );
    assert_eq!(
        prepare_members(&frame, &[1, 3], &canonical, &vision).unwrap_err(),
        "division 3 unit intel unavailable"
    );
    assert_eq!(
        prepare_members(&frame, &[1], &|_| None, &vision).unwrap_err(),
        "country 11 has no CountryLinks row"
    );
}

#[test]
fn composition_aggregates_globally_in_first_encounter_and_keeps_clones_separate() {
    content_reader_script::test_game::enter();
    let content = content();
    let token_rows = [
        LexicalTokenRow {
            token: 1,
            literal: b"foot".to_vec(),
        },
        LexicalTokenRow {
            token: 2,
            literal: b"Horse".to_vec(),
        },
    ];
    let tokens = TokenNames::from_rows(&token_rows, true);
    let template = |entries| DivisionTemplateIconRow {
        subunits: entries,
        ..Default::default()
    };
    let subunit = |token, local, count| TemplateSubunit {
        token,
        local,
        count,
    };
    let mut entries = Vec::new();
    let mut indices = BTreeMap::new();
    aggregate(
        &content,
        &template(vec![subunit(2, 0, 1), subunit(1, 0, 2)]),
        3,
        tokens,
        1,
        100_000,
        0,
        60_000_000,
        &mut entries,
        &mut indices,
    )
    .unwrap();
    aggregate(
        &content,
        &template(vec![subunit(1, 0, 4), subunit(2, 1, 2)]),
        2,
        tokens,
        2,
        100_000,
        0,
        60_000_000,
        &mut entries,
        &mut indices,
    )
    .unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|e| (e.name.as_str(), e.bounds))
            .collect::<Vec<_>>(),
        [("Horse", [3, 3]), ("Foot", [14, 14]), ("Horse", [4, 4])]
    );
    assert_eq!(
        format_entries(&entries, &loc(), false).unwrap(),
        "3\u{13}unit_Horse_icon_small|1    14\u{13}unit_Foot_icon_small|1    4\u{13}unit_Horse_icon_small|1 "
    );
}

#[test]
fn native_type_mask_and_nato_frame_select_ship_prefix_and_native_range_key() {
    content_reader_script::test_game::enter();
    let content = content();
    let entries = [Entry {
        name: "Boat".into(),
        category: content.subunits.category(b"Boat").unwrap(),
        bounds: [2, 5],
    }];
    assert_eq!(
        format_entries(&entries, &loc(), true).unwrap(),
        "2-5\u{13}ship_Boat_icon_small|2 "
    );
}

#[test]
fn count_ranges_are_exact_at_full_intel_and_clamp_nonpositive_counts() {
    content_reader_script::test_game::enter();
    let d = content().defines;
    assert_eq!(count_range(9, 1, 75_000, 123, 60_827_914, d), [9, 9]);
    assert_eq!(count_range(-9, 1, 0, 123, 60_827_914, d), [0, 0]);
    assert_eq!(count_range(0, 1, 0, 123, 60_827_914, d), [0, 0]);
    assert_eq!(
        count_range(
            9,
            1,
            0,
            123,
            60_827_914,
            CountDefines {
                maximum: d.minimum,
                ..d
            }
        ),
        [9, 9]
    );
    let [min, max] = count_range(1, 1, 0, 123, 60_827_914, d);
    assert_eq!(min, 1);
    assert!(max >= 1);
}

#[test]
fn count_ranges_depend_on_native_month_seed_and_subunit_index() {
    content_reader_script::test_game::enter();
    let d = content().defines;
    let range = count_range(100, 1, 40_000, 123, 60_827_914, d);
    assert!(range[0] < 100 && range[1] > 100);
    assert_ne!(range, count_range(100, 2, 40_000, 123, 60_827_914, d));
    assert_ne!(range, count_range(100, 1, 40_000, 124, 60_827_914, d));
    assert_eq!(range, count_range(100, 1, 40_000, 123, 60_827_915, d));
}

#[test]
fn seeded_count_range_keeps_wrapping_native_salt() {
    content_reader_script::test_game::enter();
    let d = content().defines;
    let law = [d.minimum, d.maximum, d.range];
    let hour = 60_827_914;
    assert_eq!(
        count_range(100, u32::MAX, 40_000, -1, hour, d),
        count_range_seeded(100, 40_000, 0x4A0, hour, law),
    );
    assert_eq!(
        count_range_seeded(100, 40_000, u32::MAX.wrapping_add(0x5E2), hour, law),
        count_range_seeded(100, 40_000, 0x5E1, hour, law),
    );
}

#[test]
fn content_reader_keeps_native_category_reset_and_unknown_tail_refusals() {
    content_reader_script::test_game::enter();
    let mut content = content();
    assert_eq!(
        content.subunits.category(b"Boat").unwrap(),
        content_reader_script::test_game::category("screen_ship")
    );
    content
        .subunits
        .read(b"sub_units={boat={type=infantry}}", "replaced.txt");
    assert_eq!(
        content.subunits.category(b"Boat").unwrap(),
        content_reader_script::test_game::category("infantry")
    );
    content.subunits.read(
        b"sub_units={bad={weights={x @variable=1}}}",
        "unsupported.txt",
    );
    assert!(
        content
            .subunits
            .category(b"Boat")
            .unwrap_err()
            .contains("unsupported.txt")
    );
}

#[test]
fn land_unit_byte_does_not_turn_truncated_unknown_intel_into_a_full_summary() {
    let vision = Vision::sharing(1, []).with_truncated_unit_intel([(7, 2)]);
    assert_eq!(vision.land_byte_known(7, 2, 3), Some(250));
    assert_eq!(vision.land_byte_known(8, 2, 3), None);
    assert_eq!(vision.land_byte_known(8, 1, 3), Some(255));
}
