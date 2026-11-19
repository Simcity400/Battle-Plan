//! The frames a second the app's setting holds every map to, and the grid of deadlines a map's
//! frames stand on under it.
//!
//! The limit is the rate the map presents at while it is below the display's, so it is also the
//! rate the frame is budgeted for and the rate the simulation is capped by: it reaches both
//! through [`super::display_refresh::hz`], the one rate a map frame is paced at. Off, nothing here
//! is in a frame's way.
//!
//! [`Grid`] is how a map holds the rate: each frame falls due an interval after the one before it
//! was due, so the frames stand evenly rather than being slept between, and a frame that begins
//! more than an interval after it was due starts the grid again from itself, so a late frame is
//! drawn late and is never followed by one drawn at once to catch the grid up.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

/// Frames a second every map presents at most; 0 for no limit.
static LIMIT: AtomicU64 = AtomicU64::new(0);

/// Holds every map to `hz` frames a second from its next frame on; `None` lifts the limit.
pub fn limit_to(hz: Option<f64>) {
    LIMIT.store(held(hz).map_or(0, f64::to_bits), Ordering::Release);
}

/// The frames a second the maps are held to; none while they draw as the display allows.
pub fn limit() -> Option<f64> {
    let held = LIMIT.load(Ordering::Acquire);
    (held != 0).then(|| f64::from_bits(held))
}

/// The limit a rate asks for: one outside [`dashboard_core::protocol::MAP_FRAME_LIMIT_RANGE`] is
/// no limit rather than a rate no map could draw. The protocol and the harness's own flag refuse
/// such a rate by name before it reaches here.
fn held(hz: Option<f64>) -> Option<f64> {
    hz.filter(|hz| dashboard_core::protocol::MAP_FRAME_LIMIT_RANGE.contains(hz))
}

/// One frame's interval under the limit in force.
fn interval() -> Option<Duration> {
    limit().map(|hz| Duration::from_secs_f64(1.0 / hz))
}

/// One map's grid of frame deadlines. Each map draws its frames on a thread of its own, so each
/// keeps its own grid of the one limit.
#[derive(Clone, Copy, Debug, Default)]
pub struct Grid {
    /// When the next frame falls due; none before the first frame and while no limit is in force.
    due: Option<Instant>,
}

impl Grid {
    /// How long a frame wanted at `now` waits before it may be drawn; zero while it may be drawn
    /// now, and zero whatever the grid holds once the limit is lifted.
    pub fn until(&self, now: Instant) -> Duration {
        match (limit(), self.due) {
            (Some(_), Some(due)) => due.saturating_duration_since(now),
            _ => Duration::ZERO,
        }
    }

    /// The frame wanted at `now` is being drawn: the next falls due an interval past this one's
    /// own due time.
    pub fn take(&mut self, now: Instant) {
        self.due = interval().map(|interval| stepped(self.due, now, interval));
    }
}

/// Where the frame after one begun at `now`, which was due at `due`, falls due: an interval past
/// that due time, so the frames stand evenly on one grid whatever each costs; a frame begun a
/// whole interval or more after it was due starts the grid again from itself, so the one after a
/// late frame is a whole interval away instead of being drawn at once to catch the grid up.
fn stepped(due: Option<Instant>, now: Instant, interval: Duration) -> Instant {
    match due {
        Some(due) if now.saturating_duration_since(due) < interval => due + interval,
        _ => now + interval,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A limit below one frame a second, or above any rate a display shows, is no limit, and no
    /// rate at all is none either.
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

    /// The deadlines stand on one grid whatever each frame costs, so the intervals a watcher sees
    /// are the limit's own and not the limit plus a frame's work.
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
            // Each frame costs a different part of its interval; the next still falls due on the grid.
            at += interval / 4 * (frame % 3 + 1);
            let waited = due.unwrap().saturating_duration_since(at);
            assert!(!waited.is_zero(), "frame {frame} spent less than its interval");
            at += waited;
        }
        for (frame, pair) in begun.windows(2).enumerate() {
            assert_eq!(pair[1] - pair[0], interval, "frames {frame} and {}", frame + 1);
        }
    }

    /// A frame that overran is drawn late, and the one after it is not drawn at once to catch the
    /// grid up: a frame still inside its interval keeps the grid, one past it starts the grid again.
    #[test]
    fn a_frame_a_whole_interval_late_starts_the_grid_again_instead_of_being_caught_up() {
        let interval = Duration::from_secs_f64(1.0 / 60.0);
        let origin = Instant::now();
        let first = stepped(None, origin, interval);
        assert_eq!(first, origin + interval);
        // A frame a third of an interval late keeps the grid: the next stands where it always would.
        let late = first + interval / 3;
        let kept = stepped(Some(first), late, interval);
        assert_eq!(kept, first + interval);
        assert_eq!(kept.saturating_duration_since(late), interval - interval / 3);
        // One a whole interval or more late starts the grid again, a whole interval from itself.
        let overran = kept + interval * 2;
        assert_eq!(stepped(Some(kept), overran, interval), overran + interval);
        assert_eq!(stepped(Some(kept), kept + interval, interval), kept + interval * 2);
    }

    /// A grid holding nothing lets the frame through, and one whose limit was lifted lets it
    /// through whatever deadline it still holds.
    #[test]
    fn an_unheld_grid_and_a_lifted_limit_both_let_the_frame_through() {
        let now = Instant::now();
        assert_eq!(Grid::default().until(now), Duration::ZERO);
        let held = Grid { due: Some(now + Duration::from_secs(10)) };
        assert_eq!(limit(), None, "no test sets the process-wide limit");
        assert_eq!(held.until(now), Duration::ZERO);
    }
}
