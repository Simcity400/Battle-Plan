//! A columnar recording's runs as final blocks, made from their hours: each hour's manifest and
//! sections, as the collector's frame writer makes them ([`crate::native_frame`]), applied to a
//! walker's tables and written once per block ([`replay_columnar::block`],
//! [`replay_columnar::records`]), every [`RESTART_BLOCKS`]th block of a run ending with a
//! restart of every table ([`replay_columnar::restart`]). The blocks go to a
//! [`crate::columnar_file::FileWriter`] in order.
//!
//! A table's block is written against the table as its first hour in the block finds it, held
//! for the block alone and only for tables the block changes.

use crate::{
    columnar_file::{Block, RestartState, referenced_ids},
    columnar_frames::{keeps_last, schema},
};
use replay_columnar::{
    block::TableEncoder,
    envelope::{RESTART_BLOCKS, UNIT_ORDINALS},
    manifest::{Manifest, Status},
    records::RecordsEncoder,
    restart::{self, Acquisition, Last},
    state::Kind,
    walk::State,
};
use std::io;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// `state` keeping the last complete reads its restarts carry.
fn kept(mut state: State) -> io::Result<State> {
    state.keep_last(&keeps_last())?;
    Ok(state)
}

/// The compressor's estimate of `bytes`' stored size, for an encoder's choices.
pub fn estimate(bytes: &[u8]) -> usize {
    if bytes.is_empty() {
        return 0;
    }
    zstd::bulk::compress(bytes, 3).map_or(usize::MAX, |stored| stored.len())
}

/// A block made, raw, as [`crate::columnar_file::FileWriter::block`] takes it.
pub struct Finished {
    pub first_hour: i32,
    pub last_hour: i32,
    pub hours: u32,
    pub records: Vec<u8>,
    pub tables: Vec<(u32, Vec<u8>)>,
    pub sectioned: Vec<u64>,
    pub read: Vec<u64>,
    pub status_changed: Vec<u64>,
    /// The run's strings first used since its previous restart, and every table's state.
    pub restart: Option<(Vec<Box<[u8]>>, Vec<RestartState>)>,
}

impl Finished {
    pub fn block(&self) -> Block<'_> {
        Block {
            first_hour: self.first_hour,
            last_hour: self.last_hour,
            hours: self.hours,
            records: &self.records,
            tables: &self.tables,
            sectioned: &self.sectioned,
            read: &self.read,
            status_changed: &self.status_changed,
            restart: self
                .restart
                .as_ref()
                .map(|(strings, states)| (strings.as_slice(), states.as_slice())),
        }
    }
}

/// The block under way.
struct Open {
    first_hour: i32,
    last_hour: i32,
    hours: u32,
    records: RecordsEncoder,
    tables: Vec<Option<TableEncoder>>,
    sectioned: Vec<u64>,
    read: Vec<u64>,
    status_changed: Vec<u64>,
}

/// The run under way.
struct Run {
    branch: u64,
    next_hour: i32,
    ordinal: u32,
    /// The run's strings first used since its previous restart.
    strings: Vec<Box<[u8]>>,
    statuses: Vec<Status>,
    /// Each table's first hour of its current availability, as its restarts carry it.
    available_from: Vec<Option<i32>>,
}

/// Each table's first hour of its current availability after an hour at `hour` whose statuses
/// are `now`, the hour before's `before`: the hour a table turns available or unavailable, or
/// its first.
pub fn note_availability(available_from: &mut [Option<i32>], before: &[Status], now: &[Status], hour: i32) {
    let available = |status: Status| matches!(status, Status::Complete | Status::Truncated { .. });
    for ((from, &before), &now) in available_from.iter_mut().zip(before).zip(now) {
        if available(now) != available(before) || from.is_none() {
            *from = Some(hour);
        }
    }
}

/// Present hours a table's block holds per group: a walker reads one group's bytes at a time.
const GROUP_HOURS: u32 = 48;

/// Makes runs' hours into blocks.
pub struct BlockBuilder {
    /// Blocks from a run's start or restart to its next restart.
    restart_blocks: u32,
    kinds: Vec<Vec<Kind>>,
    columns: Vec<usize>,
    state: State,
    run: Option<Run>,
    open: Option<Open>,
}

impl BlockBuilder {
    pub fn new() -> io::Result<Self> {
        let kinds = schema()?;
        let columns = kinds.iter().map(Vec::len).collect();
        Ok(Self {
            restart_blocks: RESTART_BLOCKS,
            state: kept(State::new(kinds.clone())?)?,
            kinds,
            columns,
            run: None,
            open: None,
        })
    }

    /// A builder restarting every `blocks` blocks, for measuring other cadences.
    pub fn with_restart_blocks(mut self, blocks: u32) -> Self {
        self.restart_blocks = blocks.max(1);
        self
    }

    /// The walker's tables after the last hour.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// Whether a block is under way.
    pub fn open(&self) -> bool {
        self.open.is_some()
    }

    /// Takes up a run of `branch` after `ordinals` of its hours, a whole number of blocks, from
    /// `state`, the tables after its last hour, `strings`, the run's strings since its last
    /// restart, and `available_from`, each table's first hour of its current availability.
    pub fn resume(
        &mut self,
        state: State,
        branch: u64,
        next_hour: i32,
        ordinals: u32,
        strings: Vec<Box<[u8]>>,
        available_from: Vec<Option<i32>>,
    ) -> io::Result<()> {
        if available_from.len() != self.kinds.len() {
            return Err(invalid("A run is taken up with every table's availability"));
        }
        if ordinals % UNIT_ORDINALS != 0 {
            return Err(invalid("A run is taken up only after a whole block"));
        }
        let statuses = (0..self.kinds.len())
            .map(|table| state.status(table).unwrap_or(Status::NotRecorded))
            .collect();
        self.state = kept(state)?;
        self.run = Some(Run {
            branch,
            next_hour,
            ordinal: ordinals,
            strings,
            statuses,
            available_from,
        });
        self.open = None;
        Ok(())
    }

    /// Starts a run of `branch` at `first_hour` from empty tables: the run under way must have
    /// ended ([`Self::end_run`]).
    pub fn start_run(&mut self, branch: u64, first_hour: i32) -> io::Result<()> {
        if self.open.is_some() {
            return Err(invalid("A run starts once the one before it ended"));
        }
        self.state = kept(State::new(self.kinds.clone())?)?;
        self.run = Some(Run {
            branch,
            next_hour: first_hour,
            ordinal: 0,
            strings: Vec::new(),
            statuses: vec![Status::NotRecorded; self.kinds.len()],
            available_from: vec![None; self.kinds.len()],
        });
        Ok(())
    }

    /// The run's next hour: its manifest's bytes and its sections' in the manifest's order,
    /// together. The block it fills, when it fills one.
    pub fn hour(&mut self, manifest: &[u8], sections: &[u8]) -> io::Result<Option<Finished>> {
        let manifest = Manifest::read(manifest, &self.columns)?;
        let run = self.run.as_mut().ok_or_else(|| invalid("An hour needs a run under way"))?;
        if manifest.branch != run.branch || manifest.hour != run.next_hour {
            return Err(invalid("A run's hours are continuous from its first, on its branch"));
        }
        let mut spans = Vec::with_capacity(manifest.sections.len());
        let mut at = 0usize;
        for section in &manifest.sections {
            let length = usize::try_from(section.bytes)
                .map_err(|_| invalid("A section passes the address space"))?;
            let end = at
                .checked_add(length)
                .filter(|&end| end <= sections.len())
                .ok_or_else(|| invalid("An hour's sections end before its manifest's"))?;
            spans.push(at..end);
            at = end;
        }
        if at != sections.len() {
            return Err(invalid("An hour holds bytes past its manifest's sections"));
        }
        if self.open.is_none() {
            let words = self.kinds.len().div_ceil(64);
            self.open = Some(Open {
                first_hour: manifest.hour,
                last_hour: manifest.hour,
                hours: 0,
                records: RecordsEncoder::new(&self.columns, &run.statuses)?,
                tables: (0..self.kinds.len()).map(|_| None).collect(),
                sectioned: vec![0; words],
                read: vec![0; words],
                status_changed: vec![0; words],
            });
        }
        let open = self.open.as_mut().expect("a block under way");
        for section in &manifest.sections {
            if open.tables[section.table].is_none() {
                let held = self.state.table(section.table).expect("schema table").share()?;
                open.tables[section.table] = Some(TableEncoder::new(&self.kinds[section.table], held, GROUP_HOURS)?);
            }
        }
        self.state
            .apply(&manifest, |index| Ok(&sections[spans[index].clone()]))?;
        open.records.hour(&manifest)?;
        for section in &manifest.sections {
            let table = section.table;
            let truncated = matches!(manifest.statuses[table], Status::Truncated { .. });
            open.tables[table].as_mut().expect("opened").hour(
                self.state.table(table).expect("schema table"),
                self.state.changes(table).expect("schema table"),
                section,
                truncated,
            )?;
            open.sectioned[table / 64] |= 1 << (table % 64);
        }
        note_availability(&mut run.available_from, &run.statuses, &manifest.statuses, manifest.hour);
        for (table, (&now, before)) in manifest.statuses.iter().zip(&mut run.statuses).enumerate() {
            if now != *before {
                open.status_changed[table / 64] |= 1 << (table % 64);
                *before = now;
            }
        }
        for (word, &bits) in open.read.iter_mut().zip(&manifest.read) {
            *word |= bits;
        }
        for append in &manifest.appends {
            run.strings.push(append.to_vec().into_boxed_slice());
        }
        open.last_hour = manifest.hour;
        open.hours += 1;
        run.ordinal += 1;
        run.next_hour = manifest
            .hour
            .checked_add(1)
            .ok_or_else(|| invalid("Hours pass 2^31"))?;
        if run.ordinal % UNIT_ORDINALS == 0 {
            return self.close().map(Some);
        }
        Ok(None)
    }

    /// The block under way, with a restart when due.
    fn close(&mut self) -> io::Result<Finished> {
        let open = self.open.take().ok_or_else(|| invalid("No block is under way"))?;
        let run = self.run.as_mut().ok_or_else(|| invalid("A block needs a run under way"))?;
        let mut tables = Vec::new();
        for (table, encoder) in open.tables.into_iter().enumerate() {
            if let Some(encoder) = encoder {
                tables.push((table as u32, encoder.finish(&mut estimate)?));
            }
        }
        let records = open.records.finish()?;
        let due = open.hours == UNIT_ORDINALS
            && (run.ordinal / UNIT_ORDINALS) % self.restart_blocks == 0;
        let restart = if due {
            let mut states = Vec::with_capacity(self.kinds.len());
            for table in 0..self.kinds.len() {
                let status = self.state.status(table).expect("schema table");
                let refusal = match status {
                    Status::Unavailable { reason } => Some(reason),
                    _ => None,
                };
                let held = self.state.table(table).expect("schema table");
                let last = self.state.last(table);
                let mut ids = referenced_ids(held, refusal)?;
                if let Some(Last::Apart(apart, _)) = last {
                    let more = referenced_ids(apart, None)?;
                    engine_allocation::reserve(&mut ids, more.len())?;
                    ids.extend(more);
                }
                states.push(RestartState {
                    bytes: restart::write_with(
                        held,
                        status,
                        &Acquisition {
                            last_read: self.state.sampled_at(table),
                            available_from: run.available_from[table],
                            refusal,
                        },
                        self.state.truncated_view(table).unwrap_or(&[]),
                        last,
                    )?,
                    ids,
                });
            }
            self.state.freeze_dictionary()?;
            Some((std::mem::take(&mut run.strings), states))
        } else {
            None
        };
        Ok(Finished {
            first_hour: open.first_hour,
            last_hour: open.last_hour,
            hours: open.hours,
            records,
            tables,
            sectioned: open.sectioned,
            read: open.read,
            status_changed: open.status_changed,
            restart,
        })
    }

    /// Ends the run under way: its last block, short, when one is under way.
    pub fn end_run(&mut self) -> io::Result<Option<Finished>> {
        let last = if self.open.is_some() {
            Some(self.close()?)
        } else {
            None
        };
        self.run = None;
        Ok(last)
    }
}
