//! The dashboard's query layer (SPEC part 3): every reply is made of a few reads of the
//! recording's own tables, through a timeline ([`timeline`]) and the one identity rule
//! ([`identity`]), from the reads at the storage level ([`read`], [`pieces`]). Declared series
//! are served with no product code ([`series`]). A reply costs what it asks: the rows, blocks,
//! pieces or pages it names, never the campaign's length.
//!
//! What it keeps while a recording is open is index state, not answers: each timeline's runs
//! and each timeline's holder runs ([`Index`]).

pub mod chart;
pub mod cost;
#[cfg(test)]
mod fixture;
pub mod identity;
pub mod log;
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

/// What the layer keeps of one open recording: each timeline asked, resolved, and its holders.
#[derive(Default)]
pub struct Index {
    timelines: Mutex<HashMap<u64, (Arc<timeline::Timeline>, Arc<identity::Holders>)>>,
}

impl Index {
    /// A query of timeline `id`: its runs and holders read once, then kept.
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

/// One timeline of one recording, as a reply reads it.
pub struct Query<'a> {
    pub replay: &'a Replay,
    pub timeline: Arc<timeline::Timeline>,
    pub holders: Arc<identity::Holders>,
}

/// The timeline a request's `branchId` names.
pub fn branch(params: &serde_json::Value) -> io::Result<u64> {
    params
        .get("branchId")
        .and_then(serde_json::Value::as_str)
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid branch"))
}
