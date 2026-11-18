//! The recording a replay's map is read from in place ([`Natives`]).
use replay::{campaign::Replay, columnar_natives::Natives};
use std::{
    io,
    sync::{Arc, Mutex, PoisonError},
};

/// Source-local read context: the replay's recording, placed for its map to be read in place.
pub struct MapBinding {
    replay: Arc<Replay>,
    /// The recording's runs, placed by the first read that asks.
    natives: Mutex<Option<Arc<Natives>>>,
}

impl MapBinding {
    pub fn new(source: &dashboard_base::Source) -> Arc<Self> {
        Arc::new(Self {
            replay: source.replay(),
            natives: Mutex::new(None),
        })
    }

    pub fn replay_handle(&self) -> Arc<Replay> {
        Arc::clone(&self.replay)
    }

    /// The replay's recording, its runs placed for reading in place.
    pub fn natives(&self) -> io::Result<Arc<Natives>> {
        let mut held = self.natives.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(natives) = held.as_ref() {
            return Ok(Arc::clone(natives));
        }
        let natives = Arc::new(Natives::new(&self.replay)?);
        *held = Some(Arc::clone(&natives));
        Ok(natives)
    }

    /// What serves the map: the replay's recording, read in place.
    pub fn columnar(&self) -> io::Result<Arc<crate::map_source::Columnar>> {
        Ok(Arc::new(crate::map_source::Columnar::new(self.natives()?)))
    }
}
