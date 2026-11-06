//! A branch of the intelligence family in its ordered emit: the observers hourly preparation
//! kept, told each projected hour ([`super::facts::Hour`]) from the reads and countries as last
//! handed on, publishing what they publish where hourly preparation did: each replay block's
//! espionage day pages once its hours are observed, and everything else at each flush, with
//! the days rendered so far when a batch ends partway through a block.

use super::{Content, Observer, Reads, TABLES, facts};
use dashboard_base::{prepared::frame_changes::FrameChanges, source::Source};
use dashboard_core::config::LoadContext;
use replay_data::rows::{TABLE_COUNT, TableId};
use replay_frame::frame::{DecodedFrame, Metrics, not_selected};
use std::{io, sync::atomic::AtomicBool};

/// What every branch's emit reads: the recording, its content options and the content bound.
#[derive(Clone, Copy)]
pub struct Inputs<'a> {
    pub source: &'a Source,
    pub context: &'a LoadContext,
    pub content: &'a Content,
}

/// The family's publication, as a branch's emit writes into it.
pub trait Pages {
    /// The writer the family's pages go into, once what it published before is committed.
    fn writer(&mut self) -> io::Result<&mut store::Writer>;
}

/// One branch's observers, the countries and reads they observe, and the next of its replay
/// blocks to finish.
pub struct Emit {
    branch: u64,
    observer: Box<Observer>,
    /// Countries and CountryLinks as last handed on; no other table.
    frame: DecodedFrame,
    reads: Box<Reads>,
    block: usize,
    /// Every block the observed hours reach is finished: what the flush writes is all the
    /// branch goes on with.
    finished: bool,
}

/// What [`Emit::save`] writes: the next block to finish.
const SAVED: usize = std::mem::size_of::<u64>();

impl Emit {
    pub fn new(branch: u64) -> Self {
        Self::with(branch, Box::default(), 0)
    }

    fn with(branch: u64, observer: Box<Observer>, block: usize) -> Self {
        Self {
            branch,
            observer,
            frame: DecodedFrame {
                branch,
                hour: 0,
                tables: std::array::from_fn(|_| not_selected()),
                metrics: Metrics::default(),
            },
            reads: Box::default(),
            block,
            finished: true,
        }
    }

    /// Observes `hour`: the blocks it passed are finished before, and its own once it is its
    /// block's last.
    pub fn observe(
        &mut self,
        inputs: Inputs<'_>,
        mut hour: facts::Hour,
        pages: &mut dyn Pages,
    ) -> io::Result<()> {
        let at = hour.hour;
        let windows = inputs.source.block_windows(self.branch);
        while windows
            .get(self.block)
            .is_some_and(|window| window.last_hour < at)
        {
            self.finish(inputs, pages.writer()?)?;
        }
        if let Some([countries, links]) = hour.countries.take() {
            self.frame.tables[TableId::Countries.index()] = countries;
            self.frame.tables[TableId::CountryLinks.index()] = links;
        }
        self.frame.hour = at;
        let touched = self.reads.apply(&mut hour);
        let reads = [
            (TableId::OperativeMemberships, facts::MEMBERSHIPS_READ),
            (TableId::Operations, facts::OPERATIONS_READ),
        ];
        let mut table_read = [false; TABLE_COUNT];
        for (table, bit) in reads {
            table_read[table.index()] = hour.reads & bit != 0;
        }
        // A segment's first hour, as the first hour of a block read alone, changed everything.
        let mut rows_changed = [hour.opens; TABLE_COUNT];
        let mut rebuilds = [hour.opens; TABLE_COUNT];
        for (position, table) in TABLES.iter().enumerate() {
            rows_changed[table.index()] |= hour.changed & (1 << position) != 0;
            rebuilds[table.index()] |= hour.rebuilt & (1 << position) != 0;
        }
        let changes = FrameChanges::observed(rows_changed, table_read, [false; TABLE_COUNT], None);
        // The roster reads less of Operations than the other observers: an hour that changed
        // none of it is no change of Operations to the roster.
        let roster = (rows_changed[TableId::Operations.index()] && !hour.roster_operations).then(|| {
            let mut rows_changed = rows_changed;
            rows_changed[TableId::Operations.index()] = false;
            FrameChanges::observed(rows_changed, table_read, [false; TABLE_COUNT], None)
        });
        let rebuilt = |table: TableId| rebuilds[table.index()];
        let cancelled = AtomicBool::new(false);
        let observed = if self.observer.reads_history_of(&changes, &self.reads) {
            // The espionage log reads its own series as last committed.
            pages.writer()?;
            let (store, key) =
                dashboard_base::prepared::files(inputs.source, inputs.context.content.as_ref())?;
            self.observer.observe_reads(
                inputs.source,
                inputs.content,
                &self.frame,
                (hour.flags, &self.reads, &touched),
                (&changes, roster.as_ref()),
                &rebuilt,
                Some((&store, &key)),
                &cancelled,
            )
        } else {
            self.observer.observe_reads(
                inputs.source,
                inputs.content,
                &self.frame,
                (hour.flags, &self.reads, &touched),
                (&changes, roster.as_ref()),
                &rebuilt,
                None,
                &cancelled,
            )
        };
        observed?;
        self.finished = false;
        if windows
            .get(self.block)
            .is_some_and(|window| window.last_hour <= at)
        {
            self.finish(inputs, pages.writer()?)?;
        }
        Ok(())
    }

    /// Finishes the next block, as hourly preparation does after observing its hours.
    fn finish(&mut self, inputs: Inputs<'_>, writer: &mut store::Writer) -> io::Result<()> {
        let cancelled = AtomicBool::new(false);
        self.observer.finish_block(
            inputs.source,
            writer,
            self.branch,
            self.block,
            &cancelled,
        )?;
        self.block += 1;
        self.finished = true;
        Ok(())
    }

    /// Writes what the observers publish at a flush into `writer`, the espionage days rendered
    /// so far among it; once `through` names the branch's last hour, every block left is
    /// finished first.
    pub fn flush(
        &mut self,
        inputs: Inputs<'_>,
        writer: &mut store::Writer,
        through: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        if through.is_some() {
            while self.block < inputs.source.block_windows(self.branch).len() {
                self.finish(inputs, writer)?;
            }
        }
        // Partway through a block, the days its coverage reaches are published with it.
        self.observer
            .publish_days(inputs.source, writer, self.branch, cancelled)?;
        self.observer.flush(writer, self.branch, cancelled)
    }

    /// What the branch goes on with beside the flush's continuation: the next block to finish.
    /// None partway through a block, whose finished days the continuation does not hold.
    pub fn save(&self) -> Option<Vec<u8>> {
        self.finished
            .then(|| (self.block as u64).to_le_bytes().to_vec())
    }

    /// The branch going on from what [`Self::save`] wrote, its observers from the continuation
    /// each section `read` reads.
    pub fn resume(
        branch: u64,
        read: &mut dyn FnMut(&str) -> io::Result<Option<store::Range>>,
        saved: &[u8],
    ) -> io::Result<Self> {
        let block: [u8; SAVED] = saved.try_into().map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "Not a saved intelligence emit")
        })?;
        let block = usize::try_from(u64::from_le_bytes(block)).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "A saved block past the recording")
        })?;
        let observer = Observer::read_saved(branch, read)?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "The intelligence continuation is missing",
            )
        })?;
        Ok(Self::with(branch, Box::new(observer), block))
    }
}
