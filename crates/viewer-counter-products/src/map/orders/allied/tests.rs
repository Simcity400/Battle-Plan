use super::*;
use replay_data::{
    rows::FactionMemberRow, rows::LandTheatreRow, rows::Recorded, rows::ReservedCountryNamingRow,
    rows::SentVolunteerRow, rows::SubjectRow, rows::TableRows, rows::WarRelationRow,
    rows::ZeroSubjectInputs,
};
use replay_frame::{frame::Metrics, frame::TableView};

fn frame(roster: &[u32]) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 7,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
        }),
    };
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                country_array: Recorded::Value(Some(roster.to_vec())),
                ..Default::default()
            },
        ]));
    frame
}

fn links() -> BTreeMap<u32, u32> {
    [
        (101, 1),
        (102, 2),
        (103, 3),
        (104, 4),
        (105, 5),
        (106, 6),
        (501, 1),
        (777, 6),
    ]
    .into_iter()
    .collect()
}

fn faction(frame: &mut DecodedFrame, countries: &[u32]) {
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(
            countries
                .iter()
                .enumerate()
                .map(|(ordinal, &country)| FactionMemberRow {
                    country,
                    faction: 909,
                    ordinal: ordinal as u32,
                })
                .collect(),
        ));
}

fn volunteers(frame: &mut DecodedFrame, country: u32, host: i32) {
    *frame.tables[TableId::SentVolunteers.index()] =
        TableView::Complete(TableRows::SentVolunteers(vec![SentVolunteerRow {
            country,
            ordinal: 0,
            division: 0, // Native tests the list count, not reference resolution.
        }]));
    *frame.tables[TableId::LandTheatres.index()] =
        TableView::Complete(TableRows::LandTheatres(vec![LandTheatreRow {
            owner: country,
            volunteers_theatre: host,
            ..Default::default()
        }]));
}

fn unavailable(frame: &mut DecodedFrame, table: TableId) {
    frame.tables[table.index()] = TableView::Unavailable {
        reason: "fixture absent".into(),
    }
    .into();
}

#[test]
fn disabled_allied_plans_read_no_relation_or_roster_inputs() {
    let mut frame = frame(&[]);
    for &table in TABLES.iter().chain([&TableId::LandTheatres]) {
        unavailable(&mut frame, table);
    }
    assert_eq!(
        Selection::read(&frame, &BTreeMap::new(), 73, false),
        Selection {
            countries: vec![73],
            ..Default::default()
        }
    );
}

#[test]
fn allied_country_walk_keeps_physical_order_duplicates_and_own_alias_suppression() {
    let mut frame = frame(&[0, 106, 103, 501, 102, 106, 104, 105]);
    faction(&mut frame, &[101, 102, 104]);
    *frame.tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(vec![
        SubjectRow {
            overlord: 101,
            subject: 103,
            ..Default::default()
        },
        SubjectRow {
            overlord: 103,
            subject: 105, // Transitive subjects do not qualify.
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
            first_country: 101,
            second_country: 104,
            // Date/cancel metadata does not override the active relation census.
            start_hour: 900,
            end_hour: 1,
            cancel: 1,
            ..Default::default()
        }]));
    volunteers(&mut frame, 501, 777);
    let selected = Selection::read(&frame, &links(), 1, true);
    assert_eq!(selected.countries, [1, 6, 3, 2, 6]);
    assert!(selected.refused.is_empty());
}

#[test]
fn volunteer_host_arm_is_directional_and_overrides_full_visibility_war_veto() {
    let mut frame = frame(&[0, 101, 106]);
    *frame.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
            first_country: 101,
            second_country: 106,
            ..Default::default()
        }]));
    volunteers(&mut frame, 101, 106);
    assert_eq!(Selection::read(&frame, &links(), 1, true).countries, [1, 6]);
    volunteers(&mut frame, 106, 101);
    assert_eq!(Selection::read(&frame, &links(), 1, true).countries, [1]);
}

#[test]
fn known_full_visibility_does_not_require_volunteer_inputs() {
    let mut frame = frame(&[0, 101, 102]);
    faction(&mut frame, &[101, 102]);
    unavailable(&mut frame, TableId::SentVolunteers);
    unavailable(&mut frame, TableId::LandTheatres);
    unavailable(&mut frame, TableId::Subjects);
    let selected = Selection::read(&frame, &links(), 1, true);
    assert_eq!(selected.countries, [1, 2]);
    assert!(selected.refused.is_empty());
}

#[test]
fn known_volunteer_host_does_not_require_full_visibility_inputs() {
    let mut frame = frame(&[0, 101, 106]);
    volunteers(&mut frame, 101, 106);
    unavailable(&mut frame, TableId::Subjects);
    let selected = Selection::read(&frame, &links(), 1, true);
    assert_eq!(selected.countries, [1, 6]);
    assert!(selected.refused.is_empty());
}

#[test]
fn empty_sent_list_is_known_false_and_does_not_read_theatres() {
    let mut frame = frame(&[0, 101, 106]);
    unavailable(&mut frame, TableId::LandTheatres);
    let selected = Selection::read(&frame, &links(), 1, true);
    assert_eq!(selected.countries, [1]);
    assert!(selected.refused.is_empty());
    unavailable(&mut frame, TableId::SentVolunteers);
    let selected = Selection::read(&frame, &links(), 1, true);
    assert_eq!(selected.countries, [1]);
    assert_eq!(
        selected.refused["allied-plan SentVolunteers count unavailable"],
        1
    );
}

#[test]
fn unknown_sent_owner_is_not_an_empty_viewer_count() {
    let mut frame = frame(&[0, 101, 106]);
    *frame.tables[TableId::SentVolunteers.index()] =
        TableView::Complete(TableRows::SentVolunteers(vec![SentVolunteerRow {
            country: 999,
            ..Default::default()
        }]));
    unavailable(&mut frame, TableId::LandTheatres);
    let selected = Selection::read(&frame, &links(), 1, true);
    assert_eq!(selected.countries, [1]);
    assert_eq!(
        selected.refused["allied-plan SentVolunteers owner missing from CountryLinks"],
        1
    );
}

#[test]
fn known_volunteer_match_survives_unrelated_unknown_sent_theatre_and_host_joins() {
    let mut frame = frame(&[0, 101, 106, 102]);
    volunteers(&mut frame, 101, 106);
    let TableView::Complete(TableRows::SentVolunteers(sent)) =
        &mut *frame.tables[TableId::SentVolunteers.index()]
    else {
        panic!("fixture")
    };
    sent.push(SentVolunteerRow {
        country: 999,
        ..Default::default()
    });
    let TableView::Complete(TableRows::LandTheatres(theatres)) =
        &mut *frame.tables[TableId::LandTheatres.index()]
    else {
        panic!("fixture")
    };
    theatres.extend([
        LandTheatreRow {
            id: 2,
            owner: 101,
            volunteers_theatre: 999,
            ..Default::default()
        },
        LandTheatreRow {
            id: 3,
            owner: 999,
            volunteers_theatre: 102,
            ..Default::default()
        },
    ]);
    let selected = Selection::read(&frame, &links(), 1, true);
    assert_eq!(selected.countries, [1, 6]);
    assert_eq!(
        selected.refused["allied-plan volunteer theatre host missing from CountryLinks"],
        1
    );
}

#[test]
fn reserved_theatre_owner_is_known_other_for_positive_volunteer_arm() {
    let mut frame = frame(&[0, 101, 102]);
    volunteers(&mut frame, 101, 0);
    let TableView::Complete(TableRows::LandTheatres(theatres)) =
        &mut *frame.tables[TableId::LandTheatres.index()]
    else {
        panic!("fixture")
    };
    theatres.push(LandTheatreRow {
        id: 2,
        owner: 0,
        volunteers_theatre: 999,
        ..Default::default()
    });
    let selected = Selection::read(&frame, &links(), 1, true);
    assert_eq!(selected.countries, [1]);
    assert!(selected.refused.is_empty());
}

fn truncate(frame: &mut DecodedFrame, table: TableId) {
    let TableView::Complete(rows) = &*frame.tables[table.index()] else {
        panic!("fixture")
    };
    let rows = rows.clone();
    *frame.tables[table.index()] = TableView::Truncated { rows, missing: 1 };
}

#[test]
fn volunteer_prefix_match_proves_true_but_omitted_rows_cannot_prove_absence() {
    let mut frame = frame(&[0, 101, 106, 102]);
    volunteers(&mut frame, 101, 106);
    truncate(&mut frame, TableId::SentVolunteers);
    truncate(&mut frame, TableId::LandTheatres);
    let selected = Selection::read(&frame, &links(), 1, true);
    assert_eq!(selected.countries, [1, 6]);
    assert_eq!(
        selected.refused["allied-plan volunteer theatre roster unavailable"],
        1
    );
    *frame.tables[TableId::SentVolunteers.index()] = TableView::Truncated {
        rows: TableRows::SentVolunteers(Vec::new()),
        missing: 1,
    };
    let selected = Selection::read(&frame, &links(), 1, true);
    assert_eq!(selected.countries, [1]);
    assert_eq!(
        selected.refused["allied-plan SentVolunteers count unavailable"],
        2
    );
}

#[test]
fn historical_or_refused_physical_roster_keeps_own_plan_with_named_gap() {
    for country_array in [Recorded::NotRecorded, Recorded::Value(None)] {
        let mut frame = frame(&[0, 101, 102]);
        *frame.tables[TableId::ReservedCountryNaming.index()] =
            TableView::Complete(TableRows::ReservedCountryNaming(vec![
                ReservedCountryNamingRow {
                    country_array,
                    ..Default::default()
                },
            ]));
        let selected = Selection::read(&frame, &links(), 1, true);
        assert_eq!(selected.countries, [1]);
        assert_eq!(selected.refused.values().sum::<usize>(), 1);
        assert!(
            selected
                .refused
                .keys()
                .all(|key| key.contains("country_array"))
        );
    }
}

#[test]
fn physical_reserved_alias_requires_genuine_viewer_operands() {
    let frame = frame(&[0, 101, 0, 0]);
    let selected = Selection::read(&frame, &links(), 1, true);
    assert_eq!(selected.countries, [1]);
    assert_eq!(
        selected.refused["allied-plan reserved-country viewer operands missing"],
        2
    );
}

fn zero_inputs(
    frame: &mut DecodedFrame,
    war: Recorded<Option<[u8; 2]>>,
    subject: Recorded<Option<ZeroSubjectInputs>>,
    faction: Recorded<Option<u64>>,
) {
    let roster = OriginalTagLists::read(frame)
        .country_array()
        .unwrap()
        .to_vec();
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                country_array: Recorded::Value(Some(roster)),
                faction,
                land_theatre_count: Recorded::Value(Some(0)),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(vec![
            CountryCounterInputRow {
                forward_visibility: replay_data::rows::Recorded::NotRecorded,
                country: 101,
                zero_war: war,
                zero_subject: subject,
                ..Default::default()
            },
        ]));
}

#[test]
fn active_zero_war_is_known_false_without_faction_subject_or_theatres() {
    let mut frame = frame(&[0, 101, 0]);
    zero_inputs(
        &mut frame,
        Recorded::Value(Some([1, 0])),
        Recorded::NotRecorded,
        Recorded::NotRecorded,
    );
    unavailable(&mut frame, TableId::FactionMembers);
    unavailable(&mut frame, TableId::Subjects);
    unavailable(&mut frame, TableId::LandTheatres);
    let selected = Selection::read(&frame, &links(), 1, true);
    assert_eq!(selected.countries, [1]);
    assert!(selected.refused.is_empty());
}

#[test]
fn same_reserved_faction_admits_repeated_occurrences_and_complete_empty_theatres() {
    let mut frame = frame(&[0, 101, 0, 0]);
    faction(&mut frame, &[101]);
    zero_inputs(
        &mut frame,
        Recorded::Value(Some([1, 17])),
        Recorded::NotRecorded,
        Recorded::Value(Some(909)),
    );
    let selected = Selection::read(&frame, &links(), 1, true);
    assert_eq!(selected.countries, [1, 0, 0]);
    assert!(selected.refused.is_empty());
}

#[test]
fn genuine_zero_subject_can_succeed_when_faction_is_unknown() {
    let mut frame = frame(&[0, 101, 0]);
    zero_inputs(
        &mut frame,
        Recorded::Value(Some([0, 0])),
        Recorded::Value(Some(ZeroSubjectInputs {
            relation_type: Some(engine_native::csubject_relation::TYPE_VALUE),
            parties: Some([Some(106), Some(0)]),
        })),
        Recorded::NotRecorded,
    );
    unavailable(&mut frame, TableId::FactionMembers);
    let selected = Selection::read(&frame, &links(), 1, true);
    assert_eq!(selected.countries, [1, 0]);
    assert!(selected.refused.is_empty());
    unavailable(&mut frame, TableId::LandTheatres);
    let selected = Selection::read(&frame, &links(), 1, true);
    assert_eq!(selected.countries, [1]);
    assert_eq!(
        selected.refused["allied-plan admitted reserved-country theatre roster unavailable"],
        1
    );
}

#[test]
fn reserved_subject_null_wrong_type_and_nonzero_parties_are_known_false() {
    for subject in [
        ZeroSubjectInputs::default(),
        ZeroSubjectInputs {
            relation_type: Some(0x380a),
            parties: None,
        },
        ZeroSubjectInputs {
            relation_type: Some(engine_native::csubject_relation::TYPE_VALUE),
            parties: Some([Some(101), Some(501)]),
        },
    ] {
        let mut frame = frame(&[0, 101, 0]);
        zero_inputs(
            &mut frame,
            Recorded::Value(Some([0, 0])),
            Recorded::Value(Some(subject)),
            Recorded::NotRecorded,
        );
        unavailable(&mut frame, TableId::LandTheatres);
        let selected = Selection::read(&frame, &links(), 1, true);
        assert_eq!(selected.countries, [1]);
        assert!(selected.refused.is_empty());
    }
}

#[test]
fn unknown_zero_war_cannot_be_overridden_by_family_and_missing_fields_stay_named() {
    for war in [Recorded::NotRecorded, Recorded::Value(None)] {
        let mut frame = frame(&[0, 101, 0]);
        faction(&mut frame, &[101]);
        zero_inputs(
            &mut frame,
            war,
            Recorded::Value(Some(ZeroSubjectInputs {
                relation_type: Some(engine_native::csubject_relation::TYPE_VALUE),
                parties: Some([Some(0), None]),
            })),
            Recorded::Value(Some(909)),
        );
        let selected = Selection::read(&frame, &links(), 1, true);
        assert_eq!(selected.countries, [1]);
        assert_eq!(selected.refused.values().sum::<usize>(), 1);
        assert!(selected.refused.keys().all(|key| key.contains("zero_war")));
    }
}

#[test]
fn zero_subject_refusal_is_not_native_null_and_inconsistent_branches_are_refused() {
    for subject in [
        Recorded::NotRecorded,
        Recorded::Value(None),
        Recorded::Value(Some(ZeroSubjectInputs {
            relation_type: Some(engine_native::csubject_relation::TYPE_VALUE),
            parties: None,
        })),
        Recorded::Value(Some(ZeroSubjectInputs {
            relation_type: Some(engine_native::csubject_relation::TYPE_VALUE),
            parties: Some([None, Some(106)]),
        })),
        Recorded::Value(Some(ZeroSubjectInputs {
            relation_type: Some(engine_native::csubject_relation::TYPE_VALUE),
            parties: Some([Some(106), None]),
        })),
    ] {
        let mut frame = frame(&[0, 101, 0]);
        zero_inputs(
            &mut frame,
            Recorded::Value(Some([0, 0])),
            subject,
            Recorded::Value(Some(0)),
        );
        let selected = Selection::read(&frame, &links(), 1, true);
        assert_eq!(selected.countries, [1]);
        assert_eq!(selected.refused.values().sum::<usize>(), 1);
        assert!(
            selected
                .refused
                .keys()
                .all(|key| key.contains("zero_subject"))
        );
    }
}

#[test]
fn one_genuine_zero_party_proves_visibility_without_filling_the_other_word() {
    for parties in [[Some(0), None], [None, Some(0)]] {
        let mut frame = frame(&[0, 101, 0]);
        zero_inputs(
            &mut frame,
            Recorded::Value(Some([0, 0])),
            Recorded::Value(Some(ZeroSubjectInputs {
                relation_type: Some(engine_native::csubject_relation::TYPE_VALUE),
                parties: Some(parties),
            })),
            Recorded::NotRecorded,
        );
        unavailable(&mut frame, TableId::FactionMembers);
        let selected = Selection::read(&frame, &links(), 1, true);
        assert_eq!(selected.countries, [1, 0]);
        assert!(selected.refused.is_empty());
    }
}

#[test]
fn admitted_zero_requires_a_current_complete_count_certificate_without_hiding_positive_ally() {
    for count in [
        Recorded::NotRecorded,
        Recorded::Value(None),
        Recorded::Value(Some(1)),
    ] {
        let mut frame = frame(&[0, 101, 102, 0]);
        faction(&mut frame, &[101, 102]);
        zero_inputs(
            &mut frame,
            Recorded::Value(Some([0, 0])),
            Recorded::Value(Some(ZeroSubjectInputs {
                relation_type: Some(engine_native::csubject_relation::TYPE_VALUE),
                parties: Some([Some(0), None]),
            })),
            Recorded::Value(Some(0)),
        );
        let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
            &mut *frame.tables[TableId::ReservedCountryNaming.index()]
        else {
            panic!("fixture")
        };
        rows[0].land_theatre_count = count;
        let selected = Selection::read(&frame, &links(), 1, true);
        assert_eq!(selected.countries, [1, 2]);
        assert_eq!(selected.refused.values().sum::<usize>(), 1);
        assert!(
            selected
                .refused
                .keys()
                .all(|reason| reason.contains("count") || reason.contains("certificate"))
        );
    }
}

#[test]
fn reserved_theatre_certificate_checks_native_ordinals_and_unique_full_handles() {
    for (case, entries, accepted) in [
        (
            "physical order differs from ordinal order",
            [(30, 1), (10, 0)],
            true,
        ),
        ("missing ordinal one", [(30, 2), (10, 0)], false),
        ("duplicate full handle", [(10, 1), (10, 0)], false),
    ] {
        let theatres = entries
            .map(|(id, ordinal)| LandTheatreRow {
                id,
                ordinal,
                ..Default::default()
            })
            .to_vec();
        let mut frame = frame(&[0, 101, 0, 0]);
        zero_inputs(
            &mut frame,
            Recorded::Value(Some([0, 0])),
            Recorded::Value(Some(ZeroSubjectInputs {
                relation_type: Some(engine_native::csubject_relation::TYPE_VALUE),
                parties: Some([Some(0), None]),
            })),
            Recorded::NotRecorded,
        );
        let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
            &mut *frame.tables[TableId::ReservedCountryNaming.index()]
        else {
            panic!("fixture")
        };
        rows[0].land_theatre_count = Recorded::Value(Some(2));
        *frame.tables[TableId::LandTheatres.index()] =
            TableView::Complete(TableRows::LandTheatres(theatres));
        let selected = Selection::read(&frame, &links(), 1, true);
        if accepted {
            assert_eq!(selected.countries, [1, 0, 0], "{case}");
            assert!(
                selected.refused.is_empty(),
                "{case}: {:?}",
                selected.refused
            );
        } else {
            assert_eq!(selected.countries, [1], "{case}");
            assert_eq!(
                selected.refused["allied-plan reserved-country theatre certificate mismatch"], 2,
                "{case}"
            );
        }
    }
}

#[test]
fn known_positive_subject_survives_unknown_faction_but_active_war_vetoes() {
    let mut frame = frame(&[0, 101, 102]);
    *frame.tables[TableId::Subjects.index()] =
        TableView::Complete(TableRows::Subjects(vec![SubjectRow {
            overlord: 101,
            subject: 102,
            ..Default::default()
        }]));
    unavailable(&mut frame, TableId::FactionMembers);
    unavailable(&mut frame, TableId::SentVolunteers);
    let selected = Selection::read(&frame, &links(), 1, true);
    assert_eq!(selected.countries, [1, 2]);
    assert!(selected.refused.is_empty());
    *frame.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
            first_country: 101,
            second_country: 102,
            ..Default::default()
        }]));
    // A known empty sent list makes the second native OR arm false.
    *frame.tables[TableId::SentVolunteers.index()] =
        TableView::Complete(TableRows::SentVolunteers(Vec::new()));
    assert_eq!(Selection::read(&frame, &links(), 1, true).countries, [1]);
}

#[test]
fn known_direct_subject_prefix_survives_unrelated_join_failure_without_inventing_absence() {
    for truncated in [false, true] {
        let mut frame = frame(&[0, 101, 102, 103]);
        *frame.tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(vec![
            SubjectRow {
                overlord: 101,
                subject: 102,
                ..Default::default()
            },
            SubjectRow {
                overlord: 999,
                subject: 103,
                ..Default::default()
            },
        ]));
        if truncated {
            truncate(&mut frame, TableId::Subjects);
        }
        let selected = Selection::read(&frame, &links(), 1, true);
        assert_eq!(selected.countries, [1, 2]);
        assert_eq!(selected.refused.values().sum::<usize>(), 1);
        assert!(
            selected
                .refused
                .keys()
                .all(|reason| reason.contains("Subjects"))
        );
    }
}

#[test]
fn known_war_prefix_vetoes_without_reaching_family_inputs_but_no_pair_stays_unknown() {
    let mut frame = frame(&[0, 101, 102, 103]);
    *frame.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
            first_country: 101,
            second_country: 102,
            ..Default::default()
        }]));
    truncate(&mut frame, TableId::WarRelations);
    unavailable(&mut frame, TableId::FactionMembers);
    unavailable(&mut frame, TableId::Subjects);
    let selected = Selection::read(&frame, &links(), 1, true);
    assert_eq!(selected.countries, [1]);
    assert_eq!(selected.refused.len(), 1);
    assert_eq!(selected.refused["allied-plan WarRelations unavailable"], 1);
}

#[test]
fn known_faction_prefix_proves_same_pointer_without_inventing_missing_members() {
    let mut frame = frame(&[0, 101, 102, 103]);
    faction(&mut frame, &[101, 102]);
    truncate(&mut frame, TableId::FactionMembers);
    unavailable(&mut frame, TableId::Subjects);
    let selected = Selection::read(&frame, &links(), 1, true);
    assert_eq!(selected.countries, [1, 2]);
    assert_eq!(selected.refused.len(), 1);
    assert_eq!(
        selected.refused["allied-plan FactionMembers unavailable"],
        1
    );
}

#[test]
fn conflicting_faction_is_local_to_that_country_and_unknown_owner_cannot_poison_known_pair() {
    let mut frame = frame(&[0, 101, 102, 104]);
    faction(&mut frame, &[101, 102, 104]);
    let TableView::Complete(TableRows::FactionMembers(rows)) =
        &mut *frame.tables[TableId::FactionMembers.index()]
    else {
        panic!("fixture")
    };
    rows.extend([
        FactionMemberRow {
            country: 502,
            faction: 808,
            ordinal: 0,
        },
        FactionMemberRow {
            country: 999,
            faction: 707,
            ordinal: 0,
        },
    ]);
    let mut links = links();
    links.insert(502, 2);
    let selected = Selection::read(&frame, &links, 1, true);
    assert_eq!(selected.countries, [1, 4]);
    assert_eq!(selected.refused.len(), 1);
    assert_eq!(
        selected.refused["allied-plan FactionMembers has inconsistent faction identity"],
        1
    );
}
