use super::*;
use dashboard_base::test_fixtures::{CAMPAIGN, file_source, records, source};
use replay::{
    campaign::Replay,
    spool::{ControlRecord, Record},
};
use replay_data::rows::{CampaignRow, TableRows};
use replay_frame::frame::{DecodedFrame, TableView};
use std::io::Write;

/// A replay of branch 1 at hours 100 and 101, of `campaign`, its session still being recorded.
fn replay(campaign: &[u8]) -> Replay {
    let frame = |hour: i32| -> DecodedFrame {
        let mut frame = source(1, hour, None);
        frame.tables[TableId::Campaign.index()] =
            TableView::Complete(TableRows::Campaign(vec![CampaignRow {
                id: campaign.to_vec(),
            }]))
            .into();
        frame
    };
    Replay::from_records(campaign.to_vec(), &[records(1, vec![frame(100), frame(101)])]).unwrap()
}

/// [`replay`] with its run's directory damaged: a recording that cannot be read whole.
fn damaged(campaign: &[u8]) -> Replay {
    let whole = replay(campaign);
    let directory = whole.recording().root().runs[0];
    let mut bytes = whole.bytes().to_vec();
    bytes[(directory.offset + directory.stored / 2) as usize] ^= 0xff;
    let replay = Replay::open(bytes).unwrap();
    assert!(!replay.recording().damage().is_empty());
    replay
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

/// A recording that cannot be read whole is refused by name, and an index left from another
/// is not taken for its own: the recording is indexed again.
#[test]
fn a_recording_that_cannot_be_read_whole_is_refused_by_name() {
    let (source, root) = file_source(replay(CAMPAIGN));
    let stop = AtomicBool::new(false);
    let target = path(&source).unwrap();
    assert_eq!(prepare(&source, &stop).unwrap().refused, None);
    assert_eq!(outcome(&source).unwrap(), Outcome::Indexed);
    fs::write(&target, b"an index of another recording").unwrap();
    assert_eq!(outcome(&source).unwrap(), Outcome::Stale);
    assert_eq!(prepare(&source, &stop).unwrap().refused, None);
    assert_eq!(outcome(&source).unwrap(), Outcome::Indexed);
    crate::prepared::clear(&source, None, &stop).unwrap();
    fs::remove_dir_all(&root).unwrap();

    let (source, root) = file_source(damaged(CAMPAIGN));
    let target = path(&source).unwrap();
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, b"an index of the recording before its damage").unwrap();
    let reason = prepare(&source, &stop).unwrap().refused.unwrap();
    assert!(reason.contains("cannot be read whole"), "{reason}");
    assert!(!target.exists());
    let Err(refused) = natives(&source).unwrap() else {
        panic!("a damaged recording was taken whole");
    };
    assert_eq!(refused, reason);
    assert_eq!(outcome(&source).unwrap(), Outcome::Refused(reason));
    crate::prepared::clear(&source, None, &stop).unwrap();
    fs::remove_dir_all(&root).unwrap();
}

/// Clearing a recording's preparation leaves nothing of its index: neither the index, nor why
/// its recording was refused, nor a file a preparation left part written.
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
    let (refused, refused_root) = file_source(damaged(CAMPAIGN));
    assert!(prepare(&refused, &stop).unwrap().refused.is_some());
    let held = folder(&refused).unwrap();
    assert!(held.join("refused.txt").exists());
    crate::prepared::clear(&refused, None, &stop).unwrap();
    assert!(!held.exists());
    assert!(matches!(outcome(&refused).unwrap(), Outcome::Stale));
    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&refused_root).unwrap();
}

/// A refused index is the campaign map's refusal: the map's status row names why. Restored,
/// the refusal is named again without the index being prepared again.
#[test]
fn a_refused_index_is_the_campaign_map_s_status_and_restored() {
    use crate::prepared::{Section, State, driver};
    let stop = AtomicBool::new(false);
    let map = |sections: &[Section]| -> Section {
        sections
            .iter()
            .find(|section| section.id == "map")
            .expect("the campaign map's status row")
            .clone()
    };
    let (source, root) = file_source(damaged(CAMPAIGN));
    let accepted = driver::prepare(&source, None, &stop, |_| {}).unwrap();
    let refused = map(&accepted.sections);
    assert_eq!(refused.state, State::Failed);
    let Outcome::Refused(reason) = outcome(&source).unwrap() else {
        panic!("the refusal does not stand");
    };
    assert!(reason.contains("cannot be read whole"), "{reason}");
    assert!(
        refused
            .error
            .as_deref()
            .is_some_and(|error| error.ends_with(&reason)),
        "{refused:?}"
    );
    let restored = driver::restore(&source, None, &stop)
        .unwrap()
        .expect("a refusal that stands is restored");
    assert_eq!(map(&restored.sections).error, refused.error);
    crate::prepared::clear(&source, None, &stop).unwrap();
    fs::remove_dir_all(&root).unwrap();
}

/// What faction states' programmes and minefield reports read beyond the row frames.
const LEFT: [TableId; 5] = [
    TableId::ProjectPrograms,
    TableId::Projects,
    TableId::VariantDefinitions,
    TableId::CountryCounterInputs,
    TableId::NavalAccidentReports,
];

/// The index holds every table a registered product reads through the native hour reader: a
/// native capture holding what faction states and minefields read, which their row frames leave
/// to it, is served to both from the index preparation keeps.
#[test]
fn a_native_capture_holding_what_products_read_is_served_from_its_index() {
    use crate::native_fixtures::{Recorded, UNIT_HOURS};
    let frame = |hour: i32| {
        let mut frame = source(1, hour, None);
        for table in LEFT {
            *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
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
    recorded.record(&replay);
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
