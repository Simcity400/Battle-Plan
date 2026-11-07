//! A columnar recording file (`H4COLUMN` version 4, [`replay_columnar::envelope`]) written run
//! by run, a block at a time ([`crate::columnar_blocks`]): each block's hour records and its
//! tables' blocks, every [`RESTART_BLOCKS`]th with a restart of every table, each stored as a
//! change from the run's previous restart's but every [`KEYFRAME_RESTARTS`]th. A revision
//! publishes everything written so far under a new root; the file ends with its root and
//! footer.
//!
//! [`RESTART_BLOCKS`]: replay_columnar::envelope::RESTART_BLOCKS
//! [`KEYFRAME_RESTARTS`]: replay_columnar::envelope::KEYFRAME_RESTARTS

use replay_columnar::{
    envelope::{self, Extent, IndexPage, KEYFRAME_RESTARTS, RestartDirectory, RestartTable, RunDirectory},
    leaf::{Reader, Writer as Bytes},
    state::{Bits, Column, Table},
};
use std::{
    ffi::{OsStr, OsString},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use zstd::zstd_safe::{CCtx, CParameter};

#[cfg(test)]
mod tests;

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
        read_at(&mut file, length - held as u64, &mut footer[..held])?;
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

/// The compression a writer stores blocks and restarts with ([`crate::compression`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub level: i32,
    /// A block's extents are compressed on the thread pool at once, as an offline writer does;
    /// the recorder's writes one after another on its own thread.
    pub parallel: bool,
}

impl Settings {
    /// Smaller files, more of the recorder's CPU while it writes: zstd 19.
    pub const HIGH: Self = Self {
        level: 19,
        parallel: false,
    };
    /// Larger files, less of the recorder's CPU while it writes: zstd 3.
    pub const LOW: Self = Self {
        level: 3,
        parallel: false,
    };
}

/// `raw` compressed at `level`, with `base`'s raw bytes as its dictionary when given.
pub(crate) fn compress(level: i32, raw: &[u8], base: Option<&[u8]>) -> io::Result<Vec<u8>> {
    let needed = (raw.len() + base.map_or(0, <[u8]>::len)).max(2);
    let window = (usize::BITS - (needed - 1).leading_zeros()).clamp(10, LARGEST_WINDOW_LOG);
    let mut compressor = CCtx::try_create().ok_or_else(engine_allocation::oom)?;
    compressor
        .set_parameter(CParameter::CompressionLevel(level))
        .map_err(zstd_error)?;
    compressor
        .set_parameter(CParameter::WindowLog(window))
        .map_err(zstd_error)?;
    if let Some(base) = base {
        compressor.ref_prefix(base).map_err(zstd_error)?;
    }
    let mut stored = engine_allocation::vec(zstd::zstd_safe::compress_bound(raw.len()))?;
    compressor.compress2(&mut stored, raw).map_err(zstd_error)?;
    Ok(stored)
}

/// Each of `jobs` (raw bytes and the dictionary they are compressed with) compressed at
/// `level`, on the thread pool when `parallel`.
fn compress_all(level: i32, parallel: bool, jobs: &[(&[u8], Option<&[u8]>)]) -> io::Result<Vec<Vec<u8>>> {
    use rayon::prelude::*;
    if parallel {
        jobs.par_iter().map(|&(raw, base)| compress(level, raw, base)).collect()
    } else {
        jobs.iter().map(|&(raw, base)| compress(level, raw, base)).collect()
    }
}

/// The largest window a block is compressed with: what every zstd decoder takes by default.
const LARGEST_WINDOW_LOG: u32 = 27;

/// A finished block, as [`FileWriter::block`] takes it.
pub struct Block<'a> {
    pub first_hour: i32,
    pub last_hour: i32,
    pub hours: u32,
    /// Its hour records, raw ([`replay_columnar::records`]).
    pub records: &'a [u8],
    /// Each table with hours in the block and its block, raw ([`replay_columnar::block`]),
    /// ascending by table.
    pub tables: &'a [(u32, Vec<u8>)],
    pub sectioned: &'a [u64],
    pub read: &'a [u64],
    pub status_changed: &'a [u64],
    /// The run's strings first used since its previous restart, then every table's state.
    pub restart: Option<(&'a [Box<[u8]>], &'a [RestartState])>,
}

/// One table's restart.
pub struct RestartState {
    pub bytes: Vec<u8>,
    /// The run's dictionary ids the state references.
    pub ids: Vec<u32>,
}

/// The finished file, and what its writing stored.
pub struct Finished<W> {
    pub file: W,
    pub written: Written,
}

/// Stored bytes a writer wrote, by what they hold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Written {
    pub records: u64,
    pub tables: u64,
    pub restarts: u64,
    /// Pages, run directories, control records and roots.
    pub index: u64,
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

/// The capture a root's control extents hold: a capture record, then each later revision's
/// newly added control records.
pub fn read_capture(
    extents: &[Extent],
    mut raw: impl FnMut(&Extent) -> io::Result<Vec<u8>>,
) -> io::Result<Option<Capture>> {
    let Some((first, rest)) = extents.split_first() else {
        return Ok(None);
    };
    let mut capture = Capture::read(&raw(first)?)?;
    for extent in rest {
        let bytes = raw(extent)?;
        let mut reader = Reader::new(&bytes);
        read_controls(&mut reader, bytes.len(), &mut capture.controls)?;
        reader.finish()?;
    }
    Ok(Some(capture))
}

fn read_at<F: Read + Seek>(file: &mut F, offset: u64, bytes: &mut [u8]) -> io::Result<()> {
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(bytes)
}

/// The run being written.
struct Run {
    run: u32,
    branch: u64,
    epoch: u8,
    first_hour: i32,
    ordinal: u32,
    chain: [u8; 32],
    pages: Vec<Extent>,
    /// Each restart's dictionary chunk's first id, by restart ordinal.
    chunks: Vec<u32>,
    /// The run's dictionary ids the restarts so far hold.
    ids: u32,
    /// The run's previous restart's raw bytes of every table, which the next restart's are a
    /// change from; none before its first.
    previous: Vec<Vec<u8>>,
    /// Each table's block of the last page, raw, which its next block is a change from; none
    /// past a restart or before the run's first page.
    blocks: Vec<Option<Vec<u8>>>,
}

/// Where a writer takes up a file it wrote before ([`FileWriter::resume`]): the file's runs, the
/// last published revision, and, when its last run may go on, that run.
pub struct Resumed {
    pub length: u64,
    /// The branch and hour of the last hour the file holds.
    pub last: Option<(u64, i32)>,
    pub identity: envelope::Identity,
    pub revision: u64,
    pub runs: Vec<Extent>,
    pub controls: Vec<Extent>,
    /// The control records the file's capture holds.
    pub controls_written: usize,
    pub open: Option<OpenRun>,
}

/// A run a writer takes up: its directory and restarts, and every table's raw bytes at its last
/// restart.
pub struct OpenRun {
    pub run: u32,
    pub directory: RunDirectory,
    pub chunks: Vec<u32>,
    pub ids: u32,
    pub previous: Vec<Vec<u8>>,
    /// Each table's block of the run's last page, raw, when that page carries no restart.
    pub blocks: Vec<Option<Vec<u8>>>,
}

pub struct FileWriter<W: Write + Seek> {
    file: W,
    settings: Settings,
    compressor: CCtx<'static>,
    /// Taken by the root.
    identity: Option<envelope::Identity>,
    tables: usize,
    run: Option<Run>,
    runs: Vec<Extent>,
    /// The last root revision published; zero before the first.
    revision: u64,
    /// Earlier revisions' extents of the control records they added, and how many those hold.
    control_chunks: Vec<Extent>,
    controls_written: usize,
    /// A write failed partway: what the file holds no longer matches what was given.
    poisoned: bool,
    written: Written,
    /// The branch and hour of the last hour written.
    last: Option<(u64, i32)>,
    /// Changes each page before it is written, for tests of damage.
    #[cfg(any(test, feature = "test-support"))]
    tamper: Option<Box<dyn Fn(&mut IndexPage) + Send>>,
}

impl<W: Write + Seek> FileWriter<W> {
    /// Starts the file at its header.
    pub fn create(mut file: W, identity: envelope::Identity, tables: usize, settings: Settings) -> io::Result<Self> {
        file.write_all(envelope::MAGIC)?;
        file.write_all(&envelope::VERSION.to_le_bytes())?;
        Self::with(file, identity, tables, settings)
    }

    fn with(file: W, identity: envelope::Identity, tables: usize, settings: Settings) -> io::Result<Self> {
        let mut compressor = CCtx::try_create().ok_or_else(engine_allocation::oom)?;
        compressor
            .set_parameter(CParameter::CompressionLevel(settings.level))
            .map_err(zstd_error)?;
        Ok(Self {
            file,
            settings,
            compressor,
            identity: Some(identity),
            tables,
            run: None,
            runs: Vec::new(),
            revision: 0,
            control_chunks: Vec::new(),
            controls_written: 0,
            poisoned: false,
            written: Written::default(),
            last: None,
            #[cfg(any(test, feature = "test-support"))]
            tamper: None,
        })
    }

    /// A writer changing each page with `tamper` before it is written, for tests of damage.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_tamper(mut self, tamper: impl Fn(&mut IndexPage) + Send + 'static) -> Self {
        self.tamper = Some(Box::new(tamper));
        self
    }

    /// Takes up `file`, which `resumed` read, after its last published revision: what follows
    /// it is cut, and its last run goes on when `resumed` holds it open.
    pub fn resume(
        mut file: W,
        resumed: Resumed,
        tables: usize,
        settings: Settings,
        truncate: impl FnOnce(&mut W, u64) -> io::Result<()>,
    ) -> io::Result<Self> {
        truncate(&mut file, resumed.length)?;
        file.seek(SeekFrom::Start(resumed.length))?;
        let mut writer = Self::with(file, resumed.identity, tables, settings)?;
        writer.revision = resumed.revision;
        writer.last = resumed.last;
        writer.runs = resumed.runs;
        writer.control_chunks = resumed.controls.get(1..).unwrap_or(&[]).to_vec();
        writer.controls_written = resumed.controls_written;
        if let Some(open) = resumed.open {
            if writer.runs.len() != open.run as usize + 1 {
                return Err(invalid("A run taken up is the file's last"));
            }
            writer.runs.pop();
            writer.run = Some(Run {
                run: open.run,
                branch: open.directory.branch,
                epoch: open.directory.epoch,
                first_hour: open.directory.first_hour,
                ordinal: open.directory.ordinals,
                chain: open.directory.chain,
                pages: open.directory.pages,
                chunks: open.chunks,
                ids: open.ids,
                previous: open.previous,
                // A run is taken up after a restart, or at a page no later walk starts past.
                blocks: open.blocks,
            });
        }
        Ok(writer)
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

    /// Whether a run is under way.
    pub fn running(&self) -> bool {
        self.run.is_some()
    }

    /// The runs the file holds, the one under way included once it holds an hour.
    pub fn published_runs(&self) -> usize {
        self.runs.len() + usize::from(self.run.as_ref().is_some_and(|run| run.ordinal > 0))
    }

    /// The stored bytes written so far.
    pub fn written(&self) -> Written {
        self.written
    }

    /// The branch and hour of the last hour written.
    pub fn last(&self) -> Option<(u64, i32)> {
        self.last
    }

    /// Starts a run of `branch` at `first_hour`, finishing the run under way first. A run
    /// starts from empty tables and its own dictionary.
    pub fn start_run(&mut self, branch: u64, epoch: u8, first_hour: i32) -> io::Result<()> {
        self.usable()?;
        self.finish_run()?;
        let run = u32::try_from(self.runs.len()).map_err(|_| invalid("Runs pass 2^32"))?;
        self.run = Some(Run {
            run,
            branch,
            epoch,
            first_hour,
            ordinal: 0,
            chain: [0; 32],
            pages: Vec::new(),
            chunks: Vec::new(),
            ids: 0,
            previous: Vec::new(),
            blocks: Vec::new(),
        });
        Ok(())
    }

    /// Writes the next block of the run under way: its records, its tables' blocks, the
    /// restart ending it when it carries one, and its page.
    pub fn block(&mut self, block: Block<'_>) -> io::Result<()> {
        self.usable()?;
        let result = self.write_block(block);
        self.poison(result)
    }

    fn write_block(&mut self, block: Block<'_>) -> io::Result<()> {
        let run = self.run.as_ref().ok_or_else(|| invalid("A block needs a run under way"))?;
        let (index, first) = (run.run, run.ordinal);
        if first % envelope::UNIT_ORDINALS != 0 || block.hours == 0 || block.hours > envelope::UNIT_ORDINALS {
            return Err(invalid("A block follows its run's last whole block"));
        }
        let expected = i64::from(run.first_hour) + i64::from(first);
        if i64::from(block.first_hour) != expected
            || i64::from(block.last_hour) != expected + i64::from(block.hours) - 1
        {
            return Err(invalid("A block's hours are its run's next"));
        }
        if block.tables.windows(2).any(|pair| pair[0].0 >= pair[1].0)
            || block.tables.last().is_some_and(|&(table, _)| table as usize >= self.tables)
        {
            return Err(invalid("A block's tables are in order"));
        }
        let mut chained = engine_allocation::filled(self.tables.div_ceil(64), 0_u64)?;
        let mut held = std::mem::take(&mut self.run.as_mut().expect("a run under way").blocks);
        if held.len() < self.tables {
            engine_allocation::resize_default(&mut held, self.tables)?;
        }
        let mut jobs: Vec<(&[u8], Option<&[u8]>)> = engine_allocation::vec(1 + block.tables.len())?;
        jobs.push((block.records, None));
        for (table, payload) in block.tables {
            let base = held[*table as usize].as_deref();
            if base.is_some() {
                chained[*table as usize / 64] |= 1 << (table % 64);
            }
            jobs.push((payload, base));
        }
        let mut stored = compress_all(self.settings.level, self.settings.parallel, &jobs)?.into_iter();
        let records = self.append(envelope::Kind::Records, index, 0, first, block.hours, block.records.len(), &stored.next().expect("records"))?;
        self.written.records += records.stored;
        let mut tables = engine_allocation::vec(block.tables.len())?;
        for ((table, payload), stored) in block.tables.iter().zip(stored) {
            let extent = self.append(envelope::Kind::Block, index, *table, first, block.hours, payload.len(), &stored)?;
            self.written.tables += extent.stored;
            tables.push(extent);
        }
        // A walk starts past a restart with no block before it.
        let mut next = engine_allocation::collect_vec((0..self.tables).map(|_| None))?;
        if block.restart.is_none() {
            for (table, payload) in block.tables {
                next[*table as usize] = Some(engine_allocation::copy_vec(payload)?);
            }
        }
        self.run.as_mut().expect("a run under way").blocks = next;
        let restart = match block.restart {
            Some((strings, states)) => Some(self.restart(strings, states, block.last_hour, first + block.hours - 1)?),
            None => None,
        };
        let run = self.run.as_mut().expect("a run under way");
        run.chain = envelope::chain_block(&run.chain, &records, &tables);
        let bits = |words: &[u64]| Bits::from_words(engine_allocation::copy_vec(words)?);
        #[allow(unused_mut)]
        let mut page = IndexPage {
            run: index,
            first,
            hours: block.hours,
            first_hour: block.first_hour,
            last_hour: block.last_hour,
            records,
            tables,
            chained: Bits::from_words(chained)?,
            restart,
            sectioned: bits(block.sectioned)?,
            read: bits(block.read)?,
            status_changed: bits(block.status_changed)?,
            chain: run.chain,
        };
        #[cfg(any(test, feature = "test-support"))]
        if let Some(tamper) = &self.tamper {
            tamper(&mut page);
        }
        let page = page.write()?;
        let page = self.extent(envelope::Kind::IndexPage, index, 0, first, block.hours, &page, None)?;
        self.written.index += page.stored;
        let run = self.run.as_mut().expect("a run under way");
        engine_allocation::reserve(&mut run.pages, 1)?;
        run.pages.push(page);
        run.ordinal += block.hours;
        self.last = Some((run.branch, block.last_hour));
        Ok(())
    }

    /// The restart after ordinal `after`: the run's strings first used since its previous
    /// restart, then each table's state, a change from the previous restart's unless it is
    /// stored whole.
    fn restart(&mut self, strings: &[Box<[u8]>], states: &[RestartState], hour: i32, after: u32) -> io::Result<RestartDirectory> {
        if states.len() != self.tables {
            return Err(invalid("A restart holds every table"));
        }
        let run = self.run.as_ref().expect("a run under way");
        let index = run.run;
        let first_id = run.ids;
        let count = u32::try_from(strings.len()).map_err(|_| invalid("A run's dictionary passes 2^32 ids"))?;
        let ids = first_id
            .checked_add(count)
            .ok_or_else(|| invalid("A run's dictionary passes 2^32 ids"))?;
        let mut chunk = Bytes::new();
        chunk.varint(u64::from(count))?;
        for string in strings {
            chunk.varint(string.len() as u64)?;
            chunk.raw(string)?;
        }
        let chunk = self.extent(envelope::Kind::DictionaryChunk, index, 0, after, 0, chunk.bytes(), None)?;
        self.written.restarts += chunk.stored;
        let run = self.run.as_mut().expect("a run under way");
        let ordinal = u32::try_from(run.chunks.len()).map_err(|_| invalid("Restarts pass 2^32"))?;
        engine_allocation::reserve(&mut run.chunks, 1)?;
        run.chunks.push(first_id);
        run.ids = ids;
        let starts = engine_allocation::copy_vec(&run.chunks)?;
        let whole = ordinal % KEYFRAME_RESTARTS == 0;
        let previous = std::mem::take(&mut run.previous);
        let mut jobs: Vec<(&[u8], Option<&[u8]>)> = engine_allocation::vec(states.len())?;
        for (table, state) in states.iter().enumerate() {
            if state.ids.iter().any(|&id| id >= ids) {
                return Err(invalid("A table's restart references a dictionary id past the run's"));
            }
            let base = (!whole).then(|| previous.get(table).map(Vec::as_slice)).flatten();
            jobs.push((&state.bytes, base));
        }
        let stored = compress_all(self.settings.level, self.settings.parallel, &jobs)?;
        let mut tables = engine_allocation::vec(states.len())?;
        for (table, (state, stored)) in states.iter().zip(stored).enumerate() {
            let stream = u32::try_from(table).map_err(|_| invalid("Tables pass 2^32"))?;
            let extent = self.append(envelope::Kind::RestartTable, index, stream, after, 0, state.bytes.len(), &stored)?;
            self.written.restarts += extent.stored;
            tables.push(RestartTable {
                table: stream,
                extent,
                base: jobs[table].1.is_some(),
                chunks: chunks_holding(&starts, &state.ids)?,
            });
        }
        let run = self.run.as_mut().expect("a run under way");
        run.previous = engine_allocation::collect_vec(states.iter().map(|state| state.bytes.clone()))?;
        Ok(RestartDirectory {
            after,
            hour,
            ordinal,
            chunk,
            ids: (first_id, count),
            tables,
        })
    }

    /// Ends the run under way with its directory.
    pub fn finish_run(&mut self) -> io::Result<()> {
        self.usable()?;
        // A run that took no hour wrote nothing and is none.
        let Some(run) = self.run.take().filter(|run| run.ordinal > 0) else {
            return Ok(());
        };
        let result = self.directory(&run).map(|directory| self.runs.push(directory));
        self.poison(result)
    }

    fn directory(&mut self, run: &Run) -> io::Result<Extent> {
        let directory = RunDirectory {
            branch: run.branch,
            epoch: run.epoch,
            first_hour: run.first_hour,
            ordinals: run.ordinal,
            chain: run.chain,
            pages: engine_allocation::copy_vec(&run.pages)?,
        }
        .write(run.run)?;
        let extent = self.extent(envelope::Kind::RunDirectory, run.run, 0, 0, 0, &directory, None)?;
        self.written.index += extent.stored;
        engine_allocation::reserve(&mut self.runs, 1)?;
        Ok(extent)
    }

    /// Publishes everything written so far as the next root revision: the run under way's
    /// directory and a root whose control records hold `capture`. `sync` makes what is written
    /// durable before what names it. A failure leaves the file at its last published revision.
    pub fn revise(&mut self, capture: Option<&Capture>, mut sync: impl FnMut(&mut W) -> io::Result<()>) -> io::Result<()> {
        self.usable()?;
        let result = self.publish(capture, &mut sync);
        self.poison(result)
    }

    fn publish(&mut self, capture: Option<&Capture>, sync: &mut dyn FnMut(&mut W) -> io::Result<()>) -> io::Result<()> {
        let mut runs = engine_allocation::copy_vec(&self.runs)?;
        if let Some(run) = self.run.take() {
            let directory = self.directory(&run);
            self.run = Some(run);
            if self.run.as_ref().is_some_and(|run| run.ordinal > 0) {
                runs.push(directory?);
            }
        }
        self.root(runs, capture, sync)
    }

    fn root(&mut self, runs: Vec<Extent>, capture: Option<&Capture>, sync: &mut dyn FnMut(&mut W) -> io::Result<()>) -> io::Result<()> {
        let controls = self.controls(capture, runs.len())?;
        sync(&mut self.file)?;
        let identity = self.identity.take().ok_or_else(|| invalid("A finished file takes no revision"))?;
        let root = envelope::Root {
            identity,
            revision: self.revision + 1,
            runs,
            controls,
            facts: Vec::new(),
        };
        let written = root.write();
        self.identity = Some(root.identity);
        let root = self.extent(envelope::Kind::Root, 0, 0, 0, 0, &written?, None)?;
        self.written.index += root.stored;
        sync(&mut self.file)?;
        self.file.write_all(&envelope::footer(root.offset, root.stored, &root.digest))?;
        self.file.flush()?;
        sync(&mut self.file)?;
        self.revision += 1;
        Ok(())
    }

    /// The root's control records: `capture`, which names a session for each of `runs` runs.
    /// Its record holds no controls; earlier revisions' control extents are kept, and only
    /// those added since are written.
    fn controls(&mut self, capture: Option<&Capture>, runs: usize) -> io::Result<Vec<Extent>> {
        let mut controls = Vec::new();
        if let Some(capture) = capture {
            if capture.run_sessions.len() != runs {
                return Err(invalid("A capture names a session for every run"));
            }
            let added = capture
                .controls
                .get(self.controls_written..)
                .ok_or_else(|| invalid("A capture's control records only grow"))?;
            if !added.is_empty() {
                let mut bytes = Bytes::new();
                write_controls(&mut bytes, added)?;
                let chunk = self.extent(envelope::Kind::Controls, 0, 0, 0, 0, bytes.bytes(), None)?;
                self.written.index += chunk.stored;
                engine_allocation::reserve(&mut self.control_chunks, 1)?;
                self.control_chunks.push(chunk);
                self.controls_written = capture.controls.len();
            }
            let bytes = capture.write_with(&[])?;
            let record = self.extent(envelope::Kind::Controls, 0, 0, 0, 0, &bytes, None)?;
            self.written.index += record.stored;
            engine_allocation::reserve(&mut controls, 1 + self.control_chunks.len())?;
            controls.push(record);
            controls.extend_from_slice(&self.control_chunks);
        }
        Ok(controls)
    }

    /// Finishes the run under way, then appends the root and the footer naming it. `sync`
    /// makes what is written durable before what names it.
    pub fn finish(self, sync: impl FnMut(&mut W) -> io::Result<()>) -> io::Result<Finished<W>> {
        self.finish_with(None, sync)
    }

    /// [`Self::finish`], the root's control records holding `capture`, which names a session
    /// for every run.
    pub fn finish_with_capture(self, capture: &Capture, sync: impl FnMut(&mut W) -> io::Result<()>) -> io::Result<Finished<W>> {
        self.finish_with(Some(capture), sync)
    }

    fn finish_with(mut self, capture: Option<&Capture>, mut sync: impl FnMut(&mut W) -> io::Result<()>) -> io::Result<Finished<W>> {
        self.usable()?;
        self.finish_run()?;
        let runs = std::mem::take(&mut self.runs);
        self.root(runs, capture, &mut sync)?;
        Ok(Finished {
            file: self.file,
            written: self.written,
        })
    }

    /// One compressed extent, described by its envelope; with `base`, compressed with those raw
    /// bytes as its dictionary.
    #[allow(clippy::too_many_arguments)]
    fn extent(
        &mut self,
        kind: envelope::Kind,
        run: u32,
        stream: u32,
        first: u32,
        hours: u32,
        raw: &[u8],
        base: Option<&[u8]>,
    ) -> io::Result<Extent> {
        let stored = match base {
            Some(_) => compress(self.settings.level, raw, base)?,
            None => {
                let needed = raw.len().max(2);
                let window = (usize::BITS - (needed - 1).leading_zeros()).clamp(10, LARGEST_WINDOW_LOG);
                self.compressor
                    .set_parameter(CParameter::WindowLog(window))
                    .map_err(zstd_error)?;
                let mut stored = engine_allocation::vec(zstd::zstd_safe::compress_bound(raw.len()))?;
                self.compressor.compress2(&mut stored, raw).map_err(zstd_error)?;
                stored
            }
        };
        self.append(kind, run, stream, first, hours, raw.len(), &stored)
    }

    /// `stored`, compressed from `raw` bytes, appended as an extent described by its envelope.
    #[allow(clippy::too_many_arguments)]
    fn append(
        &mut self,
        kind: envelope::Kind,
        run: u32,
        stream: u32,
        first: u32,
        hours: u32,
        raw: usize,
        stored: &[u8],
    ) -> io::Result<Extent> {
        let offset = self.file.stream_position()?;
        self.file.write_all(stored)?;
        Ok(Extent {
            kind,
            run,
            stream,
            first,
            hours,
            offset,
            stored: stored.len() as u64,
            raw: raw as u64,
            digest: envelope::digest(stored),
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

/// `recording` with only the runs of the branches `keep` keeps, written to `out`: each kept
/// run's extents as they are, its pages and directory written again for its place in the new
/// file. The capture names only the kept runs and their sessions, and holds no branch start,
/// gap or event loss of a branch left out; `sync` makes what is written durable before what
/// names it ([`FileWriter::finish`]). None, with nothing written, when it keeps no run. A
/// recording that cannot be read whole is refused.
pub fn keep_branches<W: Write + Seek>(
    recording: &crate::columnar_frames::Recording,
    keep: impl Fn(u64) -> bool,
    out: W,
    settings: Settings,
    sync: impl FnMut(&mut W) -> io::Result<()>,
) -> io::Result<Option<Finished<W>>> {
    if !recording.damage().is_empty() {
        return Err(invalid("A columnar recording that cannot be read whole is not rewritten"));
    }
    let mut capture = recording
        .capture()?
        .ok_or_else(|| invalid("A columnar recording names no capture"))?;
    if capture.run_sessions.len() != recording.run_count() as usize {
        return Err(invalid("A capture names a session for every run"));
    }
    let mut kept = Vec::new();
    for run in 0..recording.run_count() {
        if keep(recording.run(run)?.branch) {
            engine_allocation::reserve(&mut kept, 1)?;
            kept.push(run);
        }
    }
    let mut commands = false;
    for bytes in &capture.controls {
        if let crate::spool::Record::Control(crate::spool::ControlRecord::PlayerCommands { packet, .. }) =
            crate::spool::decode_record(bytes)?
        {
            commands |= keep(packet.branch);
        }
    }
    if kept.is_empty() && !commands {
        return Ok(None);
    }
    let tables = crate::columnar_frames::schema()?.len();
    let mut writer = FileWriter::create(out, recording.root_identity().clone(), tables, settings)?;
    for (to, &run) in (0_u32..).zip(&kept) {
        let held = recording.run(run)?;
        let mut pages = engine_allocation::vec(held.pages())?;
        let mut chain = [0; 32];
        for at in 0..held.pages() {
            let mut page = recording.page(run, at)?;
            let copy = |writer: &mut FileWriter<W>, extent: &Extent| -> io::Result<Extent> {
                let stored = recording.stored_bytes(extent)?;
                let offset = writer.file.stream_position()?;
                writer.file.write_all(&stored)?;
                Ok(Extent { run: to, offset, ..*extent })
            };
            page.run = to;
            page.records = copy(&mut writer, &page.records)?;
            writer.written.records += page.records.stored;
            for table in &mut page.tables {
                *table = copy(&mut writer, table)?;
                writer.written.tables += table.stored;
            }
            if let Some(restart) = &mut page.restart {
                restart.chunk = copy(&mut writer, &restart.chunk)?;
                writer.written.restarts += restart.chunk.stored;
                for table in &mut restart.tables {
                    table.extent = copy(&mut writer, &table.extent)?;
                    writer.written.restarts += table.extent.stored;
                }
            }
            chain = envelope::chain_block(&chain, &page.records, &page.tables);
            page.chain = chain;
            let bytes = page.write()?;
            let extent = writer.extent(envelope::Kind::IndexPage, to, 0, page.first, page.hours, &bytes, None)?;
            writer.written.index += extent.stored;
            pages.push(extent);
        }
        let directory = RunDirectory {
            branch: held.branch,
            epoch: held.epoch,
            first_hour: held.first_hour,
            ordinals: held.ordinals,
            chain,
            pages,
        }
        .write(to)?;
        let extent = writer.extent(envelope::Kind::RunDirectory, to, 0, 0, 0, &directory, None)?;
        writer.written.index += extent.stored;
        writer.runs.push(extent);
    }
    let mut sessions = Vec::new();
    let mut run_sessions = Vec::new();
    for &run in &kept {
        let session = &capture.sessions[capture.run_sessions[run as usize] as usize];
        let at = match sessions.iter().position(|held| held == session) {
            Some(at) => at,
            None => {
                engine_allocation::reserve(&mut sessions, 1)?;
                sessions.push(engine_allocation::copy_vec(session)?);
                sessions.len() - 1
            }
        };
        engine_allocation::reserve(&mut run_sessions, 1)?;
        run_sessions.push(at as u32);
    }
    let mut controls = Vec::new();
    for record in std::mem::take(&mut capture.controls) {
        if !of_left_out(&record, &keep)? {
            engine_allocation::reserve(&mut controls, 1)?;
            controls.push(record);
        }
    }
    let mut referenced_sessions = std::collections::BTreeSet::new();
    for bytes in &controls {
        match crate::spool::decode_record(bytes)? {
            crate::spool::Record::Control(control) => {
                referenced_sessions.insert(control.session());
            }
            crate::spool::Record::BranchStart(branch) => {
                referenced_sessions.insert(branch.session);
            }
            _ => {}
        }
    }
    for bytes in &capture.sessions {
        use crate::identity::IdentityCodec;
        let mut input = crate::codec::Decoder::new(bytes);
        input.format = capture.format;
        let session = crate::identity::SessionStart::decode(&mut input)?;
        input.finish()?;
        if referenced_sessions.contains(&session.id) && !sessions.contains(bytes) {
            engine_allocation::reserve(&mut sessions, 1)?;
            sessions.push(engine_allocation::copy_vec(bytes)?);
        }
    }
    let stop = capture.stop.filter(|stop| keep(stop.branch));
    writer
        .finish_with_capture(
            &Capture {
                sessions,
                run_sessions,
                stop,
                controls,
                ..capture
            },
            sync,
        )
        .map(Some)
}

/// Whether `record`, a capture's control record as the row recording encodes it, is a branch
/// start, gap or event loss of a branch `keep` leaves out.
fn of_left_out(record: &[u8], keep: &impl Fn(u64) -> bool) -> io::Result<bool> {
    use crate::spool::{ControlRecord, RECORD_HEADER_BYTES, Record};
    let payload = record
        .get(RECORD_HEADER_BYTES..)
        .ok_or_else(|| invalid("A capture's control record is shorter than its header"))?;
    let branch = match Record::decode(&mut crate::codec::Decoder::new(payload))? {
        Record::BranchStart(start) => Some(start.id),
        Record::Gap(gap) => Some(gap.branch),
        Record::Control(ControlRecord::EventLoss { branch, .. }) => Some(branch),
        Record::Control(ControlRecord::PlayerCommands { packet, .. }) => Some(packet.branch),
        _ => None,
    };
    Ok(branch.is_some_and(|branch| !keep(branch)))
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

fn zstd_error(code: usize) -> io::Error {
    invalid(zstd::zstd_safe::get_error_name(code))
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
