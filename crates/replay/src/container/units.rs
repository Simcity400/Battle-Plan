//! Blocks written in parallel: keyframes in order (they read earlier anchors), the rest on workers.
//! A split or refusal stops it and the sequential writer repeats the run, so bytes and errors match.

use super::*;
use std::{
    collections::VecDeque,
    panic,
    sync::{
        Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, AtomicUsize},
    },
    time::Duration,
};

struct Planned<'r> {
    position: usize,
    /// Its session's capture mode.
    mode: CaptureMode,
    frames: Vec<RecordRef<'r>>,
}

pub(super) enum Written {
    Pending(Pending),
    /// After reporting this many hours and calling progress this many times.
    Stopped {
        hours: usize,
        progress: usize,
    },
}

/// The sequential writer's checks of the records themselves.
fn plan<'r, R: SpoolRecord>(
    segments: &[&'r [R]],
    identity: &Identity,
    existing: Option<&Replay>,
    interval: u32,
) -> io::Result<(Pending, Vec<Planned<'r>>)> {
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
    let mut planned: Vec<Planned<'r>> = Vec::new();
    for (position, segment) in segments.iter().enumerate() {
        if segment.is_empty() {
            continue;
        }
        if !matches!(
            segment[0].view(),
            RecordRef::SessionStart(_) | RecordRef::BranchStart(_)
        ) {
            return Err(invalid_static("segment does not begin"));
        }
        let mut current_session = None;
        let mut current_branch = None;
        let mut needs_keyframe = true;
        let mut block: Option<Hour> = None;
        let mut segment_state = SegmentState::default();
        for record in segment.iter() {
            let record = record.view();
            segment_state.accept_view(&record)?;
            match record {
                RecordRef::SessionStart(session) => {
                    if !session.identity.same_content(identity) {
                        return Err(invalid_static("session content differs"));
                    }
                    if sessions
                        .values()
                        .any(|old| old.capture_mode != session.capture_mode)
                    {
                        return Err(invalid_static("session capture mode differs"));
                    }
                    if let Some(previous) = sessions.get(&session.id) {
                        if previous != session {
                            return Err(invalid_static("conflicting session context"));
                        }
                    } else {
                        session.validate()?;
                        sessions.insert(session.id, session.clone());
                        pending.sessions.push(session.clone());
                    }
                    block = None;
                    current_session = Some(session.id);
                    current_branch = None;
                    needs_keyframe = true;
                }
                RecordRef::BranchStart(branch) => {
                    branch.validate()?;
                    if !sessions.contains_key(&branch.session)
                        || current_session.is_some_and(|session| session != branch.session)
                    {
                        return Err(invalid_static("branch without its session"));
                    }
                    if let Some(previous) = branches.get(&branch.id) {
                        if previous != branch {
                            return Err(invalid_static("conflicting branch context"));
                        }
                    } else {
                        branches.insert(branch.id, branch.clone());
                        pending.branches.push(branch.clone());
                    }
                    block = None;
                    current_session = Some(branch.session);
                    current_branch = Some(branch.id);
                    needs_keyframe = true;
                }
                RecordRef::Frame(_) | RecordRef::Metadata(_) => {
                    let (branch, hour, keyframe) = record.frame().unwrap();
                    if current_branch != Some(branch)
                        || (needs_keyframe && !keyframe)
                        || last_hours.get(&branch).is_some_and(|last| hour <= *last)
                        || hour < branches[&branch].first_hour
                    {
                        return Err(invalid_static("frame out of its context"));
                    }
                    if keyframe {
                        block = Some(hour);
                        planned.push(Planned {
                            position,
                            mode: sessions[&branches[&branch].session].capture_mode,
                            frames: Vec::new(),
                        });
                    }
                    let first = block.ok_or_else(|| invalid_static("no block keyframe"))?;
                    if i64::from(hour) - i64::from(first) >= i64::from(interval) {
                        return Err(invalid_static("block exceeds its interval"));
                    }
                    planned.last_mut().unwrap().frames.push(record);
                    last_hours.insert(branch, hour);
                    needs_keyframe = false;
                }
                RecordRef::Gap(gap) => {
                    gap.validate()?;
                    if current_branch != Some(gap.branch) {
                        return Err(invalid_static("gap without its branch"));
                    }
                    pending.gaps.push(gap.clone());
                }
                RecordRef::Control(control) => {
                    if current_session != Some(control.session()) {
                        return Err(invalid_static("control without its session"));
                    }
                    if !(matches!(control, ControlRecord::EventLoss { .. })
                        && pending.controls.contains(control))
                    {
                        pending.controls.push(control.clone());
                    }
                }
            }
        }
    }
    Ok((pending, planned))
}

fn restore<'r>(
    read: Option<&SharedReader<'_>>,
    position: usize,
    record: RecordRef<'r>,
) -> io::Result<Cow<'r, Frame>> {
    match record {
        RecordRef::Frame(frame) => Ok(Cow::Borrowed(frame)),
        RecordRef::Metadata(metadata) => {
            let frame = match read {
                Some(read) => read(position, metadata)?,
                None => metadata.frame()?,
            };
            if (frame.branch, frame.hour, frame.keyframe)
                != (metadata.branch, metadata.hour, metadata.keyframe)
            {
                return Err(invalid_static("restored frame differs"));
            }
            Ok(Cow::Owned(frame))
        }
        _ => unreachable!("a frame record"),
    }
}

struct Started {
    writing: Writing,
    observed: Vec<Observed>,
    written: usize,
}

/// Blocks held at once, from keyframe until packed: as many as the sequential writer holds unpacked.
pub(super) const HELD: usize = pack::WAITING + 1;

/// What a test sees of the writer's scheduling, on the thread that reaches it, which it may hold.
#[cfg(test)]
pub(super) enum Step {
    /// A block admitted: this many are now begun and not yet packed.
    Admitted(usize),
    /// The keyframe thread waits, `drained` when every begun block has a worker.
    Waiting { drained: bool },
    /// The keyframe thread admits no more blocks.
    Finished,
    /// A worker is about to count its compressed block packed.
    Packing,
    /// A block counted packed: this many are left.
    Packed(usize),
}

#[cfg(test)]
pub(super) type Watch = std::sync::Arc<dyn Fn(Step) + Send + Sync>;

#[cfg(test)]
thread_local! {
    /// Tests watch the writes this thread starts.
    pub(super) static WATCH: std::cell::RefCell<Option<Watch>> =
        const { std::cell::RefCell::new(None) };
}

enum Outcome {
    Packed(CompressedBlock, Vec<Observed>),
    Stop,
}

enum Begun {
    Writing(Started),
    Compressing(grouped::Compressing, Vec<Observed>),
}

/// A block written ahead: packed with its hours reported, or written and compressing.
enum Finished {
    Packed(CompressedBlock),
    Compressing(grouped::Compressing, Vec<Observed>),
}

/// Blocks written while a spool still grows, for its finalization to take over.
pub struct Ahead {
    existing: Option<Replay>,
    /// Finalization takes over only records that start with these.
    records: Vec<(u32, Vec<Recovered>)>,
    anchors: grouped::Anchors,
    blocks: Vec<Finished>,
    open: Option<Started>,
    /// Hours of packed blocks, reported already.
    hours: usize,
}

impl Ahead {
    pub fn new(existing: Option<Replay>) -> io::Result<Self> {
        let anchors = match &existing {
            Some(replay) => grouped::Anchors::seed(replay)?,
            None => grouped::Anchors::default(),
        };
        Ok(Self {
            existing,
            records: Vec::new(),
            anchors,
            blocks: Vec::new(),
            open: None,
            hours: 0,
        })
    }

    pub fn existing(&self) -> Option<&Replay> {
        self.existing.as_ref()
    }

    /// Hours whose blocks are packed.
    pub fn hours(&self) -> usize {
        self.hours
    }

    /// Hours whose frames are written, packed or not.
    pub fn written(&self) -> usize {
        let compressing = self.blocks.iter().map(|block| match block {
            Finished::Packed(_) => 0,
            Finished::Compressing(_, observed) => observed.len(),
        });
        self.hours + compressing.sum::<usize>() + self.open.as_ref().map_or(0, |open| open.written)
    }

    /// Whether finalizing `segments` writes first what this wrote.
    pub fn continues<R: PartialEq<Recovered>>(&self, segments: &[(u32, &[R])]) -> bool {
        self.records.len() <= segments.len()
            && self.records.iter().zip(segments).enumerate().all(
                |(at, ((number, kept), (next, records)))| {
                    number == next
                        && records.len() >= kept.len()
                        && records
                            .iter()
                            .zip(kept)
                            .all(|(record, kept)| record == kept)
                        && (at + 1 == self.records.len() || records.len() == kept.len())
                },
            )
    }

    /// Writes the frames of `segments` not yet written and compresses finished blocks,
    /// reporting their hours in order, until `stop`. False where it cannot go on.
    pub fn advance(
        &mut self,
        campaign_id: &[u8],
        segments: &[(u32, &[Recovered])],
        read: impl Fn(u32, &FrameMetadata) -> io::Result<Frame> + Sync,
        mut observe: impl FnMut(&Observed) -> io::Result<()>,
        stop: &(dyn Fn() -> bool + Sync),
    ) -> io::Result<bool> {
        if !self.continues(segments) {
            return Ok(false);
        }
        for (at, (number, records)) in segments.iter().enumerate() {
            match self.records.get_mut(at) {
                Some((_, kept)) => kept.extend_from_slice(&records[kept.len()..]),
                None => self.records.push((*number, records.to_vec())),
            }
        }
        let identity = match &self.existing {
            Some(replay) => Some(replay.identity.clone()),
            None => segments
                .iter()
                .flat_map(|(_, records)| records.iter())
                .find_map(|record| match record {
                    Recovered::SessionStart(session) => Some(session.identity.clone()),
                    _ => None,
                }),
        };
        let records: Vec<&[Recovered]> = segments.iter().map(|(_, records)| *records).collect();
        let Some(Ok((_, planned))) = identity.map(|identity| {
            plan(
                &records,
                &identity,
                self.existing.as_ref(),
                KEYFRAME_INTERVAL,
            )
        }) else {
            return Ok(false);
        };
        let cores = pack::Cores::new(1);
        let read = |position: usize, metadata: &FrameMetadata| read(segments[position].0, metadata);
        let observation =
            |observed: &mut Vec<Observed>, frame: &Frame, decoded: &DecodedFrame, sizes| {
                engine_allocation::reserve(observed, 1)?;
                let hour = Observed::new(frame, decoded, sizes, observed.last())?;
                observed.push(hour);
                Ok(())
            };
        loop {
            // Blocks compress in order, and at most one waits, so a fast game waits in the spool.
            if let Some(at) = self
                .blocks
                .iter()
                .position(|block| matches!(block, Finished::Compressing(..)))
            {
                let Finished::Compressing(compressing, observed) = &mut self.blocks[at] else {
                    unreachable!("a compressing block");
                };
                let Some(packed) = compressing.run(&cores, stop)? else {
                    return Ok(true);
                };
                for hour in observed.iter() {
                    observe(hour)?;
                }
                self.hours += observed.len();
                self.blocks[at] = Finished::Packed(packed);
                continue;
            }
            let at = self.blocks.len();
            let Some(block) = planned.get(at) else {
                return Ok(true);
            };
            if stop() {
                return Ok(true);
            }
            let open = match &mut self.open {
                Some(open) => open,
                None => {
                    let frame = restore(Some(&read), block.position, block.frames[0])?;
                    let mut writing = Writing::new(&frame, block.mode)?;
                    let mut observed = Vec::new();
                    if !writing.frame(
                        &frame,
                        campaign_id,
                        MAX_BLOCK_RAW_BYTES,
                        &mut self.anchors,
                        &cores,
                        &mut |frame, decoded, sizes| {
                            observation(&mut observed, frame, decoded, sizes)
                        },
                        None,
                    )? {
                        return Ok(false);
                    }
                    self.open.insert(Started {
                        writing,
                        observed,
                        written: 1,
                    })
                }
            };
            for record in &block.frames[open.written..] {
                if stop() {
                    return Ok(true);
                }
                let frame = restore(Some(&read), block.position, *record)?;
                let observed = &mut open.observed;
                if !open.writing.frame(
                    &frame,
                    campaign_id,
                    MAX_BLOCK_RAW_BYTES,
                    &mut self.anchors,
                    &cores,
                    &mut |frame, decoded, sizes| observation(observed, frame, decoded, sizes),
                    None,
                )? {
                    return Ok(false);
                }
                open.written += 1;
            }
            if at + 1 == planned.len() {
                return Ok(true);
            }
            let Started {
                writing, observed, ..
            } = self.open.take().unwrap();
            self.blocks.push(Finished::Compressing(
                grouped::Compressing::new(writing.finish()),
                observed,
            ));
        }
    }
}

struct Lanes {
    started: VecDeque<(usize, Begun)>,
    done: BTreeMap<usize, Outcome>,
    keyframes_done: bool,
    taken: usize,
    /// Blocks begun and not yet packed.
    held: usize,
}

struct Shared {
    lanes: Mutex<Lanes>,
    changed: Condvar,
    stop: AtomicBool,
    frames: AtomicUsize,
    cores: pack::Cores,
    #[cfg(test)]
    watch: Option<Watch>,
}

impl Shared {
    #[cfg(test)]
    fn watch(&self, step: Step) {
        if let Some(watch) = &self.watch {
            watch(step);
        }
    }

    fn lock(&self) -> MutexGuard<'_, Lanes> {
        self.lanes.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn stopped(&self) -> bool {
        self.stop.load(std::sync::atomic::Ordering::Acquire)
    }

    /// A block packed, so another may begin.
    fn packed(&self) {
        let mut lanes = self.lock();
        lanes.held -= 1;
        #[cfg(test)]
        self.watch(Step::Packed(lanes.held));
        drop(lanes);
        self.changed.notify_all();
    }

    /// Takes the guard: stopping under the lock wakes a thread between its check and its wait.
    fn stop(&self, _held: &MutexGuard<'_, Lanes>) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
    }

    fn finish(&self, block: usize, outcome: Outcome) {
        let mut lanes = self.lock();
        if matches!(outcome, Outcome::Stop) {
            self.stop(&lanes);
        }
        lanes.done.insert(block, outcome);
        drop(lanes);
        self.changed.notify_all();
    }
}

/// Stops the workers however the reporting thread leaves.
struct Stopping<'a>(&'a Shared);

impl Drop for Stopping<'_> {
    fn drop(&mut self) {
        let lanes = self.0.lock();
        self.0.stop(&lanes);
        drop(lanes);
        self.0.changed.notify_all();
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn write<R: SpoolRecord>(
    segments: &[&[R]],
    identity: &Identity,
    campaign_id: &[u8],
    existing: Option<&Replay>,
    interval: u32,
    mut observe: Option<&mut HourObserver<'_>>,
    progress: &mut dyn FnMut() -> io::Result<()>,
    read: Option<&SharedReader<'_>>,
    block_limit: usize,
    threads: usize,
    ahead: Option<Ahead>,
) -> io::Result<Written> {
    let reported_ahead = ahead.as_ref().map_or(0, Ahead::hours);
    let Ok((mut pending, planned)) = plan(segments, identity, existing, interval) else {
        return Ok(Written::Stopped {
            hours: reported_ahead,
            progress: 0,
        });
    };
    let mut started = VecDeque::new();
    let mut packed = BTreeMap::new();
    let mut anchors = match existing {
        Some(replay) => grouped::Anchors::seed(replay)?,
        None => grouped::Anchors::default(),
    };
    let mut first = 0;
    if let Some(ahead) = ahead {
        anchors = ahead.anchors;
        for block in ahead.blocks {
            match block {
                Finished::Packed(block) => {
                    packed.insert(first, Outcome::Packed(block, Vec::new()));
                }
                Finished::Compressing(compressing, observed) => {
                    started.push_back((first, Begun::Compressing(compressing, observed)));
                }
            }
            first += 1;
        }
        if let Some(open) = ahead.open {
            started.push_back((first, Begun::Writing(open)));
            first += 1;
        }
        if first > planned.len() {
            return Err(invalid_static("more blocks written ahead than planned"));
        }
    }
    let held = started.len();
    let shared = Shared {
        lanes: Mutex::new(Lanes {
            started,
            done: packed,
            keyframes_done: false,
            taken: 0,
            held,
        }),
        changed: Condvar::new(),
        stop: AtomicBool::new(false),
        frames: AtomicUsize::new(0),
        cores: pack::Cores::idle(threads),
        #[cfg(test)]
        watch: WATCH.with_borrow(Clone::clone),
    };
    let shared = &shared;
    let planned = &planned;
    let reporting = observe.is_some();
    let observation = move |observed: &mut Vec<Observed>,
                            frame: &Frame,
                            decoded: &DecodedFrame,
                            sizes: [usize; TABLE_COUNT]|
          -> io::Result<()> {
        if reporting {
            engine_allocation::reserve(observed, 1)?;
            let hour = Observed::new(frame, decoded, sizes, observed.last())?;
            observed.push(hour);
        }
        Ok(())
    };
    // Bounds the blocks held: one waiting, `HELD` unpacked, twice the threads ahead of those reported.
    let keyframes = move || {
        for (at, block) in planned.iter().enumerate().skip(first) {
            {
                let mut lanes = shared.lock();
                while (!lanes.started.is_empty()
                    || lanes.held >= HELD
                    || at >= lanes.taken + 2 * threads)
                    && !shared.stopped()
                {
                    #[cfg(test)]
                    shared.watch(Step::Waiting {
                        drained: lanes.started.is_empty(),
                    });
                    lanes = shared
                        .changed
                        .wait(lanes)
                        .unwrap_or_else(PoisonError::into_inner);
                }
                lanes.held += 1;
                #[cfg(test)]
                shared.watch(Step::Admitted(lanes.held));
            }
            if shared.stopped() {
                break;
            }
            let started = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                shared.cores.busy(|| -> io::Result<Started> {
                    let frame = restore(read, block.position, block.frames[0])?;
                    let mut writing = Writing::new(&frame, block.mode)?;
                    let mut observed = Vec::new();
                    let written = writing.frame(
                        &frame,
                        campaign_id,
                        block_limit,
                        &mut anchors,
                        &shared.cores,
                        &mut |frame, decoded, sizes| {
                            observation(&mut observed, frame, decoded, sizes)
                        },
                        None,
                    )?;
                    if !written {
                        return Err(invalid_static("a keyframe is never split"));
                    }
                    Ok(Started {
                        writing,
                        observed,
                        written: 1,
                    })
                })
            }));
            shared
                .frames
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            match started {
                Ok(Ok(started)) => {
                    shared
                        .lock()
                        .started
                        .push_back((at, Begun::Writing(started)));
                    shared.changed.notify_all();
                }
                _ => {
                    shared.finish(at, Outcome::Stop);
                    break;
                }
            }
        }
        shared.lock().keyframes_done = true;
        #[cfg(test)]
        shared.watch(Step::Finished);
        shared.changed.notify_all();
    };
    let work = move || {
        loop {
            let (at, begun) = {
                let mut lanes = shared.lock();
                loop {
                    if shared.stopped() {
                        return;
                    }
                    if let Some(next) = lanes.started.pop_front() {
                        break next;
                    }
                    if lanes.keyframes_done {
                        return;
                    }
                    lanes = shared
                        .changed
                        .wait(lanes)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            };
            shared.changed.notify_all();
            let block = &planned[at];
            let outcome = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                shared.cores.busy(|| -> io::Result<Outcome> {
                    let mut started = match begun {
                        Begun::Writing(started) => started,
                        Begun::Compressing(mut compressing, observed) => {
                            let Some(packed) =
                                compressing.run(&shared.cores, &|| shared.stopped())?
                            else {
                                return Ok(Outcome::Stop);
                            };
                            shared.packed();
                            return Ok(Outcome::Packed(packed, observed));
                        }
                    };
                    let mut anchors = grouped::Anchors::default();
                    for record in &block.frames[started.written..] {
                        if shared.stopped() {
                            return Ok(Outcome::Stop);
                        }
                        let frame = restore(read, block.position, *record)?;
                        let observed = &mut started.observed;
                        if !started.writing.frame(
                            &frame,
                            campaign_id,
                            block_limit,
                            &mut anchors,
                            &shared.cores,
                            &mut |frame, decoded, sizes| {
                                observation(observed, frame, decoded, sizes)
                            },
                            None,
                        )? {
                            return Ok(Outcome::Stop);
                        }
                        shared
                            .frames
                            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                        shared.changed.notify_all();
                    }
                    let Started {
                        writing, observed, ..
                    } = started;
                    let packed = writing.finish().compress(&shared.cores)?;
                    #[cfg(test)]
                    shared.watch(Step::Packing);
                    shared.packed();
                    Ok(Outcome::Packed(packed, observed))
                })
            }));
            shared.finish(
                at,
                match outcome {
                    Ok(Ok(outcome)) => outcome,
                    _ => Outcome::Stop,
                },
            );
        }
    };
    let keyframes = Mutex::new(Some(keyframes));
    let keyframes = || {
        let keyframes = keyframes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(mut keyframes) = keyframes {
            keyframes()
        }
    };
    std::thread::scope(|scope| -> io::Result<Written> {
        let stopping = Stopping(shared);
        let lane = spawn(scope, "replay keyframes", &keyframes);
        let workers = (0..threads.saturating_sub(1).max(1))
            .filter(|_| spawn(scope, "replay block writer", &work))
            .count();
        if !lane || workers == 0 {
            drop(stopping);
            return Ok(Written::Stopped {
                hours: reported_ahead,
                progress: 0,
            });
        }
        let (mut next, mut observed, mut reported) = (0, reported_ahead, 0);
        loop {
            let outcome = {
                let mut lanes = shared.lock();
                loop {
                    if let Some(outcome) = lanes.done.remove(&next) {
                        break Some(outcome);
                    }
                    if next == planned.len() {
                        break None;
                    }
                    let written = shared.frames.load(std::sync::atomic::Ordering::Acquire);
                    if written > reported {
                        drop(lanes);
                        while reported < written {
                            progress()?;
                            reported += 1;
                        }
                        lanes = shared.lock();
                        continue;
                    }
                    lanes = shared
                        .changed
                        .wait_timeout(lanes, Duration::from_millis(250))
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                }
            };
            match outcome {
                Some(Outcome::Packed(block, hours)) => {
                    shared.lock().taken = next + 1;
                    shared.changed.notify_all();
                    if let Some(observe) = observe.as_deref_mut() {
                        for hour in &hours {
                            observe(hour)?;
                            observed += 1;
                        }
                    }
                    engine_allocation::reserve(&mut pending.blocks, 1)?;
                    pending.blocks.push(block);
                    next += 1;
                }
                Some(Outcome::Stop) => {
                    return Ok(Written::Stopped {
                        hours: observed,
                        progress: reported,
                    });
                }
                None => break,
            }
        }
        while reported < shared.frames.load(std::sync::atomic::Ordering::Acquire) {
            progress()?;
            reported += 1;
        }
        Ok(Written::Pending(pending))
    })
}

fn spawn<'scope, 'env>(
    scope: &'scope std::thread::Scope<'scope, 'env>,
    name: &str,
    run: &'env (dyn Fn() + Sync),
) -> bool {
    let priority = job_priority::JobPriority::current();
    #[cfg(test)]
    let settings = grouped::written_settings();
    std::thread::Builder::new()
        .name(name.into())
        .spawn_scoped(scope, move || {
            #[cfg(test)]
            grouped::write_with(settings);
            priority.enter(job_priority::Level::Normal);
            run()
        })
        .is_ok()
}
