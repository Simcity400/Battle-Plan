
pub mod chart;
pub mod cost;
pub mod fields;
#[cfg(test)]
mod fixture;
pub mod identity;
pub mod log;
pub mod named;
pub mod pieces;
pub mod read;
pub mod series;
pub mod tile;
pub mod timeline;

use replay::campaign::Replay;
use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex, PoisonError},
};

#[derive(Default)]
pub struct Index {
    timelines: Mutex<HashMap<u64, (Arc<timeline::Timeline>, Arc<identity::Holders>)>>,
}

impl Index {
    pub fn query<'a>(&self, replay: &'a Replay, id: u64) -> io::Result<Query<'a>> {
        if let Some((timeline, holders)) = self.timelines.lock().unwrap_or_else(PoisonError::into_inner).get(&id) {
            return Ok(Query { replay, timeline: Arc::clone(timeline), holders: Arc::clone(holders) });
        }
        let timeline = Arc::new(timeline::resolve(replay, id)?);
        let holders = Arc::new(identity::holders(replay.recording(), &timeline)?);
        self.timelines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, (Arc::clone(&timeline), Arc::clone(&holders)));
        Ok(Query { replay, timeline, holders })
    }
}

pub struct Query<'a> {
    pub replay: &'a Replay,
    pub timeline: Arc<timeline::Timeline>,
    pub holders: Arc<identity::Holders>,
}

pub fn branch(params: &serde_json::Value) -> io::Result<u64> {
    params
        .get("branchId")
        .and_then(serde_json::Value::as_str)
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid branch"))
}
