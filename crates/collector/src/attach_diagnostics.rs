use crate::{
    acquisition::{Completion, Timeline},
    acquisition_jobs::{Job, Report},
    extended::division_presentation,
    sampling::{Captured as Frame, HourCost},
    status::{DebugValue, map, sequence, status},
};
use engine_native::memory::ReadCounts;
use replay_data::rows::{TABLE_COUNT, TableId};
use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU8, AtomicU64, AtomicUsize, Ordering},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ThreadWork {
    pub cycles: u64,
    pub scheduled_ns: u64,
}
impl ThreadWork {
    pub(crate) fn delta(self, end: Self) -> Option<Self> {
        Some(Self {
            cycles: end.cycles.checked_sub(self.cycles)?,
            scheduled_ns: end.scheduled_ns.checked_sub(self.scheduled_ns)?,
        })
    }
}
pub(crate) fn measuring() -> bool {
    RECORDER
        .slots
        .iter()
        .any(|slot| slot.state.load(Ordering::Relaxed) == ACQUIRING)
}
pub(crate) fn thread_work() -> Option<ThreadWork> {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentThread, GetThreadTimes},
    };
    unsafe extern "system" {
        fn QueryThreadCycleTime(
            thread: windows_sys::Win32::Foundation::HANDLE,
            cycles: *mut u64,
        ) -> i32;
    }
    let [mut created, mut exited, mut kernel, mut user] = [FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    }; 4];
    let mut cycles = 0;
    let thread = unsafe { GetCurrentThread() };
    if unsafe { GetThreadTimes(thread, &mut created, &mut exited, &mut kernel, &mut user) } == 0
        || unsafe { QueryThreadCycleTime(thread, &mut cycles) } == 0
    {
        return None;
    }
    let ticks =
        |time: FILETIME| u64::from(time.dwLowDateTime) | (u64::from(time.dwHighDateTime) << 32);
    Some(ThreadWork {
        cycles,
        scheduled_ns: (ticks(kernel) + ticks(user)).saturating_mul(100),
    })
}

const CALLBACKS: usize = 2;
const JOBS: usize = 8;
const HANDOFFS: usize = 2 * crate::acquisition_jobs::MAX_JOBS;
const JOB_NAMES: [&str; JOBS] = [
    "Governance",
    "GatedProjects",
    "IntelResidual",
    "Slow",
    "Characters",
    "PresentationPrepare",
    "PresentationSettle",
    "Variants",
];

#[derive(Clone, Copy)]
struct Body {
    timeline: Timeline,
    succeeded: bool,
}
#[derive(Clone, Copy)]
struct Handoff {
    identity: crate::acquisition_diagnostics::Identity,
    body: Body,
}
#[derive(Clone, Copy)]
struct CharacterCopy {
    backend_ns: u64,
    raw_bytes: u64,
    planned_succeeded: bool,
    planned_failed: bool,
    quiet_roster: bool,
}
#[derive(Clone, Copy)]
struct Sample {
    hour: i32,
    qpc: i64,
    frequency: u64,
    callback_ns: u64,
    cost: HourCost,
    tables: [u64; TABLE_COUNT],
    reads: [ReadCounts; TABLE_COUNT],
    presentation: [u64; division_presentation::PHASES],
    jobs: [Option<Body>; JOBS],
    handoffs: [Option<Handoff>; HANDOFFS],
    handoffs_len: usize,
    handoffs_complete: bool,
    technology_certified: Option<bool>,
    projects_certified: Option<bool>,
    characters: Option<CharacterCopy>,
    governance_phases: Option<[u64; 9]>,
    projects_phases: Option<[u64; 6]>,
    project_variables_decode_ns: Option<u64>,
    complete: bool,
    unavailable: bool,
}
impl Sample {
    fn new(hour: i32, qpc: i64, frequency: u64) -> Self {
        Self {
            hour,
            qpc,
            frequency,
            callback_ns: 0,
            cost: HourCost::default(),
            tables: [0; TABLE_COUNT],
            reads: [ReadCounts::ZERO; TABLE_COUNT],
            presentation: [0; division_presentation::PHASES],
            jobs: [None; JOBS],
            handoffs: [None; HANDOFFS],
            handoffs_len: 0,
            handoffs_complete: true,
            technology_certified: None,
            projects_certified: None,
            characters: None,
            governance_phases: None,
            projects_phases: None,
            project_variables_decode_ns: None,
            complete: false,
            unavailable: false,
        }
    }
    fn status(&self) -> impl serde::Serialize + '_ {
        let frequency = self.frequency;
        let ns = move |ticks| crate::acquisition_diagnostics::ns(ticks, frequency);
        status!({
            "hour": self.hour, "qpc": self.qpc, "complete": self.complete,
            "unavailable": self.unavailable,
            "unavailable_reason": self.unavailable.then_some("nonblocking diagnostic staging or handoff contention"),
            "callback_ns": self.complete.then_some(self.callback_ns),
            "first_since_attach": self.complete.then_some(self.cost.first_since_attach),
            "continuous": self.complete.then_some(self.cost.continuous),
            "force_full": self.complete.then_some(self.cost.force_full),
            "recovery": self.complete.then_some(self.cost.recovery),
            "refresh": self.complete.then_some(self.cost.refresh),
            "setup_ns": self.complete.then_some(self.cost.setup_ns),
            "parallel_ns": self.complete.then_some(self.cost.parallel_ns),
            "residual_ns": self.complete.then_some(self.cost.residual_ns),
            "tables": sequence(TableId::ALL.into_iter().filter(move |table| self.tables[table.index()] != 0 || self.reads[table.index()] != ReadCounts::ZERO).map(move |table| {
                let index = table.index(); let reads = self.reads[index];
                status!({ "table": DebugValue(table), "wall_ns": (self.tables[index] != 0).then_some(self.tables[index]),
                    "copy_calls": reads.copy_calls, "scattered_calls": reads.scattered_calls,
                    "record_calls": reads.record_calls, "plan_calls": reads.plan_calls,
                    "spans": reads.spans, "bytes": reads.bytes })
            })),
            "jobs": map(JOB_NAMES.into_iter().zip(self.jobs).map(move |(name, body)| (name, body.map(|body| {
                let t = body.timeline;
                let duration = |a: i64, b: i64| (a != 0 && b >= a).then(|| ns((b-a) as u64));
                status!({ "succeeded": body.succeeded,
                    "queued_qpc": (t.queued_qpc != 0).then_some(t.queued_qpc),
                    "body_begin_qpc": (t.body_begin_qpc != 0).then_some(t.body_begin_qpc),
                    "body_end_qpc": (t.body_end_qpc != 0).then_some(t.body_end_qpc),
                    "queue_ns": duration(t.queued_qpc, t.body_begin_qpc),
                    "body_ns": duration(t.body_begin_qpc, t.body_end_qpc),
                    "publication_ns": duration(t.body_end_qpc, t.published_qpc),
                    "growths": t.growths, "callback_thread": t.caller,
                    "thread_cycles": t.thread_work.map(|work| work.cycles),
                    "thread_scheduled_ns": t.thread_work.map(|work| work.scheduled_ns) })
            })))),
            "handoffs_complete": self.handoffs_complete,
            "handoffs": sequence(self.handoffs[..self.handoffs_len].iter().flatten().map(move |handoff| { let timeline = handoff.body.timeline; status!({ "succeeded": handoff.body.succeeded, "identity": DebugValue(crate::acquisition_jobs::ROLES[handoff.identity.role]), "part": handoff.identity.part, "kind": handoff.identity.kind, "queued_qpc": timeline.queued_qpc, "body_begin_qpc": timeline.body_begin_qpc, "body_end_qpc": timeline.body_end_qpc, "published_qpc": timeline.published_qpc, "collected_qpc": timeline.collected_qpc, "growths": timeline.growths, "thread_cycles": timeline.thread_work.map(|work| work.cycles), "thread_scheduled_ns": timeline.thread_work.map(|work| work.scheduled_ns) }) })),
            "technology_certified": self.technology_certified,
            "projects_certified": self.projects_certified,
            "characters_copy": self.characters.map(|copy| status!({
                "backend_ns": copy.backend_ns, "raw_bytes": copy.raw_bytes,
                "planned_succeeded": copy.planned_succeeded, "planned_failed": copy.planned_failed,
                "quiet_roster": copy.quiet_roster,
            })),
            "presentation_phases_ns": map(division_presentation::PHASE_NAMES.into_iter().zip(self.presentation).map(move |(name,ticks)| (name,(ticks != 0).then(|| ns(ticks))))),
            "governance_phases_ns": self.governance_phases.map(|phases| map(collector_governance::governance::ACQUISITION_PHASE_NAMES.into_iter().zip(phases).map(move |(name, ticks)| (name, (ticks != 0).then(|| ns(ticks)))))),
            "projects_phases_ns": self.projects_phases.map(|phases| map(collector_projects::projects::ACQUISITION_PHASE_NAMES.into_iter().zip(phases).map(move |(name, ticks)| (name, (ticks != 0).then(|| ns(ticks)))))),
            "variants": self.cost.variants,
            "project_variables_decode_ns": self.project_variables_decode_ns,
        })
    }
}
const UNSTARTED: u8 = 0;
const ACQUIRING: u8 = 1;
const PUBLISHED: u8 = 2;
const UNAVAILABLE: u8 = 3;

struct Slot {
    state: AtomicU8,
    hour: AtomicI32,
    qpc: AtomicI64,
    frequency: AtomicU64,
    sample: Mutex<Option<Sample>>,
    decoded: AtomicBool,
    decode_ns: AtomicU64,
}
impl Slot {
    const fn new() -> Self {
        Self {
            state: AtomicU8::new(UNSTARTED),
            hour: AtomicI32::new(0),
            qpc: AtomicI64::new(0),
            frequency: AtomicU64::new(0),
            sample: Mutex::new(None),
            decoded: AtomicBool::new(false),
            decode_ns: AtomicU64::new(0),
        }
    }
    fn begin(&self, hour: i32, qpc: i64, frequency: u64) {
        self.hour.store(hour, Ordering::Relaxed);
        self.qpc.store(qpc, Ordering::Relaxed);
        self.frequency.store(frequency, Ordering::Relaxed);
        self.state.store(ACQUIRING, Ordering::Release);
    }
    fn unavailable(&self) {
        self.state.store(UNAVAILABLE, Ordering::Release);
    }
    fn publish(&self, sample: Sample) {
        if self.state.load(Ordering::Acquire) != ACQUIRING {
            return;
        }
        match self.sample.try_lock() {
            Ok(mut stored) => {
                *stored = Some(sample);
                self.state.store(PUBLISHED, Ordering::Release);
            }
            Err(_) => self.unavailable(),
        }
    }
    fn snapshot(&self) -> Option<Sample> {
        let state = self.state.load(Ordering::Acquire);
        if state == UNSTARTED {
            return None;
        }
        let mut sample = Sample::new(
            self.hour.load(Ordering::Relaxed),
            self.qpc.load(Ordering::Relaxed),
            self.frequency.load(Ordering::Relaxed),
        );
        sample.unavailable = state == UNAVAILABLE;
        if state == PUBLISHED {
            match self.sample.try_lock() {
                Ok(stored) => match *stored {
                    Some(stored) => sample = stored,
                    None => sample.unavailable = true,
                },
                Err(_) => sample.unavailable = true,
            }
        }
        if self.decoded.load(Ordering::Acquire) {
            sample.project_variables_decode_ns = Some(self.decode_ns.load(Ordering::Relaxed));
        }
        Some(sample)
    }
}
struct Recorder {
    next: AtomicUsize,
    slots: [Slot; CALLBACKS],
}
impl Recorder {
    const fn new() -> Self {
        Self {
            next: AtomicUsize::new(0),
            slots: [const { Slot::new() }; CALLBACKS],
        }
    }
    fn begin(&self, hour: i32, qpc: i64, frequency: u64) -> Option<usize> {
        if self.next.load(Ordering::Relaxed) >= CALLBACKS {
            return None;
        }
        let index = self.next.fetch_add(1, Ordering::Relaxed);
        if index >= CALLBACKS {
            return None;
        }
        self.slots[index].begin(hour, qpc, frequency);
        Some(index)
    }
    fn decoded(&self, qpc: i64, ns: u64) {
        for slot in &self.slots {
            if slot.state.load(Ordering::Acquire) != UNSTARTED
                && slot.qpc.load(Ordering::Relaxed) == qpc
            {
                slot.decode_ns.store(ns, Ordering::Relaxed);
                slot.decoded.store(true, Ordering::Release);
            }
        }
    }
}
static DECODE_ENABLED: AtomicBool = AtomicBool::new(true);
static RECORDER: Recorder = Recorder::new();
thread_local! {
    static ACTIVE: Cell<Option<usize>> = const { Cell::new(None) };
    static STAGING: RefCell<Option<Sample>> = const { RefCell::new(None) };
}

pub(crate) struct Callback {
    index: Option<usize>,
    thread: PhantomData<*mut ()>,
}
pub(crate) fn begin(hour: i32, qpc: i64, frequency: u64) -> Callback {
    let mut index = RECORDER.begin(hour, qpc, frequency);
    if let Some(current) = index {
        let staged = ACTIVE.get().is_none()
            && STAGING.with(|staging| {
                let Ok(mut staging) = staging.try_borrow_mut() else {
                    return false;
                };
                *staging = Some(Sample::new(hour, qpc, frequency));
                true
            });
        if staged {
            ACTIVE.set(Some(current));
        } else {
            RECORDER.slots[current].unavailable();
            index = None;
        }
    }
    Callback {
        index,
        thread: PhantomData,
    }
}
impl Callback {
    pub(crate) fn finish(mut self, frame: &Frame, callback_ns: u64) {
        self.publish(Some((frame, callback_ns)));
    }
    fn publish(&mut self, frame: Option<(&Frame, u64)>) {
        let Some(index) = self.index.take() else {
            return;
        };
        ACTIVE.set(None);
        let sample = STAGING.with(|staging| {
            staging
                .try_borrow_mut()
                .ok()
                .and_then(|mut staging| staging.take())
        });
        let Some(mut sample) = sample else {
            RECORDER.slots[index].unavailable();
            return;
        };
        if let Some((frame, callback_ns)) = frame {
            sample.callback_ns = callback_ns;
            sample.cost = frame.hour_cost;
            sample.tables = frame.table_ns;
            sample.reads = frame.table_reads;
            sample.presentation = frame.extended.presentation_phase_ticks();
            sample.complete = true;
        }
        RECORDER.slots[index].publish(sample);
    }
}
impl Drop for Callback {
    fn drop(&mut self) {
        self.publish(None);
    }
}
pub(crate) fn completed(completed: &Completion<Job, Report>) {
    let Some(active) = ACTIVE.get() else {
        return;
    };
    let index = match &completed.request {
        Job::Governance(_) => 0,
        Job::GatedProjects(_) => 1,
        Job::IntelResidual(_) => 2,
        Job::Slow(_) => 3,
        Job::Characters(request) if request.payload.part == 0 => 4,
        Job::PresentationPrepare(_) => 5,
        Job::PresentationSettle(_) => 6,
        Job::Variants(_) => 7,
        _ => JOBS,
    };
    STAGING.with(|staging| {
        let Ok(mut staging) = staging.try_borrow_mut() else {
            RECORDER.slots[active].unavailable();
            return;
        };
        let Some(sample) = staging.as_mut() else {
            RECORDER.slots[active].unavailable();
            return;
        };
        if sample.handoffs_len < HANDOFFS {
            sample.handoffs[sample.handoffs_len] = Some(Handoff {
                identity: completed.request.diagnostic_identity(),
                body: Body {
                    timeline: completed.timeline,
                    succeeded: completed.report.is_ok(),
                },
            });
            sample.handoffs_len += 1;
        } else {
            sample.handoffs_complete = false;
        }
        let workers = match &completed.request {
            Job::Governance(request) => Some(&request.workers),
            Job::GatedProjects(request) => Some(&request.workers),
            Job::Variants(request) => Some(&request.workers),
            _ => None,
        };
        if let Some(workers) = workers {
            let complete = workers.for_each_handoff(|handoff| {
                if sample.handoffs_len < HANDOFFS {
                    sample.handoffs[sample.handoffs_len] = Some(Handoff {
                        identity: handoff.identity,
                        body: Body {
                            timeline: handoff.timeline,
                            succeeded: handoff.succeeded,
                        },
                    });
                    sample.handoffs_len += 1;
                } else {
                    sample.handoffs_complete = false;
                }
            });
            sample.handoffs_complete &= complete;
        }
        if index == JOBS {
            return;
        }
        sample.jobs[index] = Some(Body {
            timeline: completed.timeline,
            succeeded: completed.report.is_ok(),
        });
        if let Job::Governance(request) = &completed.request {
            sample.governance_phases = request.captured.acquisition_phases();
            sample.technology_certified = Some(request.captured.technology_certified());
        }
        if let Job::GatedProjects(request) = &completed.request {
            sample.projects_phases = request.captured.acquisition_phases();
            sample.projects_certified = Some(request.captured.projects_certified());
        }
        if let Job::Characters(request) = &completed.request {
            let log = &request.payload.log;
            sample.characters = Some(CharacterCopy {
                backend_ns: log.backend_ns,
                raw_bytes: log.raw_bytes.iter().sum(),
                planned_succeeded: log.planned_capture_succeeded(),
                planned_failed: log.planned_capture_failed(),
                quiet_roster: request.payload.quiet_roster(),
            });
        }
    });
}
pub(crate) fn project_variables_decoded(qpc: i64, ns: u64) {
    if !DECODE_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    RECORDER.decoded(qpc, ns);
    let last = &RECORDER.slots[CALLBACKS - 1];
    if last.state.load(Ordering::Acquire) != UNSTARTED && qpc >= last.qpc.load(Ordering::Relaxed) {
        DECODE_ENABLED.store(false, Ordering::Relaxed);
    }
}
pub(crate) struct Status;
impl serde::Serialize for Status {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let samples = RECORDER.slots.each_ref().map(Slot::snapshot);
        status!({
            "scope": "First two clocked acquisition callbacks since DLL attach, retained permanently; status only. Callback-owned staging and try-lock handoff never wait for status or decode. Handoffs retain every top-level callback collection and owned cold-child timelines returned by their parent; kind 4=technology, 5=project pools, 6=cold optional storage, 7=cold context seeds, 8=project runtime, 9=cold storage warmup (inclusive of kind-6 children). Projects project_native includes both fan-outs and their ordered join; runtime_native is the remaining serial tail. Local child publication/collection stamps denote return, not an executor handoff. Children remain inclusive in parent timing; overflow sets handoffs_complete=false without substituting later jobs. Ticket is not retained in these collection copies. Contention is unavailable, unwound acquisition is incomplete, and neither is replaced by a later callback. Decode timings use independent atomic publication. Tables copy existing acquisition timers and raw source-request counters before decode: shared reads stay charged to their anchors, absent/zero timers are null or omitted, not measured zero. Job bodies, tables and nested phases overlap and must not be added. Thread cycles measure scheduled execution, not nanoseconds; thread_scheduled_ns is GetThreadTimes at scheduler-tick resolution. Governance native includes guards, validation, dictionary admission and allocation; fingerprint is only hashing. Hold and politics_finish build certified owned caches. Inclusive capture/family phases contain their child phases. Characters is part 0; backend_ns is nested guarded-copy wall, not pure memory latency. Presentation phases are acquired before FIFO decode; unset phases are null. Variants retains the existing inclusive cold-walk/context/part profile.",
            "callbacks": sequence(samples.iter().map(|sample| sample.as_ref().map(Sample::status))),
        }).serialize(serializer)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retains_first_two_and_keeps_incomplete_or_missing_timings_explicit() {
        let recorder = Recorder::new();
        assert!(recorder.slots[0].snapshot().is_none());
        assert_eq!(recorder.begin(10, 100, 10), Some(0));
        assert_eq!(recorder.begin(11, 200, 10), Some(1));
        assert_eq!(recorder.begin(12, 300, 10), None);
        recorder.slots[0].publish(Sample::new(10, 100, 10));
        let first = recorder.slots[0].snapshot().unwrap();
        let value = serde_json::to_value(first.status()).unwrap();
        assert_eq!(value["hour"], 10);
        assert_eq!(value["complete"], false);
        assert_eq!(value["unavailable"], false);
        assert!(value["callback_ns"].is_null());
        assert!(value["jobs"]["Slow"].is_null());
        assert!(value["presentation_phases_ns"]["sources_fallback"].is_null());
        assert_eq!(value["tables"].as_array().unwrap().len(), 0);
        assert_eq!(recorder.slots[1].snapshot().unwrap().hour, 11);
    }
    #[test]
    fn held_status_lock_refuses_handoff_without_waiting_or_replacing_the_hour() {
        let recorder = Recorder::new();
        assert_eq!(recorder.begin(10, 100, 10), Some(0));
        let held = recorder.slots[0].sample.lock().unwrap();
        let mut sample = Sample::new(10, 100, 10);
        sample.complete = true;
        recorder.slots[0].publish(sample);
        let lost = recorder.slots[0].snapshot().unwrap();
        assert!(lost.unavailable && !lost.complete);
        assert_eq!(lost.hour, 10);
        drop(held);
        assert_eq!(recorder.begin(11, 200, 10), Some(1));
        assert_eq!(recorder.begin(12, 300, 10), None);
        recorder.slots[0].publish(Sample::new(12, 300, 10));
        assert_eq!(recorder.slots[0].snapshot().unwrap().hour, 10);
        assert!(recorder.slots[0].snapshot().unwrap().unavailable);
    }
    #[test]
    fn decode_publication_never_takes_the_callback_handoff_lock() {
        let recorder = Recorder::new();
        recorder.begin(10, 100, 10);
        recorder.begin(11, 200, 10);
        let mut sample = Sample::new(10, 100, 10);
        sample.complete = true;
        recorder.slots[0].publish(sample);
        let held = recorder.slots[0].sample.lock().unwrap();
        recorder.decoded(100, 123);
        recorder.decoded(300, 999);
        drop(held);
        assert_eq!(
            recorder.slots[0]
                .snapshot()
                .unwrap()
                .project_variables_decode_ns,
            Some(123)
        );
        assert!(
            recorder.slots[1]
                .snapshot()
                .unwrap()
                .project_variables_decode_ns
                .is_none()
        );
    }
}
