
use std::time::{Duration, Instant};

const RESTARTED_MS: f64 = 1_000.0;

#[derive(Debug, Default)]
pub(crate) struct PageClock {
    offset: Option<f64>,
    newest: f64,
    epoch: Option<Instant>,
}

impl PageClock {
    pub(crate) fn age(&mut self, composed_at: Option<f64>, at: Instant) -> Duration {
        let Some(composed_at) = composed_at.filter(|at| at.is_finite() && *at >= 0.0) else {
            return Duration::ZERO;
        };
        let epoch = *self.epoch.get_or_insert(at);
        let here = at.saturating_duration_since(epoch).as_secs_f64() * 1e3;
        if composed_at + RESTARTED_MS < self.newest {
            self.offset = None;
            self.newest = composed_at;
        } else {
            self.newest = self.newest.max(composed_at);
        }
        let gap = here - composed_at;
        let offset = self.offset.map_or(gap, |held| held.min(gap));
        self.offset = Some(offset);
        Duration::from_secs_f64(((gap - offset) / 1e3).max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::PageClock;
    use std::time::{Duration, Instant};

    #[test]
    fn an_anchors_age_is_its_delivery_delay_once_the_pages_clock_is_known() {
        let start = Instant::now();
        let mut clock = PageClock::default();
        let at = |milliseconds: f64| start + Duration::from_secs_f64(milliseconds / 1e3);
        let age = |clock: &mut PageClock, composed: f64, arrived: f64| {
            clock.age(Some(composed), at(arrived)).as_secs_f64() * 1e3
        };
        assert_eq!(age(&mut clock, 10_000.0, 4.0), 0.0);
        assert!((age(&mut clock, 10_100.0, 124.0) - 20.0).abs() < 1e-6);
        assert_eq!(age(&mut clock, 10_200.0, 202.0), 0.0);
        assert!((age(&mut clock, 10_300.0, 307.0) - 5.0).abs() < 1e-6);
        assert!((age(&mut clock, 10_299.0, 308.0) - 7.0).abs() < 1e-6);
        for unstamped in [None, Some(f64::NAN), Some(f64::INFINITY), Some(-1.0)] {
            assert_eq!(clock.age(unstamped, at(400.0)), Duration::ZERO);
        }
        assert_eq!(age(&mut clock, 2.0, 500.0), 0.0);
        assert!((age(&mut clock, 102.0, 615.0) - 15.0).abs() < 1e-6);
    }
}
