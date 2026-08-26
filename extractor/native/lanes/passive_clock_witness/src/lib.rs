use serde::{Deserialize, Deserializer, Serialize};

/// Deserialize an `Option<T>` that is NULLABLE but not OMITTABLE.
///
/// Plain `Option<T>` is not that. serde's derive treats a missing field on an
/// `Option` as `None` all by itself -- `deny_unknown_fields` does not change
/// it, and neither does the absence of `#[serde(default)]`. An earlier
/// revision of this lane asserted the opposite in a doc comment AND in a test,
/// and both were wrong (Codex round 3 finding 1). The behaviour is recorded
/// empirically in `runs/tj_tg_followups/r3_serde_option_probe.out.txt`.
///
/// Naming a `deserialize_with` suppresses that implicit default, so a missing
/// key is `missing field` and an explicit `null` is `None`. That is the
/// contract the host fields need: "the producer never wrote this" and "the
/// producer could not read it" are different facts, and only the second is
/// admissible.
fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}

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
    /// The OBSERVER PROCESS's available parallelism, from its own
    /// environment.
    ///
    /// Described twice before this wording, wrongly both times.
    ///
    ///  * Not the affinity-MASKED width. `[Environment]::ProcessorCount`
    ///    honours the mask of the process that ASKS, and the asker is the
    ///    observer, which the harness leaves unmasked on purpose -- it names
    ///    "bounded control/observer helpers" in its own
    ///    `unmasked_participants` list. So under `-CpuAffinityCores` this does
    ///    not narrow (Codex round 1 finding 2).
    ///  * Not the MACHINE's logical processor count either. That holds only on
    ///    a single-group machine: on a processor-group machine a Windows
    ///    PowerShell process is not a group-aware inventory of the box (Codex
    ///    round 3 finding 6, still unfixed here in round 4).
    ///
    /// What it is for: it is the denominator
    /// [`Self::target_processor_affinity_cores`] is read against, and a change
    /// in it BETWEEN two runs on the same machine is a real signal about the
    /// host.
    ///
    /// Recorded and never gated on -- this struct's verdicts are about the
    /// engine, and a narrow host is a slow run, not a failed one. It comes
    /// from the observer's own environment and has no failure mode, which is
    /// why it is the one host field that is not optional.
    pub available_parallelism: i32,
    /// Population count of the TARGET's processor affinity mask, in its
    /// PRIMARY processor group: how many logical processors the game was
    /// allowed to use.
    ///
    /// With no mask applied the mask is all ones and, on a single-group
    /// machine, this equals `available_parallelism` -- so the pair reads "n of
    /// m cores" on every row and a masked run is visible without a second
    /// receipt. That matters because `-CpuAffinityCores 0` writes no
    /// `HARNESS_CPU_AFFINITY_V1.json` at all, so the file's ABSENCE was the
    /// only evidence a run was unmasked.
    ///
    /// PRIMARY GROUP ONLY, so the comparison above is machine-wide only on a
    /// single-group machine. `Process.ProcessorAffinity` is a bare `KAFFINITY`
    /// carrying no processor group, and the harness refuses to mask a
    /// multi-group machine for exactly that reason (Codex round 2 finding 6).
    ///
    /// `None` when the query failed, with the cause in
    /// [`Self::host_state_unavailable`]. Never zero: a zero here would read as
    /// "no cores", which is not a thing that can be observed.
    #[serde(deserialize_with = "required_option")]
    pub target_processor_affinity_cores: Option<i32>,
    /// The target's `PROCESS_MEMORY_COUNTERS.PageFaultCount` at the close of
    /// the observation window.
    ///
    /// Cumulative since process start, so it is a DIFFERENCE between samples
    /// that means something, not a level. Widened to `i64` at the schema
    /// boundary: the field is a `DWORD`, and a signed 64-bit integer holds it
    /// without inventing an unsigned type for JSON consumers to disagree
    /// about.
    ///
    /// `None` when the query failed -- see [`Self::host_state_unavailable`].
    #[serde(deserialize_with = "required_option")]
    pub target_page_fault_count: Option<i64>,
    /// The target's `PROCESS_MEMORY_COUNTERS.WorkingSetSize` in bytes at the
    /// close of the observation window. A level, not a difference.
    ///
    /// `None` when the query failed -- see [`Self::host_state_unavailable`].
    #[serde(deserialize_with = "required_option")]
    pub target_working_set_bytes: Option<i64>,
    /// `None` when every host query succeeded; otherwise the named causes,
    /// joined with `"; "`.
    ///
    /// WHY THE HOST FIELDS ARE OPTIONAL AT ALL. An earlier revision of this
    /// lane made the observer THROW when the memory-counter query failed,
    /// which would have aborted a graded capture over a number nothing gates
    /// on -- against the project's rule that partial data still seals (Codex
    /// round 2 finding 3). The engine sample above is authoritative and its
    /// failure is a real failure; these are diagnostics, and an unavailable
    /// diagnostic must not destroy the sample that was already taken.
    ///
    /// The pairing is the contract: a `None` host field is only meaningful
    /// beside a named cause here, and a row with nulls and no cause is one
    /// nobody can act on. `clock_baseline_summary.py` refuses exactly that
    /// shape.
    ///
    /// The host fields are nullable but NOT omittable, and that takes
    /// [`required_option`] to say -- plain `Option<T>` accepts a missing key
    /// as `None`. See that function.
    ///
    /// Presence is only half of it: [`WitnessRun::admit`] also refuses a row
    /// whose host fields are `None` while this names no cause, so the pairing
    /// is enforced and not merely described (Codex round 3 finding 2). That is
    /// the same rule `clock_baseline_summary.py`'s `admission_problem`
    /// applies, and the two readers agreeing is the point.
    #[serde(deserialize_with = "required_option")]
    pub host_state_unavailable: Option<String>,
    pub advancing: bool,
}

/// The witness row's schema.
///
/// v3 adds the four host-and-pacing fields -- `available_parallelism`,
/// `target_processor_affinity_cores`, `target_page_fault_count`,
/// `target_working_set_bytes` -- and `host_state_unavailable`, which names the
/// cause when one of them is null. `ClockSample` is
/// `deny_unknown_fields`, so a v2 row and a v3 reader are mutually
/// unparseable in both directions and the version had to move with them.
/// There is no v2 compatibility path and there is deliberately none: old
/// captures are disposable before production, and a reader that silently
/// accepted a row with no host state would reintroduce exactly the blind spot
/// this version exists to close.
pub const WITNESS_SCHEMA_V3: &str = "hoi4-passive-client-state-witness-v3";

/// WHICH CAUSE EXPLAINS WHICH HOST FIELD.
///
/// The observer contains each host query separately and prefixes the reason it
/// appends with that query's own name, so `host_state_unavailable` is not one
/// opaque string -- it says which call failed. An earlier revision of this
/// reader accepted ANY non-blank cause as cover for ANY null, so a row whose
/// affinity was missing and whose only cause was
/// `"K32GetProcessMemoryInfo: Win32 error 5"` was admitted (Codex round 7
/// finding 3). The confound witness this field exists to build is only useful
/// if a gap in it is named, and a cause for a different query does not name
/// this one.
///
/// The same table lives in `clock_baseline_summary.py`'s
/// `HOST_STATE_CAUSE_PREFIX`, and the two readers applying the same rule is
/// the point of having it in both.
const HOST_STATE_CAUSE_PREFIXES: [(&str, &str); 3] = [
    ("target_processor_affinity_cores", "ProcessorAffinity:"),
    ("target_page_fault_count", "K32GetProcessMemoryInfo:"),
    ("target_working_set_bytes", "K32GetProcessMemoryInfo:"),
];
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
        if sample.schema != WITNESS_SCHEMA_V3 {
            return Err(WitnessFailure::UnusableSample {
                reason: format!("schema '{}' is not '{WITNESS_SCHEMA_V3}'", sample.schema),
            });
        }
        // THE NULL/CAUSE PAIRING, enforced rather than described.
        //
        // A host field may be `None` -- the observer writes that when a query
        // fails, and an unavailable diagnostic must not abort a capture. But
        // `None` with no named cause is unaccounted partial data: the row says
        // neither what the host was doing nor why it could not say. Refusing
        // it here is what makes this reader agree with
        // `clock_baseline_summary.py`, which refuses the same shape; an
        // earlier revision left this to the Python side alone and the two
        // disagreed (Codex round 3 finding 2).
        //
        // Note this ALSO catches a host key the producer omitted entirely on
        // any path where `required_option` did not already reject it, so the
        // presence contract and the pairing contract close over each other.
        let cause = sample.host_state_unavailable.as_deref();
        if cause.is_some_and(|reason| reason.trim().is_empty()) {
            return Err(WitnessFailure::UnusableSample {
                reason: format!(
                    "phase '{}' carries a blank host_state_unavailable; the \
                     observer writes null when nothing failed and a named \
                     cause when something did, and neither is whitespace.",
                    sample.phase
                ),
            });
        }
        let missing: [(&str, bool); 3] = [
            (
                "target_processor_affinity_cores",
                sample.target_processor_affinity_cores.is_none(),
            ),
            (
                "target_page_fault_count",
                sample.target_page_fault_count.is_none(),
            ),
            (
                "target_working_set_bytes",
                sample.target_working_set_bytes.is_none(),
            ),
        ];
        for (field, is_missing) in missing {
            if !is_missing {
                continue;
            }
            let prefix = HOST_STATE_CAUSE_PREFIXES
                .iter()
                .find(|(name, _)| *name == field)
                .map(|(_, prefix)| *prefix)
                .unwrap_or_default();
            match cause {
                None => {
                    return Err(WitnessFailure::UnusableSample {
                        reason: format!(
                            "phase '{}' has a null '{field}' and names no \
                             cause. A host query that failed must say so; a \
                             null that says nothing is unaccounted partial \
                             data.",
                            sample.phase
                        ),
                    });
                }
                Some(reason) if !reason.contains(prefix) => {
                    return Err(WitnessFailure::UnusableSample {
                        reason: format!(
                            "phase '{}' has a null '{field}' but \
                             host_state_unavailable is {reason:?}, which does \
                             not mention '{prefix}'. The observer names the \
                             query that failed, and a cause for a different \
                             query does not account for this one.",
                            sample.phase
                        ),
                    });
                }
                Some(_) => {}
            }
        }
        if cause.is_some() && missing.iter().all(|(_, is_missing)| !is_missing) {
            return Err(WitnessFailure::UnusableSample {
                reason: format!(
                    "phase '{}' names a host cause ({:?}) while every host \
                     counter has a value; the observer writes a cause only \
                     for a query that failed, so a row with both is not one \
                     it wrote.",
                    sample.phase, sample.host_state_unavailable
                ),
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
