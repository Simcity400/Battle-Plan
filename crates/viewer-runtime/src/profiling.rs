
use std::{ops::Range, sync::OnceLock, time::Instant};

mod frame_cpu;
mod thread_clock;
pub use frame_cpu::{
    FrameOwners, OwnerSample, ThreadSpan, begin_thread_frame, record_count,
    record_owner_sample, take_thread_spans, thread_span,
};
pub use thread_clock::cpu_clock_hz;

pub const SWEEP_START: u32 = 121;

pub fn hz() -> f64 {
    static HZ: OnceLock<f64> = OnceLock::new();
    *HZ.get_or_init(|| parse_hz(std::env::var("BATTLEPLAN_VIEWER_PROFILE_HZ").or_else(|_| std::env::var("HOI4_VIEWER_PROFILE_HZ")).ok().as_deref()))
}

pub fn sweep() -> Range<u32> {
    static FRAMES: OnceLock<u32> = OnceLock::new();
    let frames = *FRAMES
        .get_or_init(|| parse_sweep(std::env::var("BATTLEPLAN_VIEWER_PROFILE_SWEEP").or_else(|_| std::env::var("HOI4_VIEWER_PROFILE_SWEEP")).ok().as_deref()));
    sweep_range(frames)
}

pub fn parse_hz(value: Option<&str>) -> f64 {
    value
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|hz| hz.is_finite() && (1.0..=1000.0).contains(hz))
        .unwrap_or(60.0)
}

const MAX_SWEEP: u32 = 1_296_000;

pub fn parse_sweep(value: Option<&str>) -> u32 {
    value
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|frames| (1..=MAX_SWEEP).contains(frames))
        .unwrap_or(120)
}

pub fn sweep_range(frames: u32) -> Range<u32> {
    SWEEP_START..SWEEP_START.saturating_add(frames)
}

pub fn current_thread_cpu_time() -> Option<std::time::Duration> {
    thread_clock::current()
}

#[derive(Clone, Debug)]
pub struct BuilderSpan {
    pub name: &'static str,
    pub ms: f64,
    pub cpu_ms: Option<f64>,
}

pub struct BuilderSpans {
    enabled: bool,
    records: Vec<BuilderSpan>,
}

impl BuilderSpans {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            records: Vec::new(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn measure<T>(&mut self, name: &'static str, work: impl FnOnce(&mut Self) -> T) -> T {
        if !self.enabled {
            return work(self);
        }
        let started = Instant::now();
        let cpu_started = current_thread_cpu_time();
        let value = work(self);
        let cpu_ms = cpu_started
            .zip(current_thread_cpu_time())
            .and_then(|(before, after)| after.checked_sub(before))
            .map(|used| used.as_secs_f64() * 1000.0);
        self.records.push(BuilderSpan {
            name,
            ms: started.elapsed().as_secs_f64() * 1000.0,
            cpu_ms,
        });
        value
    }

    pub fn fork(&self) -> Self {
        Self {
            enabled: self.enabled,
            records: Vec::new(),
        }
    }

    pub fn append(&mut self, mut worker: Self) {
        if self.enabled {
            self.records.append(&mut worker.records);
        }
    }

    pub fn record(&mut self, record: BuilderSpan) {
        if self.enabled {
            self.records.push(record);
        }
    }

    pub fn into_records(self) -> Vec<BuilderSpan> {
        self.records
    }
}

pub fn laps_line(laps: &[(&'static str, f64)], min_ms: f64) -> String {
    laps.iter()
        .filter(|(_, ms)| *ms >= min_ms)
        .map(|(name, ms)| format!("{name}={ms:.3}"))
        .collect::<Vec<_>>()
        .join(" ")
}

std::thread_local! {
    static SPANS: std::cell::RefCell<Vec<(&'static str, f64, bool)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

pub struct Span {
    name: &'static str,
    start: Option<Instant>,
    cycles: Option<u64>,
}

pub fn span(name: &'static str) -> Span {
    Span {
        name,
        start: crate::profile_writer::profiling().then(Instant::now),
        cycles: (replay_frame::cpu::enabled() && shader_pipeline::gpu::stats::on_window_thread())
            .then(replay_frame::cpu::thread_cycles),
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        if let Some(cycles) = self.cycles {
            replay_frame::cpu::note(self.name, "span", cycles);
        }
        let Some(start) = self.start else {
            return;
        };
        SPANS.with(|spans| {
            add_span(
                &mut spans.borrow_mut(),
                self.name,
                start.elapsed().as_secs_f64() * 1000.0,
                false,
            );
        });
    }
}

pub fn record_lap(name: &'static str, elapsed: std::time::Duration) {
    if crate::profile_writer::profiling() {
        SPANS.with(|spans| {
            add_span(
                &mut spans.borrow_mut(),
                name,
                elapsed.as_secs_f64() * 1000.0,
                true,
            );
        });
    }
}

fn add_span(
    spans: &mut Vec<(&'static str, f64, bool)>,
    name: &'static str,
    ms: f64,
    required: bool,
) {
    match spans.iter_mut().find(|(span, _, _)| *span == name) {
        Some((_, total, always)) => {
            *total += ms;
            *always |= required;
        }
        None => spans.push((name, ms, required)),
    }
}

fn spans_line(spans: &[(&'static str, f64, bool)]) -> String {
    spans
        .iter()
        .filter(|(_, ms, required)| *required || *ms >= 0.05)
        .map(|(name, ms, _)| format!("{name}={ms:.3}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn take_spans() -> String {
    SPANS.with(|spans| spans_line(&std::mem::take(&mut *spans.borrow_mut())))
}

pub struct ThreadRecords {
    frame: frame_cpu::FrameRecords,
    spans: Vec<(&'static str, f64, bool)>,
}

pub fn take_thread_records() -> Option<ThreadRecords> {
    crate::profile_writer::profiling().then(|| ThreadRecords {
        frame: frame_cpu::take_frame_records(),
        spans: SPANS.with(|spans| std::mem::take(&mut *spans.borrow_mut())),
    })
}

pub fn adopt_thread_records(records: Option<ThreadRecords>) {
    let Some(records) = records else {
        return;
    };
    frame_cpu::adopt_frame_records(records.frame);
    SPANS.with(|spans| {
        let mut spans = spans.borrow_mut();
        for (name, ms, required) in records.spans {
            add_span(&mut spans, name, ms, required);
        }
    });
}

std::thread_local! {
    static PASS_COUNTS: std::cell::RefCell<PassCounts> = std::cell::RefCell::default();
}

type GpuCounts = [u64; shader_pipeline::gpu::stats::NAMES.len()];

#[derive(Default)]
struct PassCounts {
    last: Option<GpuCounts>,
    laps: Vec<(&'static str, GpuCounts)>,
}

fn pass_counts(name: &'static str) {
    let now = shader_pipeline::gpu::stats::peek();
    PASS_COUNTS.with(|counts| {
        let mut counts = counts.borrow_mut();
        let since = counts.last.map_or([0; shader_pipeline::gpu::stats::NAMES.len()], |last| {
            std::array::from_fn(|at| now[at].saturating_sub(last[at]))
        });
        counts.last = Some(now);
        match counts.laps.iter_mut().find(|(lap, _)| *lap == name) {
            Some((_, total)) => total.iter_mut().zip(since).for_each(|(total, n)| *total += n),
            None => counts.laps.push((name, since)),
        }
    });
}

pub fn take_pass_counts() -> Vec<(&'static str, Vec<(&'static str, u64)>)> {
    PASS_COUNTS.with(|counts| {
        let mut counts = counts.borrow_mut();
        counts.last = None;
        std::mem::take(&mut counts.laps)
            .into_iter()
            .filter_map(|(lap, counts)| {
                let named: Vec<_> = shader_pipeline::gpu::stats::NAMES
                    .iter()
                    .copied()
                    .zip(counts)
                    .filter(|(_, n)| *n != 0)
                    .collect();
                (!named.is_empty()).then_some((lap, named))
            })
            .collect()
    })
}

#[derive(Default)]
pub struct CpuLaps {
    last: Option<Instant>,
    laps: Vec<(&'static str, f64)>,
    cycles: Option<u64>,
    walls: Option<(Instant, Option<std::time::Duration>)>,
}

impl CpuLaps {
    pub fn start(&mut self, clock: Option<Instant>) {
        self.last = clock;
        self.laps.clear();
        self.cycles = replay_frame::cpu::enabled().then(replay_frame::cpu::thread_cycles);
        self.walls = None;
    }

    pub fn record_walls(&mut self) {
        if crate::view_record::on() {
            self.walls = Some((Instant::now(), current_thread_cpu_time()));
        }
    }

    pub fn lap(&mut self, name: &'static str) {
        self.note(name);
        if self.last.is_some() {
            self.lap_at(name, Instant::now(), false);
        }
    }

    pub fn lap_sum(&mut self, name: &'static str) {
        self.note(name);
        if self.last.is_some() {
            self.lap_at(name, Instant::now(), true);
        }
    }

    fn note(&mut self, name: &'static str) {
        if let Some((wall, cpu)) = self.walls {
            let (now, now_cpu) = (Instant::now(), current_thread_cpu_time());
            crate::view_record::add("render_wall_us", name, (now - wall).as_micros() as u64);
            if let (Some(cpu), Some(now_cpu)) = (cpu, now_cpu) {
                crate::view_record::add("render_cpu_us", name, now_cpu.saturating_sub(cpu).as_micros() as u64);
            }
            self.walls = Some((now, now_cpu));
        }
        if let Some(cycles) = self.cycles {
            replay_frame::cpu::note(name, "lap", cycles);
            self.cycles = Some(replay_frame::cpu::thread_cycles());
            pass_counts(name);
        }
    }

    fn lap_at(&mut self, name: &'static str, now: Instant, sum: bool) {
        let Some(last) = self.last else {
            return;
        };
        let ms = now.duration_since(last).as_secs_f64() * 1000.0;
        if let Some((_, total)) = self
            .laps
            .iter_mut()
            .find(|(label, _)| sum && *label == name)
        {
            *total += ms;
        } else {
            self.laps.push((name, ms));
        }
        self.last = Some(now);
    }

    pub fn line(&self) -> String {
        laps_line(&self.laps, 0.0)
    }

    pub fn line_with_prefix(&self, prefix: &str) -> String {
        self.laps
            .iter()
            .map(|(name, ms)| format!("{prefix}{}={ms:.3}", name.replace(' ', "_")))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_spans_disabled_run_once_without_records() {
        let mut calls = 0;
        let mut spans = BuilderSpans::new(false);
        let result: Result<_, &str> = spans.measure("outer", |spans| {
            calls += 1;
            spans.measure("outer.inner", |_| {
                calls += 1;
                Err::<(), _>("kept refusal")
            })
        });
        assert_eq!(result, Err("kept refusal"));
        assert_eq!(calls, 2);
        assert!(spans.records.is_empty());
        assert_eq!(spans.records.capacity(), 0);
        assert!(spans.into_records().is_empty());
    }

    #[test]
    fn builder_spans_keep_nested_completion_order_and_error_results() {
        let mut calls = Vec::new();
        let mut spans = BuilderSpans::new(true);
        let result = spans.measure("fog.intel_lookup", |spans| {
            calls.push("lookup");
            spans.measure("fog.intel_build", |_| {
                calls.push("build");
                Err::<(), _>("same refusal")
            })
        });
        assert_eq!(result, Err("same refusal"));
        assert_eq!(calls, ["lookup", "build"]);
        assert_eq!(
            spans
                .into_records()
                .iter()
                .map(|span| span.name)
                .collect::<Vec<_>>(),
            ["fog.intel_build", "fog.intel_lookup"]
        );
    }

    #[test]
    fn builder_spans_adopt_worker_records_in_join_order() {
        for enabled in [false, true] {
            let mut spans = BuilderSpans::new(enabled);
            let workers: Vec<_> = ["bake.worker_first", "bake.worker_second"]
                .map(|name| (name, spans.fork()))
                .into_iter()
                .collect();
            let workers = std::thread::scope(|scope| {
                let handles: Vec<_> = workers
                    .into_iter()
                    .map(|(name, mut worker)| {
                        scope.spawn(move || {
                            worker.measure(name, |_| 7);
                            worker
                        })
                    })
                    .collect();
                spans.measure("bake.join", |_| {
                    handles
                        .into_iter()
                        .map(|handle| handle.join().unwrap())
                        .collect::<Vec<_>>()
                })
            });
            for worker in workers {
                spans.append(worker);
            }
            if enabled {
                assert_eq!(
                    spans
                        .records
                        .iter()
                        .map(|span| span.name)
                        .collect::<Vec<_>>(),
                    ["bake.join", "bake.worker_first", "bake.worker_second"]
                );
                assert!(
                    spans
                        .records
                        .iter()
                        .all(|span| span.ms.is_finite() && span.ms >= 0.0)
                );
            } else {
                assert!(spans.records.is_empty());
                assert_eq!(spans.records.capacity(), 0);
            }
        }
    }

    #[test]
    fn builder_spans_keep_completed_scopes_when_a_parent_panics() {
        let mut spans = BuilderSpans::new(true);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            spans.measure("fog.parent", |spans| {
                spans.measure("fog.completed", |_| ());
                panic!("unfinished scope");
            });
        }));
        assert!(result.is_err());
        assert_eq!(
            spans
                .into_records()
                .iter()
                .map(|span| span.name)
                .collect::<Vec<_>>(),
            ["fog.completed"]
        );
    }

    #[test]
    fn the_pace_takes_finite_rates_within_bounds() {
        assert_eq!(parse_hz(None), 60.0);
        assert_eq!(parse_hz(Some("120")), 120.0);
        assert_eq!(parse_hz(Some(" 94.5 ")), 94.5);
        for bad in ["0", "-5", "NaN", "inf", "1e9", "fast", ""] {
            assert_eq!(parse_hz(Some(bad)), 60.0, "{bad}");
        }
    }

    #[test]
    fn the_sweep_length_is_bounded_and_cannot_overflow() {
        assert_eq!(parse_sweep(None), 120);
        assert_eq!(parse_sweep(Some("7200")), 7200);
        for bad in ["0", "-1", "4294967295", "many"] {
            assert_eq!(parse_sweep(Some(bad)), 120, "{bad}");
        }
        assert_eq!(sweep_range(240), 121..361);
        assert_eq!(sweep_range(u32::MAX).end, u32::MAX);
    }

    #[test]
    fn repeated_pass_laps_sum_in_first_use_order_and_reset() {
        let start = Instant::now();
        let mut cpu = CpuLaps::default();
        cpu.start(Some(start));
        for (name, micros) in [("terrain", 100), ("post LUT", 125), ("terrain", 425)] {
            cpu.lap_at(name, start + std::time::Duration::from_micros(micros), true);
        }
        assert_eq!(cpu.line(), "terrain=0.400 post LUT=0.025");
        assert_eq!(
            cpu.line_with_prefix("pass_"),
            "pass_terrain=0.400 pass_post_LUT=0.025"
        );
        cpu.start(None);
        cpu.lap_sum("disabled");
        assert_eq!(cpu.line(), "");
    }

    #[test]
    fn explicit_coarse_laps_keep_short_and_zero_waits() {
        let mut spans = Vec::new();
        add_span(&mut spans, "ordinary", 0.01, false);
        add_span(&mut spans, "caller", 0.01, true);
        add_span(&mut spans, "wait", 0.0, true);
        add_span(&mut spans, "caller", 0.02, true);
        assert_eq!(spans_line(&spans), "caller=0.030 wait=0.000");
    }

    #[test]
    fn a_laps_line_keeps_order_and_drops_short_laps() {
        let laps = [("poll", 0.01), ("render", 1.2345), ("names", 0.05)];
        assert_eq!(laps_line(&laps, 0.05), "render=1.234 names=0.050");
        assert_eq!(laps_line(&laps, 0.0), "poll=0.010 render=1.234 names=0.050");
        let mut cpu = CpuLaps::default();
        cpu.lap("ignored");
        assert_eq!(cpu.line(), "");
    }
}

pub fn counting() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BATTLEPLAN_VIEWER_COUNT_OVERLAY").or_else(|| std::env::var_os("HOI4_VIEWER_COUNT_OVERLAY")).is_some())
}
