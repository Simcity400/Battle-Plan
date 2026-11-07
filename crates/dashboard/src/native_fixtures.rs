//! A replay beside the native columnar recording of its hours, as a native preparation finds
//! it: each hour's tables written from the replay's own frame of it.

use crate::Source;
use replay::{
    columnar_file::{self, Capture},
    columnar_frames::{
        fixtures,
        synthetic::{Input, Sample, Synthetic},
    },
    container::Replay,
    publication::replace_file,
};
use replay_data::{
    identity::IdentityCodec,
    rows::{TableId, TableSchema},
};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

/// The hours of a native recording's unit: a synthetic run holds whole units.
pub(crate) const UNIT_HOURS: i32 = 720;

/// Writes `T`'s view of `frame` as the native recorder writes it: read where the frame read it,
/// and sampled at the hour its view was.
pub(crate) fn write<T: TableSchema>(walk: &mut Synthetic, frame: &DecodedFrame) {
    let index = T::ID.index();
    let read = frame.metrics.table_ns[index] != 0;
    let view = &frame.tables[index];
    let sample = match view.sampled_at() {
        Some(hour) if read && hour == frame.hour => Sample::FromReadBit,
        Some(hour) => Sample::Carried(hour),
        None => Sample::Reset,
    };
    let input = match &**view {
        TableView::Complete(rows) => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: None,
            read,
            sample,
        },
        TableView::Truncated { rows, missing } => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: Some(*missing),
            read,
            sample,
        },
        TableView::Unavailable { reason } => Input::Unavailable {
            reason,
            read,
            sample,
        },
        TableView::NotRecorded { .. } => Input::NotRecorded { read, sample },
    };
    walk.table(T::ID, input).unwrap();
}

/// `STREAMS`, the tables the native recording holds, and `write_hour`, writing each of them.
macro_rules! streams {
    ($($table:ident),+ $(,)?) => {
        const STREAMS: &[replay_data::rows::TableId] = &[$(replay_data::rows::TableId::$table),+];

        fn write_hour(
            walk: &mut replay::columnar_frames::synthetic::Synthetic,
            frame: &replay_frame::frame::DecodedFrame,
        ) {
            $($crate::native_fixtures::write::<replay_data::rows::tables::$table>(walk, frame);)+
        }
    };
}
pub(crate) use streams;

fn encoded(value: &impl IdentityCodec) -> Vec<u8> {
    let mut out = Vec::new();
    value.encode(&mut out).unwrap();
    out
}

/// A replay and the native recording beside it, in scratch of their own.
pub(crate) struct Recorded {
    root: PathBuf,
    path: PathBuf,
}

impl Recorded {
    pub(crate) fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("hoi4-native-recorded-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("campaign.battleplan");
        Self { root, path }
    }

    /// The scratch folder, which the recording is removed with.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Writes `replay` over the one written before, as a recorder grows it, and beside it the
    /// native recording of `runs` (branch, first hour, units) holding exactly its hours, of its
    /// one session: `write` writes `streams` of each hour from the replay's frame of it.
    pub(crate) fn record(
        &self,
        replay: &Replay,
        runs: &[(u64, i32, u32)],
        streams: &[TableId],
        write: fn(&mut Synthetic, &DecodedFrame),
    ) {
        let mut frames = BTreeMap::new();
        for block in &replay.index {
            replay
                .read_block_recorded_alone(
                    block,
                    streams,
                    |_, frame| {
                        frames.insert((frame.branch, frame.hour), frame.clone());
                        Ok(())
                    },
                    || Ok(()),
                )
                .unwrap();
        }
        let session = &replay.sessions[0];
        let capture = Capture {
            campaign_id: replay.campaign_id.clone(),
            content: encoded(&session.identity),
            capture_mode: replay.capture_mode.byte(),
            sessions: vec![encoded(session)],
            run_sessions: vec![0; runs.len()],
            stop: None,
            format: replay.format.version(),
            journaled: 0,
            controls: Vec::new(),
        };
        let (native, seen) = fixtures::written_captured(
            &replay.campaign_id,
            Some(&capture),
            runs,
            1,
            streams,
            |walk, run, ordinal| {
                let (branch, first, _) = runs[run as usize];
                let hour = first + ordinal as i32;
                let frame = frames
                    .get(&(branch, hour))
                    .unwrap_or_else(|| panic!("the replay records hour {hour} of branch {branch}"));
                write(walk, frame);
            },
        );
        assert_eq!(
            seen.len(),
            frames.len(),
            "the native recording holds exactly the replay's hours"
        );
        let folder = columnar_file::campaign_folder(&self.root, &replay.campaign_id);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(format!("session-{}.0.h4col", session.id)), native).unwrap();
        if self.path.exists() {
            let next = self.root.join("next");
            std::fs::write(&next, replay.bytes()).unwrap();
            replace_file(&next, &self.path).unwrap();
        } else {
            std::fs::write(&self.path, replay.bytes()).unwrap();
        }
    }

    /// The recording as a session opens it.
    pub(crate) fn source(&self, id: &str) -> Source {
        Source::open(
            crate::SourceKey {
                source_id: id.into(),
                revision: crate::protocol::replay_revision(&self.path).unwrap(),
            },
            self.path.clone(),
            &AtomicBool::new(false),
        )
        .unwrap()
    }
}

impl Drop for Recorded {
    fn drop(&mut self) {
        if let Ok(store) = crate::prepared::recording_store(&self.path) {
            let _ = store.clear();
            let _ = std::fs::remove_dir_all(store.directory());
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
