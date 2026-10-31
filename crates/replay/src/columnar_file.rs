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
/// recording, counting from 0: `columnar.<n>.h4col`, `.partial` appended while its collector
/// writes it. Written into `out`, so the collector names it without allocating.
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
    /// The row recording's format version.
    pub format: u16,
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
    const VERSION: u64 = 2;

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
        out.varint(u64::from(self.format))?;
        out.varint(self.controls.len() as u64)?;
        for control in &self.controls {
            out.varint(control.len() as u64)?;
            out.raw(control)?;
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
        let format = u16::try_from(reader.varint()?)
            .map_err(|_| invalid("A capture's format passes 2^16"))?;
        let controls_count = count(&mut reader)?;
        let mut controls = engine_allocation::vec(controls_count)?;
        for _ in 0..controls_count {
            controls.push(owned(&mut reader)?);
        }
        reader.finish()?;
        Ok(Self {
            campaign_id,
            content,
            capture_mode,
            sessions,
            run_sessions,
            stop,
            format,
            controls,
        })
    }
}

/// A file whose writer stopped after publishing a revision, read back to that revision.
pub struct Unfinished {
    /// The file's length through the revision's footer.
    length: u64,
    root: envelope::Root,
    /// The capture the revision's control records hold.
    pub capture: Option<Capture>,
    /// The branch and last hour of its last run that holds an hour.
    pub last: Option<(u64, i32)>,
}

impl Unfinished {
    /// The last published revision of `file`: its last footer whose root verifies, read a
    /// block at a time from the end, then the root's control records and run directories.
    pub fn read<F: Read + Seek>(file: &mut F) -> io::Result<Self> {
        const BLOCK: usize = 1 << 20;
        let length = file.seek(SeekFrom::End(0))?;
        let (header, footer) = (envelope::HEADER_BYTES as u64, envelope::FOOTER_BYTES as u64);
        let mut start = [0; envelope::HEADER_BYTES];
        if length < header + footer
            || read_at(file, 0, &mut start).is_err()
            || &start[..8] != envelope::MAGIC
            || start[8..] != envelope::VERSION.to_le_bytes()
        {
            return Err(invalid("Not a version 3 H4COLUMN recording"));
        }
        let magic = envelope::FOOTER_MAGIC.len();
        let mut block = engine_allocation::filled(BLOCK, 0_u8)?;
        let (mut work, mut stop) = (0_u64, length);
        while stop > header {
            let first = stop.saturating_sub(BLOCK as u64).max(header);
            let count = (stop - first) as usize;
            read_at(file, first, &mut block[..count])?;
            for at in (0..(count + 1).saturating_sub(magic)).rev() {
                if block[at..at + magic] != *envelope::FOOTER_MAGIC {
                    continue;
                }
                let end = first + (at + magic) as u64;
                if end < header + footer {
                    continue;
                }
                let mut bytes = [0; envelope::FOOTER_BYTES];
                read_at(file, end - footer, &mut bytes)?;
                let root = Extent {
                    kind: envelope::Kind::Root,
                    run: 0,
                    stream: 0,
                    first: 0,
                    hours: 0,
                    offset: u64::from_le_bytes(bytes[..8].try_into().expect("eight bytes")),
                    stored: u64::from_le_bytes(bytes[8..16].try_into().expect("eight bytes")),
                    raw: 0,
                    digest: bytes[16..32].try_into().expect("sixteen bytes"),
                };
                // A root sits right before its footer; candidates' roots together read at most
                // twice the file's length.
                if root.offset < header
                    || root.offset.checked_add(root.stored) != Some(end - footer)
                {
                    continue;
                }
                work += root.stored;
                if work > 2 * length {
                    return Err(invalid("No root of the file verifies"));
                }
                if let Some(root) = extent_bytes(file, &root)
                    .ok()
                    .and_then(|raw| envelope::Root::read(&raw).ok())
                {
                    return Self::from_root(file, root, end);
                }
            }
            if first == header {
                break;
            }
            // A footer magic may straddle the block start.
            stop = first + magic as u64 - 1;
        }
        Err(invalid("No root of the file verifies"))
    }

    fn from_root<F: Read + Seek>(file: &mut F, root: envelope::Root, length: u64) -> io::Result<Self> {
        let capture = match root.controls.first() {
            Some(controls) => Some(Capture::read(&extent_bytes(file, controls)?)?),
            None => None,
        };
        let mut last = None;
        for (index, run) in root.runs.iter().enumerate().rev() {
            let index = u32::try_from(index).map_err(|_| invalid("Runs pass 2^32"))?;
            let directory = RunDirectory::read(&extent_bytes(file, run)?, run, index)?;
            if directory.ordinals > 0 {
                let hours = i32::try_from(directory.ordinals - 1)
                    .map_err(|_| invalid("A run's hours pass 2^31"))?;
                let hour = directory
                    .first_hour
                    .checked_add(hours)
                    .ok_or_else(|| invalid("A run's last hour passes 2^31"))?;
                last = Some((directory.branch, hour));
                break;
            }
        }
        Ok(Self {
            length,
            root,
            capture,
            last,
        })
    }

    /// Cuts `file` back to the revision and appends the next, whose control records hold
    /// `capture`, which names a session for every run. `sync` makes what is written durable
    /// before what names it.
    pub fn finish<F: Write + Seek>(
        self,
        file: &mut F,
        truncate: impl FnOnce(&mut F, u64) -> io::Result<()>,
        mut sync: impl FnMut(&mut F) -> io::Result<()>,
        capture: &Capture,
        settings: Settings,
    ) -> io::Result<()> {
        if capture.run_sessions.len() != self.root.runs.len() {
            return Err(invalid("A capture names a session for every run"));
        }
        truncate(file, self.length)?;
        file.seek(SeekFrom::Start(self.length))?;
        let mut compressor = CCtx::try_create().ok_or_else(engine_allocation::oom)?;
        compressor
            .set_parameter(CParameter::CompressionLevel(settings.level))
            .map_err(zstd_error)?;
        let window_log = settings.window_log;
        let controls = append_extent(
            file,
            &mut compressor,
            envelope::Kind::Controls,
            &capture.write()?,
            window_log,
        )?;
        sync(file)?;
        let mut all = Vec::new();
        engine_allocation::reserve(&mut all, 1)?;
        all.push(controls);
        let root = envelope::Root {
            identity: self.root.identity,
            revision: self.root.revision + 1,
            runs: self.root.runs,
            controls: all,
        }
        .write()?;
        let root = append_extent(file, &mut compressor, envelope::Kind::Root, &root, window_log)?;
        sync(file)?;
        file.write_all(&envelope::footer(root.offset, root.stored, &root.digest))?;
        file.flush()?;
        sync(file)
    }
}

/// `extent`'s raw bytes in `file`, once its digest is checked.
fn extent_bytes<F: Read + Seek>(file: &mut F, extent: &Extent) -> io::Result<Vec<u8>> {
    let stored = usize::try_from(extent.stored).map_err(|_| engine_allocation::oom())?;
    let mut bytes = engine_allocation::filled(stored, 0_u8)?;
    read_at(file, extent.offset, &mut bytes)?;
    if envelope::digest(&bytes) != extent.digest {
        return Err(invalid("An extent's bytes do not match its digest"));
    }
    zstd::stream::decode_all(bytes.as_slice())
}

fn read_at<F: Read + Seek>(file: &mut F, offset: u64, bytes: &mut [u8]) -> io::Result<()> {
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(bytes)
}

/// Compresses `raw` at the end of `file` as an extent of `kind`.
fn append_extent<F: Write + Seek>(
    file: &mut F,
    compressor: &mut CCtx<'static>,
    kind: envelope::Kind,
    raw: &[u8],
    window_log: u32,
) -> io::Result<Extent> {
    compressor
        .set_parameter(CParameter::WindowLog(window_log))
        .map_err(zstd_error)?;
    let mut stored = engine_allocation::vec(zstd::zstd_safe::compress_bound(raw.len()))?;
    compressor.compress2(&mut stored, raw).map_err(zstd_error)?;
    let offset = file.stream_position()?;
    file.write_all(&stored)?;
    Ok(Extent {
        kind,
        run: 0,
        stream: 0,
        first: 0,
        hours: 0,
        offset,
        stored: stored.len() as u64,
        raw: raw.len() as u64,
        digest: envelope::digest(&stored),
    })
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
    /// The last root revision published; zero before the first.
    revision: u64,
    /// The open position's unit pieces earlier revisions published.
    pieces: Vec<Extent>,
    /// The ordinal the open position's unpublished bytes start at, once a revision cut them.
    pending_from: Option<u32>,
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
            revision: 0,
            pieces: Vec::new(),
            pending_from: None,
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

    /// The runs a revision now publishes: every finished run, and the one under way once it
    /// holds an hour.
    pub fn published_runs(&self) -> usize {
        self.runs.len() + usize::from(self.run.as_ref().is_some_and(|run| run.ordinal > 0))
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

    /// Finishes the run under way, then appends the root and the footer naming it. `sync`
    /// makes what is written durable before what names it, as [`Self::revise`]'s does.
    pub fn finish(
        self,
        mut sync: impl FnMut(&mut W) -> io::Result<()>,
    ) -> io::Result<Finished<W>> {
        self.finish_with(None, &mut sync)
    }

    /// [`Self::finish`], the root's control records holding `capture`, which names a session
    /// for every run.
    pub fn finish_with_capture(
        self,
        capture: &Capture,
        mut sync: impl FnMut(&mut W) -> io::Result<()>,
    ) -> io::Result<Finished<W>> {
        self.finish_with(Some(capture), &mut sync)
    }

    /// Publishes everything appended so far as the next root revision: the open position's
    /// unpublished bytes as each stream's next unit piece, its page, the active run's directory
    /// and a root whose control records hold `capture`. `sync` makes what is written durable
    /// before what names it. A failure leaves the file at its last published revision.
    pub fn revise(
        &mut self,
        capture: Option<&Capture>,
        mut sync: impl FnMut(&mut W) -> io::Result<()>,
    ) -> io::Result<()> {
        self.usable()?;
        let result = self.publish(capture, &mut sync);
        self.poison(result)
    }

    fn publish(
        &mut self,
        capture: Option<&Capture>,
        sync: &mut dyn FnMut(&mut W) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut runs = Vec::new();
        engine_allocation::reserve(&mut runs, self.runs.len() + 1)?;
        runs.extend_from_slice(&self.runs);
        if let Some(run) = self.run.as_ref().filter(|run| run.ordinal > 0) {
            let (index, ordinal, position) = (run.run, run.ordinal, run.position);
            let mut pages = engine_allocation::copy_vec(&run.pages)?;
            if let Some((first_hour, last_hour)) = position {
                let first = (ordinal - 1) / envelope::UNIT_ORDINALS * envelope::UNIT_ORDINALS;
                let from = self.pending_from.unwrap_or(first);
                if ordinal > from {
                    let written = self.write_pending(index, from, ordinal - from, true)?;
                    engine_allocation::reserve(&mut self.pieces, written.len())?;
                    self.pieces.extend(written);
                    self.pieces.sort_unstable_by_key(|piece| (piece.stream, piece.first));
                }
                self.pending_from = Some(ordinal);
                let run = self.run.as_ref().expect("a run under way");
                let bits = |words: &[u64]| Bits::from_words(engine_allocation::copy_vec(words)?);
                let page = IndexPage {
                    run: index,
                    first,
                    hours: ordinal - first,
                    first_hour,
                    last_hour,
                    units: engine_allocation::copy_vec(&self.pieces)?,
                    restart: None,
                    sectioned: bits(&run.sectioned)?,
                    read: bits(&run.read)?,
                    status_changed: bits(&run.status_changed)?,
                    chain: run.chain,
                }
                .write()?;
                self.index_bytes += page.len() as u64;
                let page = self.extent(
                    envelope::Kind::IndexPage,
                    index,
                    0,
                    first,
                    ordinal - first,
                    &page,
                    self.settings.window_log,
                )?;
                engine_allocation::reserve(&mut pages, 1)?;
                pages.push(page);
            }
            let run = self.run.as_ref().expect("a run under way");
            let directory = RunDirectory {
                branch: run.branch,
                epoch: run.epoch,
                first_hour: run.first_hour,
                ordinals: run.ordinal,
                chain: run.chain,
                pages,
            }
            .write(index)?;
            self.index_bytes += directory.len() as u64;
            let directory = self.extent(
                envelope::Kind::RunDirectory,
                index,
                0,
                0,
                0,
                &directory,
                self.settings.window_log,
            )?;
            runs.push(directory);
        }
        let controls = self.controls(capture, runs.len())?;
        sync(&mut self.file)?;
        let identity = self
            .identity
            .take()
            .ok_or_else(|| invalid("A finished file takes no revision"))?;
        let root = envelope::Root {
            identity,
            revision: self.revision + 1,
            runs,
            controls,
        };
        let written = root.write();
        self.identity = Some(root.identity);
        let root = written?;
        self.index_bytes += root.len() as u64;
        let root = self.extent(envelope::Kind::Root, 0, 0, 0, 0, &root, self.settings.window_log)?;
        sync(&mut self.file)?;
        self.file
            .write_all(&envelope::footer(root.offset, root.stored, &root.digest))?;
        sync(&mut self.file)?;
        self.revision += 1;
        Ok(())
    }

    /// The root's control records: `capture`, which names a session for each of `runs` runs.
    fn controls(&mut self, capture: Option<&Capture>, runs: usize) -> io::Result<Vec<Extent>> {
        let mut controls = Vec::new();
        if let Some(capture) = capture {
            if capture.run_sessions.len() != runs {
                return Err(invalid("A capture names a session for every run"));
            }
            let bytes = capture.write()?;
            self.index_bytes += bytes.len() as u64;
            let window_log = self.settings.window_log;
            engine_allocation::reserve(&mut controls, 1)?;
            controls.push(self.extent(envelope::Kind::Controls, 0, 0, 0, 0, &bytes, window_log)?);
        }
        Ok(controls)
    }

    fn finish_with(
        mut self,
        capture: Option<&Capture>,
        sync: &mut dyn FnMut(&mut W) -> io::Result<()>,
    ) -> io::Result<Finished<W>> {
        self.finish_run()?;
        let controls = self.controls(capture, self.runs.len())?;
        sync(&mut self.file)?;
        let identity = self
            .identity
            .take()
            .ok_or_else(|| invalid("A file has one root"))?;
        let root = envelope::Root {
            identity,
            revision: self.revision + 1,
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
        sync(&mut self.file)?;
        self.file
            .write_all(&envelope::footer(root.offset, root.stored, &root.digest))?;
        self.file.flush()?;
        sync(&mut self.file)?;
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

    /// Every nonempty stream's unit of the position, ascending by stream: the pieces revisions
    /// published, then the rest.
    fn units(&mut self, run: u32, first: u32, hours: u32) -> io::Result<Vec<Extent>> {
        let from = self.pending_from.take().unwrap_or(first);
        let mut units = std::mem::take(&mut self.pieces);
        let rest = self.write_pending(run, from, first + hours - from, false)?;
        engine_allocation::reserve(&mut units, rest.len())?;
        units.extend(rest);
        units.sort_unstable_by_key(|unit| (unit.stream, unit.first));
        // A walker decodes each stream's whole unit, its pieces in turn.
        let mut raws: Vec<usize> = Vec::new();
        engine_allocation::reserve(&mut raws, units.len())?;
        for (at, unit) in units.iter().enumerate() {
            let raw = usize::try_from(unit.raw).map_err(|_| engine_allocation::oom())?;
            if at > 0 && units[at - 1].stream == unit.stream {
                *raws.last_mut().expect("the stream's first piece") += raw;
            } else {
                raws.push(raw);
            }
        }
        let logs = unit_windows(&raws, self.settings.budget)?;
        engine_allocation::reserve(&mut self.walker_memory, 1)?;
        self.walker_memory.push(
            raws.iter()
                .zip(&logs)
                .map(|(&raw, &log)| raw.min(1 << log))
                .sum(),
        );
        Ok(units)
    }

    /// Each nonempty stream's bytes not yet written, for `hours` ordinals from `from`, as a
    /// unit piece, the manifests' first; `pad` aligns table pieces so a later piece continues
    /// them.
    fn write_pending(
        &mut self,
        run: u32,
        from: u32,
        hours: u32,
        pad: bool,
    ) -> io::Result<Vec<Extent>> {
        let mut written = Vec::new();
        engine_allocation::reserve(&mut written, self.units.len() + 1)?;
        let manifests = std::mem::take(&mut self.manifests).into_bytes();
        if !manifests.is_empty() {
            written.push((framing::MANIFESTS, manifests));
        }
        for (table, unit) in self.units.iter_mut().enumerate() {
            if pad {
                unit.align(8)?;
            }
            let raw = std::mem::take(unit).into_bytes();
            if !raw.is_empty() {
                written.push((framing::table_stream(table), raw));
            }
        }
        let raws = engine_allocation::collect_vec(written.iter().map(|(_, raw)| raw.len()))?;
        let logs = unit_windows(&raws, self.settings.budget)?;
        let mut pieces = Vec::new();
        engine_allocation::reserve(&mut pieces, written.len())?;
        for ((stream, raw), log) in written.into_iter().zip(logs) {
            pieces.push(self.extent(envelope::Kind::Unit, run, stream, from, hours, &raw, log)?);
        }
        Ok(pieces)
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
