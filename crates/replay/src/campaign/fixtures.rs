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
    pub fn from_records(campaign_id: Vec<u8>, segments: &[Vec<Record>]) -> io::Result<Self> {
        Self::open(encode_records(campaign_id, segments)?)
    }

    pub fn from_records_derived(campaign_id: Vec<u8>, segments: &[Vec<Record>]) -> io::Result<(Self, Vec<DecodedFrame>)> {
        let mut derived = Vec::new();
        let bytes = encoded(campaign_id, segments, Some(&mut derived))?;
        Ok((Self::open(bytes)?, derived))
    }

    pub fn write_atomic(&self, path: &std::path::Path) -> io::Result<()> {
        let temporary = path.with_extension("write.tmp");
        std::fs::write(&temporary, self.bytes())?;
        std::fs::rename(&temporary, path)
    }

    pub fn read_block(&self, block: &super::BlockIndex) -> io::Result<Vec<crate::frame::Frame>> {
        self.read_block_selected(block, &crate::rows::TableId::ALL)
    }

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

    pub fn from_recovered(campaign_id: Vec<u8>, segments: &[(u32, Recovery<Record>)]) -> io::Result<Self> {
        let mut ordered: Vec<_> = segments.iter().collect();
        ordered.sort_unstable_by_key(|(number, _)| *number);
        let records: Vec<Vec<Record>> = ordered
            .into_iter()
            .map(|(_, recovery)| recovery.records.clone())
            .collect();
        Self::from_records(campaign_id, &records)
    }

    pub fn damaged_block(&self, block: &super::BlockIndex) -> io::Result<Vec<u8>> {
        let page = self.recording().block(block.run, block.page)?;
        let mut frames = vec![(page.head.clock.offset, u64::from(page.head.clock.stored))];
        for entry in &page.tables {
            frames.extend(unit_frames(&entry.unit));
        }
        Ok(self.damaged(&frames))
    }

    pub fn damaged_table(&self, block: &super::BlockIndex, table: crate::rows::TableId) -> io::Result<Vec<u8>> {
        let entry = self.recording().table_entry(block.run, block.page, table.index())?;
        let frames: Vec<(u64, u64)> = entry.unit.parts.iter().flat_map(|part| part.frames.iter().map(|frame| (frame.offset, u64::from(frame.stored)))).collect();
        if frames.is_empty() {
            return Err(io::Error::other("the block holds no part of the table"));
        }
        Ok(self.damaged(&frames))
    }

    fn damaged(&self, frames: &[(u64, u64)]) -> Vec<u8> {
        let mut bytes = self.bytes().to_vec();
        for &(offset, stored) in frames.iter().filter(|(_, stored)| *stored > 0) {
            bytes[(offset + stored / 2) as usize] ^= 0x40;
        }
        bytes
    }
}

const SETTINGS: Settings = Settings { level: LEVEL };

pub fn encode_records(campaign_id: Vec<u8>, segments: &[Vec<Record>]) -> io::Result<Vec<u8>> {
    encoded(campaign_id, segments, None)
}

fn encoded(campaign_id: Vec<u8>, segments: &[Vec<Record>], derived: Option<&mut Vec<DecodedFrame>>) -> io::Result<Vec<u8>> {
    let records = records_of(segments, None)?;
    let identity = identity_of(campaign_id, &records)?;
    let mode = crate::scope::CaptureMode::from_byte(identity.capture_mode)?;
    let file = FileWriter::create(Cursor::new(Vec::new()), identity, TABLE_COUNT, SETTINGS, None)?;
    let mut file = write_runs(file, segments, false, derived.map(|derived| (derived, mode)))?;
    add_records(&mut file, &records, FormatVersion::CURRENT)?;
    Ok(file.finish_runs(&[], true, |_| Ok(()))?.0.into_inner())
}

pub fn encode_records_naming(format: u16, campaign_id: Vec<u8>, segments: &[Vec<Record>]) -> io::Result<Vec<u8>> {
    let records = records_of(segments, None)?;
    let identity = identity_of(campaign_id, &records)?;
    let file = FileWriter::create(Cursor::new(Vec::new()), identity, TABLE_COUNT, SETTINGS, None)?;
    let mut file = write_runs(file, segments, false, None)?;
    let mut bytes = records.encode(FormatVersion::CURRENT)?;
    bytes[super::records::MAGIC.len()..super::records::MAGIC.len() + 2].copy_from_slice(&format.to_le_bytes());
    let container = file.container();
    let extent = container.put_zstd(Kind::Controls, Tags::default(), &bytes, LEVEL)?;
    container.append(SectionKey::RECORDS, extent)?;
    Ok(file.finish_runs(&[], true, |_| Ok(()))?.0.into_inner())
}

pub fn encode_records_at(format: u16, campaign_id: Vec<u8>, segments: &[Vec<Record>]) -> io::Result<Vec<u8>> {
    encode_framed_at(format, campaign_id, segments, |_, _| Ok(()))
}

pub fn encode_framed_at(
    format: u16,
    campaign_id: Vec<u8>,
    segments: &[Vec<Record>],
    mut each: impl FnMut(&mut crate::columnar_frames::synthetic::Synthetic, &DecodedFrame) -> io::Result<()>,
) -> io::Result<Vec<u8>> {
    use crate::{
        columnar_frames::synthetic::{Input, Sample, Synthetic},
        rows::{TableId, tables},
    };
    let records = records_of(segments, None)?;
    let columns = crate::columnar_frames::columns_at(format)?;
    let schema = envelope::schema_digest_prefix(&crate::columnar_frames::schema_at(format)?[..columns.len()], &columns).ok_or_else(|| io::Error::other("the format frames no schema of this build"))?;
    let identity = envelope::Identity { schema, ..identity_of(campaign_id, &records)? };
    let file = FileWriter::create(Cursor::new(Vec::new()), identity, columns.len(), SETTINGS, None)?;
    let mut file = HourWriter::framed(file, LEVEL, replay_columnar::unit::Settings::at(LEVEL), &columns)?;
    let mut run: Option<(Synthetic, BranchId, Hour)> = None;
    for segment in segments {
        let mut decoder = FrameDecoder::new();
        for record in segment {
            let Record::Frame(frame) = record else { continue };
            let decoded = decoder.advance(frame)?.clone();
            if !run.as_ref().is_some_and(|(_, branch, next)| *branch == decoded.branch && *next == decoded.hour) {
                let epoch = if run.is_some() { envelope::epoch::GAP } else { envelope::epoch::START };
                file.start_run(decoded.branch, epoch, decoded.hour)?;
                run = Some((Synthetic::framed_at(decoded.branch, decoded.hour, format, columns.clone())?, decoded.branch, decoded.hour));
            }
            let (synthetic, _, next) = run.as_mut().expect("a run under way");
            let clock = decoded.complete::<tables::Clock>()?;
            synthetic.table(TableId::Clock, Input::Snapshot { rows: clock, partial: None, read: true, sample: Sample::FromReadBit })?;
            let campaign = decoded.complete::<tables::Campaign>()?;
            synthetic.table(TableId::Campaign, Input::Snapshot { rows: campaign, partial: None, read: true, sample: Sample::FromReadBit })?;
            each(synthetic, &decoded)?;
            synthetic.end_encoded(|manifest, sections| file.hour(manifest, sections))??;
            *next = decoded.hour + 1;
        }
    }
    let mut file = file.finish()?;
    add_records(&mut file, &records, FormatVersion::read(format)?)?;
    Ok(file.finish_runs(&[], true, |_| Ok(()))?.0.into_inner())
}

fn add_records<W: Write + Seek + Read>(file: &mut FileWriter<W>, records: &Records, format: FormatVersion) -> io::Result<()> {
    let bytes = records.encode(format)?;
    let container = file.container();
    let extent = container.put_zstd(Kind::Controls, Tags::default(), &bytes, LEVEL)?;
    container.append(SectionKey::RECORDS, extent)
}

impl Replay {
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
        add_records(&mut file, &records, FormatVersion::CURRENT)?;
        Self::open(file.finish_runs(&[], true, |_| Ok(()))?.0.into_inner())
    }
}

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
            Record::Gap(gap) => {
                if !records.gaps.contains(gap) && !existing.is_some_and(|replay| replay.gaps.contains(gap)) {
                    records.gaps.push(gap.try_clone()?);
                }
            }
            Record::Control(control) => {
                if let ControlRecord::PlayerCommands { packet, .. } = control
                    && let Some(phase) = &packet.phase
                    && !records.branches.iter().any(|known| known.id == phase.start.id)
                    && !existing.is_some_and(|replay| replay.branches.iter().any(|known| known.id == phase.start.id))
                {
                    records.branches.push(phase.start.clone());
                }
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

fn unit_frames(entry: &replay_columnar::unit::TableEntry) -> Vec<(u64, u64)> {
    let mut out: Vec<(u64, u64)> = entry.parts.iter().flat_map(|part| part.frames.iter().map(|frame| (frame.offset, u64::from(frame.stored)))).collect();
    for restore in entry.first.iter().chain(&entry.restore) {
        for (_, chunks) in &restore.streams {
            out.extend(chunks.iter().map(|chunk| (chunk.frame.offset, u64::from(chunk.frame.stored))));
        }
    }
    out
}
