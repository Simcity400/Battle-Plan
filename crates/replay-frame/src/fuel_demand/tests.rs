use super::*;
use crate::codec::Decoder;
use crate::format::FormatVersion;
use crate::frame::TableView;
use crate::rows::compatibility::FUEL_DEMAND_PARTS_FORMAT;
use crate::rows::*;
use crate::value::Value;

fn unit(reference: u64, rate: i64) -> FuelDemandUnit {
    FuelDemandUnit {
        reference,
        ordinal: reference as u32,
        rate_raw: Some(rate),
        ..Default::default()
    }
}

fn pack() -> FuelDemandRead {
    FuelDemandRead {
        hour: FuelDemandHour::Host,
        country_instance: Some(9),
        divisions: vec![
            unit(7 << 32 | 1, 100),
            unit(7 << 32 | 2, -5),
            unit(7 << 32 | 1, i64::MAX),
        ],
        groups: vec![FuelDemandGroup {
            reference: 11,
            members: vec![7 << 32 | 1],
            ..Default::default()
        }],
        fleets: vec![FuelDemandFleet {
            reference: 12,
            name: Some(b"fleet".to_vec()),
            ..Default::default()
        }],
        task_forces: vec![unit(13, 1)],
        sites: vec![
            FuelDemandSite {
                list: 0,
                ordinal: 0,
                base: 14,
                wings: vec![FuelDemandWing {
                    reference: 15,
                    context: Some(1),
                    ..Default::default()
                }],
                ..Default::default()
            },
            FuelDemandSite {
                list: 1,
                ordinal: 0,
                base: 16,
                ..Default::default()
            },
        ],
        contexts: vec![
            FuelDemandContext {
                region: 9,
                sums: [1; 24],
            },
            FuelDemandContext {
                region: 3,
                sums: [-2; 24],
            },
        ],
        army_reason: b"army".to_vec(),
        ..Default::default()
    }
}

fn row(demand: FuelDemandRead) -> CountryFuelRow {
    CountryFuelRow {
        country: 4,
        country_instance: Recorded::Value(Some(9)),
        demand: Recorded::Value(demand),
        ..Default::default()
    }
}

fn split(demand: FuelDemandRead) -> (CountryFuelRow, Vec<FuelDemandPartRow>) {
    let (header, parts) = demand.split(4).unwrap();
    (row(header), parts)
}

fn encoded<T: Value>(value: &T) -> Vec<u8> {
    let mut bytes = Vec::new();
    Value::encode(value, &mut bytes).unwrap();
    bytes
}

#[test]
fn a_pack_hour_read_with_its_row_costs_one_unchanging_byte() {
    assert_eq!(encoded(&FuelDemandHour::Host), [0]);
    assert_eq!(
        encoded(&FuelDemandHour::Stamped(-2)),
        [1, 0xFE, 0xFF, 0xFF, 0xFF]
    );
    for hour in [FuelDemandHour::Host, FuelDemandHour::Stamped(i32::MIN)] {
        let bytes = encoded(&hour);
        let mut input = Decoder::new(&bytes);
        assert_eq!(<FuelDemandHour as Value>::decode(&mut input).unwrap(), hour);
        input.finish().unwrap();
    }
    assert_eq!(FuelDemandHour::Host.at(17), 17);
    for (hour, bytes) in [
        (FuelDemandHour::Host, 1),
        (FuelDemandHour::Stamped(-2), 5),
        (FuelDemandHour::Historical(-2), 4),
    ] {
        assert_eq!(Value::recorded_bytes(&hour).unwrap(), bytes);
    }
    assert_eq!(FuelDemandHour::Stamped(-7).at(17), -7);
    let bytes = crate::rows::with_historical_stamp(41, || encoded(&FuelDemandHour::Host));
    assert_eq!(bytes, 41_i32.to_le_bytes());
    let mut input = Decoder::new(&bytes);
    input.format = FUEL_DEMAND_PARTS_FORMAT - 1;
    assert_eq!(
        <FuelDemandHour as Value>::decode(&mut input).unwrap(),
        FuelDemandHour::Historical(41)
    );
    input.finish().unwrap();
    let (first, _) = split(pack());
    let (next, _) = split(pack());
    assert!(next.delta(Some(&first)).unwrap().is_none());
}

#[test]
fn a_pack_splits_into_a_row_a_part_and_assembles_exactly() {
    let whole = row(pack());
    let (header, parts) = split(pack());
    let Recorded::Value(demand) = &header.demand else {
        panic!()
    };
    assert_eq!(
        demand,
        &FuelDemandRead {
            hour: FuelDemandHour::Host,
            country_instance: Some(9),
            army_reason: b"army".to_vec(),
            ..Default::default()
        }
    );
    assert_eq!(parts.len(), 3 + 1 + 1 + 1 + 2 + 2);
    assert!(parts.windows(2).all(|pair| pair[0].key() < pair[1].key()));
    let repeated: Vec<_> = parts
        .iter()
        .filter(|part| part.kind == 0 && part.ident == 7 << 32 | 1)
        .map(|part| (part.occurrence, part.position))
        .collect();
    assert_eq!(repeated, [(0, 0), (1, 2)]);
    for part in &parts {
        let mut bytes = Vec::new();
        crate::rows::encode_row(part, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(
            &crate::rows::decode_row::<FuelDemandPartRow>(0, &mut input).unwrap(),
            part
        );
        input.finish().unwrap();
    }
    let view = TableView::Complete(TableRows::FuelDemandParts(parts));
    let assembled =
        with_demand_parts(std::slice::from_ref(&header), &view, FormatVersion::CURRENT).unwrap();
    assert_eq!(assembled, [whole]);
}

#[test]
fn one_division_whose_rate_moves_stores_only_its_part() {
    let (before_header, before) = split(pack());
    let mut moved = pack();
    moved.divisions[1].rate_raw = Some(-6);
    let (after_header, after) = split(moved);
    assert!(after_header.delta(Some(&before_header)).unwrap().is_none());
    let changed: Vec<_> = before
        .iter()
        .zip(&after)
        .filter(|(before, after)| before != after)
        .map(|(_, after)| (after.kind, after.ident))
        .collect();
    assert_eq!(changed, [(0, 7 << 32 | 2)]);
    let at = after
        .iter()
        .position(|part| part.kind == 0 && part.ident == 7 << 32 | 2)
        .unwrap();
    let delta = after[at].delta(Some(&before[at])).unwrap();
    assert!(delta.is_some_and(|delta| delta.mask == 1 << 1));
    let mut joined = pack();
    joined.divisions.insert(0, unit(99, 3));
    let (_, joined) = split(joined);
    for part in before.iter().filter(|part| part.kind == 0) {
        let same = joined.iter().find(|row| row.key() == part.key()).unwrap();
        assert_eq!((&same.part, same.position), (&part.part, part.position + 1));
    }
}

#[test]
fn missing_or_inconsistent_parts_refuse_only_their_pack() {
    let (header, mut parts) = split(pack());
    let other = CountryFuelRow {
        country: 5,
        demand: Recorded::Value(FuelDemandRead::default()),
        ..Default::default()
    };
    parts.retain(|part| !(part.kind == 0 && part.position == 1));
    let view = TableView::Complete(TableRows::FuelDemandParts(parts));
    let assembled = with_demand_parts(
        &[header.clone(), other.clone()],
        &view,
        FormatVersion::CURRENT,
    )
    .unwrap();
    let Recorded::Value(refused) = &assembled[0].demand else {
        panic!()
    };
    assert!(refused.divisions.is_empty() && !refused.reason.is_empty());
    assert_eq!(
        refused.native_rates()[0],
        Err("fuel current demand read identity unavailable")
    );
    assert_eq!(assembled[1], other);
    for view in [
        TableView::Truncated {
            rows: TableRows::FuelDemandParts(Vec::new()),
            missing: 1,
        },
        TableView::Unavailable {
            reason: "fuel walk failed".into(),
        },
    ] {
        let assembled =
            with_demand_parts(std::slice::from_ref(&header), &view, FormatVersion::CURRENT)
                .unwrap();
        let Recorded::Value(refused) = &assembled[0].demand else {
            panic!()
        };
        assert!(!refused.reason.is_empty());
    }
}

#[test]
fn older_recordings_keep_their_inline_lists_and_refuse_only_the_parts() {
    let previous = FormatVersion::read(FUEL_DEMAND_PARTS_FORMAT - 1).unwrap();
    let reason = previous.table_refusal(TableId::FuelDemandParts).unwrap();
    assert!(reason.contains("FuelDemandParts"), "{reason}");
    assert!(previous.table_refusal(TableId::CountryFuel).is_none());
    assert!(
        FormatVersion::CURRENT
            .table_refusal(TableId::FuelDemandParts)
            .is_none()
    );
    let mut old = pack();
    old.hour = FuelDemandHour::Historical(1000);
    let inline = row(old);
    let view = TableView::Unavailable {
        reason: reason,
    };
    assert_eq!(
        with_demand_parts(std::slice::from_ref(&inline), &view, previous).unwrap(),
        [inline]
    );
    let bytes = crate::rows::with_historical_stamp(1000, || encoded(&pack()));
    let mut input = Decoder::new(&bytes);
    input.format = previous.version();
    let decoded = <FuelDemandRead as Value>::decode(&mut input).unwrap();
    input.finish().unwrap();
    let mut expected = pack();
    expected.hour = FuelDemandHour::Historical(1000);
    assert_eq!(decoded, expected);
    assert_eq!(Value::recorded_bytes(&decoded).unwrap(), bytes.len());
}

#[test]
fn a_list_splits_up_to_the_last_occurrence_its_key_holds() {
    let repeated = |count: usize| FuelDemandRead {
        divisions: vec![unit(7, 1); count],
        ..Default::default()
    };
    let full = repeated(usize::from(u16::MAX) + 1);
    assert!(full.splittable());
    let (_, parts) = full.split(4).unwrap();
    assert_eq!(parts.len(), usize::from(u16::MAX) + 1);
    assert_eq!(
        parts.last().map(|part| (part.occurrence, part.position)),
        Some((u16::MAX, u32::from(u16::MAX)))
    );
    let over = repeated(usize::from(u16::MAX) + 2);
    assert!(!over.splittable());
    assert!(over.split(4).is_err());
    let whole = row(repeated(usize::from(u16::MAX) + 2));
    let view = TableView::Complete(TableRows::FuelDemandParts(Vec::new()));
    assert_eq!(
        with_demand_parts(std::slice::from_ref(&whole), &view, FormatVersion::CURRENT).unwrap(),
        [whole]
    );
}
