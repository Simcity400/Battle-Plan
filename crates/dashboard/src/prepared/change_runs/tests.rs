use super::*;
use dashboard_base::test_fixtures::{CAMPAIGN, file_source, records, session, source};
use replay::{
    columnar_file::Capture,
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    columnar_runs::captures,
    container::Replay,
    spool::{ControlRecord, Record},
};
use replay_data::{
    identity::{CaptureMode, IdentityCodec},
    rows::{CampaignRow, TableRows},
};
use replay_frame::frame::{DecodedFrame, TableView};
use std::io::Write;

/// A replay of branch 1 at hours 100 and 101, of `campaign`, its session still being recorded.
fn replay(campaign: &[u8]) -> Replay {
    recorded(campaign, false)
}

/// [`replay`], its session's detach recorded when `detached`.
fn recorded(campaign: &[u8], detached: bool) -> Replay {
    let frame = |hour: i32| -> DecodedFrame {
        let mut frame = source(1, hour, None);
        frame.tables[TableId::Campaign.index()] =
            TableView::Complete(TableRows::Campaign(vec![CampaignRow {
                id: campaign.to_vec(),
            }]))
            .into();
        frame
    };
    let mut records = records(1, vec![frame(100), frame(101)]);
    if detached {
        records.push(Record::Control(ControlRecord::Detach {
            session: 1,
            last_hour: Some(101),
        }));
    }
    Replay::from_records(campaign.to_vec(), &[records]).unwrap()
}

fn encoded(value: &impl IdentityCodec) -> Vec<u8> {
    let mut out = Vec::new();
    value.encode(&mut out).unwrap();
    out
}

/// A capture is the replay's only when the replay records an hour of it: a continuation of a
/// branch the replay holds, published before a save that failed, is not yet.
#[test]
fn a_capture_is_the_replay_s_only_when_it_records_an_hour_of_it() {
    let replay = recorded(CAMPAIGN, true);
    assert!(records_any(&replay, &[(1, 100, 2)]));
    assert!(records_any(&replay, &[(1, 90, 11)]));
    assert!(records_any(&replay, &[(2, 100, 1), (1, 101, 5)]));
    assert!(!records_any(&replay, &[(1, 102, 2)]));
    assert!(!records_any(&replay, &[(1, 90, 10)]));
    assert!(!records_any(&replay, &[(2, 100, 2)]));
    assert!(!records_any(&replay, &[(1, 100, 0)]));
}

/// A native recording's capture serves only the replay of its sessions: of its campaign and
/// capture mode, each run's branch of the session it names, with that session's content.
#[test]
fn a_native_capture_serves_only_the_replay_of_its_sessions() {
    let replay = replay(CAMPAIGN);
    let runs = [(1, 100, 2)];
    let held = session(1);
    let capture = Capture {
        campaign_id: CAMPAIGN.to_vec(),
        content: encoded(&held.identity),
        capture_mode: CaptureMode::Full.byte(),
        sessions: vec![encoded(&held)],
        run_sessions: vec![0],
        stop: None,
        format: 1,
        journaled: 0,
        controls: Vec::new(),
    };
    assert!(captures(&capture, &runs, &replay));
    let mut content = held.identity.clone();
    content.game_version = "another".into();
    for other in [
        Capture {
            campaign_id: b"another campaign".to_vec(),
            ..capture.clone()
        },
        Capture {
            content: encoded(&content),
            ..capture.clone()
        },
        Capture {
            capture_mode: CaptureMode::Full.byte() + 1,
            ..capture.clone()
        },
        Capture {
            sessions: vec![encoded(&session(2))],
            ..capture.clone()
        },
        Capture {
            run_sessions: vec![0, 0],
            ..capture.clone()
        },
    ] {
        assert!(!captures(&other, &runs, &replay), "{other:?}");
    }
    // A run of a branch the replay does not record.
    assert!(!captures(&capture, &[(2, 100, 2)], &replay));
}

/// A preparation publishes its own whole file: another of the same index, started and
/// finished while it writes, neither cuts it short nor is cut short by it, and one that fails
/// leaves the index in place and no file of its own.
#[test]
fn a_preparation_publishes_only_its_own_whole_file() {
    let root = std::env::temp_dir().join(format!("change-runs-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let target = root.join("index.h4runs");
    publish(&target, |mut first| {
        first.write_all(&[1; 1_000])?;
        publish(&target, |mut second| {
            second.write_all(&[2; 10])?;
            Ok(second)
        })?;
        assert_eq!(fs::read(&target)?, [2; 10]);
        first.write_all(&[1; 1_000])?;
        Ok(first)
    })
    .unwrap();
    assert_eq!(fs::read(&target).unwrap(), [1; 2_000]);
    let failed = publish(&target, |mut file| {
        file.write_all(&[3; 5])?;
        Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"))
    });
    assert_eq!(failed.unwrap_err().kind(), io::ErrorKind::Interrupted);
    assert_eq!(fs::read(&target).unwrap(), [1; 2_000]);
    let left: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(left, [std::ffi::OsString::from("index.h4runs")]);
    fs::remove_dir_all(&root).unwrap();
}

/// The index is placed from the recording the source loaded, as its store is: a file now
/// naming another campaign at its path is not read, and moves nothing.
#[test]
fn the_index_is_placed_from_the_loaded_recording_without_reading_it_again() {
    let campaign = b"00000000-0000-4000-8000-000000000000";
    let (source, root) = file_source(replay(campaign));
    let identity = dashboard_base::prepared::recording_identity(source.path()).unwrap();
    let placed = path(&source).unwrap();
    assert_eq!(
        placed,
        store::Store::local(identity)
            .unwrap()
            .folder()
            .join("change-runs")
            .join("index.h4runs")
    );
    fs::write(
        source.path(),
        replay(b"00000000-0000-4000-8000-000000000001").bytes(),
    )
    .unwrap();
    assert_ne!(
        dashboard_base::prepared::recording_identity(source.path()).unwrap(),
        identity
    );
    assert_eq!(path(&source).unwrap(), placed);
    fs::remove_dir_all(&root).unwrap();
}

/// Native recordings of a replay's campaign that cannot be indexed whole are refused by name,
/// and an index left from recordings since gone is removed; a replay still being recorded with
/// none has nothing refused: its index holds no native recording and its session pending.
#[test]
fn native_recordings_that_cannot_be_indexed_whole_are_refused_by_name() {
    let (source, root) = file_source(replay(CAMPAIGN));
    let stop = AtomicBool::new(false);
    let target = path(&source).unwrap();
    assert_eq!(prepare(&source, &stop).unwrap().refused, None);
    let summary = IndexFile::summary_of(&target).unwrap();
    assert!(summary.natives.is_empty() && summary.runs.is_empty());
    assert_eq!(summary.pending, [encoded(&source.replay().sessions[0])]);
    assert_eq!(outcome(&source).unwrap(), Outcome::Indexed);
    fs::write(&target, b"an index of native recordings since gone").unwrap();
    assert_eq!(outcome(&source).unwrap(), Outcome::Stale);
    let native = columnar_file::campaign_folder(&root, CAMPAIGN).join("session-1.0.h4col");
    fs::create_dir_all(native.parent().unwrap()).unwrap();
    fs::write(&native, [0_u8; 4_096]).unwrap();
    let reason = prepare(&source, &stop).unwrap().refused.unwrap();
    assert!(reason.contains("session-1.0.h4col"), "{reason}");
    assert!(!target.exists());
    assert_eq!(outcome(&source).unwrap(), Outcome::Refused(reason));
    crate::prepared::clear(&source, None, &stop).unwrap();
    fs::remove_dir_all(&root).unwrap();
}

/// A replay that records its session's end, with no native recording of it, is refused,
/// naming the session, and the index there is removed: no native recording is not taken for
/// none needed. The refusal stands while its native recordings are as they were.
#[test]
fn an_ended_session_with_no_native_recording_is_refused_by_name() {
    let (source, root) = file_source(recorded(CAMPAIGN, true));
    let stop = AtomicBool::new(false);
    let target = path(&source).unwrap();
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, b"an index of a native recording since gone").unwrap();
    let reason = prepare(&source, &stop).unwrap().refused.unwrap();
    assert!(reason.contains("session 1"), "{reason}");
    assert!(!target.exists());
    let Err(refused) = natives(&source).unwrap() else {
        panic!("an ended session with no native recording was taken");
    };
    assert_eq!(refused, reason);
    assert_eq!(outcome(&source).unwrap(), Outcome::Refused(reason));
    crate::prepared::clear(&source, None, &stop).unwrap();
    fs::remove_dir_all(&root).unwrap();
}

/// Clearing a recording's preparation leaves nothing of its index: neither the index, nor why
/// its native recordings were refused, nor a file a preparation left part written.
#[test]
fn clearing_a_preparation_removes_its_index_its_refusal_and_part_written_files() {
    let stop = AtomicBool::new(false);
    let (source, root) = file_source(replay(CAMPAIGN));
    assert_eq!(prepare(&source, &stop).unwrap().refused, None);
    let held = folder(&source).unwrap();
    assert!(path(&source).unwrap().exists());
    fs::write(held.join("index.h4runs.1-0.partial"), b"part written").unwrap();
    crate::prepared::clear(&source, None, &stop).unwrap();
    assert!(!held.exists());
    assert_eq!(outcome(&source).unwrap(), Outcome::Stale);
    let (ended, ended_root) = file_source(recorded(CAMPAIGN, true));
    assert!(prepare(&ended, &stop).unwrap().refused.is_some());
    let held = folder(&ended).unwrap();
    assert!(held.join("refused.txt").exists());
    crate::prepared::clear(&ended, None, &stop).unwrap();
    assert!(!held.exists());
    assert_eq!(outcome(&ended).unwrap(), Outcome::Stale);
    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&ended_root).unwrap();
}

/// Native recordings refused for a recording's index are its campaign map's refusal alone: the
/// map's status row names why, and every other row is what the same recording, still being
/// recorded, prepares. Restored, the refusal is named again without the index being prepared
/// again, until its native recordings change.
#[test]
fn a_refused_index_is_the_campaign_map_s_status_and_restored_until_its_natives_change() {
    use crate::prepared::{Section, State, driver};
    let stop = AtomicBool::new(false);
    let map = |sections: &[Section]| -> Section {
        sections
            .iter()
            .find(|section| section.id == "map")
            .expect("the campaign map's status row")
            .clone()
    };
    let others = |sections: &[Section]| -> Vec<(String, State)> {
        sections
            .iter()
            .filter(|section| section.id != "map")
            .map(|section| (section.id.clone(), section.state))
            .collect()
    };
    let (live, live_root) = file_source(replay(CAMPAIGN));
    let expected = driver::prepare(&live, None, &stop, |_| {}).unwrap();
    let (source, root) = file_source(recorded(CAMPAIGN, true));
    let accepted = driver::prepare(&source, None, &stop, |_| {}).unwrap();
    let refused = map(&accepted.sections);
    assert_eq!(refused.state, State::Failed);
    let Outcome::Refused(reason) = outcome(&source).unwrap() else {
        panic!("the refusal does not stand");
    };
    assert!(reason.contains("session 1"), "{reason}");
    assert!(
        refused
            .error
            .as_deref()
            .is_some_and(|error| error.ends_with(&reason)),
        "{refused:?}"
    );
    assert!(
        map(&expected.sections)
            .error
            .is_none_or(|error| !error.contains("change-run index"))
    );
    assert_eq!(others(&accepted.sections), others(&expected.sections));
    let restored = driver::restore(&source, None, &stop)
        .unwrap()
        .expect("a refusal that stands is restored");
    assert_eq!(map(&restored.sections).error, refused.error);
    let folder = columnar_file::campaign_folder(&root, CAMPAIGN);
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("session-1.0.h4col"), [0_u8; 4_096]).unwrap();
    assert_eq!(outcome(&source).unwrap(), Outcome::Stale);
    assert!(driver::restore(&source, None, &stop).unwrap().is_none());
    for (source, root) in [(source, root), (live, live_root)] {
        crate::prepared::clear(&source, None, &stop).unwrap();
        fs::remove_dir_all(&root).unwrap();
    }
}

/// What faction states' programmes and minefield reports read that row frames leave to the
/// native capture.
const LEFT: [TableId; 5] = [
    TableId::ProjectPrograms,
    TableId::Projects,
    TableId::VariantDefinitions,
    TableId::CountryCounterInputs,
    TableId::NavalAccidentReports,
];

/// `T` read whole and empty at the hour.
fn read_empty<T: replay_data::rows::TableSchema>(walk: &mut Synthetic) {
    walk.table::<T::Row>(
        T::ID,
        Input::Snapshot {
            rows: &[],
            partial: None,
            read: true,
            sample: Sample::FromReadBit,
        },
    )
    .unwrap();
}

/// The native recorder's hour of [`LEFT`] and the campaign.
fn write_left(walk: &mut Synthetic, frame: &DecodedFrame) {
    use replay_data::rows::tables;
    crate::native_fixtures::write::<tables::Campaign>(walk, frame);
    read_empty::<tables::ProjectPrograms>(walk);
    read_empty::<tables::Projects>(walk);
    read_empty::<tables::VariantDefinitions>(walk);
    read_empty::<tables::CountryCounterInputs>(walk);
    read_empty::<tables::NavalAccidentReports>(walk);
}

/// The index holds every table a registered product reads through the native hour reader: a
/// native capture holding what faction states and minefields read, which their row frames leave
/// to it, is served to both from the index preparation keeps.
#[test]
fn a_native_capture_holding_what_products_read_is_served_from_its_index() {
    use crate::native_fixtures::{Recorded, UNIT_HOURS};
    let frame = |hour: i32| {
        let mut frame = source(1, hour, None);
        for table in LEFT {
            *frame.tables[table.index()] = TableView::NotRecorded {
                mode: CaptureMode::Full,
            };
        }
        for table in std::iter::once(TableId::Campaign).chain(LEFT) {
            frame.metrics.table_ns[table.index()] = 1;
        }
        frame
    };
    let mut input = records(1, (0..UNIT_HOURS).map(frame).collect());
    input.push(Record::Control(ControlRecord::Detach {
        session: 1,
        last_hour: Some(UNIT_HOURS - 1),
    }));
    let replay = Replay::from_records(CAMPAIGN.to_vec(), &[input]).unwrap();
    let recorded = Recorded::new();
    let streams: Vec<TableId> = std::iter::once(TableId::Campaign).chain(LEFT).collect();
    recorded.record(&replay, &[(1, 0, 1)], &streams, write_left);
    let source = recorded.source("native");
    let stop = AtomicBool::new(false);
    assert_eq!(prepare(&source, &stop).unwrap().refused, None);
    assert_eq!(outcome(&source).unwrap(), Outcome::Indexed);
    let mut served = Vec::new();
    for key in ["faction-state", "minefields"] {
        let read = (crate::products::find(key).unwrap().native_tables)();
        let frame =
            dashboard_base::prepared::query::native::frame_at(&source, 1, 100, &read, &stop)
                .unwrap()
                .expect("hour 100 is recorded");
        for table in LEFT.into_iter().filter(|table| read.contains(table)) {
            assert!(
                matches!(&*frame.tables[table.index()], TableView::Complete(_)),
                "{key} reads {table:?} unserved"
            );
            served.push(table);
        }
    }
    let mut left = LEFT.to_vec();
    for tables in [&mut served, &mut left] {
        tables.sort_unstable();
        tables.dedup();
    }
    assert_eq!(served, left);
}
