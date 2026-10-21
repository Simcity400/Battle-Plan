//! The viewer's opt-in profiling switches and the CPU laps they print.
//!
//! Every switch is an environment variable read once; unset, none of them adds
//! work to a frame. The viewer README lists them.

use std::{ops::Range, sync::OnceLock, time::Instant};

/// The first frame of a profiled `--window-dump` sweep; the frames before it
/// settle the first hour and its caches.
pub const SWEEP_START: u32 = 121;

/// The pace of a profiled capture, `BATTLE_PLAN_VIEWER_PROFILE_HZ` frames a second.
pub fn hz() -> f64 {
    static HZ: OnceLock<f64> = OnceLock::new();
    *HZ.get_or_init(|| parse_hz(std::env::var("BATTLE_PLAN_VIEWER_PROFILE_HZ").or_else(|_| std::env::var("HOI4_VIEWER_PROFILE_HZ")).ok().as_deref()))
}

/// The frames of a profiled capture's sweep: `BATTLE_PLAN_VIEWER_PROFILE_SWEEP`
/// frames from [`SWEEP_START`].
pub fn sweep() -> Range<u32> {
    static FRAMES: OnceLock<u32> = OnceLock::new();
    let frames = *FRAMES
        .get_or_init(|| parse_sweep(std::env::var("BATTLE_PLAN_VIEWER_PROFILE_SWEEP").or_else(|_| std::env::var("HOI4_VIEWER_PROFILE_SWEEP")).ok().as_deref()));
    sweep_range(frames)
}

/// Frames a second: a finite 1..=1000, else 60.
pub fn parse_hz(value: Option<&str>) -> f64 {
    value
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|hz| hz.is_finite() && (1.0..=1000.0).contains(hz))
        .unwrap_or(60.0)
}

/// The longest sweep, about three hours at 120 Hz.
const MAX_SWEEP: u32 = 1_296_000;

/// Sweep frames: 1..=[`MAX_SWEEP`], else 120.
pub fn parse_sweep(value: Option<&str>) -> u32 {
    value
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|frames| (1..=MAX_SWEEP).contains(frames))
        .unwrap_or(120)
}

/// The sweep's frames, which cannot overflow however large `frames` is.
pub fn sweep_range(frames: u32) -> Range<u32> {
    SWEEP_START..SWEEP_START.saturating_add(frames)
}

/// Kernel and user CPU time used by the calling thread, excluding waits.
/// Returns `None` if unavailable; callers sample it only while profiling.
pub fn current_thread_cpu_time() -> Option<std::time::Duration> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::FILETIME,
            System::Threading::{GetCurrentThread, GetThreadTimes},
        };
        let empty = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut created, mut exited, mut kernel, mut user) = (empty, empty, empty, empty);
        // SAFETY: the current thread's pseudo handle needs no closing, and
        // each output points to a live FILETIME for the duration of the call.
        let read = unsafe {
            GetThreadTimes(
                GetCurrentThread(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        };
        if read == 0 {
            return None;
        }
        let ticks = |time: FILETIME| {
            (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
        };
        let total = ticks(kernel).checked_add(ticks(user))?;
        Some(std::time::Duration::new(
            total / 10_000_000,
            ((total % 10_000_000) * 100) as u32,
        ))
    }
    #[cfg(not(windows))]
    None
}

/// `name=ms` pairs of the laps at least `min_ms` long, three decimals.
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

/// Part of a frame on this thread, timed from [`span`] until it drops while profiling
/// (`BATTLE_PLAN_VIEWER_PROFILE`); [`take_spans`] reports each name's total.
pub struct Span {
    name: &'static str,
    start: Option<Instant>,
}

pub fn span(name: &'static str) -> Span {
    Span {
        name,
        start: crate::profile_writer::profiling().then(Instant::now),
    }
}

impl Drop for Span {
    fn drop(&mut self) {
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

/// Adds an already measured coarse lap under `BATTLE_PLAN_VIEWER_PROFILE`. Unlike
/// ordinary spans, these laps report zero and sub-0.05 ms totals too, so a short
/// worker wait is distinguishable from a lap that was not measured.
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

/// This thread's spans since the last call, at least 0.05 ms each, plus every
/// explicitly recorded coarse lap.
pub fn take_spans() -> String {
    SPANS.with(|spans| spans_line(&std::mem::take(&mut *spans.borrow_mut())))
}

/// Named CPU laps of one frame, recorded only while `start` was given a clock.
#[derive(Default)]
pub struct CpuLaps {
    last: Option<Instant>,
    laps: Vec<(&'static str, f64)>,
}

impl CpuLaps {
    /// Starts a frame's laps at `clock`; `None` records nothing.
    pub fn start(&mut self, clock: Option<Instant>) {
        self.last = clock;
        self.laps.clear();
    }

    pub fn lap(&mut self, name: &'static str) {
        if self.last.is_some() {
            self.lap_at(name, Instant::now(), false);
        }
    }

    /// Ends a lap, summing repeated pass/view labels in their first-use order.
    pub fn lap_sum(&mut self, name: &'static str) {
        if self.last.is_some() {
            self.lap_at(name, Instant::now(), true);
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

    /// Prefixes a separate set of laps and makes space-containing pass labels
    /// single field names in the frame's existing profiling line.
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
