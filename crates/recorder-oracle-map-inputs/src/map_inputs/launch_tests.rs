use super::*;
use replay_data::{
    rows::LaunchNavalBase, rows::LaunchNavalBases, rows::ReservedLaunchSources, rows::TableRows,
};
use replay_frame::frame::TableView;

fn fixture(physical: Vec<u32>, countries: &[u8], words: &[i32]) -> (DecodedFrame, Save) {
    let (mut frame, mut save, _, _) = super::super::super::fixtures::fixture();
    save.country_naval_launch = content_reader_save::naval_launch::parse_country_occurrences(
        &content_reader_parser::parser::parse(countries).unwrap().items,
    )
    .unwrap();
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"RESERVED".to_vec(),
                country_array: Recorded::Value(Some(physical)),
                owned_state_count: Recorded::Value(Some(3)),
                launch_sources: Recorded::Value(Some(ReservedLaunchSources {
                    sample_hour: replay_data::rows::PacketHour::Host,
                    naval_bases: LaunchNavalBases::Bases(
                        words
                            .iter()
                            .map(|&word| LaunchNavalBase {
                                base_province: word as u32,
                                country: u32::MAX,
                            })
                            .collect(),
                    ),
                    owned_states: Some(vec![9, 2, 9]),
                })),
                ..Default::default()
            },
        ]));
    (frame, save)
}

fn row(frame: &mut DecodedFrame) -> &mut ReservedCountryNamingRow {
    let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *frame.tables[TableId::ReservedCountryNaming.index()]
    else {
        panic!("reserved singleton fixture");
    };
    &mut rows[0]
}

fn run(frame: &DecodedFrame, save: &Save) -> Report {
    let mut report = Report::default();
    report.attempt("reserved launch", |report| compare(frame, save, report));
    report
}

fn native_null(frame: &mut DecodedFrame) {
    let Recorded::Value(Some(packet)) = &mut row(frame).launch_sources else {
        panic!("launch packet fixture");
    };
    packet.naval_bases = LaunchNavalBases::NativeNull;
}

#[test]
fn native_null_accepts_only_saved_absence_and_reports_the_pointer_limit() {
    for countries in [
        b"RESERVED={} RESERVED={}".as_slice(),
        b"RESERVED={strategic_navy={}} RESERVED={strategic_navy={naval_base={}}}",
    ] {
        let (mut frame, save) = fixture(vec![0, 0, 0], countries, &[]);
        native_null(&mut frame);
        let report = run(&frame, &save);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(report.failed_comparisons.is_empty());
        assert_eq!(
            report.compared_tables["reserved launch native-null saved absence"],
            1
        );
        assert_eq!(
            report.compared_tables["reserved launch saved naval bases"],
            2
        );
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("native-null launch provider"))
        );
    }
}

#[test]
fn native_null_rejects_every_saved_base_word_at_every_reserved_alias() {
    let (mut frame, save) = fixture(vec![0, 0, 101, 0], b"RESERVED={} GER={} RESERVED={}", &[]);
    native_null(&mut frame);
    for ordinal in [0, 2] {
        for word in [0, -1, i32::MIN, 8] {
            let mut save = save.clone();
            save.country_naval_launch[ordinal].naval_bases = Some(
                vec![word]
                    .into_iter()
                    .map(content_reader_save::naval_launch::NavalBaseOccurrence::from)
                    .collect(),
            );
            let report = run(&frame, &save);
            assert!(report.differences.iter().any(|difference| {
                difference.contains(&format!("emission {ordinal}.naval_base"))
                    && difference.contains("native-null provider")
            }));
        }
    }
}

#[test]
fn native_null_without_saved_alias_checks_emissions_and_keeps_positive_bases() {
    let (mut frame, mut save) = fixture(
        vec![0, 101],
        b"GER={strategic_navy={naval_base={8={}}}}",
        &[],
    );
    native_null(&mut frame);
    let report = run(&frame, &save);
    assert!(report.differences.is_empty());
    assert!(report.failed_comparisons.is_empty());
    assert_eq!(
        report.compared_tables["reserved launch native-null saved absence"],
        1
    );
    assert_eq!(
        report.compared_tables["reserved launch saved naval bases"],
        0
    );
    save.country_naval_launch
        .push(CountryNavalLaunchOccurrence {
            ordinal: 1,
            tag: b"RESERVED".to_vec(),
            naval_bases: Some(
                vec![8]
                    .into_iter()
                    .map(content_reader_save::naval_launch::NavalBaseOccurrence::from)
                    .collect(),
            ),
        });
    let report = run(&frame, &save);
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("actual0 country emission count"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("physical country emission count"))
    );
}

#[test]
fn native_null_needs_the_current_hour_and_authenticated_save_join() {
    let (mut frame, mut save) = fixture(vec![0, 0], b"RESERVED={}", &[]);
    native_null(&mut frame);
    let baseline = frame.clone();
    for physical in [
        Recorded::NotRecorded,
        Recorded::Value(None),
        Recorded::Value(Some(vec![1, 0])),
    ] {
        row(&mut frame).country_array = physical;
        let report = run(&frame, &save);
        assert!(
            report
                .failed_comparisons
                .values()
                .any(|reason| reason.contains("authenticated physical country array"))
        );
        assert!(
            !report
                .compared_tables
                .contains_key("reserved launch native-null saved absence")
        );
    }
    frame = baseline;
    save.country_naval_launch[0].tag = b"OTHER".to_vec();
    assert!(
        run(&frame, &save)
            .differences
            .iter()
            .any(|difference| difference.contains("emission 0.tag"))
    );
    save.country_naval_launch[0].ordinal = 1;
    assert!(
        run(&frame, &save)
            .failed_comparisons
            .values()
            .any(|reason| reason.contains("ordinals"))
    );
}

#[test]
fn every_signed_base_word_compares_in_native_order_with_repeats() {
    let words = [8, i32::MIN, 8, 0, -1];
    let (frame, mut save) = fixture(
        vec![0, 0],
        b"RESERVED={strategic_navy={naval_base={8={} -2147483648={} 8={} 0={} -1={}}}}",
        &words,
    );
    let report = run(&frame, &save);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(report.failed_comparisons.is_empty());
    for wrong in [
        vec![8, 8, i32::MIN, 0, -1],
        vec![8, i32::MIN, 0, -1],
        vec![8, i32::MIN, 8, 0, -2],
        vec![8, i32::MIN, 8, 0, -1, 8],
    ] {
        save.country_naval_launch[0].naval_bases = Some(
            wrong
                .into_iter()
                .map(content_reader_save::naval_launch::NavalBaseOccurrence::from)
                .collect(),
        );
        assert!(
            run(&frame, &save)
                .differences
                .iter()
                .any(|difference| difference.contains(".naval_base"))
        );
    }
}

#[test]
fn repeated_physical_aliases_are_compared_at_every_saved_position() {
    let (frame, mut save) = fixture(vec![0, 0, 101, 0], b"RESERVED={strategic_navy={naval_base={7={}}}} GER={} RESERVED={strategic_navy={naval_base={7={}}}}", &[7]);
    let report = run(&frame, &save);
    assert!(report.differences.is_empty());
    assert!(report.failed_comparisons.is_empty());
    assert_eq!(
        report.compared_tables["reserved launch saved naval bases"],
        2
    );
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("duplicate-country/handle refusals"))
    );
    save.country_naval_launch[2].naval_bases = Some(
        vec![8]
            .into_iter()
            .map(content_reader_save::naval_launch::NavalBaseOccurrence::from)
            .collect(),
    );
    assert!(
        run(&frame, &save)
            .differences
            .iter()
            .any(|difference| difference.contains("emission 2.naval_base"))
    );
    save.country_naval_launch.swap(1, 2);
    let report = run(&frame, &save);
    assert!(
        report
            .failed_comparisons
            .values()
            .any(|reason| reason.contains("ordinals"))
    );
}

#[test]
fn admitted_full_alias_ledger_suppresses_only_the_obsolete_unsupported_note() {
    let (frame, mut save) = fixture(vec![0, 0, 0], b"RESERVED={strategic_navy={naval_base={7={}}}} RESERVED={strategic_navy={naval_base={7={}}}}", &[7]);
    save.country_emissions = Some(
        (1..=2)
            .map(|physical_slot| content_reader_save::SavedCountryEmission {
                identity: content_reader_save::CountryEmissionIdentity {
                    physical_slot,
                    own_id: 0,
                    actual: content_reader_save::ActualCountry::Reserved,
                    tag: b"RESERVED".to_vec(),
                },
                intel_present: false,
            })
            .collect(),
    );
    let report = run(&frame, &save);
    assert!(report.differences.is_empty());
    assert_eq!(
        report.compared_tables["reserved launch saved naval bases"],
        2
    );
    assert!(
        !report
            .limitations
            .keys()
            .any(|reason| reason.contains("duplicate-country/handle refusals"))
    );
    save.country_naval_launch[1].naval_bases = Some(
        vec![8]
            .into_iter()
            .map(content_reader_save::naval_launch::NavalBaseOccurrence::from)
            .collect(),
    );
    assert!(
        run(&frame, &save)
            .differences
            .iter()
            .any(|difference| difference.contains("emission 1.naval_base"))
    );
}

#[test]
fn matching_tag_does_not_authenticate_an_alias_or_its_position() {
    let (frame, save) = fixture(vec![0, 101, 0], b"RESERVED={strategic_navy={}} GER={}", &[]);
    let report = run(&frame, &save);
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("positive own-ID"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("emission 1.tag"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("strategic_navy"))
    );
}

#[test]
fn no_positive_physical_alias_has_no_saved_payload_and_never_infers_empty() {
    let (frame, mut save) = fixture(vec![0, 101], b"GER={}", &[13, 13]);
    let report = run(&frame, &save);
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("no saved reserved-navy payload"))
    );
    save.country_naval_launch
        .push(CountryNavalLaunchOccurrence {
            ordinal: 1,
            tag: b"RESERVED".to_vec(),
            naval_bases: Some(
                vec![13, 13]
                    .into_iter()
                    .map(content_reader_save::naval_launch::NavalBaseOccurrence::from)
                    .collect(),
            ),
        });
    let report = run(&frame, &save);
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("physical country emission count"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("actual0 country emission count"))
    );
}

#[test]
fn missing_navy_is_not_empty_and_missing_census_is_not_omission() {
    let (mut frame, mut save) = fixture(vec![0, 0], b"RESERVED={}", &[]);
    assert!(
        run(&frame, &save)
            .differences
            .iter()
            .any(|difference| difference.contains("strategic_navy"))
    );
    save.country_naval_launch[0].naval_bases = Some(vec![]);
    assert!(run(&frame, &save).differences.is_empty());
    for physical in [
        Recorded::NotRecorded,
        Recorded::Value(None),
        Recorded::Value(Some(vec![1, 0])),
    ] {
        row(&mut frame).country_array = physical;
        assert!(
            run(&frame, &save)
                .failed_comparisons
                .values()
                .any(|reason| reason.contains("authenticated physical country array"))
        );
    }
}

#[test]
fn unavailable_domains_fail_independently_and_historical_packet_stays_absent() {
    let (mut frame, save) = fixture(vec![0, 0], b"RESERVED={strategic_navy={}}", &[]);
    row(&mut frame).launch_sources = Recorded::NotRecorded;
    let report = run(&frame, &save);
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .contains_key("ReservedCountryNaming.launch_sources not recorded")
    );
    row(&mut frame).launch_sources = Recorded::Value(None);
    assert_eq!(run(&frame, &save).differences.len(), 1);
    for naval_failed in [true, false] {
        row(&mut frame).launch_sources = Recorded::Value(Some(ReservedLaunchSources {
            sample_hour: replay_data::rows::PacketHour::Host,
            naval_bases: if naval_failed {
                LaunchNavalBases::Unavailable
            } else {
                LaunchNavalBases::Bases(vec![])
            },
            owned_states: if naval_failed {
                Some(vec![9, 2, 9])
            } else {
                None
            },
        }));
        let report = run(&frame, &save);
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains(if naval_failed {
            ".naval_bases"
        } else {
            ".owned_states"
        }));
        assert_eq!(
            report
                .compared_tables
                .contains_key("reserved launch saved naval bases"),
            !naval_failed
        );
    }
}

#[test]
fn count_mismatch_is_not_a_saved_order_check() {
    let (mut frame, save) = fixture(vec![0, 0], b"RESERVED={strategic_navy={}}", &[]);
    for count in [
        Recorded::Value(Some(2)),
        Recorded::Value(Some(-1)),
        Recorded::Value(None),
        Recorded::NotRecorded,
    ] {
        row(&mut frame).owned_state_count = count;
        let report = run(&frame, &save);
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("owned_states.count"))
        );
    }
}

#[test]
fn unrelated_positive_join_failure_keeps_reserved_saved_differences() {
    let (frame, save) = fixture(
        vec![0, 999, 0],
        b"UNKNOWN={} RESERVED={strategic_navy={naval_base={8={}}}}",
        &[7],
    );
    let report = run(&frame, &save);
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("emission 1.naval_base"))
    );
    assert!(
        report
            .failed_comparisons
            .values()
            .any(|reason| reason.contains("unresolved country reference 999"))
    );
}
