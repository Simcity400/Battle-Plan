//! The page's clock as the map sees it, so an anchor is placed by the time the page composed it
//! rather than the time it arrived.
//!
//! The page stamps each anchor with its own monotonic clock (`performance.now()`), whose origin is
//! the page's, not this process's. The offset between the two is estimated from the smallest gap
//! seen between a stamp and the moment its anchor arrived here: every gap is the offset plus that
//! message's delivery delay, so the smallest is the closest to the offset. An anchor's age is then
//! that anchor's gap less the smallest, which is never negative and never counts time the page did
//! not spend. The first anchor of a session sets the estimate, so its own age is zero: the map can
//! only ever be behind the page, never ahead of it.

use std::time::{Duration, Instant};

/// A stamp this much before the newest seen is a page whose clock started again (a reload restarts
/// `performance.now()` at zero), not two anchors crossing, which cross by at most the IPC queue.
const RESTARTED_MS: f64 = 1_000.0;

#[derive(Debug, Default)]
pub(crate) struct PageClock {
    /// The smallest gap seen between a stamp and its arrival, in milliseconds.
    offset: Option<f64>,
    /// The newest stamp seen, which a restarted page falls below.
    newest: f64,
    /// This process's clock as the gaps are measured from it.
    epoch: Option<Instant>,
}

impl PageClock {
    /// How long ago the page composed an anchor it stamped `composed_at`, which arrived `at`.
    /// Zero for an anchor the page did not stamp.
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

    /// The first anchor of a session sets the offset, so its age is zero; every later anchor's age
    /// is the delivery delay it spent beyond the quickest seen, whatever the two clocks' origins.
    /// Two anchors crossing are not a restart; a page that reloaded stamps from near zero again
    /// and its offset is estimated afresh, so its anchors' ages stay its own delays.
    #[test]
    fn an_anchors_age_is_its_delivery_delay_once_the_pages_clock_is_known() {
        let start = Instant::now();
        let mut clock = PageClock::default();
        let at = |milliseconds: f64| start + Duration::from_secs_f64(milliseconds / 1e3);
        let age = |clock: &mut PageClock, composed: f64, arrived: f64| {
            clock.age(Some(composed), at(arrived)).as_secs_f64() * 1e3
        };
        // The page's clock reads 10,000 more than this one; the first anchor took 4 ms to arrive.
        assert_eq!(age(&mut clock, 10_000.0, 4.0), 0.0);
        // Composed 100 ms later, arriving 24 ms after that: 20 ms more delay than the quickest.
        assert!((age(&mut clock, 10_100.0, 124.0) - 20.0).abs() < 1e-6);
        // Delivered in 2 ms, quicker than any before: the offset falls and its own age is zero.
        assert_eq!(age(&mut clock, 10_200.0, 202.0), 0.0);
        assert!((age(&mut clock, 10_300.0, 307.0) - 5.0).abs() < 1e-6);
        // Two anchors crossing by a millisecond: the later stamp is not a restarted page.
        assert!((age(&mut clock, 10_299.0, 308.0) - 7.0).abs() < 1e-6);
        // An anchor the page did not stamp is placed where it was composed.
        for unstamped in [None, Some(f64::NAN), Some(f64::INFINITY), Some(-1.0)] {
            assert_eq!(clock.age(unstamped, at(400.0)), Duration::ZERO);
        }
        // The page reloaded: its clock starts again, and the stale offset would age every anchor
        // by ten seconds.
        assert_eq!(age(&mut clock, 2.0, 500.0), 0.0);
        assert!((age(&mut clock, 102.0, 615.0) - 15.0).abs() < 1e-6);
    }
}
