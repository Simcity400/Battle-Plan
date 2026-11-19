use crate::playhead_host::{
    open,
    views::tests::{RECORDING, hourly_steps, record},
    shared_maps_tests::Reads,
};
use replay_data::{
    rows::{TableId, tables},
    scope::CaptureMode,
};
use replay_frame::frame::TableView;
use std::sync::{Arc, atomic::Ordering};

#[test]
fn the_dump_reads_its_hour_at_the_head_with_its_layers_composed_and_holds_nothing_once_done() {
    let entry = open(record("dump", 8), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let (layer, composes) = Reads::new(&[TableId::Countries]);
    let tables = [TableId::Countries, TableId::LandCombats];
    let head = super::at(Arc::clone(&entry), vec![layer], 5, RECORDING.branch, &tables, "units".into()).unwrap();
    let frame = head.frame();
    assert_eq!((frame.branch, frame.hour), (RECORDING.branch, 5), "the dump's own hour, not the first or the newest");
    assert_eq!(composes.load(Ordering::Relaxed), 1, "its layers are composed at the hour");
    assert_eq!(frame.complete::<tables::LandCombats>().map(<[_]>::len).ok(), Some(2), "rows at the head");
    assert_eq!(*frame.tables[TableId::Countries.index()], TableView::Unavailable { reason: "the collector could not read it".into() }, "a table's status as the recording words it");
    assert!(frame.tables[TableId::Divisions.index()].is_not_declared(), "a table the dump did not declare is never empty or not recorded");
    assert_eq!(head.declared(), tables.len(), "the dump's reader declares the tables today's map frame selects");
    assert!(super::at(Arc::clone(&entry), Vec::new(), 40, RECORDING.branch, &tables, "units".into()).is_err(), "an hour the units do not hold is refused by name");
    drop(head);
    assert_eq!(Arc::strong_count(&entry), 1, "done, the dump holds nothing of the playhead");
}
