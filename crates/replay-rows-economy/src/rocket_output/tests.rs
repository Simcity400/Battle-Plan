use super::RocketProductionOutputRow;
use replay_rows::{codec::Decoder, rows::RowCodec};

#[test]
fn signed_rocket_totals_round_trip_and_delta_without_ready_count_inference() {
    let row = RocketProductionOutputRow {
        country: u32::MAX,
        variant: 0x1234_5678_0000_0046,
        spent_raw: i64::MIN,
        instant_raw: i64::MAX,
        since_hour: i32::MIN,
        exact: true,
        ticks: 1,
        units: i64::MIN,
        units_exact: true,
        deliveries: 1,
        ..Default::default()
    };
    row.check_key_fields().unwrap();
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        RocketProductionOutputRow::decode_fields(row.key(), &mut input).unwrap(),
        row
    );
    input.finish().unwrap();
    let mut next = row.clone();
    next.spent_raw = i64::MAX;
    next.instant_raw = i64::MIN;
    next.units = -2;
    next.ticks = 2;
    next.deliveries = 2;
    let delta = next.delta(Some(&row)).unwrap().unwrap();
    assert_eq!(
        RocketProductionOutputRow::apply_delta(&delta, Some(row)).unwrap(),
        next
    );
    assert!(next.delta(Some(&next)).unwrap().is_none());
}

#[test]
fn complete_empty_world_and_local_failure_scopes_are_distinct() {
    let empty = RocketProductionOutputRow {
        exact: true,
        units_exact: true,
        since_hour: 100,
        ..Default::default()
    };
    empty.check_key_fields().unwrap();
    assert!(empty.delta(None).unwrap().is_some());
    let world = RocketProductionOutputRow {
        ticks: 1,
        inexact_ticks: 1,
        unknown_design_ticks: 1,
        ..empty
    };
    world.check_key_fields().unwrap();
    let local = RocketProductionOutputRow {
        country: 5001,
        exact: false,
        ..world
    };
    local.check_key_fields().unwrap();
    assert_ne!(local.key(), empty.key());
}

#[test]
fn coverage_amounts_and_impossible_counts_refuse_by_name() {
    let row = RocketProductionOutputRow {
        country: 5001,
        ticks: 1,
        ..Default::default()
    };
    for bad in [
        RocketProductionOutputRow {
            spent_raw: 1,
            ..row.clone()
        },
        RocketProductionOutputRow {
            instant_raw: -1,
            ..row.clone()
        },
        RocketProductionOutputRow {
            units: 1,
            ..row.clone()
        },
        RocketProductionOutputRow {
            inexact_ticks: 2,
            ..row.clone()
        },
        RocketProductionOutputRow {
            unknown_design_ticks: 1,
            ..row.clone()
        },
        RocketProductionOutputRow {
            unknown_country_ticks: 1,
            inexact_ticks: 1,
            ..row.clone()
        },
        RocketProductionOutputRow {
            inexact_deliveries: 1,
            ..row.clone()
        },
        RocketProductionOutputRow {
            variant: 0x99_0000_0046,
            unknown_design_ticks: 1,
            inexact_ticks: 1,
            ..row.clone()
        },
        RocketProductionOutputRow {
            variant: 0x99_0000_0046,
            ticks: 0,
            ..row.clone()
        },
        RocketProductionOutputRow {
            country: 0,
            variant: 0x99_0000_0046,
            ..row
        },
    ] {
        assert_eq!(
            bad.check_key_fields().unwrap_err().to_string(),
            "rocket output scope/counts inconsistent"
        );
    }
}

#[test]
fn full_country_design_key_and_truncated_payload_are_preserved() {
    let first = RocketProductionOutputRow {
        country: 5001,
        variant: 0x11_0000_0046,
        ticks: 1,
        ..Default::default()
    };
    let second = RocketProductionOutputRow {
        variant: 0x12_0000_0046,
        ..first
    };
    let other = RocketProductionOutputRow {
        country: 5002,
        ..first
    };
    assert_ne!(first.key(), second.key());
    assert_ne!(first.key(), other.key());
    let mut bytes = Vec::new();
    first.encode_fields(&mut bytes).unwrap();
    bytes.pop();
    assert!(
        RocketProductionOutputRow::decode_fields(first.key(), &mut Decoder::new(&bytes)).is_err()
    );
}

#[test]
fn unknown_owner_design_deliveries_cannot_claim_exact_or_unaccounted_units() {
    let unknown = RocketProductionOutputRow {
        variant: 0x99_0000_0046,
        units: 1,
        deliveries: 1,
        inexact_deliveries: 1,
        ..Default::default()
    };
    unknown.check_key_fields().unwrap();
    for bad in [
        RocketProductionOutputRow {
            units_exact: true,
            ..unknown.clone()
        },
        RocketProductionOutputRow {
            inexact_deliveries: 0,
            ..unknown
        },
    ] {
        assert_eq!(
            bad.check_key_fields().unwrap_err().to_string(),
            "rocket output scope/counts inconsistent"
        );
        assert!(bad.delta(None).is_err());
        let mut bytes = Vec::new();
        bad.encode_fields(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        let decoded = RocketProductionOutputRow::decode_fields(bad.key(), &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(decoded, bad);
        assert_eq!(
            decoded.check_key_fields().unwrap_err().to_string(),
            "rocket output scope/counts inconsistent"
        );
    }
    RocketProductionOutputRow {
        deliveries: 1,
        inexact_deliveries: 1,
        units_exact: true,
        ..Default::default()
    }
    .check_key_fields()
    .unwrap();
    RocketProductionOutputRow {
        variant: unknown.variant,
        ticks: 1,
        inexact_ticks: 1,
        unknown_country_ticks: 1,
        units_exact: true,
        ..Default::default()
    }
    .check_key_fields()
    .unwrap();
}
