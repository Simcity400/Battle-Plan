//! Independently compressed blocks, an indexed trailer, and atomic publication.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use crate::{
    BranchId, FORMAT_VERSION, Hour, KEYFRAME_INTERVAL,
    branches::{BranchReason, BranchStart, Gap},
    codec::{Decoder, invalid, put_bytes, put_string},
    frame::{DecodedFrame, Frame, FrameDecoder, TABLE_COUNT, TableView},
    identity::{Identity, SessionStart, Verification},
    rows::{Row, TableId},
    spool::{Record, Recovery, SegmentState},
};

const HEADER_MAGIC: &[u8; 4] = b"H4RP";
const TRAILER_MAGIC: &[u8; 4] = b"H4TR";
const FOOTER_MAGIC: &[u8; 4] = b"H4FT";
const FOOTER_SIZE: usize = 24;
pub const MAX_BLOCK_RAW_BYTES: usize = 1 << 30;

type FrameObserver<'a> =
    dyn FnMut(&Frame, &DecodedFrame, [usize; TABLE_COUNT]) -> io::Result<()> + 'a;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockIndex {
    pub branch: BranchId,
    pub first_hour: Hour,
    pub last_hour: Hour,
    pub offset: u64,
    pub length: u64,
    pub raw_length: u64,
}

#[derive(Debug)]
pub enum SeekResult {
    Frame(Box<DecodedFrame>),
    Gap {
        branch: BranchId,
        hour: Hour,
        reason: String,
    },
}

/// Damage may have no trustworthy hour; keep its segment and byte position instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryNote {
    pub session: Option<u64>,
    pub segment: u32,
    pub valid_bytes: u64,
    pub reason: String,
}

impl RecoveryNote {
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        if self.reason.trim().is_empty() {
            return Err(invalid("empty recovery reason"));
        }
        out.push(u8::from(self.session.is_some()));
        if let Some(session) = self.session {
            out.extend_from_slice(&session.to_le_bytes());
        }
        out.extend_from_slice(&self.segment.to_le_bytes());
        out.extend_from_slice(&self.valid_bytes.to_le_bytes());
        put_string(out, &self.reason)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let session = if input.bool()? {
            Some(input.u64()?)
        } else {
            None
        };
        let note = Self {
            session,
            segment: input.u32()?,
            valid_bytes: input.u64()?,
            reason: input.string()?,
        };
        if note.reason.trim().is_empty() {
            return Err(invalid("empty recovery reason"));
        }
        Ok(note)
    }
}

/// Metadata is parsed when opened; seeking inflates only the selected block.
/// The byte backing can later become a mapping without changing block semantics.
pub struct Replay {
    bytes: Vec<u8>,
    pub campaign_id: Vec<u8>,
    pub identity: Identity,
    pub keyframe_interval: u32,
    pub sessions: Vec<SessionStart>,
    pub branches: Vec<BranchStart>,
    pub gaps: Vec<Gap>,
    pub controls: Vec<Record>,
    pub recovery_notes: Vec<RecoveryNote>,
    pub index: Vec<BlockIndex>,
    header_end: usize,
    trailer_start: usize,
}

struct RawBlock {
    branch: BranchId,
    first_hour: Hour,
    last_hour: Hour,
    frames: Vec<u8>,
    count: u32,
}

impl RawBlock {
    fn new(frame: &Frame) -> Self {
        Self {
            branch: frame.branch,
            first_hour: frame.hour,
            last_hour: frame.hour,
            frames: Vec::new(),
            count: 0,
        }
    }
    fn push(&mut self, frame: &Frame) -> io::Result<[usize; TABLE_COUNT]> {
        let mut payload = Vec::new();
        let sizes = frame.encode_with_sizes(&mut payload)?;
        put_bytes(&mut self.frames, &payload)?;
        if self.frames.len() > MAX_BLOCK_RAW_BYTES - 4 {
            return Err(invalid("replay block exceeds raw size limit"));
        }
        self.count = self
            .count
            .checked_add(1)
            .ok_or_else(|| invalid("too many frames"))?;
        self.last_hour = frame.hour;
        Ok(sizes)
    }
    fn raw(&self) -> Vec<u8> {
        let mut raw = Vec::with_capacity(self.frames.len() + 4);
        raw.extend_from_slice(&self.count.to_le_bytes());
        raw.extend_from_slice(&self.frames);
        raw
    }
}

struct Pending {
    sessions: Vec<SessionStart>,
    branches: Vec<BranchStart>,
    gaps: Vec<Gap>,
    controls: Vec<Record>,
    blocks: Vec<RawBlock>,
}

impl Replay {
    pub fn from_records(campaign_id: Vec<u8>, segments: &[Vec<Record>]) -> io::Result<Self> {
        let segments: Vec<_> = segments.iter().map(Vec::as_slice).collect();
        Self::from_record_slices(campaign_id, &segments, &mut |_, _, _| Ok(()))
    }

    fn from_record_slices(
        campaign_id: Vec<u8>,
        segments: &[&[Record]],
        observe: &mut FrameObserver<'_>,
    ) -> io::Result<Self> {
        if campaign_id.is_empty() {
            return Err(invalid("empty campaign identity"));
        }
        let identity = segments
            .iter()
            .flat_map(|segment| segment.iter())
            .find_map(|record| match record {
                Record::SessionStart(session) => Some(session.identity.clone()),
                _ => None,
            })
            .ok_or_else(|| invalid("replay has no session identity"))?;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(HEADER_MAGIC);
        bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&KEYFRAME_INTERVAL.to_le_bytes());
        put_bytes(&mut bytes, &campaign_id)?;
        let mut encoded_identity = Vec::new();
        identity.encode(&mut encoded_identity)?;
        put_bytes(&mut bytes, &encoded_identity)?;
        let pending = collect(
            segments,
            &identity,
            &campaign_id,
            None,
            KEYFRAME_INTERVAL,
            observe,
        )?;
        let mut index = Vec::new();
        append_blocks(&mut bytes, &mut index, &pending.blocks)?;
        finish_file(
            &mut bytes,
            &pending.sessions,
            &pending.branches,
            &pending.gaps,
            &pending.controls,
            &[],
            &index,
        )?;
        Self::open(bytes)
    }

    /// Existing compressed blocks and their original offsets are copied verbatim.
    pub fn append(&self, segments: &[Vec<Record>]) -> io::Result<Self> {
        let segments: Vec<_> = segments.iter().map(Vec::as_slice).collect();
        self.append_record_slices(&segments, &mut |_, _, _| Ok(()))
    }

    fn append_record_slices(
        &self,
        segments: &[&[Record]],
        observe: &mut FrameObserver<'_>,
    ) -> io::Result<Self> {
        let pending = collect(
            segments,
            &self.identity,
            &self.campaign_id,
            Some(self),
            self.keyframe_interval,
            observe,
        )?;
        let mut bytes = self.bytes[..self.trailer_start].to_vec();
        let mut index = self.index.clone();
        append_blocks(&mut bytes, &mut index, &pending.blocks)?;
        let mut sessions = self.sessions.clone();
        sessions.extend(pending.sessions);
        let mut branches = self.branches.clone();
        branches.extend(pending.branches);
        let mut gaps = self.gaps.clone();
        gaps.extend(pending.gaps);
        let mut controls = self.controls.clone();
        controls.extend(pending.controls);
        finish_file(
            &mut bytes,
            &sessions,
            &branches,
            &gaps,
            &controls,
            &self.recovery_notes,
            &index,
        )?;
        Self::open(bytes)
    }

    pub fn from_recovered(campaign_id: Vec<u8>, segments: &[(u32, Recovery)]) -> io::Result<Self> {
        Self::from_recovered_with(campaign_id, segments, |_, _, _| Ok(()))
    }

    /// Report each frame using its validated decoded view and the sizes produced
    /// by the block encoder. Observation does not publish the file; callers must
    /// report success only after the returned replay has been written atomically.
    pub fn from_recovered_with(
        campaign_id: Vec<u8>,
        segments: &[(u32, Recovery)],
        mut observe: impl FnMut(&Frame, &DecodedFrame, [usize; TABLE_COUNT]) -> io::Result<()>,
    ) -> io::Result<Self> {
        let records = ordered_records(segments)?;
        Self::from_record_slices(campaign_id, &records, &mut observe)?.add_recovery_notes(segments)
    }

    pub fn append_recovered(&self, segments: &[(u32, Recovery)]) -> io::Result<Self> {
        self.append_recovered_with(segments, |_, _, _| Ok(()))
    }

    /// Observe only the new frames; existing compressed blocks are copied intact.
    pub fn append_recovered_with(
        &self,
        segments: &[(u32, Recovery)],
        mut observe: impl FnMut(&Frame, &DecodedFrame, [usize; TABLE_COUNT]) -> io::Result<()>,
    ) -> io::Result<Self> {
        let records = ordered_records(segments)?;
        self.append_record_slices(&records, &mut observe)?
            .add_recovery_notes(segments)
    }

    fn add_recovery_notes(mut self, segments: &[(u32, Recovery)]) -> io::Result<Self> {
        let mut sorted: Vec<_> = segments.iter().collect();
        sorted.sort_unstable_by_key(|(number, _)| *number);
        for (number, recovery) in sorted {
            let session = recovery.records.first().and_then(|record| match record {
                Record::SessionStart(session) => Some(session.id),
                Record::BranchStart(branch) => Some(branch.session),
                _ => None,
            });
            if let Some(reason) = &recovery.damage {
                self.recovery_notes.push(RecoveryNote {
                    session,
                    segment: *number,
                    valid_bytes: recovery.valid_bytes as u64,
                    reason: reason.clone(),
                });
            }
        }
        self.bytes.truncate(self.trailer_start);
        finish_file(
            &mut self.bytes,
            &self.sessions,
            &self.branches,
            &self.gaps,
            &self.controls,
            &self.recovery_notes,
            &self.index,
        )?;
        Self::open(self.bytes)
    }

    pub fn content_verified(&self) -> bool {
        matches!(self.identity.verification, Verification::Verified { .. })
            && self.sessions.iter().all(|session| {
                matches!(session.identity.verification, Verification::Verified { .. })
            })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Exact recoverable frame hours, never the index's range with holes filled.
    pub fn recorded_hours(&self, branch: BranchId) -> io::Result<Vec<Hour>> {
        let mut hours = Vec::new();
        for block in self.index.iter().filter(|block| block.branch == branch) {
            hours.extend(self.read_block(block)?.into_iter().map(|frame| frame.hour));
        }
        Ok(hours)
    }

    pub fn open(bytes: Vec<u8>) -> io::Result<Self> {
        if bytes.len() < FOOTER_SIZE {
            return Err(invalid("missing replay footer"));
        }
        let footer_start = bytes.len() - FOOTER_SIZE;
        let mut footer = Decoder::new(&bytes[footer_start..]);
        expect_magic(&mut footer, FOOTER_MAGIC)?;
        let trailer_start = to_usize(footer.u64()?)?;
        let trailer_length = to_usize(footer.u64()?)?;
        let trailer_crc = footer.u32()?;
        footer.finish()?;
        if trailer_start.checked_add(trailer_length) != Some(footer_start) {
            return Err(invalid("invalid trailer extent"));
        }
        let trailer_bytes = bytes
            .get(trailer_start..footer_start)
            .ok_or_else(|| invalid("trailer outside replay"))?;
        if crc32fast::hash(trailer_bytes) != trailer_crc {
            return Err(invalid("trailer CRC mismatch"));
        }
        let mut header = Decoder::new(&bytes[..trailer_start]);
        expect_magic(&mut header, HEADER_MAGIC)?;
        let version = header.u16()?;
        if version != FORMAT_VERSION {
            return Err(invalid(format!(
                "unsupported replay format version {version}; expected {FORMAT_VERSION}"
            )));
        }
        let keyframe_interval = header.u32()?;
        if keyframe_interval != KEYFRAME_INTERVAL {
            return Err(invalid("unsupported keyframe interval"));
        }
        let campaign_id = header.bytes()?;
        if campaign_id.is_empty() {
            return Err(invalid("empty campaign identity"));
        }
        let identity_bytes = header.bytes()?;
        let mut identity_decoder = Decoder::new(&identity_bytes);
        let identity = Identity::decode(&mut identity_decoder)?;
        identity_decoder.finish()?;
        let header_end = trailer_start - header.remaining();
        let mut trailer = Decoder::new(trailer_bytes);
        expect_magic(&mut trailer, TRAILER_MAGIC)?;
        let sessions = decode_list(&mut trailer, SessionStart::decode)?;
        let branches = decode_list(&mut trailer, BranchStart::decode)?;
        let gaps = decode_list(&mut trailer, Gap::decode)?;
        let controls = decode_list(&mut trailer, Record::decode)?;
        let recovery_notes = decode_list(&mut trailer, RecoveryNote::decode)?;
        let count = trailer.count(40)?;
        let mut index = Vec::with_capacity(count);
        for _ in 0..count {
            index.push(BlockIndex {
                branch: trailer.u64()?,
                first_hour: trailer.i32()?,
                last_hour: trailer.i32()?,
                offset: trailer.u64()?,
                length: trailer.u64()?,
                raw_length: trailer.u64()?,
            });
        }
        trailer.finish()?;
        validate_metadata(
            &identity,
            &sessions,
            &branches,
            &gaps,
            &controls,
            &index,
            header_end,
            trailer_start,
            keyframe_interval,
        )?;
        Ok(Self {
            bytes,
            campaign_id,
            identity,
            keyframe_interval,
            sessions,
            branches,
            gaps,
            controls,
            recovery_notes,
            index,
            header_end,
            trailer_start,
        })
    }

    pub fn seek(&self, branch: BranchId, hour: Hour) -> io::Result<SeekResult> {
        let gap = |reason| SeekResult::Gap {
            branch,
            hour,
            reason,
        };
        if !self.branches.iter().any(|entry| entry.id == branch) {
            return Ok(gap("unknown branch".into()));
        }
        let Some(index) = self.index.iter().find(|index| {
            index.branch == branch && index.first_hour <= hour && hour <= index.last_hour
        }) else {
            return Ok(gap(self.gap_reason(branch, hour)));
        };
        let frames = match self.read_block(index) {
            Ok(frames) => frames,
            Err(error) => return Ok(gap(format!("unreadable block: {error}"))),
        };
        let mut decoder = FrameDecoder::new();
        for frame in frames {
            if frame.hour > hour {
                break;
            }
            let decoded = match decoder.apply(&frame) {
                Ok(decoded) => decoded,
                Err(error) => {
                    return Ok(gap(format!(
                        "invalid frame baseline at hour {}: {error}",
                        frame.hour
                    )));
                }
            };
            if let Err(error) = validate_campaign(&decoded, &self.campaign_id) {
                return Ok(gap(error.to_string()));
            }
            if frame.hour == hour {
                return Ok(SeekResult::Frame(Box::new(decoded)));
            }
        }
        Ok(gap(self.gap_reason(branch, hour)))
    }

    fn gap_reason(&self, branch: BranchId, hour: Hour) -> String {
        let reason = self
            .gaps
            .iter()
            .filter(|gap| gap.branch == branch && gap.first_hour <= hour && hour <= gap.last_hour)
            .map(|gap| gap.reason.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        if reason.is_empty() {
            "hour was not recorded".into()
        } else {
            reason
        }
    }

    /// Decode one block for sequential playback, inspection, or history recovery.
    pub fn read_block(&self, index: &BlockIndex) -> io::Result<Vec<Frame>> {
        if index.raw_length > MAX_BLOCK_RAW_BYTES as u64 {
            return Err(invalid("block exceeds raw size limit"));
        }
        let start = to_usize(index.offset)?;
        let end = to_usize(
            index
                .offset
                .checked_add(index.length)
                .ok_or_else(|| invalid("block offset overflow"))?,
        )?;
        let compressed = self
            .bytes
            .get(start..end)
            .ok_or_else(|| invalid("block outside replay"))?;
        if compressed.get(..4) != Some(&[0x28, 0xb5, 0x2f, 0xfd]) {
            return Err(invalid("block is not a zstd frame"));
        }
        let frame_size = zstd::zstd_safe::find_frame_compressed_size(compressed)
            .map_err(|_| invalid("invalid zstd frame"))?;
        if frame_size != compressed.len() {
            return Err(invalid(
                "block contains trailing or concatenated zstd frames",
            ));
        }
        let mut decompressor = zstd::stream::read::Decoder::with_buffer(compressed)?.single_frame();
        decompressor.window_log_max(26)?;
        let mut raw = Vec::new();
        decompressor
            .take(index.raw_length + 1)
            .read_to_end(&mut raw)?;
        if raw.len() as u64 != index.raw_length {
            return Err(invalid("block raw length mismatch"));
        }
        let mut decoder = Decoder::new(&raw);
        let count = decoder.count(4)?;
        if count == 0 {
            return Err(invalid("empty block"));
        }
        let mut frames = Vec::with_capacity(count);
        let mut last = None;
        for position in 0..count {
            let payload = decoder.bytes()?;
            let mut frame_decoder = Decoder::new(&payload);
            let frame = Frame::decode(&mut frame_decoder)?;
            frame_decoder.finish()?;
            if frame.branch != index.branch
                || frame.keyframe != (position == 0)
                || last.is_some_and(|hour| frame.hour <= hour)
            {
                return Err(invalid(
                    "block frame branch, keyframe, or hour ordering mismatch",
                ));
            }
            last = Some(frame.hour);
            frames.push(frame);
        }
        decoder.finish()?;
        if frames[0].hour != index.first_hour || last != Some(index.last_hour) {
            return Err(invalid("block index hour range mismatch"));
        }
        Ok(frames)
    }

    pub fn write_atomic(&self, destination: &Path) -> io::Result<()> {
        self.write_atomic_with(destination, |_| Ok(()))
    }

    /// Fault checkpoints happen before replacement; failure leaves the destination untouched.
    pub fn write_atomic_with(
        &self,
        destination: &Path,
        mut checkpoint: impl FnMut(PublishStage) -> io::Result<()>,
    ) -> io::Result<()> {
        self.publish_with(
            &mut FilePublisher {
                temporary: temporary_path(destination),
                destination: destination.into(),
            },
            &mut checkpoint,
        )
    }

    pub fn publish_with(
        &self,
        publisher: &mut impl AtomicPublisher,
        mut checkpoint: impl FnMut(PublishStage) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut file = publisher.create()?;
        checkpoint(PublishStage::TempCreated)?;
        file.write_all(&self.bytes[..self.header_end])?;
        checkpoint(PublishStage::HeaderWritten)?;
        for (position, block) in self.index.iter().enumerate() {
            let start = to_usize(block.offset)?;
            let end = to_usize(block.offset + block.length)?;
            let midpoint = start + (end - start) / 2;
            file.write_all(&self.bytes[start..midpoint])?;
            checkpoint(PublishStage::BlockPartWritten(position))?;
            file.write_all(&self.bytes[midpoint..end])?;
            checkpoint(PublishStage::BlockWritten(position))?;
        }
        let footer_start = self.bytes.len() - FOOTER_SIZE;
        file.write_all(&self.bytes[self.trailer_start..footer_start])?;
        checkpoint(PublishStage::TrailerWritten)?;
        file.write_all(&self.bytes[footer_start..])?;
        checkpoint(PublishStage::FooterWritten)?;
        publisher.sync(&mut file)?;
        checkpoint(PublishStage::Flushed)?;
        checkpoint(PublishStage::BeforeReplace)?;
        publisher.replace(file)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishStage {
    TempCreated,
    HeaderWritten,
    BlockPartWritten(usize),
    BlockWritten(usize),
    TrailerWritten,
    FooterWritten,
    Flushed,
    BeforeReplace,
}

/// The same publication sequence runs against files and pure failure-injection writers.
pub trait AtomicPublisher {
    type Writer: Write;
    fn create(&mut self) -> io::Result<Self::Writer>;
    fn sync(&mut self, writer: &mut Self::Writer) -> io::Result<()>;
    fn replace(&mut self, writer: Self::Writer) -> io::Result<()>;
}

struct FilePublisher {
    temporary: PathBuf,
    destination: PathBuf,
}
impl AtomicPublisher for FilePublisher {
    type Writer = File;
    fn create(&mut self) -> io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.temporary)
    }
    fn sync(&mut self, writer: &mut File) -> io::Result<()> {
        writer.sync_all()
    }
    fn replace(&mut self, writer: File) -> io::Result<()> {
        drop(writer);
        replace_file(&self.temporary, &self.destination)
    }
}

fn temporary_path(destination: &Path) -> PathBuf {
    let mut name = destination.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // The source is a closed, fully synced sibling file on the same volume.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    std::fs::rename(source, destination)?;
    if let Some(parent) = destination.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn expect_magic(decoder: &mut Decoder<'_>, expected: &[u8; 4]) -> io::Result<()> {
    if decoder.take(4)? == expected {
        Ok(())
    } else {
        Err(invalid("invalid replay magic"))
    }
}
fn to_usize(value: u64) -> io::Result<usize> {
    usize::try_from(value).map_err(|_| invalid("file extent exceeds platform address space"))
}
fn put_count(out: &mut Vec<u8>, count: usize) -> io::Result<()> {
    out.extend_from_slice(
        &u32::try_from(count)
            .map_err(|_| invalid("too many metadata entries"))?
            .to_le_bytes(),
    );
    Ok(())
}
fn encode_list<T>(
    out: &mut Vec<u8>,
    values: &[T],
    encode: impl Fn(&T, &mut Vec<u8>) -> io::Result<()>,
) -> io::Result<()> {
    put_count(out, values.len())?;
    for value in values {
        let mut payload = Vec::new();
        encode(value, &mut payload)?;
        put_bytes(out, &payload)?;
    }
    Ok(())
}
fn decode_list<T>(
    decoder: &mut Decoder<'_>,
    decode: impl Fn(&mut Decoder<'_>) -> io::Result<T>,
) -> io::Result<Vec<T>> {
    let count = decoder.count(4)?;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let payload = decoder.bytes()?;
        let mut item = Decoder::new(&payload);
        values.push(decode(&mut item)?);
        item.finish()?;
    }
    Ok(values)
}

fn append_blocks(
    bytes: &mut Vec<u8>,
    index: &mut Vec<BlockIndex>,
    blocks: &[RawBlock],
) -> io::Result<()> {
    if blocks.is_empty() {
        return Ok(());
    }
    let workers = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(blocks.len());
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|worker| {
                scope.spawn(move || -> io::Result<Vec<(usize, Vec<u8>)>> {
                    let mut output = Vec::new();
                    for position in (worker..blocks.len()).step_by(workers) {
                        let raw = blocks[position].raw();
                        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3)?;
                        encoder.include_checksum(true)?;
                        encoder.write_all(&raw)?;
                        output.push((position, encoder.finish()?));
                    }
                    Ok(output)
                })
            })
            .collect();
        let mut output = Vec::with_capacity(blocks.len());
        for handle in handles {
            output.extend(
                handle
                    .join()
                    .map_err(|_| invalid("block compressor panicked"))??,
            );
        }
        Ok::<_, io::Error>(output)
    })?;
    let mut results = results;
    results.sort_unstable_by_key(|(position, _)| *position);
    for (position, compressed) in results {
        let block = &blocks[position];
        index.push(BlockIndex {
            branch: block.branch,
            first_hour: block.first_hour,
            last_hour: block.last_hour,
            offset: bytes.len() as u64,
            length: compressed.len() as u64,
            raw_length: (block.frames.len() + 4) as u64,
        });
        bytes.extend_from_slice(&compressed);
    }
    Ok(())
}

fn finish_file(
    bytes: &mut Vec<u8>,
    sessions: &[SessionStart],
    branches: &[BranchStart],
    gaps: &[Gap],
    controls: &[Record],
    recovery_notes: &[RecoveryNote],
    index: &[BlockIndex],
) -> io::Result<()> {
    let trailer_start = bytes.len();
    bytes.extend_from_slice(TRAILER_MAGIC);
    encode_list(bytes, sessions, SessionStart::encode)?;
    encode_list(bytes, branches, BranchStart::encode)?;
    encode_list(bytes, gaps, Gap::encode)?;
    encode_list(bytes, controls, Record::encode)?;
    encode_list(bytes, recovery_notes, RecoveryNote::encode)?;
    put_count(bytes, index.len())?;
    for block in index {
        bytes.extend_from_slice(&block.branch.to_le_bytes());
        bytes.extend_from_slice(&block.first_hour.to_le_bytes());
        bytes.extend_from_slice(&block.last_hour.to_le_bytes());
        bytes.extend_from_slice(&block.offset.to_le_bytes());
        bytes.extend_from_slice(&block.length.to_le_bytes());
        bytes.extend_from_slice(&block.raw_length.to_le_bytes());
    }
    let trailer_length = bytes.len() - trailer_start;
    let crc = crc32fast::hash(&bytes[trailer_start..]);
    bytes.extend_from_slice(FOOTER_MAGIC);
    bytes.extend_from_slice(&(trailer_start as u64).to_le_bytes());
    bytes.extend_from_slice(&(trailer_length as u64).to_le_bytes());
    bytes.extend_from_slice(&crc.to_le_bytes());
    Ok(())
}

fn validate_campaign(frame: &DecodedFrame, campaign_id: &[u8]) -> io::Result<()> {
    if let TableView::Complete(rows) = &frame.tables[TableId::Campaign.index()]
        && (rows.len() != 1
            || !matches!(&rows[0], Row::Campaign(campaign) if campaign.id == campaign_id))
    {
        return Err(invalid(
            "frame campaign identity differs from replay header",
        ));
    }
    Ok(())
}

fn ordered_records(segments: &[(u32, Recovery)]) -> io::Result<Vec<&[Record]>> {
    let mut sorted: Vec<_> = segments.iter().collect();
    sorted.sort_unstable_by_key(|(number, _)| *number);
    if sorted.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(invalid("duplicate spool segment number"));
    }
    Ok(sorted
        .into_iter()
        .map(|(_, recovery)| recovery.records.as_slice())
        .collect())
}

fn collect(
    segments: &[&[Record]],
    identity: &Identity,
    campaign_id: &[u8],
    existing: Option<&Replay>,
    interval: u32,
    observe: &mut FrameObserver<'_>,
) -> io::Result<Pending> {
    identity.validate()?;
    let mut sessions: BTreeMap<_, _> = existing
        .into_iter()
        .flat_map(|replay| &replay.sessions)
        .map(|session| (session.id, session.clone()))
        .collect();
    let mut branches: BTreeMap<_, _> = existing
        .into_iter()
        .flat_map(|replay| &replay.branches)
        .map(|branch| (branch.id, branch.clone()))
        .collect();
    let mut last_hours: BTreeMap<_, _> = existing
        .into_iter()
        .flat_map(|replay| &replay.index)
        .map(|block| (block.branch, block.last_hour))
        .collect();
    let mut pending = Pending {
        sessions: Vec::new(),
        branches: Vec::new(),
        gaps: Vec::new(),
        controls: Vec::new(),
        blocks: Vec::new(),
    };
    let mut block: Option<RawBlock> = None;
    for segment in segments {
        if segment.is_empty() {
            continue;
        }
        if !matches!(segment[0], Record::SessionStart(_) | Record::BranchStart(_)) {
            return Err(invalid(
                "segment does not begin with session or branch context",
            ));
        }
        let mut current_session = None;
        let mut current_branch = None;
        let mut needs_keyframe = true;
        let mut frame_decoder = FrameDecoder::new();
        let mut segment_state = SegmentState::default();
        for record in *segment {
            segment_state.accept(record)?;
            match record {
                Record::SessionStart(session) => {
                    if !session.identity.same_content(identity) {
                        return Err(invalid("session content differs from replay content"));
                    }
                    if let Some(previous) = sessions.get(&session.id) {
                        if previous != session {
                            return Err(invalid("conflicting session context"));
                        }
                    } else {
                        session.validate()?;
                        sessions.insert(session.id, session.clone());
                        pending.sessions.push(session.clone());
                    }
                    if let Some(previous) = block.take() {
                        pending.blocks.push(previous);
                    }
                    current_session = Some(session.id);
                    current_branch = None;
                    needs_keyframe = true;
                }
                Record::BranchStart(branch) => {
                    branch.validate()?;
                    if !sessions.contains_key(&branch.session) {
                        return Err(invalid("branch refers to an unknown session"));
                    }
                    if current_session.is_some_and(|session| session != branch.session) {
                        return Err(invalid("branch changed session without session context"));
                    }
                    if let Some(previous) = branches.get(&branch.id) {
                        if previous != branch {
                            return Err(invalid("conflicting branch context"));
                        }
                    } else {
                        branches.insert(branch.id, branch.clone());
                        pending.branches.push(branch.clone());
                    }
                    if let Some(previous) = block.take() {
                        pending.blocks.push(previous);
                    }
                    current_session = Some(branch.session);
                    current_branch = Some(branch.id);
                    needs_keyframe = true;
                    frame_decoder = FrameDecoder::new();
                }
                Record::Frame(frame) => {
                    if current_branch != Some(frame.branch) {
                        return Err(invalid("frame has no matching branch context"));
                    }
                    if needs_keyframe && !frame.keyframe {
                        return Err(invalid("segment or branch starts without keyframe"));
                    }
                    if last_hours
                        .get(&frame.branch)
                        .is_some_and(|hour| frame.hour <= *hour)
                    {
                        return Err(invalid("branch frame hour did not increase"));
                    }
                    if frame.hour < branches[&frame.branch].first_hour {
                        return Err(invalid("frame precedes its branch start"));
                    }
                    if frame.keyframe {
                        if let Some(previous) = block.take() {
                            pending.blocks.push(previous);
                        }
                        block = Some(RawBlock::new(frame));
                    }
                    let block = block
                        .as_mut()
                        .ok_or_else(|| invalid("frame has no block keyframe"))?;
                    if i64::from(frame.hour) - i64::from(block.first_hour) >= i64::from(interval) {
                        return Err(invalid("block exceeds its keyframe hour interval"));
                    }
                    let decoded = frame_decoder.apply(frame)?;
                    validate_campaign(&decoded, campaign_id)?;
                    let sizes = block.push(frame)?;
                    observe(frame, &decoded, sizes)?;
                    last_hours.insert(frame.branch, frame.hour);
                    needs_keyframe = false;
                }
                Record::Gap(gap) => {
                    gap.validate()?;
                    if current_branch != Some(gap.branch) {
                        return Err(invalid("gap has no matching branch context"));
                    }
                    pending.gaps.push(gap.clone());
                }
                Record::Detach { session, .. }
                | Record::ClockWitness { session, .. }
                | Record::GapSummary { session, .. } => {
                    if current_session != Some(*session) {
                        return Err(invalid("control has no matching session context"));
                    }
                    pending.controls.push(record.clone());
                }
            }
        }
        if let Some(previous) = block.take() {
            pending.blocks.push(previous);
        }
    }
    // A complete CRC-valid record can survive a failed flush. Preserve its frame
    // and the reported write-loss diagnostic; seeking gives the recovered frame priority.
    Ok(pending)
}

#[allow(clippy::too_many_arguments)]
fn validate_metadata(
    identity: &Identity,
    sessions: &[SessionStart],
    branches: &[BranchStart],
    gaps: &[Gap],
    controls: &[Record],
    index: &[BlockIndex],
    header_end: usize,
    trailer_start: usize,
    interval: u32,
) -> io::Result<()> {
    if sessions.is_empty() {
        return Err(invalid("replay has no sessions"));
    }
    let mut session_ids = BTreeSet::new();
    for session in sessions {
        if !session_ids.insert(session.id) || !session.identity.same_content(identity) {
            return Err(invalid("duplicate session or conflicting content"));
        }
    }
    let mut branch_ids = BTreeMap::new();
    let mut sessions_with_start_branch = BTreeSet::new();
    for branch in branches {
        branch.validate()?;
        if !session_ids.contains(&branch.session) || branch_ids.contains_key(&branch.id) {
            return Err(invalid("unknown session or duplicate branch"));
        }
        if branch.reason == BranchReason::SessionStart
            && !sessions_with_start_branch.insert(branch.session)
        {
            return Err(invalid("session has multiple process-start branches"));
        }
        // Parentage is the collector's evidence at branch creation. Recovery may
        // have lost the supporting hour or its entire segment; do not rewrite it.
        branch_ids.insert(branch.id, branch.session);
    }
    for gap in gaps {
        gap.validate()?;
        if !branch_ids.contains_key(&gap.branch) {
            return Err(invalid("gap refers to an unknown branch"));
        }
    }
    for control in controls {
        match control {
            Record::Detach { session, .. }
            | Record::ClockWitness { session, .. }
            | Record::GapSummary { session, .. }
                if session_ids.contains(session) => {}
            _ => return Err(invalid("invalid replay control or unknown session")),
        }
    }
    let mut expected_offset = header_end as u64;
    let mut last_hours = BTreeMap::new();
    let mut closed_branches = BTreeSet::new();
    let mut closed_sessions = BTreeSet::new();
    let mut previous_branch = None;
    let mut previous_session = None;
    for block in index {
        let session = *branch_ids
            .get(&block.branch)
            .ok_or_else(|| invalid("block refers to an unknown branch"))?;
        if previous_branch != Some(block.branch) {
            if let Some(previous) = previous_branch {
                closed_branches.insert(previous);
            }
            if closed_branches.contains(&block.branch) {
                return Err(invalid("branch blocks are not contiguous"));
            }
        }
        if previous_session != Some(session) {
            if let Some(previous) = previous_session {
                closed_sessions.insert(previous);
            }
            if closed_sessions.contains(&session) {
                return Err(invalid("session blocks are not contiguous"));
            }
        }
        previous_branch = Some(block.branch);
        previous_session = Some(session);
        if block.first_hour > block.last_hour
            || i64::from(block.last_hour) - i64::from(block.first_hour) >= i64::from(interval)
            || last_hours
                .get(&block.branch)
                .is_some_and(|hour| block.first_hour <= *hour)
        {
            return Err(invalid("invalid or overlapping block hour ranges"));
        }
        if block.offset != expected_offset
            || block.length == 0
            || block.raw_length < 4
            || block.raw_length > MAX_BLOCK_RAW_BYTES as u64
        {
            return Err(invalid("invalid block extent or raw length"));
        }
        expected_offset = block
            .offset
            .checked_add(block.length)
            .ok_or_else(|| invalid("block offset overflow"))?;
        if expected_offset > trailer_start as u64 {
            return Err(invalid("block overlaps replay trailer"));
        }
        last_hours.insert(block.branch, block.last_hour);
    }
    if expected_offset != trailer_start as u64 {
        return Err(invalid("unindexed bytes before replay trailer"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        branches::Parentage,
        frame::{FrameEncoder, Metrics},
        identity::Roster,
        rows::{CampaignRow, ClockRow},
        spool::{encode_record, recover_prefix},
    };

    const CAMPAIGN: &[u8] = b"container-test-campaign";

    fn session(id: u64) -> SessionStart {
        SessionStart {
            id,
            process_id: id as u32,
            process_start_time: id,
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
                recorder_version: format!("test-{id}"),
                collector_hash: [id as u8; 32],
                verification: Verification::Verified {
                    launcher: Roster::default(),
                    memory: Roster::default(),
                },
            },
        }
    }

    fn branch(id: u64, hour: i32) -> BranchStart {
        BranchStart {
            id,
            session: id,
            loaded_hour: Some(hour - 1),
            first_hour: hour,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        }
    }

    fn source(branch: u64, hour: i32) -> DecodedFrame {
        DecodedFrame {
            branch,
            hour,
            metrics: Metrics::default(),
            tables: [
                TableView::Complete(vec![Row::Clock(ClockRow {
                    hour,
                    ..ClockRow::default()
                })]),
                TableView::Complete(vec![Row::Campaign(CampaignRow {
                    id: CAMPAIGN.into(),
                })]),
                TableView::Complete(Vec::new()),
                TableView::Complete(Vec::new()),
                TableView::Complete(Vec::new()),
                TableView::Complete(Vec::new()),
            ],
        }
    }

    fn records(id: u64, hours: &[i32]) -> Vec<Record> {
        let mut encoder = FrameEncoder::new();
        let mut records = vec![
            Record::SessionStart(session(id)),
            Record::BranchStart(branch(id, hours[0])),
        ];
        for &hour in hours {
            records.push(Record::Frame(
                encoder.encode(&source(id, hour), false).unwrap(),
            ));
        }
        records
    }

    fn rebuild(replay: &Replay, prefix: Vec<u8>, index: &[BlockIndex]) -> Vec<u8> {
        let mut bytes = prefix;
        finish_file(
            &mut bytes,
            &replay.sessions,
            &replay.branches,
            &replay.gaps,
            &replay.controls,
            &replay.recovery_notes,
            index,
        )
        .unwrap();
        bytes
    }

    #[test]
    fn exact_seek_reports_holes_and_append_preserves_blocks_and_session_provenance() {
        let mut first = records(1, &[1, 3, 721]);
        first.insert(
            3,
            Record::Gap(Gap {
                branch: 1,
                first_hour: 2,
                last_hour: 2,
                reason: "queue full".into(),
            }),
        );
        let old = Replay::from_records(CAMPAIGN.into(), &[first]).unwrap();
        assert_eq!(old.index.len(), 2);
        assert!(
            matches!(old.seek(1, 2).unwrap(), SeekResult::Gap { reason, .. } if reason == "queue full")
        );
        assert!(matches!(old.seek(1, 720).unwrap(), SeekResult::Gap { .. }));
        let mut next = records(2, &[722, 723]);
        if let Record::SessionStart(session) = &mut next[0] {
            session.identity.verification = Verification::Unchecked;
        }
        let appended = old.append(&[next]).unwrap();
        assert!(old.content_verified());
        assert!(!appended.content_verified());
        assert_eq!(appended.sessions.len(), 2);
        assert_eq!(appended.sessions[0], old.sessions[0]);
        assert_eq!(
            &appended.bytes()[..old.trailer_start],
            &old.bytes()[..old.trailer_start]
        );
        for entry in &old.index {
            let range = entry.offset as usize..(entry.offset + entry.length) as usize;
            assert_eq!(&old.bytes()[range.clone()], &appended.bytes()[range]);
        }
        assert!(
            matches!(appended.seek(2, 723).unwrap(), SeekResult::Frame(frame) if *frame == source(2, 723))
        );
    }

    #[test]
    fn incompatible_campaign_or_content_cannot_append() {
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1])]).unwrap();
        let mut wrong_campaign = records(2, &[2]);
        if let Record::Frame(frame) = &mut wrong_campaign[2] {
            frame.tables[TableId::Campaign.index()] =
                crate::frame::TableUpdate::Snapshot(vec![Row::Campaign(CampaignRow {
                    id: b"other".into(),
                })]);
        }
        assert!(replay.append(&[wrong_campaign]).is_err());
        let mut wrong_content = records(2, &[2]);
        if let Record::SessionStart(session) = &mut wrong_content[0] {
            session.identity.vanilla_hash[0] ^= 1;
        }
        assert!(replay.append(&[wrong_content]).is_err());
        assert!(matches!(replay.seek(1, 1).unwrap(), SeekResult::Frame(_)));
    }

    #[test]
    fn old_replay_version_is_refused_before_decoding_frames() {
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1])]).unwrap();
        let mut bytes = replay.bytes().to_vec();
        bytes[4..6].copy_from_slice(&1_u16.to_le_bytes());
        let error = Replay::open(bytes).err().unwrap().to_string();
        assert_eq!(error, "unsupported replay format version 1; expected 2");
    }

    #[test]
    fn corrupt_block_does_not_hide_an_unrelated_block() {
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2, 721])]).unwrap();
        let first = &replay.index[0];
        let mut bytes = replay.bytes().to_vec();
        bytes[(first.offset + first.length - 1) as usize] ^= 0x40;
        let damaged = Replay::open(bytes).unwrap();
        assert!(
            matches!(damaged.seek(1, 1).unwrap(), SeekResult::Gap { reason, .. } if reason.contains("unreadable block"))
        );
        assert!(
            matches!(damaged.seek(1, 721).unwrap(), SeekResult::Frame(frame) if *frame == source(1, 721))
        );
    }

    #[test]
    fn hostile_extents_raw_lengths_and_concatenated_frames_are_refused() {
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
        let mut index = replay.index.clone();
        index[0].offset = u64::MAX;
        assert!(
            Replay::open(rebuild(
                &replay,
                replay.bytes()[..replay.trailer_start].into(),
                &index
            ))
            .is_err()
        );
        index = replay.index.clone();
        index[0].raw_length = MAX_BLOCK_RAW_BYTES as u64 + 1;
        assert!(
            Replay::open(rebuild(
                &replay,
                replay.bytes()[..replay.trailer_start].into(),
                &index
            ))
            .is_err()
        );
        index = replay.index.clone();
        index[0].raw_length = 4;
        let short = Replay::open(rebuild(
            &replay,
            replay.bytes()[..replay.trailer_start].into(),
            &index,
        ))
        .unwrap();
        assert!(
            matches!(short.seek(1, 1).unwrap(), SeekResult::Gap { reason, .. } if reason.contains("raw length"))
        );
        index = replay.index.clone();
        let mut prefix = replay.bytes()[..replay.trailer_start].to_vec();
        prefix.extend_from_slice(&replay.bytes()[replay.header_end..replay.trailer_start]);
        index[0].length *= 2;
        let concatenated = Replay::open(rebuild(&replay, prefix, &index)).unwrap();
        assert!(
            matches!(concatenated.seek(1, 1).unwrap(), SeekResult::Gap { reason, .. } if reason.contains("concatenated"))
        );
        index = replay.index.clone();
        index[0].last_hour = 3;
        let wrong_range = Replay::open(rebuild(
            &replay,
            replay.bytes()[..replay.trailer_start].into(),
            &index,
        ))
        .unwrap();
        assert!(
            matches!(wrong_range.seek(1, 3).unwrap(), SeekResult::Gap { reason, .. } if reason.contains("range mismatch"))
        );
        let mut bytes = replay.bytes().to_vec();
        bytes[replay.trailer_start + 5] ^= 1;
        assert!(Replay::open(bytes).is_err());
    }

    #[test]
    fn recovery_retains_tail_damage_without_inventing_an_hour() {
        let records = records(1, &[1, 2]);
        let mut bytes: Vec<_> = records
            .iter()
            .flat_map(|record| encode_record(record).unwrap())
            .collect();
        bytes.pop();
        let recovery = recover_prefix(&bytes);
        let expected = recovery.damage.clone().unwrap();
        let replay = Replay::from_recovered(CAMPAIGN.into(), &[(17, recovery)]).unwrap();
        assert_eq!(replay.recovery_notes.len(), 1);
        assert_eq!(replay.recovery_notes[0].segment, 17);
        assert_eq!(replay.recovery_notes[0].reason, expected);
        assert!(matches!(replay.seek(1, 1).unwrap(), SeekResult::Frame(_)));
        assert!(matches!(replay.seek(1, 2).unwrap(), SeekResult::Gap { .. }));
    }

    #[test]
    fn recovered_frame_wins_over_uncertain_flush_gap_and_parent_claim_survives_damage() {
        let mut first = records(1, &[1, 2]);
        first.push(Record::Gap(Gap {
            branch: 1,
            first_hour: 2,
            last_hour: 2,
            reason: "flush failed after write".into(),
        }));
        let mut second = records(2, &[11]);
        if let Record::BranchStart(branch) = &mut second[1] {
            branch.loaded_hour = Some(10);
            branch.parent = Parentage::Probable {
                branch: 999,
                hour: 10,
            };
            branch.reason = BranchReason::LoadSignal;
        }
        let replay = Replay::from_records(CAMPAIGN.into(), &[first, second]).unwrap();
        assert!(
            matches!(replay.seek(1, 2).unwrap(), SeekResult::Frame(frame) if *frame == source(1, 2))
        );
        assert_eq!(replay.gaps[0].reason, "flush failed after write");
        assert_eq!(
            replay.branches[1].parent,
            Parentage::Probable {
                branch: 999,
                hour: 10
            }
        );
        assert!(matches!(replay.seek(2, 11).unwrap(), SeekResult::Frame(_)));
    }

    #[test]
    fn recovery_orders_segments_numerically_and_rejects_duplicate_numbers() {
        let first = records(1, &[1]);
        let mut later = vec![Record::BranchStart(branch(1, 1))];
        let mut encoder = FrameEncoder::new();
        later.push(Record::Frame(encoder.encode(&source(1, 2), true).unwrap()));
        let recovery = |records| Recovery {
            records,
            valid_bytes: 0,
            damage: None,
        };
        let replay = Replay::from_recovered(
            CAMPAIGN.into(),
            &[(8, recovery(later)), (2, recovery(first.clone()))],
        )
        .unwrap();
        assert!(matches!(replay.seek(1, 2).unwrap(), SeekResult::Frame(_)));
        assert!(
            Replay::from_recovered(
                CAMPAIGN.into(),
                &[(2, recovery(first.clone())), (2, recovery(first))]
            )
            .is_err()
        );
    }

    #[test]
    fn finalization_observer_sees_decoded_failures_and_identical_wire_sizes() {
        let mut input = records(1, &[1, 2, 3]);
        if let Record::Frame(frame) = &mut input[3] {
            frame.tables[TableId::Provinces.index()] =
                crate::frame::TableUpdate::Unchanged { baseline: 999 };
        }
        let segments = [(
            0,
            Recovery {
                records: input,
                valid_bytes: 0,
                damage: None,
            },
        )];
        let mut observed = Vec::new();
        let replay =
            Replay::from_recovered_with(CAMPAIGN.into(), &segments, |frame, decoded, sizes| {
                assert_eq!(sizes, frame.table_encoded_sizes()?);
                observed.push(decoded.clone());
                Ok(())
            })
            .unwrap();
        assert_eq!(
            observed.iter().map(|frame| frame.hour).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert!(
            matches!(&observed[1].tables[TableId::Provinces.index()], TableView::Unavailable { reason } if reason.contains("999"))
        );
        assert!(
            matches!(&observed[2].tables[TableId::Provinces.index()], TableView::Unavailable { reason } if reason.contains("2"))
        );
        for frame in &observed {
            assert!(
                matches!(replay.seek(frame.branch, frame.hour).unwrap(), SeekResult::Frame(decoded) if *decoded == *frame)
            );
        }
        assert_eq!(
            replay.bytes(),
            Replay::from_recovered(CAMPAIGN.into(), &segments)
                .unwrap()
                .bytes()
        );

        let next = [(
            1,
            Recovery {
                records: records(2, &[4, 5]),
                valid_bytes: 0,
                damage: None,
            },
        )];
        let mut appended_hours = Vec::new();
        let appended = replay
            .append_recovered_with(&next, |frame, _, _| {
                appended_hours.push((frame.branch, frame.hour));
                Ok(())
            })
            .unwrap();
        assert_eq!(appended_hours, [(2, 4), (2, 5)]);
        assert_eq!(
            appended.bytes(),
            replay.append_recovered(&next).unwrap().bytes()
        );
        assert!(
            replay
                .append_recovered_with(&next, |_, _, _| Err(io::Error::other("report aborted")))
                .is_err()
        );
    }

    struct MemoryWriter {
        bytes: Vec<u8>,
        fail_after: Option<usize>,
    }
    impl Write for MemoryWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.fail_after == Some(0) {
                return Err(io::Error::other("partial write failure"));
            }
            let count = self
                .fail_after
                .map_or(bytes.len(), |remaining| remaining.min(bytes.len()));
            if let Some(remaining) = &mut self.fail_after {
                *remaining -= count;
            }
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    struct MemoryPublisher {
        existing: Vec<u8>,
        fail_after: Option<usize>,
        fail_create: bool,
        fail_sync: bool,
        fail_replace: bool,
        synced: bool,
    }
    impl MemoryPublisher {
        fn new(existing: &[u8]) -> Self {
            Self {
                existing: existing.into(),
                fail_after: None,
                fail_create: false,
                fail_sync: false,
                fail_replace: false,
                synced: false,
            }
        }
    }
    impl AtomicPublisher for MemoryPublisher {
        type Writer = MemoryWriter;
        fn create(&mut self) -> io::Result<MemoryWriter> {
            if self.fail_create {
                return Err(io::Error::other("create failure"));
            }
            Ok(MemoryWriter {
                bytes: Vec::new(),
                fail_after: self.fail_after,
            })
        }
        fn sync(&mut self, _: &mut MemoryWriter) -> io::Result<()> {
            if self.fail_sync {
                return Err(io::Error::other("sync failure"));
            }
            self.synced = true;
            Ok(())
        }
        fn replace(&mut self, writer: MemoryWriter) -> io::Result<()> {
            assert!(self.synced);
            if self.fail_replace {
                return Err(io::Error::other("replace failure"));
            }
            self.existing = writer.bytes;
            Ok(())
        }
    }

    #[test]
    fn every_interrupted_append_preserves_previous_file_byte_for_byte() {
        let old = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
        let appended = old.append(&[records(2, &[3, 4])]).unwrap();
        let mut stages = Vec::new();
        let mut success = MemoryPublisher::new(old.bytes());
        appended
            .publish_with(&mut success, |stage| {
                stages.push(stage);
                Ok(())
            })
            .unwrap();
        assert_eq!(success.existing, appended.bytes());
        for fail_stage in stages {
            let mut disk = MemoryPublisher::new(old.bytes());
            assert!(
                appended
                    .publish_with(&mut disk, |stage| if stage == fail_stage {
                        Err(io::Error::other("interrupted"))
                    } else {
                        Ok(())
                    })
                    .is_err()
            );
            assert_eq!(disk.existing, old.bytes());
        }
        for fail_after in 0..appended.bytes().len() {
            let mut disk = MemoryPublisher::new(old.bytes());
            disk.fail_after = Some(fail_after);
            assert!(appended.publish_with(&mut disk, |_| Ok(())).is_err());
            assert_eq!(disk.existing, old.bytes());
        }
        for failure in 0..3 {
            let mut disk = MemoryPublisher::new(old.bytes());
            disk.fail_create = failure == 0;
            disk.fail_sync = failure == 1;
            disk.fail_replace = failure == 2;
            assert!(appended.publish_with(&mut disk, |_| Ok(())).is_err());
            assert_eq!(disk.existing, old.bytes());
        }
    }
}
