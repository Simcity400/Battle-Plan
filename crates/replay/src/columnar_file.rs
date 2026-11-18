//! A columnar recording file's world log (format 12, [`replay_columnar::pages`]) and records, in
//! its container ([`replay_columnar::container`]), written run by run as the recorder's store
//! closes them ([`replay_columnar::unit`]): each table part's frames and each restore's chunks the
//! moment they are compressed ([`replay_columnar::unit::PartSink`]), then at each block's end its clock, the strings its
//! hours appended, its tables' summary pieces and its page; every [`replay_columnar::envelope::SPAN_BLOCKS`] blocks a span's
//! directory and index naming them table by table. A revision publishes everything written so far
//! under a new root; the file ends with its root and footer.

use replay_columnar::{
    envelope::{self, Extent},
    leaf::{Reader, Writer as Bytes},
    state::{Column, Table},
};
use std::{
    ffi::{OsStr, OsString},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

/// The longest name [`capture_name`] writes.
pub const CAPTURE_NAME_BYTES: usize = "columnar.4294967295.h4col.partial".len();

/// The name in its campaign's spool directory of the session's `n`th capture's columnar
/// recording, counting from 0: `columnar.<n>.h4col`, `.partial` appended while the recorder
/// writes it. Written into `out`, so it is named without allocating.
pub fn capture_name(n: u32, partial: bool, out: &mut [u8; CAPTURE_NAME_BYTES]) -> &str {
    let mut cursor = io::Cursor::new(&mut out[..]);
    let suffix = if partial { ".partial" } else { "" };
    write!(cursor, "columnar.{n}.h4col{suffix}").expect("the longest name fits");
    let length = cursor.position() as usize;
    std::str::from_utf8(&out[..length]).expect("ASCII")
}

/// The capture a spool directory's file `name` holds ([`capture_name`]), and whether it is
/// still partial.
pub fn capture_of(name: &OsStr) -> Option<(u32, bool)> {
    let name = name.to_str()?.strip_prefix("columnar.")?;
    let (number, partial) = match name.strip_suffix(".h4col.partial") {
        Some(number) => (number, true),
        None => (name.strip_suffix(".h4col")?, false),
    };
    Some((counted(number)?, partial))
}

/// The folder of `replay_dir` holding `campaign`'s published columnar recordings: its id's
/// bytes, ASCII lower-cased as recordings are matched, in hex.
pub fn campaign_folder(replay_dir: &Path, campaign: &[u8]) -> PathBuf {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut name = String::with_capacity(2 * campaign.len());
    for byte in campaign.iter().map(u8::to_ascii_lowercase) {
        name.push(char::from(HEX[usize::from(byte >> 4)]));
        name.push(char::from(HEX[usize::from(byte & 15)]));
    }
    replay_dir.join(name)
}

/// The published name of `session`'s `n`th capture in its campaign's folder, counting from 0:
/// `<session dir>.<n>.h4col`.
pub fn published_name(session: &OsStr, n: u32) -> OsString {
    let mut name = session.to_os_string();
    name.push(format!(".{n}.h4col"));
    name
}

/// The session dir and capture a published name holds ([`published_name`]).
pub fn published_of(name: &OsStr) -> Option<(&str, u32)> {
    let (session, number) = name.to_str()?.strip_suffix(".h4col")?.rsplit_once('.')?;
    Some((session, counted(number)?)).filter(|(session, _)| !session.is_empty())
}

/// Whether the recordings at `a` and `b` are one: of one length and ending in one footer, which
/// names a root whose digest covers everything it names.
pub fn same_recording(a: &Path, b: &Path) -> io::Result<bool> {
    let end = |path: &Path| -> io::Result<(u64, [u8; envelope::FOOTER_BYTES])> {
        let mut file = std::fs::File::open(path)?;
        let length = file.seek(SeekFrom::End(0))?;
        let mut footer = [0; envelope::FOOTER_BYTES];
        let held = length.min(envelope::FOOTER_BYTES as u64) as usize;
        file.seek(SeekFrom::Start(length - held as u64))?;
        file.read_exact(&mut footer[..held])?;
        Ok((length, footer))
    };
    Ok(end(a)? == end(b)?)
}

/// A capture number as names write it: decimal, without leading zeros.
fn counted(number: &str) -> Option<u32> {
    let valid = number.bytes().all(|byte| byte.is_ascii_digit())
        && (number == "0" || !number.starts_with('0'));
    number.parse().ok().filter(|_| valid)
}

/// The compression a writer stores a recording with ([`crate::compression`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub level: i32,
}

impl Settings {
    /// Smaller files, more of the recorder's CPU while it writes: zstd 19.
    pub const HIGH: Self = Self { level: 19 };
    /// Larger files, less of the recorder's CPU while it writes: zstd 7, the last level before
    /// bytes stop falling much for their CPU.
    pub const LOW: Self = Self { level: 7 };
}

/// One table's restart state as the collector's frame writer keeps it.
pub struct RestartState {
    pub bytes: Vec<u8>,
    /// The run's dictionary ids the state references.
    pub ids: Vec<u32>,
}

#[cfg(any(test, feature = "recording"))]
mod writer;
#[cfg(any(test, feature = "recording"))]
pub use writer::*;

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
    /// The row recording's format version.
    pub format: u16,
    /// The journal segments ([`crate::columnar_journal`]) the file holds all of.
    pub journaled: u32,
    /// The capture's branch starts, gaps and control records but its clock witnesses, in
    /// recording order, each as the row recording encodes it (`spool::encode_record`).
    pub controls: Vec<Vec<u8>>,
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
    const VERSION: u64 = 3;

    pub fn write(&self) -> io::Result<Vec<u8>> {
        self.write_with(&self.controls)
    }

    /// The record holding `controls` in place of the capture's own.
    pub(crate) fn write_with(&self, controls: &[Vec<u8>]) -> io::Result<Vec<u8>> {
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
        out.varint(u64::from(self.format))?;
        out.varint(u64::from(self.journaled))?;
        write_controls(&mut out, controls)?;
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
        let format = u16::try_from(reader.varint()?)
            .map_err(|_| invalid("A capture's format passes 2^16"))?;
        let journaled = u32::try_from(reader.varint()?)
            .map_err(|_| invalid("A capture's journal passes 2^32 segments"))?;
        let mut controls = Vec::new();
        read_controls(&mut reader, bytes.len(), &mut controls)?;
        reader.finish()?;
        Ok(Self {
            campaign_id,
            content,
            capture_mode,
            sessions,
            run_sessions,
            stop,
            format,
            journaled,
            controls,
        })
    }
}

fn write_controls(out: &mut Bytes, controls: &[Vec<u8>]) -> io::Result<()> {
    out.varint(controls.len() as u64)?;
    for control in controls {
        out.varint(control.len() as u64)?;
        out.raw(control)?;
    }
    Ok(())
}

fn read_controls(reader: &mut Reader<'_>, limit: usize, into: &mut Vec<Vec<u8>>) -> io::Result<()> {
    let count = usize::try_from(reader.varint()?)
        .ok()
        .filter(|&count| count <= limit)
        .ok_or_else(|| invalid("A control count passes its record"))?;
    engine_allocation::reserve(into, count)?;
    for _ in 0..count {
        let length = usize::try_from(reader.varint()?)
            .map_err(|_| invalid("A control record passes the address space"))?;
        into.push(engine_allocation::copy_vec(reader.raw(length)?)?);
    }
    Ok(())
}

/// The capture a root's records extents hold: each revision's newly added control records, in
/// order, then the capture record, which every revision replaces.
pub fn read_capture(
    extents: &[Extent],
    mut raw: impl FnMut(&Extent) -> io::Result<Vec<u8>>,
) -> io::Result<Option<Capture>> {
    let Some((last, chunks)) = extents.split_last() else {
        return Ok(None);
    };
    let mut capture = Capture::read(&raw(last)?)?;
    for extent in chunks {
        let bytes = raw(extent)?;
        let mut reader = Reader::new(&bytes);
        read_controls(&mut reader, bytes.len(), &mut capture.controls)?;
        reader.finish()?;
    }
    Ok(Some(capture))
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

#[cfg(any(test, feature = "test-support"))]
pub use hours::HourWriter;

/// Hours written into a file as the recorder writes them, without its records: for tests that
/// make recordings of synthetic hours.
#[cfg(any(test, feature = "test-support"))]
mod hours {
    use super::{BlockEnd, FileWriter};
    use replay_columnar::{
        envelope::UNIT_ORDINALS,
        lane::{Clock, ClockHour},
        manifest::Manifest,
        store::{Store, StringSource, journal::Journal},
        unit::{self, Encoder},
    };
    use std::{
        io::{self, Read, Seek, Write},
        sync::{Arc, Mutex, PoisonError},
    };

    /// Every string the hours appended, for the store to read again what it let go of.
    #[derive(Clone, Default)]
    struct Appended(Arc<Mutex<Vec<Vec<u8>>>>);

    impl StringSource for Appended {
        fn text(&mut self, id: u32) -> io::Result<Option<Vec<u8>>> {
            Ok(self.0.lock().unwrap_or_else(PoisonError::into_inner).get(id as usize).cloned())
        }
    }

    /// The block under way: its hours, clock, strings appended and their first id.
    struct OpenBlock {
        hours: u32,
        clock: Clock,
        count: u64,
        strings: Vec<u8>,
        first: u32,
    }

    /// A file's runs written from hours (manifest and sections, as the journal holds them): the
    /// recorder's store and encoder, every block's page with its clock, strings and summaries.
    /// The derived tables are their writers', run each hour as the recorder runs them: what the
    /// hours say of those tables is left out.
    pub struct HourWriter<W: Write + Seek + Read> {
        file: FileWriter<W>,
        store: Store,
        adapter: Journal,
        writers: recorder_derive::Writers,
        columns: Vec<usize>,
        appended: Appended,
        running: bool,
        block: Option<OpenBlock>,
        summaries: crate::campaign::live::Summaries,
        /// The run under way's first hour and its hours so far.
        run: (i32, u32),
    }

    impl<W: Write + Seek + Read> HourWriter<W> {
        pub fn new(file: FileWriter<W>, level: i32) -> io::Result<Self> {
            Self::with_unit(file, level, unit::Settings::at(level))
        }

        /// [`Self::new`], its blocks restored and packed as `settings` say.
        pub fn with_unit(file: FileWriter<W>, level: i32, settings: unit::Settings) -> io::Result<Self> {
            let kinds = crate::columnar_frames::schema()?;
            let columns = kinds.iter().map(Vec::len).collect();
            let mut store = Store::new(kinds.clone(), crate::columnar_frames::keeps_last().to_vec());
            store.declare_logs(&crate::rows::logs())?;
            store.attach(Encoder::new(&kinds, settings)?);
            let appended = Appended::default();
            store.read_strings_from(Box::new(appended.clone()));
            let writers = recorder_derive::writers();
            let adapter = Journal::leaving(&writers.tables(), crate::columnar_frames::names());
            Ok(Self { file, store, adapter, writers, columns, appended, running: false, block: None, summaries: crate::campaign::live::Summaries::new(level)?, run: (0, 0) })
        }

        pub fn file(&mut self) -> &mut FileWriter<W> {
            &mut self.file
        }

        /// The frame of `tables` as the store holds them after the hour written last, named
        /// `(branch, hour)`: what a reader of the file is to read back at that hour.
        pub fn frame(&self, tables: &[crate::rows::TableId], (branch, hour): (u64, i32), mode: crate::scope::CaptureMode) -> io::Result<crate::frame::DecodedFrame> {
            let format = crate::format::FormatVersion::CURRENT.version();
            let mut frame = crate::frame::stored_frame(tables.iter().map(|&table| (table, self.store.view(table.index()))), (branch, hour), mode, format)?;
            for &table in tables {
                let view = self.store.view(table.index());
                if matches!(view.status(), replay_columnar::manifest::Status::Complete) {
                    let held = std::mem::replace(&mut frame.tables[table.index()], crate::frame::not_selected());
                    frame.tables[table.index()] = held.with_sampled_at(view.sampled_at());
                }
            }
            Ok(frame)
        }

        /// [`Self::frame`] of the derived writers' tables, every other table not selected, each
        /// table its writer ran this hour named read without a time, as a reader names it.
        pub fn derived_frame(&self, at: (u64, i32), mode: crate::scope::CaptureMode) -> io::Result<crate::frame::DecodedFrame> {
            let tables: Vec<crate::rows::TableId> = self.writers.tables().into_iter().map(|index| crate::rows::TableId::ALL[index]).collect();
            let mut frame = self.frame(&tables, at, mode)?;
            for table in tables {
                // The writers stamp a table's sample hour with the hour exactly when they ran.
                if self.store.view(table.index()).sampled_at() == Some(at.1) {
                    frame.metrics.table_ns[table.index()] = crate::frame::READ_TIME_NOT_RECORDED;
                }
            }
            Ok(frame)
        }

        /// Starts a run of `branch` at `first_hour`, ending the one under way.
        pub fn start_run(&mut self, branch: u64, epoch: u8, first_hour: i32) -> io::Result<()> {
            self.end_run()?;
            self.file.start_run(branch, epoch, first_hour)?;
            self.store.begin_run(first_hour);
            self.writers.start_run();
            self.summaries.start_run();
            self.run = (first_hour, 0);
            self.appended.0.lock().unwrap_or_else(PoisonError::into_inner).clear();
            self.running = true;
            Ok(())
        }

        /// The run's next hour: its manifest and its sections, in the manifest's order.
        pub fn hour(&mut self, manifest: &[u8], sections: &[u8]) -> io::Result<()> {
            if !self.running {
                return Err(io::Error::other("An hour needs a run under way"));
            }
            let manifest = Manifest::read(manifest, &self.columns)?;
            let first = self.store.strings().next();
            if self.block.is_none() {
                self.summaries.open(&mut self.store, manifest.hour);
            }
            let block = self.block.get_or_insert_with(|| OpenBlock { hours: 0, clock: Clock::default(), count: 0, strings: Vec::new(), first });
            for text in &manifest.appends {
                unit::put_varint(&mut block.strings, text.len() as u64);
                block.strings.extend_from_slice(text);
                self.appended.0.lock().unwrap_or_else(PoisonError::into_inner).push(text.to_vec());
            }
            block.count += manifest.appends.len() as u64;
            block.clock.hour(&ClockHour { flags: manifest.flags, total_ns: manifest.total_ns, pool_bytes: manifest.pool_bytes, dropped_frames: manifest.dropped_frames });
            block.hours += 1;
            let ends = block.hours == UNIT_ORDINALS;
            self.adapter.begin_hour(&mut self.store, &manifest)?;
            let mut at = 0_usize;
            for entry in &manifest.sections {
                let end = at + entry.bytes as usize;
                let bytes = sections.get(at..end).ok_or_else(|| io::Error::other("An hour's sections end before its manifest's"))?;
                self.adapter.section(&mut self.store, entry, bytes)?;
                at = end;
            }
            self.adapter.applied(&mut self.store);
            self.writers.hour(manifest.branch, manifest.hour, self.run.1 == 0, &mut self.store)?;
            self.summaries.hour(&self.store, manifest.hour);
            self.store.end_hour(&mut self.file)?;
            self.adapter.moved(&mut self.store);
            self.run.1 += 1;
            if ends {
                self.end_block()?;
            }
            Ok(())
        }

        fn end_block(&mut self) -> io::Result<()> {
            let Some(block) = self.block.take() else { return Ok(()) };
            let mut strings = Vec::new();
            unit::put_varint(&mut strings, block.count);
            strings.extend_from_slice(&block.strings);
            let clock = block.clock.finish();
            let (first_hour, hours) = self.run;
            let (summaries, span) = self.summaries.finish(first_hour + hours as i32 - 1, (hours - block.hours) / UNIT_ORDINALS, first_hour, &mut self.file)?;
            let (_, span) = self.file.end_block(BlockEnd { clock: &clock, strings: (&strings, block.first), summaries, span })?;
            self.summaries.written(&span);
            self.store.end_block();
            Ok(())
        }

        /// Ends the run under way: its last block, short, when one is under way.
        pub fn end_run(&mut self) -> io::Result<()> {
            if !std::mem::take(&mut self.running) {
                return Ok(());
            }
            self.store.end_run(&mut self.file)?;
            self.end_block()?;
            let tail = self.summaries.tail(self.run.0, self.run.1, &mut self.file)?;
            self.file.tail(&tail)?;
            self.file.finish_run()
        }

        /// Ends the run under way; the file writer.
        pub fn finish(mut self) -> io::Result<FileWriter<W>> {
            self.end_run()?;
            Ok(self.file)
        }
    }
}

/// Kind-only while a capture runs, whatever diagnostic scope encloses it: a writer in a capture
/// must not allocate to fail. Offline writers keep the message.
fn invalid(message: &'static str) -> io::Error {
    if engine_allocation::capture_active() {
        io::Error::from(io::ErrorKind::InvalidData)
    } else {
        io::Error::new(io::ErrorKind::InvalidData, message)
    }
}

/// Where the recorder's store reads again a string it let go of: the strings frames of the
/// blocks its run wrote, read from the file.
pub struct FileStrings {
    file: std::fs::File,
    /// Every string chunk the run's written blocks hold: its first string id and its frame, in
    /// order.
    blocks: std::sync::Arc<std::sync::Mutex<Vec<(u32, replay_columnar::unit::FrameEntry)>>>,
    /// The chunks read during the hour under way, by their place: each decoded, and where each
    /// string lies in it.
    held: std::collections::HashMap<usize, (Vec<u8>, Vec<(u32, u32)>)>,
}

impl FileStrings {
    /// Strings read from `file` through the blocks `blocks` lists as they are written.
    pub fn new(file: std::fs::File, blocks: std::sync::Arc<std::sync::Mutex<Vec<(u32, replay_columnar::unit::FrameEntry)>>>) -> Self {
        Self { file, blocks, held: std::collections::HashMap::new() }
    }
}

impl replay_columnar::store::StringSource for FileStrings {
    fn text(&mut self, id: u32) -> io::Result<Option<Vec<u8>>> {
        let (at, frame, first) = {
            let blocks = self.blocks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let at = blocks.partition_point(|&(first, _)| first <= id);
            let Some(at) = at.checked_sub(1) else { return Ok(None) };
            let (first, frame) = blocks[at].clone();
            (at, frame, first)
        };
        if !self.held.contains_key(&at) {
            let _label = engine_allocation::labels::enter(engine_allocation::labels::owner("strings: chunks read again"), engine_allocation::labels::Kind::Operator);
            let decoded = crate::columnar_frames::strings_frame(&self.file, &frame)?;
            self.held.insert(at, decoded);
        }
        let (raw, places) = &self.held[&at];
        Ok(places.get((id - first) as usize).map(|&(at, length)| raw[at as usize..(at + length) as usize].to_vec()))
    }

    fn release(&mut self) {
        self.held = std::collections::HashMap::new();
    }
}
