use super::*;
use content_reader_save::{ActualCountry, CountryEmissionIdentity, SaveDocument};
use replay_data::{rows::Recorded, rows::ReservedCountryNamingRow};

fn fixture(
    count: usize,
    saved_field: &str,
    reference: Recorded<Option<i32>>,
    bytes: Recorded<Option<Vec<u8>>>,
) -> (DecodedFrame, Save) {
    let (mut frame, _, _, _) = super::super::fixtures::fixture();
    let countries = format!("RES={{{saved_field}}} ").repeat(count);
    let document = SaveDocument::read(
        format!(
            "HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 \
         debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 \
         theater_group_index=6 military_deployment_line_index=7 \
         military_deployment_conveyor_index=8 equipment_variant_index=9 \
         country_leader_index=10 navy_id=11 land_combat_id=12 \
         multiplayer_random_count=-13 countries={{{countries}}} states={{}} provinces={{}}"
        )
        .as_bytes(),
    )
    .unwrap();
    let layout: Vec<_> = (0..count)
        .map(|ordinal| CountryEmissionIdentity {
            physical_slot: u32::try_from(ordinal + 1).unwrap(),
            own_id: 0,
            actual: ActualCountry::Reserved,
            tag: b"RES".to_vec(),
        })
        .collect();
    let save = document.project(&layout).unwrap().parse().unwrap();
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"RES".to_vec(),
                country_array: Recorded::Value(Some(vec![0; count + 1])),
                original_tag_reference: reference,
                original_tag: bytes,
                ..Default::default()
            },
        ]));
    (frame, save)
}

fn row(frame: &mut DecodedFrame) -> &mut ReservedCountryNamingRow {
    let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *frame.tables[TableId::ReservedCountryNaming.index()]
    else {
        panic!("reserved singleton fixture")
    };
    &mut rows[0]
}

fn run(frame: &DecodedFrame, save: &Save) -> io::Result<Report> {
    let mut report = Report::default();
    reserved_original_tag(frame, save, &mut report)?;
    Ok(report)
}

#[test]
fn every_admitted_physical_alias_compares_positive_text_including_empty() {
    for count in [1, 2] {
        for (saved, bytes) in [
            ("original_tag=\"A\"", b"A".to_vec()),
            ("original_tag=\"\"", Vec::new()),
            ("original_tag=\"A\"", b"A\0suffix".to_vec()),
            ("original_tag=\"\"", b"\0suffix".to_vec()),
        ] {
            let (frame, save) = fixture(
                count,
                saved,
                Recorded::Value(Some(7)),
                Recorded::Value(Some(bytes)),
            );
            let report = run(&frame, &save).unwrap();
            assert!(report.differences.is_empty());
            assert_eq!(report.scalar_checks, count);
            assert_eq!(
                report.compared_tables["reserved original tag saved emissions"],
                count
            );
        }
    }
    for saved in ["", "original_tag=\"B\""] {
        let (frame, save) = fixture(
            2,
            saved,
            Recorded::Value(Some(7)),
            Recorded::Value(Some(b"A".to_vec())),
        );
        let report = run(&frame, &save).unwrap();
        assert_eq!(report.differences.len(), 2);
        for slot in [1, 2] {
            assert!(
                report
                    .differences
                    .iter()
                    .any(|difference| difference.contains(&format!("physical slot {slot}")))
            );
        }
    }
}

#[test]
fn nonpositive_reference_checks_omission_without_demanding_bytes() {
    for reference in [0, -1, i32::MIN] {
        for bytes in [Recorded::Value(None), Recorded::NotRecorded] {
            let (frame, save) = fixture(2, "", Recorded::Value(Some(reference)), bytes.clone());
            let report = run(&frame, &save).unwrap();
            assert!(report.differences.is_empty());
            assert_eq!(report.scalar_checks, 2);
            assert!(
                !report
                    .limitations
                    .contains_key("ReservedCountryNaming.original_tag not recorded")
            );
            let (frame, save) = fixture(
                1,
                "original_tag=\"A\"",
                Recorded::Value(Some(reference)),
                bytes,
            );
            assert_eq!(run(&frame, &save).unwrap().differences.len(), 1);
        }
    }
}

#[test]
fn current_reached_faults_fail_and_historical_fields_remain_unknown() {
    let (mut frame, save) = fixture(
        1,
        "original_tag=\"A\"",
        Recorded::Value(None),
        Recorded::Value(Some(b"A".to_vec())),
    );
    assert_eq!(run(&frame, &save).unwrap().differences.len(), 1);
    row(&mut frame).original_tag_reference = Recorded::NotRecorded;
    let report = run(&frame, &save).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.scalar_checks, 0);
    assert!(
        report
            .limitations
            .contains_key("ReservedCountryNaming.original_tag_reference not recorded")
    );

    row(&mut frame).original_tag_reference = Recorded::Value(Some(7));
    row(&mut frame).original_tag = Recorded::Value(None);
    assert_eq!(run(&frame, &save).unwrap().differences.len(), 1);
    row(&mut frame).original_tag = Recorded::NotRecorded;
    let report = run(&frame, &save).unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .contains_key("ReservedCountryNaming.original_tag not recorded")
    );
    let mut missing = save;
    missing
        .countries
        .get_mut(b"RES".as_slice())
        .unwrap()
        .original_tag = None;
    assert_eq!(run(&frame, &missing).unwrap().differences.len(), 1);
}

#[test]
fn saved_counterpart_requires_the_full_ledger_and_exact_reserved_identity() {
    let (mut frame, save) = fixture(1, "", Recorded::Value(Some(0)), Recorded::Value(None));
    let mut missing = save.clone();
    missing.countries.remove(b"RES".as_slice());
    assert!(run(&frame, &missing).is_err());
    for field in [0, 1, 2] {
        let mut wrong = save.clone();
        let identity = &mut wrong.country_emissions.as_mut().unwrap()[0].identity;
        match field {
            0 => identity.own_id = 7,
            1 => identity.tag = b"OTHER".to_vec(),
            _ => identity.physical_slot = 0,
        }
        assert!(run(&frame, &wrong).is_err());
    }
    let mut unadmitted = save;
    unadmitted.country_emissions = None;
    for physical in [Recorded::Value(Some(vec![0, 0])), Recorded::Value(None)] {
        row(&mut frame).country_array = physical;
        assert!(run(&frame, &unadmitted).is_err());
    }
    row(&mut frame).country_array = Recorded::NotRecorded;
    let report = run(&frame, &unadmitted).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.scalar_checks, 0);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("cannot establish"))
    );
}

#[test]
fn no_reserved_emission_has_no_saved_counterpart_and_does_not_demand_operands() {
    let (frame, save) = fixture(0, "", Recorded::Value(None), Recorded::Value(None));
    let report = run(&frame, &save).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.scalar_checks, 0);
    assert_eq!(
        report.compared_tables["reserved original tag saved emissions"],
        0
    );
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("no positive physical actual0 occurrence"))
    );
}
