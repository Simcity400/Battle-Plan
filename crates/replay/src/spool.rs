//! Uncompressed, independently recoverable spool segments.

use crate::identity::IdentityCodec;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, PoisonError},
    time::{Duration, Instant},
};

use crate::{
    BranchId, Hour,
    branches::{BranchStart, Gap},
    codec::{Decoder, invalid, invalid_static, put_string},
    format::FormatVersion,
    frame::{EncodedFrame, Frame, Metrics, TableUpdate},
    identity::SessionStart,
    rows::{TABLE_COUNT, TableId, compatibility::RecordKind},
};
use engine_allocation::allocation_error;

use replay_frame::envelope::seal_record;
pub use replay_frame::envelope::{MAX_RECORD_BYTES, RECORD_HEADER_BYTES, RECORD_MAGIC};
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
    /// The verified capture prefix is readable, but the capture ended in failure.
    Partial {
        session: u64,
        reason: String,
        failed_hour: Option<i32>,
    },
    /// The same process session reattached and continued `branch`: the
    /// collector was detached for part of the interval between the read of
    /// `after_hour`, the session's last recorded frame before it detached,
    /// and the read of `hour`, the reattached session's first observation.
    /// Every event family's record of that interval is incomplete; events it
    /// did record there remain observed. The two hours are read anchors, not
    /// the times the hooks were removed and installed. The frame at
    /// `hour` may itself be lost, which a gap records separately. A writer
    /// that retries the record after a failed write can repeat it; the
    /// replay keeps one.
    EventLoss {
        session: u64,
        branch: BranchId,
        after_hour: i32,
        hour: i32,
    },
}

impl ControlRecord {
    fn encoded_bytes(&self) -> io::Result<usize> {
        let tagged_session = size_of::<u8>() + size_of::<u64>();
        let (fields, reason) = match self {
            Self::Detach { last_hour, .. } => (
                size_of::<u8>() + usize::from(last_hour.is_some()) * size_of::<i32>(),
                None,
            ),
            Self::ClockWitness { load_signal, .. } => (
                size_of::<u64>()
                    + size_of::<i32>()
                    + size_of::<u8>()
                    + usize::from(load_signal.is_some()) * size_of::<i64>(),
                None,
            ),
            Self::GapSummary { reason, .. } => (size_of::<u64>(), Some(reason)),
            Self::Partial {
                reason,
                failed_hour,
                ..
            } => (
                size_of::<u8>() + usize::from(failed_hour.is_some()) * size_of::<i32>(),
                Some(reason),
            ),
            Self::EventLoss { .. } => (size_of::<BranchId>() + 2 * size_of::<i32>(), None),
        };
        engine_allocation::add(
            tagged_session + fields + usize::from(reason.is_some()) * size_of::<u32>(),
            reason.map_or(0, String::len),
        )
    }

    pub const fn session(&self) -> u64 {
        match self {
            Self::Detach { session, .. }
            | Self::ClockWitness { session, .. }
            | Self::GapSummary { session, .. }
            | Self::Partial { session, .. }
            | Self::EventLoss { session, .. } => *session,
        }
    }

    /// These tags are shared by spool payloads and the replay trailer.
    pub fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        engine_allocation::reserve(out, self.encoded_bytes()?)?;
        match self {
            Self::EventLoss {
                session,
                branch,
                after_hour,
                hour,
            } => {
                if after_hour >= hour {
                    return Err(invalid_static("event loss must end after it begins"));
                }
                out.push(8);
                out.extend_from_slice(&session.to_le_bytes());
                out.extend_from_slice(&branch.to_le_bytes());
                out.extend_from_slice(&after_hour.to_le_bytes());
                out.extend_from_slice(&hour.to_le_bytes());
            }
            Self::Partial {
                session,
                reason,
                failed_hour,
            } => {
                if reason.trim().is_empty() {
                    return Err(invalid_static("partial capture needs a reason"));
                }
                out.push(7);
                out.extend_from_slice(&session.to_le_bytes());
                put_string(out, reason)?;
                out.push(u8::from(failed_hour.is_some()));
                if let Some(hour) = failed_hour {
                    out.extend_from_slice(&hour.to_le_bytes());
                }
            }
            Self::GapSummary {
                session,
                dropped_frames,
                reason,
            } => {
                if *dropped_frames == 0 || reason.trim().is_empty() {
                    return Err(invalid_static("gap summary needs a count and reason"));
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
        Self::decode_version(input, FormatVersion::CURRENT)
    }

    pub(crate) fn decode_version(
        input: &mut Decoder<'_>,
        format: FormatVersion,
    ) -> io::Result<Self> {
        Self::decode_tag(input.u8()?, input, format)
    }

    fn decode_tag(tag: u8, input: &mut Decoder<'_>, format: FormatVersion) -> io::Result<Self> {
        format.require(match tag {
            4 => RecordKind::Detach,
            5 => RecordKind::ClockWitness,
            6 => RecordKind::GapSummary,
            7 => RecordKind::Partial,
            8 => RecordKind::EventLoss,
            _ => return Err(invalid("unknown control record tag")),
        })?;
        Ok(match tag {
            8 => {
                let (session, branch) = (input.u64()?, input.u64()?);
                let (after_hour, hour) = (input.i32()?, input.i32()?);
                if after_hour >= hour {
                    return Err(invalid("event loss must end after it begins"));
                }
                Self::EventLoss {
                    session,
                    branch,
                    after_hour,
                    hour,
                }
            }
            7 => {
                let session = input.u64()?;
                let reason = input.string()?;
                if reason.trim().is_empty() {
                    return Err(invalid("partial capture needs a reason"));
                }
                Self::Partial {
                    session,
                    reason,
                    failed_hour: if input.bool()? {
                        Some(input.i32()?)
                    } else {
                        None
                    },
                }
            }
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
        engine_allocation::reserve(out, 1)?;
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
        Self::decode_version(input, FormatVersion::CURRENT)
    }

    fn decode_version(input: &mut Decoder<'_>, format: FormatVersion) -> io::Result<Self> {
        Ok(match input.u8()? {
            0 => Self::SessionStart(SessionStart::decode(input)?),
            1 => Self::BranchStart(BranchStart::decode(input)?),
            // Recovery checks the envelope's recording version before
            // decoding this payload.
            2 => Self::Frame(Frame::decode_version(input, None, format)?),
            3 => Self::Gap(Gap::decode(input)?),
            tag @ 4..=8 => Self::Control(ControlRecord::decode_tag(tag, input, format)?),
            _ => return Err(invalid("unknown spool record tag")),
        })
    }
}

pub fn encode_record(record: &Record) -> io::Result<Vec<u8>> {
    let payload = match record {
        Record::Control(control) => control.encoded_bytes()?,
        Record::BranchStart(branch) => size_of::<u8>() + branch.encoded_bytes(),
        Record::Gap(gap) => engine_allocation::add(size_of::<u8>(), gap.encoded_bytes()?)?,
        Record::SessionStart(session) => {
            engine_allocation::add(size_of::<u8>(), session.encoded_bytes()?)?
        }
        Record::Frame(_) => size_of::<u8>(),
    };
    let capacity = RECORD_HEADER_BYTES
        .checked_add(payload)
        .ok_or_else(engine_allocation::oom)?;
    let mut out = engine_allocation::vec(capacity)?;
    out.resize(RECORD_HEADER_BYTES, 0);
    record.encode(&mut out)?;
    seal_record(out)
}

fn encode_frame_record(frame: EncodedFrame) -> io::Result<Vec<u8>> {
    frame.into_spool_record()
}

/// This checks segment context, not table baselines, which belong to FrameDecoder.
#[derive(Debug, Default)]
pub(crate) struct SegmentState {
    records: usize,
    session: Option<u64>,
    branch: Option<u64>,
    branches: Vec<u64>,
    first_hour: Option<i32>,
    last_hour: Option<i32>,
    needs_keyframe: bool,
    gap_before_keyframe: bool,
    detached: bool,
    partial: bool,
}

impl SegmentState {
    pub(crate) fn accept(&mut self, record: &Record) -> io::Result<()> {
        self.accept_view(&record.view())
    }

    pub(crate) fn accept_view(&mut self, record: &RecordRef<'_>) -> io::Result<()> {
        if self.detached {
            return Err(invalid_static("record after detach"));
        }
        if self.records == 0
            && !matches!(
                record,
                RecordRef::SessionStart(_) | RecordRef::BranchStart(_)
            )
        {
            return Err(invalid_static(
                "segment does not begin with session or branch context",
            ));
        }
        match record {
            RecordRef::SessionStart(session) => {
                if self.records != 0 {
                    return Err(invalid_static("session start occurs inside a segment"));
                }
                self.session = Some(session.id);
                self.needs_keyframe = true;
            }
            RecordRef::BranchStart(branch) => {
                if self.branch.is_some() && self.needs_keyframe && !self.gap_before_keyframe {
                    return Err(invalid_static(
                        "branch has no initial keyframe or explicit gap",
                    ));
                }
                self.require_session(branch.session)?;
                if self.branches.contains(&branch.id) {
                    return Err(invalid_static("repeated branch start inside a segment"));
                }
                engine_allocation::reserve(&mut self.branches, 1)?;
                self.branches.push(branch.id);
                self.session = Some(branch.session);
                self.branch = Some(branch.id);
                self.first_hour = Some(branch.first_hour);
                self.last_hour = None;
                self.needs_keyframe = true;
                self.gap_before_keyframe = false;
            }
            RecordRef::Frame(_) | RecordRef::Metadata(_) => {
                let (branch, hour, keyframe) = record.frame().unwrap();
                return self.accept_frame(branch, hour, keyframe);
            }
            RecordRef::Gap(gap) => {
                if self.branch != Some(gap.branch) {
                    return Err(invalid_static("gap lacks matching branch context"));
                }
                if gap.first_hour > gap.last_hour || gap.reason.is_empty() {
                    return Err(invalid_static("invalid gap range or empty reason"));
                }
                if self.first_hour.is_some_and(|hour| gap.first_hour < hour) {
                    return Err(invalid_static("gap precedes declared first branch hour"));
                }
                // Disk recovery may preserve only gaps for a branch before a load.
                // Its first hour may already have a frame in an earlier segment.
                self.gap_before_keyframe |= self.needs_keyframe;
            }
            RecordRef::Control(ControlRecord::ClockWitness { session, .. })
            | RecordRef::Control(ControlRecord::GapSummary { session, .. }) => {
                self.require_session(*session)?
            }
            RecordRef::Control(ControlRecord::Partial {
                session, reason, ..
            }) => {
                self.require_session(*session)?;
                if self.partial || reason.trim().is_empty() {
                    return Err(invalid_static("duplicate or invalid partial capture"));
                }
                self.partial = true;
            }
            RecordRef::Control(ControlRecord::EventLoss {
                session,
                branch,
                after_hour,
                hour,
            }) => {
                self.require_session(*session)?;
                if self.branch != Some(*branch) {
                    return Err(invalid_static("event loss lacks matching branch context"));
                }
                if after_hour >= hour {
                    return Err(invalid_static("event loss must end after it begins"));
                }
            }
            RecordRef::Control(ControlRecord::Detach { session, last_hour }) => {
                self.require_session(*session)?;
                if self.needs_keyframe && !self.partial {
                    return Err(invalid_static("detach before initial keyframe"));
                }
                if last_hour.is_some() && *last_hour != self.last_hour {
                    return Err(invalid_static(
                        "detach hour differs from last recorded frame",
                    ));
                }
                self.detached = true;
            }
        }
        self.records += 1;
        Ok(())
    }

    fn accept_frame(&mut self, branch: u64, hour: i32, keyframe: bool) -> io::Result<()> {
        if self.detached || self.partial {
            return Err(invalid_static("record after detach"));
        }
        if self.records == 0 {
            return Err(invalid_static(
                "segment does not begin with session or branch context",
            ));
        }
        if self.branch != Some(branch) {
            return Err(invalid_static("frame lacks matching branch context"));
        }
        if self.needs_keyframe && !keyframe {
            return Err(invalid_static(
                "branch or segment does not start with a keyframe",
            ));
        }
        if self.first_hour.is_some_and(|first| hour < first) {
            return Err(invalid_static("frame precedes declared first branch hour"));
        }
        if self.last_hour.is_some_and(|last| hour <= last) {
            return Err(invalid_static("frame hours do not increase within branch"));
        }
        self.last_hour = Some(hour);
        self.needs_keyframe = false;
        self.records += 1;
        Ok(())
    }

    fn require_session(&self, session: u64) -> io::Result<()> {
        if self.session.is_some_and(|expected| expected != session) {
            Err(invalid_static("record belongs to another session"))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug)]
pub struct Recovery<T = Record> {
    /// Original envelope contract; never discarded by recovery/finalization.
    pub format: Option<FormatVersion>,
    pub records: Vec<T>,
    pub valid_bytes: usize,
    pub damage: Option<String>,
}

/// Where a record lies in its segment: its header's first byte, its payload's length and checksum.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Placement {
    pub offset: usize,
    pub length: usize,
    pub crc: u32,
}

/// A recovered frame without the rows finalization's checks skip; it is read again by placement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameMetadata {
    pub branch: BranchId,
    pub hour: Hour,
    pub keyframe: bool,
    pub placement: Placement,
    pub campaign: TableUpdate,
    pub clock: TableUpdate,
}

impl FrameMetadata {
    pub fn of(mut frame: Frame, placement: Placement) -> Self {
        let hour = frame.hour;
        let mut take = |table: TableId| {
            std::mem::replace(
                &mut frame.tables[table.index()],
                TableUpdate::Unchanged { baseline: hour },
            )
        };
        let (campaign, clock) = (take(TableId::Campaign), take(TableId::Clock));
        Self {
            branch: frame.branch,
            hour,
            keyframe: frame.keyframe,
            placement,
            campaign,
            clock,
        }
    }

    /// Every table but the kept ones unavailable.
    pub fn frame(&self) -> io::Result<Frame> {
        let mut tables = engine_allocation::vec(TABLE_COUNT)?;
        for table in TableId::ALL {
            tables.push(match table {
                TableId::Campaign => self.campaign.clone(),
                TableId::Clock => self.clock.clone(),
                _ => TableUpdate::Unavailable {
                    reason: engine_allocation::string("finalization metadata only")?,
                },
            });
        }
        Ok(Frame {
            branch: self.branch,
            hour: self.hour,
            keyframe: self.keyframe,
            metrics: Metrics::default(),
            tables: tables.try_into().unwrap(),
        })
    }
}

/// A recovered record, its frame kept as metadata so a long spool fits in memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recovered {
    SessionStart(SessionStart),
    BranchStart(BranchStart),
    Frame(FrameMetadata),
    Gap(Gap),
    Control(ControlRecord),
}

impl Recovered {
    pub fn keep(record: Record, placement: Placement) -> Self {
        match record {
            Record::SessionStart(session) => Self::SessionStart(session),
            Record::BranchStart(branch) => Self::BranchStart(branch),
            Record::Frame(frame) => Self::Frame(FrameMetadata::of(frame, placement)),
            Record::Gap(gap) => Self::Gap(gap),
            Record::Control(control) => Self::Control(control),
        }
    }
}

#[derive(Clone, Copy)]
pub enum RecordRef<'a> {
    SessionStart(&'a SessionStart),
    BranchStart(&'a BranchStart),
    Frame(&'a Frame),
    Metadata(&'a FrameMetadata),
    Gap(&'a Gap),
    Control(&'a ControlRecord),
}

impl RecordRef<'_> {
    pub fn frame(&self) -> Option<(BranchId, Hour, bool)> {
        match self {
            Self::Frame(frame) => Some((frame.branch, frame.hour, frame.keyframe)),
            Self::Metadata(frame) => Some((frame.branch, frame.hour, frame.keyframe)),
            _ => None,
        }
    }
}

pub trait SpoolRecord: Sync {
    fn view(&self) -> RecordRef<'_>;
}

impl SpoolRecord for Record {
    fn view(&self) -> RecordRef<'_> {
        match self {
            Self::SessionStart(session) => RecordRef::SessionStart(session),
            Self::BranchStart(branch) => RecordRef::BranchStart(branch),
            Self::Frame(frame) => RecordRef::Frame(frame),
            Self::Gap(gap) => RecordRef::Gap(gap),
            Self::Control(control) => RecordRef::Control(control),
        }
    }
}

impl SpoolRecord for Recovered {
    fn view(&self) -> RecordRef<'_> {
        match self {
            Self::SessionStart(session) => RecordRef::SessionStart(session),
            Self::BranchStart(branch) => RecordRef::BranchStart(branch),
            Self::Frame(metadata) => RecordRef::Metadata(metadata),
            Self::Gap(gap) => RecordRef::Gap(gap),
            Self::Control(control) => RecordRef::Control(control),
        }
    }
}

/// Recover only this segment's valid prefix. Never scan past damage for another magic.
#[cfg(any(test, feature = "test-support"))]
pub fn recover_prefix(bytes: &[u8]) -> Recovery {
    recover_records(
        io::Cursor::new(bytes),
        bytes.len(),
        |record, _| record,
        |_| Ok(()),
        false,
        1,
    )
    .unwrap()
}

/// `keep` runs on the pack's threads; allocation and I/O failures abort rather than become damage.
pub fn recover_reader<T: SpoolRecord + Send>(
    input: impl Read,
    length: usize,
    keep: impl Fn(Record, Placement) -> T + Sync,
    observe: impl FnMut(&T) -> io::Result<()>,
) -> io::Result<Recovery<T>> {
    recover_records(
        input,
        length,
        keep,
        observe,
        true,
        crate::container::pack::workers(),
    )
}

/// The frame recovered at `placement`, refused unless its record still has that length and checksum.
pub fn read_frame_at(
    input: &mut (impl Read + Seek),
    placement: Placement,
    length: usize,
) -> io::Result<Frame> {
    let missing = || invalid_static("recovered spool frame is missing");
    if placement.offset >= length {
        return Err(missing());
    }
    input.seek(SeekFrom::Start(placement.offset as u64))?;
    let mut envelopes = Envelopes {
        input,
        length,
        read_bytes: placement.offset,
        format: None,
    };
    let envelope = envelopes.read(Vec::new())?.ok_or_else(missing)?;
    if envelope.crc != placement.crc || envelope.payload.len() != placement.length {
        return Err(invalid_static("spool record changed since it was recovered"));
    }
    match envelope.record()? {
        Record::Frame(frame) => Ok(frame),
        _ => Err(missing()),
    }
}

struct Envelope {
    offset: usize,
    format: FormatVersion,
    crc: u32,
    payload: Vec<u8>,
}

impl Envelope {
    fn record(&self) -> io::Result<Record> {
        if crc32fast::hash(&self.payload) != self.crc {
            return Err(invalid("record CRC mismatch"));
        }
        let mut input = Decoder::new(&self.payload);
        let record = Record::decode_version(&mut input, self.format)?;
        input.finish()?;
        Ok(record)
    }

    fn placed(&self) -> Placed {
        Placed {
            placement: Placement {
                offset: self.offset,
                length: self.payload.len(),
                crc: self.crc,
            },
            end: self.offset + RECORD_HEADER_BYTES + self.payload.len(),
            format: self.format,
        }
    }

    fn end(&self) -> usize {
        self.placed().end
    }
}

#[derive(Clone, Copy)]
struct Placed {
    placement: Placement,
    end: usize,
    format: FormatVersion,
}

/// Decodes on `threads` threads but accepts in order, finding damage where a sequential read does.
fn recover_records<T: SpoolRecord + Send>(
    input: impl Read,
    length: usize,
    keep: impl Fn(Record, Placement) -> T + Sync,
    mut observe: impl FnMut(&T) -> io::Result<()>,
    fail_resources: bool,
    threads: usize,
) -> io::Result<Recovery<T>> {
    let mut result = Recovery {
        format: None,
        records: Vec::new(),
        valid_bytes: 0,
        damage: None,
    };
    let mut envelopes = Envelopes::new(input, length);
    let mut accepted = Accepted::default();
    let mut failure = None;
    let mut ended = false;
    crate::container::pack::ordered(
        threads,
        || {
            if ended {
                return None;
            }
            let offset = envelopes.read_bytes;
            let envelope = envelopes.read(Vec::new());
            ended = !matches!(envelope, Ok(Some(_)));
            envelope.map_err(|error| (offset, error)).transpose()
        },
        |envelope| {
            envelope.map(|envelope| {
                let placed = envelope.placed();
                let record = envelope.record();
                drop(envelope);
                (record.map(|record| keep(record, placed.placement)), placed)
            })
        },
        |read| {
            let (offset, kept) = match read {
                Err((offset, error)) => (offset, Err(error)),
                Ok((record, placed)) => (
                    placed.placement.offset,
                    record.and_then(|kept| {
                        accepted.accept(&kept.view(), placed)?;
                        observe(&kept)?;
                        engine_allocation::reserve(&mut result.records, 1)?;
                        Ok(kept)
                    }),
                ),
            };
            match kept {
                Ok(kept) => {
                    result.records.push(kept);
                    result.valid_bytes = accepted.valid_bytes;
                    result.format = accepted.format;
                    true
                }
                Err(error) => {
                    failure = Some((offset, error));
                    false
                }
            }
        },
    );
    let Some((offset, error)) = failure.or_else(|| {
        accepted
            .finished()
            .err()
            .map(|error| (accepted.valid_bytes, error))
    }) else {
        return Ok(result);
    };
    if fail_resources
        && !matches!(
            error.kind(),
            io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
        )
    {
        return Err(error);
    }
    result.damage = Some(
        if offset == length && accepted.state.needs_keyframe && !accepted.state.partial {
            "segment ended before initial keyframe".into()
        } else {
            format!("segment byte {offset}: {error}")
        },
    );
    Ok(result)
}

struct Envelopes<R> {
    input: R,
    length: usize,
    read_bytes: usize,
    format: Option<FormatVersion>,
}

impl<R: Read> Envelopes<R> {
    fn new(input: R, length: usize) -> Self {
        Self {
            input,
            length,
            read_bytes: 0,
            format: None,
        }
    }

    fn read(&mut self, mut payload: Vec<u8>) -> io::Result<Option<Envelope>> {
        let remaining = self.length - self.read_bytes;
        if remaining == 0 {
            return Ok(None);
        }
        if remaining < RECORD_HEADER_BYTES {
            return Err(invalid("truncated record header"));
        }
        let mut header = [0; RECORD_HEADER_BYTES];
        self.input.read_exact(&mut header)?;
        if header[..4] != RECORD_MAGIC {
            return Err(invalid(format!(
                "unsupported spool record magic {:?}; expected {:?}",
                String::from_utf8_lossy(&header[..4]),
                String::from_utf8_lossy(&RECORD_MAGIC),
            )));
        }
        let format = FormatVersion::read(u16::from_le_bytes(header[4..6].try_into().unwrap()))?;
        format.require(RecordKind::SpoolEnvelope)?;
        if self.format.is_some_and(|first| first != format) {
            return Err(invalid("SpoolEnvelope format changed inside a segment"));
        }
        let length = u32::from_le_bytes(header[6..10].try_into().unwrap()) as usize;
        if length == 0 || length > MAX_RECORD_BYTES {
            return Err(invalid("invalid record length"));
        }
        if length > remaining - RECORD_HEADER_BYTES {
            return Err(invalid("truncated record payload"));
        }
        engine_allocation::resize(&mut payload, length, 0)?;
        self.input.read_exact(&mut payload)?;
        let envelope = Envelope {
            offset: self.read_bytes,
            format,
            crc: u32::from_le_bytes(header[10..14].try_into().unwrap()),
            payload,
        };
        self.format = Some(format);
        self.read_bytes = envelope.end();
        Ok(Some(envelope))
    }
}

#[derive(Default)]
struct Accepted {
    valid_bytes: usize,
    format: Option<FormatVersion>,
    state: SegmentState,
}

impl Accepted {
    fn accept(&mut self, record: &RecordRef<'_>, placed: Placed) -> io::Result<()> {
        self.state.accept_view(record)?;
        self.format = Some(placed.format);
        self.valid_bytes = placed.end;
        Ok(())
    }

    fn finished(&self) -> io::Result<()> {
        if self.state.needs_keyframe && !self.state.partial {
            return Err(invalid("segment ended before initial keyframe"));
        }
        Ok(())
    }
}

/// A reusable payload buffer and one decoded record, independent of segment size.
pub struct RecordReader<R> {
    envelopes: Envelopes<R>,
    accepted: Accepted,
    payload: Vec<u8>,
}

impl<R: Read> RecordReader<R> {
    pub fn new(input: R, length: usize) -> Self {
        Self {
            envelopes: Envelopes::new(input, length),
            accepted: Accepted::default(),
            payload: Vec::new(),
        }
    }

    pub fn next_record(&mut self) -> io::Result<Option<Record>> {
        let Some(envelope) = self.envelopes.read(std::mem::take(&mut self.payload))? else {
            self.accepted.finished()?;
            return Ok(None);
        };
        let record = envelope.record().and_then(|record| {
            self.accepted
                .accept(&record.view(), envelope.placed())
                .map(|()| record)
        });
        self.payload = envelope.payload;
        record.map(Some)
    }
}

/// Periodic synchronization work for one live segment, excluding forced barriers.
#[derive(Clone, Copy, Debug, Default)]
pub struct BackgroundSyncWork {
    pub requested: u64,
    pub completed: u64,
    pub total_ns: u64,
    pub max_ns: u64,
    pub pending: bool,
    /// Durability threads started for the segment: one persistent worker
    /// serves every periodic request.
    pub threads: u64,
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
        File::sync_all(self).map_err(allocation_error)
    }
}

/// One ordered writer and at most one periodic durability worker per segment.
/// The worker owns a cloned handle, never record bytes or capture buffers.
pub struct FileSink {
    file: File,
    sync_file: File,
    worker: Option<DurabilityWorker>,
    pending: bool,
    work: BackgroundSyncWork,
}

/// The segment's persistent durability thread. It starts at the first
/// periodic request and then sleeps between requests, so periodic
/// synchronization spawns no thread per request. At most one request is in
/// flight; dropping the sink finishes it and joins the thread.
struct DurabilityWorker {
    shared: Arc<DurabilityShared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[derive(Default)]
struct DurabilityShared {
    state: Mutex<DurabilityState>,
    changed: Condvar,
}

#[derive(Default)]
struct DurabilityState {
    requested: bool,
    finished: Option<(u64, io::Result<()>)>,
    stop: bool,
}

impl DurabilityShared {
    fn lock(&self) -> std::sync::MutexGuard<'_, DurabilityState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The worker: one sync per request, until stopped with none pending.
    fn serve(&self, file: File) {
        let mut state = self.lock();
        loop {
            if state.requested {
                state.requested = false;
                drop(state);
                let started = Instant::now();
                let result = file.sync_all();
                let elapsed = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
                state = self.lock();
                state.finished = Some((elapsed, result));
                self.changed.notify_all();
            } else if state.stop {
                return;
            } else {
                state = self
                    .changed
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        }
    }
}

impl DurabilityWorker {
    fn start(file: File) -> io::Result<Self> {
        let shared = Arc::new(DurabilityShared::default());
        let worker = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("replay-durability".into())
            .spawn(move || worker.serve(file))?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    fn request(&self) {
        let mut state = self.shared.lock();
        state.requested = true;
        self.shared.changed.notify_all();
    }

    /// The in-flight request has neither finished nor lost its thread.
    fn busy(&self) -> bool {
        self.shared.lock().finished.is_none()
            && self
                .thread
                .as_ref()
                .is_some_and(|thread| !thread.is_finished())
    }

    /// Wait for the in-flight request. A worker that ended without reporting
    /// one cannot certify durability.
    fn wait(&self) -> (u64, io::Result<()>) {
        let mut state = self.shared.lock();
        loop {
            if let Some(finished) = state.finished.take() {
                return finished;
            }
            if self
                .thread
                .as_ref()
                .is_none_or(|thread| thread.is_finished())
            {
                return (0, Err(io::Error::from(io::ErrorKind::Other)));
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, Duration::from_millis(50))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

impl Drop for DurabilityWorker {
    fn drop(&mut self) {
        self.shared.lock().stop = true;
        self.shared.changed.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
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
        Err(allocation_error(io::Error::last_os_error()))
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
        if !std::mem::take(&mut self.pending) {
            return Ok(());
        }
        let worker = self.worker.as_ref().expect("a pending sync has a worker");
        let (elapsed, result) = worker.wait();
        self.work.completed += 1;
        self.work.total_ns = self.work.total_ns.saturating_add(elapsed);
        self.work.max_ns = self.work.max_ns.max(elapsed);
        result.map_err(allocation_error)
    }
}

impl Write for FileSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.file.write(bytes).map_err(allocation_error)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush().map_err(allocation_error)
    }
}

impl SegmentSink for FileSink {
    fn sync_all(&mut self) -> io::Result<()> {
        self.finish_pending()?;
        // Include writes accepted after the background request began.
        self.file.sync_all().map_err(allocation_error)
    }

    fn start_sync(&mut self) -> io::Result<()> {
        if !self.poll_sync()? {
            return Ok(());
        }
        let worker = match &mut self.worker {
            Some(worker) => worker,
            empty => {
                let file = self.sync_file.try_clone().map_err(allocation_error)?;
                let worker = DurabilityWorker::start(file).map_err(allocation_error)?;
                self.work.threads += 1;
                empty.insert(worker)
            }
        };
        worker.request();
        self.pending = true;
        self.work.requested += 1;
        Ok(())
    }

    fn poll_sync(&mut self) -> io::Result<bool> {
        if self.pending && self.worker.as_ref().is_some_and(DurabilityWorker::busy) {
            return Ok(false);
        }
        self.finish_pending()?;
        Ok(true)
    }

    fn background_sync_work(&self) -> Option<BackgroundSyncWork> {
        Some(BackgroundSyncWork {
            pending: self.pending,
            ..self.work
        })
    }
}

impl Drop for FileSink {
    fn drop(&mut self) {
        // The collector DLL must never unload with its worker still running.
        // Normal close reports failures through sync_all; error cleanup only
        // finishes the request, and dropping the worker joins its thread.
        let _ = self.finish_pending();
        self.worker = None;
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
        fs::create_dir_all(&directory).map_err(allocation_error)?;
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
            .open(self.directory.join(format!("spool.{index:04}.h4s")))
            .map_err(allocation_error)?;
        let sync_file = independent_sync_file(&file)?;
        Ok(FileSink {
            file,
            sync_file,
            worker: None,
            pending: false,
            work: BackgroundSyncWork::default(),
        })
    }
}

pub type SpoolWriteStartHook = fn(Option<(u32, u64)>);

pub struct SegmentedSpool<B: SegmentBackend = FileBackend> {
    backend: B,
    sink: Option<B::Sink>,
    state: SegmentState,
    next_segment: u32,
    last_sync: Instant,
    last_error: Option<io::ErrorKind>,
    committed_segment: Option<u32>,
    committed_bytes: u64,
    current_segment: Option<u32>,
    current_bytes: u64,
    write_start_hook: Option<SpoolWriteStartHook>,
    commit_hook: Option<fn(u32, u64, bool)>,
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
            committed_segment: None,
            committed_bytes: 0,
            current_segment: None,
            current_bytes: 0,
            write_start_hook: None,
            commit_hook: None,
        }
    }

    #[cfg(test)]
    pub fn next_segment_index(&self) -> u32 {
        self.next_segment
    }
    pub fn is_open(&self) -> bool {
        self.sink.is_some()
    }
    pub fn last_error(&self) -> Option<io::ErrorKind> {
        self.last_error
    }

    pub fn committed_watermark(&self) -> Option<(u32, u64)> {
        self.committed_segment
            .map(|segment| (segment, self.committed_bytes))
    }

    /// Publish before periodic sync can allocate; true identifies a complete frame.
    /// The callback must not allocate.
    pub fn set_commit_hook(&mut self, hook: fn(u32, u64, bool)) {
        self.commit_hook = Some(hook);
    }

    /// Publish the preceding campaign watermark before a write can become in flight.
    /// The callback must not allocate.
    pub fn set_write_start_hook(&mut self, hook: SpoolWriteStartHook) {
        self.write_start_hook = Some(hook);
    }

    /// Current open segment only; completed durations exclude forced barriers.
    pub fn background_sync_work(&self) -> Option<BackgroundSyncWork> {
        self.sink.as_ref()?.background_sync_work()
    }

    /// The caller resets its table encoder and supplies a new keyframe on every restart.
    /// Lost hours must be named in `Gap` records in this context or subsequent appends.
    #[cfg(test)]
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
            keyframe.branch(),
            keyframe.hour(),
            keyframe.keyframe(),
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
            return Err(invalid_static(
                "close the current segment before starting another",
            ));
        }
        if context.is_empty() || !keyframe {
            return Err(invalid_static("segment start needs context and a keyframe"));
        }
        let mut state = SegmentState::default();
        let count = context
            .len()
            .checked_add(1)
            .ok_or_else(engine_allocation::oom)?;
        let mut encoded = engine_allocation::vec(count)?;
        for record in context {
            if matches!(
                record,
                Record::Frame(_)
                    | Record::Control(ControlRecord::Detach { .. } | ControlRecord::Partial { .. })
            ) {
                return Err(invalid_static("segment context contains frame or detach"));
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
            .ok_or_else(|| invalid_static("segment index exhausted"))?;
        let sink = match self.backend.create(index) {
            Ok(sink) => sink,
            Err(error) => return self.failed(error),
        };
        self.sink = Some(sink);
        self.current_segment = Some(index);
        self.current_bytes = 0;
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
            return Err(invalid_static(
                "no writable segment; restart with context and a keyframe",
            ));
        }
        let bytes = encode_record(record)?;
        self.state.accept(record)?;
        self.append_bytes(
            bytes,
            matches!(record, Record::Control(ControlRecord::Detach { .. })),
        )
    }

    pub fn append_encoded_frame(&mut self, frame: EncodedFrame) -> io::Result<()> {
        if self.sink.is_none() {
            return Err(invalid_static(
                "no writable segment; restart with context and a keyframe",
            ));
        }
        let (branch, hour, keyframe) = (frame.branch(), frame.hour(), frame.keyframe());
        let bytes = encode_frame_record(frame)?;
        self.state.accept_frame(branch, hour, keyframe)?;
        self.append_bytes(bytes, false)
    }

    fn append_bytes(&mut self, bytes: Vec<u8>, force_sync: bool) -> io::Result<()> {
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
        let preceding = self.committed_watermark();
        let sink = self
            .sink
            .as_mut()
            .ok_or_else(|| invalid_static("no writable segment"))?;
        if let Some(hook) = self.write_start_hook {
            hook(preceding);
        }
        if engine_allocation::capture_stopped() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        sink.write_all(bytes)?;
        sink.flush()?;
        if engine_allocation::capture_stopped() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        self.current_bytes = self.current_bytes.saturating_add(bytes.len() as u64);
        self.committed_segment = self.current_segment;
        self.committed_bytes = self.current_bytes;
        if let Some(hook) = self.commit_hook {
            hook(
                self.committed_segment.unwrap(),
                self.committed_bytes,
                bytes[RECORD_HEADER_BYTES] == 2,
            );
        }
        Ok(())
    }

    fn sync_current(&mut self) -> io::Result<()> {
        let sink = self
            .sink
            .as_mut()
            .ok_or_else(|| invalid_static("no writable segment"))?;
        sink.flush()?;
        sink.sync_all()?;
        self.last_sync = Instant::now();
        Ok(())
    }

    fn failed<T>(&mut self, error: io::Error) -> io::Result<T> {
        // Dropping this handle is final: even a partial record is never appended to again.
        self.sink = None;
        self.state = SegmentState::default();
        self.last_error = Some(error.kind());
        Err(error)
    }
}

#[cfg(test)]
mod tests;
