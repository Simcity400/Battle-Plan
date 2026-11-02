//! The change-run index a replay's map is read from: bound, or found in the replay's store.
use replay::container::Replay;
use std::{
    io,
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
    time::SystemTime,
};

/// Source-local read context: the change-run index serving the replay's map, bound or found
/// in the replay's store where preparation writes it.
pub struct MapBinding {
    replay: Arc<Replay>,
    columnar: Option<Arc<crate::map_source::Columnar>>,
    /// Where preparation writes the index in the replay's store, placed when the source
    /// is bound from the recording it loaded, so no read of the map opens the recording;
    /// none for a replay held by name alone, or one whose store cannot be placed.
    index: Option<PathBuf>,
    /// The prepared index file found not to serve the replay, by its length and last write.
    refused: Mutex<Option<(u64, Option<SystemTime>)>>,
}

impl MapBinding {
    pub fn new(source: &dashboard::Source) -> Arc<Self> {
        let index = match dashboard::prepared::change_runs::path(source) {
            Ok(path) => Some(path),
            // A replay held by name alone has no store beside it.
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => {
                eprintln!("Change-run map source unavailable: {error}; using the replay reader");
                None
            }
        };
        Arc::new(Self {
            replay: source.replay(),
            columnar: None,
            index,
            refused: Mutex::new(None),
        })
    }

    /// The map of `replay` served from `columnar` alone.
    pub fn columnar_only(
        replay: Arc<Replay>,
        columnar: Arc<crate::map_source::Columnar>,
    ) -> Arc<Self> {
        Arc::new(Self {
            replay,
            columnar: Some(columnar),
            index: None,
            refused: Mutex::new(None),
        })
    }

    pub fn replay_handle(&self) -> Arc<Replay> {
        Arc::clone(&self.replay)
    }

    /// The index serving the map: the one bound, else the one preparation wrote beside the
    /// replay's store when it serves this replay; a file found not to is not opened again
    /// until it changes, and without one the map is the replay's blocks. An error is an index
    /// that exists but cannot be read. It reads the index's file, never the recording.
    pub fn columnar(&self) -> io::Result<Option<Arc<crate::map_source::Columnar>>> {
        if let Some(columnar) = &self.columnar {
            return Ok(Some(Arc::clone(columnar)));
        }
        let Some(path) = &self.index else {
            return Ok(None);
        };
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let identity = (metadata.len(), metadata.modified().ok());
        let mut refused = self.refused.lock().unwrap_or_else(PoisonError::into_inner);
        if *refused == Some(identity) {
            return Ok(None);
        }
        let file = match replay::columnar_runs::IndexFile::open(path) {
            Ok(file) => Arc::new(file),
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                *refused = Some(identity);
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let columnar = crate::map_source::Columnar::verify(&self.replay, file);
        if columnar.is_none() {
            *refused = Some(identity);
        }
        Ok(columnar)
    }
}
