use replay::campaign::Replay;
use replay_recording::columnar_natives::Natives;
use std::{
    io,
    sync::{Arc, Mutex, PoisonError},
};

pub struct MapBinding {
    pub source: Arc<dashboard_base::Source>,
    natives: Mutex<Option<Arc<Natives>>>,
}

impl MapBinding {
    pub fn new(source: &Arc<dashboard_base::Source>) -> Arc<Self> {
        Arc::new(Self {
            source: Arc::clone(source),
            natives: Mutex::new(None),
        })
    }

    pub fn replay_handle(&self) -> Arc<Replay> {
        self.source.replay()
    }

    pub fn natives(&self) -> io::Result<Arc<Natives>> {
        let mut held = self.natives.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(natives) = held.as_ref() {
            return Ok(Arc::clone(natives));
        }
        let natives = Arc::new(Natives::new(self.source.replay_ref().recording())?);
        *held = Some(Arc::clone(&natives));
        Ok(natives)
    }

    pub fn columnar(&self) -> io::Result<Arc<crate::map_source::Columnar>> {
        Ok(Arc::new(crate::map_source::Columnar::new(self.natives()?)))
    }
}
