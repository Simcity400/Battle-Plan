//! One exact-hour map reader for viewer playback and acceptance measurements.
use super::{
    Checkpoint, Current, Position, Progress, decode_checkpoint, gap_reason, header, invalid,
    section, validate_selection,
};
use replay::{container::Parallel, container::Replay};
use replay_data::{BranchId, Hour, rows::TableId, rows::tables};
use replay_frame::{frame::DecodedFrame, frame::Frame, frame::FrameDecoder, frame::TableView};
use std::{io, ops::Range, sync::Arc, time::Instant};
use store::{SourceKey, Store};

mod load;
use load::Load;

#[cfg(test)]
mod reuse_tests;

#[cfg(test)]
type SourceReadHook = Box<dyn FnMut(Hour, &io::Result<Vec<Frame>>) + Send + Sync>;

#[derive(Clone)]
struct Block {
    index: usize,
    through: Hour,
    frames: Arc<Vec<Frame>>,
}

struct Admitted {
    frames: Vec<(Block, Range<usize>)>,
    cursor: Position,
}

/// Restores a sparse native checkpoint, then reads the recording's selected
/// updates through the target. Displayed-hour presentation and clocks stay with
/// the caller. Only one parsed source block is retained between requests.
pub struct Reader {
    store: Arc<Store>,
    key: SourceKey,
    progress: Progress,
    replay: Arc<Replay>,
    selected: Vec<TableId>,
    decoder: FrameDecoder,
    cursor: Option<Position>,
    block: Option<Block>,
    requested: Option<(BranchId, Hour)>,
    workers: usize,
    #[cfg(test)]
    source_read_hook: Option<SourceReadHook>,
    /// Native frames applied by the most recent request.
    pub applied_frames: u64,
    /// Checkpoint headers read by the most recent seek.
    pub checkpoint_reads: u64,
    /// Whole checkpoint range bytes read by the most recent request.
    pub checkpoint_bytes: u64,
    /// Selected source block loads, including extension after a prefix seek.
    pub source_reads: u64,
    /// Container bytes of loaded source blocks; an upper bound on selected I/O.
    pub source_block_bytes: u64,
    /// Source frames parsed, including those preceding the native checkpoint.
    pub source_decoded_frames: u64,
    /// Time inside the selected source reader, including table decoding.
    pub source_load_us: u64,
    /// Frames in the one retained source block, for acceptance reporting.
    pub retained_source_frames: usize,
}

impl Reader {
    pub fn new(
        store: impl Into<Arc<Store>>,
        key: SourceKey,
        progress: Progress,
        replay: Arc<Replay>,
        selected: &[TableId],
    ) -> io::Result<Self> {
        validate_selection(selected)?;
        if progress.stride == 0 {
            return Err(invalid("map checkpoint stride is zero"));
        }
        let mut selection: Vec<u16> = selected.iter().map(|table| *table as u16).collect();
        selection.sort_unstable();
        if selection != progress.selected {
            return Err(invalid(
                "map checkpoint table selection differs from progress",
            ));
        }
        Ok(Self {
            store: store.into(),
            key,
            progress,
            replay,
            selected: selected.to_vec(),
            decoder: FrameDecoder::for_tables(selected),
            cursor: None,
            block: None,
            requested: None,
            workers: std::thread::available_parallelism()
                .map_or(1, |count| count.get() / 2)
                .max(1),
            #[cfg(test)]
            source_read_hook: None,
            applied_frames: 0,
            checkpoint_reads: 0,
            checkpoint_bytes: 0,
            source_reads: 0,
            source_block_bytes: 0,
            source_decoded_frames: 0,
            source_load_us: 0,
            retained_source_frames: 0,
        })
    }

    pub fn replay_handle(&self) -> &Arc<Replay> {
        &self.replay
    }

    pub fn range(&self, branch: BranchId) -> Option<(Hour, Hour)> {
        self.progress.ranges.get(&branch).copied()
    }

    pub fn covers(&self, branch: BranchId, hour: Hour) -> bool {
        self.progress
            .branches
            .get(&branch)
            .and_then(|p| p.last_hour)
            .is_some_and(|last| hour <= last)
    }

    pub fn walk_start(&self, branch: BranchId, hour: Hour) -> io::Result<Option<Hour>> {
        if let Some(frame) = self.decoder.current().filter(|frame| {
            frame.branch == branch
                && frame.hour < hour
                && (self.steps_to(branch, hour)
                    || i64::from(hour) - i64::from(frame.hour) < i64::from(self.progress.stride))
        }) {
            return Ok(Some(frame.hour));
        }
        Ok(self.find_before(branch, hour)?.0.map(|(_, hour)| hour))
    }

    fn find_before(&self, branch: BranchId, hour: Hour) -> io::Result<(Option<(u64, Hour)>, u64)> {
        let mut low = 0;
        let mut high = self
            .progress
            .branches
            .get(&branch)
            .map_or(0, |p| p.checkpoints);
        let mut before = None;
        let mut reads = 0;
        while low < high {
            let mid = low + (high - low) / 2;
            let (page_branch, page_hour) = header(self.page(branch, mid)?)?;
            reads += 1;
            if page_branch != branch {
                return Err(invalid("map checkpoint belongs to another branch"));
            }
            if page_hour <= hour {
                before = Some((mid, page_hour));
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        Ok((before, reads))
    }

    pub fn seek(&mut self, branch: BranchId, hour: Hour) -> io::Result<Option<Current>> {
        self.seek_with(branch, hour, |_| {})
    }

    /// `None` leaves an unprepared target to ordinary playback. Recorded holes
    /// within the accepted prefix keep their exact named gap reasons.
    pub fn seek_with(
        &mut self,
        branch: BranchId,
        hour: Hour,
        passed: impl FnMut(&DecodedFrame),
    ) -> io::Result<Option<Current>> {
        if self.seek_native(branch, hour, passed)? {
            self.current_at(branch, hour).map(Some)
        } else {
            Ok(None)
        }
    }

    fn seek_native(
        &mut self,
        branch: BranchId,
        hour: Hour,
        passed: impl FnMut(&DecodedFrame),
    ) -> io::Result<bool> {
        self.reset_metrics();
        if !self.store.is_reusable(&self.key)? || !self.covers(branch, hour) {
            return Ok(false);
        }
        let (before, reads) = self.find_before(branch, hour)?;
        self.checkpoint_reads = reads;
        let Some((range, checkpoint_hour)) = before else {
            self.decoder = FrameDecoder::for_tables(&self.selected);
            self.cursor = None;
            self.requested = Some((branch, hour));
            self.gap(branch, hour)?;
            return Ok(true);
        };
        let file = self.page(branch, range)?;
        self.checkpoint_bytes = file.len();
        let checkpoint = decode_checkpoint(file, &self.selected)?;
        if checkpoint.frame.hour != checkpoint_hour {
            return Err(invalid("map checkpoint search differs from its header"));
        }
        self.validate_checkpoint(branch, &checkpoint)?;
        self.cursor = Some(checkpoint.position);
        self.decoder = FrameDecoder::resume_for_tables(&self.selected, checkpoint.frame)?;
        self.advance_to(branch, hour, passed, Load::RequestedPrefix)?;
        self.requested = Some((branch, hour));
        Ok(true)
    }

    /// Applies one hour in place for a step. A block load is retained so later
    /// steps never repeat source-prefix decoding within that block.
    pub fn advance(
        &mut self,
        branch: BranchId,
        hour: Hour,
        passed: impl FnMut(&DecodedFrame),
    ) -> io::Result<Option<Current>> {
        if self.advance_without_output(branch, hour, passed)? {
            self.current_at(branch, hour).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Advances through a cancellation checkpoint without exporting an owned
    /// endpoint snapshot. Visitors still receive the earlier applied hours,
    /// and all source, campaign and gap validation is unchanged. `false` leaves
    /// a target outside prepared coverage to the ordinary replay reader.
    pub fn advance_without_output(
        &mut self,
        branch: BranchId,
        hour: Hour,
        passed: impl FnMut(&DecodedFrame),
    ) -> io::Result<bool> {
        self.reset_metrics();
        if !self.store.is_reusable(&self.key)? || !self.covers(branch, hour) {
            return Ok(false);
        }
        if self
            .decoder
            .current()
            .is_none_or(|frame| frame.branch != branch || frame.hour > hour)
            || (!self.steps_to(branch, hour)
                && self.decoder.current().is_some_and(|frame| {
                    i64::from(hour) - i64::from(frame.hour) >= i64::from(self.progress.stride)
                }))
        {
            return self.seek_native(branch, hour, passed);
        }
        self.advance_to(branch, hour, passed, self.load(branch, hour))?;
        self.requested = Some((branch, hour));
        Ok(true)
    }

    fn current_at(&self, branch: BranchId, hour: Hour) -> io::Result<Current> {
        match self.decoder.current() {
            Some(frame) if frame.branch == branch && frame.hour == hour => {
                Ok(Current::Frame(Arc::new(frame.clone())))
            }
            _ => self.gap(branch, hour),
        }
    }

    fn reset_metrics(&mut self) {
        self.applied_frames = 0;
        self.checkpoint_reads = 0;
        self.checkpoint_bytes = 0;
        self.source_reads = 0;
        self.source_block_bytes = 0;
        self.source_decoded_frames = 0;
        self.source_load_us = 0;
    }

    fn steps_to(&self, branch: BranchId, hour: Hour) -> bool {
        self.requested
            .is_some_and(|(b, h)| b == branch && h.checked_add(1) == Some(hour))
    }

    fn load(&self, branch: BranchId, hour: Hour) -> Load {
        if self.steps_to(branch, hour) {
            Load::AcceptedBlock
        } else if self.requested.is_some_and(|(b, h)| b == branch && h < hour) {
            Load::AcceptedBlockOrPrefix
        } else {
            Load::RequestedPrefix
        }
    }

    fn page(&self, branch: BranchId, range: u64) -> io::Result<store::Range> {
        self.store
            .read(&self.key, &section(branch), range)?
            .ok_or_else(|| invalid("map checkpoint page is missing"))
    }

    fn advance_to(
        &mut self,
        branch: BranchId,
        hour: Hour,
        passed: impl FnMut(&DecodedFrame),
        load: Load,
    ) -> io::Result<()> {
        if self
            .decoder
            .current()
            .is_some_and(|f| f.branch == branch && f.hour == hour)
        {
            return Ok(());
        }
        // Validate gap metadata before any visitor, even without an endpoint snapshot.
        self.gap(branch, hour)?;
        let Some(cursor) = self.cursor else {
            return Ok(());
        };
        let previous = self.decoder.current().map(|frame| frame.hour);
        let admitted = self.admit(branch, hour, previous, cursor, load)?;
        self.apply_admitted(hour, admitted, passed)
    }

    fn admit(
        &mut self,
        branch: BranchId,
        hour: Hour,
        mut previous: Option<Hour>,
        mut cursor: Position,
        load: Load,
    ) -> io::Result<Admitted> {
        let progress = &self.progress.branches[&branch];
        let last_block = progress
            .last_block
            .ok_or_else(|| invalid("map accepted source cursor is missing"))?;
        let last_hour = progress.last_hour.unwrap();
        let mut admitted = Vec::new();
        // Admit every source load before visitors, so a refusal cannot repeat their history.
        while cursor.block <= last_block {
            let index = self
                .replay
                .index
                .get(cursor.block)
                .ok_or_else(|| invalid("map source cursor exceeds recording"))?;
            if index.branch != branch {
                cursor.block += 1;
                cursor.next_frame = 0;
                continue;
            }
            if index.first_hour > hour {
                break;
            }
            // A checkpoint at the block's last hour already consumed its final frame.
            if previous == Some(index.last_hour) {
                cursor.block += 1;
                cursor.next_frame = 0;
                continue;
            }
            let accepted_end = index.last_hour.min(last_hour);
            let required = load.required(hour, accepted_end);
            if self
                .block
                .as_ref()
                .is_none_or(|block| block.index != cursor.block || block.through < required)
            {
                self.block = None;
                self.retained_source_frames = 0;
                let on_worker = || {
                    #[cfg(windows)]
                    unsafe {
                        use windows_sys::Win32::System::Threading::{
                            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
                        };
                        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
                    }
                };
                let parallel = Parallel {
                    workers: self.workers,
                    on_worker: &on_worker,
                };
                let started = Instant::now();
                let result = load.read(required, accepted_end, |through| {
                    let result = self.replay.read_block_selected_through(
                        index,
                        Some(&self.selected),
                        through,
                        (self.workers > 1).then_some(&parallel),
                    );
                    #[cfg(test)]
                    if let Some(hook) = self.source_read_hook.as_mut() {
                        hook(through, &result);
                    }
                    result
                });
                self.source_load_us +=
                    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
                let (through, frames) = result?;
                self.source_reads += 1;
                self.source_block_bytes += index.length;
                self.source_decoded_frames += frames.len() as u64;
                self.retained_source_frames = frames.len();
                self.block = Some(Block {
                    index: cursor.block,
                    through,
                    frames: Arc::new(frames),
                });
            }
            let block = self.block.as_ref().unwrap();
            if cursor.next_frame > block.frames.len()
                || (cursor.next_frame > 0
                    && block
                        .frames
                        .get(cursor.next_frame - 1)
                        .map(|frame| frame.hour)
                        != previous)
            {
                return Err(invalid("map source cursor differs from decoded baseline"));
            }
            let start = cursor.next_frame;
            while let Some(frame) = block
                .frames
                .get(cursor.next_frame)
                .filter(|f| f.hour <= hour)
            {
                if frame.branch != branch || previous.is_some_and(|h| frame.hour <= h) {
                    return Err(invalid(
                        "map source frames do not advance within their branch",
                    ));
                }
                previous = Some(frame.hour);
                cursor.next_frame += 1;
            }
            if start != cursor.next_frame {
                engine_allocation::reserve(&mut admitted, 1)?;
                admitted.push((block.clone(), start..cursor.next_frame));
            }
            if cursor.next_frame < block.frames.len() || block.through < index.last_hour {
                break;
            }
            cursor.block += 1;
            cursor.next_frame = 0;
        }
        // Check Clear after all I/O and before touching an external visitor.
        if self.source_reads != 0 && !self.store.is_reusable(&self.key)? {
            return Err(invalid("map store was cleared during source read"));
        }
        Ok(Admitted {
            frames: admitted,
            cursor,
        })
    }

    fn apply_admitted(
        &mut self,
        hour: Hour,
        admitted: Admitted,
        mut passed: impl FnMut(&DecodedFrame),
    ) -> io::Result<()> {
        for (block, range) in admitted.frames {
            for offset in range {
                let decoded = self.decoder.advance(&block.frames[offset])?;
                Self::validate_campaign(&self.replay.campaign_id, decoded)?;
                self.applied_frames += 1;
                if decoded.hour == hour {
                    self.cursor = Some(Position {
                        block: block.index,
                        next_frame: offset + 1,
                    });
                    return Ok(());
                }
                passed(decoded);
            }
        }
        self.cursor = Some(admitted.cursor);
        Ok(())
    }

    fn validate_checkpoint(&self, branch: BranchId, checkpoint: &Checkpoint) -> io::Result<()> {
        let progress = &self.progress.branches[&branch];
        if checkpoint.frame.branch != branch
            || checkpoint.position.next_frame == 0
            || progress
                .last_block
                .is_none_or(|block| checkpoint.position.block > block)
            || progress
                .last_hour
                .is_none_or(|hour| checkpoint.frame.hour > hour)
        {
            return Err(invalid("map checkpoint cursor is invalid"));
        }
        let source = self
            .replay
            .index
            .get(checkpoint.position.block)
            .ok_or_else(|| invalid("map checkpoint source cursor is invalid"))?;
        if source.branch != branch
            || checkpoint.frame.hour < source.first_hour
            || checkpoint.frame.hour > source.last_hour
        {
            return Err(invalid("map checkpoint hour differs from source metadata"));
        }
        Self::validate_campaign(&self.replay.campaign_id, &checkpoint.frame)
    }

    fn validate_campaign(campaign_id: &[u8], frame: &DecodedFrame) -> io::Result<()> {
        if let TableView::Complete(rows) = &*frame.tables[TableId::Campaign.index()]
            && rows
                .rows::<tables::Campaign>()
                .is_none_or(|rows| rows.len() != 1 || rows[0].id != campaign_id)
        {
            return Err(invalid(
                "frame campaign identity differs from replay header",
            ));
        }
        Ok(())
    }

    fn gap(&self, branch: BranchId, hour: Hour) -> io::Result<Current> {
        Ok(Current::Gap {
            branch,
            hour,
            reason: gap_reason(&self.store, &self.key, &self.progress, branch, hour)?,
        })
    }
}
