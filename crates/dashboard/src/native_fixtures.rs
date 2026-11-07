//! A campaign recording in scratch of its own, rewritten as a recorder grows it.

use crate::Source;
use replay::{campaign::Replay, publication::replace_file};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

/// The hours of a fixture's span: enough for whole blocks of every table.
pub(crate) const UNIT_HOURS: i32 = 720;

/// `STREAMS`, the tables a fixture's frames read each hour.
macro_rules! streams {
    ($($table:ident),+ $(,)?) => {
        const STREAMS: &[replay_data::rows::TableId] = &[$(replay_data::rows::TableId::$table),+];
    };
}
pub(crate) use streams;

/// A recording in scratch of its own.
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

    /// Writes `replay` over the one written before, as a recorder grows it.
    pub(crate) fn record(&self, replay: &Replay) {
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
