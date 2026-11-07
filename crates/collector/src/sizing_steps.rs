//! The sizing pass's CPU by step, cumulative since attach; status only.

use crate::status::{sequence, status};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy)]
pub(crate) enum Step {
    Logistics,
    Observers,
    FuelDemand,
    FuelUnits,
    Peace,
    NavalResults,
    Lines,
    Weather,
    Registry,
    Construction,
    Losses,
    SourceDemand,
    TableDemand,
    Destruction,
    NavalManagement,
    Intel,
    Names,
    Map,
    Presentation,
    GameScore,
    Raids,
    Characters,
    SavedTargets,
    Politics,
    Visibility,
    IdleGrowth,
    CombatHints,
}

const NAMES: [&str; 27] = [
    "logistics",
    "observers",
    "fuel_demand",
    "fuel_units",
    "peace",
    "naval_results",
    "lines",
    "weather",
    "registry",
    "construction",
    "losses",
    "source_demand",
    "table_demand",
    "destruction",
    "naval_management",
    "intel",
    "names",
    "map",
    "presentation",
    "game_score",
    "raids",
    "characters",
    "saved_targets",
    "politics",
    "visibility",
    "idle_growth",
    "combat_hints",
];

static TOTALS: [[AtomicU64; 2]; NAMES.len()] =
    [const { [const { AtomicU64::new(0) }; 2] }; NAMES.len()];

/// Charges the calling thread's cycles since the previous mark to each step.
pub(crate) struct Steps(u64);
impl Steps {
    pub(crate) fn begin() -> Self {
        Self(crate::spool_phases::thread_cycles())
    }
    pub(crate) fn mark(&mut self, step: Step) {
        let now = crate::spool_phases::thread_cycles();
        let totals = &TOTALS[step as usize];
        totals[0].fetch_add(1, Ordering::Relaxed);
        totals[1].fetch_add(now.saturating_sub(self.0), Ordering::Relaxed);
        self.0 = now;
    }
}

pub(crate) fn status() -> impl serde::Serialize {
    let hz = crate::hook_timing::tsc_hz();
    status!({
        "scope": "cumulative sizing-pass CPU by step since attach, from QueryThreadCycleTime on the thread running the pass; each step is charged the cycles since the previous step, including any tick-gate wait spent on-CPU",
        "steps": sequence(NAMES.iter().zip(&TOTALS).map(move |(name, totals)| {
            let cycles = totals[1].load(Ordering::Relaxed);
            status!({
                "name": name,
                "calls": totals[0].load(Ordering::Relaxed),
                "cycle_ns": crate::hook_timing::ticks_to_ns(cycles, hz),
            })
        })),
        "table_demand": sequence(crate::country_scale::table_demand::costs().map(move |(name, calls, cycles)| {
            status!({
                "name": name,
                "calls": calls,
                "cycle_ns": crate::hook_timing::ticks_to_ns(cycles, hz),
            })
        })),
    })
}
