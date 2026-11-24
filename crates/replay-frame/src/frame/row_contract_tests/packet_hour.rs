use super::*;
use crate::{
    cadence::{self, Cadence},
    format::FormatVersion,
    frame::{DecodedFrame, FrameDecoder, FrameEncoder, FrameInput, Metrics, TableInput, TableView},
    rows::compatibility::{ChangeKind, EXPERIENCE_FORMAT, PACKET_HOUR_FORMAT, SCRIPTED_MAP_FORMAT},
    value::Value,
};

#[test]
fn host_hours_cost_no_bytes_and_historical_stamps_decode_only_from_their_format() {
    let mut bytes = Vec::new();
    Value::encode(&PacketHour::Host, &mut bytes).unwrap();
    assert!(bytes.is_empty());
    assert_eq!(Value::recorded_bytes(&PacketHour::Host).unwrap(), 0);
    let mut input = Decoder::new(&[]);
    assert_eq!(PacketHour::decode(&mut input).unwrap(), PacketHour::Host);
    input.finish().unwrap();

    for stamp in [i32::MIN, -1, 0, i32::MAX] {
        let wire = stamp.to_le_bytes();
        let mut input = Decoder::new(&wire);
        input.format = STAMPED_PACKET_FORMAT;
        let historical = PacketHour::decode(&mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(historical, PacketHour::Stamped(stamp));
        assert_eq!(historical.stamp(), Some(stamp));
        assert_eq!(Value::recorded_bytes(&historical).unwrap(), wire.len());
        assert!(historical.fresh_at(stamp));
        assert!(!historical.fresh_at(stamp.wrapping_add(1)));
        assert!(Value::encode(&historical, &mut Vec::new()).is_err());
        assert!(Value::encoded_bytes(&historical).is_err());
        let mut input = Decoder::new(&wire);
        assert_eq!(PacketHour::decode(&mut input).unwrap(), PacketHour::Host);
        assert_eq!(input.remaining(), wire.len());
    }
    assert!(PacketHour::Host.fresh_at(i32::MIN) && PacketHour::Host.fresh_at(i32::MAX));
    assert_eq!(PacketHour::Host.stamp(), None);
    let mut input = Decoder::new(&[0, 0, 0]);
    input.format = STAMPED_PACKET_FORMAT;
    assert!(PacketHour::decode(&mut input).is_err());
}

fn host_rows(table: TableId, tick: usize) -> TableRows {
    sample_extended_table(table, tick)
        .filter(|rows| !rows.is_empty())
        .unwrap_or_else(|| panic!("{table:?} host fixture"))
}

fn frame(hour: i32, table: TableId, view: TableView) -> FrameInput {
    let mut input = FrameInput {
        branch: 1,
        hour,
        tables: std::array::from_fn(|_| {
            TableInput::Sampled(TableView::Unavailable {
                reason: "outside packet-hour fixture".into(),
            })
        }),
        metrics: Metrics::default(),
    };
    input.metrics.table_ns[table.index()] = 1;
    input.tables[table.index()] = TableInput::Sampled(view);
    input
}

fn packets_fresh(frame: &DecodedFrame, table: TableId) -> bool {
    let TableView::Complete(rows) = &*frame.tables[table.index()] else {
        return false;
    };
    packet_hours(rows)
        .into_iter()
        .all(|hour| hour.fresh_at(frame.hour))
}

#[test]
fn equal_consecutive_acquisitions_of_every_host_encode_unchanged_and_stay_fresh() {
    for table in PACKET_HOUR_HOSTS {
        let rows = host_rows(table, 5);
        assert!(
            !packet_hours(&rows).is_empty(),
            "{table:?} fixture holds no packet"
        );
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        for hour in 1000..1004 {
            let wire = encoder
                .encode_input(frame(hour, table, TableView::Complete(rows.clone())), false)
                .unwrap();
            match (&wire.tables[table.index()], hour) {
                (crate::frame::TableUpdate::Snapshot(_), 1000) => {}
                (crate::frame::TableUpdate::Unchanged { baseline }, _) => {
                    assert_eq!(*baseline, hour - 1, "{table:?}");
                }
                (update, _) => panic!("{table:?} at {hour}: {update:?}"),
            }
            let decoded = decoder.apply(&wire).unwrap();
            assert_eq!(
                *decoded.tables[table.index()],
                TableView::Complete(rows.clone())
            );
            assert!(packets_fresh(&decoded, table), "{table:?} at {hour}");
        }
    }
}

#[test]
fn keyframes_seeks_and_refusals_prove_freshness_from_the_frame_alone() {
    for table in PACKET_HOUR_HOSTS {
        let rows = host_rows(table, 7);
        let mut encoder = FrameEncoder::new();
        let mut frames = Vec::new();
        for (hour, keyframe) in [(2000, false), (2001, false), (2002, true), (2003, false)] {
            frames.push(
                encoder
                    .encode_input(
                        frame(hour, table, TableView::Complete(rows.clone())),
                        keyframe,
                    )
                    .unwrap(),
            );
        }
        assert!(frames[2].keyframe);
        let mut seek = FrameDecoder::new();
        for wire in &frames[2..] {
            let decoded = seek.apply(wire).unwrap();
            assert!(packets_fresh(&decoded, table), "{table:?} at {}", wire.hour);
        }

        let mut decoder = FrameDecoder::new();
        decoder.apply(&frames[0]).unwrap();
        for reason in [cadence::SKIPPED_REASON, "guarded host read failed"] {
            let wire = encoder
                .encode_input(
                    frame(
                        frames.last().unwrap().hour + 1,
                        table,
                        TableView::Unavailable {
                            reason: reason.into(),
                        },
                    ),
                    false,
                )
                .unwrap();
            assert!(matches!(
                wire.tables[table.index()],
                crate::frame::TableUpdate::Unavailable { .. }
            ));
            let decoded = FrameDecoder::new().apply(&wire).unwrap();
            assert!(!packets_fresh(&decoded, table), "{table:?}");
            frames.push(wire);
        }
        let wire = encoder
            .encode_input(
                frame(
                    frames.last().unwrap().hour + 1,
                    table,
                    TableView::Complete(rows.clone()),
                ),
                false,
            )
            .unwrap();
        assert!(matches!(
            wire.tables[table.index()],
            crate::frame::TableUpdate::Snapshot(_)
        ));
    }
}
