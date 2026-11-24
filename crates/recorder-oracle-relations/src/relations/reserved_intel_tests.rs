use super::*;
use content_reader_save::{CountryEmissionIdentity, SaveDocument};
use replay_data::{rows::Recorded, rows::ReservedCountryIntelSample, rows::TableRows};
use replay_frame::frame::TableView;

const RESERVED_TAG: &[u8] = b"R01";

fn layout(physical: &[u32]) -> Vec<CountryEmissionIdentity> {
    physical
        .iter()
        .skip(1)
        .enumerate()
        .map(|(ordinal, &raw)| CountryEmissionIdentity {
            physical_slot: (ordinal + 1) as u32,
            own_id: raw,
            actual: if raw == 0 {
                ActualCountry::Reserved
            } else {
                ActualCountry::Canonical(7)
            },
            tag: if raw == 0 {
                RESERVED_TAG.to_vec()
            } else {
                b"GER".to_vec()
            },
        })
        .collect()
}

fn source(physical: &[u32], reserved_body: &[u8]) -> Vec<u8> {
    let mut source = br#"HOI4txt
player="GER" date="1936.1.1.1" game_unique_id="campaign" game_unique_seed=7 debug_current_ref_id=65580 unit=4008 order_index=7484 front_index=8230 theatre_index=112 theater_group_index=910 military_deployment_line_index=7783 military_deployment_conveyor_index=736 equipment_variant_index=6284 country_leader_index=1 navy_id=8184 land_combat_id=15224 multiplayer_random_count=-1334833904 version="build"
states={} provinces={} countries={
"#.to_vec();
    for &raw in physical.iter().skip(1) {
        if raw == 0 {
            source.extend_from_slice(RESERVED_TAG);
            source.extend_from_slice(b"={");
            source.extend_from_slice(reserved_body);
            source.extend_from_slice(b"}\n");
        } else {
            source.extend_from_slice(b"GER={}\n");
        }
    }
    source.extend_from_slice(b"}\n");
    source
}

fn project(physical: &[u32], reserved_body: &[u8]) -> io::Result<Save> {
    let document = SaveDocument::read(&source(physical, reserved_body))?;
    document.project(&layout(physical))?.parse()
}

fn fixture(
    physical: Vec<u32>,
    count: u32,
    rows: Vec<CountryIntelRow>,
    reserved_body: &[u8],
) -> (DecodedFrame, Save) {
    let (mut frame, _, _, _) = super::super::super::fixtures::fixture();
    let save = project(&physical, reserved_body).unwrap();
    *frame.tables[TableId::CountryIntel.index()] =
        TableView::Complete(TableRows::CountryIntel(rows));
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: RESERVED_TAG.to_vec(),
                country_array: Recorded::Value(Some(physical)),
                country_intel_sample: Recorded::Value(Some(ReservedCountryIntelSample {
                    sample_hour: frame.hour,
                    vector_count: count,
                })),
                ..Default::default()
            },
        ]));
    (frame, save)
}

fn naming(frame: &mut DecodedFrame) -> &mut ReservedCountryNamingRow {
    let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *frame.tables[TableId::ReservedCountryNaming.index()]
    else {
        unreachable!()
    };
    &mut rows[0]
}

fn rows(frame: &mut DecodedFrame) -> &mut Vec<CountryIntelRow> {
    let TableView::Complete(TableRows::CountryIntel(rows)) =
        &mut *frame.tables[TableId::CountryIntel.index()]
    else {
        unreachable!()
    };
    rows
}

fn run(frame: &DecodedFrame, save: &Save) -> io::Result<Report> {
    let mut report = Report::default();
    compare(frame, save, &mut report)?;
    Ok(report)
}

fn sparse() -> Vec<CountryIntelRow> {
    vec![
        CountryIntelRow {
            owner_country: 0,
            target_index: 0,
            army: -100_001,
            air: 222_222,
            civilian: -333_333,
            navy: 444_444,
        },
        CountryIntelRow {
            owner_country: 0,
            target_index: 2,
            ..Default::default()
        },
    ]
}

const SPARSE_BODY: &[u8] =
    b"intel={intel={{army=-1.00001 airforce=2.22222 civilian=-3.33333 navy=4.44444} {} {}}}";

#[test]
fn no_positive_physical_zero_occurrence_requires_exact_runtime_vector_witness() {
    let (frame, save) = fixture(vec![0, 101], 3, sparse(), b"");
    let report = run(&frame, &save).unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("no actual Country0 occurrence after slot0"))
    );
    assert!(!is_serialized_reserved_tag(&save, RESERVED_TAG));
    assert_eq!(
        report.compared_tables.get("CountryIntel actual owner0"),
        None
    );
}

#[test]
fn one_zero_occurrence_compares_row0_sparse_holes_final_sentinel_and_all_four_lanes() {
    let (frame, save) = fixture(vec![0, 101, 0], 3, sparse(), SPARSE_BODY);
    let report = run(&frame, &save).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["CountryIntel actual owner0"], 3);
    assert!(is_serialized_reserved_tag(&save, RESERVED_TAG));
    for field in ["army", "airforce", "civilian", "navy"] {
        let mut changed = save.clone();
        let record = &mut changed.country_intel.get_mut(RESERVED_TAG).unwrap()[0];
        match field {
            "army" => record.army += 1,
            "airforce" => record.air += 1,
            "civilian" => record.civilian += 1,
            "navy" => record.navy += 1,
            _ => unreachable!(),
        }
        let report = run(&frame, &changed).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains(field));
    }
}

#[test]
fn admitted_count_zero_requires_outer_object_and_omitted_inner_list() {
    let (frame, save) = fixture(vec![0, 101, 0], 0, Vec::new(), b"intel={}");
    assert!(run(&frame, &save).unwrap().differences.is_empty());
    assert!(project(&[0, 101, 0], b"intel={intel={}}").is_err());
    let (_, missing_outer) = fixture(vec![0, 101, 0], 0, Vec::new(), b"");
    let report = run(&frame, &missing_outer).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("absent outer serialized intel object"));
    let (_, nonempty) = fixture(vec![0, 101, 0], 0, Vec::new(), b"intel={intel={{}}}");
    let report = run(&frame, &nonempty).unwrap();
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains(".count"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("intel[0]"))
    );
    let mut refused = frame;
    naming(&mut refused).country_intel_sample = Recorded::Value(None);
    assert!(run(&refused, &save).is_err());
}

#[test]
fn signed_lane_extremes_compare_without_fraction_normalization() {
    let body = b"intel={intel={{army=-92233720368547.75808 airforce=92233720368547.75807 civilian=-0.00001 navy=0.00001}}}";
    let (frame, save) = fixture(
        vec![0, 101, 0],
        1,
        vec![CountryIntelRow {
            owner_country: 0,
            target_index: 0,
            army: i64::MIN,
            air: i64::MAX,
            civilian: -1,
            navy: 1,
        }],
        body,
    );
    assert!(run(&frame, &save).unwrap().differences.is_empty());
}

#[test]
fn repeated_interleaved_actual_zero_emissions_each_compare_every_positional_lane() {
    let physical = vec![0, 101, 0, 102, 0];
    let (frame, save) = fixture(physical.clone(), 3, sparse(), SPARSE_BODY);
    let report = run(&frame, &save).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.compared_tables["CountryIntel actual owner0"], 6);
    let document = SaveDocument::read(&source(&physical, SPARSE_BODY)).unwrap();
    assert!(
        document.parse().is_err(),
        "ordinary parsing must retain duplicate refusals"
    );
    assert!(
        document
            .project(&layout(&physical))
            .unwrap()
            .parse()
            .is_ok()
    );
    let mut changed = save.clone();
    changed.country_intel.get_mut(RESERVED_TAG).unwrap()[0].navy += 1;
    let report = run(&frame, &changed).unwrap();
    assert_eq!(report.differences.len(), 2);
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("emission 1.intel[0].navy"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("emission 3.intel[0].navy"))
    );
    let mut shorter = save;
    shorter.country_intel.get_mut(RESERVED_TAG).unwrap().pop();
    let report = run(&frame, &shorter).unwrap();
    assert_eq!(report.differences.len(), 4);
    assert_eq!(
        report
            .differences
            .iter()
            .filter(|line| line.contains(".count"))
            .count(),
        2
    );
}

#[test]
fn original_occurrence_count_order_tag_and_unequal_payloads_cannot_be_projected_away() {
    let physical = [0, 101, 0, 0];
    let expected = layout(&physical);
    let bytes = source(&physical, SPARSE_BODY);
    let document = SaveDocument::read(&bytes).unwrap();
    assert!(document.project(&expected[..2]).is_err());
    let mut reordered = expected.clone();
    reordered.swap(0, 1);
    assert!(document.project(&reordered).is_err());
    let mut changed_tag = expected.clone();
    changed_tag[1].tag = b"other".to_vec();
    assert!(document.project(&changed_tag).is_err());
    let mut changed_bytes = bytes;
    let first = changed_bytes
        .windows(7)
        .position(|bytes| bytes == b"4.44444")
        .unwrap();
    changed_bytes[first] = b'5';
    let changed_document = SaveDocument::read(&changed_bytes).unwrap();
    assert!(changed_document.project(&expected).is_err());
}

#[test]
fn zero_domain_requires_unique_in_range_rows_and_positive_count_final_sentinel() {
    let (frame, save) = fixture(vec![0, 101, 0], 3, sparse(), SPARSE_BODY);
    let mut missing_final = frame.clone();
    rows(&mut missing_final).pop();
    assert!(run(&missing_final, &save).is_err());
    let mut duplicate = frame.clone();
    let repeated = rows(&mut duplicate)[0].clone();
    rows(&mut duplicate).push(repeated);
    assert!(run(&duplicate, &save).is_err());
    let mut outside = frame.clone();
    rows(&mut outside)[0].target_index = 3;
    assert!(run(&outside, &save).is_err());
    let mut invalid_count = frame.clone();
    naming(&mut invalid_count).country_intel_sample =
        Recorded::Value(Some(ReservedCountryIntelSample {
            sample_hour: frame.hour,
            vector_count: u32::MAX,
        }));
    assert!(run(&invalid_count, &save).is_err());
    let mut count_zero_with_rows = frame.clone();
    naming(&mut count_zero_with_rows).country_intel_sample =
        Recorded::Value(Some(ReservedCountryIntelSample {
            sample_hour: frame.hour,
            vector_count: 0,
        }));
    assert!(run(&count_zero_with_rows, &save).is_err());
}

#[test]
fn absent_refused_or_incomplete_identity_never_manufactures_a_fresh_empty_match() {
    let (frame, save) = fixture(vec![0, 101, 0], 3, sparse(), SPARSE_BODY);
    for value in [Recorded::NotRecorded, Recorded::Value(None)] {
        let mut changed = frame.clone();
        naming(&mut changed).country_intel_sample = value;
        assert!(run(&changed, &save).is_err());
    }
    for value in [
        Recorded::NotRecorded,
        Recorded::Value(None),
        Recorded::Value(Some(vec![101, 0])),
    ] {
        let mut changed = frame.clone();
        naming(&mut changed).country_array = value;
        assert!(run(&changed, &save).is_err());
    }
    let mut unavailable = frame.clone();
    *unavailable.tables[TableId::ReservedCountryNaming.index()] = TableView::Unavailable {
        reason: "required prefix unavailable".into(),
    };
    assert!(run(&unavailable, &save).is_err());
    let mut unprojected = save;
    unprojected.country_emissions = None;
    assert!(run(&frame, &unprojected).is_err());
}

#[test]
fn carried_sample_keeps_original_hour_and_future_sample_refuses() {
    let (mut frame, mut save) = fixture(vec![0, 101, 0], 3, sparse(), SPARSE_BODY);
    naming(&mut frame).country_intel_sample = Recorded::Value(Some(ReservedCountryIntelSample {
        sample_hour: frame.hour - 1,
        vector_count: 3,
    }));
    save.country_intel.get_mut(RESERVED_TAG).unwrap()[0].army += 1;
    let report = run(&frame, &save).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(
        report.carried["actual owner0 country intel"],
        Some(frame.hour - 1)
    );
    assert!(
        !report
            .compared_tables
            .contains_key("CountryIntel actual owner0")
    );
    naming(&mut frame).country_intel_sample = Recorded::Value(Some(ReservedCountryIntelSample {
        sample_hour: frame.hour + 1,
        vector_count: 3,
    }));
    assert!(run(&frame, &save).is_err());
}

#[test]
fn zero_refusal_preserves_unchanged_positive_four_lane_differences() {
    let (mut frame, mut save) = fixture(vec![0, 101, 0], 3, sparse(), SPARSE_BODY);
    naming(&mut frame).country_intel_sample = Recorded::Value(None);
    rows(&mut frame).insert(
        0,
        CountryIntelRow {
            owner_country: 102,
            target_index: 0,
            army: 1,
            air: 2,
            civilian: 3,
            navy: 4,
        },
    );
    save.country_intel.insert(
        b"GER".to_vec(),
        vec![content_reader_save::CountryIntelRecord::default()],
    );
    let context = super::super::super::country_context(&frame).unwrap();
    let mut report = Report::default();
    assert!(
        !report.attempt("reserved country intel sample", |report| compare(
            &frame, &save, report
        ))
    );
    super::super::compare_country_intel(&frame, &save, &context, &mut report).unwrap();
    assert!(
        report
            .failed_comparisons
            .contains_key("reserved country intel sample")
    );
    assert_eq!(report.differences.len(), 4);
    for field in ["army", "airforce", "civilian", "navy"] {
        assert!(
            report
                .differences
                .iter()
                .any(|line| line.contains(&format!("GER.intel[0].{field}")))
        );
    }
}

#[test]
fn largest_admitted_sparse_domain_reports_count_without_dense_allocation() {
    let count = i32::MAX as u32;
    let (frame, save) = fixture(
        vec![0, 101, 0],
        count,
        vec![CountryIntelRow {
            owner_country: 0,
            target_index: count - 1,
            ..Default::default()
        }],
        b"intel={}",
    );
    let report = run(&frame, &save).unwrap();
    assert_eq!(report.differences.len(), 2);
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains(".count"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains(&format!("intel[0..{count}]")))
    );
}

#[test]
fn historical_recordless_sample_stays_named_absence_and_tag_alone_never_bypasses_positive_join() {
    let (mut frame, save) = fixture(vec![0, 101], 0, Vec::new(), b"");
    naming(&mut frame).country_intel_sample = Recorded::NotRecorded;
    let report = run(&frame, &save).unwrap();
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("country_intel_sample not recorded"))
    );
    assert!(report.differences.is_empty());
    let (fresh, _) = fixture(vec![0, 101, 0], 3, sparse(), SPARSE_BODY);
    let document = SaveDocument::read(&source(&[0, 101, 0], SPARSE_BODY)).unwrap();
    let unprojected = document.parse().unwrap();
    assert!(!is_serialized_reserved_tag(&unprojected, RESERVED_TAG));
    let context = super::super::super::country_context(&fresh).unwrap();
    assert!(
        super::super::compare_country_intel(&fresh, &unprojected, &context, &mut Report::default())
            .is_err()
    );
}
