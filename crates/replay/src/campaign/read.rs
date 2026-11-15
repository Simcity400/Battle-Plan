//! A campaign recording's hours as the row decoder's frames ([`Frames`]): a block's first hour
//! is a keyframe, each table of it a Snapshot, so a block's hours are the same whichever blocks
//! are read with it; every hour's views go on from the hour before's. A later hour of a run goes on
//! from where each table stands unless a restore of the table lies between; any other hour is a
//! seek: each table restored at its last restore before the hour and advanced to it.

use super::{BlockIndex, Replay, SeekResult};
use crate::{
    BranchId, Hour,
    codec::invalid_static,
    columnar_frames::{Frames, Pool},
    frame::{DecodedFrame, RecordedFrame},
    rows::TableId,
};
use std::{io, sync::Arc};

/// How a view build spreads its tables over threads.
pub struct Parallel<'a> {
    /// Worker threads at most; one builds on the calling thread.
    pub workers: usize,
    /// Runs first on each worker, for the caller's thread setup such as a lower scheduling
    /// priority, which a spawned thread does not inherit.
    pub on_worker: &'a (dyn Fn() + Sync),
}

impl Replay {
    /// `block`'s run and the run ordinal of hour `hour` in it.
    fn ordinal(&self, block: &BlockIndex, hour: Hour) -> io::Result<u32> {
        self.recording.run(block.run)?.ordinal(hour).ok_or_else(|| invalid_static("a block's hour lies outside its run"))
    }

    /// Each hour of `blocks`, in the order given, with `tables` built: the recorded facts of
    /// each hour as the row recording shaped them and its frame. `visit` stops the read by
    /// returning false.
    pub fn visit_blocks(
        &self,
        blocks: &[&BlockIndex],
        tables: &[TableId],
        mut visit: impl FnMut(&BlockIndex, &RecordedFrame, &DecodedFrame) -> io::Result<bool>,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        if blocks.is_empty() {
            return Ok(());
        }
        let pool = self.pool.as_deref();
        let mut frames = Frames::new(&self.recording, tables)?;
        for block in blocks {
            check()?;
            let first = self.ordinal(block, block.first_hour)?;
            let last = self.ordinal(block, block.last_hour)?;
            Pool::on(pool, || frames.go_to(block.run, first))?;
            frames.keyframe();
            for ordinal in first..=last {
                if ordinal > first {
                    check()?;
                    Pool::on(pool, || frames.go_to(block.run, ordinal))?;
                }
                // Read as timelines, an hour is named by the timeline its page was clipped for.
                let (frame, recorded) = Pool::on(pool, || frames.frame_recorded(block.branch))?;
                if !visit(block, &recorded, &frame)? {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// One block's hours with `tables` built, as [`Self::visit_blocks`] reads them: each hour's
    /// recorded facts and frame.
    pub fn read_block_alone(
        &self,
        block: &BlockIndex,
        tables: &[TableId],
        mut visit: impl FnMut(&RecordedFrame, &DecodedFrame) -> io::Result<()>,
        check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        self.visit_blocks(&[block], tables, |_, recorded, frame| visit(recorded, frame).map(|()| true), check)
    }

    /// The block holding `hour` of `branch`, when one does.
    pub fn block_at(&self, branch: BranchId, hour: Hour) -> Option<&BlockIndex> {
        self.index.iter().find(|block| block.branch == branch && block.first_hour <= hour && hour <= block.last_hour)
    }

    pub fn seek(&self, branch: BranchId, hour: Hour) -> io::Result<SeekResult> {
        self.seek_selected(branch, hour, None)
    }

    /// [`Self::seek`] building only `tables`.
    pub fn seek_selected(&self, branch: BranchId, hour: Hour, tables: Option<&[TableId]>) -> io::Result<SeekResult> {
        self.seek_selected_with(branch, hour, tables, || Ok(()))
    }

    /// [`Self::seek_selected`] with the caller's cancellation, checked before the read.
    pub fn seek_selected_with(&self, branch: BranchId, hour: Hour, tables: Option<&[TableId]>, mut check: impl FnMut() -> io::Result<()>) -> io::Result<SeekResult> {
        let Some(block) = self.block_at(branch, hour).copied() else {
            return Ok(SeekResult::Gap { branch, hour, reason: self.gap_reason(branch, hour)? });
        };
        check()?;
        let ordinal = self.ordinal(&block, hour)?;
        let mut frames = Frames::new(&self.recording, tables.unwrap_or(&TableId::ALL))?;
        let frame = Pool::on(self.pool.as_deref(), || frames.seek(block.run, ordinal).and_then(|()| frames.frame(branch)))?;
        Ok(SeekResult::Frame(engine_allocation::boxed(frame)?))
    }
}

/// Frames of a recording's hours asked in turn: a later hour of the run goes on from the hour
/// built last, any other is a seek. For playback stepping forward, an hour at a time or by jumps.
pub struct HourCursor {
    replay: Arc<Replay>,
    tables: Vec<TableId>,
    /// Its frames' complete views carry change facts and field stamps.
    changes: bool,
    /// The hour built last and its run.
    last: Option<(BranchId, Hour, u32)>,
    frames: Option<Frames<Held>>,
}

/// The recording of the replay a cursor holds, for its frames to read.
struct Held(Arc<Replay>);

impl std::ops::Deref for Held {
    type Target = crate::columnar_frames::Recording;

    fn deref(&self) -> &Self::Target {
        &self.0.recording
    }
}

impl HourCursor {
    /// A cursor building `tables`, its complete views carrying change facts and field stamps
    /// with `changes`: an hour stepped to from the one built last names what it changed.
    pub fn new(replay: Arc<Replay>, tables: &[TableId], changes: bool) -> Self {
        Self { replay, tables: tables.to_vec(), changes, last: None, frames: None }
    }

    pub fn replay(&self) -> &Arc<Replay> {
        &self.replay
    }

    /// The hour it built last.
    pub fn last(&self) -> Option<(BranchId, Hour)> {
        self.last.map(|(branch, hour, _)| (branch, hour))
    }

    /// Whether a seek of `hour` of `branch` goes on from the hour it built last: that hour or a
    /// later one of its run.
    pub fn continues(&self, branch: BranchId, hour: Hour) -> bool {
        self.last.is_some_and(|(held, last, run)| held == branch && hour >= last && self.replay.block_at(branch, hour).is_some_and(|block| block.run == run))
    }

    /// Lets go of its readers and what they hold.
    pub fn release(&mut self) {
        self.frames = None;
        self.last = None;
    }

    /// The frame of `hour` of `branch`, or the gap holding it.
    pub fn seek(&mut self, branch: BranchId, hour: Hour) -> io::Result<SeekResult> {
        let Some(block) = self.replay.block_at(branch, hour).copied() else {
            return Ok(SeekResult::Gap { branch, hour, reason: self.replay.gap_reason(branch, hour)? });
        };
        let ordinal = self.replay.ordinal(&block, hour)?;
        if self.frames.is_none() {
            let frames = Frames::new(Held(Arc::clone(&self.replay)), &self.tables)?;
            self.frames = Some(if self.changes { frames.with_change_facts() } else { frames });
        }
        let frames = self.frames.as_mut().expect("made");
        let read = Pool::on(self.replay.pool.as_deref(), || frames.go_to(block.run, ordinal).and_then(|()| frames.frame(branch)));
        match read {
            Ok(frame) => {
                self.last = Some((branch, hour, block.run));
                Ok(SeekResult::Frame(engine_allocation::boxed(frame)?))
            }
            Err(error) => {
                self.release();
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        branches::{BranchReason, BranchStart, Parentage},
        cells::TableReader,
        frame::{Frame, Metrics, SharedView, TableUpdate, TableView},
        identity::{CaptureMode, Identity, SampleBoundary, SessionStart, Verification},
        rows::{CampaignRow, ClockRow, TableRows},
        spool::Record,
    };
    use replay_columnar::envelope::UNIT_ORDINALS;

    const CAMPAIGN: &[u8] = b"hour cursor";
    const BLOCK: Hour = UNIT_ORDINALS as Hour;
    const TABLES: [TableId; 2] = [TableId::Clock, TableId::Campaign];
    /// Branch 1's first run: seven blocks, the last short; the writer restores each table at
    /// block 5's end, six blocks after its first.
    const FIRST: std::ops::Range<Hour> = 10..10 + 6 * BLOCK + 40;
    /// Its second run, after a hole.
    const LATER: Hour = 10 + 7 * BLOCK;

    /// One hour of branch 1: its clock, the campaign, every other table empty.
    fn frame(hour: Hour) -> Frame {
        Frame {
            branch: 1,
            hour,
            keyframe: true,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                let rows = match TableId::ALL[index] {
                    TableId::Clock => TableRows::Clock(vec![ClockRow { hour, ..Default::default() }]),
                    TableId::Campaign => TableRows::Campaign(vec![CampaignRow { id: CAMPAIGN.to_vec() }]),
                    table => TableRows::empty(table),
                };
                match crate::frame::current_absence(rows.table()) {
                    Some(TableView::NotRecorded { mode }) => TableUpdate::NotRecorded { mode },
                    Some(TableView::Unavailable { reason }) => TableUpdate::Unavailable { reason },
                    None => TableUpdate::Snapshot(rows),
                    _ => unreachable!("absence is an omission or a named refusal"),
                }
            }),
        }
    }

    fn replay() -> Replay {
        let session = SessionStart {
            id: 1,
            process_id: 1,
            process_start_time: 1,
            qpc_frequency: 100,
            sample_boundary: SampleBoundary::EndOfHour,
            capture_mode: CaptureMode::Full,
            identity: Identity {
                game_version: "synthetic".into(),
                exe_hash: [1; 32],
                exe_size: 1,
                vanilla_hash: [2; 32],
                dlc: Vec::new(),
                mods: Vec::new(),
                coverage_policy: 1,
                language: "english".into(),
                recorder_version: "test".into(),
                collector_hash: [3; 32],
                verification: Verification::Unchecked,
            },
        };
        let branch = BranchStart { id: 1, session: 1, loaded_hour: Some(FIRST.start - 1), first_hour: FIRST.start, parent: Parentage::Unknown, reason: BranchReason::SessionStart };
        let mut records = vec![Record::SessionStart(session), Record::BranchStart(branch)];
        records.extend(FIRST.chain(LATER..LATER + 30).map(|hour| Record::Frame(frame(hour))));
        Replay::from_records(CAMPAIGN.to_vec(), &[records]).unwrap()
    }

    /// Each table's restores so far.
    fn restores(cursor: &HourCursor) -> Vec<u32> {
        TABLES.iter().map(|&table| cursor.frames.as_ref().and_then(|frames| frames.reader(table)).map_or(0, TableReader::restores)).collect()
    }

    /// A cursor moved forward a day at a time through a run, across its block ends and across a
    /// table's restore, builds at every stop the frame a fresh seek builds. It goes on from the
    /// hour built last for that hour and any later one of its run, never for an earlier hour or
    /// another run, and a table restores only across a block end it was restored at.
    #[test]
    fn a_cursor_moved_forward_by_days_goes_on_and_builds_what_a_seek_builds() {
        let replay = Arc::new(replay());
        let run = replay.block_at(1, FIRST.start).unwrap().run;
        assert_ne!(replay.block_at(1, LATER).unwrap().run, run, "a hole starts another run");
        let block_of = |hour: Hour| ((hour - FIRST.start) / BLOCK) as u32;
        let restored = |table: TableId, block: u32| replay.recording.table_entry(run, block, table.index()).unwrap().unit.restore.is_some();
        let sampled = |frame: &DecodedFrame| frame.tables.iter().map(SharedView::sampled_at).collect::<Vec<_>>();
        let mut cursor = HourCursor::new(Arc::clone(&replay), &TABLES, false);
        let mut crossed = [false; 2];
        let mut previous: Option<Hour> = None;
        for hour in FIRST.skip(5).step_by(24) {
            if let Some(last) = previous {
                assert!(cursor.continues(1, hour), "hour {hour} goes on from {last}");
            }
            let before = restores(&cursor);
            let SeekResult::Frame(frame) = cursor.seek(1, hour).unwrap() else { panic!("hour {hour} is a gap") };
            let SeekResult::Frame(sought) = replay.seek_selected(1, hour, Some(&TABLES)).unwrap() else { panic!("hour {hour} is a gap") };
            assert_eq!(frame.tables, sought.tables, "hour {hour}");
            assert_eq!(sampled(&frame), sampled(&sought), "hour {hour}");
            let Some(last) = previous else {
                previous = Some(hour);
                continue;
            };
            let after = restores(&cursor);
            for (at, &table) in TABLES.iter().enumerate() {
                let across = (block_of(last)..block_of(hour)).any(|block| restored(table, block));
                if block_of(last) != block_of(hour) {
                    crossed[usize::from(across)] = true;
                }
                assert_eq!(after[at] - before[at], u32::from(across), "{table:?} from hour {last} to {hour}");
            }
            previous = Some(hour);
        }
        assert_eq!(crossed, [true, true], "the walk crosses a block end with no restore and one with");
        let last = previous.unwrap();
        assert!(cursor.continues(1, last));
        assert!(!cursor.continues(1, last - 1));
        assert!(!cursor.continues(1, LATER));
        assert!(!cursor.continues(2, last + 1));
    }

    /// A walk of a branch's blocks, across block ends and from one run to the next, hands at every
    /// hour the frame and recorded facts each block read alone hands: views, sample hours, updates,
    /// and a keyframe at each block's first hour and no other.
    #[test]
    fn a_walk_across_block_ends_hands_the_frames_each_block_read_alone_hands() {
        let replay = replay();
        let blocks: Vec<&BlockIndex> = replay.index.iter().filter(|block| block.branch == 1).collect();
        assert!(blocks.len() > 2 && blocks[0].run != blocks[blocks.len() - 1].run, "blocks of two runs");
        let mut alone = Vec::new();
        for block in &blocks {
            replay
                .read_block_alone(
                    block,
                    &TABLES,
                    |recorded, frame| {
                        alone.push((recorded.clone(), frame.clone()));
                        Ok(())
                    },
                    || Ok(()),
                )
                .unwrap();
        }
        let mut at = 0;
        replay
            .visit_blocks(
                &blocks,
                &TABLES,
                |block, recorded, frame| {
                    let (want_recorded, want) = &alone[at];
                    let hour = frame.hour;
                    assert_eq!(hour, want.hour);
                    assert_eq!(recorded.keyframe, want_recorded.keyframe, "hour {hour}");
                    assert_eq!(recorded.keyframe, hour == block.first_hour, "hour {hour}");
                    for table in TABLES {
                        let index = table.index();
                        assert_eq!(frame.tables[index], want.tables[index], "{table:?} at hour {hour}");
                        assert_eq!(frame.tables[index].sampled_at(), want.tables[index].sampled_at(), "{table:?} at hour {hour}");
                        assert_eq!(recorded.tables[index], want_recorded.tables[index], "{table:?} at hour {hour}");
                    }
                    at += 1;
                    Ok(true)
                },
                || Ok(()),
            )
            .unwrap();
        assert_eq!(at, alone.len());
    }

    /// A cursor made with change facts hands them through its frames: the next hour, within a
    /// block or across its end, names the clock row it wrote and leaves the campaign alone; a jump
    /// names none; a cursor made without them never does.
    #[test]
    fn a_cursor_with_change_facts_hands_them_through_its_frames() {
        use crate::frame::{HourInput, TableChanges};
        let replay = Arc::new(replay());
        let at = |cursor: &mut HourCursor, hour: Hour| {
            let SeekResult::Frame(frame) = cursor.seek(1, hour).unwrap() else { panic!("hour {hour} is a gap") };
            *frame
        };
        let since = |frame: &DecodedFrame, before: &DecodedFrame, table: TableId| {
            let identity = before.tables[table.index()].identity();
            match HourInput::new(frame).changes().since(table, Some(&identity), u64::MAX) {
                TableChanges::Rows(facts) => Some((facts.changed().len(), facts.changed().iter().any(|row| row.inserted), facts.deleted().len())),
                TableChanges::Unchanged => Some((0, false, 0)),
                TableChanges::Discontinuity(_) => None,
            }
        };
        let mut cursor = HourCursor::new(Arc::clone(&replay), &TABLES, true);
        let first = at(&mut cursor, FIRST.start + 3);
        let next = at(&mut cursor, FIRST.start + 4);
        assert!(matches!(
            HourInput::new(&next).changes().since(TableId::Clock, Some(&first.tables[TableId::Clock.index()].identity()), u64::MAX),
            TableChanges::Rows(_)
        ));
        assert_eq!(since(&next, &first, TableId::Clock), Some((1, false, 0)), "the next hour writes its clock row");
        assert!(SharedView::ptr_eq(&next.tables[TableId::Campaign.index()], &first.tables[TableId::Campaign.index()]));
        assert_eq!(since(&next, &first, TableId::Campaign), Some((0, false, 0)));
        let jumped = at(&mut cursor, FIRST.start + 6);
        assert_eq!(since(&jumped, &next, TableId::Clock), None, "a jump of two hours");
        let before = at(&mut cursor, FIRST.start + BLOCK - 1);
        let after = at(&mut cursor, FIRST.start + BLOCK);
        assert_eq!(since(&after, &before, TableId::Clock), Some((1, false, 0)), "the next hour across a block end");

        let mut plain = HourCursor::new(replay, &TABLES, false);
        let first = at(&mut plain, FIRST.start + 3);
        let next = at(&mut plain, FIRST.start + 4);
        assert!(next.tables[TableId::Clock.index()].changes_since(&first.tables[TableId::Clock.index()].identity()).is_none());
        assert_eq!(since(&next, &first, TableId::Clock), None, "a cursor without change facts names none");
    }
}
