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

/// `name=ms` pairs of the laps at least `min_ms` long, three decimals.
pub fn laps_line(laps: &[(&'static str, f64)], min_ms: f64) -> String {
    laps.iter()
        .filter(|(_, ms)| *ms >= min_ms)
        .map(|(name, ms)| format!("{name}={ms:.3}"))
        .collect::<Vec<_>>()
        .join(" ")
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
        if let Some(last) = self.last {
            let now = Instant::now();
            self.laps
                .push((name, now.duration_since(last).as_secs_f64() * 1000.0));
            self.last = Some(now);
        }
    }

    pub fn line(&self) -> String {
        laps_line(&self.laps, 0.0)
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
    fn a_laps_line_keeps_order_and_drops_short_laps() {
        let laps = [("poll", 0.01), ("render", 1.2345), ("names", 0.05)];
        assert_eq!(laps_line(&laps, 0.05), "render=1.234 names=0.050");
        assert_eq!(laps_line(&laps, 0.0), "poll=0.010 render=1.234 names=0.050");
        let mut cpu = CpuLaps::default();
        cpu.lap("ignored");
        assert_eq!(cpu.line(), "");
    }
}
