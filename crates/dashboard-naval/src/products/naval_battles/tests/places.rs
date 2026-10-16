//! Detail observation of a frame without the tables sunk ships' place names
//! read: an hour that finds no ship observes as with them, and one that
//! finds a ship stops preparation rather than title it from that frame.
use super::*;
use dashboard_base::prepared::frame_changes::FrameChangeTracker;
use replay::rows::{SunkShipEntry, SunkShipsRow};

/// Hour `hour` of the fixture with one sunk ship retained since hour 100.
fn sunk_frame(hour: i32) -> DecodedFrame {
    let mut f = frame(hour);
    *f.tables[TableId::SunkShips.index()] =
        TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow {
            entries: vec![SunkShipEntry {
                ship_name: b"synthetic sinking".to_vec(),
                victim_country: 22,
                attacker_country: 11,
                sink_hour: 100,
                battle: 900,
                province: 45,
                province_present: 1,
                ..Default::default()
            }],
        }]));
    f.metrics.table_ns[TableId::SunkShips.index()] = 1;
    f
}

#[test]
fn naval_details_without_place_tables_stop_at_a_ship_first_read_there() {
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let frames = [sunk_frame(100), sunk_frame(101)];
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "naval-places".into(),
            revision: "1".into(),
        },
        "naval-places.battleplan".into(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap()),
        &flag,
    )
    .unwrap();
    let format = source.replay_ref().format;
    let content = crate::products::ships_sunk::prepared_content(&source, &context);
    let observe = |places: [bool; 2]| -> Vec<io::Result<()>> {
        let mut scan = Scan::default();
        scan.configure(&source, 1, &content).unwrap();
        let mut tracker = FrameChangeTracker::new();
        frames
            .iter()
            .zip(places)
            .map(|(frame, places)| {
                let changes = tracker.observe(frame, format);
                scan.observe_prepared(frame, &changes);
                scan.observe_details_read(&source, &context, frame, &flag, &changes, places)
            })
            .collect()
    };
    // With the tables at the hour the ship is first read, the hour after
    // finds no ship and observes without them.
    let read = observe([true, false]);
    assert!(read.iter().all(Result::is_ok), "{read:?}");
    // Without them there, preparation stops: not a refusal of the recording.
    let read = observe([false, false]);
    let error = read[0].as_ref().unwrap_err();
    assert_ne!(error.kind(), io::ErrorKind::InvalidData);
    assert!(
        error.to_string().contains("hour 100"),
        "the error names the hour: {error}"
    );
}
