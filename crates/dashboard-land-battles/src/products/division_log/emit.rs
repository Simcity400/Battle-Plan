//! The division log's ordered emit on one branch: the walk's scan, keeping each hour a columnar
//! walk projected ([`columnar::Hour`], [`Scan::observe_projected`]) and publishing each replay
//! block's window, the manifest and the totals once the block's hours are kept, as the walk
//! published them at the block's end ([`Scan::publish_block`]), and the totals at every flush
//! between, whose coverage they answer through.

use super::{Names, Scan, columnar, windows};
use dashboard_base::{products::battles::BattleWindow, source::Source};
use replay::container::Replay;
use replay_data::format::FormatVersion;
use serde::{Deserialize, Serialize};
use std::{io, sync::atomic::AtomicBool};

/// What a branch's emit saves: its scan, and the next block it publishes.
#[derive(Serialize)]
struct Saving<'a> {
    scan: &'a Scan,
    next: usize,
}

#[derive(Deserialize)]
struct Saved {
    scan: Scan,
    next: usize,
}

/// The family's emit on one branch.
pub struct Emit {
    scan: Box<Scan>,
    branch: u64,
    /// The catalogue its pages name divisions and templates from.
    names: Names,
    blocks: Vec<BattleWindow>,
    /// The next block it publishes.
    next: usize,
}

impl Emit {
    pub fn new(replay: &Replay, branch: u64, names: Names) -> Self {
        Self {
            scan: Box::new(Scan::new(replay.format)),
            branch,
            names,
            blocks: windows(replay, branch),
            next: 0,
        }
    }

    /// Keeps one projected hour, publishing every block ending before it first and the block it
    /// ends after it.
    pub fn observe(
        &mut self,
        source: &Source,
        writer: &mut store::Writer,
        hour: columnar::Hour,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let at = hour.hour;
        self.publish(source, writer, at, cancelled)?;
        self.scan.observe_projected(hour)?;
        self.publish(source, writer, at.saturating_add(1), cancelled)
    }

    /// Publishes every block ending before `before`.
    fn publish(
        &mut self,
        source: &Source,
        writer: &mut store::Writer,
        before: i32,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        while self
            .blocks
            .get(self.next)
            .is_some_and(|block| block.last_hour < before)
        {
            self.scan.publish_block_named(
                source,
                &self.names,
                writer,
                self.branch,
                self.next,
                cancelled,
            )?;
            self.next += 1;
        }
        Ok(())
    }

    /// Publishes the totals through the hour kept last, and every block left once the branch
    /// ends (`through`).
    pub fn flush(
        &mut self,
        source: &Source,
        writer: &mut store::Writer,
        through: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        if through.is_some() {
            self.publish(source, writer, i32::MAX, cancelled)?;
        }
        self.scan.publish_counts(writer, self.branch, cancelled)
    }

    /// Hours `first..` were lost from the recording, after those it published: it goes on after
    /// them, every total restarted.
    pub fn lost(&mut self, first: i32) -> io::Result<()> {
        self.scan.lost(first)
    }

    /// What it goes on with after the blocks it published.
    pub fn save(&self) -> io::Result<Vec<u8>> {
        let saving = Saving {
            scan: &*self.scan,
            next: self.next,
        };
        dashboard_base::prepared::series::compress(&serde_json::to_vec(&saving)?, 3)
    }

    /// The emit [`Self::save`] wrote, of a recording at `format`, naming from `names`.
    pub fn load(
        replay: &Replay,
        branch: u64,
        format: FormatVersion,
        names: Names,
        saved: &[u8],
    ) -> io::Result<Self> {
        let bytes = dashboard_base::prepared::series::decompress(saved)?;
        let Saved { mut scan, next } = serde_json::from_slice(&bytes)?;
        scan.configure(format);
        Ok(Self {
            scan: Box::new(scan),
            branch,
            names,
            blocks: windows(replay, branch),
            next,
        })
    }
}
