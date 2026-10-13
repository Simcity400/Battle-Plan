//! Bounded callback tails over the same recorded hours as callback_work.

use crate::{
    acquisition_jobs,
    sampling::HourCost,
    status::{DebugValue, sequence, status},
};

const CAPACITY: usize = 16;
const HISTOGRAM_FLOOR_MS: [u64; 9] = [0, 2, 4, 8, 16, 32, 64, 128, 256];

#[derive(Clone, Copy, Default)]
struct Hour {
    hour: i32,
    qpc: i64,
    callback_ns: u64,
    cost: HourCost,
}

impl Hour {
    fn status(&self) -> impl serde::Serialize + '_ {
        let cost = &self.cost;
        status!({
            "hour": self.hour,
            "qpc": self.qpc,
            "daily_slot": self.hour.rem_euclid(replay::cadence::STAGGER_PERIOD),
            "callback_ns": self.callback_ns,
            "entry_ns": cost.entry_ns,
            "buffer_wait_ns": cost.buffer_wait_ns,
            "setup_ns": cost.setup_ns,
            "parallel_ns": cost.parallel_ns,
            "residual_ns": cost.residual_ns,
            "segments_ns": cost.segments_ns.as_slice(),
            "slowest_join": cost.slowest_join.map(|join| status!({
                "role": DebugValue(acquisition_jobs::ROLES[join.role]),
                "wait_ns": join.wait_ns,
                "collect_stall_ns": join.collect_stall_ns,
            })),
            "collect_stalls_over_1ms": cost.collect_stalls_over_1ms,
            "growths": cost.growths,
            "first_since_attach": cost.first_since_attach,
            "continuous": cost.continuous,
            "force_full": cost.force_full,
            "recovery": cost.recovery,
            "refresh": cost.refresh,
        })
    }
}

#[derive(Default)]
pub(crate) struct Recorder {
    histogram: [u64; HISTOGRAM_FLOOR_MS.len()],
    slowest: [Hour; CAPACITY],
    len: usize,
}

impl Recorder {
    /// A successful recorded hour, with no allocation and stable order on ties.
    pub(crate) fn record(&mut self, hour: i32, qpc: i64, callback_ns: u64, cost: HourCost) {
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
        status!({
            "qpc_frequency": qpc_frequency,
            "histogram_floor_ms": HISTOGRAM_FLOOR_MS.as_slice(),
            "histogram": self.histogram.as_slice(),
            "capacity": CAPACITY,
            "slowest": sequence(self.slowest[..self.len].iter().map(Hour::status)),
            "scope": "Same recorded-hour cohort as callback_work since attach. callback_ns is callback_work's sample and the ordering key; ties keep recording order. qpc is the body-entry raw session QPC; divide by qpc_frequency to align frame timestamps. entry_ns is game-thread wall before the body, outside callback_work. setup+parallel+residual partition callback_ns (nanosecond rounding applies). segments split parallel; see acquisition_work.block_phases.segments_scope. daily_slot names the staggered slot due that hour; see daily_slots. growths count checked heap growths on the game thread and in that hour's joined jobs.",
        })
    }
}
