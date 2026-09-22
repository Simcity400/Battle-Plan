//! Bounded discovery traces for fresh raw acquisition with exact read occurrences.

use engine::read_plan::{Control, Hint, PreparedPlan, Read};
use std::sync::atomic::{AtomicU64, Ordering};

static ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static COMPLETE: AtomicU64 = AtomicU64::new(0);
static CHANGED: AtomicU64 = AtomicU64::new(0);
static FAULTED: AtomicU64 = AtomicU64::new(0);
static REBUILDS: AtomicU64 = AtomicU64::new(0);
static PREFIX_MISMATCHES: AtomicU64 = AtomicU64::new(0);
static INELIGIBLE: AtomicU64 = AtomicU64::new(0);
static EXECUTIONS: AtomicU64 = AtomicU64::new(0);
static PLANNED_READS: AtomicU64 = AtomicU64::new(0);
static CANONICAL_READS: AtomicU64 = AtomicU64::new(0);
static CONTROL_FRONTIERS: AtomicU64 = AtomicU64::new(0);
static PREPARE_NS: AtomicU64 = AtomicU64::new(0);
static BACKEND_NS: AtomicU64 = AtomicU64::new(0);
static OVERFLOW_NO_RETRY: AtomicU64 = AtomicU64::new(0);
static REDUNDANT_CONTROLS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn diagnostics() -> serde_json::Value {
    serde_json::json!({
        "scope": "opted-in raw acquisition since attach; every plan executes fresh source reads",
        "attempts": ATTEMPTS.load(Ordering::Relaxed),
        "complete": COMPLETE.load(Ordering::Relaxed),
        "control_changed": CHANGED.load(Ordering::Relaxed),
        "read_failed": FAULTED.load(Ordering::Relaxed),
        "rebuilds": REBUILDS.load(Ordering::Relaxed),
        "prefix_mismatches": PREFIX_MISMATCHES.load(Ordering::Relaxed),
        "overflow_after_planned_source_no_retry": OVERFLOW_NO_RETRY.load(Ordering::Relaxed),
        "ineligible_traces": INELIGIBLE.load(Ordering::Relaxed),
        "supported_executions": EXECUTIONS.load(Ordering::Relaxed),
        "planned_read_occurrences": PLANNED_READS.load(Ordering::Relaxed),
        "canonical_source_calls": CANONICAL_READS.load(Ordering::Relaxed),
        "control_frontiers_reached": CONTROL_FRONTIERS.load(Ordering::Relaxed),
        "redundant_controls_removed": REDUNDANT_CONTROLS.load(Ordering::Relaxed),
        "worker_prepare_ns": PREPARE_NS.load(Ordering::Relaxed),
        "backend_execution_ns": BACKEND_NS.load(Ordering::Relaxed),
        "backend_scope": "execute_read_plan calls only; excludes canonical continuation and raw metadata restoration",
        "prepare_scope": "compilation, retirement and trace-capacity growth on recycle; outside family decoding",
    })
}

#[derive(Debug)]
pub(crate) struct Observations {
    pub active: bool,
    pub eligible: bool,
    pub after_reads: usize,
    pub controls: Vec<Control>,
    pub expected: Vec<u8>,
    pub hints: Vec<Hint>,
    required_controls: usize,
    required_bytes: usize,
}

impl Observations {
    pub fn new(controls: usize, bytes: usize) -> Self {
        Self {
            active: false,
            eligible: false,
            after_reads: 0,
            controls: Vec::with_capacity(controls),
            expected: Vec::with_capacity(bytes),
            hints: Vec::with_capacity(controls),
            required_controls: 0,
            required_bytes: 0,
        }
    }

    pub fn begin(&mut self) {
        self.controls.clear();
        self.expected.clear();
        self.hints.clear();
        self.after_reads = 0;
        self.required_controls = 0;
        self.required_bytes = 0;
        self.active = true;
        self.eligible = true;
    }

    pub fn observe(&mut self, offset: usize, bytes: &[u8]) {
        if !self.active || bytes.is_empty() {
            return;
        }
        self.required_controls = self.required_controls.saturating_add(1);
        self.required_bytes = self.required_bytes.saturating_add(bytes.len());
        if !self.eligible {
            return;
        }
        if self.controls.len() == self.controls.capacity()
            || bytes.len() > self.expected.capacity() - self.expected.len()
        {
            self.eligible = false;
            return;
        }
        self.controls.push(Control {
            after_reads: self.after_reads,
            offset,
            len: bytes.len(),
            expected_offset: self.expected.len(),
        });
        self.expected.extend_from_slice(bytes);
    }

    pub fn escape(&mut self) {
        if self.active {
            self.eligible = false;
        }
    }

    pub fn hint(&mut self, address: usize, len: usize) {
        if self.active
            && self.hints.len() < self.hints.capacity()
            && len != 0
            && address != 0
            && crate::raw::is_canonical_range(address, len)
        {
            self.hints.push(Hint {
                after_reads: self.after_reads,
                address,
                len,
            });
        }
    }

    pub fn capacity_bytes(&self) -> u64 {
        crate::raw_memory::vector_bytes(&self.controls)
            + crate::raw_memory::vector_bytes(&self.expected)
            + crate::raw_memory::vector_bytes(&self.hints)
    }

    pub fn grow_for_next(&mut self) {
        if self.required_controls > self.controls.capacity() {
            self.controls
                .reserve_exact(self.required_controls - self.controls.len());
        }
        if self.required_bytes > self.expected.capacity() {
            self.expected
                .reserve_exact(self.required_bytes - self.expected.len());
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Request {
    pub address: usize,
    pub len: usize,
    pub start: usize,
    pub mask: u32,
    pub bulk: bool,
}

impl Request {
    pub fn matches(self, address: usize, len: usize, mask: u32, bulk: bool, start: usize) -> bool {
        self.address == address
            && self.len == len
            && self.mask == mask
            && self.bulk == bulk
            && self.start == start
    }
}

#[derive(Debug)]
pub(crate) struct Compiled<const N: usize> {
    pub key: [usize; 4],
    pub requests: Vec<Request>,
    pub plan: PreparedPlan,
    pub spans: Vec<crate::raw::Span>,
    pub has_bulk: bool,
    pub raw_bytes: [u64; N],
    frontiers: Vec<usize>,
    first_source_read: Option<usize>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Attempt {
    #[default]
    None,
    Canonical,
    Complete,
    Changed,
    Faulted,
}

#[derive(Debug)]
pub(crate) struct CapturePlan<const N: usize> {
    pub enabled: bool,
    pub scattered: bool,
    pub active: bool,
    pub key: [usize; 4],
    pub requests: Vec<Request>,
    pub compiled: Option<Compiled<N>>,
    pub retire: bool,
    pub sealed: bool,
    pub prefix_len: usize,
    pub prefix_cursor: usize,
    pub failed_read: Option<usize>,
    pub blocked: bool,
    pub attempt: Attempt,
    pub canonical_reads: usize,
    pub source_observed: bool,
}

impl<const N: usize> Default for CapturePlan<N> {
    fn default() -> Self {
        Self {
            enabled: false,
            scattered: false,
            active: false,
            key: [0; 4],
            requests: Vec::new(),
            compiled: None,
            retire: false,
            sealed: false,
            prefix_len: 0,
            prefix_cursor: 0,
            failed_read: None,
            blocked: false,
            attempt: Attempt::None,
            canonical_reads: 0,
            source_observed: false,
        }
    }
}

impl<const N: usize> CapturePlan<N> {
    pub fn begin(&mut self, key: [usize; 4]) {
        self.active = true;
        self.key = key;
        self.requests.clear();
        self.sealed = false;
        self.prefix_len = 0;
        self.prefix_cursor = 0;
        self.failed_read = None;
        self.blocked = false;
        self.attempt = Attempt::Canonical;
        self.canonical_reads = 0;
        self.source_observed = false;
        ATTEMPTS.fetch_add(1, Ordering::Relaxed);
    }

    pub fn block(&mut self) {
        if !self.blocked {
            PREFIX_MISMATCHES.fetch_add(1, Ordering::Relaxed);
        }
        self.blocked = true;
        self.retire = true;
        self.sealed = false;
    }

    pub fn note_execution(&mut self, completed: usize, failed: bool) {
        EXECUTIONS.fetch_add(1, Ordering::Relaxed);
        PLANNED_READS.fetch_add((completed + usize::from(failed)) as u64, Ordering::Relaxed);
        if let Some(compiled) = &self.compiled {
            self.source_observed = compiled
                .first_source_read
                .is_some_and(|first| first < completed + usize::from(failed));
            let frontiers = compiled
                .frontiers
                .partition_point(|frontier| *frontier <= completed);
            CONTROL_FRONTIERS.fetch_add(frontiers as u64, Ordering::Relaxed);
        }
    }

    pub fn note_backend_elapsed(elapsed: std::time::Duration) {
        BACKEND_NS.fetch_add(
            elapsed.as_nanos().min(u64::MAX as u128) as u64,
            Ordering::Relaxed,
        );
    }

    pub fn finish(&mut self, eligible: bool, overflowed: bool) {
        if !self.active {
            return;
        }
        self.active = false;
        if overflowed && self.source_observed {
            OVERFLOW_NO_RETRY.fetch_add(1, Ordering::Relaxed);
            self.retire = true;
        }
        if self.prefix_cursor != self.prefix_len {
            self.block();
        }
        self.sealed = eligible && !self.blocked && self.attempt != Attempt::Complete;
        if !eligible && self.attempt != Attempt::Complete {
            INELIGIBLE.fetch_add(1, Ordering::Relaxed);
            self.retire = true;
        }
        match self.attempt {
            Attempt::Complete => {
                COMPLETE.fetch_add(1, Ordering::Relaxed);
            }
            Attempt::Changed => {
                CHANGED.fetch_add(1, Ordering::Relaxed);
            }
            Attempt::Faulted => {
                FAULTED.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
        CANONICAL_READS.fetch_add(self.canonical_reads as u64, Ordering::Relaxed);
    }

    /// Called only by the recycle worker, before clearing the captured bytes.
    pub fn prepare(
        &mut self,
        observations: &mut Observations,
        has_bulk: bool,
        raw_bytes: [u64; N],
        len: usize,
    ) {
        let started = std::time::Instant::now();
        if self.sealed && observations.eligible && !self.requests.is_empty() {
            let reads = self
                .requests
                .iter()
                .map(|request| Read {
                    address: request.address,
                    destination_offset: request.start,
                    len: request.len,
                })
                .collect();
            let (controls, expected) = compact_controls(&self.requests, observations, len);
            if let Some(plan) = PreparedPlan::new(reads, controls, expected, len)
                .and_then(|plan| plan.with_hints(observations.hints.clone()))
            {
                let requests = self.requests.clone();
                let spans = requests
                    .iter()
                    .filter(|request| request.len != 0)
                    .map(|request| crate::raw::Span {
                        addr: request.address,
                        len: request.len as u32,
                        ok: true,
                        mask: request.mask,
                        start: request.start as u32,
                        bulk: request.bulk,
                    })
                    .collect();
                let first_source_read = requests.iter().position(|request| request.len != 0);
                let mut frontiers: Vec<usize> = plan
                    .controls()
                    .iter()
                    .map(|control| control.after_reads)
                    .collect();
                frontiers.dedup();
                self.compiled = Some(Compiled {
                    key: self.key,
                    requests,
                    plan,
                    spans,
                    has_bulk,
                    raw_bytes,
                    frontiers,
                    first_source_read,
                });
                self.retire = false;
                REBUILDS.fetch_add(1, Ordering::Relaxed);
            } else {
                self.retire = true;
            }
        }
        if self.retire {
            self.compiled = None;
            self.retire = false;
        }
        if observations.after_reads > self.requests.capacity() {
            self.requests
                .reserve_exact(observations.after_reads - self.requests.len());
        }
        observations.grow_for_next();
        self.sealed = false;
        PREPARE_NS.fetch_add(
            started.elapsed().as_nanos().min(u64::MAX as u128) as u64,
            Ordering::Relaxed,
        );
    }

    pub fn capacity_bytes(&self) -> u64 {
        crate::raw_memory::vector_bytes(&self.requests)
            + self.compiled.as_ref().map_or(0, |compiled| {
                crate::raw_memory::vector_bytes(&compiled.requests)
                    + crate::raw_memory::vector_bytes(&compiled.spans)
                    + compiled.plan.capacity_bytes() as u64
                    + crate::raw_memory::vector_bytes(&compiled.frontiers)
            })
    }
}

/// Raw destinations are append-only: after a control has checked owned bytes,
/// later reads cannot overwrite them. Keep the earliest check of every byte;
/// repeated parses of the same view need no additional native-plan comparison.
/// This optimization is deliberately outside PreparedPlan's general API,
/// whose callers may use overlapping writable destinations.
fn compact_controls(
    requests: &[Request],
    observations: &Observations,
    len: usize,
) -> (Vec<Control>, Vec<u8>) {
    let mut end = 0;
    let append_only = requests.iter().all(|request| {
        let valid = request.start == end;
        end = end.saturating_add(request.len);
        valid
    }) && end == len;
    if !append_only
        || observations.controls.iter().any(|control| {
            control
                .offset
                .checked_add(control.len)
                .is_none_or(|end| end > len)
        })
    {
        return (observations.controls.clone(), observations.expected.clone());
    }
    let mut checked = vec![0_u8; len];
    let mut controls = Vec::with_capacity(observations.controls.len());
    let mut expected = Vec::with_capacity(observations.expected.len());
    for control in &observations.controls {
        let covered = &mut checked[control.offset..control.offset + control.len];
        if !covered.contains(&0) {
            continue;
        }
        covered.fill(1);
        controls.push(Control {
            expected_offset: expected.len(),
            ..*control
        });
        expected.extend_from_slice(
            &observations.expected[control.expected_offset..control.expected_offset + control.len],
        );
    }
    REDUNDANT_CONTROLS.fetch_add(
        (observations.controls.len() - controls.len()) as u64,
        Ordering::Relaxed,
    );
    (controls, expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::{Memory, memory::SliceMemory, read_plan::Outcome};

    #[test]
    fn redundant_checks_keep_the_first_failure_frontier_and_fresh_unchecked_payload() {
        let requests = [
            Request {
                address: 0x1000,
                start: 0,
                len: 4,
                mask: 1,
                bulk: false,
            },
            Request {
                address: 0x1004,
                start: 4,
                len: 4,
                mask: 1,
                bulk: false,
            },
        ];
        let mut observations = Observations::new(8, 32);
        observations.begin();
        observations.after_reads = 1;
        observations.observe(0, &[1, 2, 3, 4]);
        observations.after_reads = 2;
        observations.observe(2, &[3, 4]);
        observations.observe(4, &[5, 6]);
        observations.observe(0, &[1, 2, 3, 4]);
        let (controls, expected) = compact_controls(&requests, &observations, 8);
        assert_eq!(controls.len(), 2);
        let plan = PreparedPlan::new(
            requests
                .iter()
                .map(|r| Read {
                    address: r.address,
                    destination_offset: r.start,
                    len: r.len,
                })
                .collect(),
            controls,
            expected,
            8,
        )
        .unwrap();
        for (byte, outcome) in [
            (1, Outcome::ControlChanged { completed_reads: 1 }),
            (5, Outcome::ControlChanged { completed_reads: 2 }),
            (7, Outcome::Complete),
        ] {
            let mut data = [1, 2, 3, 4, 5, 6, 7, 8];
            data[byte] = 99;
            let mut output = [0; 8];
            assert_eq!(
                SliceMemory {
                    base: 0x1000,
                    bytes: &data
                }
                .execute_read_plan(&plan, &mut output),
                Some(outcome)
            );
            if outcome == Outcome::Complete {
                assert_eq!(output, data);
            }
        }
        let mut overlap = requests;
        overlap[1].start = 2;
        assert_eq!(
            compact_controls(&overlap, &observations, 6),
            (observations.controls.clone(), observations.expected.clone())
        );
    }
}
