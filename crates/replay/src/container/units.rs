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

struct Lanes {
    started: VecDeque<(usize, Started)>,
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
) -> io::Result<Written> {
    let Ok((mut pending, planned)) = plan(segments, identity, existing, interval) else {
        return Ok(Written::Stopped {
            hours: 0,
            progress: 0,
        });
    };
    let mut anchors = match existing {
        Some(replay) => grouped::Anchors::seed(replay)?,
        None => grouped::Anchors::default(),
    };
    let shared = Shared {
        lanes: Mutex::new(Lanes {
            started: VecDeque::new(),
            done: BTreeMap::new(),
            keyframes_done: false,
            taken: 0,
            held: 0,
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
            observed.push(Observed::new(frame, decoded, sizes)?);
        }
        Ok(())
    };
    // Bounds the blocks held: one waiting, `HELD` unpacked, twice the threads ahead of those reported.
    let keyframes = move || {
        for (at, block) in planned.iter().enumerate() {
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
                    let mut writing = Writing::new(&frame);
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
                    Ok(Started { writing, observed })
                })
            }));
            shared
                .frames
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            match started {
                Ok(Ok(started)) => {
                    shared.lock().started.push_back((at, started));
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
            let (at, mut started) = {
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
                    let mut anchors = grouped::Anchors::default();
                    for record in &block.frames[1..] {
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
                    let Started { writing, observed } = started;
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
                hours: 0,
                progress: 0,
            });
        }
        let (mut next, mut observed, mut reported) = (0, 0, 0);
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
