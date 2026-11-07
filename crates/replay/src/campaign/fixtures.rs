//! Campaign recordings made from spool records, for tests: every frame decoded as the row
//! decoder does, written through the collector's frame writer ([`crate::native_frame`]) and
//! the recorder's block builder into one capture, which a campaign writer saves with the
//! records.

use super::{CampaignWriter, Records, Replay};
use crate::{
    BranchId, Hour,
    columnar_blocks::BlockBuilder,
    columnar_file::{FileWriter, Settings},
    columnar_frames::{Recording, schema},
    frame::{FrameDecoder, FrameInput, TableInput, TableView},
    identity::IdentityCodec,
    native_frame::FrameWriter,
    rows::TABLE_COUNT,
    spool::{Record, Recovery},
};
use replay_columnar::envelope;
use std::{
    io::{self, Cursor},
    sync::Arc,
};

impl Replay {
    /// The recording of `segments`' records, in order, of campaign `campaign_id`.
    pub fn from_records(campaign_id: Vec<u8>, segments: &[Vec<Record>]) -> io::Result<Self> {
        Self::open(encode_records(campaign_id, segments)?)
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

    /// The bytes of this recording, opened from memory, with every stored extent of `block`'s
    /// page damaged mid-extent: the index still lists the block and reading its hours fails.
    pub fn damaged_block(&self, block: &super::BlockIndex) -> io::Result<Vec<u8>> {
        let page = self.recording().page(block.run, block.page as usize)?;
        let mut extents = vec![page.records];
        extents.extend(page.tables.iter().copied());
        if let Some(restart) = &page.restart {
            extents.push(restart.chunk);
            extents.extend(restart.tables.iter().map(|table| table.extent));
        }
        Ok(self.damaged(&extents))
    }

    /// [`Self::damaged_block`] of `table`'s block in `block`'s page alone.
    pub fn damaged_table(&self, block: &super::BlockIndex, table: crate::rows::TableId) -> io::Result<Vec<u8>> {
        let page = self.recording().page(block.run, block.page as usize)?;
        let extent = page
            .tables
            .iter()
            .find(|extent| extent.stream == table.index() as u32)
            .ok_or_else(|| io::Error::other("the block holds no hour of the table"))?;
        Ok(self.damaged(&[*extent]))
    }

    fn damaged(&self, extents: &[envelope::Extent]) -> Vec<u8> {
        let mut bytes = self.bytes().to_vec();
        for extent in extents.iter().filter(|extent| extent.stored > 0) {
            bytes[(extent.offset + extent.stored / 2) as usize] ^= 0x40;
        }
        bytes
    }
}

/// The bytes of the recording [`Replay::from_records`] opens.
pub fn encode_records(campaign_id: Vec<u8>, segments: &[Vec<Record>]) -> io::Result<Vec<u8>> {
    let mut records = records_of(segments, None)?;
    let identity = identity_of(campaign_id, &records)?;
    let capture = Recording::open(capture_of(&identity, segments)?)?;
    let mut campaign = CampaignWriter::create(Cursor::new(Vec::new()), identity, LEVEL)?;
    campaign.ingest(&capture, SPOOL, 0, &|_| true)?;
    campaign.add_records(&mut records)?;
    campaign.revise(|_| Ok(()))?;
    Ok(campaign.into_inner().into_inner())
}

impl Replay {
    /// This recording with a save of `segments` added, as the recorder adds one: its bytes
    /// through its last revision, then the new revision.
    pub fn append(&self, segments: &[Vec<Record>]) -> io::Result<Self> {
        let mut records = records_of(segments, Some(self))?;
        let capture = Recording::open(capture_of(&self.recording().root().identity, segments)?)?;
        let mut file = Cursor::new(self.bytes()[..self.saved_length() as usize].to_vec());
        file.set_position(self.saved_length());
        let mut campaign = CampaignWriter::resumed(self, file, LEVEL)?;
        let number = self.sources.iter().map(|source| source.capture + 1).max().unwrap_or(0);
        campaign.ingest(&capture, SPOOL, number, &|_| true)?;
        campaign.add_records(&mut records)?;
        campaign.revise(|_| Ok(()))?;
        Self::open(campaign.into_inner().into_inner())
    }
}

/// A finished capture of `segments`' frames in campaign `campaign_id`, as the recorder's capture
/// writer leaves one in the session spool.
pub fn capture_bytes(campaign_id: &[u8], segments: &[Vec<Record>]) -> io::Result<Vec<u8>> {
    let identity = identity_of(campaign_id.to_vec(), &records_of(segments, None)?)?;
    capture_of(&identity, segments)
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
const SPOOL: &str = "fixture";

/// The records `segments` add to `existing`: each session and branch once.
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
            Record::Gap(gap) => records.gaps.push(gap.try_clone()?),
            Record::Control(control) => records.controls.push(control.clone()),
            Record::Frame(_) => {}
        }
    }
    Ok(records)
}

/// The capture of `segments`' frames: a run for each stretch of consecutive hours of a branch.
fn capture_of(identity: &envelope::Identity, segments: &[Vec<Record>]) -> io::Result<Vec<u8>> {
    let settings = Settings {
        level: LEVEL,
        parallel: false,
    };
    let mut file = FileWriter::create(
        Cursor::new(Vec::new()),
        identity.clone(),
        TABLE_COUNT,
        settings,
    )?;
    let mut blocks = BlockBuilder::new()?;
    let mut run: Option<(FrameWriter, BranchId, Hour)> = None;
    let mut started = false;
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
                if let Some(block) = blocks.end_run()? {
                    file.block(block.block())?;
                }
                let epoch = if started {
                    envelope::epoch::GAP
                } else {
                    envelope::epoch::START
                };
                started = true;
                file.start_run(decoded.branch, epoch, decoded.hour)?;
                blocks.start_run(decoded.branch, decoded.hour)?;
                run = Some((FrameWriter::new()?, decoded.branch, decoded.hour));
            }
            let (writer, _, next) = run.as_mut().expect("a run under way");
            let input = FrameInput {
                branch: decoded.branch,
                hour: decoded.hour,
                tables: std::array::from_fn(|index| match &*decoded.tables[index] {
                    TableView::Complete(rows) => TableInput::SharedComplete(Arc::new(rows.clone())),
                    view => TableInput::Sampled(view.clone()),
                }),
                metrics: decoded.metrics.stored(),
            };
            let hour = writer.hour(&input)?;
            let joined: Vec<u8> = hour.sections().flat_map(|(_, bytes)| bytes.iter().copied()).collect();
            if let Some(block) = blocks.hour(&hour.manifest, &joined)? {
                file.block(block.block())?;
            }
            writer.commit(hour)?;
            *next = decoded.hour + 1;
        }
    }
    if let Some(block) = blocks.end_run()? {
        file.block(block.block())?;
    }
    Ok(file.finish(|_| Ok(()))?.file.into_inner())
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
