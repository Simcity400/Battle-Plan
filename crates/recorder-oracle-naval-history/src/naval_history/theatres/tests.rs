use super::*;
use content_reader_save::Decimal;
use replay_data::{
    rows::ArmyGroupMemberRow, rows::DivisionRow, rows::LandTheatreRow, rows::OrdersGroupMemberRow,
    rows::OrdersGroupRow, rows::ReservedCountryNamingRow, rows::TableId, rows::TableRows,
};
use replay_frame::{frame::Metrics, frame::TableView};

const POSITIVE: &[u8] = br#"countries={PST={theatres={theatre={
    id={id=2 type=67}
    orders_group={id={id=57 type=53} name="positive"
        field_marshal_group=no motorization_level=0
        member={unit={id=111 type=51}}}
}}}}"#;

const SERIALIZED_ZERO: &[u8] = br#"countries={
    PST={theatres={theatre={id={id=2 type=67}
        orders_group={id={id=57 type=53} name="positive"
            field_marshal_group=no motorization_level=0
            member={unit={id=111 type=51}}}}}}
    R00={theatres={theatre={id={id=3 type=67}
        orders_group={id={id=58 type=53} name="zero"
            field_marshal_group=no motorization_level=0
            member={unit={id=112 type=51}}}
        field_marshal_group={id={id=59 type=53} name="zero parent"
            field_marshal_group=yes motorization_level=0
            orders_group={id=58 type=53}}}}}
}"#;

fn handle(id: u64, kind: u64) -> u64 {
    (id << 32) | kind
}

fn frame(alias: bool) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 1,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
        metrics: Metrics::default(),
    };
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"R00".to_vec(),
                country_array: Recorded::Value(Some(if alias {
                    vec![0, 7, 0]
                } else {
                    vec![0, 7]
                })),
                land_theatre_count: Recorded::Value(Some(1)),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::LandTheatres.index()] =
        TableView::Complete(TableRows::LandTheatres(vec![
            LandTheatreRow {
                id: handle(2, 67),
                owner: 7,
                ordinal: 0,
                volunteers_theatre: 0,
            },
            LandTheatreRow {
                id: handle(3, 67),
                owner: 0,
                ordinal: 0,
                volunteers_theatre: 0,
            },
        ]));
    *frame.tables[TableId::OrdersGroups.index()] =
        TableView::Complete(TableRows::OrdersGroups(vec![
            OrdersGroupRow {
                handle: handle(57, 53),
                country: 7,
                theatre: handle(2, 67),
                theatre_ordinal: Some(0),
                name: Recorded::Value(Some(b"positive".to_vec())),
                ..Default::default()
            },
            OrdersGroupRow {
                handle: handle(58, 53),
                country: 9,
                theatre: handle(3, 67),
                theatre_ordinal: Some(0),
                parent: handle(59, 53),
                name: Recorded::Value(Some(b"zero".to_vec())),
                ..Default::default()
            },
            OrdersGroupRow {
                handle: handle(59, 53),
                country: 0,
                theatre: handle(3, 67),
                is_army_group: 1,
                theatre_ordinal: Some(0),
                name: Recorded::Value(Some(b"zero parent".to_vec())),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::OrdersGroupMembers.index()] =
        TableView::Complete(TableRows::OrdersGroupMembers(vec![
            OrdersGroupMemberRow {
                group: handle(57, 53),
                ordinal: 0,
                unit: handle(111, 51),
            },
            OrdersGroupMemberRow {
                group: handle(58, 53),
                ordinal: 0,
                unit: handle(112, 51),
            },
        ]));
    *frame.tables[TableId::ArmyGroupMembers.index()] =
        TableView::Complete(TableRows::ArmyGroupMembers(vec![ArmyGroupMemberRow {
            group: handle(59, 53),
            ordinal: 0,
            child: handle(58, 53),
        }]));
    *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(vec![
        DivisionRow {
            id: handle(111, 51),
            orders_group: handle(57, 53),
            theatre: Some(0).into(),
            ..Default::default()
        },
        DivisionRow {
            id: handle(112, 51),
            orders_group: handle(58, 53),
            theatre: Some(0).into(),
            ..Default::default()
        },
    ]));
    frame
}

fn divisions() -> BTreeMap<u64, Division> {
    [111, 112]
        .into_iter()
        .map(|id| {
            let id = handle(id, 51);
            (
                id,
                Division {
                    id,
                    roster_owner: b"PST".to_vec(),
                    logical_owner: b"PST".to_vec(),
                    expeditionary_owner: None,
                    province: None,
                    template: None,
                    alternate_template: None,
                    separate_counter: false,
                    hit_points: Decimal::parse(b"0").unwrap(),
                    organisation: Decimal::parse(b"0").unwrap(),
                    country_intel: Vec::new(),
                    presentation: Default::default(),
                },
            )
        })
        .collect()
}

fn run(frame: &DecodedFrame, bytes: &[u8]) -> Report {
    let saved = saved::parse(&content_reader_parser::parser::parse(bytes).unwrap().items).unwrap();
    let positive = CountryRow {
        tag: b"PST".to_vec(),
        ..Default::default()
    };
    let other = CountryRow {
        tag: b"OTH".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(7, &positive), (9, &other)]);
    let mut report = Report::default();
    let _ = compare_theatres(frame, &saved, &links, &mut report);
    let _ = compare_order_of_battle(frame, &divisions(), &saved, &links, &mut report);
    report
}

fn limits(report: &Report) -> String {
    report
        .limitations
        .iter()
        .map(|(reason, rows)| format!("{reason}: {rows:?}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn mixed_positive_and_unsaved_zero_payloads_preserve_positive_comparisons() {
    let report = run(&frame(false), POSITIVE);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report.failed_comparisons.is_empty(),
        "{:?}",
        report.failed_comparisons
    );
    assert_eq!(report.compared_tables["LandTheatres"], 1);
    assert_eq!(report.compared_tables["OrdersGroups"], 1);
    assert_eq!(report.compared_tables["OrdersGroupMembers"], 1);
    assert_eq!(report.compared_tables["Divisions.orders_group"], 1);
    assert_eq!(
        report.compared_tables["OrdersGroupMembers against division parent pointers"],
        2
    );
    let limitations = limits(&report);
    assert!(
        limitations.contains("CArmy does not serialize its parent pointer"),
        "{limitations}"
    );
    assert!(limitations.contains("OrdersGroups"), "{limitations}");
}

#[test]
fn unique_physical_zero_alias_compares_its_dynamic_tag_and_all_saved_payloads() {
    let report = run(&frame(true), SERIALIZED_ZERO);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report.failed_comparisons.is_empty(),
        "{:?}",
        report.failed_comparisons
    );
    assert_eq!(report.compared_tables["LandTheatres"], 2);
    assert_eq!(report.compared_tables["OrdersGroups"], 3);
    assert_eq!(report.compared_tables["OrdersGroupMembers"], 2);
    assert_eq!(report.compared_tables["ArmyGroupMembers"], 1);
    assert_eq!(report.compared_tables["Divisions.orders_group"], 2);
    assert!(!limits(&report).contains(UNSAVED_RESERVED_THEATRE));
}

#[test]
fn serialized_zero_member_mismatches_remain_differences() {
    let mut frame = frame(true);
    let TableView::Complete(TableRows::OrdersGroupMembers(rows)) =
        &mut *frame.tables[TableId::OrdersGroupMembers.index()]
    else {
        unreachable!()
    };
    rows[1].unit = handle(113, 51);
    let report = run(&frame, SERIALIZED_ZERO);
    let differences = report.differences.join("\n");
    assert!(
        differences.contains(&format!("orders group {} member 0", handle(58, 53))),
        "{differences}"
    );
    assert!(
        differences.contains("recorded member vector"),
        "{differences}"
    );
}

#[test]
fn a_known_physical_alias_cannot_waive_a_missing_saved_country_or_payload() {
    let report = run(&frame(true), POSITIVE);
    let differences = report.differences.join("\n");
    assert!(
        differences.contains("reserved country physical save census"),
        "{differences}"
    );
    assert!(differences.contains("LandTheatres"), "{differences}");
    assert!(differences.contains("OrdersGroupMembers"), "{differences}");
    assert!(!limits(&report).contains(UNSAVED_RESERVED_THEATRE));
}

#[test]
fn a_serialized_definition_keeps_extra_members_strict_even_with_a_zero_theatre_backlink() {
    let mut frame = frame(false);
    let TableView::Complete(TableRows::OrdersGroups(rows)) =
        &mut *frame.tables[TableId::OrdersGroups.index()]
    else {
        unreachable!()
    };
    rows[0].theatre = handle(3, 67);
    let TableView::Complete(TableRows::OrdersGroupMembers(rows)) =
        &mut *frame.tables[TableId::OrdersGroupMembers.index()]
    else {
        unreachable!()
    };
    rows.push(OrdersGroupMemberRow {
        group: handle(57, 53),
        ordinal: 1,
        unit: handle(113, 51),
    });
    let report = run(&frame, POSITIVE);
    let differences = report.differences.join("\n");
    assert!(
        differences.contains(&format!("orders group {}.theatre", handle(57, 53))),
        "{differences}"
    );
    assert!(
        differences.contains(&format!("OrdersGroupMembers ({}, 1)", handle(57, 53))),
        "{differences}"
    );
}

#[test]
fn a_refused_certificate_keeps_positive_checks_and_zero_extras_strict() {
    for certificate in [Recorded::Value(None), Recorded::NotRecorded] {
        let mut frame = frame(false);
        let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
            &mut *frame.tables[TableId::ReservedCountryNaming.index()]
        else {
            unreachable!()
        };
        rows[0].land_theatre_count = certificate;
        let report = run(&frame, POSITIVE);
        assert!(
            report.failed_comparisons["reserved land theatre serialization"]
                .contains("certificate")
        );
        assert_eq!(report.compared_tables["OrdersGroups"], 1);
        assert!(
            report
                .differences
                .iter()
                .any(|line| line.contains("LandTheatres"))
        );
    }
}

#[test]
fn an_unavailable_physical_roster_cannot_prove_save_omission() {
    let mut frame = frame(false);
    let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *frame.tables[TableId::ReservedCountryNaming.index()]
    else {
        unreachable!()
    };
    rows[0].country_array = Recorded::Value(None);
    let report = run(&frame, POSITIVE);
    assert!(
        report.failed_comparisons["reserved land theatre serialization"]
            .contains("physical country array")
    );
    assert_eq!(report.compared_tables["OrdersGroupMembers"], 1);
    assert!(!limits(&report).contains(UNSAVED_RESERVED_THEATRE));
}

#[test]
fn gaps_in_omitted_member_ordinals_are_refused() {
    let mut frame = frame(false);
    let TableView::Complete(TableRows::OrdersGroupMembers(rows)) =
        &mut *frame.tables[TableId::OrdersGroupMembers.index()]
    else {
        unreachable!()
    };
    rows[1].ordinal = 1;
    let report = run(&frame, POSITIVE);
    assert!(report.failed_comparisons["reserved OrdersGroupMembers"].contains("not contiguous"));
    assert_eq!(report.compared_tables["OrdersGroups"], 1);
    assert_eq!(report.compared_tables["OrdersGroupMembers"], 1);
}

fn empty_zero_frame(alias: bool) -> DecodedFrame {
    let mut frame = frame(alias);
    let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *frame.tables[TableId::ReservedCountryNaming.index()]
    else {
        unreachable!()
    };
    rows[0].land_theatre_count = Recorded::Value(Some(0));
    let TableView::Complete(TableRows::LandTheatres(rows)) =
        &mut *frame.tables[TableId::LandTheatres.index()]
    else {
        unreachable!()
    };
    rows.truncate(1);
    let TableView::Complete(TableRows::OrdersGroups(rows)) =
        &mut *frame.tables[TableId::OrdersGroups.index()]
    else {
        unreachable!()
    };
    rows.truncate(1);
    let TableView::Complete(TableRows::OrdersGroupMembers(rows)) =
        &mut *frame.tables[TableId::OrdersGroupMembers.index()]
    else {
        unreachable!()
    };
    rows.truncate(1);
    *frame.tables[TableId::ArmyGroupMembers.index()] =
        TableView::Complete(TableRows::ArmyGroupMembers(Vec::new()));
    let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *frame.tables[TableId::Divisions.index()]
    else {
        unreachable!()
    };
    rows.truncate(1);
    frame
}

#[test]
fn empty_reserved_rosters_still_compare_unique_alias_country_presence_once() {
    let frame = empty_zero_frame(true);
    let report = run(&frame, POSITIVE);
    let differences = report.differences.join("\n");
    assert_eq!(report.differences.len(), 1, "{differences}");
    assert!(
        differences.contains("reserved country physical save census: replay true, save false"),
        "{differences}"
    );
    assert_eq!(report.compared_tables["OrdersGroupMembers"], 1);
    let mut bytes = POSITIVE.to_vec();
    bytes.pop();
    bytes.extend_from_slice(b" R00={}}");
    let report = run(&frame, &bytes);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report.failed_comparisons.is_empty(),
        "{:?}",
        report.failed_comparisons
    );
}

#[test]
fn empty_reserved_rosters_do_not_deduplicate_repeated_aliases() {
    let mut frame = empty_zero_frame(true);
    let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *frame.tables[TableId::ReservedCountryNaming.index()]
    else {
        unreachable!()
    };
    rows[0].country_array = Recorded::Value(Some(vec![0, 7, 0, 0]));
    let report = run(&frame, POSITIVE);
    assert!(
        report.failed_comparisons["reserved land theatre serialization"]
            .contains("repeated reserved country")
    );
    assert_eq!(report.compared_tables["OrdersGroups"], 1);
}

#[test]
fn a_missing_omitted_member_join_refuses_only_the_zero_validation() {
    let mut frame = frame(false);
    let TableView::Complete(TableRows::OrdersGroupMembers(rows)) =
        &mut *frame.tables[TableId::OrdersGroupMembers.index()]
    else {
        unreachable!()
    };
    rows.push(OrdersGroupMemberRow {
        group: handle(58, 53),
        ordinal: 1,
        unit: handle(113, 51),
    });
    let report = run(&frame, POSITIVE);
    assert!(
        report.failed_comparisons["reserved OrdersGroupMembers"]
            .contains("absent from the complete division census")
    );
    assert_eq!(report.compared_tables["OrdersGroups"], 1);
    assert_eq!(report.compared_tables["OrdersGroupMembers"], 1);
}

#[test]
fn mixed_serialized_and_omitted_ordinals_keep_all_positive_payload_checks() {
    let mut frame = frame(false);
    let TableView::Complete(TableRows::OrdersGroups(rows)) =
        &mut *frame.tables[TableId::OrdersGroups.index()]
    else {
        unreachable!()
    };
    rows[0].theatre = handle(3, 67);
    rows[1].theatre_ordinal = Some(1);
    let report = run(&frame, POSITIVE);
    assert!(
        !report
            .failed_comparisons
            .contains_key("reserved OrdersGroups"),
        "{:?}",
        report.failed_comparisons
    );
    assert_eq!(report.compared_tables["OrdersGroups"], 1);
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains(&format!("orders group {}.theatre", handle(57, 53))))
    );
}

#[test]
fn pure_regular_member_qualification_excludes_all_serialized_payloads_and_checks_parents() {
    let mut frame = frame(false);
    let mut saved = saved::parse(&content_reader_parser::parser::parse(POSITIVE).unwrap().items).unwrap();
    assert_eq!(
        omitted_regular_group_units(&frame, &saved, None).unwrap(),
        BTreeSet::from([handle(112, 51)])
    );
    saved
        .theatre_countries
        .as_mut()
        .unwrap()
        .get_mut(b"PST".as_slice())
        .unwrap()
        .land[0]
        .definitions[0]
        .members
        .push(handle(112, 51));
    assert!(
        omitted_regular_group_units(&frame, &saved, None)
            .unwrap()
            .is_empty()
    );
    saved
        .theatre_countries
        .as_mut()
        .unwrap()
        .get_mut(b"PST".as_slice())
        .unwrap()
        .land[0]
        .definitions[0]
        .members
        .pop();
    let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *frame.tables[TableId::Divisions.index()]
    else {
        unreachable!()
    };
    rows[1].orders_group = 0;
    assert!(
        omitted_regular_group_units(&frame, &saved, None)
            .unwrap_err()
            .to_string()
            .contains("disagree with division parents")
    );
}

#[test]
fn pure_qualification_refuses_missing_reverse_membership_and_duplicate_identities() {
    let saved = saved::parse(&content_reader_parser::parser::parse(POSITIVE).unwrap().items).unwrap();
    let mut missing = frame(false);
    let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *missing.tables[TableId::Divisions.index()]
    else {
        unreachable!()
    };
    rows.push(DivisionRow {
        id: handle(113, 51),
        orders_group: handle(58, 53),
        ..Default::default()
    });
    assert!(
        omitted_regular_group_units(&missing, &saved, None)
            .unwrap_err()
            .to_string()
            .contains("without reciprocal member")
    );
    let mut duplicate = frame(false);
    let TableView::Complete(TableRows::OrdersGroups(rows)) =
        &mut *duplicate.tables[TableId::OrdersGroups.index()]
    else {
        unreachable!()
    };
    rows.push(rows[1].clone());
    assert!(
        omitted_regular_group_units(&duplicate, &saved, None)
            .unwrap_err()
            .to_string()
            .contains("duplicate recorded orders group identity")
    );
    let mut duplicate = frame(false);
    let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *duplicate.tables[TableId::Divisions.index()]
    else {
        unreachable!()
    };
    rows.push(rows[1].clone());
    assert!(
        omitted_regular_group_units(&duplicate, &saved, None)
            .unwrap_err()
            .to_string()
            .contains("duplicate recorded division identity")
    );
}

#[test]
fn pure_qualification_reuses_the_full_affected_theatre_roster_validation() {
    let saved = saved::parse(&content_reader_parser::parser::parse(POSITIVE).unwrap().items).unwrap();
    let mut frame = frame(false);
    let TableView::Complete(TableRows::OrdersGroups(rows)) =
        &mut *frame.tables[TableId::OrdersGroups.index()]
    else {
        unreachable!()
    };
    rows[1].theatre_ordinal = Some(1);
    assert!(
        omitted_regular_group_units(&frame, &saved, None)
            .unwrap_err()
            .to_string()
            .contains("theatre roster ordinals are not contiguous")
    );
    let report = run(&frame, POSITIVE);
    assert!(report.failed_comparisons["reserved OrdersGroups"].contains("not contiguous"));
    assert_eq!(report.compared_tables["OrdersGroups"], 1);
    assert_eq!(report.compared_tables["OrdersGroupMembers"], 1);
}

#[test]
fn unsaved_zero_members_and_parent_pointers_still_cross_check() {
    let mut frame = frame(false);
    let TableView::Complete(TableRows::OrdersGroups(rows)) =
        &mut *frame.tables[TableId::OrdersGroups.index()]
    else {
        unreachable!()
    };
    rows[1].parent = 0;
    let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *frame.tables[TableId::Divisions.index()]
    else {
        unreachable!()
    };
    rows[1].orders_group = 0;
    let report = run(&frame, POSITIVE);
    let differences = report.differences.join("\n");
    assert!(
        differences.contains("parent recorded child vector"),
        "{differences}"
    );
    assert!(
        differences.contains("orders_group recorded member vector"),
        "{differences}"
    );
}

#[test]
fn unresolved_positive_group_countries_remain_refused() {
    let mut frame = frame(false);
    let TableView::Complete(TableRows::OrdersGroups(rows)) =
        &mut *frame.tables[TableId::OrdersGroups.index()]
    else {
        unreachable!()
    };
    rows[0].country = 42;
    let report = run(&frame, POSITIVE);
    assert!(
        report.failed_comparisons["army OrdersGroups"].contains("unresolved country reference 42")
    );
    assert_eq!(report.compared_tables["OrdersGroupMembers"], 1);
}

#[test]
fn repeated_reserved_country_save_tags_require_admitted_full_save_occurrences() {
    let items = content_reader_parser::parser::parse(br#"countries={R00={} R00={}}"#)
        .unwrap()
        .items;
    assert!(
        saved::parse(&items)
            .unwrap_err()
            .to_string()
            .contains("duplicate saved theatre country tag")
    );
    let mut frame = frame(true);
    let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *frame.tables[TableId::ReservedCountryNaming.index()]
    else {
        unreachable!()
    };
    rows[0].country_array = Recorded::Value(Some(vec![0, 7, 0, 0]));
    let report = run(&frame, SERIALIZED_ZERO);
    assert!(
        report.failed_comparisons["reserved land theatre serialization"]
            .contains("repeated reserved country save tags")
    );
    assert_eq!(report.compared_tables["OrdersGroups"], 3);
}

#[test]
fn projected_repeated_reserved_aliases_compare_theatres_and_order_of_battle() {
    use content_reader_save::SaveDocument;
    use replay_data::rows::CountryLinkRow;
    let mut frame = frame(true);
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 7,
            canonical_slot: 1,
            tag: b"PST".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
            id: 7,
            canonical_slot: 1,
        }]));
    let TableView::Complete(TableRows::OrdersGroups(rows)) =
        &mut *frame.tables[TableId::OrdersGroups.index()]
    else {
        unreachable!()
    };
    rows[1].country = 7;
    let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *frame.tables[TableId::ReservedCountryNaming.index()]
    else {
        unreachable!()
    };
    rows[0].country_array = Recorded::Value(Some(vec![0, 7, 0, 0]));
    let positive = r#"theatres={theatre={id={id=2 type=67}
        orders_group={id={id=57 type=53} name="positive"
            field_marshal_group=no motorization_level=0 member={unit={id=111 type=51}}}}}"#;
    let zero = r#"theatres={theatre={id={id=3 type=67}
        orders_group={id={id=58 type=53} name="zero"
            field_marshal_group=no motorization_level=0 member={unit={id=112 type=51}}}
        field_marshal_group={id={id=59 type=53} name="zero parent"
            field_marshal_group=yes motorization_level=0 orders_group={id=58 type=53}}}}"#;
    let document = |first: &str, second: &str| {
        SaveDocument::read(format!(
        "HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 \
         debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 \
         theater_group_index=6 military_deployment_line_index=7 \
         military_deployment_conveyor_index=8 equipment_variant_index=9 \
         country_leader_index=10 navy_id=11 land_combat_id=12 \
         multiplayer_random_count=-13 countries={{PST={{{positive}}} R00={{{first}}} R00={{{second}}}}} \
         states={{}} provinces={{}}"
    ).as_bytes()).unwrap()
    };
    let source = document(zero, zero);
    assert!(source.parse().is_err());
    let saved = recorder_oracle_support::support::parse_country_emissions(&source, &frame).unwrap();
    let positive = frame.complete::<tables::Countries>().unwrap()[0].clone();
    let links = BTreeMap::from([(7, &positive)]);
    let mut report = Report::default();
    report.record_country_emissions(&saved);
    compare_theatres(&frame, &saved.naval_history, &links, &mut report).unwrap();
    compare_order_of_battle(
        &frame,
        &divisions(),
        &saved.naval_history,
        &links,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["LandTheatres"], 2);
    assert_eq!(report.compared_tables["OrdersGroups"], 3);
    assert!(
        omitted_regular_group_units(
            &frame,
            &saved.naval_history,
            saved.country_emissions.as_deref()
        )
        .unwrap()
        .is_empty()
    );
    let changed = zero.replace("name=\"zero\"", "name=\"changed\"");
    for source in [document(&changed, zero), document(zero, &changed)] {
        assert!(
            recorder_oracle_support::support::parse_country_emissions(&source, &frame)
                .unwrap_err()
                .to_string()
                .contains("differ at")
        );
    }
}

#[test]
fn unit_theatres_compare_with_saved_theatre_unit_lists() {
    const STATIONED: &[u8] = br#"countries={PST={theatres={theatre={
    id={id=2 type=67}
    unit={id=111 type=51}
    orders_group={id={id=57 type=53} name="positive"
        field_marshal_group=no motorization_level=0
        member={unit={id=111 type=51}}}
}}}}"#;
    let set = |frame: &mut DecodedFrame, theatres: [Recorded<Option<u64>>; 2]| {
        let TableView::Complete(TableRows::Divisions(rows)) =
            &mut *frame.tables[TableId::Divisions.index()]
        else {
            unreachable!()
        };
        for (row, theatre) in rows.iter_mut().zip(theatres) {
            row.theatre = theatre;
        }
    };
    let theatre_differences = |report: &Report| {
        report
            .differences
            .iter()
            .filter(|difference| difference.contains(".theatre"))
            .count()
    };
    let mut frame = frame(false);
    let report = run(&frame, STATIONED);
    assert_eq!(theatre_differences(&report), 1, "{:?}", report.differences);
    set(&mut frame, [Some(handle(2, 67)).into(), Some(0).into()]);
    let report = run(&frame, STATIONED);
    assert_eq!(theatre_differences(&report), 0, "{:?}", report.differences);
    assert_eq!(report.compared_tables["Divisions.theatre"], 2);
    set(&mut frame, [None.into(), Some(handle(3, 67)).into()]);
    let report = run(&frame, STATIONED);
    assert_eq!(theatre_differences(&report), 1, "{:?}", report.differences);
    assert!(limits(&report).contains(UNSAVED_RESERVED_THEATRE));
    set(&mut frame, [Recorded::NotRecorded, Recorded::NotRecorded]);
    let report = run(&frame, STATIONED);
    assert_eq!(theatre_differences(&report), 0, "{:?}", report.differences);
    assert!(limits(&report).contains("Divisions.theatre"));
}
