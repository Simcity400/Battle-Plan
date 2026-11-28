use super::{BLOCK, Frames, Recording, TableReader, invalid};
use replay_data::{BranchId, Hour, rows::{TableId, tables}};
use std::{io, ops::RangeInclusive};

pub struct CampaignBlocks<'r> {
    recording: &'r Recording,
    frames: Option<Frames<&'r Recording>>,
}

impl<'r> CampaignBlocks<'r> {
    pub fn new(recording: &'r Recording) -> Self {
        Self { recording, frames: None }
    }

    pub fn visit_block(
        &mut self,
        run: u32,
        branch: BranchId,
        hours: RangeInclusive<Hour>,
        campaign: &[u8],
        mut verified: impl FnMut(Hour),
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        let held = self.frames.take();
        check()?;
        let recorded = self.recording.run(run)?;
        let ordinal = |hour| recorded.ordinal(hour).ok_or_else(|| invalid("a block's hour lies outside its run"));
        let (first, last) = (ordinal(*hours.start())?, ordinal(*hours.end())?);
        if first > last || first / BLOCK != last / BLOCK {
            return Err(invalid("Campaign coverage needs an ordered range in one block"));
        }
        let mut frames = match held {
            Some(frames) if hours.start().checked_sub(1).is_some_and(|before| frames.at() == Some((run, before))) => frames,
            _ => Frames::new(self.recording, &[TableId::Campaign])?,
        };
        if frames.at().is_some() && first > 0 && first.is_multiple_of(BLOCK)
            && self.recording.table_entry(run, first / BLOCK - 1, TableId::Campaign.index())?.unit.restore.is_some()
        {
            frames.seek_with(run, first, TableReader::seek)?;
        } else {
            frames.go_to(run, first)?;
        }
        frames.keyframe();
        for ordinal in first..=last {
            if ordinal > first {
                check()?;
                frames.go_to(run, ordinal)?;
            }
            let frame = frames.frame(branch)?;
            check()?;
            if frame.complete::<tables::Campaign>().is_ok_and(|rows| rows.len() == 1 && rows[0].id.as_slice() == campaign) {
                verified(frame.hour);
            }
        }
        if frames.reader(TableId::Campaign).is_some() {
            self.frames = Some(frames);
        }
        Ok(())
    }
}
