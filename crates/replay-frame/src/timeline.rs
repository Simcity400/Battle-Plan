//! Measurement-only spans of a preparation's tasks and phases: when each began and ended,
//! relative to the process's start, with the segment, lane and bytes it was of. Off unless a
//! measurement harness calls [`enable`]; then a span costs a clock read at each end and one
//! entry, and otherwise a relaxed load.

use serde_json::{Value, json};
use std::{
    sync::{
        Mutex, OnceLock, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

static ENABLED: AtomicBool = AtomicBool::new(false);
static ORIGIN: OnceLock<Instant> = OnceLock::new();
static SPANS: Mutex<Vec<Closed>> = Mutex::new(Vec::new());

struct Closed {
    kind: &'static str,
    start: Duration,
    end: Duration,
    segment: Option<usize>,
    lane: Option<Box<str>>,
    bytes: Option<usize>,
}

/// Records spans for the rest of the process.
pub fn enable() {
    ORIGIN.get_or_init(origin);
    ENABLED.store(true, Ordering::Release);
}

#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

struct Open {
    kind: &'static str,
    start: Instant,
    segment: Option<usize>,
    lane: Option<Box<str>>,
    bytes: Option<usize>,
}

/// A span of `kind` from now until dropped.
#[must_use]
pub struct Span(Option<Open>);

#[inline]
pub fn span(kind: &'static str) -> Span {
    if !enabled() {
        return Span(None);
    }
    Span(Some(Open {
        kind,
        start: Instant::now(),
        segment: None,
        lane: None,
        bytes: None,
    }))
}

impl Span {
    pub fn at(mut self, segment: usize) -> Self {
        if let Some(open) = &mut self.0 {
            open.segment = Some(segment);
        }
        self
    }

    pub fn lane(mut self, lane: &str) -> Self {
        if let Some(open) = &mut self.0 {
            open.lane = Some(lane.into());
        }
        self
    }

    pub fn bytes(&mut self, bytes: usize) {
        if let Some(open) = &mut self.0 {
            open.bytes = Some(bytes);
        }
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        let Some(open) = self.0.take() else {
            return;
        };
        let origin = *ORIGIN.get_or_init(origin);
        let closed = Closed {
            kind: open.kind,
            start: open.start.saturating_duration_since(origin),
            end: Instant::now().saturating_duration_since(origin),
            segment: open.segment,
            lane: open.lane,
            bytes: open.bytes,
        };
        SPANS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(closed);
    }
}

/// Every span closed so far, by start: `[kind, start µs, end µs, segment, lane, bytes]`.
pub fn snapshot() -> Value {
    let spans = SPANS.lock().unwrap_or_else(PoisonError::into_inner);
    let mut order: Vec<&Closed> = spans.iter().collect();
    order.sort_by_key(|span| (span.start, span.end));
    Value::Array(
        order
            .into_iter()
            .map(|span| {
                json!([
                    span.kind,
                    span.start.as_micros() as u64,
                    span.end.as_micros() as u64,
                    span.segment,
                    span.lane.as_deref(),
                    span.bytes,
                ])
            })
            .collect(),
    )
}

/// The process's start, as an instant: now, less the time since its creation.
fn origin() -> Instant {
    let now = Instant::now();
    since_creation()
        .and_then(|since| now.checked_sub(since))
        .unwrap_or(now)
}

#[cfg(windows)]
fn since_creation() -> Option<Duration> {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentProcess, GetProcessTimes},
    };
    // FILETIME counts 100 ns from 1601, 11,644,473,600 s before the Unix epoch.
    const UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let [mut created, mut exited, mut kernel, mut user] = [zero; 4];
    // SAFETY: the pseudo-handle of the current process and four owned FILETIMEs.
    let read = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    if read == 0 {
        return None;
    }
    let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
    let created = Duration::from_nanos(ticks.checked_sub(UNIX_EPOCH_TICKS)?.checked_mul(100)?);
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .checked_sub(created)
}

#[cfg(not(windows))]
fn since_creation() -> Option<Duration> {
    None
}
