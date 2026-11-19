//! The one head of a display frame: the hour of the replay everything it draws stands at.
//!
//! The playback clock says where playback is. The simulation publishes the hours it has read from
//! the recording a little ahead of it ([`super::simulation_rate::Reach`]), so the hours a frame can
//! draw from real data end at the newest hour published and reach back over the publications the
//! frame still holds. Within that, a display frame places every mover between the places two
//! publications gave it, writes the change sets of the hours it has reached, and reports an hour on
//! screen. All of them take this one head, so nothing on screen stands at an hour something else on
//! screen does not.
//!
//! The head follows the clock, so the replay's hours advance exactly as evenly as playback does,
//! and is steered by a bounded correction ([`PULL`]): it is slowed while the newest hour published
//! stands less than [`KEEP`] of an interval above it, so it does not run past the newest placement
//! when a publication is late, and it never falls further behind the clock than [`BEHIND_MOST`],
//! because past that a mover held at its last published place is the lesser fault. It never goes
//! backwards and never holds: a hold is a visible stall and the jump after it a visible skip, where
//! a correction of an eighth of the playback rate is not something a watcher can see. A pause, a
//! seek, a speed change and a saved frame take the clock's own head exactly, on the frame they
//! reach, so a stopped picture is exact at the hour it shows.
//!
//! The newest hour published is a staircase: it steps up by a publication's hours and is then still
//! while the head advances. [`KEEP`] binds only where the head would otherwise pass the newest
//! placement, not on the ordinary low point of that staircase. No constant delay is taken: a
//! publication's places reach back from its own hour, which is above the head, so a head pulled
//! further back would stand before the oldest of them.

use super::{anchored::Head, simulation_rate};

/// How far below the newest hour published the head is slowed to keep, in publication intervals.
pub const KEEP: f64 = 0.125;

/// The most the head may stand behind the playback clock, in publication intervals; past that a
/// mover held at its last published place is the lesser fault.
pub const BEHIND_MOST: f64 = 1.0;

/// The largest share of the playback rate the correction adds or takes, reached at an error of one
/// publication interval.
pub const PULL: f64 = 0.125;

/// A clock that moved further than this many publication intervals in one frame is a seek, a branch
/// change or a jump of the host's anchor, not playback: the head takes it exactly.
const JUMPED: f64 = 4.0;

/// The head of the display frames of one view, steered between the frames it is asked for.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameHead {
    /// The hour the frame before showed, as an absolute replay hour; none before the first frame
    /// and after a seek.
    at: Option<f64>,
    /// The clock at that frame, which gives the hours playback itself advanced since.
    clock: f64,
}

/// What a display frame knows when it takes its head.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// Where playback stands: the clock's own head, its fraction included.
    pub clock: f64,
    /// The newest hour published as real data: the hour of the newest publication the frame holds.
    /// None before the first, which leaves the clock's own head.
    pub published: Option<i32>,
    /// Replay hours a second of playback, for the hours of one publication interval.
    pub speed: f64,
    /// The hours are moving and the picture is a live one. A pause, a speed of zero, a seek's first
    /// picture, the end of a recording and a saved frame are not moving: they take the clock
    /// exactly, so the picture a watcher is left looking at is exact at the hour it shows.
    pub moving: bool,
}

impl FrameHead {
    /// The head this frame draws everything at.
    pub fn take(&mut self, frame: Frame) -> Head {
        let interval = interval_hours(frame.speed);
        let advance = frame.clock - self.clock;
        let steady = frame.moving && interval > 0.0 && (0.0..interval * JUMPED).contains(&advance);
        let Some(at) = self.at.filter(|_| steady) else {
            return self.stand(frame.clock);
        };
        // Never past the hours real data covers, never further behind the clock than the delay the
        // picture is allowed, and never past the clock itself: playback is the authority on where
        // the replay stands.
        let target = frame
            .published
            .map_or(frame.clock, |hour| (f64::from(hour) - interval * KEEP).min(frame.clock))
            .max(frame.clock - interval * BEHIND_MOST);
        let error = ((target - at) / interval).clamp(-1.0, 1.0);
        let moved = (at + advance * (1.0 + error * PULL)).clamp(at, frame.clock);
        if super::hour_check::on() {
            judge(at, moved, advance, frame.clock, interval);
        }
        self.clock = frame.clock;
        self.at = Some(moved);
        split(moved)
    }

    /// The head stands exactly where playback does, and the frames after it follow from there.
    fn stand(&mut self, clock: f64) -> Head {
        self.clock = clock;
        self.at = Some(clock);
        split(clock)
    }

    /// The history this head followed is gone (a seek, a branch change, a reader reopened): the
    /// next frame stands at the clock rather than steering from an hour of another history.
    pub fn forget(&mut self) {
        self.at = None;
        self.clock = 0.0;
    }

    /// Where it stands, as an absolute replay hour; none before its first frame.
    pub fn hour(&self) -> Option<f64> {
        self.at
    }
}

/// `BATTLE_PLAN_VIEWER_CHECK_HOUR` on the head itself: what a watcher is owed of it, whatever the
/// publications do. It never goes backwards and never holds while playback moves (a hold is a
/// visible stall and the jump after it a visible skip); it never advances by more than playback
/// did, plus the correction; and it never stands further behind the clock than [`BEHIND_MOST`] of
/// an interval, so the delay is a bound and not a drift.
fn judge(was: f64, now: f64, advance: f64, clock: f64, interval: f64) {
    super::hour_check::judged();
    let step = now - was;
    if advance > 0.0 && step <= 0.0 {
        super::hour_check::found("the frame's head", format!("held at {was} while playback advanced {advance} hours"));
    }
    if step > advance * (1.0 + PULL) + 1e-6 {
        super::hour_check::found("the frame's head", format!("advanced {step} hours where playback advanced {advance}"));
    }
    if clock - now > interval * BEHIND_MOST + advance + 1e-6 {
        super::hour_check::found(
            "the frame's head",
            format!("stands {} hours behind the clock, past the {} it is allowed", clock - now, interval * BEHIND_MOST),
        );
    }
}

/// The hour a frame at `head` has on screen: past the newest hour `published`, every mover holds
/// its last place and every timed layer its last set, so the picture is that hour's.
pub fn shown(head: Head, published: Option<i32>) -> Head {
    match published {
        Some(hour) if head.hour > hour || (head.hour == hour && head.fraction > 0.0) => Head { hour, fraction: 0.0 },
        _ => head,
    }
}

/// The hours of one publication interval at `speed` replay hours a second.
fn interval_hours(speed: f64) -> f64 {
    if speed > 0.0 {
        speed * simulation_rate::interval().as_secs_f64()
    } else {
        0.0
    }
}

/// An absolute replay hour as the whole hour and the fraction into it. The hour is never held as an
/// `f32`: a replay hour is about sixty million, where neighbouring `f32` values are four hours
/// apart ([`Head`]).
fn split(at: f64) -> Head {
    let hour = at.floor();
    Head { hour: hour as i32, fraction: (at - hour) as f32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEED: f64 = 1440.0;
    const CLOCK: f64 = 60_800_000.0;

    fn interval() -> f64 {
        interval_hours(SPEED)
    }

    /// A frame of a 240 Hz display at the top speed, with the newest publication prepared the
    /// reach ahead of the clock as the simulation prepares it.
    fn frame(clock: f64, published: f64) -> Frame {
        Frame { clock, published: Some(published as i32), speed: SPEED, moving: true }
    }

    /// A head that ran past the newest hour published shows that hour, not its own: nothing on
    /// screen moved past it. Before, the map reported the head's hour while its movers held still.
    #[test]
    fn a_head_past_the_newest_hour_published_shows_that_hour() {
        let head = Head { hour: 150, fraction: 0.5 };
        assert_eq!(shown(head, Some(148)), Head { hour: 148, fraction: 0.0 });
        assert_eq!(shown(Head { hour: 148, fraction: 0.25 }, Some(148)), Head { hour: 148, fraction: 0.0 });
        assert_eq!(shown(head, Some(160)), head);
        assert_eq!(shown(head, None), head);
    }

    /// Playing, with the publications landing on time, the head is the clock's own: the hours it
    /// shows advance by exactly what playback advanced, frame after frame, and the gate's "6 game
    /// hours a frame at 240 Hz" is the clock's arithmetic and not an average.
    #[test]
    fn the_head_advances_by_the_hours_playback_advanced_while_the_publications_land() {
        let mut head = FrameHead::default();
        let reach = interval() * 2.2;
        let mut clock = CLOCK;
        let mut shown = Vec::new();
        for _ in 0..240 {
            shown.push(head.take(frame(clock, clock + reach)));
            clock += SPEED / 240.0;
        }
        let hours: Vec<f64> = shown
            .windows(2)
            .map(|pair| f64::from(pair[1].hour - pair[0].hour) + f64::from(pair[1].fraction) - f64::from(pair[0].fraction))
            .collect();
        assert!(hours.iter().all(|step| (step - 6.0).abs() < 1e-6), "{hours:?}");
        assert_eq!(shown[0].hour, CLOCK as i32, "the first frame stands at the clock");
    }

    /// A publication that does not land leaves the newest hour published standing while the clock
    /// runs on. The head slows, by an eighth of the playback rate at the most, so it stays inside
    /// the hours real data covers instead of running past the newest placement; and it never holds,
    /// which a watcher would see as a stall and the catch-up after it as a skip.
    #[test]
    fn a_late_publication_slows_the_head_and_never_stops_it() {
        let mut head = FrameHead::default();
        let mut clock = CLOCK;
        // The hours published end within the place the head keeps: a publication that has not
        // landed, or a reach that prepares one barely ahead of the head.
        let published = clock + interval() * 0.1;
        head.take(frame(clock, published));
        let mut least = f64::INFINITY;
        let mut shown = head.hour().expect("the first frame stood");
        for _ in 0..8 {
            clock += SPEED / 240.0;
            let at = head.take(frame(clock, published));
            let now = f64::from(at.hour) + f64::from(at.fraction);
            let step = now - shown;
            least = least.min(step);
            assert!(step > 0.0, "the head never holds: {step}");
            shown = now;
        }
        assert!(least < 6.0, "the head slowed: {least}");
        assert!(least >= 6.0 * (1.0 - PULL) - 1e-6, "by no more than an eighth: {least}");
        assert!(shown < clock, "and so stands behind the clock it was keeping up with: {shown} against {clock}");
    }

    /// However long a publication stays away, the head stands no further behind the clock than the
    /// one interval: past that a mover held at its last published place is the
    /// lesser fault, and the picture's own delay is bounded.
    #[test]
    fn the_head_never_falls_further_behind_the_clock_than_one_interval() {
        let mut head = FrameHead::default();
        let mut clock = CLOCK;
        let published = clock + interval() * 2.2;
        for _ in 0..2_000 {
            head.take(frame(clock, published));
            clock += SPEED / 240.0;
        }
        let behind = clock - head.hour().expect("a head");
        assert!(behind <= interval() * BEHIND_MOST + 6.0, "{behind} hours behind");
    }

    /// The hours stopped (a pause, a speed of zero, the end of the recording) or a saved frame: the
    /// head is the clock's own, exactly, on the frame it reaches, and the picture a watcher is left
    /// looking at stands at the hour it shows.
    #[test]
    fn a_stopped_picture_stands_exactly_at_the_clock() {
        let mut head = FrameHead::default();
        let mut clock = CLOCK;
        let published = clock + interval() * 2.2;
        // Playing behind a late publication, so the head has a delay to answer with.
        for _ in 0..240 {
            head.take(frame(clock, published));
            clock += SPEED / 240.0;
        }
        assert_ne!(head.hour(), Some(clock), "the head had fallen behind");
        let at = head.take(Frame { moving: false, ..frame(clock, published) });
        assert_eq!((at.hour, at.fraction), (clock as i32, 0.0));
        assert_eq!(head.hour(), Some(clock));
    }

    /// A seek takes the head exactly where it is sought, on the frame it reaches, rather than
    /// steering from the hour of the history it left.
    #[test]
    fn a_seek_is_answered_on_the_frame_it_reaches() {
        let mut head = FrameHead::default();
        let published = CLOCK + interval() * 2.2;
        head.take(frame(CLOCK, published));
        let sought = CLOCK - 500_000.0;
        let at = head.take(frame(sought, sought + interval() * 2.2));
        assert_eq!(at.hour, sought as i32, "the head is at the hour sought");
        head.forget();
        let at = head.take(frame(CLOCK, published));
        assert_eq!(at.hour, CLOCK as i32);
    }

    /// A speed change is answered on the next display frame: the head advances by what playback
    /// advanced at the new speed, and nothing of the rate before it is carried.
    #[test]
    fn a_speed_change_is_answered_on_the_next_display_frame() {
        let mut head = FrameHead::default();
        let mut clock = CLOCK;
        let published = clock + interval() * 2.2;
        for _ in 0..8 {
            clock += SPEED / 240.0;
            head.take(frame(clock, published + (clock - CLOCK)));
        }
        let was = head.hour().expect("a head");
        // A quarter of the speed, and the frame after it.
        let slow = SPEED / 4.0;
        clock += slow / 240.0;
        let at = head.take(Frame { clock, published: Some((clock + interval() * 2.2) as i32), speed: slow, moving: true });
        let step = f64::from(at.hour) + f64::from(at.fraction) - was;
        assert!((step - slow / 240.0).abs() < 1e-6, "the head advanced {step} at the new speed's {}", slow / 240.0);
    }

    /// The hours running the other way (a reversal: the clock taken back while playing) is answered
    /// on the frame it reaches, and no hour of the head before it is shown after it.
    #[test]
    fn a_reversal_shows_nothing_of_the_head_before_it() {
        let mut head = FrameHead::default();
        let mut clock = CLOCK;
        let published = clock + interval() * 2.2;
        for _ in 0..8 {
            head.take(frame(clock, published + (clock - CLOCK)));
            clock += SPEED / 240.0;
        }
        let back = clock - 5_000.0;
        let at = head.take(frame(back, back + interval() * 2.2));
        assert_eq!(at.hour, back as i32, "the head stands where the hours were taken back to");
        // And it plays on from there rather than from where it was.
        let on = head.take(frame(back + SPEED / 240.0, back + interval() * 2.2));
        assert!(f64::from(on.hour) < clock, "{} is past the head it left", on.hour);
    }

    /// Before the first publication the head is the clock's: nothing published is nothing to keep
    /// behind, and the first picture stands at the hour it was composed for.
    #[test]
    fn nothing_published_leaves_the_head_at_the_clock() {
        let mut head = FrameHead::default();
        let at = head.take(Frame { clock: CLOCK, published: None, speed: SPEED, moving: true });
        assert_eq!(at.hour, CLOCK as i32);
    }

    /// The fraction of an hour survives: at sixty million neighbouring `f32` values are four hours
    /// apart, so the head is split into an integer hour and the fraction into it.
    #[test]
    fn a_quarter_of_an_hour_is_kept_at_sixty_million() {
        let at = split(CLOCK + 0.25);
        assert_eq!((at.hour, at.fraction), (CLOCK as i32, 0.25));
    }
}
