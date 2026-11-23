#[cfg(feature = "callback_profile")]
pub(crate) const SEGMENTS_SCOPE: &str = "game-thread wall of the parallel phase split at fixed marks: 0 first held burst of independent jobs; 1 completion-driven dependency scheduling through Late/history and FinalInputs dispatch (root census -> Orders/parts -> stitch -> Fronts and, with Naval/wing, ChildFronts; division inputs/parts -> Divisions -> ArmyIcons and PresentationPrepare -> status ranges -> Presentation; census consumers, raw verdicts, map sample, AirMissions and ScoresRails are joined only when their full producer barrier is ready); 2-7 unused; 8 weather/lexical + core joins (presentation publication edges continue during these and later joins); 9 supply/portraits/order/template/slow/war overview/stability/raid/gated/character input/governance/research joins + event-feed seal dispatch + ShipModels detail joins + Ordinary family joins + event-feed seal join + slot-8 production and part joins + independent Variants join; 10 capture_tables; 11 Late join + Fronts join + presentation and movement joins + order transport + border inputs; 12 history-part joins + FinalInputs join + capture_tables_late; 13 Intel and AirBases joins + visibility + failure handling; 14 ShipModels completed projection";
use crate::SEGMENTS;
use crate::callback_profile_state::{JOIN_SCALARS, JoinSample, thread_cycles};
#[cfg(test)]
use crate::callback_profile_state::{record_join, reset_join, take_join};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const LIMIT: usize = 1024;
const THRESHOLD_NS: u64 = 10_000_000;
const CALLBACK_SCALARS: usize = 14;
const SCALARS: usize = CALLBACK_SCALARS + JOIN_SCALARS;
const BASE_WIDTH: usize = SCALARS + SEGMENTS + crate::roles::ROLE_COUNT;
const WIDTH: usize = BASE_WIDTH + 1;
static SAMPLES: Samples<LIMIT> = Samples::new();

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub hour: Option<i32>,
    pub qpc_begin: i64,
    pub qpc_end: i64,
    pub wall_ns: u64,
    pub thread_cycles: Option<u64>,
    pub thread_id: u32,
    pub frame_grow_ns: u64,
    pub setup_ns: u64,
    pub parallel_ns: u64,
    pub residual_ns: u64,
    pub dispatch_ns: u64,
    pub phase_calls: u64,
    pub join: JoinSample,
    pub segments_ns: [u64; SEGMENTS],
    pub join_wait_ns: [u64; crate::roles::ROLE_COUNT],
    pub frequency: u64,
}

impl Default for Sample {
    fn default() -> Self {
        Self {
            hour: None,
            qpc_begin: 0,
            qpc_end: 0,
            wall_ns: 0,
            thread_cycles: None,
            thread_id: 0,
            frame_grow_ns: 0,
            setup_ns: 0,
            parallel_ns: 0,
            residual_ns: 0,
            dispatch_ns: 0,
            phase_calls: 0,
            join: JoinSample::default(),
            segments_ns: [0; SEGMENTS],
            join_wait_ns: [0; crate::roles::ROLE_COUNT],
            frequency: 1,
        }
    }
}

impl Sample {
    fn fields(self) -> [u64; WIDTH] {
        let mut fields = [0; WIDTH];
        fields[..CALLBACK_SCALARS].copy_from_slice(&[
            u64::from(self.hour.is_some()),
            self.hour.unwrap_or(0) as u32 as u64,
            self.qpc_begin as u64,
            self.qpc_end as u64,
            self.wall_ns,
            u64::from(self.thread_cycles.is_some()),
            self.thread_cycles.unwrap_or(0),
            self.frame_grow_ns,
            self.setup_ns,
            self.parallel_ns,
            self.residual_ns,
            self.dispatch_ns,
            self.phase_calls,
            u64::from(self.thread_id),
        ]);
        fields[CALLBACK_SCALARS..SCALARS].copy_from_slice(&self.join.fields());
        let joins = SCALARS + SEGMENTS;
        fields[SCALARS..joins].copy_from_slice(&self.segments_ns);
        fields[joins..BASE_WIDTH].copy_from_slice(&self.join_wait_ns);
        fields[WIDTH - 1] = self.frequency;
        fields
    }

    fn from_fields(fields: [u64; WIDTH]) -> Self {
        let joins = SCALARS + SEGMENTS;
        Self {
            hour: (fields[0] != 0).then_some(fields[1] as u32 as i32),
            qpc_begin: fields[2] as i64,
            qpc_end: fields[3] as i64,
            wall_ns: fields[4],
            thread_cycles: (fields[5] != 0).then_some(fields[6]),
            thread_id: fields[13] as u32,
            frame_grow_ns: fields[7],
            setup_ns: fields[8],
            parallel_ns: fields[9],
            residual_ns: fields[10],
            dispatch_ns: fields[11],
            phase_calls: fields[12],
            join: JoinSample::from_fields(fields[CALLBACK_SCALARS..SCALARS].try_into().unwrap()),
            segments_ns: fields[SCALARS..joins].try_into().unwrap(),
            join_wait_ns: fields[joins..BASE_WIDTH].try_into().unwrap(),
            frequency: fields[WIDTH - 1],
        }
    }

    fn json(self) -> impl serde::Serialize {
        collector_status::status::status!({
            "hour": self.hour,
            "qpc_begin": self.qpc_begin,
            "qpc_end": self.qpc_end,
            "wall_ticks": self.qpc_end.saturating_sub(self.qpc_begin).max(0),
            "wall_ns": self.wall_ns,
            "thread_cycles": self.thread_cycles,
            "thread_id": self.thread_id,
            "frame_grow_ns": self.frame_grow_ns,
            "setup_ns": self.setup_ns,
            "parallel_ns": self.parallel_ns,
            "residual_ns": self.residual_ns,
            "dispatch_ns": self.dispatch_ns,
            "phase_calls": self.phase_calls,
            "join": self.join.json(),
            "segments_ns": collector_status::status::sequence(self.segments_ns.into_iter()),
            "join_wait_ns": collector_status::status::sequence(self.join_wait_ns.into_iter()),
        })
    }
}

pub struct Stamp {
    qpc_begin: i64,
    cycles: Option<u64>,
    thread: std::marker::PhantomData<*mut ()>,
}

impl Stamp {
    pub fn start() -> Self {
        Self {
            qpc_begin: collector_primitives::clock::qpc(),
            cycles: thread_cycles(),
            thread: std::marker::PhantomData,
        }
    }

    pub fn finish(self, frequency: u64) -> Sample {
        let cycles = thread_cycles();
        let qpc_end = collector_primitives::clock::qpc();
        let ticks = qpc_end.saturating_sub(self.qpc_begin).max(0) as u128;
        Sample {
            qpc_begin: self.qpc_begin,
            qpc_end,
            frequency,
            wall_ns: (ticks * 1_000_000_000 / u128::from(frequency)).min(u128::from(u64::MAX))
                as u64,
            thread_cycles: self
                .cycles
                .zip(cycles)
                .and_then(|(begin, end)| end.checked_sub(begin)),
            thread_id: unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() },
            ..Sample::default()
        }
    }
}

struct Slot {
    ready: AtomicBool,
    fields: [AtomicU64; WIDTH],
}

impl Slot {
    const fn new() -> Self {
        Self {
            ready: AtomicBool::new(false),
            fields: [const { AtomicU64::new(0) }; WIDTH],
        }
    }

    fn publish(&self, sample: Sample) {
        for (field, value) in self.fields.iter().zip(sample.fields()) {
            field.store(value, Ordering::Relaxed);
        }
        self.ready.store(true, Ordering::Release);
    }

    fn read(&self) -> Option<Sample> {
        self.ready.load(Ordering::Acquire).then(|| {
            Sample::from_fields(
                self.fields
                    .each_ref()
                    .map(|field| field.load(Ordering::Relaxed)),
            )
        })
    }
}

struct Samples<const N: usize> {
    observations: AtomicU64,
    qualifying: AtomicU64,
    slots: [Slot; N],
}

impl<const N: usize> Samples<N> {
    const fn new() -> Self {
        Self {
            observations: AtomicU64::new(0),
            qualifying: AtomicU64::new(0),
            slots: [const { Slot::new() }; N],
        }
    }

    fn record(&self, sample: Sample) {
        self.observations.fetch_add(1, Ordering::Relaxed);
        if sample.wall_ns < THRESHOLD_NS {
            return;
        }
        let index = self.qualifying.fetch_add(1, Ordering::Relaxed);
        if index < N as u64 {
            self.slots[index as usize].publish(sample);
        }
    }

    fn status(&self) -> impl serde::Serialize + '_ {
        let qualifying = self.qualifying.load(Ordering::Relaxed);
        let reserved = qualifying.min(N as u64) as usize;
        let ready = std::array::from_fn::<_, N, _>(|index| {
            index < reserved && self.slots[index].read().is_some()
        });
        let published = ready.iter().filter(|&&ready| ready).count();
        let samples = collector_status::status::sequence(
            self.slots
                .iter()
                .zip(ready)
                .filter(|&(_, ready)| ready)
                .map(|(slot, _)| slot.read().unwrap().json()),
        );
        let roles = collector_status::status::sequence(
            crate::roles::ROLES
                .iter()
                .map(collector_status::status::DebugValue),
        );
        collector_status::status::status!({
            "threshold_ns": THRESHOLD_NS,
            "capacity": N,
            "observations": self.observations.load(Ordering::Relaxed),
            "qualifying_observations": qualifying,
            "published": published,
            "pending_publication": reserved - published,
            "dropped": qualifying.saturating_sub(N as u64),
            "segments_scope": SEGMENTS_SCOPE,
            "join_role_order": roles,
            "join_scope": "Largest join or readiness wall_ns within each callback, first on ties. Readiness has no individual join lifecycle (null fields). *_qpc fields are raw session QPC timestamps; *_wait_ticks are raw QPC durations, not nanoseconds. Divide tick differences by the session QPC frequency to obtain seconds. Zero lifecycle timestamps can mean unavailable stages, including rejected work. runner_thread_cycles is the runner thread's cycle delta; null means unavailable. Join lifecycle intervals overlap callback and phase intervals.",
            "scope": "First qualifying callback samples for this DLL attachment; no overwriting. Wall covers the instrumented capture's acquisition interval after the hourly update returned, including reusable-buffer wait. Setup/parallel/residual are completed body counter deltas; dispatch, segments and joins overlap those phases and must not be added to them. Zero phase_calls can denote a failed or cancelled callback. Thread cycles exclude descheduled time and are not nanoseconds; null means unavailable. Samples are published atomically; live counters are not a simultaneous snapshot. Recording the sample occurs after its measured interval.",
            "samples": samples,
        })
    }
}

/// # Safety
/// Only the admitted capture callback may publish its own sample for the current hour, once that
/// callback's measurements are complete.
pub unsafe fn record(sample: Sample) {
    SAMPLES.record(sample);
}

pub fn status() -> impl serde::Serialize + 'static {
    SAMPLES.status()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_capacity_and_overflow_keep_first_qualifying_samples() {
        let samples = Samples::<2>::new();
        let sample = |hour, wall_ns| Sample {
            hour: Some(hour),
            wall_ns,
            ..Sample::default()
        };
        samples.record(sample(1, THRESHOLD_NS - 1));
        samples.record(sample(2, THRESHOLD_NS));
        samples.record(sample(3, THRESHOLD_NS + 1));
        samples.record(sample(4, THRESHOLD_NS * 2));
        samples.record(sample(5, 0));
        let status = serde_json::to_value(samples.status()).unwrap();
        assert_eq!(status["observations"], 5);
        assert_eq!(status["qualifying_observations"], 3);
        assert_eq!(status["published"], 2);
        assert_eq!(status["pending_publication"], 0);
        assert_eq!(status["dropped"], 1);
        assert_eq!(status["samples"][0]["hour"], 2);
        assert_eq!(status["samples"][1]["hour"], 3);
    }

    #[test]
    fn unpublished_fields_stay_hidden_and_optional_values_are_not_zero() {
        let samples = Samples::<1>::new();
        samples.observations.store(1, Ordering::Relaxed);
        samples.qualifying.store(1, Ordering::Relaxed);
        samples.slots[0].fields[4].store(THRESHOLD_NS, Ordering::Relaxed);
        let pending = serde_json::to_value(samples.status()).unwrap();
        assert_eq!(pending["published"], 0);
        assert_eq!(pending["pending_publication"], 1);
        let sample = Sample {
            hour: Some(i32::MIN),
            qpc_begin: -20,
            qpc_end: i64::MAX,
            wall_ns: THRESHOLD_NS,
            thread_cycles: Some(0),
            thread_id: 123,
            frame_grow_ns: 7,
            setup_ns: 8,
            parallel_ns: 9,
            residual_ns: 10,
            dispatch_ns: 11,
            phase_calls: 1,
            join: JoinSample {
                role: Some(2),
                wall_ns: 1234,
                readiness: false,
                profile: crate::acquisition::Profile {
                    queued_qpc: -1,
                    claimed_qpc: 2,
                    runner_begin_qpc: 3,
                    runner_end_qpc: 4,
                    published_qpc: 5,
                    collected_qpc: i64::MAX,
                    submit_lock_wait_ticks: 6,
                    finish_lock_wait_ticks: 8,
                    runner_thread_cycles: Some(0),
                    runner_thread_id: 456,
                },
            },
            segments_ns: std::array::from_fn(|index| 100 + index as u64),
            join_wait_ns: std::array::from_fn(|index| 200 + index as u64),
            frequency: 10_000_000,
        };
        samples.slots[0].publish(sample);
        assert_eq!(
            samples.slots[0].read().map(Sample::fields),
            Some(sample.fields())
        );
        let status = serde_json::to_value(samples.status()).unwrap();
        assert_eq!(status["pending_publication"], 0);
        assert_eq!(status["samples"][0]["hour"], i32::MIN);
        assert_eq!(status["samples"][0]["thread_cycles"], 0);
        assert_eq!(status["samples"][0]["thread_id"], 123);
        assert_eq!(status["samples"][0]["join"]["role"], 2);
        assert_eq!(status["samples"][0]["join"]["runner_thread_cycles"], 0);
        assert_eq!(status["samples"][0]["join"]["runner_thread_id"], 456);
        assert_eq!(status["samples"][0]["join"]["finish_lock_wait_ticks"], 8);
        let join = status["samples"][0]["join"].as_object().unwrap();
        assert!(!join.contains_key("completion_lock_wait_ticks"));
        assert!(!join.contains_key("wake_begin_qpc"));
        assert!(!join.contains_key("wake_end_qpc"));
        let missing = serde_json::to_value(Sample::default().json()).unwrap();
        assert!(missing["hour"].is_null());
        assert!(missing["thread_cycles"].is_null());
        assert!(missing["join"]["role"].is_null());
        assert!(missing["join"]["runner_thread_cycles"].is_null());
    }

    #[test]
    fn slowest_join_retains_first_tie_and_does_not_cross_callbacks() {
        reset_join();
        assert!(take_join().role.is_none());
        record_join(0, 10, crate::acquisition::Profile::default());
        record_join(1, 9, crate::acquisition::Profile::default());
        record_join(2, 10, crate::acquisition::Profile::default());
        assert_eq!(take_join().role, Some(0));
        assert!(take_join().role.is_none());
        record_join(3, 11, crate::acquisition::Profile::default());
        record_join(4, 12, crate::acquisition::Profile::default());
        assert_eq!(take_join().role, Some(4));
        record_join(5, 13, crate::acquisition::Profile::default());
        reset_join();
        assert!(take_join().role.is_none());
    }
}
