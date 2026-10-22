//! Air force and navy size prepared block by block. A pre-pass worker reads
//! the block alone and steps a scan of its own through it: the scan's state
//! right after the first hour it holds nothing a carried scan would hold
//! from earlier hours ([`Family::settled`]: the block's first hour for navy;
//! for air, the first hour the block reads or forgets variant names), which
//! is what decides its later hours, and its counters then ([`Family::tail`]);
//! then each later hour's rows (the runs that changed and the rows removed),
//! and the scan at the block's end. The scan step reads the recording alone,
//! so the worker does it; the rows' projections name parts through the
//! recorded content, so the fold makes them, each hour apart from the
//! others.
//!
//! The fold observes the block's first hour on the carried state, as hourly
//! preparation does, and the caller observes the hours after it through the
//! tail's hour by hour. When the carried scan then decides every later hour
//! as the worker's scan did ([`Family::continues`]), every later hour's rows
//! are the worker's: the fold renumbers what the worker's scan numbered in
//! its own order as the carried scan numbers it ([`Family::numbering`]:
//! navy's ship classes), projects the rows, publishes each hour's rows on
//! the carried state (keys, compositions and parts from the carried series),
//! and carries the scan to the block's end ([`Family::carry`]). Otherwise,
//! or where the worker's scan was never settled, the caller observes the
//! rest of the block hour by hour.

use super::{Family, Prepared, Projected, Rows, project, step_rows, step_scan};
use dashboard_base::prepared::priority::{JobPriority, Level};
use dashboard_base::{
    prepared::{
        frame_changes::FrameChanges,
        prepass::{Extract, Held, HeldBytes},
    },
    source::Source,
};
use dashboard_core::config::LoadContext;
use replay::container::Replay;
use replay_frame::frame::DecodedFrame;
use std::{any::Any, io, sync::atomic::AtomicBool};

/// A block's facts for the fold.
pub struct BlockFacts<F: Family> {
    /// The block's first frame, observed on the carried state.
    first: DecodedFrame,
    /// The worker's scan right after the first hour it was settled, and
    /// that hour; none where it never was.
    tail: Option<(i32, F::Tail)>,
    /// Each hour's rows after the tail's, in order.
    hours: Vec<Rows<F>>,
    /// The hour whose scan step failed, and why: nothing after it was read.
    failed: Option<(i32, io::Error)>,
    /// The worker's scan at the block's end.
    scan: F::Scan,
    /// How the worker's scan's numbers name the carried scan's, once
    /// [`Prepared::fold_continues`] made it.
    numbering: Option<F::Numbering>,
    /// The block's last frame, which hourly preparation resumes the next
    /// block of the branch from.
    pub last: Option<DecodedFrame>,
}

/// A pre-pass worker's extract of one block.
pub struct BlockExtract<F: Family> {
    scan: F::Scan,
    first: Option<DecodedFrame>,
    tail: Option<(i32, F::Tail)>,
    hours: Vec<Rows<F>>,
    failed: Option<(i32, io::Error)>,
    last: Option<DecodedFrame>,
}

impl<F: Family> BlockExtract<F> {
    pub fn start() -> Box<Self> {
        Box::new(Self {
            scan: F::Scan::default(),
            first: None,
            tail: None,
            hours: Vec::new(),
            failed: None,
            last: None,
        })
    }
}

impl<F: Family> Extract for BlockExtract<F>
where
    F::Scan: Send + 'static,
    F::Raw: Send + 'static,
    F::Scanned: Send + 'static,
    F::Tail: 'static,
    F: 'static,
{
    fn observe(
        &mut self,
        replay: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        self.last = Some(frame.clone());
        if self.failed.is_some() {
            return Ok(());
        }
        // Cancellation is checked between blocks.
        let running = AtomicBool::new(false);
        if self.first.is_none() {
            step_scan::<F>(&mut self.scan, frame, replay.format, &running, changes)?;
            self.first = Some(frame.clone());
        } else {
            let rows = if self.tail.is_some() {
                step_rows::<F>(
                    &mut self.scan,
                    true,
                    frame,
                    replay.format,
                    &running,
                    changes,
                )
                .map(Some)
            } else {
                step_scan::<F>(&mut self.scan, frame, replay.format, &running, changes)
                    .map(|()| None)
            };
            match rows {
                // The fold observes the hours through the tail's itself.
                Ok(Some(rows)) => self.hours.push(rows),
                Ok(None) => {}
                Err(error) => {
                    self.failed = Some((frame.hour, error));
                    return Ok(());
                }
            }
        }
        if self.tail.is_none() && F::settled(&self.scan) {
            self.tail = Some((frame.hour, F::tail(&self.scan, frame.hour)));
        }
        Ok(())
    }

    /// What the block's facts hold: the first and last frames (each view
    /// once), the scan's tail, each hour's rows after it
    /// (their runs, removed tags and projection bases) and the scan at hand,
    /// which is the scan at the block's end the facts keep; each allocation
    /// shared among them once.
    fn bytes(&self) -> u64 {
        let mut held = HeldBytes::default();
        if let Some(frame) = &self.first {
            held.frame(frame);
        }
        if let Some((_, tail)) = &self.tail {
            held.value(tail);
        }
        if let Some(last) = &self.last {
            held.frame(last);
        }
        held.heap(&self.hours).value(&self.scan).total()
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        let Self {
            scan,
            first,
            tail,
            hours,
            failed,
            last,
        } = *self;
        let first = first.ok_or_else(|| io::Error::other("Force block has no first hour"))?;
        Ok(Box::new(BlockFacts::<F> {
            first,
            tail,
            hours,
            failed,
            scan,
            numbering: None,
            last,
        }))
    }
}

/// One hour's rows: each channel's runs by tag, the tags removed and each
/// channel's projection base.
impl<F: Family> Held for Rows<F> {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.observed)
            .heap(&self.removed)
            .heap(&self.bases);
    }
}

impl<F: Family> Rows<F> {
    /// The rows and projection bases renumbered as the carried scan numbers
    /// them.
    fn renumber(&mut self, numbering: &F::Numbering) {
        for rows in &mut self.observed {
            for raw in rows.values_mut() {
                F::renumber(numbering, raw);
            }
        }
        for base in self.bases.iter_mut().flatten() {
            F::renumber_base(numbering, base);
        }
    }
}

impl<F: Family> BlockFacts<F> {
    /// The block's first hour.
    pub fn first_hour(&self) -> i32 {
        self.first.hour
    }

    /// The hour of the worker's tail, after which the fold may publish the
    /// worker's rows; none where its scan was never settled.
    pub fn tail_hour(&self) -> Option<i32> {
        self.tail.as_ref().map(|&(hour, _)| hour)
    }

    /// The hours after the tail's the worker stepped, failed or not.
    #[cfg(any(test, feature = "test-support"))]
    pub fn later_hours(&self) -> usize {
        self.hours.len() + usize::from(self.failed.is_some())
    }
}

impl<F: Family> Prepared<F>
where
    F::Raw: Send,
    F::Scanned: Send,
    F::Part: Send,
{
    /// Observe the block's first hour on the carried state, with the change
    /// facts a tracker of the block alone finds there, as hourly preparation
    /// observes it: the hour and its outcome.
    pub fn fold_first(
        &mut self,
        facts: &BlockFacts<F>,
        replay: &Replay,
        source: &Source,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> (i32, io::Result<()>) {
        let changes = FrameChanges::alone(&facts.first, replay.format);
        let result = self.observe(&facts.first, replay, source, context, cancelled, &changes);
        (facts.first.hour, result)
    }

    /// Whether the carried state, having observed the block through the
    /// tail's hour, decides the rest of the block as the worker's scan did,
    /// with the worker's numbers named as the carried scan names them (kept
    /// in `facts` for the rest). False where the worker has no tail.
    pub fn fold_continues(&self, facts: &mut BlockFacts<F>) -> bool {
        let Some((_, tail)) = &facts.tail else {
            return false;
        };
        let numbering = F::numbering(&self.scan, &facts.scan);
        let continues = F::continues(&self.scan, tail, &facts.scan, &numbering);
        facts.numbering = Some(numbering);
        continues
    }

    /// Publish every hour's rows after the tail's on the carried state, each
    /// hour's outcome handed to `observed` (false once it refused the
    /// branch), and carry the scan to the block's end. The projections are made on up to
    /// `threads` threads, each hour's alone. The last hour reached.
    pub fn fold_rest(
        &mut self,
        facts: BlockFacts<F>,
        source: &Source,
        context: &LoadContext,
        threads: usize,
        observed: &mut dyn FnMut(i32, io::Result<()>) -> io::Result<bool>,
    ) -> io::Result<Option<i32>> {
        let BlockFacts {
            tail,
            mut hours,
            failed,
            scan,
            numbering,
            ..
        } = facts;
        let (_, tail) =
            tail.ok_or_else(|| io::Error::other("Force block's worker was never settled"))?;
        let numbering = numbering.unwrap_or_else(|| F::numbering(&self.scan, &scan));
        let projected = project_hours(&mut hours, &numbering, source, context, threads);
        let mut reached = None;
        for (rows, projected) in hours.into_iter().zip(projected) {
            let hour = rows.hour;
            reached = Some(hour);
            let result = self.apply(rows, projected);
            if !observed(hour, result)? {
                return Ok(reached);
            }
        }
        if let Some((hour, error)) = failed {
            observed(hour, Err(error))?;
            return Ok(Some(hour));
        }
        F::carry(&mut self.scan, tail, scan, numbering);
        Ok(reached)
    }
}

/// Each hour's rows renumbered by `numbering` and their projections, in
/// hour order, made on up to `threads` threads.
fn project_hours<F: Family>(
    hours: &mut [Rows<F>],
    numbering: &F::Numbering,
    source: &Source,
    context: &LoadContext,
    threads: usize,
) -> Vec<Projected<F>>
where
    F::Raw: Send,
    F::Scanned: Send,
    F::Part: Send,
{
    // Cancellation is checked between blocks.
    let running = AtomicBool::new(false);
    let threads = threads.clamp(1, hours.len().max(1));
    if threads == 1 {
        return hours
            .iter_mut()
            .map(|rows| {
                rows.renumber(numbering);
                project(rows, source, context, &running)
            })
            .collect();
    }
    let chunk = hours.len().div_ceil(threads);
    let running = &running;
    // Measurement only: the projections' CPU is the fold's family's.
    let cpu_family = replay_frame::cpu::family();
    std::thread::scope(|scope| {
        let handles: Vec<_> = hours
            .chunks_mut(chunk)
            .map(|rows| {
                let priority = JobPriority::current();
                scope.spawn(move || {
                    priority.enter(Level::Normal);
                    let _cpu = replay_frame::cpu::scope("fold projection", cpu_family);
                    rows.iter_mut()
                        .map(|rows| {
                            rows.renumber(numbering);
                            project(rows, source, context, running)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| match handle.join() {
                Ok(projected) => projected,
                Err(payload) => std::panic::resume_unwind(payload),
            })
            .collect()
    })
}

#[cfg(test)]
mod bytes_tests {
    use super::*;
    use crate::products::{
        air_force::prepared::Air, navy::prepared::Navy, parts_race_prepared::test_fixtures,
    };
    use dashboard_base::{
        prepared::frame_changes::FrameChangeTracker,
        test_fixtures::{CAMPAIGN, records},
    };

    #[test]
    fn the_first_force_hour_steps_the_scan_without_building_discarded_rows() {
        let frames = vec![test_fixtures::frame(100, 3)];
        let before = dashboard_base::prepared::prepass::noted("forces.rows");
        let extract = extracted::<Navy>(&frames);
        assert!(extract.first.is_some());
        assert!(extract.tail.is_some());
        assert_eq!(
            dashboard_base::prepared::prepass::noted("forces.rows"),
            before
        );
    }

    #[test]
    fn navy_block_keeps_sparse_deltas_after_its_first_hour_scan() {
        use dashboard_base::prepared::observer_tests;
        use replay_data::{rows::TableId, rows::TableRows};
        use replay_frame::frame::FrameEncoder;
        use std::sync::Arc;

        let mut frame = test_fixtures::frame(100, 32);
        let replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame.clone()])]).unwrap();
        let mut encoder = FrameEncoder::default();
        let mut tracker = FrameChangeTracker::new();
        let mut extract = BlockExtract::<Navy>::start();
        let mut oracle = <Navy as Family>::Scan::default();
        let running = AtomicBool::new(false);
        let rows_before = dashboard_base::prepared::prepass::noted("forces.rows");
        for hour in 100..103 {
            frame.hour = hour;
            if hour > 100 {
                let replay_frame::frame::TableView::Complete(TableRows::Ships(rows)) =
                    &mut *frame.tables[TableId::Ships.index()]
                else {
                    unreachable!()
                };
                if hour == 101 {
                    rows[0].strength += 1;
                } else {
                    rows[0].organisation += 1;
                }
            }
            let raw = Arc::new(encoder.encode(frame.clone(), hour == 100).unwrap());
            let changes = tracker.observe_updates(&frame, raw, replay.format);
            let before = Navy::work(&extract.scan).history_visits;
            extract.observe(&replay, &frame, &changes).unwrap();
            Navy::step(
                &mut oracle,
                &super::super::updates(&frame, Navy::TABLES),
                &frame,
                replay.format,
                &running,
                None,
            )
            .unwrap();
            if hour == 100 {
                assert!(extract.tail.is_some());
                assert!(extract.hours.is_empty());
                assert_eq!(
                    dashboard_base::prepared::prepass::noted("forces.rows"),
                    rows_before
                );
            } else {
                assert_eq!(
                    Navy::work(&extract.scan).history_visits - before,
                    usize::from(hour == 101),
                    "only a strength change revisits its owner"
                );
            }
        }
        assert_eq!(extract.hours.len(), 2);
        Navy::finish_hour(&mut extract.scan);
        Navy::finish_hour(&mut oracle);
        for channel in 0..4 {
            observer_tests::assert_reply_parity(
                &Navy::runs(&extract.scan, channel),
                &Navy::runs(&oracle, channel),
            )
            .unwrap();
        }
        observer_tests::assert_reply_parity(
            &Navy::metadata(&extract.scan),
            &Navy::metadata(&oracle),
        )
        .unwrap();
    }

    /// A block's extract of `frames`, in order.
    fn extracted<F: Family + 'static>(frames: &[DecodedFrame]) -> Box<BlockExtract<F>>
    where
        F::Scan: Send + 'static,
        F::Raw: Send + 'static,
        F::Scanned: Send + 'static,
        F::Tail: 'static,
    {
        // The recording the extract is given; it reads only the frames.
        let replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frames[0].clone()])]).unwrap();
        let mut tracker = FrameChangeTracker::new();
        let mut extract = BlockExtract::<F>::start();
        for frame in frames {
            let changes = tracker.observe(frame, replay.format);
            extract.observe(&replay, frame, &changes).unwrap();
        }
        assert_eq!(
            extract.hours.len(),
            frames.len() - 1,
            "every later hour's rows kept"
        );
        extract
    }

    /// What an extract's first and last frames alone hold.
    fn frames_alone<F: Family>(extract: &BlockExtract<F>) -> u64 {
        let mut held = HeldBytes::default();
        if let Some(first) = &extract.first {
            held.frame(first);
        }
        if let Some(last) = &extract.last {
            held.frame(last);
        }
        held.total()
    }

    /// What a block's extract reports beyond its first and last frames when
    /// each of `countries` countries' forces change at each of `hours` hours.
    fn beyond_frames<F: Family + 'static>(countries: u32, hours: i32) -> u64
    where
        F::Scan: Send + 'static,
        F::Raw: Send + 'static,
        F::Scanned: Send + 'static,
        F::Tail: 'static,
    {
        let frames: Vec<DecodedFrame> = (100..100 + hours)
            .map(|hour| test_fixtures::frame(hour, countries))
            .collect();
        let extract = extracted::<F>(&frames);
        let (reported, frames) = (extract.bytes(), frames_alone(&extract));
        assert!(reported >= frames);
        reported - frames
    }

    /// A block whose forces hold after its first hour keeps few rows, but
    /// still its first and last frames and the scan (its census of every
    /// country's forces and its tracks): it reports at least those.
    fn holds_its_frames_and_scan<F: Family + 'static>()
    where
        F::Scan: Send + 'static,
        F::Raw: Send + 'static,
        F::Scanned: Send + 'static,
        F::Tail: 'static,
    {
        let frames: Vec<DecodedFrame> = (100..104)
            .map(|hour| {
                let mut frame = test_fixtures::frame(100, 64);
                frame.hour = hour;
                frame
            })
            .collect();
        let extract = extracted::<F>(&frames);
        let mut scan = HeldBytes::default();
        scan.value(&extract.scan);
        let (reported, frames, scan) = (extract.bytes(), frames_alone(&extract), scan.total());
        assert!(scan > 0);
        assert!(reported >= frames + scan, "{reported} {frames} {scan}");
    }

    /// Every later hour's rows are kept (each changed country's run by tag,
    /// in each channel it changed in), and the scan keeps every run, so a
    /// block of K countries changing at each of H hours holds more than its
    /// frames by at least two run entries per country and later hour.
    fn grows_with_countries_times_hours<F: Family + 'static>()
    where
        F::Scan: Send + 'static,
        F::Raw: Send + 'static,
        F::Scanned: Send + 'static,
        F::Tail: 'static,
    {
        let row = 2 * size_of::<(String, F::Raw)>() as u64;
        let (small, wide, long, both) = (
            beyond_frames::<F>(2, 4),
            beyond_frames::<F>(64, 4),
            beyond_frames::<F>(2, 32),
            beyond_frames::<F>(64, 32),
        );
        assert!(wide - small >= (64 - 2) * (4 - 1) * row, "{small} {wide}");
        assert!(long - small >= 2 * (32 - 4) * row, "{small} {long}");
        assert!(both - wide >= 64 * (32 - 4) * row, "{wide} {both}");
        assert!(both - long >= (64 - 2) * (32 - 1) * row, "{long} {both}");
    }

    #[test]
    fn air_extract_bytes_count_its_frames_and_scan() {
        holds_its_frames_and_scan::<Air>();
    }

    #[test]
    fn navy_extract_bytes_count_its_frames_and_scan() {
        holds_its_frames_and_scan::<Navy>();
    }

    #[test]
    fn air_extract_bytes_grow_with_countries_times_hours() {
        grows_with_countries_times_hours::<Air>();
    }

    #[test]
    fn navy_extract_bytes_grow_with_countries_times_hours() {
        grows_with_countries_times_hours::<Navy>();
    }
}
