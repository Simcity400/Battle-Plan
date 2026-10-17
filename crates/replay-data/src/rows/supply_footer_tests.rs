//! The unit supply footers at display precision ([`crate::display`]):
//! DivisionStatus.supply ([`DivisionSupplyFooter`]) and the task force and
//! railway gun counter inputs' supply ([`UnitSupplyInputs`]).
use super::*;
use crate::rows::compatibility::{
    DISPLAY_PRECISION_FORMAT, EFFECT_INPUTS_FORMAT, RETIRED_INPUTS_FORMAT, SUPPLY_FOOTER_FORMAT,
    SUPPLY_SHORTFALL_FORMAT, UNIT_SUPPLY_FOOTER_FORMAT,
};
use crate::value::Value;

/// A footer as the game holds it: exact fixed5 amounts.
fn native() -> DivisionSupplyFooter {
    DivisionSupplyFooter {
        asked: 123_456,
        received: 1_499,
        from_state: 1_500,
        air_drops: 3,
        disruption: -4,
        division_demand: UnitSupplyDemand {
            count: 7,
            value: 2_500,
        },
        airplane_demand: UnitSupplyDemand {
            count: -1,
            value: 0,
        },
        ship_demand: UnitSupplyDemand {
            count: i32::MIN,
            value: i64::MAX,
        },
        origins: Some(vec![
            UnitSupplyOrigin {
                province: 5,
                requested: 10_499,
                received: 4_999,
            },
            UnitSupplyOrigin {
                province: 5,
                requested: -1,
                received: 0,
            },
        ]),
    }
}

/// [`native`] as the footer prints it: hundredths half away from zero (two-decimal
/// formatting), a nonzero amount below half a hundredth keeps one with its sign, counts,
/// provinces and sentinel words exact.
fn displayed() -> DivisionSupplyFooter {
    DivisionSupplyFooter {
        asked: 123_000,
        received: 1_000,
        from_state: 2_000,
        air_drops: 1_000,
        disruption: -1_000,
        division_demand: UnitSupplyDemand {
            count: 7,
            value: 3_000,
        },
        airplane_demand: UnitSupplyDemand {
            count: -1,
            value: 0,
        },
        ship_demand: UnitSupplyDemand {
            count: i32::MIN,
            value: i64::MAX,
        },
        origins: Some(vec![
            UnitSupplyOrigin {
                province: 5,
                requested: 10_000,
                received: 5_000,
            },
            UnitSupplyOrigin {
                province: 5,
                requested: -1_000,
                received: 0,
            },
        ]),
    }
}

fn status(supply: Option<DivisionSupplyFooter>) -> DivisionStatusRow {
    DivisionStatusRow {
        division: 0x1234_5678_0000_0035,
        supply: Recorded::Value(supply),
        ..Default::default()
    }
}

/// The payload a row stores in field 22.
fn stored_payload(row: &DivisionStatusRow) -> Vec<u8> {
    let mut bytes = Vec::new();
    encode_row(row, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    input.varint().unwrap();
    for (index, kind) in DivisionStatusRow::SCHEMA.iter().enumerate() {
        let field = kind.decode(&mut input).unwrap();
        if index == 22 {
            let Field::Opaque(Opaque::Encoded(payload, _)) = field else {
                panic!("DivisionStatus.supply is not Opaque");
            };
            return payload.to_vec();
        }
    }
    unreachable!("DivisionStatus has field 22")
}

#[test]
fn division_supply_footer_rounds_to_the_printed_hundredth() {
    let mut row = status(Some(native()));
    assert!(row.has_display_steps());
    row.round_to_display();
    assert_eq!(row, status(Some(displayed())));
    row.round_to_display();
    assert_eq!(row, status(Some(displayed())), "rounding is idempotent");
    assert_eq!(
        DivisionSupplyFooter::STEP.describe(),
        "0.01 (fixed5, half away from zero, nonzero kept)"
    );
}

#[test]
fn division_supply_footer_stores_hundredths_and_round_trips() {
    // The encoder rounds again before dividing, so the exact footer and the
    // rounded one write the same bytes.
    let exact = stored_payload(&status(Some(native())));
    assert_eq!(exact, stored_payload(&status(Some(displayed()))));
    // Some, then each amount divided by the step, the received amount as the
    // steps it fell short of the asked amount by; counts stay native.
    let mut expected = vec![1];
    for units in [123_i64, 122, 2, 1, -1] {
        expected.extend(units.to_le_bytes());
    }
    expected.extend(7_i32.to_le_bytes());
    expected.extend(3_i64.to_le_bytes());
    assert_eq!(&exact[..expected.len()], expected.as_slice());

    for supply in [Some(displayed()), None] {
        let row = status(supply);
        let mut bytes = Vec::new();
        encode_row(&row, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(decode_row::<DivisionStatusRow>(0, &mut input).unwrap(), row);
        input.finish().unwrap();
    }
}

#[test]
fn an_unchanged_division_supply_footer_writes_no_change() {
    let previous = status(Some(displayed()));
    // Drift that stays inside each printed hundredth rounds back onto the
    // baseline, so the row writes nothing.
    let mut drifted = status(Some(DivisionSupplyFooter {
        asked: 123_499,
        received: 501,
        origins: Some(vec![
            UnitSupplyOrigin {
                province: 5,
                requested: 9_500,
                received: 5_499,
            },
            UnitSupplyOrigin {
                province: 5,
                requested: -499,
                received: 0,
            },
        ]),
        ..native()
    }));
    drifted.round_to_display();
    assert_eq!(drifted, previous);
    assert!(
        !drifted
            .encode_change(Some(&previous), 0, &mut Vec::new())
            .unwrap()
    );
    // A printed change marks only the supply field, in whole hundredths.
    let mut moved = previous.clone();
    if let Recorded::Value(Some(footer)) = &mut moved.supply {
        footer.received = 2_000;
    }
    let delta = moved.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 22);
    assert_eq!(delta.apply(Some(previous)).unwrap(), moved);
}

/// The payload formats 99 to 101 wrote for [`native`]: its words exact, in
/// the [`UnitSupplyInputs`] layout. A current task force footer whose amounts
/// are a thousand times the words stores exactly those words.
fn older_payload() -> Vec<u8> {
    let words = DivisionSupplyFooter {
        asked: 123_456_000,
        received: 1_499_000,
        from_state: 1_500_000,
        air_drops: 3_000,
        disruption: -4_000,
        division_demand: UnitSupplyDemand {
            count: 7,
            value: 2_500_000,
        },
        airplane_demand: UnitSupplyDemand {
            count: -1,
            value: 0,
        },
        ship_demand: UnitSupplyDemand {
            count: i32::MIN,
            value: i64::MAX,
        },
        origins: Some(vec![
            UnitSupplyOrigin {
                province: 5,
                requested: 10_499_000,
                received: 4_999_000,
            },
            UnitSupplyOrigin {
                province: 5,
                requested: -1_000,
                received: 0,
            },
        ]),
    };
    let mut payload = Vec::new();
    Value::encode_reserved(&Some(UnitSupplyInputs::from(words)), &mut payload).unwrap();
    payload
}

#[test]
fn older_recordings_decode_their_exact_footer_unscaled() {
    let payload = older_payload();
    let decode = |format| {
        <Recorded<Option<DivisionSupplyFooter>> as NativeValue>::from_field(Field::Opaque(
            Opaque::Encoded(&payload, format),
        ))
    };
    for format in [EFFECT_INPUTS_FORMAT, DISPLAY_PRECISION_FORMAT - 1] {
        assert_eq!(decode(format).unwrap(), Recorded::Value(Some(native())));
    }
    // The current format reads the same words as hundredths.
    let Recorded::Value(Some(current)) = decode(SUPPLY_FOOTER_FORMAT).unwrap() else {
        panic!("current footer did not decode");
    };
    assert_eq!(current.asked, 123_456_000);
    assert_eq!(current.ship_demand.value, i64::MAX);
}

fn encoded(footer: &DivisionSupplyFooter) -> Vec<u8> {
    let mut bytes = Vec::new();
    Value::encode(footer, &mut bytes).unwrap();
    bytes
}

#[test]
fn a_supplied_division_stores_no_shortfall_while_its_demand_drifts() {
    let supplied = |amount: i64| DivisionSupplyFooter {
        asked: amount,
        received: amount,
        origins: Some(vec![UnitSupplyOrigin {
            province: 5,
            requested: amount,
            received: amount,
        }]),
        ..displayed()
    };
    let before = stored_payload(&status(Some(supplied(123_000))));
    let after = stored_payload(&status(Some(supplied(125_000))));
    // Some, the asked word, then the received word: no step short. The
    // origins follow the demands (Some, a count, the province), each its
    // requested word then its shortfall.
    let (asked, received) = (1..9, 9..17);
    let (requested, origin_received) = (86..94, 94..102);
    assert_eq!(before.len(), origin_received.end);
    for payload in [&before, &after] {
        // The origins' tag (after the demands) marks shortfalls.
        assert_eq!(payload[77], 3);
        assert_eq!(payload[received.clone()], 0_i64.to_le_bytes());
        assert_eq!(payload[origin_received.clone()], 0_i64.to_le_bytes());
    }
    // Only the asked and requested words move with the demand.
    let moved: Vec<usize> = (0..before.len())
        .filter(|&at| before[at] != after[at])
        .collect();
    assert!(!moved.is_empty());
    assert!(
        moved
            .iter()
            .all(|at| asked.contains(at) || requested.contains(at)),
        "{moved:?}"
    );
    // The historical writer stands in for format 101, which stored the
    // received amount itself.
    let older = crate::rows::with_historical_stamp(0, || encoded(&supplied(123_000)));
    assert_eq!(older[8..16], 123_i64.to_le_bytes());
    assert_eq!(SUPPLY_SHORTFALL_FORMAT, 102);
}

#[test]
fn earlier_provisional_captures_decode_their_received_amounts_exactly() {
    for footer in [
        displayed(),
        DivisionSupplyFooter {
            origins: None,
            ..displayed()
        },
    ] {
        // Format-102 captures written before the shortfalls stored the
        // UnitSupplyInputs layout in steps: the received steps themselves,
        // the origins tagged 0 or 1.
        let mut earlier = Vec::new();
        Value::encode(&UnitSupplyInputs::from(footer.clone()), &mut earlier).unwrap();
        let current = encoded(&footer);
        assert_eq!(earlier.len(), current.len());
        assert_ne!(earlier, current);
        assert_eq!(earlier[76] | 2, current[76]);
        let mut input = Decoder::new(&earlier);
        assert_eq!(
            <DivisionSupplyFooter as Value>::decode(&mut input).unwrap(),
            footer
        );
        input.finish().unwrap();
        // A reader older than the shortfalls refuses the current layout's
        // origins tag.
        let mut input = Decoder::new(&current);
        let error = <UnitSupplyInputs as Value>::decode(&mut input).unwrap_err();
        // The exact error, which fails that reader's whole block: it never
        // misreads the footer.
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "invalid nested option tag");
    }
}

#[test]
fn every_amount_pair_round_trips_through_its_shortfall() {
    let step = DivisionSupplyFooter::STEP;
    // Sentinel words, the widest multiples of the step and the smallest
    // amounts, as the footer stores them.
    let amounts = [
        i64::MIN,
        i64::MIN + 1,
        i64::MIN + 2,
        -1_000,
        -1,
        0,
        1,
        500,
        2_000,
        i64::MAX - 1,
        i64::MAX,
    ]
    .map(|amount| step.round(amount));
    for asked in amounts {
        for received in amounts {
            let footer = DivisionSupplyFooter {
                asked,
                received,
                origins: Some(vec![
                    UnitSupplyOrigin {
                        province: -1,
                        requested: asked,
                        received,
                    },
                    UnitSupplyOrigin {
                        province: 7,
                        requested: received,
                        received: asked,
                    },
                ]),
                ..displayed()
            };
            let bytes = encoded(&footer);
            assert_eq!(bytes.len(), Value::encoded_bytes(&footer).unwrap());
            assert_eq!(
                bytes[8..16],
                step.units(asked)
                    .wrapping_sub(step.units(received))
                    .to_le_bytes(),
                "{asked} {received}"
            );
            let mut input = Decoder::new(&bytes);
            assert_eq!(
                <DivisionSupplyFooter as Value>::decode(&mut input).unwrap(),
                footer,
                "{asked} {received}"
            );
            input.finish().unwrap();
        }
    }
}

#[test]
fn task_force_and_railway_gun_footers_store_the_printed_hundredth() {
    // The same footer formatter serves every unit counter's supply child.
    let task_force = |supply: DivisionSupplyFooter| TaskForceCounterInputRow {
        task_force: 9,
        supply: Recorded::Value(Some(supply.into())),
        ..Default::default()
    };
    let gun = |supply: DivisionSupplyFooter| RailwayGunCounterInputRow {
        gun: 9,
        supply: Recorded::Value(Some(supply.into())),
        ..Default::default()
    };
    let mut row = task_force(native());
    assert!(row.has_display_steps());
    row.round_to_display();
    assert_eq!(row, task_force(displayed()));
    let mut row = gun(native());
    row.round_to_display();
    assert_eq!(row, gun(displayed()));
    assert_eq!(UnitSupplyInputs::STEP, DivisionSupplyFooter::STEP);

    // Stored as whole hundredths, and a drift inside them writes no change.
    let previous = task_force(displayed());
    let mut bytes = Vec::new();
    encode_row(&previous, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        decode_row::<TaskForceCounterInputRow>(0, &mut input).unwrap(),
        previous
    );
    input.finish().unwrap();
    let mut drifted = task_force(DivisionSupplyFooter {
        asked: 123_499,
        received: 501,
        ..native()
    });
    drifted.round_to_display();
    assert_eq!(drifted, previous);
    assert!(
        !drifted
            .encode_change(Some(&previous), 0, &mut Vec::new())
            .unwrap()
    );

    // Format 101 recorded the footer exact and decodes it unscaled.
    let payload = older_payload();
    let decode = |format| {
        <Recorded<Option<UnitSupplyInputs>> as NativeValue>::from_field(Field::Opaque(
            Opaque::Encoded(&payload, format),
        ))
    };
    assert_eq!(
        decode(DISPLAY_PRECISION_FORMAT - 1).unwrap(),
        Recorded::Value(Some(native().into()))
    );
    let Recorded::Value(Some(current)) = decode(UNIT_SUPPLY_FOOTER_FORMAT).unwrap() else {
        panic!("current footer did not decode");
    };
    assert_eq!(current.asked, 123_456_000);
}

#[test]
fn a_provisional_tombstone_reads_as_not_recorded() {
    let decode = |format| {
        <Recorded<Option<DivisionSupplyFooter>> as NativeValue>::from_field(Field::Opaque(
            Opaque::Encoded(&[], format),
        ))
    };
    // Format-102 writers stored the retired field's empty payload, which no
    // recorded footer encodes.
    assert_eq!(
        decode(RETIRED_INPUTS_FORMAT).unwrap(),
        Recorded::NotRecorded
    );
    // Before the retirement an empty payload is malformed.
    assert!(decode(DISPLAY_PRECISION_FORMAT - 1).is_err());

    // A whole row from such a recording keeps every other field.
    let row = DivisionStatusRow {
        supply_ratio: 50_000,
        dig_in_factor: Recorded::Value(Some(7)),
        ..status(Some(displayed()))
    };
    let mut bytes = Vec::new();
    encode_row(&row, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    input.varint().unwrap();
    for kind in &DivisionStatusRow::SCHEMA[..22] {
        kind.decode(&mut input).unwrap();
    }
    let start = bytes.len() - input.remaining();
    DivisionStatusRow::SCHEMA[22].decode(&mut input).unwrap();
    let end = bytes.len() - input.remaining();
    let mut tombstone = bytes[..start].to_vec();
    tombstone.extend(0_u32.to_le_bytes());
    tombstone.extend(&bytes[end..]);
    let mut input = Decoder::new(&tombstone);
    let decoded = decode_row::<DivisionStatusRow>(0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(
        decoded,
        DivisionStatusRow {
            supply: Recorded::NotRecorded,
            ..row.clone()
        }
    );
    // Such a recording is written again byte for byte (sizes, finalization),
    // and a later current row changes only the footer.
    let mut again = Vec::new();
    encode_row(&decoded, 0, &mut again).unwrap();
    assert_eq!(again, tombstone);
    let delta = row.delta(Some(&decoded)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 22);
    assert_eq!(delta.apply(Some(decoded.clone())).unwrap(), row);
    // An absence that is not a restored tombstone still refuses to encode.
    let historical = DivisionStatusRow {
        name: Recorded::NotRecorded,
        ..decoded.clone()
    };
    assert!(historical.delta(None).is_err());
    assert_eq!(
        TableRows::DivisionStatus(vec![decoded])
            .field_gaps()
            .collect::<Vec<_>>(),
        [("supply", 1)]
    );
}

#[test]
fn the_division_supply_footer_is_recorded_again() {
    assert!(
        !compatibility::RETIRED_FIELDS
            .iter()
            .any(|field| field.table == TableId::DivisionStatus && field.index == 22),
        "DivisionStatus.supply is not a tombstone"
    );
    const { assert!(<Option<DivisionSupplyFooter> as NativeValue>::RESTORED) };
    assert_eq!(SUPPLY_FOOTER_FORMAT, 102);
}
