//! Bounded callback tails over the same recorded hours as callback_work.

use crate::{
    acquisition_jobs,
    sampling::HourCost,
    status::{DebugValue, sequence, status},
};

const CAPACITY: usize = 64;
const HISTOGRAM_FLOOR_MS: [u64; 12] = [0, 2, 4, 8, 16, 32, 50, 64, 100, 128, 256, 1000];
/// The per-hour bound on the game-thread block.
const BUDGET_NS: u64 = 50_000_000;
const SLOTS: usize = replay_data::cadence::STAGGER_PERIOD as usize;

/// Every hour over [`BUDGET_NS`], split by what held the game thread.
#[derive(Clone, Copy)]
struct OverBudget {
    hours: u64,
    with_buffer_wait: u64,
    /// Over budget even without its buffer wait; the splits below count these.
    acquisition: u64,
    acquisition_max_ns: u64,
    by_free_frames: [u64; crate::sampling::BUFFERS],
    by_daily_slot: [u64; SLOTS],
    by_dominant_segment: [u64; crate::sampling::SEGMENTS],
    setup_over_parallel: u64,
    by_slowest_join: [u64; acquisition_jobs::ROLE_COUNT],
    no_slowest_join: u64,
    force_full: u64,
    /// Over budget with at least half the budget blocked on the allocator.
    allocator_bound: u64,
}

impl Default for OverBudget {
    fn default() -> Self {
        Self {
            hours: 0,
            with_buffer_wait: 0,
            acquisition: 0,
            acquisition_max_ns: 0,
            by_free_frames: [0; crate::sampling::BUFFERS],
            by_daily_slot: [0; SLOTS],
            by_dominant_segment: [0; crate::sampling::SEGMENTS],
            setup_over_parallel: 0,
            by_slowest_join: [0; acquisition_jobs::ROLE_COUNT],
            no_slowest_join: 0,
            force_full: 0,
            allocator_bound: 0,
        }
    }
}

impl OverBudget {
    fn record(&mut self, hour: i32, callback_ns: u64, cost: &HourCost) {
        if callback_ns <= BUDGET_NS {
            return;
        }
        self.hours += 1;
        let free = usize::from(cost.free_frames).min(crate::sampling::BUFFERS - 1);
        self.by_free_frames[free] += 1;
        if cost.buffer_wait_ns != 0 {
            self.with_buffer_wait += 1;
        }
        let acquisition_ns = callback_ns.saturating_sub(cost.buffer_wait_ns);
        if acquisition_ns <= BUDGET_NS {
            return;
        }
        self.acquisition += 1;
        self.acquisition_max_ns = self.acquisition_max_ns.max(acquisition_ns);
        self.by_daily_slot[hour.rem_euclid(SLOTS as i32) as usize] += 1;
        if let Some(segment) = dominant_segment(cost) {
            self.by_dominant_segment[segment] += 1;
        }
        if cost.setup_ns > cost.parallel_ns {
            self.setup_over_parallel += 1;
        }
        match cost.slowest_join.and_then(|join| join.role) {
            Some(role) => self.by_slowest_join[role] += 1,
            None => self.no_slowest_join += 1,
        }
        if cost.force_full || cost.first_since_attach {
            self.force_full += 1;
        }
        if cost.allocator_wait_ns >= BUDGET_NS / 2 {
            self.allocator_bound += 1;
        }
    }

    fn status(&self) -> impl serde::Serialize + '_ {
        status!({
            "budget_ns": BUDGET_NS,
            "hours": self.hours,
            "with_buffer_wait": self.with_buffer_wait,
            "acquisition": self.acquisition,
            "acquisition_max_ns": self.acquisition_max_ns,
            "by_free_frames": self.by_free_frames.as_slice(),
            "by_daily_slot": self.by_daily_slot.as_slice(),
            "by_dominant_segment": self.by_dominant_segment.as_slice(),
            "setup_over_parallel": self.setup_over_parallel,
            "by_slowest_join": crate::status::map(self.by_slowest_join.iter().enumerate().filter(|(_, count)| **count != 0).map(|(role, count)| (DebugValue(acquisition_jobs::ROLES[role]), *count))),
            "no_slowest_join": self.no_slowest_join,
            "force_full": self.force_full,
            "allocator_bound": self.allocator_bound,
            "scope": "hours whose callback_ns exceeds budget_ns; by_free_frames counts every one by the frames still free once it took its own; acquisition counts those still over without their buffer wait, and the daily-slot, segment, setup, join, forced-full and allocator-bound (allocator_wait_ns at least half the budget) splits count only those",
        })
    }
}

#[derive(Clone, Copy, Default)]
struct Hour {
    hour: i32,
    qpc: i64,
    callback_ns: u64,
    cost: HourCost,
}

impl Hour {
    fn status(&self, frequency: u64) -> impl serde::Serialize + '_ {
        let cost = &self.cost;
        #[cfg(feature = "callback_profile")]
        let timeline = cost.join_timeline.map(|span| span.status(frequency));
        #[cfg(not(feature = "callback_profile"))]
        let timeline = None::<u64>;
        status!({
            "hour": self.hour,
            "qpc": self.qpc,
            "daily_slot": self.hour.rem_euclid(replay_data::cadence::STAGGER_PERIOD),
            "callback_ns": self.callback_ns,
            "entry_ns": cost.entry_ns,
            "buffer_wait_ns": cost.buffer_wait_ns,
            "free_frames": cost.free_frames,
            "allocator_wait_ns": cost.allocator_wait_ns,
            "setup_ns": cost.setup_ns,
            "parallel_ns": cost.parallel_ns,
            "residual_ns": cost.residual_ns,
            "segments_ns": cost.segments_ns.as_slice(),
            "slowest_join": cost.slowest_join.map(|join| status!({
                "role": join.role.map(|role| DebugValue(acquisition_jobs::ROLES[role])),
                "kind": if join.readiness { "readiness" } else { "join" },
                "wait_ns": join.wait_ns,
                "collect_stall_ns": (!join.readiness).then_some(join.collect_stall_ns),
            })),
            "collect_stalls_over_1ms": cost.collect_stalls_over_1ms,
            "growths": cost.growths,
            "diagnostics": cost.diagnostics.status(frequency),
            "idle": cost.idle.status(frequency),
            "variants": cost.variants,
            "join_timeline": timeline,
            "dominant_segment": dominant_segment(cost),
            "first_since_attach": cost.first_since_attach,
            "continuous": cost.continuous,
            "force_full": cost.force_full,
            "recovery": cost.recovery,
            "refresh": cost.refresh,
        })
    }
}

pub(crate) struct Recorder {
    histogram: [u64; HISTOGRAM_FLOOR_MS.len()],
    slowest: [Hour; CAPACITY],
    len: usize,
    dominant_segment_counts: [u64; crate::sampling::SEGMENTS],
    no_dominant_segment: u64,
    over_budget: OverBudget,
}

/// The game process's priority class now. Below normal, other programs' ordinary threads outrank
/// every game and collector thread, so a busy machine stalls whole hours for seconds.
fn process_priority_class() -> &'static str {
    use windows_sys::Win32::System::Threading::{
        ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, GetCurrentProcess,
        GetPriorityClass, HIGH_PRIORITY_CLASS, IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS,
        REALTIME_PRIORITY_CLASS,
    };
    // SAFETY: the current-process pseudo-handle needs no close.
    match unsafe { GetPriorityClass(GetCurrentProcess()) } {
        IDLE_PRIORITY_CLASS => "idle",
        BELOW_NORMAL_PRIORITY_CLASS => "below_normal",
        NORMAL_PRIORITY_CLASS => "normal",
        ABOVE_NORMAL_PRIORITY_CLASS => "above_normal",
        HIGH_PRIORITY_CLASS => "high",
        REALTIME_PRIORITY_CLASS => "realtime",
        _ => "unknown",
    }
}

fn dominant_segment(cost: &HourCost) -> Option<usize> {
    cost.segments_ns
        .iter()
        .enumerate()
        .filter(|(_, ns)| **ns != 0)
        .max_by_key(|(index, ns)| (**ns, std::cmp::Reverse(*index)))
        .map(|(index, _)| index)
}

impl Default for Recorder {
    fn default() -> Self {
        Self {
            histogram: [0; HISTOGRAM_FLOOR_MS.len()],
            slowest: [Hour::default(); CAPACITY],
            len: 0,
            dominant_segment_counts: [0; crate::sampling::SEGMENTS],
            no_dominant_segment: 0,
            over_budget: OverBudget::default(),
        }
    }
}

impl Recorder {
    /// A successful recorded hour, with no allocation and stable order on ties.
    pub(crate) fn record(&mut self, hour: i32, qpc: i64, callback_ns: u64, cost: HourCost) {
        self.over_budget.record(hour, callback_ns, &cost);
        match dominant_segment(&cost) {
            Some(segment) => {
                self.dominant_segment_counts[segment] =
                    self.dominant_segment_counts[segment].saturating_add(1)
            }
            None => self.no_dominant_segment = self.no_dominant_segment.saturating_add(1),
        }
        let bucket =
            HISTOGRAM_FLOOR_MS.partition_point(|&floor| callback_ns >= floor * 1_000_000) - 1;
        self.histogram[bucket] = self.histogram[bucket].saturating_add(1);
        let index =
            self.slowest[..self.len].partition_point(|entry| entry.callback_ns >= callback_ns);
        if index == CAPACITY {
            return;
        }
        let len = (self.len + 1).min(CAPACITY);
        self.slowest.copy_within(index..len - 1, index + 1);
        self.slowest[index] = Hour {
            hour,
            qpc,
            callback_ns,
            cost,
        };
        self.len = len;
    }

    pub(crate) fn status(&self, qpc_frequency: u64) -> impl serde::Serialize + '_ {
        let mut retained = [0_u64; crate::sampling::SEGMENTS];
        let mut retained_without_segment = 0_u64;
        for hour in &self.slowest[..self.len] {
            match dominant_segment(&hour.cost) {
                Some(segment) => retained[segment] += 1,
                None => retained_without_segment += 1,
            }
        }
        status!({
            "qpc_frequency": qpc_frequency,
            "histogram_floor_ms": HISTOGRAM_FLOOR_MS.as_slice(),
            "histogram": self.histogram.as_slice(),
            "capacity": CAPACITY,
            "slowest": sequence(self.slowest[..self.len].iter().map(move |hour| hour.status(qpc_frequency))),
            "dominant_segment_counts": self.dominant_segment_counts.as_slice(),
            "no_dominant_segment": self.no_dominant_segment,
            "over_budget": self.over_budget.status(),
            "process_priority_class": process_priority_class(),
            "retained_dominant_segment_counts": sequence(retained.into_iter()),
            "retained_no_dominant_segment": retained_without_segment,
            "segments_scope": crate::sampling::SEGMENTS_SCOPE,
            "join_timeline_scope": "With collector/callback_profile, the exact job collected by the slowest ordinary join, even when absent from top_by_body, including Research. Raw QPC stamps share qpc_frequency. join_begin_qpc and join_end_qpc enclose finish_into; claimed_qpc separates queue residence from pre-body delay. Body end precedes publication and collection. Null means the feature is disabled or the slowest wait was readiness, not an individual join.",
            "idle_scope": "Status-only. Inputs handoff stamps bracket the last publisher lock, census merge, preceding FIFO store loan and successor admission; zero stamps are unreached stages. Governance and GatedProjects retain existing table timers, including their four largest, not an additive subdivision of all body work. Address-wait tracing requires collector/callback_profile; null is unavailable. The first matching signal is selected by readiness sequence or exact joined ticket after every publisher settles, never by body ranking. It is a candidate OS wake, not proof of a context switch; multiple signals and dropped_signals expose ambiguity. Wake return may follow collector return. A post-store release stamp still includes any publisher descheduling before that stamp. Use ETW context switches and ready-thread events for actual scheduler latency.",
            "diagnostics_scope": "Readiness spans count waits whose first poll was not ready, including callback help, park and scheduling delay. Outstanding tickets are a lock-free scan at wait entry, not a simultaneous snapshot; slots whose generation or phase changes during inspection are omitted and counted by outstanding_changed_slots. ending_completion is the most recently published ticket inspected within the successful readiness condition, first inspected on equal publication stamps, not an inferred OS wake cause. Probing is capped at 64 predicate polls and 512 completion metadata inspections per wait; ending_probe_exhausted means attribution was truncated, and ending_completion is null rather than partial. Otherwise null means none was inspected. Jobs count all collected executor requests, including worker-joined children, rejected requests and continuations; top_by_body keeps eight by inclusive body wall, ties by admission. jobs.research retains this hour's Research request independently of the body ranking. Nested parent body spans overlap children and must not be added. Null lifecycle durations mean the job did not reach that stage. callback_thread growths are the callback TLS delta; workers includes job bodies and detached publication continuations. Longest EarlyStep is inclusive callback wall, unlike cumulative early_steps_ns which subtracts waits; census merges are nested within handler wall. Dominant segment counts cover all recorded hours since attach; retained_dominant_segment_counts covers only the retained slowest hours. The first segment wins ties; all-zero segments have no cause.",
            "scope": "Same recorded-hour cohort as callback_work since attach. callback_ns is callback_work's sample and the ordering key; ties keep recording order. qpc is the body-entry raw session QPC; divide by qpc_frequency to align frame timestamps. entry_ns is game-thread wall before the body, outside callback_work. setup+parallel+residual partition callback_ns (nanosecond rounding applies). segments split parallel; see acquisition_work.block_phases.segments_scope. daily_slot names the staggered slot due that hour; see daily_slots. growths count checked heap growths on the game thread and this hour's workers, including detached publication continuations.",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slowest_sixty_four_keep_first_ties_and_count_every_dominant_segment() {
        let mut recorder = Recorder::default();
        for hour in 0..100 {
            let mut cost = HourCost::default();
            cost.segments_ns[hour as usize % crate::sampling::SEGMENTS] = 1;
            recorder.record(hour, hour as i64 + 1, (hour / 2) as u64, cost);
        }
        recorder.record(100, 101, 0, HourCost::default());
        let status = serde_json::to_value(recorder.status(10_000_000)).unwrap();
        assert_eq!(status["capacity"], 64);
        let slowest = status["slowest"].as_array().unwrap();
        assert_eq!(slowest.len(), 64);
        assert_eq!(slowest[0]["hour"], 98);
        assert_eq!(slowest[1]["hour"], 99);
        assert_eq!(slowest[63]["hour"], 37);
        assert_eq!(
            status["dominant_segment_counts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|count| count.as_u64().unwrap())
                .sum::<u64>(),
            100
        );
        assert_eq!(status["no_dominant_segment"], 1);
        assert_eq!(
            status["retained_dominant_segment_counts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|count| count.as_u64().unwrap())
                .sum::<u64>(),
            64
        );
        assert_eq!(status["retained_no_dominant_segment"], 0);
        let mut cost = HourCost::default();
        cost.segments_ns[1] = 7;
        cost.segments_ns[4] = 7;
        assert_eq!(dominant_segment(&cost), Some(1));
        assert_eq!(dominant_segment(&HourCost::default()), None);
    }

    #[test]
    fn bench_series_keeps_complete_callback_tail_and_nested_job_objects() {
        let script = include_str!("../../../tools/bench.ps1");
        let series = script
            .split("$statusSeries.WriteLine")
            .nth(1)
            .unwrap()
            .split("$statusSeries.Flush")
            .next()
            .unwrap();
        assert!(series.contains("callback_tail = $status.callback_tail"));
        assert!(series.contains("attach_callbacks = $status.attach_callbacks"));
        assert!(series.contains("variants_profile = $status.variant_changes.variants_profile"));
        assert!(series.contains("callback_profile = $status.callback_profile"));
        assert!(series.contains("ConvertTo-Json -Compress -Depth 16"));
    }
}
