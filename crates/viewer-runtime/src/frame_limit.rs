
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

static LIMIT: AtomicU64 = AtomicU64::new(0);

pub fn limit_to(hz: Option<f64>) {
    LIMIT.store(held(hz).map_or(0, f64::to_bits), Ordering::Release);
}

pub fn limit() -> Option<f64> {
    let held = LIMIT.load(Ordering::Acquire);
    (held != 0).then(|| f64::from_bits(held))
}

fn held(hz: Option<f64>) -> Option<f64> {
    hz.filter(|hz| dashboard_core::protocol::MAP_FRAME_LIMIT_RANGE.contains(hz))
}

fn interval() -> Option<Duration> {
    limit().map(|hz| Duration::from_secs_f64(1.0 / hz))
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Grid {
    due: Option<Instant>,
}

impl Grid {
    pub fn until(&self, now: Instant) -> Duration {
        match (limit(), self.due) {
            (Some(_), Some(due)) => due.saturating_duration_since(now),
            _ => Duration::ZERO,
        }
    }

    pub fn take(&mut self, now: Instant) {
        self.due = interval().map(|interval| stepped(self.due, now, interval));
    }
}

fn stepped(due: Option<Instant>, now: Instant, interval: Duration) -> Instant {
    match due {
        Some(due) if now.saturating_duration_since(due) < interval => due + interval,
        _ => now + interval,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_rate_a_display_could_show_holds_the_maps() {
        assert_eq!(held(Some(60.0)), Some(60.0));
        assert_eq!(held(Some(1.0)), Some(1.0));
        assert_eq!(held(Some(1_000.0)), Some(1_000.0));
        assert_eq!(held(None), None);
        for refused in [0.0, -60.0, 0.5, 1_000.1, f64::NAN, f64::INFINITY] {
            assert_eq!(held(Some(refused)), None, "{refused}");
        }
    }

    #[test]
    fn frames_fall_due_evenly_however_long_each_one_takes() {
        let interval = Duration::from_secs_f64(1.0 / 60.0);
        let origin = Instant::now();
        let mut due = None;
        let mut at = origin;
        let mut begun = Vec::new();
        for frame in 0..10u32 {
            begun.push(at);
            due = Some(stepped(due, at, interval));
            at += interval / 4 * (frame % 3 + 1);
            let waited = due.unwrap().saturating_duration_since(at);
            assert!(!waited.is_zero(), "frame {frame} spent less than its interval");
            at += waited;
        }
        for (frame, pair) in begun.windows(2).enumerate() {
            assert_eq!(pair[1] - pair[0], interval, "frames {frame} and {}", frame + 1);
        }
    }

    #[test]
    fn a_frame_a_whole_interval_late_starts_the_grid_again_instead_of_being_caught_up() {
        let interval = Duration::from_secs_f64(1.0 / 60.0);
        let origin = Instant::now();
        let first = stepped(None, origin, interval);
        assert_eq!(first, origin + interval);
        let late = first + interval / 3;
        let kept = stepped(Some(first), late, interval);
        assert_eq!(kept, first + interval);
        assert_eq!(kept.saturating_duration_since(late), interval - interval / 3);
        let overran = kept + interval * 2;
        assert_eq!(stepped(Some(kept), overran, interval), overran + interval);
        assert_eq!(stepped(Some(kept), kept + interval, interval), kept + interval * 2);
    }

}
