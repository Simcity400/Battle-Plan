use super::Playhead;
use std::{
    fmt,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Mode {
    Nominal(f64),
    Wall(f64),
}
impl Mode {
    pub(super) fn parse(text: &str) -> Option<Self> {
        if let Some(speed) = text.strip_prefix("playwall:") {
            let speed = speed.parse::<f64>().ok()?;
            return (speed.is_finite() && speed > 0.0).then_some(Self::Wall(speed));
        }
        text.strip_prefix("play:")?.parse().ok().map(Self::Nominal)
    }
    pub(super) fn speed(self) -> f64 {
        match self {
            Self::Nominal(speed) | Self::Wall(speed) => speed,
        }
    }
    fn wall(self) -> bool {
        matches!(self, Self::Wall(_))
    }
}

#[derive(Default)]
pub(super) struct Clock {
    mode: Option<Mode>,
    start: Option<Instant>,
    last: Option<Instant>,
    elapsed: Duration,
}

pub(super) struct Sample {
    pub(super) now: Instant,
    pub(super) seconds: f64,
}
impl Clock {
    pub(super) fn configure(&mut self, mode: Option<Mode>, active: bool) {
        let mode = mode.filter(|_| active);
        if mode != self.mode {
            *self = Self {
                mode,
                ..Self::default()
            };
        }
    }
    pub(super) fn wall_active(&self) -> bool {
        self.mode.is_some_and(Mode::wall)
    }
    pub(super) fn sample(&mut self, now: impl FnOnce() -> Instant) -> Option<Sample> {
        self.mode?;
        let now = now();
        let start = *self.start.get_or_insert(now);
        let seconds = self
            .last
            .map_or(0.0, |last| now.duration_since(last).as_secs_f64());
        self.last = Some(now);
        self.elapsed = now.duration_since(start);
        Some(Sample { now, seconds })
    }
    pub(super) fn stamp(&self, head: Option<Playhead>, active: bool) -> Stamp {
        Stamp(
            self.mode
                .zip(head)
                .filter(|_| self.last.is_some())
                .map(|(mode, head)| (mode, head, self.elapsed, active)),
        )
    }
}

pub(super) struct Stamp(Option<(Mode, Playhead, Duration, bool)>);
impl fmt::Display for Stamp {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some((mode, head, elapsed, active)) = self.0 {
            write!(
                out,
                "playback_mode={} playhead_branch={} playhead_hour={} playhead_fraction={:.17} playback_ns={} playback_speed={} playback_active={}",
                if mode.wall() { "wall" } else { "nominal" },
                head.branch,
                head.hour,
                head.fraction,
                elapsed.as_nanos(),
                mode.speed(),
                u8::from(active)
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tick_clock;

    fn head() -> Playhead {
        Playhead {
            branch: 7,
            hour: 100,
            fraction: 0.25,
        }
    }

    #[test]
    fn modes_keep_nominal_and_require_a_finite_positive_wall_rate() {
        assert_eq!(Mode::parse("play:1440"), Some(Mode::Nominal(1440.0)));
        assert_eq!(Mode::parse("playwall:1440.25"), Some(Mode::Wall(1440.25)));
        assert_eq!(Mode::parse("play:0"), Some(Mode::Nominal(0.0)));
        assert_eq!(Mode::parse("play:-1"), Some(Mode::Nominal(-1.0)));
        assert!(matches!(Mode::parse("play:NaN"), Some(Mode::Nominal(speed)) if speed.is_nan()));
        assert_eq!(Mode::parse("play:inf"), Some(Mode::Nominal(f64::INFINITY)));
        for text in [
            "playwall:0",
            "playwall:-1",
            "playwall:NaN",
            "playwall:inf",
            "playwall:-inf",
            "playwall:1e309",
            "playwall:no",
            "playwall:",
            "pan:300",
        ] {
            assert_eq!(Mode::parse(text), None, "{text}");
        }
        assert_eq!(Mode::Wall(4.5).speed(), 4.5);
        assert_eq!(Mode::Nominal(6.0).speed(), 6.0);
    }

    #[test]
    fn disabled_and_warmup_clocks_do_no_work_and_each_sweep_starts_fresh() {
        let mut clock = Clock::default();
        clock.configure(Some(Mode::Wall(1440.0)), false);
        assert!(!clock.wall_active());
        assert!(
            clock
                .sample(|| panic!("warmup sampled the clock"))
                .is_none()
        );
        assert_eq!(clock.stamp(Some(head()), false).to_string(), "");
        let origin = Instant::now();
        clock.configure(Some(Mode::Wall(1440.0)), true);
        assert!(clock.wall_active());
        assert!(clock.last.is_none());
        assert_eq!(clock.stamp(Some(head()), true).to_string(), "");
        assert_eq!(clock.sample(|| origin).unwrap().seconds, 0.0);
        assert_eq!(
            clock
                .sample(|| origin + Duration::from_secs(9))
                .unwrap()
                .seconds,
            9.0
        );
        clock.configure(None, false);
        assert!(!clock.wall_active());
        assert!(
            clock
                .sample(|| panic!("disabled sampled the clock"))
                .is_none()
        );
        assert_eq!(clock.stamp(Some(head()), false).to_string(), "");
        clock.configure(Some(Mode::Wall(1440.0)), true);
        assert_eq!(
            clock
                .sample(|| origin + Duration::from_secs(20))
                .unwrap()
                .seconds,
            0.0
        );
        assert_eq!(clock.elapsed, Duration::ZERO);
    }

    #[test]
    fn an_unchanged_mode_keeps_its_origin_but_a_changed_rate_or_source_resets_it() {
        let origin = Instant::now();
        let mut clock = Clock::default();
        clock.configure(Some(Mode::Wall(4.0)), true);
        clock.sample(|| origin).unwrap();
        clock.configure(Some(Mode::Wall(4.0)), true);
        let now = origin + Duration::from_secs(3);
        let sample = clock.sample(|| now).unwrap();
        assert_eq!(sample.now, now);
        assert_eq!(sample.seconds, 3.0);
        assert_eq!(clock.elapsed, Duration::from_secs(3));
        clock.configure(Some(Mode::Wall(8.0)), true);
        assert!(clock.wall_active());
        assert_eq!(clock.sample(|| now).unwrap().seconds, 0.0);
        assert_eq!(clock.elapsed, Duration::ZERO);
        clock.configure(Some(Mode::Nominal(8.0)), true);
        assert!(!clock.wall_active());
        assert_eq!(
            clock
                .sample(|| now + Duration::from_secs(2))
                .unwrap()
                .seconds,
            0.0
        );
        assert_eq!(clock.elapsed, Duration::ZERO);
        assert_eq!(clock.stamp(None, true).to_string(), "");
    }

    #[test]
    fn injected_wall_stalls_advance_the_shared_tick_without_a_frame_step_or_cap() {
        let origin = Instant::now();
        let mut clock = Clock::default();
        clock.configure(Some(Mode::Wall(1440.0)), true);
        let mut head = head();
        for (millis, hour) in [(0, 100), (125, 280), (375, 640), (1000, 1540)] {
            let sample = clock
                .sample(|| origin + Duration::from_millis(millis))
                .unwrap();
            assert!(clock.wall_active());
            assert!(tick_clock(&mut head, 1440.0, sample.seconds, 10_000));
            assert_eq!((head.branch, head.hour, head.fraction), (7, hour, 0.25));
        }
        assert_eq!(clock.elapsed, Duration::from_secs(1));
        assert_eq!(
            clock.stamp(Some(head), true).to_string(),
            "playback_mode=wall playhead_branch=7 playhead_hour=1540 playhead_fraction=0.25000000000000000 playback_ns=1000000000 playback_speed=1440 playback_active=1"
        );
        let sample = clock.sample(|| origin + Duration::from_secs(10)).unwrap();
        assert_eq!(sample.seconds, 9.0);
        assert!(!tick_clock(&mut head, 1440.0, sample.seconds, 2000));
        assert_eq!((head.hour, head.fraction), (2000, 0.0));
        assert_eq!(
            clock.stamp(Some(head), false).to_string(),
            "playback_mode=wall playhead_branch=7 playhead_hour=2000 playhead_fraction=0.00000000000000000 playback_ns=10000000000 playback_speed=1440 playback_active=0"
        );
    }

    #[test]
    fn nominal_clock_stamps_do_not_turn_fixed_steps_into_wall_steps() {
        let origin = Instant::now();
        let mut clock = Clock::default();
        clock.configure(Some(Mode::Nominal(1440.0)), true);
        let mut head = head();
        for (secs, hour) in [(0, 106), (1, 112), (4, 118)] {
            clock.sample(|| origin + Duration::from_secs(secs)).unwrap();
            assert!(!clock.wall_active());
            assert!(tick_clock(&mut head, 1440.0, 1.0 / 240.0, 10_000));
            assert_eq!((head.hour, head.fraction), (hour, 0.25));
        }
        assert_eq!(clock.elapsed, Duration::from_secs(4));
        assert_eq!(
            clock.stamp(Some(head), true).to_string(),
            "playback_mode=nominal playhead_branch=7 playhead_hour=118 playhead_fraction=0.25000000000000000 playback_ns=4000000000 playback_speed=1440 playback_active=1"
        );
    }
}
