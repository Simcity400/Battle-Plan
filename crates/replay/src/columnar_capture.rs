//! A capture's columnar recording written by the recorder from the collector's journal
//! ([`crate::columnar_journal`]): each hour applied as it arrives, each block written once it
//! fills or its run ends ([`crate::columnar_blocks`]), and the file revised at each journal
//! segment's end, after which the segment is removed. The capture record counts the segments
//! the file holds, so a writer taking up its partial file after a stop resumes at the next. Each
//! revision is written with the world log's resume state ([`CaptureWriter::world_log_state`]): a
//! writer taking the file up restores the run under way from it, reading no hour again.
//!
//! The file is `columnar.<capture>.h4col.partial` beside the journal while it is written, its
//! resume state `columnar.<capture>.h4col.partial.state` ([`StateFile`]), and
//! `columnar.<capture>.h4col` once finished, the resume state gone.

use crate::{
    columnar_blocks::{BlockBuilder, Finished},
    columnar_file::{Capture, CAPTURE_NAME_BYTES, FileWriter, OpenRun, Resumed, Settings, Stop, capture_name},
    columnar_frames::{Recording, Walker, schema},
    columnar_journal::{self, Record, Tail},
};
use replay_columnar::{
    container::{self, SectionKey, StateEntry, StateFile, state_file},
    envelope::{self, UNIT_ORDINALS},
    leaf::{Reader, Writer as Bytes},
    restart,
    walk::State,
};
use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// A journal segment whose hours the file holds: removed, or, in a smoke build, kept apart for
/// checking the file against.
fn retire(segment: &Path) -> io::Result<()> {
    if cfg!(feature = "keep-journal")
        && let (Some(directory), Some(name)) = (segment.parent(), segment.file_name())
    {
        let kept = directory.join("kept-journal");
        fs::create_dir_all(&kept)?;
        return fs::rename(segment, kept.join(name));
    }
    fs::remove_file(segment)
}

/// Retires what remains of capture `capture`'s journal in `directory`, once its finished file
/// holds every hour of it.
pub fn retire_journal(directory: &Path, capture: u32) -> io::Result<()> {
    for (held, _, path) in columnar_journal::segments(directory)? {
        if held == capture {
            retire(&path)?;
        }
    }
    Ok(())
}

/// The schema version of the world log's resume state ([`CaptureWriter::world_log_state`]).
const WORLD_LOG_STATE: u32 = 1;

/// Each table's first hour of its current availability, as each of `restarts`' tables holds it.
fn availability(restarts: &[Vec<u8>], tables: usize) -> io::Result<Vec<Option<i32>>> {
    let mut available = vec![None; tables];
    for (table, bytes) in restarts.iter().enumerate().take(tables) {
        available[table] = restart::acquisition(bytes)?.1.available_from;
    }
    Ok(available)
}

/// What one call to [`CaptureWriter::advance`] got to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    /// It read every record written so far, and the collector writes more.
    Waiting,
    /// It stopped when asked, with records left to read.
    Paused,
    /// The capture ended and its file is finished.
    Finished,
}

pub struct CaptureWriter {
    directory: PathBuf,
    capture: u32,
    settings: Settings,
    partial: PathBuf,
    finished: PathBuf,
    file: Option<FileWriter<File>>,
    record: Option<Capture>,
    builder: BlockBuilder,
    /// The segment read next, and its tail once opened.
    segment: u32,
    tail: Option<Tail>,
    /// Whether it took any record of the open segment, which a stopped file then holds.
    taken: bool,
    done: bool,
}

impl CaptureWriter {
    /// Capture `capture` of the campaign spooled in `directory`, its partial file taken up after
    /// its last revision when there is one.
    pub fn open(directory: &Path, capture: u32, settings: Settings) -> io::Result<Self> {
        let mut name = [0; CAPTURE_NAME_BYTES];
        let partial = directory.join(capture_name(capture, true, &mut name));
        let finished = directory.join(capture_name(capture, false, &mut name));
        let mut writer = Self {
            directory: directory.to_path_buf(),
            capture,
            settings,
            partial,
            finished,
            file: None,
            record: None,
            builder: BlockBuilder::new()?,
            segment: 0,
            tail: None,
            taken: false,
            done: false,
        };
        if writer.partial.exists() && !writer.segment_path(0).exists() {
            writer.resume()?;
        } else {
            // No revision retired the journal's first segment: the journal rebuilds the file.
            if writer.partial.exists() {
                fs::remove_file(&writer.partial)?;
            }
            state_file::remove_at(&StateFile::path_for(&writer.partial))?;
        }
        Ok(writer)
    }

    fn segment_path(&self, segment: u32) -> PathBuf {
        let mut name = [0; columnar_journal::SEGMENT_NAME_BYTES];
        self.directory
            .join(columnar_journal::segment_name(self.capture, segment, &mut name))
    }

    /// Whether the journal segment at `path` ends its capture.
    fn ends_capture(path: &Path) -> io::Result<bool> {
        let mut tail = Tail::open(path)?;
        let mut finished = false;
        tail.records(|record| {
            finished = matches!(record, Record::Finish { .. });
            Ok(true)
        })?;
        Ok(finished)
    }

    /// Takes up the partial file at the newest revision whose resume state verifies beside it:
    /// its capture record, its runs, and the last run's tables as that state holds them.
    fn resume(&mut self) -> io::Result<()> {
        let state = StateFile::open(&StateFile::path_for(&self.partial))?;
        let file = fs::OpenOptions::new().read(true).write(true).open(&self.partial)?;
        let recovered = container::recover(&file, Some(&state))?;
        if recovered.skipped > 0 {
            // Each revision retires the journal segment it took in: the hours of those after the
            // one taken up are gone.
            return Err(invalid(format!(
                "The partial columnar file's last {} revisions name resume state that does not verify beside it, and their journal is retired",
                recovered.skipped
            )));
        }
        file.set_len(recovered.length)?;
        drop(file);
        let recording = Recording::open_path(&self.partial)?;
        let record = recording
            .capture()?
            .ok_or_else(|| invalid("A partial columnar file names no capture"))?;
        // The segment its last revision took in goes once that revision is durable: one still
        // here ending the capture means the file was finished but not yet renamed.
        if let Some(stale) = record.journaled.checked_sub(1).map(|at| self.segment_path(at))
            && stale.exists()
        {
            if Self::ends_capture(&stale)? {
                drop(recording);
                drop(state);
                state_file::remove_at(&StateFile::path_for(&self.partial))?;
                fs::rename(&self.partial, &self.finished)?;
                retire(&stale)?;
                self.done = true;
                return Ok(());
            }
            retire(&stale)?;
        }
        let open = match recovered.state.iter().find(|entry| entry.owner == SectionKey::WORLD_LOG) {
            Some(entry) => {
                entry.expect(WORLD_LOG_STATE)?;
                Some(self.take_up(&recording, &entry.bytes)?)
            }
            None => None,
        };
        let mut last = None;
        for index in (0..recording.run_count()).rev() {
            let run = recording.run(index)?;
            if run.ordinals > 0 {
                last = Some((run.branch, run.first_hour + (run.ordinals - 1) as i32));
                break;
            }
        }
        let resumed = Resumed {
            root: recording.root().clone(),
            length: recording.len(),
            last,
            controls_written: record.controls.len(),
            open,
        };
        drop(recording);
        let tables = schema()?.len();
        let file = fs::OpenOptions::new().read(true).write(true).open(&self.partial)?;
        self.file = Some(FileWriter::resume(file, resumed, tables, self.settings, Some(state), |file, length| {
            file.set_len(length)
        })?);
        self.segment = record.journaled;
        self.record = Some(record);
        Ok(())
    }

    /// The run under way as its resume state `bytes` hold it, the builder taking it up after its
    /// last block ([`Self::world_log_state`]).
    fn take_up(&mut self, recording: &Recording, bytes: &[u8]) -> io::Result<OpenRun> {
        let mut reader = Reader::new(bytes);
        let index = reader.varint_u32()?;
        let ordinals = reader.varint_u32()?;
        let restarted = match reader.u8()? {
            0 => false,
            1 => true,
            _ => return Err(invalid("The world log's resume state is malformed")),
        };
        let run = recording.run(index)?;
        if index + 1 != recording.run_count() || run.ordinals != ordinals || ordinals % UNIT_ORDINALS != 0 {
            return Err(invalid("The world log's resume state names another run than the file's last"));
        }
        let tables = schema()?.len();
        let last = run.pages() - 1;
        let mut state = State::new(schema()?)?;
        let mut walker = Walker::new(recording)?;
        let (strings, previous, available) = if restarted {
            if recording.page(index, last)?.restart.is_none() {
                return Err(invalid("The world log's resume state names a restart its run's last page holds none of"));
            }
            walker.restore(index, last, &mut state)?;
            let previous = recording.restart_tables(index, last)?;
            let available = availability(&previous, tables)?;
            (Vec::new(), previous, available)
        } else {
            let count = reader.count()?;
            if count > bytes.len() {
                return Err(invalid("The world log's resume state counts more strings than it holds"));
            }
            let mut strings = engine_allocation::vec(count)?;
            for _ in 0..count {
                let length = reader.count()?;
                strings.push(engine_allocation::copy_vec(reader.raw(length)?)?.into_boxed_slice());
            }
            if reader.count()? != tables {
                return Err(invalid("The world log's resume state does not hold every table"));
            }
            let mut held = engine_allocation::vec(tables)?;
            for _ in 0..tables {
                let length = reader.count()?;
                let table = engine_allocation::copy_vec(reader.raw(length)?)?;
                let count = reader.count()?;
                if count > bytes.len() {
                    return Err(invalid("The world log's resume state counts more chunks than it holds"));
                }
                let mut chunks = engine_allocation::vec(count)?;
                for _ in 0..count {
                    chunks.push(reader.varint_u32()?);
                }
                held.push((table, chunks));
            }
            walker.resume_from(index, &held, &strings, &mut state)?;
            let restart = recording
                .segments()
                .iter()
                .rev()
                .find(|segment| segment.run == index)
                .and_then(|segment| segment.restart);
            let previous = match restart {
                Some(at) => recording.restart_tables(index, at)?,
                None => Vec::new(),
            };
            let bytes: Vec<Vec<u8>> = engine_allocation::collect_vec(held.into_iter().map(|(table, _)| table))?;
            (strings, previous, availability(&bytes, tables)?)
        };
        reader.finish()?;
        let next_hour = run
            .first_hour
            .checked_add_unsigned(ordinals)
            .ok_or_else(|| invalid("A run's hours pass 2^31"))?;
        self.builder.resume(state, run.branch, next_hour, ordinals, strings, available)?;
        let (chunks, ids) = run.chunk_starts();
        Ok(OpenRun {
            run: index,
            directory: envelope::RunDirectory {
                branch: run.branch,
                epoch: run.epoch,
                first_hour: run.first_hour,
                ordinals,
                chain: run.chain,
                pages: run.page_extents().to_vec(),
            },
            chunks,
            ids,
            previous,
            restarted,
        })
    }

    /// The world log's resume state after the last block written: the run under way, and unless
    /// its last block ended with a restart, which holds them, the run's strings since its last
    /// restart and every table's state with the dictionary chunks it references. None without a
    /// run under way.
    fn world_log_state(&self) -> io::Result<Option<StateEntry>> {
        let Some(run) = self.file.as_ref().and_then(FileWriter::open_run) else {
            return Ok(None);
        };
        let mut out = Bytes::new();
        out.varint(u64::from(run.run))?;
        out.varint(u64::from(run.ordinals))?;
        out.u8(u8::from(run.restarted))?;
        if !run.restarted {
            let strings = self.builder.strings();
            out.varint(strings.len() as u64)?;
            for string in strings {
                out.varint(string.len() as u64)?;
                out.raw(string)?;
            }
            let states = self.builder.restart_states()?;
            out.varint(states.len() as u64)?;
            let mut held = Vec::new();
            for state in &states {
                out.varint(state.bytes.len() as u64)?;
                out.raw(&state.bytes)?;
                // Ids past the run's restarts are the strings since, which the state holds.
                held.clear();
                engine_allocation::reserve(&mut held, state.ids.len())?;
                held.extend(state.ids.iter().copied().filter(|&id| id < run.ids));
                let chunks = crate::columnar_file::chunks_holding(run.chunks, &held)?;
                out.varint(chunks.len() as u64)?;
                for chunk in chunks {
                    out.varint(u64::from(chunk))?;
                }
            }
        }
        Ok(Some(StateEntry {
            owner: SectionKey::WORLD_LOG,
            version: WORLD_LOG_STATE,
            bytes: out.into_bytes(),
        }))
    }

    /// Lets go of the journal segment it reads until its next advance, which takes up where
    /// this one left off.
    pub fn release(&mut self) {
        if let Some(tail) = &mut self.tail {
            tail.release();
        }
    }

    /// Whether its capture has ended and its file is finished.
    pub fn done(&self) -> bool {
        self.done
    }

    /// The finished file's path.
    pub fn finished_path(&self) -> &Path {
        &self.finished
    }

    /// Applies the journal's records written since the last call, until `stop`.
    pub fn advance(&mut self, stop: &dyn Fn() -> bool) -> io::Result<Progress> {
        loop {
            if self.done {
                return Ok(Progress::Finished);
            }
            let path = self.segment_path(self.segment);
            if self.tail.is_none() {
                if !path.exists() {
                    return Ok(Progress::Waiting);
                }
                self.tail = Some(Tail::open(&path)?);
            }
            let mut tail = self.tail.take().expect("opened");
            let mut ended = None;
            let mut paused = false;
            let read = tail.records(|record| {
                let next = self.take(record)?;
                if next.is_some() {
                    ended = next;
                }
                if stop() {
                    paused = true;
                    return Ok(false);
                }
                Ok(true)
            });
            let segment_ended = read?;
            match ended {
                Some(finish) if segment_ended => {
                    self.end_segment(&path, finish)?;
                    if self.done {
                        return Ok(Progress::Finished);
                    }
                }
                _ => {
                    self.tail = Some(tail);
                    return Ok(if paused { Progress::Paused } else { Progress::Waiting });
                }
            }
            if stop() {
                return Ok(Progress::Paused);
            }
        }
    }

    /// One record; at a segment's end, whether the capture finished there (and how).
    fn take(&mut self, record: Record<'_>) -> io::Result<Option<Option<Option<Stop>>>> {
        self.taken = true;
        match record {
            Record::Begin { identity, capture } => {
                if self.file.is_some() {
                    return Err(invalid("A journal begins once"));
                }
                let identity = envelope::Identity::read(identity)?;
                let record = Capture::read(capture)?;
                let file = fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&self.partial)?;
                let state = StateFile::open(&StateFile::path_for(&self.partial))?;
                self.file = Some(FileWriter::create_with(file, identity, schema()?.len(), self.settings, Some(state))?);
                self.record = Some(record);
            }
            Record::Run {
                branch,
                epoch,
                first_hour,
            } => {
                self.end_run()?;
                self.file_mut()?.start_run(branch, epoch, first_hour)?;
                self.builder.start_run(branch, first_hour)?;
            }
            Record::Hour { manifest, sections } => {
                if let Some(block) = self.builder.hour(manifest, sections)? {
                    self.write(&block)?;
                }
            }
            Record::Controls(records) => {
                let capture = self.record.as_mut().ok_or_else(|| invalid("Controls precede the journal's start"))?;
                for record in records {
                    capture.controls.push(record.to_vec());
                }
            }
            Record::Next => return Ok(Some(None)),
            Record::Finish { stop } => return Ok(Some(Some(stop))),
        }
        Ok(None)
    }

    fn file_mut(&mut self) -> io::Result<&mut FileWriter<File>> {
        self.file.as_mut().ok_or_else(|| invalid("A journal's records precede its start"))
    }

    fn write(&mut self, block: &Finished) -> io::Result<()> {
        self.file_mut()?.block(block.block())
    }

    /// Writes the run's last block, short, when one is under way, and ends the run.
    fn end_run(&mut self) -> io::Result<()> {
        if let Some(block) = self.builder.end_run()? {
            self.write(&block)?;
        }
        if let Some(file) = self.file.as_mut() {
            file.finish_run()?;
        }
        Ok(())
    }

    /// The segment at `path` ended: the file holds it once revised, and it goes. With `finish`,
    /// the capture ended there, where writing stopped short of the rows when it did.
    fn end_segment(&mut self, path: &Path, finish: Option<Option<Stop>>) -> io::Result<()> {
        // A segment ends at a block's end or its run's: a short block is the run's last.
        if self.builder.open() {
            self.end_run()?;
        }
        self.segment = self
            .segment
            .checked_add(1)
            .ok_or_else(|| invalid("A journal's segments pass 2^32"))?;
        let runs = self.file.as_ref().map_or(0, FileWriter::published_runs);
        let capture = self.record.as_mut().ok_or_else(|| invalid("A journal holds no capture"))?;
        capture.journaled = self.segment;
        capture.run_sessions = vec![0; runs];
        match finish {
            None => {
                let capture = self.record.clone();
                let state = self.world_log_state()?;
                let state = state.as_ref().map(std::slice::from_ref);
                self.file_mut()?.revise(capture.as_ref(), state, |file| file.sync_data())?;
            }
            Some(stop) => {
                let mut capture = self.record.take().expect("held");
                capture.stop = stop;
                let file = self.file.take().ok_or_else(|| invalid("A journal holds no start"))?;
                let finished = file.finish_with_capture(&capture, |file| file.sync_data())?;
                drop(finished.file);
                fs::rename(&self.partial, &self.finished)?;
                self.done = true;
            }
        }
        self.tail = None;
        self.taken = false;
        retire(&path)
    }

    /// Ends the capture after what its journal holds, its collector having stopped without
    /// finishing it: the file records that its hours stop after its last.
    pub fn finish_stopped(mut self, reason: &str) -> io::Result<Option<PathBuf>> {
        if self.done {
            return Ok(Some(self.finished));
        }
        let Some(_) = self.file else {
            return Ok(None);
        };
        // What the open segment applied stands: its blocks are written whole, its last short.
        self.end_run()?;
        let runs = self.file.as_ref().map_or(0, FileWriter::published_runs);
        let mut capture = self.record.take().ok_or_else(|| invalid("A journal holds no capture"))?;
        capture.run_sessions = vec![0; runs];
        capture.journaled = self.segment.saturating_add(u32::from(self.taken));
        if capture.stop.is_none() {
            capture.stop = Some(Stop {
                branch: self.last_branch()?.unwrap_or(0),
                last_hour: self.last_hour()?,
                reason: reason.to_owned(),
            });
        }
        let file = self.file.take().expect("held");
        let finished = file.finish_with_capture(&capture, |file| file.sync_data())?;
        drop(finished.file);
        fs::rename(&self.partial, &self.finished)?;
        retire_journal(&self.directory, self.capture)?;
        Ok(Some(self.finished))
    }

    fn last_branch(&self) -> io::Result<Option<u64>> {
        Ok(self.file.as_ref().and_then(|file| file.last()).map(|(branch, _)| branch))
    }

    fn last_hour(&self) -> io::Result<Option<i32>> {
        Ok(self.file.as_ref().and_then(|file| file.last()).map(|(_, hour)| hour))
    }
}
