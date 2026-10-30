//! A columnar recording file (`H4COLUMN` version 3, [`replay_columnar::envelope`]) written run
//! by run. Each hour's manifest and sections go to their streams' units. A full unit position
//! closes with its index page, and every [`Settings::restart_units`] positions also with a
//! restart: the run's strings first used since its previous restart, then each table's state.
//! The file ends with its root and footer.

use replay_columnar::{
    envelope::{self, Extent, IndexPage, RestartDirectory, RestartTable, RunDirectory},
    framing,
    leaf::{Reader, Writer as Bytes},
    manifest::Status,
    state::{Bits, Column, Table},
};
use std::io::{self, Seek, Write};
use zstd::zstd_safe::{CCtx, CParameter};

#[cfg(test)]
mod tests;

/// A campaign's finished columnar recording in its spool directory, written beside the rows.
pub const CAMPAIGN_FILE: &str = "columnar.h4col";

/// Compression and restart cadence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub level: i32,
    /// Restart, page, directory and root extents' zstd window log; units take theirs from
    /// `budget`.
    pub window_log: u32,
    /// A walker's decode memory across the streams of one unit.
    pub budget: usize,
    /// Unit positions from a run's start or restart to its next restart.
    pub restart_units: u32,
}

impl Settings {
    /// The recorder's: a restart every three units, as preparation's segments are planned.
    pub const RECORDING: Self = Self {
        level: 3,
        window_log: 24,
        budget: 30_000_000,
        restart_units: 3,
    };
}

/// One hour of the run under way.
pub struct Hour<'a> {
    pub hour: i32,
    pub manifest: &'a [u8],
    /// Each written section with its table, in table order.
    pub sections: &'a [(usize, &'a [u8])],
    pub statuses: &'a [Status],
    /// The read bits, one per table.
    pub read: &'a [u64],
}

/// What an hour left of its unit position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    Open,
    /// The position is full and must close before the next hour, with a restart when due.
    Full { restart: bool },
}

/// One table's restart.
pub struct RestartState {
    pub bytes: Vec<u8>,
    /// The run's dictionary ids the state references.
    pub ids: Vec<u32>,
}

/// The finished file, and what its writing measured.
pub struct Finished<W> {
    pub file: W,
    /// Per closed position, the decode memory its unit windows need.
    pub walker_memory: Vec<usize>,
    /// Raw bytes of every page, directory and the root.
    pub index_bytes: u64,
}

/// The capture a file's runs belong to, which the root's control records hold: enough to
/// pair the file with exactly the row recording of the same capture and the hours of it the
/// file covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    pub campaign_id: Vec<u8>,
    /// The content identity, as the row recording encodes it.
    pub content: Vec<u8>,
    pub capture_mode: u8,
    /// The row recording's sessions the file's runs belong to, in recording order, each as
    /// the row recording encodes it.
    pub sessions: Vec<Vec<u8>>,
    /// Each run's index in `sessions`.
    pub run_sessions: Vec<u32>,
    /// Where writing stopped short of the rows, when it did.
    pub stop: Option<Stop>,
}

/// The columnar writer stopped while the rows went on: the hours after `last_hour` of
/// `branch` are not in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stop {
    pub branch: u64,
    /// The last hour the file holds on `branch`; none when it holds none of it.
    pub last_hour: Option<i32>,
    pub reason: String,
}

impl Capture {
    const VERSION: u64 = 1;

    pub fn write(&self) -> io::Result<Vec<u8>> {
        let mut out = Bytes::new();
        out.varint(Self::VERSION)?;
        for bytes in [&self.campaign_id, &self.content] {
            out.varint(bytes.len() as u64)?;
            out.raw(bytes)?;
        }
        out.varint(u64::from(self.capture_mode))?;
        out.varint(self.sessions.len() as u64)?;
        for session in &self.sessions {
            out.varint(session.len() as u64)?;
            out.raw(session)?;
        }
        out.varint(self.run_sessions.len() as u64)?;
        for &session in &self.run_sessions {
            out.varint(u64::from(session))?;
        }
        match &self.stop {
            None => out.varint(0)?,
            Some(stop) => {
                out.varint(1)?;
                out.varint(stop.branch)?;
                out.varint(stop.last_hour.map_or(0, |hour| (hour as u32 as u64) + 1))?;
                out.varint(stop.reason.len() as u64)?;
                out.raw(stop.reason.as_bytes())?;
            }
        }
        Ok(out.into_bytes())
    }

    pub fn read(bytes: &[u8]) -> io::Result<Self> {
        let mut reader = Reader::new(bytes);
        if reader.varint()? != Self::VERSION {
            return Err(invalid("Unknown capture record version"));
        }
        let owned = |reader: &mut Reader<'_>| -> io::Result<Vec<u8>> {
            let length = usize::try_from(reader.varint()?)
                .map_err(|_| invalid("A capture field passes the address space"))?;
            engine_allocation::copy_vec(reader.raw(length)?)
        };
        let campaign_id = owned(&mut reader)?;
        let content = owned(&mut reader)?;
        let capture_mode = u8::try_from(reader.varint()?)
            .map_err(|_| invalid("A capture mode passes a byte"))?;
        let count = |reader: &mut Reader<'_>| -> io::Result<usize> {
            let count = usize::try_from(reader.varint()?)
                .map_err(|_| invalid("A capture count passes the address space"))?;
            if count > bytes.len() {
                return Err(invalid("A capture count passes its record"));
            }
            Ok(count)
        };
        let sessions_count = count(&mut reader)?;
        let mut sessions = engine_allocation::vec(sessions_count)?;
        for _ in 0..sessions_count {
            sessions.push(owned(&mut reader)?);
        }
        let runs_count = count(&mut reader)?;
        let mut run_sessions = engine_allocation::vec(runs_count)?;
        for _ in 0..runs_count {
            let session = u32::try_from(reader.varint()?)
                .map_err(|_| invalid("A run's session passes 2^32"))?;
            if session as usize >= sessions.len() {
                return Err(invalid("A run names no session of the capture"));
            }
            run_sessions.push(session);
        }
        let stop = match reader.varint()? {
            0 => None,
            1 => {
                let branch = reader.varint()?;
                let last_hour = match reader.varint()? {
                    0 => None,
                    hour => Some(
                        u32::try_from(hour - 1)
                            .map_err(|_| invalid("A stop's hour passes 2^32"))? as i32,
                    ),
                };
                let reason = String::from_utf8(owned(&mut reader)?)
                    .map_err(|_| invalid("A stop's reason is not UTF-8"))?;
                Some(Stop {
                    branch,
                    last_hour,
                    reason,
                })
            }
            _ => return Err(invalid("Unknown capture stop marker")),
        };
        reader.finish()?;
        Ok(Self {
            campaign_id,
            content,
            capture_mode,
            sessions,
            run_sessions,
            stop,
        })
    }
}

/// What a closed position wrote.
#[derive(Default)]
pub struct Closed {
    /// Every nonempty stream's unit: the manifests' first, then the tables' in stream order.
    pub units: Vec<Extent>,
    pub restart: Option<RestartDirectory>,
    /// The index page's raw bytes.
    pub page_raw: u64,
}

/// The run being written: its ordinals so far, the chain after the last, and the unit
/// position under way.
struct Run {
    run: u32,
    branch: u64,
    epoch: u8,
    first_hour: i32,
    last_hour: i32,
    ordinal: u32,
    chain: [u8; 32],
    /// The position's first and last hour; none before its first ordinal.
    position: Option<(i32, i32)>,
    sectioned: Vec<u64>,
    read: Vec<u64>,
    status_changed: Vec<u64>,
    /// Each table's status at the run's last ordinal; none before its first.
    statuses: Option<Vec<Status>>,
    /// Each restart's dictionary chunk's first id, by restart ordinal.
    chunks: Vec<u32>,
    /// The run's dictionary ids the restarts so far hold.
    ids: u32,
    pages: Vec<Extent>,
    /// The full position's restart is due.
    full: Option<bool>,
}

pub struct FileWriter<W: Write + Seek> {
    file: W,
    settings: Settings,
    compressor: CCtx<'static>,
    /// Taken by the root.
    identity: Option<envelope::Identity>,
    identity_bytes: Vec<u8>,
    /// Each table's column count, as manifests are written with.
    columns: Vec<usize>,
    run: Option<Run>,
    runs: Vec<Extent>,
    manifests: Bytes,
    units: Vec<Bytes>,
    /// Per closed position, the decode memory its unit windows need.
    walker_memory: Vec<usize>,
    /// Raw bytes of every page, directory and the root.
    index_bytes: u64,
    /// A write failed partway: what the file holds no longer matches what was given.
    poisoned: bool,
}

impl<W: Write + Seek> FileWriter<W> {
    /// Starts the file at its header.
    pub fn create(
        mut file: W,
        identity: envelope::Identity,
        columns: Vec<usize>,
        settings: Settings,
    ) -> io::Result<Self> {
        if settings.restart_units == 0 {
            return Err(invalid("Restarts need at least one unit"));
        }
        let mut compressor = CCtx::try_create().ok_or_else(engine_allocation::oom)?;
        compressor
            .set_parameter(CParameter::CompressionLevel(settings.level))
            .map_err(zstd_error)?;
        file.write_all(envelope::MAGIC)?;
        file.write_all(&envelope::VERSION.to_le_bytes())?;
        let identity_bytes = identity.chain_bytes()?;
        let mut units = Vec::new();
        engine_allocation::reserve(&mut units, columns.len())?;
        units.resize_with(columns.len(), Bytes::new);
        Ok(Self {
            file,
            settings,
            compressor,
            identity: Some(identity),
            identity_bytes,
            columns,
            run: None,
            runs: Vec::new(),
            manifests: Bytes::new(),
            units,
            walker_memory: Vec::new(),
            index_bytes: 0,
            poisoned: false,
        })
    }

    fn usable(&self) -> io::Result<()> {
        if self.poisoned {
            Err(invalid("A failed write ended this columnar file"))
        } else {
            Ok(())
        }
    }

    /// `result`, ending the file when it failed partway.
    fn poison<T>(&mut self, result: io::Result<T>) -> io::Result<T> {
        self.poisoned |= result.is_err();
        result
    }

    /// Each table's column count, as the file's manifests are written with.
    pub fn columns(&self) -> &[usize] {
        &self.columns
    }

    /// Whether a run is under way.
    pub fn running(&self) -> bool {
        self.run.is_some()
    }

    /// Starts a run of `branch` at `first_hour`, finishing the run under way first. A run
    /// starts from empty tables and its own dictionary.
    pub fn start_run(&mut self, branch: u64, epoch: u8, first_hour: i32) -> io::Result<()> {
        self.usable()?;
        self.finish_run()?;
        let run = u32::try_from(self.runs.len()).map_err(|_| invalid("Runs pass 2^32"))?;
        let words = self.columns.len().div_ceil(64);
        self.run = Some(Run {
            run,
            branch,
            epoch,
            first_hour,
            last_hour: first_hour,
            ordinal: 0,
            chain: envelope::chain_start(&self.identity_bytes, run),
            position: None,
            sectioned: engine_allocation::filled(words, 0_u64)?,
            read: engine_allocation::filled(words, 0_u64)?,
            status_changed: engine_allocation::filled(words, 0_u64)?,
            statuses: None,
            chunks: Vec::new(),
            ids: 0,
            pages: Vec::new(),
            full: None,
        });
        Ok(())
    }

    /// Appends the next hour of the run under way. An hour refused before anything is
    /// written leaves the file usable.
    pub fn hour(&mut self, hour: Hour<'_>) -> io::Result<Position> {
        self.usable()?;
        let run = self
            .run
            .as_ref()
            .ok_or_else(|| invalid("An hour needs a run under way"))?;
        if run.full.is_some() {
            return Err(invalid("A full unit position closes before the next hour"));
        }
        let expected = if run.ordinal == 0 {
            Some(run.first_hour)
        } else {
            run.last_hour.checked_add(1)
        };
        if expected != Some(hour.hour) {
            return Err(invalid("A run's hours are continuous from its first"));
        }
        if run.ordinal == u32::MAX {
            return Err(invalid("A run's ordinals pass 2^32"));
        }
        if hour.statuses.len() != self.columns.len() || hour.read.len() != run.read.len() {
            return Err(invalid("An hour covers every table"));
        }
        if !hour
            .sections
            .windows(2)
            .all(|pair| pair[0].0 < pair[1].0)
            || hour
                .sections
                .last()
                .is_some_and(|&(table, _)| table >= self.columns.len())
        {
            return Err(invalid("An hour's sections name its tables in order"));
        }
        let result = self.append(hour);
        self.poison(result)
    }

    fn append(&mut self, hour: Hour<'_>) -> io::Result<Position> {
        let run = self.run.as_mut().expect("a run under way");
        for &(table, section) in hour.sections {
            framing::put_section(&mut self.units[table], section)?;
            set(&mut run.sectioned, table);
        }
        framing::put_manifest(&mut self.manifests, hour.manifest)?;
        for (table, status) in hour.statuses.iter().enumerate() {
            if run
                .statuses
                .as_ref()
                .is_none_or(|before| before[table] != *status)
            {
                set(&mut run.status_changed, table);
            }
        }
        match &mut run.statuses {
            Some(held) => held.copy_from_slice(hour.statuses),
            None => run.statuses = Some(engine_allocation::copy_vec(hour.statuses)?),
        }
        for (word, bits) in run.read.iter_mut().zip(hour.read) {
            *word |= bits;
        }
        run.chain = envelope::chain_hour(
            &run.chain,
            hour.manifest,
            hour.sections.iter().map(|&(_, section)| section),
        );
        run.position = Some(
            run.position
                .map_or((hour.hour, hour.hour), |(first, _)| (first, hour.hour)),
        );
        run.last_hour = hour.hour;
        run.ordinal += 1;
        if run.ordinal % envelope::UNIT_ORDINALS != 0 {
            return Ok(Position::Open);
        }
        let units = run.ordinal / envelope::UNIT_ORDINALS;
        let restart = units % self.settings.restart_units == 0;
        run.full = Some(restart);
        Ok(Position::Full { restart })
    }

    /// Closes the full position the last hour filled, when no restart is due.
    pub fn close_position(&mut self) -> io::Result<Closed> {
        self.usable()?;
        if self.due()? {
            return Err(invalid("A due restart was not given"));
        }
        let result = self.close::<&[u8]>(None);
        self.poison(result)
    }

    /// Closes the full position the last hour filled with its due restart: the run's strings
    /// first used since its previous restart, in id order, then every table's state, in
    /// table order.
    pub fn close_with_restart<S: AsRef<[u8]>>(
        &mut self,
        strings: &[S],
        tables: &[RestartState],
    ) -> io::Result<Closed> {
        self.usable()?;
        if !self.due()? {
            return Err(invalid("A restart is due only every restart_units positions"));
        }
        if tables.len() != self.columns.len() {
            return Err(invalid("A restart holds every table"));
        }
        let result = self.close(Some((strings, tables)));
        self.poison(result)
    }

    /// Whether the full position the last hour filled is due a restart.
    fn due(&self) -> io::Result<bool> {
        self.run
            .as_ref()
            .and_then(|run| run.full)
            .ok_or_else(|| invalid("Only a full unit position closes early"))
    }

    /// Closes the run's last position, short or not, and writes the run's directory. A full
    /// last position whose restart was due closes without one: a restart only lets a walk
    /// start there, and the run ends anyway.
    pub fn finish_run(&mut self) -> io::Result<Closed> {
        self.usable()?;
        if self.run.is_none() {
            return Ok(Closed::default());
        }
        let result = self.end_run();
        self.poison(result)
    }

    fn end_run(&mut self) -> io::Result<Closed> {
        let closed = self.close::<&[u8]>(None)?;
        let run = self.run.take().expect("a run under way");
        let directory = RunDirectory {
            branch: run.branch,
            epoch: run.epoch,
            first_hour: run.first_hour,
            ordinals: run.ordinal,
            chain: run.chain,
            pages: run.pages,
        }
        .write(run.run)?;
        self.index_bytes += directory.len() as u64;
        let directory = self.extent(
            envelope::Kind::RunDirectory,
            run.run,
            0,
            0,
            0,
            &directory,
            self.settings.window_log,
        )?;
        engine_allocation::reserve(&mut self.runs, 1)?;
        self.runs.push(directory);
        Ok(closed)
    }

    /// Finishes the run under way, then appends the root and the footer naming it. The
    /// caller makes the file durable.
    pub fn finish(self) -> io::Result<Finished<W>> {
        self.finish_with(None)
    }

    /// [`Self::finish`], the root's control records holding `capture`, which names a session
    /// for every run.
    pub fn finish_with_capture(self, capture: &Capture) -> io::Result<Finished<W>> {
        self.finish_with(Some(capture))
    }

    fn finish_with(mut self, capture: Option<&Capture>) -> io::Result<Finished<W>> {
        self.finish_run()?;
        let mut controls = Vec::new();
        if let Some(capture) = capture {
            if capture.run_sessions.len() != self.runs.len() {
                return Err(invalid("A capture names a session for every run"));
            }
            let bytes = capture.write()?;
            self.index_bytes += bytes.len() as u64;
            let window_log = self.settings.window_log;
            engine_allocation::reserve(&mut controls, 1)?;
            controls.push(self.extent(envelope::Kind::Controls, 0, 0, 0, 0, &bytes, window_log)?);
        }
        let identity = self
            .identity
            .take()
            .ok_or_else(|| invalid("A file has one root"))?;
        let root = envelope::Root {
            identity,
            revision: 1,
            runs: std::mem::take(&mut self.runs),
            controls,
        }
        .write()?;
        self.index_bytes += root.len() as u64;
        let root = self.extent(
            envelope::Kind::Root,
            0,
            0,
            0,
            0,
            &root,
            self.settings.window_log,
        )?;
        self.file
            .write_all(&envelope::footer(root.offset, root.stored, &root.digest))?;
        self.file.flush()?;
        Ok(Finished {
            file: self.file,
            walker_memory: self.walker_memory,
            index_bytes: self.index_bytes,
        })
    }

    /// Closes the position under way: every stream's unit, the restart ending it, and its
    /// page. Nothing happens before the position's first hour.
    fn close<S: AsRef<[u8]>>(
        &mut self,
        restart: Option<(&[S], &[RestartState])>,
    ) -> io::Result<Closed> {
        let run = self
            .run
            .as_mut()
            .ok_or_else(|| invalid("A position needs a run under way"))?;
        run.full = None;
        let Some((first_hour, last_hour)) = run.position.take() else {
            return Ok(Closed::default());
        };
        let (index, ordinal) = (run.run, run.ordinal);
        let first = (ordinal - 1) / envelope::UNIT_ORDINALS * envelope::UNIT_ORDINALS;
        let hours = ordinal - first;
        let units = self.units(index, first, hours)?;
        let restart = match restart {
            Some((strings, tables)) => Some(self.restart(strings, tables, last_hour)?),
            None => None,
        };
        let run = self.run.as_mut().expect("a run under way");
        let take = |bits: &mut Vec<u64>| -> io::Result<Bits> {
            let zeroed = engine_allocation::filled(bits.len(), 0_u64)?;
            Bits::from_words(std::mem::replace(bits, zeroed))
        };
        let page = IndexPage {
            run: index,
            first,
            hours,
            first_hour,
            last_hour,
            units,
            restart,
            sectioned: take(&mut run.sectioned)?,
            read: take(&mut run.read)?,
            status_changed: take(&mut run.status_changed)?,
            chain: run.chain,
        };
        let written = page.write();
        let IndexPage { units, restart, .. } = page;
        let page = written?;
        let page_raw = page.len() as u64;
        self.index_bytes += page_raw;
        let page = self.extent(
            envelope::Kind::IndexPage,
            index,
            0,
            first,
            hours,
            &page,
            self.settings.window_log,
        )?;
        let run = self.run.as_mut().expect("a run under way");
        engine_allocation::reserve(&mut run.pages, 1)?;
        run.pages.push(page);
        Ok(Closed {
            units,
            restart,
            page_raw,
        })
    }

    /// Every nonempty stream's unit of the position: the manifests' first, then each
    /// table's in stream order.
    fn units(&mut self, run: u32, first: u32, hours: u32) -> io::Result<Vec<Extent>> {
        let mut written = Vec::new();
        engine_allocation::reserve(&mut written, self.units.len() + 1)?;
        let manifests = std::mem::take(&mut self.manifests).into_bytes();
        if !manifests.is_empty() {
            written.push((framing::MANIFESTS, manifests));
        }
        for (table, unit) in self.units.iter_mut().enumerate() {
            let raw = std::mem::take(unit).into_bytes();
            if !raw.is_empty() {
                written.push((framing::table_stream(table), raw));
            }
        }
        let raws = engine_allocation::collect_vec(written.iter().map(|(_, raw)| raw.len()))?;
        let logs = unit_windows(&raws, self.settings.budget)?;
        engine_allocation::reserve(&mut self.walker_memory, 1)?;
        self.walker_memory.push(
            raws.iter()
                .zip(&logs)
                .map(|(&raw, &log)| raw.min(1 << log))
                .sum(),
        );
        let mut units = Vec::new();
        engine_allocation::reserve(&mut units, written.len())?;
        for ((stream, raw), log) in written.into_iter().zip(logs) {
            units.push(self.extent(envelope::Kind::Unit, run, stream, first, hours, &raw, log)?);
        }
        Ok(units)
    }

    /// The restart ending the position after its last ordinal.
    fn restart<S: AsRef<[u8]>>(
        &mut self,
        strings: &[S],
        states: &[RestartState],
        hour: i32,
    ) -> io::Result<RestartDirectory> {
        if states.len() != self.columns.len() {
            return Err(invalid("A restart holds every table"));
        }
        let run = self.run.as_ref().expect("a run under way");
        let (index, after) = (run.run, run.ordinal - 1);
        let first_id = run.ids;
        let count = u32::try_from(strings.len())
            .map_err(|_| invalid("A run's dictionary passes 2^32 ids"))?;
        let ids = first_id
            .checked_add(count)
            .ok_or_else(|| invalid("A run's dictionary passes 2^32 ids"))?;
        let mut chunk = Bytes::new();
        chunk.varint(u64::from(count))?;
        for string in strings {
            let string = string.as_ref();
            chunk.varint(string.len() as u64)?;
            chunk.raw(string)?;
        }
        let window_log = self.settings.window_log;
        let chunk = self.extent(
            envelope::Kind::DictionaryChunk,
            index,
            0,
            after,
            0,
            chunk.bytes(),
            window_log,
        )?;
        let run = self.run.as_mut().expect("a run under way");
        let ordinal = u32::try_from(run.chunks.len()).map_err(|_| invalid("Restarts pass 2^32"))?;
        engine_allocation::reserve(&mut run.chunks, 1)?;
        run.chunks.push(first_id);
        run.ids = ids;
        let starts = engine_allocation::copy_vec(&run.chunks)?;
        let mut tables = Vec::new();
        engine_allocation::reserve(&mut tables, states.len())?;
        for (table, state) in states.iter().enumerate() {
            if let Some(&id) = state.ids.iter().find(|&&id| id >= ids) {
                let _ = (table, id);
                return Err(invalid("A table's restart references a dictionary id past the run's"));
            }
            let stream = u32::try_from(table).map_err(|_| invalid("Tables pass 2^32"))?;
            let extent = self.extent(
                envelope::Kind::RestartTable,
                index,
                stream,
                after,
                0,
                &state.bytes,
                window_log,
            )?;
            tables.push(RestartTable {
                table: stream,
                extent,
                chunks: chunks_holding(&starts, &state.ids)?,
            });
        }
        Ok(RestartDirectory {
            after,
            hour,
            ordinal,
            chunk,
            ids: (first_id, count),
            tables,
        })
    }

    /// One compressed extent with a `window_log` window, described by its envelope.
    #[allow(clippy::too_many_arguments)]
    fn extent(
        &mut self,
        kind: envelope::Kind,
        run: u32,
        stream: u32,
        first: u32,
        hours: u32,
        raw: &[u8],
        window_log: u32,
    ) -> io::Result<Extent> {
        self.compressor
            .set_parameter(CParameter::WindowLog(window_log))
            .map_err(zstd_error)?;
        let mut stored = engine_allocation::vec(zstd::zstd_safe::compress_bound(raw.len()))?;
        self.compressor
            .compress2(&mut stored, raw)
            .map_err(zstd_error)?;
        let offset = self.file.stream_position()?;
        self.file.write_all(&stored)?;
        Ok(Extent {
            kind,
            run,
            stream,
            first,
            hours,
            offset,
            stored: stored.len() as u64,
            raw: raw.len() as u64,
            digest: envelope::digest(&stored),
        })
    }
}

/// The dictionary ids a table's state references: its refusal, and its recorded text cells
/// at live slots.
pub fn referenced_ids(table: &Table, refusal: Option<u32>) -> io::Result<Vec<u32>> {
    let mut ids = Vec::new();
    if let Some(id) = refusal {
        engine_allocation::reserve(&mut ids, 1)?;
        ids.push(id);
    }
    for column in 0..table.kinds().count() {
        if let Some(Column::Bytes(texts)) = table.column(column) {
            for slot in table.live().ones() {
                if table.recorded(column, slot)
                    && let Some(&id) = texts.get(slot as usize)
                {
                    engine_allocation::reserve(&mut ids, 1)?;
                    ids.push(id);
                }
            }
        }
    }
    Ok(ids)
}

/// The restart ordinals whose dictionary chunks hold `ids`, ascending. `starts` holds each
/// chunk's first id, ascending from 0; an empty chunk starts where the next does, so the
/// last chunk starting at or before an id holds it.
fn chunks_holding(starts: &[u32], ids: &[u32]) -> io::Result<Vec<u32>> {
    let mut used = engine_allocation::filled(starts.len(), false)?;
    for &id in ids {
        let chunk = starts.partition_point(|&start| start <= id);
        let held = chunk
            .checked_sub(1)
            .ok_or_else(|| invalid("A dictionary id precedes the run's first chunk"))?;
        used[held] = true;
    }
    engine_allocation::collect_vec(
        (0..)
            .zip(used)
            .filter_map(|(chunk, used)| used.then_some(chunk)),
    )
}

/// Each stream's zstd window log for one unit: the largest power of two within its share
/// of a walker's decode-memory budget by raw size, then one step larger for the largest
/// streams while the unit stays within the budget. A window never grows past its stream.
pub fn unit_windows(raws: &[usize], budget: usize) -> io::Result<Vec<u32>> {
    const SMALLEST: u32 = 15;
    const LARGEST: u32 = 24;
    let total: usize = raws.iter().sum();
    let mut logs = engine_allocation::collect_vec(raws.iter().map(|&raw| {
        let share = (budget as u128 * raw as u128 / total.max(1) as u128) as usize;
        (usize::BITS - 1 - share.max(1).leading_zeros()).clamp(SMALLEST, LARGEST)
    }))?;
    let held = |raw: usize, log: u32| raw.min(1 << log);
    let mut used: usize = raws
        .iter()
        .zip(&logs)
        .map(|(&raw, &log)| held(raw, log))
        .sum();
    let mut order = engine_allocation::collect_vec(0..raws.len())?;
    order.sort_unstable_by_key(|&stream| std::cmp::Reverse(raws[stream]));
    loop {
        let mut grew = false;
        for &stream in &order {
            let (raw, log) = (raws[stream], logs[stream]);
            if log < LARGEST && (1 << log) < raw {
                let added = held(raw, log + 1) - held(raw, log);
                if used + added <= budget {
                    logs[stream] += 1;
                    used += added;
                    grew = true;
                }
            }
        }
        if !grew {
            return Ok(logs);
        }
    }
}

fn set(bits: &mut [u64], table: usize) {
    bits[table / 64] |= 1 << (table % 64);
}

fn zstd_error(code: usize) -> io::Error {
    invalid(zstd::zstd_safe::get_error_name(code))
}

/// Kind-only while a capture runs, whatever diagnostic scope encloses it: the live writer
/// must not allocate to fail. Offline writers keep the message.
fn invalid(message: &'static str) -> io::Error {
    if engine_allocation::capture_active() {
        io::Error::from(io::ErrorKind::InvalidData)
    } else {
        io::Error::new(io::ErrorKind::InvalidData, message)
    }
}
