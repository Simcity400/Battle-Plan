//! The rate the map's simulation holds: how often the playhead's step is taken, every layer's
//! product made and the picture published. It is the simulation's own measure of what it can
//! make, not the display's refresh, so a faster display asks the simulation for nothing more.
//!
//! [`PUBLICATIONS`] a second is the rate it aims at, capped by the display's refresh because a
//! publication no frame draws is work for nobody; [`Pace`] holds it, and a step that overruns its
//! interval leaves the next due at once, so the rate falls to what can be made instead of being
//! caught up in a burst. [`Reach`] is how long a publication takes to reach the screen, which the
//! simulation prepares each one ahead of the head by, together with the interval it will be drawn
//! across, so that every display frame that draws it has its head inside the hours it published.

use std::time::{Duration, Instant};

/// Publications a second the simulation aims at. Discrete state is an event a watcher has no
/// reference for except the clock on screen, and above about thirty changes a second in one place
/// a person sees blur rather than information: thirty puts every change on screen inside the 33 ms
/// a watcher is owed, and publishing more often buys nothing a person can see while the cores
/// follow the rate.
pub const PUBLICATIONS: f64 = 30.0;

/// The rate the simulation publishes at: [`PUBLICATIONS`], or the display's refresh while that is
/// lower.
pub fn rate() -> f64 {
    held(super::display_refresh::fastest())
}

/// One publication's interval at [`rate`].
pub fn interval() -> Duration {
    Duration::from_secs_f64(1.0 / rate())
}

/// The rate held under a display of `refresh` hertz.
fn held(refresh: f64) -> f64 {
    PUBLICATIONS.min(refresh)
}

/// When the next publication is due.
#[derive(Clone, Copy, Debug)]
pub struct Pace {
    due: Instant,
}

impl Pace {
    /// A pace whose first publication is due at `now`.
    pub fn new(now: Instant) -> Self {
        Self { due: now }
    }

    /// How long a publication due now has already waited; none while the next is not due yet. A
    /// publication waits past its due time for an hour to show, and the wait is bounded by this.
    pub fn late(&self, now: Instant) -> Option<Duration> {
        (now >= self.due).then(|| now.duration_since(self.due))
    }

    /// The publication due is taken at `now`. The grid holds while one starts less than an
    /// interval after it was due, so one delayed by a frame or by the hours it waited for does not
    /// move every one after it; one that starts further behind than that starts the grid again
    /// from now, so publications that run long are never made back to back to catch up.
    pub fn take(&mut self, now: Instant, interval: Duration) {
        self.due = if now.saturating_duration_since(self.due) < interval { self.due + interval } else { now };
    }

    /// How long until the next is due; zero while one is due.
    pub fn until(&self, now: Instant) -> Duration {
        self.due.saturating_duration_since(now)
    }
}

/// How long a publication takes to reach the screen: made on the simulation's side and taken by
/// the display frame. The simulation prepares each publication for the hour the head will stand at
/// that long from now, so that what a watcher sees stands at the head's own hour instead of one
/// whole making behind it. It never looks more than an interval ahead: a making that takes longer
/// than that cannot be caught up by looking further, and the picture stands at the head's hour
/// instead of running ahead of the clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct Reach {
    held: Duration,
}

impl Reach {
    /// A publication reached the screen `took` after it began, at a rate of one an `interval`.
    pub fn took(&mut self, took: Duration, interval: Duration) {
        self.held = ((self.held * 7 + took) / 8).min(interval);
    }

    /// How far ahead of the head a publication begun now is prepared: what the one before took to
    /// reach the screen, and the whole `interval` beyond it, because the frames that draw it are
    /// every frame until the next publication, and the head of the last of them stands an interval
    /// past the head of the first. A frame whose head stands past the hours published has nothing
    /// to place a mover between: it holds the last place published and the motion stands still.
    pub fn ahead(self, interval: Duration) -> Duration {
        self.held + interval
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The simulation's rate is its own: [`PUBLICATIONS`] a second whatever the display's refresh,
    /// and the display's only while that is the slower of the two.
    #[test]
    fn the_rate_is_the_simulations_own_and_never_above_the_displays() {
        for refresh in [60.0, 120.0, 144.0, 240.0, 360.0] {
            assert_eq!(held(refresh), PUBLICATIONS, "a {refresh} Hz display");
        }
        assert_eq!(held(24.0), 24.0, "never more publications than the display draws frames");
    }

    /// Asked far more often than its rate, the pace holds one publication an interval; a
    /// publication a few milliseconds late keeps the rate instead of pushing every one after it
    /// along, and one that overruns a whole interval starts the grid again rather than making the
    /// ones it covered back to back.
    #[test]
    fn the_pace_holds_one_publication_an_interval_and_never_catches_up_in_a_burst() {
        let interval = Duration::from_millis(10);
        let start = Instant::now();
        let mut pace = Pace::new(start);
        let mut taken = 0;
        // Asked every millisecond for a second: a hundred publications, not a thousand.
        for ms in 0..1_000 {
            let now = start + Duration::from_millis(ms);
            if pace.late(now).is_some() {
                pace.take(now, interval);
                taken += 1;
            }
        }
        assert_eq!(taken, 100);
        assert_eq!(pace.until(start + Duration::from_millis(999)), Duration::from_millis(1));
        // Every publication starts 3 ms after it was due, having waited for an hour to show:
        // still a hundred in the second, because the lateness is not added to the interval.
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
        // One publication held the side for 95 ms: the nine it covered are not made afterwards.
        let behind = start + Duration::from_millis(95);
        assert_eq!(pace.late(behind), Some(Duration::from_millis(85)));
        pace.take(behind, interval);
        assert_eq!(pace.late(behind), Some(Duration::ZERO), "behind, the next is due at once");
        pace.take(behind, interval);
        assert_eq!(pace.late(behind), None, "and only the one");
        assert_eq!(pace.until(start + Duration::from_millis(100)), Duration::from_millis(5));
    }

    /// The reach follows what the publications before took to reach the screen and looks the
    /// interval it will be drawn across beyond that, so every frame that draws a publication has
    /// its head inside the hours it published; a making that cannot be done in time adds no more
    /// than one interval of its own.
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
