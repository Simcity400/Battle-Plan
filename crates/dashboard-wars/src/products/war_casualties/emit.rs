//! The family's ordered emit on one branch: the casualty observer the row walk kept
//! ([`Prepared`]), observing each projected hour ([`facts::Hour`]) and publishing the same pages.
//! An hour carried whole is observed as the walk observed a frame; one carrying moved relations
//! reads only those; a quiet one advances in constant time. Each replay block's pair window is
//! published once its hours are kept, as the walk published it at the block's end.

use super::{
    check_cadence,
    facts::{self, Facts},
    prepared::{Prepared, PreparedContent},
};
use replay::campaign::Replay;
use replay_data::{cadence, rows::TableId};
use replay_frame::frame::{DecodedFrame, not_selected};
use serde::{Deserialize, Serialize};
use std::{io, sync::atomic::AtomicBool};

/// Each replay block of `branch`: its first and last hours.
fn blocks(replay: &Replay, branch: u64) -> Vec<(i32, i32)> {
    replay
        .index
        .iter()
        .filter(|block| block.branch == branch)
        .map(|block| (block.first_hour, block.last_hour))
        .collect()
}

/// What a branch's emit saves: its observer after its last flush, and the next block it
/// publishes.
#[derive(Serialize)]
struct Saving<'a> {
    prepared: &'a Prepared,
    next: usize,
}

#[derive(Deserialize)]
struct Saved {
    prepared: Prepared,
    next: usize,
}

/// The family's emit on one branch.
pub struct Emit {
    prepared: Box<Prepared>,
    branch: u64,
    blocks: Vec<(i32, i32)>,
    /// The next block it publishes.
    next: usize,
}

impl Emit {
    /// A branch's emit, presenting its replies with `content`.
    pub fn new(replay: &Replay, branch: u64, content: &PreparedContent) -> io::Result<Self> {
        check_cadence(cadence::cadence_in(TableId::WarRelations, replay.format))?;
        let mut prepared = Prepared::new(replay, branch);
        prepared.configure(content);
        Ok(Self {
            prepared: Box::new(prepared),
            branch,
            blocks: blocks(replay, branch),
            next: 0,
        })
    }

    /// Keeps one projected hour, publishing every block ending before it first and the block it
    /// ends after it.
    pub fn observe(
        &mut self,
        writer: &mut store::Writer,
        replay: &Replay,
        hour: facts::Hour,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let at = hour.hour;
        self.publish(writer, at)?;
        match hour.facts.map(|facts| *facts) {
            None => self.prepared.observe_quiet(at)?,
            Some(Facts::Whole { tables, metrics }) => {
                let mut frame = DecodedFrame {
                    branch: hour.branch,
                    hour: at,
                    metrics: *metrics,
                    tables: std::array::from_fn(|_| not_selected()),
                };
                for (table, view) in tables {
                    frame.tables[table.index()] = view;
                }
                self.prepared.observe_whole(&frame, replay, cancelled)?;
            }
            Some(Facts::Moved { relations, names }) => {
                let names = names.map(|(views, metrics)| (views, *metrics));
                self.prepared
                    .observe_moved(at, &relations, names, cancelled)?;
            }
        }
        self.publish(writer, at.saturating_add(1))
    }

    /// Publishes the pair window of every block ending before `before`.
    fn publish(&mut self, writer: &mut store::Writer, before: i32) -> io::Result<()> {
        while let Some(&(first, last)) = self.blocks.get(self.next)
            && last < before
        {
            self.prepared
                .finish_block(writer, self.branch, first, last)?;
            self.next += 1;
        }
        Ok(())
    }

    /// Publishes what it kept since it last did; every block left once the branch ends
    /// (`through`).
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        through: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        if through.is_some() {
            self.publish(writer, i32::MAX)?;
        }
        let keep = self.blocks.get(self.next).map(|&(first, _)| first);
        self.prepared
            .flush_keeping(writer, self.branch, keep, cancelled)
    }

    /// What it goes on with after its last flush.
    pub fn save(&self) -> io::Result<Vec<u8>> {
        let saving = Saving {
            prepared: &*self.prepared,
            next: self.next,
        };
        dashboard_base::prepared::series::compress(&serde_json::to_vec(&saving)?, 3)
    }

    /// The emit [`Self::save`] wrote, presenting its replies with `content` as [`Self::new`]
    /// does.
    pub fn load(
        replay: &Replay,
        branch: u64,
        content: &PreparedContent,
        saved: &[u8],
    ) -> io::Result<Self> {
        check_cadence(cadence::cadence_in(TableId::WarRelations, replay.format))?;
        let bytes = dashboard_base::prepared::series::decompress(saved)?;
        let Saved { mut prepared, next } = serde_json::from_slice(&bytes)?;
        prepared.configure(content);
        Ok(Self {
            prepared: Box::new(prepared),
            branch,
            blocks: blocks(replay, branch),
            next,
        })
    }
}

#[cfg(test)]
mod tests;
