//! Per-family preparation timings: observe and flush time and call counts,
//! each family's own time net of the families it drives.

use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub struct FamilyTimings {
    pub observe: Duration,
    pub flush: Duration,
    pub calls: u64,
}

pub fn family_time(families: &BTreeMap<&'static str, FamilyTimings>) -> Duration {
    families
        .values()
        .map(|family| family.observe + family.flush)
        .sum()
}

pub fn start_timing(
    families: &Option<BTreeMap<&'static str, FamilyTimings>>,
) -> Option<(Instant, Duration)> {
    families
        .as_ref()
        .map(|families| (Instant::now(), family_time(families)))
}

pub fn finish_timing(
    families: &mut Option<BTreeMap<&'static str, FamilyTimings>>,
    started: Option<(Instant, Duration)>,
    name: &'static str,
    flush: bool,
) {
    if let (Some(families), Some((started, prior))) = (families.as_mut(), started) {
        let elapsed = started
            .elapsed()
            .saturating_sub(family_time(families).saturating_sub(prior));
        let family = families.entry(name).or_default();
        if flush {
            family.flush += elapsed;
        } else {
            family.observe += elapsed;
        }
        family.calls += 1;
    }
}
