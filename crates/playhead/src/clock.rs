//! The head between anchors. A seek holds the head at the hour sought until its reset is taken,
//! and the clock runs from that moment: the hour sought is the first one shown. A pause or a speed
//! asked for during a seek applies from then.

use replay_data::Hour;
use std::time::Instant;

#[derive(Clone, Copy, Debug)]
pub struct Clock {
    hour: Hour,
    /// When the head was at `hour`; none while held at a seek.
    since: Option<Instant>,
    speed: f64,
    playing: bool,
    /// The seek whose reset is awaited, and the last frame that asked the head meanwhile.
    held: Option<u64>,
    asked: Option<Instant>,
    first: Hour,
    last: Hour,
}

impl Clock {
    /// Paused at `first`, of hours `first` to `last`.
    pub fn new(first: Hour, last: Hour, speed: f64) -> Self {
        Self { hour: first, since: Some(Instant::now()), speed, playing: false, held: None, asked: None, first, last }
    }

    pub fn head(&self, now: Instant) -> Hour {
        match self.since {
            Some(since) if self.playing => {
                let moved = now.saturating_duration_since(since).as_secs_f64() * self.speed;
                (f64::from(self.hour) + moved).min(f64::from(self.last)) as Hour
            }
            _ => self.hour,
        }
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    /// The seek whose reset is awaited, if any.
    pub fn held(&self) -> Option<u64> {
        self.held
    }

    /// [`Self::head`] asked by a frame at `now`: while held, the moment the clock will run from
    /// when that frame takes the reset.
    pub fn frame(&mut self, now: Instant) -> Hour {
        if self.held.is_some() {
            self.asked = Some(now);
        }
        self.head(now)
    }

    /// Held at `hour` until the reset of seek `epoch` is taken; a seek under way is replaced.
    pub fn seek(&mut self, hour: Hour, epoch: u64) {
        self.hour = hour.clamp(self.first, self.last);
        self.since = None;
        self.held = Some(epoch);
        self.asked = None;
    }

    pub fn play(&mut self, speed: f64, now: Instant) {
        self.settle(now);
        self.speed = speed;
        self.playing = true;
    }

    pub fn pause(&mut self, now: Instant) {
        self.settle(now);
        self.playing = false;
    }

    /// The reset of seek `epoch` is taken at `now`: the clock runs from the hour sought, from the
    /// frame that took it (the last that asked the head), or from `now`. A reset of any other seek
    /// changes nothing.
    pub fn shown(&mut self, epoch: u64, now: Instant) {
        if self.held == Some(epoch) {
            self.held = None;
            self.since = Some(self.asked.take().filter(|asked| *asked <= now).unwrap_or(now));
        }
    }

    fn settle(&mut self, now: Instant) {
        if self.since.is_some() {
            self.hour = self.head(now);
            self.since = Some(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Clock;
    use std::time::{Duration, Instant};

    #[test]
    fn a_seek_holds_the_head_until_its_reset_is_shown_and_runs_from_it() {
        let start = Instant::now();
        let at = |seconds: f64| start + Duration::from_secs_f64(seconds);
        let mut clock = Clock::new(0, 100_000, 1440.0);
        clock.play(1440.0, at(0.0));
        assert_eq!(clock.head(at(1.0)), 1440);
        clock.seek(5000, 7);
        assert_eq!(clock.head(at(1.0)), 5000);
        assert_eq!(clock.head(at(9.0)), 5000, "held however long the seek takes");
        clock.shown(6, at(9.0));
        assert_eq!(clock.head(at(10.0)), 5000, "another seek's reset does not start it");
        clock.shown(7, at(10.0));
        assert_eq!(clock.head(at(10.0)), 5000);
        assert_eq!(clock.head(at(10.5)), 5720);
    }

    #[test]
    fn the_clock_runs_from_the_frame_that_asked_the_head_and_took_the_reset() {
        let start = Instant::now();
        let at = |seconds: f64| start + Duration::from_secs_f64(seconds);
        let mut clock = Clock::new(0, 100_000, 24.0);
        clock.play(24.0, at(0.0));
        clock.seek(900, 1);
        assert_eq!(clock.frame(at(1.0)), 900);
        assert_eq!(clock.frame(at(2.0)), 900);
        clock.shown(1, at(2.3));
        assert_eq!(clock.head(at(3.0)), 924, "a second after the frame, not after the take");
    }

    #[test]
    fn a_seek_during_a_seek_replaces_it_and_holds_at_the_newer_hour() {
        let start = Instant::now();
        let at = |seconds: f64| start + Duration::from_secs_f64(seconds);
        let mut clock = Clock::new(0, 100_000, 1440.0);
        clock.play(1440.0, at(0.0));
        clock.seek(5000, 1);
        clock.seek(9000, 2);
        assert_eq!(clock.head(at(3.0)), 9000);
        clock.shown(1, at(3.0));
        assert_eq!(clock.head(at(4.0)), 9000, "the replaced seek's reset does not start it");
        clock.shown(2, at(4.0));
        assert_eq!(clock.head(at(5.0)), 10_440);
    }

    #[test]
    fn a_pause_or_speed_during_a_seek_applies_once_its_reset_is_shown() {
        let start = Instant::now();
        let at = |seconds: f64| start + Duration::from_secs_f64(seconds);
        let mut clock = Clock::new(0, 100_000, 1440.0);
        clock.play(1440.0, at(0.0));
        clock.seek(2000, 1);
        clock.play(24.0, at(0.5));
        assert_eq!(clock.head(at(2.0)), 2000);
        clock.shown(1, at(2.0));
        assert_eq!(clock.head(at(3.0)), 2024);
        clock.seek(3000, 2);
        clock.pause(at(3.5));
        clock.shown(2, at(4.0));
        assert_eq!(clock.head(at(60.0)), 3000);
        assert_eq!(clock.head(at(1e6)), 3000);
        clock.play(1440.0, at(60.0));
        assert_eq!(clock.head(at(1e6)), 100_000, "never past the last hour");
    }
}
