//! Uncompressed, independently recoverable spool segments.

use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crate::{
    branches::{BranchStart, Gap},
    codec::{Decoder, invalid, put_string},
    frame::Frame,
    identity::SessionStart,
};

pub const RECORD_MAGIC: [u8; 4] = *b"H4S1";
pub const RECORD_HEADER_BYTES: usize = 12;
pub const MAX_RECORD_BYTES: usize = 128 * 1024 * 1024;
const SYNC_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    SessionStart(SessionStart),
    BranchStart(BranchStart),
    Frame(Frame),
    Gap(Gap),
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

impl Record {
    /// Encode a payload; `encode_record` adds its length and checksum envelope.
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
        Ok(match input.u8()? {
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
            0 => Self::SessionStart(SessionStart::decode(input)?),
            1 => Self::BranchStart(BranchStart::decode(input)?),
            2 => Self::Frame(Frame::decode(input)?),
            3 => Self::Gap(Gap::decode(input)?),
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
            _ => return Err(invalid("unknown spool record tag")),
        })
    }
}

pub fn encode_record(record: &Record) -> io::Result<Vec<u8>> {
    let mut out = vec![0; RECORD_HEADER_BYTES];
    record.encode(&mut out)?;
    let length = out.len() - RECORD_HEADER_BYTES;
    if length > MAX_RECORD_BYTES {
        return Err(invalid("spool record exceeds 128 MiB"));
    }
    let crc = crc32fast::hash(&out[RECORD_HEADER_BYTES..]);
    out[..4].copy_from_slice(&RECORD_MAGIC);
    out[4..8].copy_from_slice(&(length as u32).to_le_bytes());
    out[8..12].copy_from_slice(&crc.to_le_bytes());
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
                if self.branch != Some(frame.branch) {
                    return Err(invalid("frame lacks matching branch context"));
                }
                if self.needs_keyframe && !frame.keyframe {
                    return Err(invalid("branch or segment does not start with a keyframe"));
                }
                if self.first_hour.is_some_and(|hour| frame.hour < hour) {
                    return Err(invalid("frame precedes declared first branch hour"));
                }
                if self.last_hour.is_some_and(|hour| frame.hour <= hour) {
                    return Err(invalid("frame hours do not increase within branch"));
                }
                self.last_hour = Some(frame.hour);
                self.needs_keyframe = false;
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
            Record::ClockWitness { session, .. } | Record::GapSummary { session, .. } => {
                self.require_session(*session)?
            }
            Record::Detach { session, last_hour } => {
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
                return Err(invalid("invalid record magic"));
            }
            let length = u32::from_le_bytes(tail[4..8].try_into().unwrap()) as usize;
            if length == 0 || length > MAX_RECORD_BYTES {
                return Err(invalid("invalid record length"));
            }
            if length > tail.len() - RECORD_HEADER_BYTES {
                return Err(invalid("truncated record payload"));
            }
            let payload = &tail[RECORD_HEADER_BYTES..RECORD_HEADER_BYTES + length];
            let expected = u32::from_le_bytes(tail[8..12].try_into().unwrap());
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

/// A narrow I/O seam permits fault tests without creating files or spawning processes.
pub trait SegmentSink: Write {
    fn sync_all(&mut self) -> io::Result<()>;
}

impl SegmentSink for File {
    fn sync_all(&mut self) -> io::Result<()> {
        File::sync_all(self)
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
    type Sink = File;

    fn create(&mut self, index: u32) -> io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.directory.join(format!("spool.{index:04}.h4s")))
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

    pub fn next_segment_index(&self) -> u32 {
        self.next_segment
    }
    pub fn is_open(&self) -> bool {
        self.sink.is_some()
    }
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// The caller resets its table encoder and supplies a new keyframe on every restart.
    /// Lost hours must be named in `Gap` records in this context or subsequent appends.
    pub fn start_segment(&mut self, context: &[Record], keyframe: &Frame) -> io::Result<()> {
        if self.sink.is_some() {
            return Err(invalid("close the current segment before starting another"));
        }
        if context.is_empty() || !keyframe.keyframe {
            return Err(invalid("segment start needs context and a keyframe"));
        }
        let mut state = SegmentState::default();
        let mut encoded = Vec::with_capacity(context.len() + 1);
        for record in context {
            if matches!(record, Record::Frame(_) | Record::Detach { .. }) {
                return Err(invalid("segment context contains frame or detach"));
            }
            state.accept(record)?;
            encoded.push(encode_record(record)?);
        }
        let record = Record::Frame(keyframe.clone());
        state.accept(&record)?;
        encoded.push(encode_record(&record)?);
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
        if let Err(error) = self.write_flushed(&bytes) {
            return self.failed(error);
        }
        if (matches!(record, Record::Detach { .. }) || self.last_sync.elapsed() >= SYNC_INTERVAL)
            && let Err(error) = self.sync_current()
        {
            return self.failed(error);
        }
        self.state = state;
        Ok(())
    }

    /// Call on the spool thread's timeout as well as on writes, so pauses still sync.
    pub fn sync_if_due(&mut self) -> io::Result<()> {
        if self.sink.is_some()
            && self.last_sync.elapsed() >= SYNC_INTERVAL
            && let Err(error) = self.sync_current()
        {
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
mod tests {
    use super::*;
    use crate::{
        branches::{BranchReason, Parentage},
        frame::{Metrics, TableUpdate},
    };
    use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

    fn branch() -> Record {
        Record::BranchStart(BranchStart {
            id: 7,
            session: 3,
            loaded_hour: Some(9),
            first_hour: 10,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        })
    }

    fn frame(hour: i32, keyframe: bool) -> Frame {
        Frame {
            branch: 7,
            hour,
            keyframe,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| TableUpdate::Unavailable {
                reason: "fixture has no tables".into(),
            }),
        }
    }

    fn valid_segment() -> Vec<u8> {
        [
            branch(),
            Record::Frame(frame(10, true)),
            Record::Frame(frame(11, false)),
        ]
        .iter()
        .flat_map(|record| encode_record(record).unwrap())
        .collect()
    }

    #[test]
    fn each_byte_cut_recovers_exactly_the_complete_prefix() {
        let records = [
            branch(),
            Record::Frame(frame(10, true)),
            Record::Frame(frame(11, false)),
        ];
        let encoded: Vec<_> = records
            .iter()
            .map(|record| encode_record(record).unwrap())
            .collect();
        let bytes: Vec<_> = encoded.iter().flatten().copied().collect();
        let ends: Vec<_> = encoded
            .iter()
            .scan(0, |sum, record| {
                *sum += record.len();
                Some(*sum)
            })
            .collect();
        for cut in 0..=bytes.len() {
            let result = recover_prefix(&bytes[..cut]);
            let count = ends.iter().filter(|&&end| end <= cut).count();
            assert_eq!(result.records, records[..count], "cut {cut}");
            assert_eq!(
                result.valid_bytes,
                count.checked_sub(1).map_or(0, |i| ends[i])
            );
            if cut != 0 && cut != ends[1] && cut != ends[2] {
                assert!(result.damage.is_some(), "cut {cut}");
            }
        }
    }

    #[test]
    fn corruption_never_hides_valid_later_segments_or_resynchronizes_inside_one() {
        let good = valid_segment();
        let first_size = encode_record(&branch()).unwrap().len();
        for byte in [0, 4, 8, RECORD_HEADER_BYTES, first_size + 8] {
            let mut broken = good.clone();
            broken[byte] ^= 0x80;
            broken.extend_from_slice(&good);
            let result = recover_prefix(&broken);
            assert!(result.damage.is_some());
            assert_eq!(result.records.len(), usize::from(byte >= first_size));
            assert_eq!(recover_prefix(&good).records.len(), 3);
        }
        let mut oversized = good.clone();
        oversized[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(recover_prefix(&oversized).records.is_empty());
    }

    #[test]
    fn valid_crc_does_not_make_bad_payload_or_context_valid() {
        let good = valid_segment();
        let mut unknown = encode_record(&branch()).unwrap();
        unknown[RECORD_HEADER_BYTES] = 255;
        let crc = crc32fast::hash(&unknown[RECORD_HEADER_BYTES..]);
        unknown[8..12].copy_from_slice(&crc.to_le_bytes());
        assert!(
            recover_prefix(&unknown)
                .damage
                .unwrap()
                .contains("unknown spool")
        );
        let no_context = encode_record(&Record::Frame(frame(10, true))).unwrap();
        assert!(recover_prefix(&no_context).records.is_empty());
        let mut dependent_start = encode_record(&branch()).unwrap();
        dependent_start.extend(encode_record(&Record::Frame(frame(10, false))).unwrap());
        assert_eq!(recover_prefix(&dependent_start).records.len(), 1);
        let mut backwards = good;
        backwards.extend(encode_record(&Record::Frame(frame(10, false))).unwrap());
        assert_eq!(recover_prefix(&backwards).records.len(), 3);
        let mut wrong_branch = frame(12, false);
        wrong_branch.branch = 8;
        let mut wrong = valid_segment();
        wrong.extend(encode_record(&Record::Frame(wrong_branch)).unwrap());
        assert_eq!(recover_prefix(&wrong).records.len(), 3);
    }

    #[test]
    fn control_payloads_preserve_optional_witness_values_and_detach_ends_segment() {
        let records = [
            Record::ClockWitness {
                session: 3,
                qpc: u64::MAX,
                hour: i32::MIN,
                load_signal: Some(i64::MIN),
            },
            Record::ClockWitness {
                session: 3,
                qpc: 0,
                hour: 0,
                load_signal: None,
            },
            Record::Detach {
                session: 3,
                last_hour: None,
            },
            Record::Detach {
                session: 3,
                last_hour: Some(i32::MAX),
            },
        ];
        for record in records {
            let mut bytes = Vec::new();
            record.encode(&mut bytes).unwrap();
            let mut decoder = Decoder::new(&bytes);
            assert_eq!(Record::decode(&mut decoder).unwrap(), record);
            decoder.finish().unwrap();
        }
        let mut bytes = valid_segment();
        bytes.extend(
            encode_record(&Record::Detach {
                session: 3,
                last_hour: Some(11),
            })
            .unwrap(),
        );
        let valid_bytes = bytes.len();
        bytes.extend(encode_record(&Record::Frame(frame(12, false))).unwrap());
        let recovered = recover_prefix(&bytes);
        assert_eq!(recovered.valid_bytes, valid_bytes);
        assert!(recovered.damage.unwrap().contains("after detach"));
    }

    #[derive(Clone, Copy, Debug)]
    enum Fault {
        WriteAfter(usize),
        Flush,
        Sync,
        Create,
    }

    #[derive(Default)]
    struct Memory {
        files: BTreeMap<u32, Vec<u8>>,
        fault: Option<Fault>,
        flushes: usize,
        syncs: usize,
    }

    #[derive(Clone, Default)]
    struct MemoryBackend(Rc<RefCell<Memory>>);

    struct MemorySink {
        shared: Rc<RefCell<Memory>>,
        index: u32,
    }

    fn injected() -> io::Error {
        io::Error::other("injected spool I/O failure")
    }

    impl SegmentBackend for MemoryBackend {
        type Sink = MemorySink;
        fn create(&mut self, index: u32) -> io::Result<MemorySink> {
            let mut memory = self.0.borrow_mut();
            if matches!(memory.fault, Some(Fault::Create)) {
                return Err(injected());
            }
            if memory.files.insert(index, Vec::new()).is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "segment reused",
                ));
            }
            Ok(MemorySink {
                shared: self.0.clone(),
                index,
            })
        }
    }

    impl Write for MemorySink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let mut memory = self.shared.borrow_mut();
            let length = memory.files[&self.index].len();
            let count = match memory.fault {
                Some(Fault::WriteAfter(limit)) => {
                    if length >= limit {
                        return Err(injected());
                    }
                    bytes.len().min(limit - length)
                }
                _ => bytes.len(),
            };
            memory
                .files
                .get_mut(&self.index)
                .unwrap()
                .extend_from_slice(&bytes[..count]);
            Ok(count)
        }
        fn flush(&mut self) -> io::Result<()> {
            let mut memory = self.shared.borrow_mut();
            memory.flushes += 1;
            if matches!(memory.fault, Some(Fault::Flush)) {
                Err(injected())
            } else {
                Ok(())
            }
        }
    }

    impl SegmentSink for MemorySink {
        fn sync_all(&mut self) -> io::Result<()> {
            let mut memory = self.shared.borrow_mut();
            memory.syncs += 1;
            if matches!(memory.fault, Some(Fault::Sync)) {
                Err(injected())
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn failed_write_flush_or_sync_permanently_abandons_that_segment() {
        for fault in [Fault::WriteAfter(0), Fault::Flush, Fault::Sync] {
            let backend = MemoryBackend::default();
            let shared = backend.0.clone();
            let mut spool = SegmentedSpool::new(backend);
            spool.start_segment(&[branch()], &frame(10, true)).unwrap();
            let old_length = shared.borrow().files[&0].len();
            shared.borrow_mut().fault = Some(match fault {
                Fault::WriteAfter(_) => Fault::WriteAfter(old_length + RECORD_HEADER_BYTES + 5),
                other => other,
            });
            // Make the sync failure due without sleeping or relying on wall time.
            spool.last_sync = Instant::now() - SYNC_INTERVAL;
            assert!(spool.append(&Record::Frame(frame(11, false))).is_err());
            assert!(!spool.is_open());
            assert!(spool.last_error().is_some());
            let abandoned = shared.borrow().files[&0].clone();
            shared.borrow_mut().fault = None;
            assert!(spool.append(&Record::Frame(frame(12, false))).is_err());
            assert!(spool.start_segment(&[branch()], &frame(12, false)).is_err());
            let gap = Record::Gap(Gap {
                branch: 7,
                first_hour: 11,
                last_hour: 11,
                reason: "spool I/O failure".into(),
            });
            spool
                .start_segment(&[branch(), gap], &frame(12, true))
                .unwrap();
            spool.append(&Record::Frame(frame(13, false))).unwrap();
            spool.finish_segment().unwrap();
            assert_eq!(spool.next_segment_index(), 2);
            assert_eq!(shared.borrow().files[&0], abandoned);
            let later = recover_prefix(&shared.borrow().files[&1]);
            assert!(later.damage.is_none());
            assert_eq!(later.records.len(), 4);
            let earlier = recover_prefix(&abandoned);
            assert_eq!(
                earlier.records.len(),
                if matches!(fault, Fault::WriteAfter(_)) {
                    2
                } else {
                    3
                }
            );
        }
    }

    #[test]
    fn failed_bootstrap_and_create_skip_segment_numbers_and_can_restart() {
        for fault in [
            Fault::Create,
            Fault::WriteAfter(15),
            Fault::Flush,
            Fault::Sync,
        ] {
            let backend = MemoryBackend::default();
            let shared = backend.0.clone();
            let mut spool = SegmentedSpool::new(backend);
            shared.borrow_mut().fault = Some(fault);
            assert!(spool.start_segment(&[branch()], &frame(10, true)).is_err());
            assert!(!spool.is_open());
            assert_eq!(spool.next_segment_index(), 1);
            shared.borrow_mut().fault = None;
            spool.start_segment(&[branch()], &frame(11, true)).unwrap();
            assert!(recover_prefix(&shared.borrow().files[&1]).damage.is_none());
        }
    }

    #[test]
    fn gap_only_context_requires_explicit_loss_and_never_replaces_a_keyframe() {
        let mut state = SegmentState::default();
        state.accept(&branch()).unwrap();
        let mut next = match branch() {
            Record::BranchStart(value) => value,
            _ => unreachable!(),
        };
        next.id = 8;
        next.loaded_hour = Some(99);
        next.first_hour = 100;
        next.reason = BranchReason::LoadSignal;
        assert!(state.accept(&Record::BranchStart(next.clone())).is_err());
        for (first_hour, last_hour, reason) in [(9, 10, "lost"), (11, 10, "lost"), (10, 10, "")] {
            assert!(
                state
                    .accept(&Record::Gap(Gap {
                        branch: 7,
                        first_hour,
                        last_hour,
                        reason: reason.into(),
                    }))
                    .is_err()
            );
            assert!(state.accept(&Record::BranchStart(next.clone())).is_err());
        }
        state
            .accept(&Record::Gap(Gap {
                branch: 7,
                first_hour: 10,
                last_hour: 12,
                reason: "spool I/O failure".into(),
            }))
            .unwrap();
        state.accept(&Record::BranchStart(next.clone())).unwrap();
        let mut dependent = frame(100, false);
        dependent.branch = 8;
        assert!(state.accept(&Record::Frame(dependent.clone())).is_err());
        next.id = 9;
        assert!(state.accept(&Record::BranchStart(next)).is_err());
        assert!(
            state
                .accept(&Record::Detach {
                    session: 3,
                    last_hour: None,
                })
                .is_err()
        );
        dependent.keyframe = true;
        state.accept(&Record::Frame(dependent)).unwrap();
    }

    #[test]
    fn two_loads_during_disk_outage_preserve_fresh_and_resumed_branch_gaps() {
        use crate::{
            container::{Replay, SeekResult},
            identity::{Identity, Verification},
        };

        for resumed in [false, true] {
            let session = Record::SessionStart(SessionStart {
                id: 3,
                process_id: 12,
                process_start_time: 34,
                qpc_frequency: 10_000_000,
                identity: Identity {
                    game_version: "test".into(),
                    exe_hash: [1; 32],
                    exe_size: 1,
                    vanilla_hash: [2; 32],
                    dlc: Vec::new(),
                    mods: Vec::new(),
                    coverage_policy: 1,
                    language: "l_english".into(),
                    recorder_version: "test".into(),
                    collector_hash: [3; 32],
                    verification: Verification::Unchecked,
                },
            });
            let start = |id, first_hour, reason| {
                Record::BranchStart(BranchStart {
                    id,
                    session: 3,
                    loaded_hour: Some(first_hour - 1),
                    first_hour,
                    parent: Parentage::Unknown,
                    reason,
                })
            };
            let lost = |branch, hour| {
                Record::Gap(Gap {
                    branch,
                    first_hour: hour,
                    last_hour: hour,
                    reason: "spool I/O failure".into(),
                })
            };
            let backend = MemoryBackend::default();
            let shared = backend.0.clone();
            let mut spool = SegmentedSpool::new(backend);
            let context = [session.clone(), branch()];
            let failed_hour = if resumed {
                spool.start_segment(&context, &frame(10, true)).unwrap();
                let length = shared.borrow().files[&0].len();
                shared.borrow_mut().fault =
                    Some(Fault::WriteAfter(length + RECORD_HEADER_BYTES + 5));
                assert!(spool.append(&Record::Frame(frame(11, false))).is_err());
                11
            } else {
                let length: usize = context
                    .iter()
                    .map(|record| encode_record(record).unwrap().len())
                    .sum();
                shared.borrow_mut().fault =
                    Some(Fault::WriteAfter(length + RECORD_HEADER_BYTES + 5));
                assert!(spool.start_segment(&context, &frame(10, true)).is_err());
                10
            };
            let abandoned = shared.borrow().files[&0].clone();
            let mut context = context.to_vec();
            context.push(lost(7, failed_hour));
            context.push(start(8, 100, BranchReason::LoadSignal));
            let mut second = frame(100, true);
            second.branch = 8;
            shared.borrow_mut().fault = Some(Fault::Create);
            assert!(spool.start_segment(&context, &second).is_err());
            context.push(lost(8, 100));
            context.push(start(9, 200, BranchReason::LoadSignal));
            let mut third = frame(200, true);
            third.branch = 9;
            shared.borrow_mut().fault = None;
            spool.start_segment(&context, &third).unwrap();
            spool.finish_segment().unwrap();

            assert_eq!(spool.next_segment_index(), 3);
            assert_eq!(shared.borrow().files[&0], abandoned);
            assert!(!shared.borrow().files.contains_key(&1));
            let recovered = recover_prefix(&shared.borrow().files[&2]);
            assert!(recovered.damage.is_none());
            context.push(Record::Frame(third.clone()));
            assert_eq!(recovered.records, context);
            let earlier = recover_prefix(&abandoned);
            let replay =
                Replay::from_recovered(b"campaign".to_vec(), &[(0, earlier), (2, recovered)])
                    .unwrap();
            assert_eq!(
                replay
                    .branches
                    .iter()
                    .map(|branch| branch.id)
                    .collect::<Vec<_>>(),
                [7, 8, 9]
            );
            assert_eq!(replay.gaps.len(), 2);
            for (branch, hour) in [(7, failed_hour), (8, 100)] {
                assert!(
                    matches!(replay.seek(branch, hour).unwrap(), SeekResult::Gap { reason, .. }
                        if reason == "spool I/O failure")
                );
            }
            assert!(matches!(replay.seek(9, 200).unwrap(), SeekResult::Frame(_)));
            assert_eq!(replay.index.len(), 1 + usize::from(resumed));
            if resumed {
                assert!(matches!(replay.seek(7, 10).unwrap(), SeekResult::Frame(_)));
            }
        }
    }

    #[test]
    fn every_record_flushes_and_pause_timeout_and_detach_sync() {
        let backend = MemoryBackend::default();
        let shared = backend.0.clone();
        let mut spool = SegmentedSpool::new(backend);
        spool.start_segment(&[branch()], &frame(10, true)).unwrap();
        assert_eq!(shared.borrow().syncs, 1);
        let flushes = shared.borrow().flushes;
        spool.append(&Record::Frame(frame(11, false))).unwrap();
        assert_eq!(shared.borrow().flushes, flushes + 1);
        spool.sync_if_due().unwrap();
        assert_eq!(shared.borrow().syncs, 1);
        spool.last_sync = Instant::now() - SYNC_INTERVAL;
        spool.sync_if_due().unwrap();
        assert_eq!(shared.borrow().syncs, 2);
        spool
            .append(&Record::Detach {
                session: 3,
                last_hour: Some(11),
            })
            .unwrap();
        assert_eq!(shared.borrow().syncs, 3);
        assert!(spool.append(&Record::Frame(frame(12, false))).is_err());
    }
}
