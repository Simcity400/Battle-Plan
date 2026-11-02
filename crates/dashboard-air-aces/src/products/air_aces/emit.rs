//! Air aces' ordered emit on one branch: the observer the row walk kept ([`Prepared`]), given
//! the hours a segment's readers kept ([`super::segment`]) in order as the walk gave it frames,
//! each quiet hour on the frame of the last kept, and publishing the pages the walk published:
//! each replay block's window once its hours are observed, and at each flush the roster
//! continuation, its read series and its variant catalogue.

use super::{
    prepared::Prepared,
    segment::{Kept, Segments, TABLES},
};
use dashboard_base::{prepared::frame_changes::FrameChanges, source::Source};
use dashboard_core::config::LoadContext;
use dashboard_keyed::segment::Hour;
use replay::container::Replay;
use replay_data::rows::{TABLE_COUNT, TableId};
use replay_frame::frame::{DecodedFrame, Metrics, not_selected};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

/// The positions in the recording's index of `branch`'s blocks, in order.
fn blocks(replay: &Replay, branch: u64) -> Vec<usize> {
    replay
        .index
        .iter()
        .enumerate()
        .filter(|(_, block)| block.branch == branch)
        .map(|(position, _)| position)
        .collect()
}

/// What a branch's emit saves: its observer after its last flush, its variant keys, the next
/// block it publishes, and the rows its retained runs hold, which its observer does not save
/// ([`Prepared::run_rows`]).
#[derive(Serialize)]
struct Saving<'a> {
    prepared: &'a Prepared,
    keys: &'a BTreeMap<String, u64>,
    next: usize,
    rows: Vec<(usize, Vec<u8>)>,
}

#[derive(Deserialize)]
struct Saved {
    prepared: Prepared,
    keys: BTreeMap<String, u64>,
    next: usize,
    rows: Vec<(usize, Vec<u8>)>,
}

/// The family's emit on one branch.
pub struct Emit {
    prepared: Box<Prepared>,
    /// The frame the observer reads: the views of the last hour kept.
    frame: DecodedFrame,
    branch: u64,
    blocks: Vec<usize>,
    /// The next of `blocks` it publishes.
    next: usize,
}

fn frame(branch: u64) -> DecodedFrame {
    DecodedFrame {
        branch,
        hour: 0,
        tables: std::array::from_fn(|_| not_selected()),
        metrics: Metrics::default(),
    }
}

impl Emit {
    pub fn new(replay: &Replay, branch: u64) -> Self {
        Self {
            prepared: Box::default(),
            frame: frame(branch),
            branch,
            blocks: blocks(replay, branch),
            next: 0,
        }
    }

    /// Observes one hour, publishing every block ending before it first and the block it ends
    /// after it.
    pub fn observe(
        &mut self,
        writer: &mut store::Writer,
        source: &Source,
        replay: &Replay,
        context: &LoadContext,
        hour: Hour<Segments>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let at = hour.hour;
        self.publish(writer, source, replay, context, at, cancelled)?;
        let changes = match hour.read {
            None => {
                let mut slot_read = [false; TABLE_COUNT];
                slot_read[TableId::AirAces.index()] = hour.dated;
                FrameChanges::observed([false; TABLE_COUNT], [false; TABLE_COUNT], slot_read, None)
            }
            Some(Kept {
                views,
                metrics,
                changes,
            }) => {
                for (&table, view) in TABLES.iter().zip(views.into_vec()) {
                    self.frame.tables[table.index()] = view;
                }
                self.frame.metrics = *metrics;
                *changes
            }
        };
        self.frame.hour = at;
        self.prepared.observe_changed(&self.frame, replay, &changes)?;
        self.publish(writer, source, replay, context, at.saturating_add(1), cancelled)
    }

    /// Publishes the window of every block ending before `before`.
    fn publish(
        &mut self,
        writer: &mut store::Writer,
        source: &Source,
        replay: &Replay,
        context: &LoadContext,
        before: i32,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        while let Some(&position) = self.blocks.get(self.next)
            && replay.index[position].last_hour < before
        {
            self.prepared
                .finish_block(writer, source, replay, position, context, cancelled)?;
            self.next += 1;
        }
        Ok(())
    }

    /// Publishes what it kept since it last did; every block left once the branch ends
    /// (`through`).
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        source: &Source,
        replay: &Replay,
        context: &LoadContext,
        through: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        if through.is_some() {
            self.publish(writer, source, replay, context, i32::MAX, cancelled)?;
        }
        self.prepared.flush(writer, self.branch, cancelled)
    }

    /// What it goes on with after its last flush.
    pub fn save(&self) -> io::Result<Vec<u8>> {
        let saving = Saving {
            prepared: &self.prepared,
            keys: self.prepared.keys(),
            next: self.next,
            rows: self.prepared.run_rows()?,
        };
        dashboard_base::prepared::series::compress(&serde_json::to_vec(&saving)?, 3)
    }

    /// The emit [`Self::save`] wrote. It goes on at a segment's first hour, which restates
    /// every table.
    pub fn load(replay: &Replay, branch: u64, saved: &[u8]) -> io::Result<Self> {
        let bytes = dashboard_base::prepared::series::decompress(saved)?;
        let Saved {
            mut prepared,
            keys,
            next,
            rows,
        } = serde_json::from_slice(&bytes)?;
        prepared.set_keys(keys);
        prepared.restore_run_rows(&rows)?;
        Ok(Self {
            prepared: Box::new(prepared),
            frame: frame(branch),
            branch,
            blocks: blocks(replay, branch),
            next,
        })
    }
}
