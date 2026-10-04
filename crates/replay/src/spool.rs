//! Uncompressed, independently recoverable spool segments.

use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crate::{
    FORMAT_VERSION,
    branches::{BranchStart, Gap},
    codec::{Decoder, invalid, put_string},
    frame::{EncodedFrame, Frame},
    identity::SessionStart,
};

pub const RECORD_MAGIC: [u8; 4] = *b"H4SV";
/// Magic, recording version, payload length and payload CRC, all scalars little-endian.
pub const RECORD_HEADER_BYTES: usize = 4 + 2 + 4 + 4;
/// One record carries one frame, and a frame must fit a replay block, so the
/// block's raw limit is the only bound. A fixed smaller cap refused keyframes
/// of campaigns with a few thousand countries (~150-240 MB at 1,500).
pub const MAX_RECORD_BYTES: usize = crate::container::MAX_BLOCK_RAW_BYTES;
const SYNC_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(
    clippy::large_enum_variant,
    reason = "Frames stay inline without a per-hour box; replay trailers retain controls in their compact enum."
)]
pub enum Record {
    SessionStart(SessionStart),
    BranchStart(BranchStart),
    Frame(Frame),
    Gap(Gap),
    Control(ControlRecord),
}

/// Per-hour trailer entries never reserve space for a frame or session identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlRecord {
    Detach {
        session: u64,
        last_hour: Option<i32>,
    },
    ClockWitness {
        session: u64,
        qpc: u64,
        hour: i32,
        load_signal: Option<i64>,
    },
    /// A bounded callback loss log overflowed; no hour range is invented.
    GapSummary {
        session: u64,
        dropped_frames: u64,
        reason: String,
    },
}

impl ControlRecord {
    pub const fn session(&self) -> u64 {
        match self {
            Self::Detach { session, .. }
            | Self::ClockWitness { session, .. }
            | Self::GapSummary { session, .. } => *session,
        }
    }

    /// These tags are shared by spool payloads and the replay trailer.
    pub fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::GapSummary {
                session,
                dropped_frames,
                reason,
            } => {
                if *dropped_frames == 0 || reason.trim().is_empty() {
                    return Err(invalid("gap summary needs a count and reason"));
                }
                out.push(6);
                out.extend_from_slice(&session.to_le_bytes());
                out.extend_from_slice(&dropped_frames.to_le_bytes());
                put_string(out, reason)?;
            }
            Self::Detach { session, last_hour } => {
                out.push(4);
                out.extend_from_slice(&session.to_le_bytes());
                out.push(u8::from(last_hour.is_some()));
                if let Some(hour) = last_hour {
                    out.extend_from_slice(&hour.to_le_bytes());
                }
            }
            Self::ClockWitness {
                session,
                qpc,
                hour,
                load_signal,
            } => {
                out.push(5);
                out.extend_from_slice(&session.to_le_bytes());
                out.extend_from_slice(&qpc.to_le_bytes());
                out.extend_from_slice(&hour.to_le_bytes());
                out.push(u8::from(load_signal.is_some()));
                if let Some(signal) = load_signal {
                    out.extend_from_slice(&signal.to_le_bytes());
                }
            }
        }
        Ok(())
    }

    pub fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Self::decode_tag(input.u8()?, input)
    }

    fn decode_tag(tag: u8, input: &mut Decoder<'_>) -> io::Result<Self> {
        Ok(match tag {
            6 => {
                let session = input.u64()?;
                let dropped_frames = input.u64()?;
                let reason = input.string()?;
                if dropped_frames == 0 || reason.trim().is_empty() {
                    return Err(invalid("gap summary needs a count and reason"));
                }
                Self::GapSummary {
                    session,
                    dropped_frames,
                    reason,
                }
            }
            4 => Self::Detach {
                session: input.u64()?,
                last_hour: if input.bool()? {
                    Some(input.i32()?)
                } else {
                    None
                },
            },
            5 => Self::ClockWitness {
                session: input.u64()?,
                qpc: input.u64()?,
                hour: input.i32()?,
                load_signal: if input.bool()? {
                    Some(input.i64()?)
                } else {
                    None
                },
            },
            _ => return Err(invalid("unknown control record tag")),
        })
    }
}

impl Record {
    /// Encode a payload; `encode_record` adds its length and checksum envelope.
    pub fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::SessionStart(value) => {
                out.push(0);
                value.encode(out)?;
            }
            Self::BranchStart(value) => {
                out.push(1);
                value.encode(out)?;
            }
            Self::Frame(value) => {
                out.push(2);
                value.encode(out)?;
            }
            Self::Gap(value) => {
                out.push(3);
                value.encode(out)?;
            }
            Self::Control(value) => value.encode(out)?,
        }
        Ok(())
    }

    pub fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Ok(match input.u8()? {
            0 => Self::SessionStart(SessionStart::decode(input)?),
            1 => Self::BranchStart(BranchStart::decode(input)?),
            // Recovery checks the envelope's recording version before
            // decoding this payload.
            2 => Self::Frame(Frame::decode(input)?),
            3 => Self::Gap(Gap::decode(input)?),
            tag @ 4..=6 => Self::Control(ControlRecord::decode_tag(tag, input)?),
            _ => return Err(invalid("unknown spool record tag")),
        })
    }
}

pub fn encode_record(record: &Record) -> io::Result<Vec<u8>> {
    let mut out = vec![0; RECORD_HEADER_BYTES];
    record.encode(&mut out)?;
    seal_record(out)
}

fn encode_frame_record(mut frame: EncodedFrame) -> io::Result<Vec<u8>> {
    frame.bytes[RECORD_HEADER_BYTES] = 2;
    seal_record(frame.bytes)
}

fn seal_record(mut out: Vec<u8>) -> io::Result<Vec<u8>> {
    let length = out.len() - RECORD_HEADER_BYTES;
    if length > MAX_RECORD_BYTES {
        return Err(invalid("spool record exceeds the replay block limit"));
    }
    let crc = crc32fast::hash(&out[RECORD_HEADER_BYTES..]);
    out[..4].copy_from_slice(&RECORD_MAGIC);
    out[4..6].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    out[6..10].copy_from_slice(&(length as u32).to_le_bytes());
    out[10..14].copy_from_slice(&crc.to_le_bytes());
    Ok(out)
}

/// This checks segment context, not table baselines, which belong to FrameDecoder.
#[derive(Clone, Debug, Default)]
pub(crate) struct SegmentState {
    records: usize,
    session: Option<u64>,
    branch: Option<u64>,
    branches: BTreeSet<u64>,
    first_hour: Option<i32>,
    last_hour: Option<i32>,
    needs_keyframe: bool,
    gap_before_keyframe: bool,
    detached: bool,
}

impl SegmentState {
    pub(crate) fn accept(&mut self, record: &Record) -> io::Result<()> {
        if self.detached {
            return Err(invalid("record after detach"));
        }
        if self.records == 0 && !matches!(record, Record::SessionStart(_) | Record::BranchStart(_))
        {
            return Err(invalid(
                "segment does not begin with session or branch context",
            ));
        }
        match record {
            Record::SessionStart(session) => {
                if self.records != 0 {
                    return Err(invalid("session start occurs inside a segment"));
                }
                self.session = Some(session.id);
                self.needs_keyframe = true;
            }
            Record::BranchStart(branch) => {
                if self.branch.is_some() && self.needs_keyframe && !self.gap_before_keyframe {
                    return Err(invalid("branch has no initial keyframe or explicit gap"));
                }
                self.require_session(branch.session)?;
                if !self.branches.insert(branch.id) {
                    return Err(invalid("repeated branch start inside a segment"));
                }
                self.session = Some(branch.session);
                self.branch = Some(branch.id);
                self.first_hour = Some(branch.first_hour);
                self.last_hour = None;
                self.needs_keyframe = true;
                self.gap_before_keyframe = false;
            }
            Record::Frame(frame) => {
                return self.accept_frame(frame.branch, frame.hour, frame.keyframe);
            }
            Record::Gap(gap) => {
                if self.branch != Some(gap.branch) {
                    return Err(invalid("gap lacks matching branch context"));
                }
                if gap.first_hour > gap.last_hour || gap.reason.is_empty() {
                    return Err(invalid("invalid gap range or empty reason"));
                }
                if self.first_hour.is_some_and(|hour| gap.first_hour < hour) {
                    return Err(invalid("gap precedes declared first branch hour"));
                }
                // Disk recovery may preserve only gaps for a branch before a load.
                // Its first hour may already have a frame in an earlier segment.
                self.gap_before_keyframe |= self.needs_keyframe;
            }
            Record::Control(ControlRecord::ClockWitness { session, .. })
            | Record::Control(ControlRecord::GapSummary { session, .. }) => {
                self.require_session(*session)?
            }
            Record::Control(ControlRecord::Detach { session, last_hour }) => {
                self.require_session(*session)?;
                if self.needs_keyframe {
                    return Err(invalid("detach before initial keyframe"));
                }
                if last_hour.is_some() && *last_hour != self.last_hour {
                    return Err(invalid("detach hour differs from last recorded frame"));
                }
                self.detached = true;
            }
        }
        self.records += 1;
        Ok(())
    }

    fn accept_frame(&mut self, branch: u64, hour: i32, keyframe: bool) -> io::Result<()> {
        if self.detached {
            return Err(invalid("record after detach"));
        }
        if self.records == 0 {
            return Err(invalid(
                "segment does not begin with session or branch context",
            ));
        }
        if self.branch != Some(branch) {
            return Err(invalid("frame lacks matching branch context"));
        }
        if self.needs_keyframe && !keyframe {
            return Err(invalid("branch or segment does not start with a keyframe"));
        }
        if self.first_hour.is_some_and(|first| hour < first) {
            return Err(invalid("frame precedes declared first branch hour"));
        }
        if self.last_hour.is_some_and(|last| hour <= last) {
            return Err(invalid("frame hours do not increase within branch"));
        }
        self.last_hour = Some(hour);
        self.needs_keyframe = false;
        self.records += 1;
        Ok(())
    }

    fn require_session(&self, session: u64) -> io::Result<()> {
        if self.session.is_some_and(|expected| expected != session) {
            Err(invalid("record belongs to another session"))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug)]
pub struct Recovery {
    pub records: Vec<Record>,
    pub valid_bytes: usize,
    pub damage: Option<String>,
}

/// Recover only this segment's valid prefix. Never scan past damage for another magic.
pub fn recover_prefix(bytes: &[u8]) -> Recovery {
    let mut result = Recovery {
        records: Vec::new(),
        valid_bytes: 0,
        damage: None,
    };
    let mut state = SegmentState::default();
    while result.valid_bytes < bytes.len() {
        let offset = result.valid_bytes;
        let decoded = (|| -> io::Result<(Record, usize)> {
            let tail = &bytes[offset..];
            if tail.len() < RECORD_HEADER_BYTES {
                return Err(invalid("truncated record header"));
            }
            if tail[..4] != RECORD_MAGIC {
                return Err(invalid(format!(
                    "unsupported spool record magic {:?}; expected {:?}",
                    String::from_utf8_lossy(&tail[..4]),
                    String::from_utf8_lossy(&RECORD_MAGIC),
                )));
            }
            let version = u16::from_le_bytes(tail[4..6].try_into().unwrap());
            if version != FORMAT_VERSION {
                return Err(invalid(format!(
                    "unsupported spool format version {version}; expected {FORMAT_VERSION}"
                )));
            }
            let length = u32::from_le_bytes(tail[6..10].try_into().unwrap()) as usize;
            if length == 0 || length > MAX_RECORD_BYTES {
                return Err(invalid("invalid record length"));
            }
            if length > tail.len() - RECORD_HEADER_BYTES {
                return Err(invalid("truncated record payload"));
            }
            let payload = &tail[RECORD_HEADER_BYTES..RECORD_HEADER_BYTES + length];
            let expected = u32::from_le_bytes(tail[10..14].try_into().unwrap());
            if crc32fast::hash(payload) != expected {
                return Err(invalid("record CRC mismatch"));
            }
            let mut input = Decoder::new(payload);
            let record = Record::decode(&mut input)?;
            input.finish()?;
            state.accept(&record)?;
            Ok((record, RECORD_HEADER_BYTES + length))
        })();
        match decoded {
            Ok((record, length)) => {
                result.records.push(record);
                result.valid_bytes += length;
            }
            Err(error) => {
                result.damage = Some(format!("segment byte {offset}: {error}"));
                return result;
            }
        }
    }
    if state.needs_keyframe {
        result.damage = Some("segment ended before initial keyframe".into());
    }
    result
}

/// Periodic synchronization work for one live segment, excluding forced barriers.
#[derive(Clone, Copy, Debug, Default)]
pub struct BackgroundSyncWork {
    pub requested: u64,
    pub completed: u64,
    pub total_ns: u64,
    pub max_ns: u64,
    pub pending: bool,
}

/// A narrow I/O seam permits fault tests without creating files or spawning processes.
pub trait SegmentSink: Write {
    fn sync_all(&mut self) -> io::Result<()>;

    /// Start periodic durability work. A sink may complete it in the background;
    /// ordinary writes remain ordered, and forced boundaries still use sync_all.
    fn start_sync(&mut self) -> io::Result<()> {
        self.sync_all()
    }

    /// Report pending durability errors before accepting more records. False
    /// means the previous request is still running, so another must not start.
    fn poll_sync(&mut self) -> io::Result<bool> {
        Ok(true)
    }

    fn background_sync_work(&self) -> Option<BackgroundSyncWork> {
        None
    }
}

impl SegmentSink for File {
    fn sync_all(&mut self) -> io::Result<()> {
        File::sync_all(self)
    }
}

/// One ordered writer and at most one periodic durability worker per segment.
/// The worker owns a cloned handle, never record bytes or capture buffers.
pub struct FileSink {
    file: File,
    sync_file: File,
    pending: Option<std::thread::JoinHandle<(u64, io::Result<()>)>>,
    work: BackgroundSyncWork,
}

#[cfg(windows)]
fn independent_sync_file(file: &File) -> io::Result<File> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use windows_sys::Win32::{
        Foundation::{GENERIC_WRITE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, ReOpenFile},
    };
    // Reopen the existing object, not its path. A duplicated synchronous writer
    // handle would share that object's I/O serialization with the flush thread.
    let handle = unsafe {
        ReOpenFile(
            file.as_raw_handle(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            0,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_handle(handle) })
    }
}

#[cfg(not(windows))]
fn independent_sync_file(file: &File) -> io::Result<File> {
    file.try_clone()
}

impl FileSink {
    fn finish_pending(&mut self) -> io::Result<()> {
        if let Some(pending) = self.pending.take() {
            let (elapsed, result) = pending
                .join()
                .map_err(|_| io::Error::other("spool durability worker panicked"))?;
            self.work.completed += 1;
            self.work.total_ns = self.work.total_ns.saturating_add(elapsed);
            self.work.max_ns = self.work.max_ns.max(elapsed);
            result?;
        }
        Ok(())
    }
}

impl Write for FileSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.file.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl SegmentSink for FileSink {
    fn sync_all(&mut self) -> io::Result<()> {
        self.finish_pending()?;
        // Include writes accepted after the background request began.
        self.file.sync_all()
    }

    fn start_sync(&mut self) -> io::Result<()> {
        if !self.poll_sync()? {
            return Ok(());
        }
        let file = self.sync_file.try_clone()?;
        self.pending = Some(
            std::thread::Builder::new()
                .name("replay-durability".into())
                .spawn(move || {
                    let started = Instant::now();
                    let result = file.sync_all();
                    (
                        started.elapsed().as_nanos().min(u64::MAX as u128) as u64,
                        result,
                    )
                })?,
        );
        self.work.requested += 1;
        Ok(())
    }

    fn poll_sync(&mut self) -> io::Result<bool> {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| !pending.is_finished())
        {
            return Ok(false);
        }
        self.finish_pending()?;
        Ok(true)
    }

    fn background_sync_work(&self) -> Option<BackgroundSyncWork> {
        Some(BackgroundSyncWork {
            pending: self.pending.is_some(),
            ..self.work
        })
    }
}

impl Drop for FileSink {
    fn drop(&mut self) {
        // The collector DLL must never unload with its worker still running.
        // Normal close reports failures through sync_all; error cleanup only joins.
        let _ = self.finish_pending();
    }
}

pub trait SegmentBackend {
    type Sink: SegmentSink;
    fn create(&mut self, index: u32) -> io::Result<Self::Sink>;
}

pub struct FileBackend {
    directory: PathBuf,
}

impl FileBackend {
    pub fn new(directory: impl Into<PathBuf>) -> io::Result<Self> {
        let directory = directory.into();
        fs::create_dir_all(&directory)?;
        Ok(Self { directory })
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

impl SegmentBackend for FileBackend {
    type Sink = FileSink;

    fn create(&mut self, index: u32) -> io::Result<FileSink> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.directory.join(format!("spool.{index:04}.h4s")))?;
        let sync_file = independent_sync_file(&file)?;
        Ok(FileSink {
            file,
            sync_file,
            pending: None,
            work: BackgroundSyncWork::default(),
        })
    }
}

pub struct SegmentedSpool<B: SegmentBackend = FileBackend> {
    backend: B,
    sink: Option<B::Sink>,
    state: SegmentState,
    next_segment: u32,
    last_sync: Instant,
    last_error: Option<String>,
}

impl<B: SegmentBackend> SegmentedSpool<B> {
    pub fn new(backend: B) -> Self {
        Self::with_next_segment_index(backend, 0)
    }

    pub fn with_next_segment_index(backend: B, next_segment: u32) -> Self {
        Self {
            backend,
            sink: None,
            state: SegmentState::default(),
            next_segment,
            last_sync: Instant::now(),
            last_error: None,
        }
    }

    #[cfg(test)]
    pub fn next_segment_index(&self) -> u32 {
        self.next_segment
    }
    pub fn is_open(&self) -> bool {
        self.sink.is_some()
    }
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Current open segment only; completed durations exclude forced barriers.
    pub fn background_sync_work(&self) -> Option<BackgroundSyncWork> {
        self.sink.as_ref()?.background_sync_work()
    }

    /// The caller resets its table encoder and supplies a new keyframe on every restart.
    /// Lost hours must be named in `Gap` records in this context or subsequent appends.
    pub fn start_segment(&mut self, context: &[Record], keyframe: &Frame) -> io::Result<()> {
        self.start_segment_with(
            context,
            keyframe.branch,
            keyframe.hour,
            keyframe.keyframe,
            || encode_frame_record(keyframe.encoded()?),
        )
    }

    pub fn start_encoded_segment(
        &mut self,
        context: &[Record],
        keyframe: EncodedFrame,
    ) -> io::Result<()> {
        self.start_segment_with(
            context,
            keyframe.branch,
            keyframe.hour,
            keyframe.keyframe,
            || encode_frame_record(keyframe),
        )
    }

    fn start_segment_with(
        &mut self,
        context: &[Record],
        branch: u64,
        hour: i32,
        keyframe: bool,
        encode: impl FnOnce() -> io::Result<Vec<u8>>,
    ) -> io::Result<()> {
        if self.sink.is_some() {
            return Err(invalid("close the current segment before starting another"));
        }
        if context.is_empty() || !keyframe {
            return Err(invalid("segment start needs context and a keyframe"));
        }
        let mut state = SegmentState::default();
        let mut encoded = Vec::with_capacity(context.len() + 1);
        for record in context {
            if matches!(
                record,
                Record::Frame(_) | Record::Control(ControlRecord::Detach { .. })
            ) {
                return Err(invalid("segment context contains frame or detach"));
            }
            state.accept(record)?;
            encoded.push(encode_record(record)?);
        }
        state.accept_frame(branch, hour, keyframe)?;
        // Preserve context-before-payload errors for the borrowed Frame API.
        // Neither entry point creates a segment until all bytes are ready.
        encoded.push(encode()?);
        let index = self.next_segment;
        self.next_segment = index
            .checked_add(1)
            .ok_or_else(|| invalid("segment index exhausted"))?;
        let sink = match self.backend.create(index) {
            Ok(sink) => sink,
            Err(error) => return self.failed(error),
        };
        self.sink = Some(sink);
        for bytes in encoded {
            if let Err(error) = self.write_flushed(&bytes) {
                return self.failed(error);
            }
        }
        if let Err(error) = self.sync_current() {
            return self.failed(error);
        }
        self.state = state;
        self.last_error = None;
        Ok(())
    }

    pub fn append(&mut self, record: &Record) -> io::Result<()> {
        if self.sink.is_none() {
            return Err(invalid(
                "no writable segment; restart with context and a keyframe",
            ));
        }
        let mut state = self.state.clone();
        state.accept(record)?;
        let bytes = encode_record(record)?;
        self.append_bytes(
            state,
            bytes,
            matches!(record, Record::Control(ControlRecord::Detach { .. })),
        )
    }

    pub fn append_encoded_frame(&mut self, frame: EncodedFrame) -> io::Result<()> {
        if self.sink.is_none() {
            return Err(invalid(
                "no writable segment; restart with context and a keyframe",
            ));
        }
        let mut state = self.state.clone();
        state.accept_frame(frame.branch, frame.hour, frame.keyframe)?;
        self.append_bytes(state, encode_frame_record(frame)?, false)
    }

    fn append_bytes(
        &mut self,
        state: SegmentState,
        bytes: Vec<u8>,
        force_sync: bool,
    ) -> io::Result<()> {
        if let Some(sink) = &mut self.sink
            && let Err(error) = sink.poll_sync()
        {
            return self.failed(error);
        }
        if let Err(error) = self.write_flushed(&bytes) {
            return self.failed(error);
        }
        let synced = if force_sync {
            self.sync_current()
        } else {
            self.sync_if_due()
        };
        if let Err(error) = synced {
            return self.failed(error);
        }
        self.state = state;
        Ok(())
    }

    /// Call on the spool thread's timeout as well as on writes, so pauses still sync.
    pub fn sync_if_due(&mut self) -> io::Result<()> {
        let result = (|| {
            if let Some(sink) = &mut self.sink
                && sink.poll_sync()?
                && self.last_sync.elapsed() >= SYNC_INTERVAL
            {
                sink.flush()?;
                sink.start_sync()?;
                self.last_sync = Instant::now();
            }
            Ok(())
        })();
        if let Err(error) = result {
            return self.failed(error);
        }
        Ok(())
    }

    pub fn finish_segment(&mut self) -> io::Result<()> {
        if self.sink.is_some()
            && let Err(error) = self.sync_current()
        {
            return self.failed(error);
        }
        self.sink = None;
        self.state = SegmentState::default();
        Ok(())
    }

    fn write_flushed(&mut self, bytes: &[u8]) -> io::Result<()> {
        let sink = self
            .sink
            .as_mut()
            .ok_or_else(|| invalid("no writable segment"))?;
        sink.write_all(bytes)?;
        sink.flush()
    }

    fn sync_current(&mut self) -> io::Result<()> {
        let sink = self
            .sink
            .as_mut()
            .ok_or_else(|| invalid("no writable segment"))?;
        sink.flush()?;
        sink.sync_all()?;
        self.last_sync = Instant::now();
        Ok(())
    }

    fn failed<T>(&mut self, error: io::Error) -> io::Result<T> {
        // Dropping this handle is final: even a partial record is never appended to again.
        self.sink = None;
        self.state = SegmentState::default();
        self.last_error = Some(error.to_string());
        Err(error)
    }
}

#[cfg(test)]
mod tests;
