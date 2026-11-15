//! Campaign recordings made from spool records, for tests: every frame decoded as the row
//! decoder does, written through the collector's frame writer ([`crate::native_frame`]) and
//! the recorder's store into the recording's world log ([`HourWriter`]), saved with the records. And
//! the same records as the journal a collector writes of them ([`write_journal`]), for tests
//! of what reads one.

use super::{Records, Replay};
use crate::{
    BranchId, Hour,
    columnar_file::{Capture, FileWriter, HourWriter, Resumed, Settings},
    columnar_frames::schema,
    columnar_journal::{JournalWriter, Witness},
    format::FormatVersion,
    frame::{DecodedFrame, FrameDecoder, FrameInput, TableInput, TableView},
    identity::IdentityCodec,
    native_frame::FrameWriter,
    rows::TABLE_COUNT,
    spool::{ControlRecord, Record, Recovery, encode_record},
};
use replay_columnar::{
    container::{SectionKey, Tags},
    envelope::{self, Kind, UNIT_ORDINALS},
};
use std::{
    io::{self, Cursor, Read, Seek, Write},
    path::Path,
    sync::Arc,
};

impl Replay {
    /// The recording of `segments`' records, in order, of campaign `campaign_id`.
    pub fn from_records(campaign_id: Vec<u8>, segments: &[Vec<Record>]) -> io::Result<Self> {
        Self::open(encode_records(campaign_id, segments)?)
    }

    /// [`Self::from_records`], and each frame's hour as the writer's store held the derived
    /// tables after it ([`HourWriter::derived_frame`]): what a reader is to read back of them.
    pub fn from_records_derived(campaign_id: Vec<u8>, segments: &[Vec<Record>]) -> io::Result<(Self, Vec<DecodedFrame>)> {
        let mut derived = Vec::new();
        let bytes = encoded(campaign_id, segments, Some(&mut derived))?;
        Ok((Self::open(bytes)?, derived))
    }

    /// Writes a recording opened from memory to `path`, through a sibling renamed into place.
    pub fn write_atomic(&self, path: &std::path::Path) -> io::Result<()> {
        let temporary = path.with_extension("write.tmp");
        std::fs::write(&temporary, self.bytes())?;
        std::fs::rename(&temporary, path)
    }

    /// Every hour of `block` as a frame holding each table whole, as the row decoder applies
    /// them to the same views [`Self::read_block_alone`] builds.
    pub fn read_block(&self, block: &super::BlockIndex) -> io::Result<Vec<crate::frame::Frame>> {
        self.read_block_selected(block, &crate::rows::TableId::ALL)
    }

    /// [`Self::read_block`] of `tables` (and Campaign); every other table unavailable.
    pub fn read_block_selected(
        &self,
        block: &super::BlockIndex,
        tables: &[crate::rows::TableId],
    ) -> io::Result<Vec<crate::frame::Frame>> {
        let mut frames = Vec::new();
        self.read_block_alone(
            block,
            tables,
            |_, decoded| {
                frames.push(whole(decoded, frames.is_empty())?);
                Ok(())
            },
            || Ok(()),
        )?;
        Ok(frames)
    }

    /// [`Self::read_block_selected`], as the row recording's shared frames.
    pub fn read_block_recorded_selected(
        &self,
        block: &super::BlockIndex,
        tables: &[crate::rows::TableId],
    ) -> io::Result<Vec<crate::frame::RecordedFrame>> {
        Ok(self
            .read_block_selected(block, tables)?
            .into_iter()
            .map(crate::frame::RecordedFrame::from)
            .collect())
    }

    /// The recording of recovered spool segments' records, in segment order.
    pub fn from_recovered(campaign_id: Vec<u8>, segments: &[(u32, Recovery<Record>)]) -> io::Result<Self> {
        let mut ordered: Vec<_> = segments.iter().collect();
        ordered.sort_unstable_by_key(|(number, _)| *number);
        let records: Vec<Vec<Record>> = ordered
            .into_iter()
            .map(|(_, recovery)| recovery.records.clone())
            .collect();
        Self::from_records(campaign_id, &records)
    }

    /// The bytes of this recording, opened from memory, with every frame of `block`'s page
    /// damaged mid-frame: its clock and every table's parts and restores. The index still lists
    /// the block and reading its hours fails.
    pub fn damaged_block(&self, block: &super::BlockIndex) -> io::Result<Vec<u8>> {
        let page = self.recording().block(block.run, block.page)?;
        let mut frames = vec![(page.head.clock.offset, u64::from(page.head.clock.stored))];
        for entry in &page.tables {
            frames.extend(unit_frames(&entry.unit));
        }
        Ok(self.damaged(&frames))
    }

    /// [`Self::damaged_block`] of `table`'s parts in `block`'s page alone.
    pub fn damaged_table(&self, block: &super::BlockIndex, table: crate::rows::TableId) -> io::Result<Vec<u8>> {
        let entry = self.recording().table_entry(block.run, block.page, table.index())?;
        let frames: Vec<(u64, u64)> = entry.unit.parts.iter().flat_map(|part| part.frames.iter().map(|frame| (frame.offset, u64::from(frame.stored)))).collect();
        if frames.is_empty() {
            return Err(io::Error::other("the block holds no part of the table"));
        }
        Ok(self.damaged(&frames))
    }

    /// Its bytes with a byte flipped in the middle of each frame (offset, stored length).
    fn damaged(&self, frames: &[(u64, u64)]) -> Vec<u8> {
        let mut bytes = self.bytes().to_vec();
        for &(offset, stored) in frames.iter().filter(|(_, stored)| *stored > 0) {
            bytes[(offset + stored / 2) as usize] ^= 0x40;
        }
        bytes
    }
}

const SETTINGS: Settings = Settings { level: LEVEL };

/// The bytes of the recording [`Replay::from_records`] opens.
pub fn encode_records(campaign_id: Vec<u8>, segments: &[Vec<Record>]) -> io::Result<Vec<u8>> {
    encoded(campaign_id, segments, None)
}

/// [`encode_records`], each hour's derived tables pushed to `derived` when given.
fn encoded(campaign_id: Vec<u8>, segments: &[Vec<Record>], derived: Option<&mut Vec<DecodedFrame>>) -> io::Result<Vec<u8>> {
    let records = records_of(segments, None)?;
    let identity = identity_of(campaign_id, &records)?;
    let mode = crate::scope::CaptureMode::from_byte(identity.capture_mode)?;
    let file = FileWriter::create(Cursor::new(Vec::new()), identity, TABLE_COUNT, SETTINGS, None)?;
    let mut file = write_runs(file, segments, false, derived.map(|derived| (derived, mode)))?;
    add_records(&mut file, &records)?;
    Ok(file.finish_runs(&[], true, |_| Ok(()))?.0.into_inner())
}

/// Names `records` in the revision `file` publishes next.
fn add_records<W: Write + Seek + Read>(file: &mut FileWriter<W>, records: &Records) -> io::Result<()> {
    let bytes = records.encode(FormatVersion::CURRENT)?;
    let container = file.container();
    let extent = container.put_zstd(Kind::Controls, Tags::default(), &bytes, LEVEL)?;
    container.append(SectionKey::RECORDS, extent)
}

impl Replay {
    /// This recording with `segments` added in one more saved revision: its bytes through the
    /// revision opened, then the new one.
    pub fn append(&self, segments: &[Vec<Record>]) -> io::Result<Self> {
        let records = records_of(segments, Some(self))?;
        let resumed = Resumed {
            root: self.recording().root().clone(),
            at: self.revision(),
            last: None,
            open: None,
        };
        let held = Cursor::new(self.bytes()[..self.saved_length() as usize].to_vec());
        let file = FileWriter::resume(held, resumed, TABLE_COUNT, SETTINGS, None)?;
        let mut file = write_runs(file, segments, true, None)?;
        add_records(&mut file, &records)?;
        Self::open(file.finish_runs(&[], true, |_| Ok(()))?.0.into_inner())
    }
}

/// The journal of capture `capture` in `directory` that a collector recording `segments`'
/// records of campaign `campaign_id` writes: each frame an hour with its segment's next clock
/// witness of that hour, every other record a control record in its place, a run for each stretch of
/// consecutive hours of a branch and a segment for each run and each full block. Ended when
/// `finish`; left as a collector that stopped leaves it otherwise.
pub fn write_journal(
    directory: &Path,
    capture: u32,
    campaign_id: &[u8],
    segments: &[Vec<Record>],
    finish: bool,
) -> io::Result<()> {
    let records = records_of(segments, None)?;
    let identity = identity_of(campaign_id.to_vec(), &records)?;
    let mut session = Vec::new();
    records.sessions[0].encode(&mut session)?;
    let begun = Capture {
        campaign_id: campaign_id.to_vec(),
        content: identity.content.clone(),
        capture_mode: identity.capture_mode,
        sessions: vec![session],
        run_sessions: Vec::new(),
        stop: None,
        format: FormatVersion::CURRENT.version(),
        journaled: 0,
        controls: Vec::new(),
    };
    std::fs::create_dir_all(directory)?;
    let mut journal = JournalWriter::create(directory, capture, &identity.chain_bytes()?, &begun)?;
    let mut run: Option<(FrameWriter, BranchId, Hour, u32)> = None;
    let mut controls = Vec::new();
    for segment in segments {
        let mut decoder = FrameDecoder::new();
        // Rows give a frame's witness with it, before or after: each hour's are taken in order.
        let mut witnesses: std::collections::BTreeMap<Hour, std::collections::VecDeque<Witness>> = Default::default();
        for record in segment {
            if let Record::Control(ControlRecord::ClockWitness {
                qpc, hour, load_signal, ..
            }) = record
            {
                witnesses.entry(*hour).or_default().push_back(Witness {
                    qpc: *qpc,
                    load_signal: *load_signal,
                });
            }
        }
        for record in segment {
            let frame = match record {
                Record::SessionStart(_) => {
                    decoder = FrameDecoder::new();
                    continue;
                }
                Record::Control(ControlRecord::ClockWitness { .. }) => continue,
                Record::Frame(frame) => frame,
                _ => {
                    if matches!(record, Record::BranchStart(_)) {
                        decoder = FrameDecoder::new();
                    }
                    controls.push(encode_record(record)?);
                    continue;
                }
            };
            journal.controls(&controls)?;
            controls.clear();
            let decoded = decoder.advance(frame)?.clone();
            let continues = run
                .as_ref()
                .is_some_and(|(_, branch, next, _)| *branch == decoded.branch && *next == decoded.hour);
            if !continues {
                let epoch = match &run {
                    None => envelope::epoch::START,
                    Some((_, branch, ..)) if *branch != decoded.branch => envelope::epoch::BRANCH,
                    Some(_) => envelope::epoch::GAP,
                };
                let _ = journal.next_segment()?;
                journal.run(decoded.branch, epoch, decoded.hour)?;
                run = Some((FrameWriter::new()?, decoded.branch, decoded.hour, 0));
            }
            let (writer, _, next, ordinal) = run.as_mut().expect("a run under way");
            let hour = writer.hour(&input_of(&decoded))?;
            let sections: Vec<&[u8]> = hour.sections().map(|(_, bytes)| bytes).collect();
            let witness = witnesses.get_mut(&decoded.hour).and_then(std::collections::VecDeque::pop_front);
            journal.hour(witness, &hour.manifest, &sections)?;
            writer.commit(hour)?;
            *next = decoded.hour + 1;
            *ordinal += 1;
            if *ordinal % UNIT_ORDINALS == 0 {
                let _ = journal.next_segment()?;
            }
        }
    }
    journal.controls(&controls)?;
    if finish {
        journal.finish(None)?;
    }
    Ok(())
}

/// `decoded` as the frame writer takes an hour.
fn input_of(decoded: &DecodedFrame) -> FrameInput {
    FrameInput {
        branch: decoded.branch,
        hour: decoded.hour,
        tables: std::array::from_fn(|index| match &*decoded.tables[index] {
            TableView::Complete(rows) => TableInput::SharedComplete(Arc::new(rows.clone())),
            view => TableInput::Sampled(view.clone()),
        }),
        metrics: decoded.metrics.stored(),
    }
}

/// The identity a recording of `records`' first session takes.
fn identity_of(campaign_id: Vec<u8>, records: &Records) -> io::Result<envelope::Identity> {
    let first = records
        .sessions
        .first()
        .ok_or_else(|| io::Error::other("a recording holds a session"))?;
    let mut content = Vec::new();
    first.identity.encode(&mut content)?;
    Ok(envelope::Identity {
        schema: envelope::schema_digest(&schema()?),
        campaign: campaign_id,
        content,
        capture_mode: first.capture_mode.byte(),
    })
}

const LEVEL: i32 = 3;

/// The records `segments` add to `existing`: each session and branch once, a branch from its
/// start or from the command phase that declares it.
fn records_of(segments: &[Vec<Record>], existing: Option<&Replay>) -> io::Result<Records> {
    let mut records = Records::default();
    for record in segments.iter().flatten() {
        match record {
            Record::SessionStart(session) => {
                if !records.sessions.contains(session)
                    && !existing.is_some_and(|replay| replay.sessions.contains(session))
                {
                    records.sessions.push(session.clone());
                }
            }
            Record::BranchStart(branch) => {
                if !records.branches.contains(branch)
                    && !existing.is_some_and(|replay| replay.branches.contains(branch))
                {
                    records.branches.push(branch.clone());
                }
            }
            // A writer retries a gap whose segment failed; the same gap is kept once.
            Record::Gap(gap) => {
                if !records.gaps.contains(gap) && !existing.is_some_and(|replay| replay.gaps.contains(gap)) {
                    records.gaps.push(gap.try_clone()?);
                }
            }
            Record::Control(control) => {
                // A command phase declares its branch when no branch start did.
                if let ControlRecord::PlayerCommands { packet, .. } = control
                    && let Some(phase) = &packet.phase
                    && !records.branches.iter().any(|known| known.id == phase.start.id)
                    && !existing.is_some_and(|replay| replay.branches.iter().any(|known| known.id == phase.start.id))
                {
                    records.branches.push(phase.start.clone());
                }
                // A writer retries an event loss whose segment failed; the same loss is kept once.
                if matches!(control, ControlRecord::EventLoss { .. })
                    && (records.controls.contains(control)
                        || existing.is_some_and(|replay| replay.controls.contains(control)))
                {
                    continue;
                }
                records.controls.push(control.clone());
            }
            Record::Frame(_) => {}
        }
    }
    Ok(records)
}

/// Writes `segments`' frames into `file`'s world log: a run for each stretch of consecutive
/// hours of a branch, the first a recording's first run unless it `started` before. With
/// `derived`, each hour's derived tables as the store holds them after it, in that mode.
fn write_runs<W: Write + Seek + Read>(
    file: FileWriter<W>,
    segments: &[Vec<Record>],
    mut started: bool,
    mut derived: Option<(&mut Vec<DecodedFrame>, crate::scope::CaptureMode)>,
) -> io::Result<FileWriter<W>> {
    let mut file = HourWriter::new(file, LEVEL)?;
    let mut run: Option<(FrameWriter, BranchId, Hour)> = None;
    for segment in segments {
        let mut decoder = FrameDecoder::new();
        for record in segment {
            let Record::Frame(frame) = record else {
                if matches!(record, Record::SessionStart(_) | Record::BranchStart(_)) {
                    decoder = FrameDecoder::new();
                }
                continue;
            };
            let decoded = decoder.advance(frame)?.clone();
            let continues = run
                .as_ref()
                .is_some_and(|(_, branch, next)| *branch == decoded.branch && *next == decoded.hour);
            if !continues {
                let epoch = if started {
                    envelope::epoch::GAP
                } else {
                    envelope::epoch::START
                };
                started = true;
                file.start_run(decoded.branch, epoch, decoded.hour)?;
                run = Some((FrameWriter::new()?, decoded.branch, decoded.hour));
            }
            let (writer, _, next) = run.as_mut().expect("a run under way");
            let hour = writer.hour(&input_of(&decoded))?;
            let joined: Vec<u8> = hour.sections().flat_map(|(_, bytes)| bytes.iter().copied()).collect();
            file.hour(&hour.manifest, &joined)?;
            writer.commit(hour)?;
            *next = decoded.hour + 1;
            if let Some((frames, mode)) = derived.as_mut() {
                frames.push(file.derived_frame((decoded.branch, decoded.hour), *mode)?);
            }
        }
    }
    file.finish()
}

/// `decoded` as a frame holding each table whole; a keyframe when it starts its block.
fn whole(decoded: &crate::frame::DecodedFrame, keyframe: bool) -> io::Result<crate::frame::Frame> {
    use crate::frame::TableUpdate;
    let tables = std::array::from_fn(|index| match &*decoded.tables[index] {
        TableView::Complete(rows) => TableUpdate::Snapshot(rows.clone()),
        TableView::Truncated { rows, missing } => TableUpdate::Truncated {
            rows: rows.clone(),
            missing: *missing,
        },
        TableView::Unavailable { reason } => TableUpdate::Unavailable {
            reason: reason.clone(),
        },
        TableView::NotRecorded { mode } => TableUpdate::NotRecorded { mode: *mode },
    });
    Ok(crate::frame::Frame {
        branch: decoded.branch,
        hour: decoded.hour,
        keyframe,
        metrics: decoded.metrics.clone(),
        tables,
    })
}

/// Every frame a table's entry names: its parts' and its restores' chunks.
fn unit_frames(entry: &replay_columnar::unit::TableEntry) -> Vec<(u64, u64)> {
    let mut out: Vec<(u64, u64)> = entry.parts.iter().flat_map(|part| part.frames.iter().map(|frame| (frame.offset, u64::from(frame.stored)))).collect();
    for restore in entry.first.iter().chain(&entry.restore) {
        for (_, chunks) in &restore.streams {
            out.extend(chunks.iter().map(|chunk| (chunk.frame.offset, u64::from(chunk.frame.stored))));
        }
    }
    out
}
