//! Air superiority's ordered emit on one branch: the fold ([`Prepared`]) of each hour its
//! segments' operators projected ([`super::columnar`]), in order, each day's sides as the
//! operators' own reads of the groups assigned them. At each flush it publishes the fold's
//! series, facts, catalogue and continuation; its replies read the groups' own pages.

use super::{columnar::Hour, prepared::Prepared};
use dashboard_base::prepared::series::{compress, decompress};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

/// What a branch's emit saves: the fold after its last flush and its series keys, which its
/// continuation does not save.
#[derive(Serialize)]
struct Saving<'a> {
    prepared: &'a Prepared,
    keys: &'a BTreeMap<String, u64>,
}

#[derive(Deserialize)]
struct Saved {
    prepared: Prepared,
    keys: BTreeMap<String, u64>,
}

/// The family's emit on one branch.
#[derive(Default)]
pub struct Emit {
    prepared: Box<Prepared>,
}

impl Emit {
    pub fn new() -> Self {
        Self::default()
    }

    /// Keeps one hour.
    pub fn observe(&mut self, hour: Hour) -> io::Result<()> {
        let Hour { hour, air, .. } = hour;
        self.prepared.observe_columnar(hour, air)
    }

    /// Publishes the fold's pages kept since it last did.
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.prepared.flush(writer, branch, cancelled)
    }

    /// What it goes on with after its last flush.
    pub fn save(&self) -> io::Result<Vec<u8>> {
        let saving = Saving {
            prepared: &self.prepared,
            keys: self.prepared.keys(),
        };
        compress(&serde_json::to_vec(&saving)?, 3)
    }

    /// The emit [`Self::save`] wrote. It goes on at a segment's first hour, whose operator gives
    /// every part whole.
    pub fn load(saved: &[u8]) -> io::Result<Self> {
        let Saved { mut prepared, keys } = serde_json::from_slice(&decompress(saved)?)?;
        prepared.set_keys(keys);
        Ok(Self {
            prepared: Box::new(prepared),
        })
    }
}
