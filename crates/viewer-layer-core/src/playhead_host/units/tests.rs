use crate::test_support::reloaded_playback::{power, reloaded, segment_of};
use super::of;
use crate::playhead_host::{
    open,
    views::{Need, Pace, apart::Apart, tests::hourly_steps},
};
use playhead::Piece;
use replay::{
    campaign::SeekResult,
};
use replay_data::{
    BranchId,
    rows::{TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use std::{sync::Arc, time::Duration};

const LIMIT: Duration = Duration::from_secs(120);

fn political_power(frame: &DecodedFrame) -> Option<i64> {
    frame.complete::<tables::Countries>().ok()?.first()?.political_power_raw
}

#[test]
fn a_recording_loaded_back_in_time_has_a_playhead_on_each_timeline_and_none_on_a_branch_it_does_not_hold() {
    let replay = reloaded(1_000);
    let ids: Vec<BranchId> = replay.timelines().unwrap().list.iter().map(|timeline| timeline.id).collect();
    assert_eq!(ids, [1, 2], "the recording's timelines");
    let pieces = |branch: BranchId| of(&replay, branch).unwrap_or_else(|error| panic!("branch {branch} is refused: {error}")).timeline.pieces().to_vec();
    assert_eq!(pieces(1), [Piece { run: 0, first: 1_000, last: 1_005 }]);
    assert_eq!(pieces(2), [Piece { run: 0, first: 1_000, last: 1_002 }, Piece { run: 1, first: 1_003, last: 1_006 }, Piece { run: 2, first: 1_007, last: 1_008 }]);
    let refused = of(&replay, 9).err().expect("a branch the recording does not hold is refused");
    assert!(refused.to_string().contains("holds no branch 9"), "refused as {refused}");
}

#[test]
fn rows_at_the_head_of_a_timeline_are_its_own_pieces_and_never_another_timelines_hours() {
    let base = 2_000;
    let replay = reloaded(base);
    for (branch, last) in [(2, base + 8), (1, base + 5)] {
        let entry = open(of(&replay, branch).unwrap(), hourly_steps()).unwrap();
        entry.set_capture(replay.capture_mode);
        let mut apart = Apart::new(&entry, branch).unwrap();
        let reader = apart.store().handle().reader_at("branch rows", &[Need::every(TableId::Countries)], Pace::Step);
        for hour in base..=last {
            entry.playhead().seek(hour);
            assert!(apart.drive(hour), "branch {branch}: the store never reached hour {hour}");
            let frame = reader.wait(|frame| frame.hour == hour, LIMIT).unwrap_or_else(|error| panic!("branch {branch}, hour {hour}: {error}"));
            let segment = segment_of(base, branch, hour);
            assert_eq!(political_power(&frame), Some(power(segment, hour)), "branch {branch}, hour {hour} reads segment {segment}");
            assert_eq!(frame.branch, branch, "hour {hour}'s rows are stamped with the branch asked");
            let today = match replay.seek_selected(branch, hour, Some(&[TableId::Countries])).unwrap() {
                SeekResult::Frame(frame) => political_power(&frame),
                SeekResult::Gap { reason, .. } => panic!("today's read of branch {branch} has a gap at hour {hour}: {reason}"),
            };
            assert_eq!(political_power(&frame), today, "branch {branch}, hour {hour}: today's read");
        }
    }
}

#[test]
fn the_segments_of_one_timeline_share_one_playhead_and_another_timeline_has_its_own() {
    let replay = reloaded(3_000);
    let (first, again, other) = (open(of(&replay, 2).unwrap(), hourly_steps()).unwrap(), open(of(&replay, 2).unwrap(), hourly_steps()).unwrap(), open(of(&replay, 1).unwrap(), hourly_steps()).unwrap());
    assert!(Arc::ptr_eq(&first, &again), "a second opening of timeline 2 shares its playhead");
    assert!(!Arc::ptr_eq(&first, &other), "timeline 1 has a playhead of its own");
    assert!(Arc::ptr_eq(first.pages(), other.pages()), "both timelines read the recording's pages once");
    assert_eq!(first.playhead().timeline().pieces().len(), 3, "segments 1, 2 and 3 are pieces of timeline 2");
}

#[test]
fn source_world_pages_and_playhead_host_share_one_page_owner_without_duplicating_pages() {
    use playhead::pages::{Pages as _, WorldPages};
    use replay::campaign::Replay;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Remove(std::path::PathBuf);
    impl Drop for Remove {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let write_temp = |tag: &str, bytes: &[u8]| {
        let path = std::env::temp_dir().join(format!("units-shared-pages-{tag}-{}-{}.battleplan", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::write(&path, bytes).unwrap();
        (path.clone(), Remove(path))
    };

    let base = 4_000;
    let built = reloaded(base);
    let (path, _cleanup) = write_temp("primary", built.bytes());
    let (copy_path, _copy_cleanup) = write_temp("copy", built.bytes());
    let mut damaged_bytes = built.bytes().to_vec();
    let extent = built.recording().run(0).unwrap().directory().open[0];
    damaged_bytes[(extent.offset + extent.stored / 2) as usize] ^= 0xff;
    let (damaged_path, _damaged_cleanup) = write_temp("damaged", &damaged_bytes);

    let raw_replay = Replay::open_path(&path).unwrap();
    assert_eq!(raw_replay.recording().live_page_counts().open_blocks, 0, "opening a recording decodes no block pages eagerly");
    let source_replay = Replay::open_timelines(&path).unwrap();
    let world_pages = WorldPages::shared(source_replay.recording()).unwrap();
    let (units, _) = super::at_path(&path, Some(1)).unwrap();
    let entry = open(units, hourly_steps()).unwrap();
    assert_eq!(raw_replay.recording().pages_id(), source_replay.recording().pages_id(), "open_path and open_timelines share one page owner");
    assert_eq!(source_replay.recording().pages_id(), world_pages.recording().pages_id(), "source and world pages share one page owner");
    let world_dyn: Arc<dyn playhead::pages::Pages> = world_pages.clone();
    assert!(Arc::ptr_eq(&world_dyn, entry.pages()), "playhead host shares the same WorldPages");
    let live_after_open = source_replay.recording().live_page_counts().open_blocks;
    assert_eq!(live_after_open, source_replay.recording().run_count() as usize, "opening timelines and host caches each run's first open block once");

    let source_entry = source_replay.recording().table_entry(0, 0, TableId::Countries.index()).unwrap();
    let world_entry = world_pages.table_entry(0, 0, TableId::Countries.index()).unwrap();
    entry.playhead().seek(base);
    let mut apart = Apart::new(&entry, 1).unwrap();
    let reader = apart.store().handle().reader_at("shared pages", &[Need::every(TableId::Countries)], Pace::Step);
    assert!(apart.drive(base));
    let frame = reader.wait(|frame| frame.hour == base, LIMIT).unwrap();
    assert_eq!(political_power(&frame), Some(power(segment_of(base, 1, base), base)));
    assert_eq!(source_replay.recording().live_page_counts().open_blocks, live_after_open, "source, world pages, and playhead host share the same live open blocks");

    let copy_replay = Replay::open_timelines(&copy_path).unwrap();
    let (copy_units, _) = super::at_path(&copy_path, Some(1)).unwrap();
    let copy_entry = open(copy_units, hourly_steps()).unwrap();
    assert_ne!(source_replay.recording().pages_id(), copy_replay.recording().pages_id(), "two live paths get distinct page owners");
    assert!(!Arc::ptr_eq(&entry, &copy_entry), "two live paths get distinct playhead hosts");
    assert!(!Arc::ptr_eq(entry.pages(), copy_entry.pages()), "two live paths get distinct WorldPages");

    let damaged_replay = Replay::open_timelines(&damaged_path).unwrap();
    let (damaged_units, _) = super::at_path(&damaged_path, Some(1)).unwrap();
    let damaged_entry = open(damaged_units, hourly_steps()).unwrap();
    damaged_entry.set_capture(damaged_replay.capture_mode);
    assert_ne!(source_replay.recording().pages_id(), damaged_replay.recording().pages_id(), "damaged second path gets its own page owner");
    assert!(!Arc::ptr_eq(&entry, &damaged_entry), "damaged second path does not reuse healthy host");
    assert!(!Arc::ptr_eq(entry.pages(), damaged_entry.pages()), "damaged second path does not reuse healthy WorldPages");
    assert!(
        damaged_entry.pages().table_entry(0, 0, TableId::Countries.index()).is_err(),
        "damaged second path surfaces damage instead of reading healthy first path"
    );
    let mut damaged_apart = Apart::new(&damaged_entry, 1).unwrap();
    let damaged_reader = damaged_apart.store().handle().reader_at("damaged branch 1", &[Need::every(TableId::Countries)], Pace::Step);
    damaged_entry.playhead().seek(base);
    assert!(damaged_apart.drive(base), "damaged branch 1 still publishes instead of failing playhead");
    let damaged_frame = damaged_reader.wait(|frame| frame.hour == base, LIMIT).unwrap();
    assert!(
        matches!(&*damaged_frame.tables[TableId::Countries.index()], replay_frame::frame::TableView::Unavailable { reason } if reason.contains("BlockPage")),
        "damaged branch 1 surfaces damage as Unavailable, never NotRecorded: {:?}",
        &*damaged_frame.tables[TableId::Countries.index()]
    );
    let (_, damaged_failures) = damaged_reader.frame_with_failures().expect("damaged branch 1 failures");
    assert!(damaged_failures.iter().any(|failure| failure.table == TableId::Countries));

    let (damaged_b2_units, _) = super::at_path(&damaged_path, Some(2)).unwrap();
    let damaged_b2_entry = open(damaged_b2_units, hourly_steps()).unwrap();
    damaged_b2_entry.set_capture(damaged_replay.capture_mode);
    let mut damaged_b2_apart = Apart::new(&damaged_b2_entry, 2).unwrap();
    let damaged_b2_reader = damaged_b2_apart.store().handle().reader_at("healthy branch 2", &[Need::every(TableId::Countries)], Pace::Step);
    damaged_b2_entry.playhead().seek(base + 3);
    assert!(damaged_b2_apart.drive(base + 3), "healthy piece on branch 2 still plays");
    let damaged_b2_frame = damaged_b2_reader.wait(|frame| frame.hour == base + 3, LIMIT).unwrap();
    assert_eq!(political_power(&damaged_b2_frame), Some(power(segment_of(base, 2, base + 3), base + 3)));
    drop((source_entry, world_entry));
}

#[test]
fn a_one_run_recording_with_a_damaged_first_page_publishes_unavailable_instead_of_not_recorded() {
    use replay::campaign::Replay;
    use replay_frame::frame::TableView;

    let base = 5_000;
    let built = crate::test_support::reloaded_playback::single_run(base);
    let mut damaged_bytes = built.bytes().to_vec();
    let extent = built.recording().run(0).unwrap().directory().open[0];
    damaged_bytes[(extent.offset + extent.stored / 2) as usize] ^= 0xff;
    let damaged = Replay::open(damaged_bytes).unwrap();
    let entry = open(of(&damaged, 1).unwrap(), hourly_steps()).unwrap();
    entry.set_capture(damaged.capture_mode);
    let mut apart = Apart::new(&entry, 1).unwrap();
    let reader = apart.store().handle().reader_at("damaged first page", &[Need::every(TableId::Countries)], Pace::Step);
    entry.playhead().seek(base);
    assert!(apart.drive(base));
    let frame = reader.wait(|frame| frame.hour == base, LIMIT).unwrap();
    assert!(
        matches!(&*frame.tables[TableId::Countries.index()], TableView::Unavailable { reason } if reason.contains("BlockPage")),
        "damaged first page surfaces damage as Unavailable, never NotRecorded: {:?}",
        &*frame.tables[TableId::Countries.index()]
    );
    let (_, failures) = reader.frame_with_failures().expect("frame with failures");
    assert!(failures.iter().any(|failure| failure.table == TableId::Countries));
}
