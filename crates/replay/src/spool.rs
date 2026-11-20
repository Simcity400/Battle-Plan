//! Uncompressed, independently recoverable spool segments.

use crate::identity::IdentityCodec;
use std::io::{self, Read, Seek, SeekFrom};
#[cfg(any(test, feature = "journal"))]
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, PoisonError},
    time::{Duration, Instant},
};

use crate::{
    BranchId, Hour,
    branches::{BranchStart, Gap},
    codec::{Decoder, invalid, invalid_static, put_string},
    format::FormatVersion,
    frame::{Frame, Metrics, TableUpdate},
    identity::SessionStart,
    rows::{TABLE_COUNT, TableId, compatibility::RecordKind},
};
#[cfg(any(test, feature = "journal"))]
use engine_allocation::allocation_error;

use replay_frame::envelope::seal_record;
pub use replay_frame::envelope::{MAX_RECORD_BYTES, RECORD_HEADER_BYTES, RECORD_MAGIC};
#[cfg(any(test, feature = "journal"))]
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
    PlayerCommands {
        session: u64,
        packet: crate::commands::Packet,
    },
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
            Self::PlayerCommands { packet, .. } => (packet.encoded_bytes()?, None),
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

    /// The record kind whose layout it is written in.
    pub const fn kind(&self) -> RecordKind {
        match self {
            Self::PlayerCommands { .. } => RecordKind::PlayerCommands,
            Self::Detach { .. } => RecordKind::Detach,
            Self::ClockWitness { .. } => RecordKind::ClockWitness,
            Self::GapSummary { .. } => RecordKind::GapSummary,
            Self::Partial { .. } => RecordKind::Partial,
            Self::EventLoss { .. } => RecordKind::EventLoss,
        }
    }

    pub const fn session(&self) -> u64 {
        match self {
            Self::PlayerCommands { session, .. } => *session,
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
            Self::PlayerCommands { session, packet } => {
                out.push(9);
                out.extend_from_slice(&session.to_le_bytes());
                packet.encode(out)?;
            }
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
        if tag == 9 {
            format.require(RecordKind::PlayerCommands)?;
            return Ok(Self::PlayerCommands {
                session: input.u64()?,
                packet: crate::commands::Packet::decode(input)?,
            });
        }
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
        input.format = format.version();
        Ok(match input.u8()? {
            0 => Self::SessionStart(SessionStart::decode(input)?),
            1 => Self::BranchStart(BranchStart::decode(input)?),
            // Recovery checks the envelope's recording version before
            // decoding this payload.
            2 => Self::Frame(Frame::decode_version(input, None, format)?),
            3 => Self::Gap(Gap::decode(input)?),
            tag @ 4..=9 => Self::Control(ControlRecord::decode_tag(tag, input, format)?),
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

pub fn decode_record(bytes: &[u8]) -> io::Result<Record> {
    let mut envelopes = Envelopes::new(io::Cursor::new(bytes), bytes.len());
    let envelope = envelopes
        .read(Vec::new())?
        .ok_or_else(|| invalid_static("spool record is empty"))?;
    let record = envelope.record()?;
    if envelopes.read(Vec::new())?.is_some() {
        return Err(invalid_static("expected one spool record"));
    }
    Ok(record)
}

/// This checks segment context, not table baselines, which belong to FrameDecoder.
#[derive(Clone, Debug, Default)]
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
    #[cfg(any(test, feature = "journal"))]
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
            RecordRef::Control(ControlRecord::PlayerCommands { session, packet }) => {
                self.require_session(*session)?;
                if packet.world == 0 {
                    return Err(invalid_static("player commands lack world context"));
                }
                if let Some(phase) = &packet.phase {
                    phase.validate()?;
                    if phase.start.session != *session || phase.start.id != packet.branch {
                        return Err(invalid_static(
                            "command phase has another branch or session",
                        ));
                    }
                }
            }
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
        crate::packing::workers(),
    )
}

/// The frame recovered at `placement`, refused unless its record still has that length and checksum.
pub fn read_frame_at(
    input: &mut (impl Read + Seek),
    placement: Placement,
    length: usize,
) -> io::Result<Frame> {
    read_frame_payload_at(input, placement, length)?.decode()
}

/// An immutable recovered envelope; checksum and frame decoding need no file lock.
pub struct FramePayload(Envelope);

impl FramePayload {
    pub fn decode(self) -> io::Result<Frame> {
        match self.0.record()? {
            Record::Frame(frame) => Ok(frame),
            _ => Err(invalid_static("recovered spool frame is missing")),
        }
    }
}

/// Copy just the held record's bytes while its seek cursor is locked.
pub fn read_frame_payload_at(
    input: &mut (impl Read + Seek),
    placement: Placement,
    length: usize,
) -> io::Result<FramePayload> {
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
        return Err(invalid_static(
            "spool record changed since it was recovered",
        ));
    }
    Ok(FramePayload(envelope))
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

fn recover_records<T: SpoolRecord + Send>(
    input: impl Read,
    length: usize,
    keep: impl Fn(Record, Placement) -> T + Sync,
    observe: impl FnMut(&T) -> io::Result<()>,
    fail_resources: bool,
    threads: usize,
) -> io::Result<Recovery<T>> {
    let mut tail = Tail::default();
    let failure = tail.extend(input, length, keep, observe, threads);
    tail.recovery(length, failure, fail_resources)
}

/// A segment's records recovered so far, continued as the segment grows.
#[derive(Clone)]
pub struct Tail<T> {
    records: Vec<T>,
    accepted: Accepted,
    /// Each record's placement and format, in order: finishing checks the file still holds them.
    placements: crc32fast::Hasher,
}

impl<T> Default for Tail<T> {
    fn default() -> Self {
        Self {
            records: Vec::new(),
            accepted: Accepted::default(),
            placements: crc32fast::Hasher::new(),
        }
    }
}

fn fold_placement(hasher: &mut crc32fast::Hasher, placement: Placement, format: FormatVersion) {
    hasher.update(&(placement.offset as u64).to_le_bytes());
    hasher.update(&(placement.length as u64).to_le_bytes());
    hasher.update(&placement.crc.to_le_bytes());
    hasher.update(&format.version().to_le_bytes());
}

impl<T: SpoolRecord + Send> Tail<T> {
    pub fn records(&self) -> &[T] {
        &self.records
    }

    pub fn valid_bytes(&self) -> usize {
        self.accepted.valid_bytes
    }

    /// Reads, on this thread, the records now whole and valid in the first `length` bytes; a failing one waits.
    pub fn grow(
        &mut self,
        input: &mut (impl Read + Seek),
        length: usize,
        keep: impl Fn(Record, Placement) -> T + Sync,
    ) -> io::Result<()> {
        input.seek(SeekFrom::Start(self.accepted.valid_bytes as u64))?;
        match self.extend(input, length, keep, |_| Ok(()), 1) {
            Some((_, error))
                if !matches!(
                    error.kind(),
                    io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
                ) =>
            {
                Err(error)
            }
            _ => Ok(()),
        }
    }

    /// What [`recover_reader`] recovers from the first `length` bytes; none if this tail read past them.
    pub fn finish(
        mut self,
        input: &mut (impl Read + Seek),
        length: usize,
        keep: impl Fn(Record, Placement) -> T + Sync,
        mut observe: impl FnMut(&T) -> io::Result<()>,
    ) -> io::Result<Option<Recovery<T>>> {
        if self.accepted.valid_bytes > length || !self.still_held(input)? {
            return Ok(None);
        }
        for record in &self.records {
            observe(record)?;
        }
        input.seek(SeekFrom::Start(self.accepted.valid_bytes as u64))?;
        let failure = self.extend(
            input,
            length,
            keep,
            observe,
            crate::packing::workers(),
        );
        self.recovery(length, failure, true).map(Some)
    }

    /// Whether each record keeps the offset, length and checksum its payload was verified against here.
    fn still_held(&self, input: &mut (impl Read + Seek)) -> io::Result<bool> {
        let mut placements = crc32fast::Hasher::new();
        let mut offset = 0;
        while offset < self.accepted.valid_bytes {
            input.seek(SeekFrom::Start(offset as u64))?;
            let mut header = [0; RECORD_HEADER_BYTES];
            if input.read_exact(&mut header).is_err() || header[..4] != RECORD_MAGIC {
                return Ok(false);
            }
            let Ok(format) =
                FormatVersion::read(u16::from_le_bytes(header[4..6].try_into().unwrap()))
            else {
                return Ok(false);
            };
            let placement = Placement {
                offset,
                length: u32::from_le_bytes(header[6..10].try_into().unwrap()) as usize,
                crc: u32::from_le_bytes(header[10..14].try_into().unwrap()),
            };
            fold_placement(&mut placements, placement, format);
            offset += RECORD_HEADER_BYTES + placement.length;
        }
        Ok(offset == self.accepted.valid_bytes
            && placements.finalize() == self.placements.clone().finalize())
    }

    /// Decodes on `threads` threads but accepts in order, finding damage where a sequential read does.
    fn extend(
        &mut self,
        input: impl Read,
        length: usize,
        keep: impl Fn(Record, Placement) -> T + Sync,
        mut observe: impl FnMut(&T) -> io::Result<()>,
        threads: usize,
    ) -> Option<(usize, io::Error)> {
        let mut envelopes = Envelopes {
            input,
            length,
            read_bytes: self.accepted.valid_bytes,
            format: self.accepted.format,
        };
        let (records, accepted, placements) =
            (&mut self.records, &mut self.accepted, &mut self.placements);
        let mut failure = None;
        let mut ended = false;
        crate::packing::ordered(
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
                            let mut next = accepted.clone();
                            next.accept(&kept.view(), placed)?;
                            observe(&kept)?;
                            engine_allocation::reserve(records, 1)?;
                            *accepted = next;
                            fold_placement(placements, placed.placement, placed.format);
                            Ok(kept)
                        }),
                    ),
                };
                match kept {
                    Ok(kept) => {
                        records.push(kept);
                        true
                    }
                    Err(error) => {
                        failure = Some((offset, error));
                        false
                    }
                }
            },
        );
        failure
    }

    fn recovery(
        self,
        length: usize,
        failure: Option<(usize, io::Error)>,
        fail_resources: bool,
    ) -> io::Result<Recovery<T>> {
        let accepted = self.accepted;
        let mut result = Recovery {
            format: accepted.format,
            records: self.records,
            valid_bytes: accepted.valid_bytes,
            damage: None,
        };
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

#[derive(Clone, Default)]
struct Accepted {
    valid_bytes: usize,
    format: Option<FormatVersion>,
    state: SegmentState,
    session_context_only: bool,
}

impl Accepted {
    fn accept(&mut self, record: &RecordRef<'_>, placed: Placed) -> io::Result<()> {
        self.state.accept_view(record)?;
        self.session_context_only = (self.session_context_only
            || matches!(record, RecordRef::SessionStart(_)))
            && record.frame().is_none();
        self.format = Some(placed.format);
        self.valid_bytes = placed.end;
        Ok(())
    }

    fn finished(&self) -> io::Result<()> {
        // A frameless checkpoint tail needs its own session context; bare branches still need frames or loss.
        if self.state.needs_keyframe
            && !self.state.partial
            && !self.state.gap_before_keyframe
            && !self.session_context_only
        {
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

#[cfg(any(test, feature = "journal"))]
mod writer;
#[cfg(any(test, feature = "journal"))]
pub use writer::*;
