//! PROFILE-only cross-thread hour lifecycle. Disabled handles allocate and clock nothing.
use replay_data::{BranchId, Hour};
use std::{
    fmt::Write,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use viewer_foundation::{
    profile_writer::{profile_print, profiling},
    profiling::current_thread_cpu_time,
};

pub(super) type Stamp = Duration;

pub(super) fn now(enabled: bool) -> Option<Stamp> {
    enabled.then(|| {
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        ORIGIN.get_or_init(Instant::now).elapsed()
    })
}

const PARTS: [&str; 8] = [
    "political",
    "names",
    "overlay",
    "rails",
    "buildings",
    "movers",
    "fog",
    "snow",
];

/// Enables the permanent lifecycle output. The event-driven coordinator never
/// blocks on decode, pool admission or delivery, so its wait-cause totals are zero.
#[derive(Clone, Default)]
pub(super) struct CoordinatorProfile(bool);
impl CoordinatorProfile {
    pub(super) fn new() -> Self {
        Self(profiling())
    }
}

#[derive(Clone, Copy)]
pub(super) struct DecodeSample {
    service: u64,
    branch: BranchId,
    hour: Hour,
    start: Stamp,
    finish: Stamp,
    pub(super) ready: Stamp,
    cpu: Option<Duration>,
}

/// Only the decoding thread is sampled. Source-loader worker CPU is not included.
pub(super) struct DecodeClock(Option<(Stamp, Option<Duration>)>);
impl DecodeClock {
    pub(super) fn start(enabled: bool) -> Self {
        Self(now(enabled).map(|start| (start, current_thread_cpu_time())))
    }
    pub(super) fn finish(self, branch: BranchId, hour: Hour) -> Option<DecodeSample> {
        self.0.map(|(start, cpu)| {
            static SERVICE: AtomicU64 = AtomicU64::new(1);
            let finish = now(true).expect("enabled");
            DecodeSample {
                service: SERVICE.fetch_add(1, Ordering::Relaxed),
                branch,
                hour,
                start,
                finish,
                ready: finish,
                cpu: cpu.and_then(|start| current_thread_cpu_time()?.checked_sub(start)),
            }
        })
    }
}

#[derive(Default)]
struct Part {
    enqueue: Option<Stamp>,
    accepted: Option<Stamp>,
    start: Option<Stamp>,
    finish: Option<Stamp>,
    ready_blocks: [Duration; 3],
    state: &'static str,
}
struct Decode {
    branch: BranchId,
    hour: Hour,
    request: Stamp,
    returned: Stamp,
    sample: Option<DecodeSample>,
}
struct Record {
    id: u64,
    epoch: u64,
    branch: BranchId,
    hour: Hour,
    ticket: Option<u64>,
    want: Option<Stamp>,
    received: Option<Stamp>,
    dispatch: Stamp,
    assembly: Option<Stamp>,
    delivery: Option<Stamp>,
    send_start: Option<Stamp>,
    send_finish: Option<Stamp>,
    state: &'static str,
    emitted: bool,
    decodes: Vec<Decode>,
    parts: [Part; 8],
    dispatch_blocks: [Duration; 3],
    assembly_blocks: [Duration; 3],
}
struct Inner {
    record: Mutex<Record>,
}
impl Drop for Inner {
    fn drop(&mut self) {
        let record = self.record.get_mut().unwrap_or_else(|p| p.into_inner());
        if let Some(line) = record.take_line(false) {
            profile_print(line);
        }
    }
}

/// Delivered targets print once both sides of the send complete; last drop reports cancellations.
#[derive(Clone, Default)]
pub struct TargetTrace(Option<Arc<Inner>>);
impl TargetTrace {
    pub(super) fn new(
        coordinator: &CoordinatorProfile,
        epoch: u64,
        branch: BranchId,
        hour: Hour,
        want: Option<Stamp>,
        received: Option<Stamp>,
    ) -> Self {
        let Some(dispatch) = now(coordinator.0) else {
            return Self::default();
        };
        static TARGET: AtomicU64 = AtomicU64::new(1);
        Self(Some(Arc::new(Inner {
            record: Mutex::new(Record {
                id: TARGET.fetch_add(1, Ordering::Relaxed),
                epoch,
                branch,
                hour,
                ticket: None,
                want,
                received,
                dispatch,
                assembly: None,
                delivery: None,
                send_start: None,
                send_finish: None,
                state: "cancelled",
                emitted: false,
                decodes: Vec::new(),
                parts: Default::default(),
                dispatch_blocks: [Duration::ZERO; 3],
                assembly_blocks: [Duration::ZERO; 3],
            }),
        })))
    }
    fn update(&self, work: impl FnOnce(&mut Record, Stamp)) {
        if let Some(inner) = &self.0 {
            let at = now(true).expect("enabled");
            work(
                &mut inner.record.lock().unwrap_or_else(|p| p.into_inner()),
                at,
            );
        }
    }
    pub(super) fn enabled(&self) -> bool {
        self.0.is_some()
    }
    pub(super) fn ticket(&self, ticket: u64) {
        self.update(|r, _| r.ticket = Some(ticket));
    }
    pub(super) fn decoded(
        &self,
        branch: BranchId,
        hour: Hour,
        request: Option<Stamp>,
        sample: Option<DecodeSample>,
    ) {
        if let Some(request) = request {
            self.update(|r, returned| {
                r.decodes.push(Decode {
                    branch,
                    hour,
                    request,
                    returned,
                    sample,
                })
            });
        }
    }
    /// Native colour reads are streamed: a large skip must not retain one
    /// profiling record per traversed hour inside the target either.
    pub(super) fn decoded_step(
        &self,
        branch: BranchId,
        hour: Hour,
        request: Option<Stamp>,
        sample: Option<DecodeSample>,
    ) {
        if let Some(request) = request {
            self.update(|r, returned| {
                let mut line = format!(
                    "PROFILE hour_colour_read id {} ticket {} decode=",
                    r.id,
                    r.ticket
                        .map_or_else(|| "-".into(), |ticket| ticket.to_string())
                );
                write_decode(
                    &mut line,
                    &Decode {
                        branch,
                        hour,
                        request,
                        returned,
                        sample,
                    },
                );
                profile_print(line);
            });
        }
    }
    pub(super) fn enqueue(&self, name: &str) {
        self.part(name, |p, at| p.enqueue = Some(at));
    }
    pub(super) fn accepted(&self, name: &str) {
        self.part(name, |p, at| p.accepted = Some(at));
    }
    pub(super) fn start(&self, name: &str) {
        self.part(name, |p, at| {
            p.start = Some(at);
            p.state = "running";
        });
    }
    pub(super) fn finish(&self, name: &str, state: &'static str) {
        self.part(name, |p, at| {
            p.finish = Some(at);
            p.state = state;
        });
    }
    fn part(&self, name: &str, work: impl FnOnce(&mut Part, Stamp)) {
        self.update(|r, at| {
            if let Some(index) = PARTS.iter().position(|part| *part == name) {
                work(&mut r.parts[index], at);
            }
        });
    }
    pub(super) fn assembled(&self) {
        self.update(|r, at| {
            r.assembly = Some(at);
            r.state = "assembled";
        });
    }
    pub(super) fn sending(&self) {
        self.update(|r, at| {
            r.send_start = Some(at);
            r.state = "unread";
        });
    }
    pub(super) fn sent(&self) {
        self.update(|r, at| r.send_finish = Some(at));
        self.emit_delivered();
    }
    pub(super) fn delivered(&self) {
        self.update(|r, at| {
            r.delivery = Some(at);
            r.state = "delivered";
        });
        self.emit_delivered();
    }
    fn emit_delivered(&self) {
        if let Some(inner) = &self.0 {
            let line = inner
                .record
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take_line(true);
            if let Some(line) = line {
                profile_print(line);
            }
        }
    }
}

fn ms(value: Duration) -> f64 {
    value.as_secs_f64() * 1000.0
}
fn stamp(value: Option<Stamp>) -> String {
    value.map_or_else(|| "-".to_owned(), |v| format!("{:.3}", ms(v)))
}
fn totals(values: [Duration; 3]) -> String {
    values.map(|v| format!("{:.3}", ms(v))).join("/")
}
fn difference(end: [Duration; 3], start: [Duration; 3]) -> [Duration; 3] {
    std::array::from_fn(|i| end[i].saturating_sub(start[i]))
}
impl Record {
    fn take_line(&mut self, delivered_only: bool) -> Option<String> {
        if self.emitted
            || (delivered_only && (self.delivery.is_none() || self.send_finish.is_none()))
        {
            return None;
        }
        self.emitted = true;
        Some(self.line())
    }
    fn line(&self) -> String {
        let mut line = format!(
            "PROFILE hour_target id {} ticket {} epoch {} branch {} hour {} status {} want {} received {} dispatch {:.3} assembly {} delivery {} send {}/{} blocked {}",
            self.id,
            self.ticket
                .map_or_else(|| "-".to_owned(), |t| t.to_string()),
            self.epoch,
            self.branch,
            self.hour,
            self.state,
            stamp(self.want),
            stamp(self.received),
            ms(self.dispatch),
            stamp(self.assembly),
            stamp(self.delivery),
            stamp(self.send_start),
            stamp(self.send_finish),
            totals(difference(self.assembly_blocks, self.dispatch_blocks))
        );
        if let Some(last) = self
            .parts
            .iter()
            .filter(|p| p.finish.is_some())
            .max_by_key(|p| p.finish)
        {
            let _ = write!(
                line,
                " ready_blocked {}",
                totals(difference(self.assembly_blocks, last.ready_blocks))
            );
        }
        line.push_str(" parts=");
        for (i, part) in self.parts.iter().enumerate() {
            if i > 0 {
                line.push(',');
            }
            let _ = write!(
                line,
                "{}:{}/{}/{}/{}/{}:{}",
                PARTS[i],
                stamp(part.enqueue),
                stamp(part.accepted),
                stamp(part.start),
                stamp(part.finish),
                stamp(
                    part.finish
                        .zip(self.assembly)
                        .map(|(finish, assembly)| assembly.saturating_sub(finish))
                ),
                if part.state.is_empty() {
                    "missing"
                } else {
                    part.state
                }
            );
        }
        line.push_str(" decodes=");
        for (i, decode) in self.decodes.iter().enumerate() {
            if i > 0 {
                line.push(',');
            }
            write_decode(&mut line, decode);
        }
        line
    }
}

fn write_decode(line: &mut String, decode: &Decode) {
    let _ = write!(
        line,
        "{}/{}:{}/{}",
        decode.branch,
        decode.hour,
        stamp(Some(decode.request)),
        stamp(Some(decode.returned))
    );
    if let Some(s) = decode.sample {
        let _ = write!(
            line,
            "/{}@{}/{}:{:.3}/{:.3}/{:.3}:{}",
            s.service,
            s.branch,
            s.hour,
            ms(s.start),
            ms(s.finish),
            ms(s.ready),
            stamp(s.cpu)
        );
    } else {
        line.push_str("/unavailable");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }
    #[test]
    fn target_trace_disabled_has_no_clock_records_or_shared_state() {
        let trace = TargetTrace::new(&CoordinatorProfile::default(), 1, 2, 3, None, None);
        assert!(!trace.enabled());
        trace.enqueue("fog");
        trace.start("fog");
        trace.finish("fog", "done");
        trace.assembled();
        trace.delivered();
        assert!(trace.0.is_none());
        assert!(now(false).is_none());
        assert!(DecodeClock::start(false).finish(2, 3).is_none());
    }
    #[test]
    fn target_trace_line_preserves_speculation_zeros_missing_and_fixed_part_order() {
        let r = fixture();
        let line = r.line();
        assert!(line.contains("blocked 2.000/4.000/0.000 ready_blocked 1.000/3.000/0.000"));
        assert!(line.contains("movers:4.000/6.000/5.000/10.000/10.000:done"));
        assert!(line.contains("fog:-/-/-/-/-:missing"));
        assert!(line.ends_with("decodes=3/4:3.000/3.000/7@3/4:0.000/1.000/1.000:0.000"));
        assert!(line.find("political:") < line.find("movers:"));
    }
    fn fixture() -> Record {
        Record {
            id: 1,
            epoch: 2,
            branch: 3,
            hour: 4,
            ticket: Some(9),
            want: Some(at(0)),
            received: Some(at(1)),
            dispatch: at(2),
            assembly: Some(at(20)),
            delivery: Some(at(22)),
            send_start: Some(at(21)),
            send_finish: Some(at(21)),
            state: "delivered",
            emitted: false,
            decodes: vec![Decode {
                branch: 3,
                hour: 4,
                request: at(3),
                returned: at(3),
                sample: Some(DecodeSample {
                    service: 7,
                    branch: 3,
                    hour: 4,
                    start: at(0),
                    finish: at(1),
                    ready: at(1),
                    cpu: Some(at(0)),
                }),
            }],
            parts: std::array::from_fn(|i| {
                if i == 5 {
                    Part {
                        enqueue: Some(at(4)),
                        accepted: Some(at(6)),
                        start: Some(at(5)),
                        finish: Some(at(10)),
                        ready_blocks: [at(2), at(1), at(0)],
                        state: "done",
                    }
                } else {
                    Part::default()
                }
            }),
            dispatch_blocks: [at(1), at(0), at(0)],
            assembly_blocks: [at(3), at(4), at(0)],
        }
    }
    #[test]
    fn target_trace_delivered_line_is_once_only_in_either_send_receive_order() {
        for sender_first in [false, true] {
            let mut r = fixture();
            r.delivery = None;
            r.send_finish = None;
            if sender_first {
                r.send_finish = Some(at(21));
            } else {
                r.delivery = Some(at(22));
            }
            assert!(r.take_line(true).is_none());
            r.delivery = Some(at(22));
            r.send_finish = Some(at(21));
            assert!(r.take_line(true).unwrap().contains("status delivered"));
            assert!(r.take_line(true).is_none());
            assert!(
                r.take_line(false).is_none(),
                "last drop must not print a duplicate"
            );
        }
        let mut cancelled = fixture();
        cancelled.delivery = None;
        cancelled.state = "cancelled";
        assert!(cancelled.take_line(true).is_none());
        assert!(
            cancelled
                .take_line(false)
                .unwrap()
                .contains("status cancelled")
        );
        assert!(cancelled.take_line(false).is_none());
    }
}
