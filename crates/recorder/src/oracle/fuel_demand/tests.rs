use super::*;
use installed_content::save;
use replay::{
    frame::TableView,
    rows::{
        CountryFuelRow, FuelDemandFleet, FuelDemandSite, FuelDemandUnit, FuelDemandWing, TableId,
        TableRows,
    },
};

#[test]
fn conditional_names_distinguish_omitted_and_emitted_empty_save_text() {
    for (actual, saved) in [
        (&b""[..], None),
        (&b"\0retained suffix"[..], Some(&b""[..])),
        (&b"command\0retained suffix"[..], Some(&b"command"[..])),
    ] {
        let mut report = Report::default();
        conditional_name(&mut report, "stored name", Some(actual), saved).unwrap();
        assert!(report.differences.is_empty());
    }
    for (actual, saved) in [
        (&b""[..], Some(&b""[..])),
        (&b"\0retained suffix"[..], None),
        (&b"command"[..], Some(&b"other"[..])),
    ] {
        let mut report = Report::default();
        conditional_name(&mut report, "stored name", Some(actual), saved).unwrap();
        assert_eq!(report.differences.len(), 1);
    }
    assert!(conditional_name(&mut Report::default(), "stored name", None, None).is_err());
}

#[test]
fn skipped_zero_wing_owner_witness_keeps_override_fallback_and_failure_independent_of_rate() {
    let (mut frame, mut saved, _, _) = super::super::tests::fixture();
    let id = 7_u64 << 32 | 69;
    saved.air_wings.insert(
        id,
        save::AirWing {
            id,
            owner: b"FRA".to_vec(),
            name: Some(Vec::new()),
            country_override: b"GER".to_vec(),
            air_base: 0,
            planes: 0,
            mission_mask: 0,
            mission_region: None,
            mission_active: None,
            region_change_penalty: None,
            pool_ordinal: 0,
        },
    );
    let country = complete::<tables::Countries>(&frame).unwrap()[0].clone();
    let mut other = country.clone();
    other.tag = b"FRA".to_vec();
    let links = BTreeMap::from([(101, &country), (201, &other)]);
    let row = CountryFuelRow {
        country: 101,
        country_instance: Recorded::Value(Some(9)),
        demand: Recorded::Value(FuelDemandRead {
            hour: frame.hour,
            country_instance: Some(9),
            sites: vec![FuelDemandSite {
                wings: vec![FuelDemandWing {
                    reference: id,
                    owner: Some(101),
                    country_override: Some(101),
                    name: Some(Vec::new()),
                    inputs: Some(Default::default()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    };
    let run = |frame: &mut DecodedFrame, saved: &Save, row: CountryFuelRow| {
        let Recorded::Value(pack) = &row.demand else {
            panic!()
        };
        assert_eq!(pack.native_rates(), [Ok(0), Ok(0), Ok(0)]);
        *frame.tables[TableId::CountryFuel.index()] =
            TableView::Complete(TableRows::CountryFuel(vec![row]));
        let mut report = Report::default();
        let result = compare(frame, saved, &links, &mut report);
        (result, report)
    };
    let (result, report) = run(&mut frame, &saved, row.clone());
    result.unwrap();
    assert!(report.differences.is_empty());
    let mut mismatch = row.clone();
    let Recorded::Value(pack) = &mut mismatch.demand else {
        panic!()
    };
    pack.sites[0].wings[0].owner = Some(201);
    let (result, report) = run(&mut frame, &saved, mismatch);
    result.unwrap();
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("selected_owner"))
    );
    saved.air_wings.get_mut(&id).unwrap().country_override =
        b"unrecorded null database tag".to_vec();
    for override_country in [0, -1] {
        let mut fallback = row.clone();
        let Recorded::Value(pack) = &mut fallback.demand else {
            panic!()
        };
        pack.sites[0].wings[0].country_override = Some(override_country);
        pack.sites[0].wings[0].owner = Some(201);
        let (result, report) = run(&mut frame, &saved, fallback.clone());
        result.unwrap();
        assert!(report.differences.is_empty());
        for (owner, reason) in [
            (None, Vec::new()),
            (Some(201), b"synthetic actor metadata failure".to_vec()),
        ] {
            let mut failed = fallback.clone();
            let Recorded::Value(pack) = &mut failed.demand else {
                panic!()
            };
            pack.sites[0].wings[0].owner = owner;
            pack.sites[0].wings[0].owner_reason = reason;
            assert!(
                run(&mut frame, &saved, failed)
                    .0
                    .unwrap_err()
                    .to_string()
                    .contains("selected owner unavailable")
            );
        }
    }
    let mut reserved = row;
    let Recorded::Value(pack) = &mut reserved.demand else {
        panic!()
    };
    pack.sites[0].wings[0].owner = Some(0);
    pack.sites[0].wings[0].country_override = Some(0);
    let (result, report) = run(&mut frame, &saved, reserved);
    result.unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|line| line.contains("raw-zero database tag"))
    );
}

#[test]
fn ordinary_fleet_saved_name_must_be_present_even_when_empty() {
    let (mut frame, mut saved, _, _) = super::super::tests::fixture();
    let id = 7_u64 << 32 | 28;
    saved
        .counter_inputs
        .fleet_orders
        .insert(b"GER".to_vec(), vec![id]);
    saved.fleets.insert(
        id,
        save::Fleet {
            id,
            owner: b"GER".to_vec(),
            home_base: None,
            name: Some(Vec::new()),
            leader: None,
        },
    );
    let country = complete::<tables::Countries>(&frame).unwrap()[0].clone();
    let links = BTreeMap::from([(101, &country)]);
    *frame.tables[TableId::CountryFuel.index()] =
        TableView::Complete(TableRows::CountryFuel(vec![CountryFuelRow {
            country: 101,
            country_instance: Recorded::Value(Some(9)),
            demand: Recorded::Value(FuelDemandRead {
                hour: frame.hour,
                country_instance: Some(9),
                fleets: vec![FuelDemandFleet {
                    reference: id,
                    name: Some(Vec::new()),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        }]));
    let mut report = Report::default();
    compare(&frame, &saved, &links, &mut report).unwrap();
    assert!(report.differences.is_empty());
    saved.fleets.get_mut(&id).unwrap().name = None;
    assert!(
        compare(&frame, &saved, &links, &mut Report::default())
            .unwrap_err()
            .to_string()
            .contains("saved fleet name missing")
    );
}

#[test]
fn fresh_cached_demand_save_witness_detects_signed_difference_and_rejects_held_date_or_missing_value()
 {
    let (mut frame, mut saved, _, _) = super::super::tests::fixture();
    let id = 7_u64 << 32 | 29;
    saved
        .counter_inputs
        .division_orders
        .insert(b"GER".to_vec(), vec![id]);
    saved.divisions.insert(
        id,
        save::Division {
            id,
            roster_owner: b"GER".to_vec(),
            logical_owner: b"GER".to_vec(),
            expeditionary_owner: None,
            province: None,
            template: None,
            alternate_template: None,
            separate_counter: false,
            hit_points: save::Decimal::parse(b"0").unwrap(),
            organisation: save::Decimal::parse(b"0").unwrap(),
            country_intel: Vec::new(),
            presentation: save::DivisionPresentation {
                fuel_requested: Some(save::Decimal::parse(b"-0.00001").unwrap()),
                ..Default::default()
            },
        },
    );
    let country = complete::<tables::Countries>(&frame).unwrap()[0].clone();
    let links = BTreeMap::from([(101, &country), (102, &country)]);
    let row = CountryFuelRow {
        country: 101,
        country_instance: Recorded::Value(Some(9)),
        demand: Recorded::Value(FuelDemandRead {
            hour: frame.hour,
            country_instance: Some(9),
            divisions: vec![FuelDemandUnit {
                reference: 7_u64 << 32 | 29,
                rate_raw: Some(-1),
                owners: Some([102, 0, 101]),
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    };
    let run = |frame: &mut DecodedFrame, row: CountryFuelRow| {
        *frame.tables[TableId::CountryFuel.index()] =
            TableView::Complete(TableRows::CountryFuel(vec![row]));
        let mut report = Report::default();
        let result = compare(frame, &saved, &links, &mut report);
        (result, report)
    };
    let (result, report) = run(&mut frame, row.clone());
    result.unwrap();
    assert!(report.differences.is_empty());
    let mut changed = row.clone();
    let Recorded::Value(pack) = &mut changed.demand else {
        panic!()
    };
    pack.divisions[0].rate_raw = Some(0);
    let (result, report) = run(&mut frame, changed);
    result.unwrap();
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("requested"))
    );
    let mut missing = row.clone();
    let Recorded::Value(pack) = &mut missing.demand else {
        panic!()
    };
    pack.divisions[0].rate_raw = None;
    assert!(run(&mut frame, missing).0.is_err());
    let mut held = row;
    let Recorded::Value(pack) = &mut held.demand else {
        panic!()
    };
    pack.hour -= 1;
    assert!(run(&mut frame, held).0.is_err());
}
