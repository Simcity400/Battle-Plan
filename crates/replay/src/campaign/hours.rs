//! A campaign recording's hours as the row decoder's frames ([`crate::columnar_frames::Views`]).
//!
//! Every block is read as a unit starting from nothing before it: its first hour's views are
//! built anew from the walk's state, as a keyframe's are, and each later hour's from the hour
//! before's, so a block's hours are the same whichever blocks are read with it. A walk starts
//! at the restart before the block's segment and applies the hours up to the block without
//! building views of them.

use super::{BlockIndex, Replay, SeekResult};
use crate::{
    BranchId, Hour,
    codec::invalid_static,
    columnar_frames::{Recording, Views, Walker, metrics},
    frame::{DecodedFrame, RecordedFrame, TableView},
    rows::{TableId, TableRows},
};
use replay_columnar::{
    manifest::{Manifest, Status},
    walk::State,
};
use std::{
    io,
    sync::{Arc, mpsc},
    thread,
};

/// How a view build spreads its tables over threads.
pub struct Parallel<'a> {
    /// Worker threads at most; one builds on the calling thread.
    pub workers: usize,
    /// Runs first on each worker, for the caller's thread setup such as a lower scheduling
    /// priority, which a spawned thread does not inherit.
    pub on_worker: &'a (dyn Fn() + Sync),
}

/// `tables`, Campaign among them, each once in table order.
fn selection(tables: &[TableId]) -> Vec<TableId> {
    let mut chosen = [false; crate::rows::TABLE_COUNT];
    chosen[TableId::Campaign.index()] = true;
    for table in tables {
        chosen[table.index()] = true;
    }
    TableId::ALL
        .into_iter()
        .filter(|table| chosen[table.index()])
        .collect()
}

/// Refuses an hour whose complete Campaign table holds anything but the recording's campaign.
pub(crate) fn validate_campaign(frame: &DecodedFrame, campaign_id: &[u8]) -> io::Result<()> {
    if let TableView::Complete(rows) = &*frame.tables[TableId::Campaign.index()]
        && (rows.len() != 1
            || !matches!(rows, TableRows::Campaign(values) if values[0].id == campaign_id))
    {
        return Err(invalid_static(
            "frame campaign identity differs from replay header",
        ));
    }
    Ok(())
}

/// Checks the Campaign of the hours a walk applies without building their frames: an hour that
/// changes the table, or the first after a restart or an hour built in full, is built for it
/// alone.
struct CampaignCheck {
    views: Views,
    status: Option<Status>,
}

impl CampaignCheck {
    fn new(replay: &Replay) -> io::Result<Self> {
        Ok(Self {
            views: Views::new(&[TableId::Campaign], replay.format, replay.capture_mode)?,
            status: None,
        })
    }

    /// The walk starts again from a restart, or built an hour in full, which this check did not see.
    fn restart(&mut self) {
        self.status = None;
    }

    fn pass(&mut self, state: &State, manifest: &Manifest<'_>, campaign_id: &[u8]) -> io::Result<()> {
        let table = TableId::Campaign.index();
        let status = manifest.statuses.get(table).copied();
        if self.status.is_none()
            || self.status != status
            || manifest.sections.iter().any(|section| section.table == table)
        {
            self.status = status;
            let frame = self.views.advance(state, manifest, true)?;
            validate_campaign(&frame, campaign_id)?;
        }
        Ok(())
    }
}

impl Replay {
    /// Each hour of `blocks`, in the order given, with `tables` (and Campaign) built: the
    /// recorded facts of each hour as the row recording shaped them and its frame. Blocks of
    /// one segment in rising order share one walk. `visit` stops the read by returning false.
    pub fn visit_blocks(
        &self,
        blocks: &[&BlockIndex],
        tables: &[TableId],
        parallel: Option<&Parallel<'_>>,
        mut visit: impl FnMut(&BlockIndex, &RecordedFrame, &DecodedFrame) -> io::Result<bool>,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        let _ = parallel;
        if blocks.is_empty() {
            return Ok(());
        }
        let selected = selection(tables);
        let recording = self.recording.selecting(&selected)?;
        let mut walker = Walker::new(&recording)?;
        let mut state = recording.state()?;
        let mut views = Views::new(&selected, self.format, self.capture_mode)?;
        let mut campaign = CampaignCheck::new(self)?;
        let mut at = 0;
        while at < blocks.len() {
            let segment_at = self.segment_of(blocks[at])?;
            let mut end = at + 1;
            while end < blocks.len()
                && self.segment_of(blocks[end])? == segment_at
                && (blocks[end].run, blocks[end].page) > (blocks[end - 1].run, blocks[end - 1].page)
            {
                end += 1;
            }
            let group = &blocks[at..end];
            let segment = &recording.segments()[segment_at];
            check()?;
            walker.begin(segment, &mut state)?;
            campaign.restart();
            let mut target = 0;
            let mut stopped = false;
            walker.walk(segment, &mut state, |state, manifest, _| {
                let block = group[target];
                if manifest.hour < block.first_hour {
                    campaign.pass(state, manifest, &self.campaign_id)?;
                    return Ok(true);
                }
                check()?;
                let first = manifest.hour == block.first_hour;
                let (mut frame, mut recorded) = views.advance_recorded(state, manifest, first)?;
                validate_campaign(&frame, &self.campaign_id)?;
                // Read as timelines, an hour is named by the timeline its page was clipped for.
                frame.branch = block.branch;
                recorded.branch = block.branch;
                campaign.restart();
                if !visit(block, &recorded, &frame)? {
                    stopped = true;
                    return Ok(false);
                }
                if manifest.hour == block.last_hour {
                    target += 1;
                    if target == group.len() {
                        return Ok(false);
                    }
                }
                Ok(true)
            })?;
            if stopped {
                return Ok(());
            }
            if target != group.len() {
                return Err(invalid_static("a block's walk ended before its last hour"));
            }
            at = end;
        }
        Ok(())
    }

    /// One block's hours with `tables` (and Campaign) built, as [`Self::visit_blocks`] reads
    /// them: each hour's recorded facts and frame.
    pub fn read_block_alone(
        &self,
        block: &BlockIndex,
        tables: &[TableId],
        mut visit: impl FnMut(&RecordedFrame, &DecodedFrame) -> io::Result<()>,
        check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        self.visit_blocks(
            &[block],
            tables,
            None,
            |_, recorded, frame| visit(recorded, frame).map(|()| true),
            check,
        )
    }

    /// The recorded facts of every hour of `block` with `tables` (and Campaign): which tables
    /// each hour changed, its changed rows' keys and fields, its metrics. They carry no values:
    /// a frame is built by [`Self::read_block_alone`], never from them.
    pub fn read_block_facts(
        &self,
        block: &BlockIndex,
        tables: &[TableId],
        check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<Vec<RecordedFrame>> {
        let mut facts = Vec::new();
        self.read_block_alone(
            block,
            tables,
            |recorded, _| {
                engine_allocation::reserve(&mut facts, 1)?;
                facts.push(recorded.clone());
                Ok(())
            },
            check,
        )?;
        Ok(facts)
    }

    /// The frame of `block`'s first hour with `tables` (and Campaign).
    pub fn block_first_frame(&self, block: &BlockIndex, tables: &[TableId]) -> io::Result<DecodedFrame> {
        let mut first = None;
        self.visit_blocks(
            &[block],
            tables,
            None,
            |_, _, frame| {
                first = Some(frame.clone());
                Ok(false)
            },
            || Ok(()),
        )?;
        first.ok_or_else(|| invalid_static("a block holds no hour"))
    }

    /// Every hour of every block of `branch`, in order, as [`Self::visit_blocks`] reads them.
    pub fn visit_branch(
        &self,
        branch: BranchId,
        tables: &[TableId],
        visit: impl FnMut(&BlockIndex, &RecordedFrame, &DecodedFrame) -> io::Result<bool>,
        check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        let blocks: Vec<_> = self.index.iter().filter(|block| block.branch == branch).collect();
        self.visit_blocks(&blocks, tables, None, visit, check)
    }

    /// The block holding `hour` of `branch`, when one does.
    pub fn block_at(&self, branch: BranchId, hour: Hour) -> Option<&BlockIndex> {
        self.index
            .iter()
            .find(|block| block.branch == branch && block.first_hour <= hour && hour <= block.last_hour)
    }

    pub fn seek(&self, branch: BranchId, hour: Hour) -> io::Result<SeekResult> {
        self.seek_selected(branch, hour, None)
    }

    /// [`Self::seek`] building only `tables` (and Campaign, which every hour is checked by).
    pub fn seek_selected(
        &self,
        branch: BranchId,
        hour: Hour,
        tables: Option<&[TableId]>,
    ) -> io::Result<SeekResult> {
        self.seek_selected_with(branch, hour, tables, || Ok(()))
    }

    /// [`Self::seek_selected`] with the caller's cancellation, checked as the walk goes.
    pub fn seek_selected_with(
        &self,
        branch: BranchId,
        hour: Hour,
        tables: Option<&[TableId]>,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<SeekResult> {
        let Some(block) = self.block_at(branch, hour).copied() else {
            return Ok(SeekResult::Gap {
                branch,
                hour,
                reason: self.gap_reason(branch, hour)?,
            });
        };
        let selected = selection(tables.unwrap_or(&TableId::ALL));
        let recording = self.recording.selecting(&selected)?;
        let segment = recording.segments()[self.segment_of(&block)?].clone();
        let mut walker = Walker::new(&recording)?;
        let mut state = recording.state()?;
        let mut views = Views::new(&selected, self.format, self.capture_mode)?;
        let mut found = None;
        let mut campaign = CampaignCheck::new(self)?;
        check()?;
        walker.begin(&segment, &mut state)?;
        walker.walk(&segment, &mut state, |state, manifest, _| {
            check()?;
            if manifest.hour < hour {
                campaign.pass(state, manifest, &self.campaign_id)?;
                return Ok(true);
            }
            let mut frame = views.advance(state, manifest, true)?;
            validate_campaign(&frame, &self.campaign_id)?;
            frame.branch = branch;
            found = Some(frame);
            Ok(false)
        })?;
        match found {
            Some(frame) => Ok(SeekResult::Frame(engine_allocation::boxed(frame)?)),
            None => Err(invalid_static("a block's walk ended before the hour sought")),
        }
    }
}

/// What a cursor's walk sends: the frame of the hour asked, or none past its segment's end.
type Reply = io::Result<Option<DecodedFrame>>;

/// A walk going on in its own thread, from one segment's start, waiting between hours asked.
struct Walk {
    ask: mpsc::SyncSender<Option<Hour>>,
    reply: mpsc::Receiver<Reply>,
    thread: Option<thread::JoinHandle<()>>,
    segment: usize,
    branch: BranchId,
    /// The hour it built last; it goes on only to later hours of its segment.
    last: Option<Hour>,
}

impl Drop for Walk {
    fn drop(&mut self) {
        let _ = self.ask.try_send(None);
        // Its thread ends at its next hour or ask; it holds only its own state.
        drop(self.thread.take());
    }
}

/// Frames of a recording's hours asked in turn: a later hour of the same segment goes on
/// from the hour before's walk and views, any other starts a walk at its segment. For playback
/// stepping forward, an hour at a time or by jumps.
pub struct HourCursor {
    replay: Arc<Replay>,
    tables: Vec<TableId>,
    changes: bool,
    walk: Option<Walk>,
}

impl HourCursor {
    /// A cursor building `tables` (and Campaign); with `changes`, complete views carry change
    /// facts and field stamps ([`Views::with_change_facts`]).
    pub fn new(replay: Arc<Replay>, tables: &[TableId], changes: bool) -> Self {
        Self {
            replay,
            tables: selection(tables),
            changes,
            walk: None,
        }
    }

    pub fn replay(&self) -> &Arc<Replay> {
        &self.replay
    }

    /// The hour it built last, when a later hour of its segment would go on from it.
    pub fn last(&self) -> Option<(BranchId, Hour)> {
        let walk = self.walk.as_ref()?;
        Some((walk.branch, walk.last?))
    }

    /// Whether a seek of `hour` of `branch` goes on from the hour it built last.
    pub fn continues(&self, branch: BranchId, hour: Hour) -> bool {
        let Some(block) = self.replay.block_at(branch, hour) else {
            return false;
        };
        let Ok(segment) = self.replay.segment_of(block) else {
            return false;
        };
        self.walk.as_ref().is_some_and(|walk| {
            walk.segment == segment && walk.branch == branch && walk.last.is_some_and(|last| last < hour)
        })
    }

    /// Lets go of its walk and what it holds.
    pub fn release(&mut self) {
        self.walk = None;
    }

    /// The frame of `hour` of `branch`, or the gap holding it.
    pub fn seek(&mut self, branch: BranchId, hour: Hour) -> io::Result<SeekResult> {
        let Some(block) = self.replay.block_at(branch, hour).copied() else {
            return Ok(SeekResult::Gap {
                branch,
                hour,
                reason: self.replay.gap_reason(branch, hour)?,
            });
        };
        let segment = self.replay.segment_of(&block)?;
        let goes_on = self.walk.as_ref().is_some_and(|walk| {
            walk.segment == segment && walk.branch == branch && walk.last.is_some_and(|last| last < hour)
        });
        if !goes_on {
            self.walk = Some(self.start(segment, branch)?);
        }
        let walk = self.walk.as_mut().expect("a walk under way");
        let asked = walk.ask.send(Some(hour)).is_ok();
        let reply = if asked {
            walk.reply.recv().unwrap_or_else(|_| Err(io::Error::other("the cursor's walk ended")))
        } else {
            Err(io::Error::other("the cursor's walk ended"))
        };
        match reply {
            Ok(Some(frame)) => {
                walk.last = Some(hour);
                Ok(SeekResult::Frame(engine_allocation::boxed(frame)?))
            }
            Ok(None) => {
                self.walk = None;
                Err(invalid_static("a block's walk ended before the hour sought"))
            }
            Err(error) => {
                self.walk = None;
                Err(error)
            }
        }
    }

    fn start(&self, segment: usize, branch: BranchId) -> io::Result<Walk> {
        let (ask, asks) = mpsc::sync_channel::<Option<Hour>>(1);
        let (replies, reply) = mpsc::sync_channel::<Reply>(1);
        let replay = Arc::clone(&self.replay);
        let tables = self.tables.clone();
        let changes = self.changes;
        let priority = job_priority::JobPriority::current();
        let thread = thread::Builder::new()
            .name("recording cursor".into())
            .spawn(move || {
                priority.enter(job_priority::Level::Normal);
                let walked = walk_asked(&replay, &tables, changes, segment, branch, &asks, &replies);
                if let Err(error) = walked {
                    let _ = replies.send(Err(error));
                } else {
                    let _ = replies.send(Ok(None));
                }
            })?;
        Ok(Walk {
            ask,
            reply,
            thread: Some(thread),
            segment,
            branch,
            last: None,
        })
    }
}

/// Walks segment `segment` of `replay`, answering each hour asked with its frame named by
/// `branch`, until asked for none or the segment ends.
fn walk_asked(
    replay: &Replay,
    tables: &[TableId],
    changes: bool,
    segment: usize,
    branch: BranchId,
    asks: &mpsc::Receiver<Option<Hour>>,
    replies: &mpsc::SyncSender<Reply>,
) -> io::Result<()> {
    let recording: Recording = replay.recording.selecting(tables)?;
    let segment = recording.segments()[segment].clone();
    let mut walker = Walker::new(&recording)?;
    let mut state: State = recording.state()?;
    let mut views = Views::new(tables, replay.format, replay.capture_mode)?;
    if changes {
        views = views.with_change_facts();
    }
    let Ok(Some(mut wanted)) = asks.recv() else {
        return Ok(());
    };
    let mut built = false;
    let mut campaign = CampaignCheck::new(replay)?;
    walker.begin(&segment, &mut state)?;
    walker.walk(&segment, &mut state, |state, manifest, _| {
        if manifest.hour < wanted {
            campaign.pass(state, manifest, &replay.campaign_id)?;
            if built {
                views.note(state, manifest, false)?;
            }
            return Ok(true);
        }
        views.note(state, manifest, !built)?;
        let frame = views.build(
            state,
            (branch, manifest.hour),
            metrics(manifest),
            None,
        )?;
        validate_campaign(&frame, &replay.campaign_id)?;
        campaign.restart();
        built = true;
        if replies.send(Ok(Some(frame))).is_err() {
            return Ok(false);
        }
        match asks.recv() {
            Ok(Some(next)) if next > manifest.hour => {
                wanted = next;
                Ok(true)
            }
            _ => Ok(false),
        }
    })?;
    Ok(())
}
