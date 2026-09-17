//! Bounded discovery traces for fresh scalar raw acquisition.

use engine::read_plan::{Control, PreparedPlan, Read};
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

pub(crate) fn diagnostics() -> serde_json::Value {
    serde_json::json!({
        "scope": "opted-in scalar raw acquisition since attach; every plan executes fresh source reads",
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
            required_controls: 0,
            required_bytes: 0,
        }
    }

    pub fn begin(&mut self) {
        self.controls.clear();
        self.expected.clear();
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

    pub fn capacity_bytes(&self) -> u64 {
        crate::raw_memory::vector_bytes(&self.controls)
            + crate::raw_memory::vector_bytes(&self.expected)
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
    pub spans: usize,
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
            let controls = observations.controls.clone();
            let expected = observations.expected.clone();
            if let Some(plan) = PreparedPlan::new(reads, controls, expected, len) {
                let requests = self.requests.clone();
                let spans = requests.iter().filter(|request| request.len != 0).count();
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
                    + compiled.plan.capacity_bytes() as u64
                    + crate::raw_memory::vector_bytes(&compiled.frontiers)
            })
    }
}
