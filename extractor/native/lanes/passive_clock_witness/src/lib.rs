use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClockSample {
    pub schema: String,
    pub observed_at_utc: String,
    pub observed_at_unix_ms: i64,
    pub phase: String,
    pub pid: u32,
    pub process_creation_filetime_utc: u64,
    pub exe_sha256: String,
    pub observer_sha256: String,
    pub access: String,
    pub expected_speed: i32,
    pub speed_initial: i32,
    pub speed_final: i32,
    pub paused_initial: bool,
    pub paused_final: bool,
    pub pause_forced_initial: bool,
    pub pause_forced_final: bool,
    pub tick_inhibited_initial: bool,
    pub tick_inhibited_final: bool,
    pub paused_in_window: bool,
    pub pause_forced_in_window: bool,
    pub tick_inhibited_in_window: bool,
    pub hour_a: i32,
    pub hour_b: i32,
    pub hour_delta: i32,
    pub tick_progress_bits_a: i32,
    pub tick_progress_bits_b: i32,
    pub window_started_at_utc: String,
    pub window_started_at_unix_ms: i64,
    pub observed_window_ms: i64,
    pub clock_progress_waited_ms: i64,
    pub clock_progress_budget_ms: i64,
    pub processor_milliseconds: i64,
    pub advancing: bool,
}

pub const WITNESS_SCHEMA_V2: &str = "hoi4-passive-client-state-witness-v2";
pub const INTERVALS_SCHEMA_V2: &str = "hoi4-passive-clock-intervals-v2";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClockState {
    Advancing,
    AdvancingWithHold,
    PausedByEngine,
    PausedUnforced,
    Stalled,
}

impl ClockState {
    pub const fn is_engine_hold(self) -> bool {
        matches!(
            self,
            ClockState::PausedByEngine | ClockState::PausedUnforced
        )
    }

    pub const fn is_paused(self) -> bool {
        matches!(
            self,
            ClockState::PausedByEngine | ClockState::PausedUnforced
        )
    }

    pub const fn engine_executed(self) -> bool {
        matches!(self, ClockState::Advancing | ClockState::AdvancingWithHold)
    }
}

pub fn classify(sample: &ClockSample) -> ClockState {
    let held = sample.paused_in_window;
    if sample.advancing {
        return if held {
            ClockState::AdvancingWithHold
        } else {
            ClockState::Advancing
        };
    }
    if sample.paused_in_window {
        if sample.pause_forced_in_window {
            return ClockState::PausedByEngine;
        }
        return ClockState::PausedUnforced;
    }
    ClockState::Stalled
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WitnessFailure {
    ClientWriteDrift {
        phase: String,
        expected_speed: i32,
        speed_initial: i32,
        speed_final: i32,
    },
    EngineUnresponsive {
        phase: String,
        consecutive_samples: usize,
        span_ms: i64,
        processor_ms_delta: i64,
        burning_cpu: bool,
    },
    UnusableSample {
        reason: String,
    },
}

impl std::fmt::Display for WitnessFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WitnessFailure::ClientWriteDrift {
                phase,
                expected_speed,
                speed_initial,
                speed_final,
            } => write!(
                f,
                "client write drift in phase '{phase}': game speed left the pre-attach value \
                 (expected={expected_speed} initial={speed_initial} final={speed_final}). \
                 The passive capture policy is zero client writes from the graded boundary \
                 through authenticated teardown, and the engine's pause path never writes \
                 the speed selector, so this is a write and not a pause."
            ),
            WitnessFailure::EngineUnresponsive {
                phase,
                consecutive_samples,
                span_ms,
                processor_ms_delta,
                burning_cpu,
            } => write!(
                f,
                "engine unresponsive in phase '{phase}': {consecutive_samples} consecutive \
                 observation windows over {span_ms} ms ran the pacing block zero times while \
                 the engine reported no pause \
                 (processor_ms_delta={processor_ms_delta}, burning_cpu={burning_cpu}). \
                 This is a hang, not client drift and not a pause."
            ),
            WitnessFailure::UnusableSample { reason } => {
                write!(f, "unusable passive witness sample: {reason}")
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StallPolicy {
    pub min_consecutive_samples: usize,
    pub min_span_ms: i64,
}

impl Default for StallPolicy {
    fn default() -> Self {
        Self {
            min_consecutive_samples: 3,
            min_span_ms: 12_000,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ClockInterval {
    pub state: ClockState,
    pub from_utc: String,
    pub to_utc: String,
    pub from_unix_ms: i64,
    pub to_unix_ms: i64,
    pub measured_ms: i64,
    pub span_ms: i64,
    pub samples: usize,
    pub hour_from: i32,
    pub hour_to: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ClockIntervalReport {
    pub schema: String,
    pub samples: usize,
    pub intervals: Vec<ClockInterval>,
    pub advancing_ms: i64,
    pub advancing_with_hold_ms: i64,
    pub paused_by_engine_ms: i64,
    pub paused_unforced_ms: i64,
    pub stalled_ms: i64,
    pub unobserved_ms: i64,
    pub wall_clock_ms: i64,
    pub observed_game_hours: i64,
}

#[derive(Clone, Debug, Default)]
pub struct WitnessRun {
    policy: StallPolicy,
    samples: Vec<(ClockSample, ClockState)>,
}

impl WitnessRun {
    pub fn new(policy: StallPolicy) -> Self {
        Self {
            policy,
            samples: Vec::new(),
        }
    }

    pub fn states(&self) -> impl Iterator<Item = ClockState> + '_ {
        self.samples.iter().map(|(_, state)| *state)
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn admit(&mut self, sample: ClockSample) -> Result<ClockState, WitnessFailure> {
        if sample.schema != WITNESS_SCHEMA_V2 {
            return Err(WitnessFailure::UnusableSample {
                reason: format!("schema '{}' is not '{WITNESS_SCHEMA_V2}'", sample.schema),
            });
        }
        if sample.observed_window_ms < 0 || sample.clock_progress_budget_ms <= 0 {
            return Err(WitnessFailure::UnusableSample {
                reason: format!(
                    "observation window {} ms against budget {} ms is not a measurement",
                    sample.observed_window_ms, sample.clock_progress_budget_ms
                ),
            });
        }
        let bracket_span_ms = sample.observed_at_unix_ms - sample.window_started_at_unix_ms;
        if bracket_span_ms < sample.observed_window_ms {
            return Err(WitnessFailure::UnusableSample {
                reason: format!(
                    "window bracket {}..{} spans {bracket_span_ms} ms and cannot contain the \
                     measured {} ms",
                    sample.window_started_at_unix_ms,
                    sample.observed_at_unix_ms,
                    sample.observed_window_ms
                ),
            });
        }
        if sample.clock_progress_waited_ms < 0
            || sample.clock_progress_waited_ms > sample.observed_window_ms
        {
            return Err(WitnessFailure::UnusableSample {
                reason: format!(
                    "clock-progress wait of {} ms does not fit inside the measured {} ms window",
                    sample.clock_progress_waited_ms, sample.observed_window_ms
                ),
            });
        }
        let recomputed_delta = i64::from(sample.hour_b) - i64::from(sample.hour_a);
        if recomputed_delta != i64::from(sample.hour_delta) {
            return Err(WitnessFailure::UnusableSample {
                reason: format!(
                    "sample claims hour_delta={} but its raw hours say {recomputed_delta} \n                     ({} -> {})",
                    sample.hour_delta, sample.hour_a, sample.hour_b
                ),
            });
        }
        let recomputed = sample.hour_b != sample.hour_a
            || sample.tick_progress_bits_b != sample.tick_progress_bits_a;
        if recomputed != sample.advancing {
            return Err(WitnessFailure::UnusableSample {
                reason: format!(
                    "sample claims advancing={} but its raw fields say {recomputed} \
                     (hour {}->{}, tick progress bits {}->{})",
                    sample.advancing,
                    sample.hour_a,
                    sample.hour_b,
                    sample.tick_progress_bits_a,
                    sample.tick_progress_bits_b
                ),
            });
        }
        if sample.speed_initial != sample.expected_speed
            || sample.speed_final != sample.expected_speed
        {
            return Err(WitnessFailure::ClientWriteDrift {
                phase: sample.phase.clone(),
                expected_speed: sample.expected_speed,
                speed_initial: sample.speed_initial,
                speed_final: sample.speed_final,
            });
        }
        let state = classify(&sample);
        self.samples.push((sample, state));
        self.check_stall_escalation()?;
        Ok(state)
    }

    fn check_stall_escalation(&self) -> Result<(), WitnessFailure> {
        let mut tail = Vec::new();
        for (sample, state) in self.samples.iter().rev() {
            if *state != ClockState::Stalled {
                break;
            }
            tail.push(sample);
        }
        if tail.len() < self.policy.min_consecutive_samples {
            return Ok(());
        }
        let newest = tail[0];
        let oldest = tail[tail.len() - 1];
        let span_ms = newest.observed_at_unix_ms - oldest.observed_at_unix_ms;
        if span_ms < self.policy.min_span_ms {
            return Ok(());
        }
        let processor_ms_delta = newest.processor_milliseconds - oldest.processor_milliseconds;
        let burning_cpu = span_ms > 0 && processor_ms_delta * 2 > span_ms;
        Err(WitnessFailure::EngineUnresponsive {
            phase: newest.phase.clone(),
            consecutive_samples: tail.len(),
            span_ms,
            processor_ms_delta,
            burning_cpu,
        })
    }

    pub fn interval_report(&self) -> ClockIntervalReport {
        let mut intervals: Vec<ClockInterval> = Vec::new();
        for (sample, state) in &self.samples {
            match intervals.last_mut() {
                Some(open) if open.state == *state => {
                    open.to_utc = sample.observed_at_utc.clone();
                    open.to_unix_ms = sample.observed_at_unix_ms;
                    open.span_ms = open.to_unix_ms - open.from_unix_ms;
                    open.measured_ms += sample.observed_window_ms;
                    open.samples += 1;
                    open.hour_to = sample.hour_b;
                }
                _ => intervals.push(ClockInterval {
                    state: *state,
                    from_utc: sample.window_started_at_utc.clone(),
                    to_utc: sample.observed_at_utc.clone(),
                    from_unix_ms: sample.window_started_at_unix_ms,
                    to_unix_ms: sample.observed_at_unix_ms,
                    measured_ms: sample.observed_window_ms,
                    span_ms: sample.observed_at_unix_ms - sample.window_started_at_unix_ms,
                    samples: 1,
                    hour_from: sample.hour_a,
                    hour_to: sample.hour_b,
                }),
            }
        }
        let total = |want: ClockState| -> i64 {
            intervals
                .iter()
                .filter(|interval| interval.state == want)
                .map(|interval| interval.measured_ms)
                .sum()
        };
        let measured_ms: i64 = intervals.iter().map(|interval| interval.measured_ms).sum();
        let wall_clock_ms = match (self.samples.first(), self.samples.last()) {
            (Some((first, _)), Some((last, _))) => {
                last.observed_at_unix_ms - first.window_started_at_unix_ms
            }
            _ => 0,
        };
        let observed_game_hours = self
            .samples
            .iter()
            .map(|(sample, _)| i64::from(sample.hour_delta.max(0)))
            .sum();
        ClockIntervalReport {
            schema: INTERVALS_SCHEMA_V2.to_string(),
            samples: self.samples.len(),
            advancing_ms: total(ClockState::Advancing),
            advancing_with_hold_ms: total(ClockState::AdvancingWithHold),
            paused_by_engine_ms: total(ClockState::PausedByEngine),
            paused_unforced_ms: total(ClockState::PausedUnforced),
            stalled_ms: total(ClockState::Stalled),
            unobserved_ms: (wall_clock_ms - measured_ms).max(0),
            wall_clock_ms,
            observed_game_hours,
            intervals,
        }
    }
}

#[cfg(test)]
mod script_order_tests;
#[cfg(test)]
mod tests;
