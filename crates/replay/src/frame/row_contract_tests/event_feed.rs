use crate::event_feed_rows::tests::row;
use crate::event_feed_rows::*;

#[test]
fn registered_history_snapshot_delta_and_older_format_keep_coverage() {
    use crate::{
        frame::{DecodedFrame, FrameDecoder, FrameEncoder, Metrics, TableView},
        rows::{TableId, TableRows, tables},
    };
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    for (hour, mut observed) in [(12, row()), (13, row())] {
        observed.read_hour = hour;
        if hour == 13 {
            observed.coverage = EventFeedCoverage::Unavailable;
            observed.failure = Some(b"world observation unavailable".to_vec());
            observed.events.clear();
        }
        let mut views = std::array::from_fn(|_| TableView::Unavailable {
            reason: "fixture".into(),
        });
        views[TableId::EventFeedHistory.index()] =
            TableView::Complete(TableRows::EventFeedHistory(vec![observed.clone()]));
        let encoded = encoder
            .encode(
                DecodedFrame {
                    branch: 1,
                    hour,
                    tables: views.map(Into::into),
                    metrics: Metrics::default(),
                },
                hour == 12,
            )
            .unwrap();
        let decoded = decoder.apply(&encoded).unwrap();
        assert_eq!(
            decoded.complete::<tables::EventFeedHistory>().unwrap(),
            &[observed]
        );
    }
    let older = crate::format::FormatVersion::read(99).unwrap();
    assert!(older.table_refusal(TableId::EventFeedHistory).is_some());
    assert!(older.table_refusal(TableId::Countries).is_none());
}
