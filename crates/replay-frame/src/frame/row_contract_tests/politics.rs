use super::*;
use crate::frame::{DecodedFrame, FrameDecoder, FrameEncoder, Metrics, TableView};
use crate::rows::sample_politics_table;

const TABLES: [TableId; 5] = [
    TableId::CountryPowerDaily,
    TableId::CountryPowerChanges,
    TableId::CountryPowerDailyCosts,
    TableId::GovernmentChanges,
    TableId::PoliticsCaptureCoverage,
];

#[test]
fn batches_survive_deltas_keyframes_and_selective_reads() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let mut selected = FrameDecoder::for_tables(&TABLES);
    for hour in 0..4 {
        let tables = std::array::from_fn(|index| {
            let table = TableId::ALL[index];
            sample_politics_table(table, hour as usize)
                .map_or_else(
                    || TableView::Unavailable {
                        reason: "unrelated fixture".into(),
                    },
                    TableView::Complete,
                )
                .into()
        });
        let source = DecodedFrame {
            branch: 1,
            hour,
            tables,
            metrics: Metrics::default(),
        };
        let frame = encoder.encode(source.clone(), hour == 2).unwrap();
        let bytes = frame.encoded().unwrap();
        let wire = bytes.decode().unwrap();
        let restored = decoder.apply(&wire).unwrap();
        let subset = selected.apply(&wire).unwrap();
        for table in TABLES {
            assert_eq!(restored.tables[table.index()], source.tables[table.index()]);
            assert_eq!(subset.tables[table.index()], source.tables[table.index()]);
        }
    }
}

#[test]
fn strict_kinds_identity_domains_and_daily_sums_reject_corruption() {
    assert!(PowerCause::from_field(Field::U16(u16::MAX)).is_err());
    assert!(PowerCurrency::from_field(Field::U8(2)).is_err());
    assert!(GovernmentCause::from_field(Field::U8(9)).is_err());
    let mut row = CountryPowerChangeRow::default();
    row.identity.object_reference = Some(7);
    assert!(row.delta(None).is_err());
    row.identity.object_kind = PowerObjectKind::Leader;
    assert!(row.delta(None).is_ok());
    row.identity.origin_token = Some(8);
    assert!(row.delta(None).is_err());
    assert!(
        CountryPowerDailyRow {
            pp_cost_raw: Some(1),
            ..Default::default()
        }
        .delta(None)
        .is_err()
    );
    assert!(
        CountryPowerDailyCostRow {
            day_hour: 1,
            ..Default::default()
        }
        .delta(None)
        .is_err()
    );
    assert!(
        PoliticsCaptureCoverageRow {
            streams: 16,
            ..Default::default()
        }
        .delta(None)
        .is_err()
    );
    assert!(
        PoliticsCaptureCoverageRow {
            since_hour: 2,
            through_hour: 1,
            ..Default::default()
        }
        .delta(None)
        .is_err()
    );
}

#[test]
fn charges_and_reconciliation_keep_signed_extremes_and_absence() {
    assert!(
        CountryPowerDailyRow {
            pp_gross_gain_raw: Some(i64::MIN),
            pp_cost_raw: Some(1),
            pp_net_gain_raw: Some(i64::MAX),
            pp_realized_delta_raw: Some(0),
            pp_sequence: Some(1),
            ..Default::default()
        }
        .delta(None)
        .is_ok()
    );
    let previous = CountryPowerChangeRow {
        country: u32::MAX,
        instance: 3,
        sequence: u64::MAX,
        balance_before_raw: i64::MIN,
        balance_after_raw: RetiredI64(0),
        realized_delta_raw: Recorded::Value(i64::MAX.wrapping_sub(i64::MIN)),
        ..Default::default()
    };
    assert_eq!(previous.balance_after(), i64::MAX);
    assert_eq!(previous.realized_delta_raw(), i128::from(u64::MAX));
    let historical = CountryPowerChangeRow {
        balance_before_raw: 100,
        balance_after_raw: RetiredI64(125),
        realized_delta_raw: Recorded::NotRecorded,
        ..Default::default()
    };
    assert_eq!(historical.balance_after(), 125);
    assert_eq!(historical.realized_delta_raw(), 25);
    let next = CountryPowerChangeRow {
        requested_delta_raw: Some(0),
        operation: PowerOperation::Clamp,
        cause: PowerCause::CapCorrection,
        ..previous.clone()
    };
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.apply(Some(previous)).unwrap(), next);
    let mut bytes = Vec::new();
    encode_row(&next, 0, &mut bytes).unwrap();
    assert_eq!(
        decode_row::<CountryPowerChangeRow>(0, &mut Decoder::new(&bytes)).unwrap(),
        next
    );
    for cut in 0..bytes.len() {
        assert!(decode_row::<CountryPowerChangeRow>(0, &mut Decoder::new(&bytes[..cut])).is_err());
    }
}
