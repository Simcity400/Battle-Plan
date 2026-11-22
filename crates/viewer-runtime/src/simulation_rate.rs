
use std::time::{Duration, Instant};

pub const PUBLICATIONS: f64 = 30.0;

pub fn rate() -> f64 {
    held(super::display_refresh::fastest())
}

pub fn interval() -> Duration {
    Duration::from_secs_f64(1.0 / rate())
}

fn held(refresh: f64) -> f64 {
    PUBLICATIONS.min(refresh)
}

#[derive(Clone, Copy, Debug)]
pub struct Pace {
    due: Instant,
}

impl Pace {
    pub fn new(now: Instant) -> Self {
        Self { due: now }
    }

    pub fn late(&self, now: Instant) -> Option<Duration> {
        (now >= self.due).then(|| now.duration_since(self.due))
    }

    pub fn take(&mut self, now: Instant, interval: Duration) {
        self.due = if now.saturating_duration_since(self.due) < interval { self.due + interval } else { now };
    }

    pub fn until(&self, now: Instant) -> Duration {
        self.due.saturating_duration_since(now)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Reach {
    held: Duration,
}

impl Reach {
    pub fn took(&mut self, took: Duration, interval: Duration) {
        self.held = ((self.held * 7 + took) / 8).min(interval);
    }

    pub fn ahead(self, interval: Duration) -> Duration {
        self.held + interval
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rate_is_the_simulations_own_and_never_above_the_displays() {
        for refresh in [60.0, 120.0, 144.0, 240.0, 360.0] {
            assert_eq!(held(refresh), PUBLICATIONS, "a {refresh} Hz display");
        }
        assert_eq!(held(24.0), 24.0, "never more publications than the display draws frames");
    }

    #[test]
    fn the_pace_holds_one_publication_an_interval_and_never_catches_up_in_a_burst() {
        let interval = Duration::from_millis(10);
        let start = Instant::now();
        let mut pace = Pace::new(start);
        let mut taken = 0;
        for ms in 0..1_000 {
            let now = start + Duration::from_millis(ms);
            if pace.late(now).is_some() {
                pace.take(now, interval);
                taken += 1;
            }
        }
        assert_eq!(taken, 100);
        assert_eq!(pace.until(start + Duration::from_millis(999)), Duration::from_millis(1));
        let mut pace = Pace::new(start);
        let mut taken = 0;
        for at in 0..1_000 {
            let now = start + Duration::from_millis(at * 10 + 3);
            assert_eq!(pace.late(now), Some(Duration::from_millis(3)), "late by what it waited");
            pace.take(now, interval);
            taken += 1;
        }
        assert_eq!(taken, 1_000);
        let mut pace = Pace::new(start);
        assert_eq!(pace.late(start), Some(Duration::ZERO));
        pace.take(start, interval);
        let behind = start + Duration::from_millis(95);
        assert_eq!(pace.late(behind), Some(Duration::from_millis(85)));
        pace.take(behind, interval);
        assert_eq!(pace.late(behind), Some(Duration::ZERO), "behind, the next is due at once");
        pace.take(behind, interval);
        assert_eq!(pace.late(behind), None, "and only the one");
        assert_eq!(pace.until(start + Duration::from_millis(100)), Duration::from_millis(5));
    }

    #[test]
    fn the_reach_covers_the_making_and_the_interval_the_publication_is_drawn_across() {
        let interval = Duration::from_millis(33);
        let mut reach = Reach::default();
        assert_eq!(reach.ahead(interval), interval, "before the first publication, the interval alone");
        for _ in 0..64 {
            reach.took(Duration::from_millis(8), interval);
        }
        let settled = reach.ahead(interval);
        assert!(settled.abs_diff(Duration::from_millis(41)) < Duration::from_micros(200), "{settled:?}");
        for _ in 0..64 {
            reach.took(Duration::from_millis(200), interval);
        }
        assert_eq!(reach.ahead(interval), interval * 2, "a making that overruns the interval adds one interval and no more");
    }
}
