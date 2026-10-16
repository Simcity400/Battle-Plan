//! Disjoint numeric observers, packed into scoped workers with a frame barrier.
use super::{FamilyTimings, Refused};
use crate::prepared::frame_changes::FrameChanges;
use replay::frame::DecodedFrame;
use std::{
    collections::BTreeMap,
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, mpsc},
    thread::{Scope, ScopedJoinHandle},
};

type Timings = Option<BTreeMap<&'static str, FamilyTimings>>;
type Observe<'a> =
    dyn FnMut(&str, &DecodedFrame, &FrameChanges, &mut Timings) -> io::Result<()> + Send + 'a;

pub(super) struct Job<'a> {
    calls: Vec<(&'static str, usize)>,
    cost: u32,
    observe: Box<Observe<'a>>,
    /// The family refused this branch's recorded data; it is not observed again.
    refused: bool,
    /// A refusal the coordinator has not recorded yet.
    refusal: Option<io::Error>,
}

impl<'a> Job<'a> {
    pub(super) fn new(
        calls: &[(&'static str, usize)],
        cost: u32,
        observe: impl FnMut(&str, &DecodedFrame, &FrameChanges, &mut Timings) -> io::Result<()>
        + Send
        + 'a,
    ) -> Self {
        Self {
            calls: calls.to_vec(),
            cost,
            observe: Box::new(observe),
            refused: false,
            refusal: None,
        }
    }

    fn call(
        &mut self,
        name: &str,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        timings: &mut Timings,
        #[cfg(test)] hook: Option<&Hook>,
        #[cfg(test)] refusals: &[(&'static str, i32)],
    ) -> Option<io::Result<()>> {
        if self.refused {
            return Some(Ok(()));
        }
        let result = catch_unwind(AssertUnwindSafe(|| {
            #[cfg(test)]
            if let Some(hook) = hook {
                match hook(name, frame, changes) {
                    Fault::Continue => {}
                    Fault::Disconnect => return None,
                    Fault::Error => {
                        return Some(Err(io::Error::other("Injected preparation lane error")));
                    }
                    Fault::Panic => panic!("Injected preparation lane panic"),
                }
            }
            Some((self.observe)(name, frame, changes, timings))
        }))
        .unwrap_or_else(|_| {
            Some(Err(io::Error::other(format!(
                "Preparation observer lane panicked: {name}"
            ))))
        });
        #[cfg(test)]
        let result = result.map(|result| injected(refusals, name, frame.hour, result));
        match result {
            Some(Err(error)) if super::refuses(&error) => {
                self.refused = true;
                self.refusal = Some(error);
                Some(Ok(()))
            }
            other => other,
        }
    }
}

// Serial observation retains just the fact, without a new frame descriptor.
// Parallel observation owns the fact in the existing shared dispatch allocation.
pub(super) enum DispatchedChanges {
    Local(Box<FrameChanges>),
    Shared(Arc<(DecodedFrame, FrameChanges)>),
}
impl DispatchedChanges {
    pub(super) fn changes(&self) -> &FrameChanges {
        match self {
            Self::Local(changes) => changes,
            Self::Shared(dispatch) => &dispatch.1,
        }
    }
}

struct Completion {
    result: Result<(), (usize, io::Error)>,
    refusals: Vec<(&'static str, io::Error)>,
    timings: Timings,
}

struct Worker<'scope> {
    send: Option<mpsc::SyncSender<Arc<(DecodedFrame, FrameChanges)>>>,
    receive: mpsc::Receiver<Completion>,
    join: Option<ScopedJoinHandle<'scope, ()>>,
    first_order: usize,
    pending: bool,
}

pub(super) struct Lanes<'scope> {
    serial: Vec<Job<'scope>>,
    workers: Vec<Worker<'scope>>,
    /// Serial refusals of the current frame, recorded by `finish`.
    refusals: Vec<(&'static str, io::Error)>,
    #[cfg(test)]
    hook: Option<Hook>,
    #[cfg(test)]
    injected: Vec<(&'static str, i32)>,
}

#[cfg(not(test))]
pub(super) fn parallelism() -> usize {
    #[cfg(windows)]
    if let Some(count) = process_affinity_parallelism() {
        return count;
    }
    std::thread::available_parallelism().map_or(1, usize::from)
}

/// Tests observe serially unless they opt into lanes with `with_test_lanes`:
/// concurrent tests would otherwise each start a lane per CPU. Lane parity
/// with serial observation is proven by the lane tests themselves.
#[cfg(test)]
pub(super) fn parallelism() -> usize {
    TEST_PARALLELISM.with(|value| value.get()).unwrap_or(1)
}

#[cfg(any(windows, test))]
fn affinity_mask_parallelism(process_mask: usize) -> Option<usize> {
    let count = process_mask.count_ones() as usize;
    (count != 0).then_some(count)
}

#[cfg(all(windows, not(test)))]
fn process_affinity_parallelism() -> Option<usize> {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessAffinityMask};
    let mut process_mask = 0usize;
    let mut system_mask = 0usize;
    // Both masks describe a processor group. A process spanning groups may
    // return zero masks; leave that case (and API failure) to std's fallback
    // rather than treating a group mask as a machine-wide processor inventory.
    let read =
        unsafe { GetProcessAffinityMask(GetCurrentProcess(), &mut process_mask, &mut system_mask) };
    if read == 0 {
        return None;
    }
    affinity_mask_parallelism(process_mask)
}

#[cfg(test)]
#[test]
fn process_affinity_masks_determine_lane_parallelism_without_changing_affinity() {
    // Either eight-core half of the sixteen-core measurement machine yields
    // P=8 and therefore six observation lanes after decoder/coordinator slots.
    for mask in [255usize, 65280] {
        assert_eq!(affinity_mask_parallelism(mask), Some(8));
        assert_eq!(
            affinity_mask_parallelism(mask).unwrap().saturating_sub(2),
            6
        );
    }
    assert_eq!(affinity_mask_parallelism(1), Some(1));
    assert_eq!(affinity_mask_parallelism(0b1010), Some(2));
    assert_eq!(
        affinity_mask_parallelism(1usize << (usize::BITS - 1)),
        Some(1)
    );
    assert_eq!(
        affinity_mask_parallelism(usize::MAX),
        Some(usize::BITS as usize)
    );
    // Zero is not an empty/one-core process. Windows can use it when a process
    // spans processor groups, so the caller must use available_parallelism.
    assert_eq!(affinity_mask_parallelism(0), None);
}

impl<'scope> Lanes<'scope> {
    pub(super) fn start<'env>(
        scope: &'scope Scope<'scope, 'env>,
        mut jobs: Vec<Job<'scope>>,
        parallelism: usize,
        measured: bool,
        refused: &Refused,
    ) -> Self {
        for job in &mut jobs {
            job.refused = !refused.live(job.calls[0].0);
        }
        let count = parallelism.saturating_sub(2).min(jobs.len());
        #[cfg(test)]
        let hook = TEST_HOOK.with(|hook| hook.borrow().clone());
        #[cfg(test)]
        let injected = TEST_REFUSALS.with(|refusals| refusals.borrow().clone());
        if count == 0 {
            return Self {
                serial: jobs,
                workers: Vec::new(),
                refusals: Vec::new(),
                #[cfg(test)]
                hook,
                #[cfg(test)]
                injected,
            };
        }
        // Millisecond weights are the observed family costs from the release
        // campaign profile. Packing affects scheduling, never observation order
        // within a family or ownership of an accumulator.
        jobs.sort_by_key(|job| (std::cmp::Reverse(job.cost), job.calls[0].1));
        let mut packed: Vec<Vec<Job<'scope>>> = (0..count).map(|_| Vec::new()).collect();
        let mut costs = vec![0u32; count];
        for job in jobs {
            let lane = (0..count).min_by_key(|&lane| (costs[lane], lane)).unwrap();
            costs[lane] += job.cost;
            packed[lane].push(job);
        }
        let workers = packed
            .into_iter()
            .map(|mut jobs| {
                let mut calls: Vec<_> = jobs
                    .iter()
                    .enumerate()
                    .flat_map(|(job, value)| {
                        value
                            .calls
                            .iter()
                            .map(move |&(name, order)| (order, job, name))
                    })
                    .collect();
                calls.sort_by_key(|&(order, _, _)| order);
                let first_order = calls[0].0;
                let (send, commands) = mpsc::sync_channel::<Arc<(DecodedFrame, FrameChanges)>>(0);
                let (results, receive) = mpsc::sync_channel(1);
                #[cfg(test)]
                let hook = hook.clone();
                #[cfg(test)]
                let injected = injected.clone();
                let join = scope.spawn(move || {
                    #[cfg(windows)]
                    unsafe {
                        use windows_sys::Win32::System::Threading::{
                            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
                        };
                        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
                    }
                    while let Ok(frame) = commands.recv() {
                        let mut timings = measured.then(BTreeMap::new);
                        let mut result = Ok(());
                        let mut refusals = Vec::new();
                        for &(order, job, name) in &calls {
                            let Some(observed) = jobs[job].call(
                                name,
                                &frame.0,
                                &frame.1,
                                &mut timings,
                                #[cfg(test)]
                                hook.as_ref(),
                                #[cfg(test)]
                                &injected,
                            ) else {
                                return;
                            };
                            if let Err(error) = observed {
                                result = Err((order, error));
                                break;
                            }
                            if let Some(error) = jobs[job].refusal.take() {
                                refusals.push((name, error));
                            }
                        }
                        drop(frame);
                        let failed = result.is_err();
                        let completion = Completion {
                            result,
                            refusals,
                            timings,
                        };
                        if results.send(completion).is_err() || failed {
                            break;
                        }
                    }
                });
                Worker {
                    send: Some(send),
                    receive,
                    join: Some(join),
                    first_order,
                    pending: false,
                }
            })
            .collect();
        Self {
            serial: Vec::new(),
            workers,
            refusals: Vec::new(),
            #[cfg(test)]
            hook,
            #[cfg(test)]
            injected,
        }
    }

    pub(super) fn count(&self) -> usize {
        self.workers.len()
    }

    pub(super) fn begin(
        &mut self,
        frame: &DecodedFrame,
        changes: FrameChanges,
    ) -> io::Result<DispatchedChanges> {
        if self.workers.is_empty() {
            return Ok(DispatchedChanges::Local(Box::new(changes)));
        }
        if self.workers.iter().any(|worker| worker.pending) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Preparation observer frame must be joined before dispatching another",
            ));
        }
        // Keep the existing single shared frame descriptor, now accompanied by
        // the one fact calculated before dispatch. No table rows are copied.
        let dispatch = Arc::new((frame.clone(), changes));
        for worker in &mut self.workers {
            worker
                .send
                .as_ref()
                .expect("live frame sender")
                .send(Arc::clone(&dispatch))
                .map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "Preparation observer lane disconnected during dispatch",
                    )
                })?;
            worker.pending = true;
        }
        Ok(DispatchedChanges::Shared(dispatch))
    }

    #[cfg(test)]
    pub(super) fn check_coordinator(
        &self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        if let Some(hook) = &self.hook {
            match hook("coordinator", frame, changes) {
                Fault::Continue => {}
                Fault::Error => {
                    return Err(io::Error::other("Injected preparation coordinator error"));
                }
                Fault::Disconnect => {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "Injected coordinator disconnect",
                    ));
                }
                Fault::Panic => panic!("Injected preparation coordinator panic"),
            }
        }
        Ok(())
    }

    pub(super) fn serial(
        &mut self,
        name: &str,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        timings: &mut Timings,
    ) -> io::Result<()> {
        if !self.workers.is_empty() {
            return Ok(());
        }
        let (job, family) = self
            .serial
            .iter_mut()
            .find_map(|job| {
                let family = job.calls.iter().find(|&&(family, _)| family == name)?.0;
                Some((job, family))
            })
            .expect("registered serial observer call");
        let result = job
            .call(
                name,
                frame,
                changes,
                timings,
                #[cfg(test)]
                self.hook.as_ref(),
                #[cfg(test)]
                &self.injected,
            )
            .unwrap_or_else(|| {
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "Preparation observer lane disconnected",
                ))
            });
        if let Some(error) = job.refusal.take() {
            self.refusals.push((family, error));
        }
        result
    }

    /// Join every dispatched lane even if the coordinator failed. No next frame
    /// can be dispatched while any result from this frame is outstanding. The
    /// frame's lane refusals are recorded at `hour` once the frame is accepted.
    pub(super) fn finish(
        &mut self,
        coordinator: io::Result<()>,
        coordinator_order: usize,
        timings: &mut Timings,
        refused: &mut Refused,
        hour: i32,
    ) -> io::Result<()> {
        let mut failure = coordinator.err().map(|error| (coordinator_order, error));
        let mut refusals = std::mem::take(&mut self.refusals);
        for worker in &mut self.workers {
            if !worker.pending {
                continue;
            }
            worker.pending = false;
            let completion = worker.receive.recv().unwrap_or_else(|_| Completion {
                result: Err((
                    worker.first_order,
                    io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "Preparation observer lane disconnected",
                    ),
                )),
                refusals: Vec::new(),
                timings: None,
            });
            merge(timings, completion.timings);
            refusals.extend(completion.refusals);
            if let Err((order, error)) = completion.result
                && failure.as_ref().is_none_or(|(prior, _)| order < *prior)
            {
                failure = Some((order, error));
            }
        }
        if let Some((_, error)) = failure {
            return Err(error);
        }
        for (family, error) in refusals {
            refused.refuse(family, hour, error.to_string(), None);
        }
        Ok(())
    }
}

impl Drop for Lanes<'_> {
    fn drop(&mut self) {
        // Close every command channel before joining. A completion channel has
        // one slot, so an observer/coordinator unwind cannot strand a sender.
        for worker in &mut self.workers {
            worker.send.take();
        }
        for worker in &mut self.workers {
            if let Some(join) = worker.join.take() {
                let _ = join.join();
            }
        }
    }
}

pub(super) fn merge(into: &mut Timings, from: Timings) {
    if let (Some(into), Some(from)) = (into.as_mut(), from) {
        for (name, value) in from {
            let entry = into.entry(name).or_default();
            entry.observe += value.observe;
            entry.flush += value.flush;
            entry.calls += value.calls;
        }
    }
}

#[cfg(test)]
pub(super) enum Fault {
    Continue,
    Disconnect,
    Error,
    Panic,
}
#[cfg(test)]
pub(super) type Hook = Arc<dyn Fn(&str, &DecodedFrame, &FrameChanges) -> Fault + Send + Sync>;
#[cfg(test)]
thread_local! {
    static TEST_PARALLELISM: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    static TEST_HOOK: std::cell::RefCell<Option<Hook>> = const { std::cell::RefCell::new(None) };
    static TEST_REFUSALS: std::cell::RefCell<Vec<(&'static str, i32)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Tests inject a family's InvalidData refusal of the frame at an hour, on the
/// coordinator or a lane, through the same rule as a recorded-data refusal.
#[cfg(test)]
pub(super) fn with_test_refusals<T>(
    refusals: &[(&'static str, i32)],
    run: impl FnOnce() -> T,
) -> T {
    struct Reset(Vec<(&'static str, i32)>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_REFUSALS.with(|refusals| *refusals.borrow_mut() = std::mem::take(&mut self.0));
        }
    }
    let _reset = Reset(TEST_REFUSALS.with(|value| value.replace(refusals.to_vec())));
    run()
}

#[cfg(test)]
fn injected<T>(
    refusals: &[(&'static str, i32)],
    family: &str,
    hour: i32,
    result: io::Result<T>,
) -> io::Result<T> {
    if result.is_ok()
        && refusals
            .iter()
            .any(|&(injected, at)| injected == family && at == hour)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Injected refusal of {family} at hour {hour}"),
        ));
    }
    result
}

/// Coordinator families observe and flush on the test's own thread.
#[cfg(test)]
pub(super) fn inject<T>(family: &str, hour: i32, result: io::Result<T>) -> io::Result<T> {
    TEST_REFUSALS.with(|refusals| injected(&refusals.borrow(), family, hour, result))
}

#[cfg(test)]
pub(super) fn with_test_lanes<T>(
    parallelism: usize,
    hook: Option<Hook>,
    run: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<usize>, Option<Hook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_PARALLELISM.with(|value| value.set(self.0));
            TEST_HOOK.with(|hook| *hook.borrow_mut() = self.1.take());
        }
    }
    let _reset = Reset(
        TEST_PARALLELISM.with(|value| value.replace(Some(parallelism))),
        TEST_HOOK.with(|value| value.replace(hook)),
    );
    run()
}
