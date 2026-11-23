
use viewer_gpu::icon_tracks::Head;
use viewer_runtime::simulation_rate;

pub const KEEP: f64 = 0.125;

pub const BEHIND_MOST: f64 = 1.0;

pub const PULL: f64 = 0.125;

const JUMPED: f64 = 4.0;

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameHead {
    at: Option<f64>,
    clock: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub clock: f64,
    pub published: Option<i32>,
    pub speed: f64,
    pub moving: bool,
}

impl FrameHead {
    pub fn take(&mut self, frame: Frame) -> Head {
        let interval = interval_hours(frame.speed);
        let advance = frame.clock - self.clock;
        let steady = frame.moving && interval > 0.0 && (0.0..interval * JUMPED).contains(&advance);
        let Some(at) = self.at.filter(|_| steady) else {
            return self.stand(frame.clock);
        };
        let target = frame
            .published
            .map_or(frame.clock, |hour| (f64::from(hour) - interval * KEEP).min(frame.clock))
            .max(frame.clock - interval * BEHIND_MOST);
        let error = ((target - at) / interval).clamp(-1.0, 1.0);
        let moved = (at + advance * (1.0 + error * PULL)).clamp(at, frame.clock);
        if viewer_runtime::hour_check::on() {
            judge(at, moved, advance, frame.clock, interval);
        }
        self.clock = frame.clock;
        self.at = Some(moved);
        split(moved)
    }

    fn stand(&mut self, clock: f64) -> Head {
        self.clock = clock;
        self.at = Some(clock);
        split(clock)
    }

    pub fn forget(&mut self) {
        self.at = None;
        self.clock = 0.0;
    }

    pub fn hour(&self) -> Option<f64> {
        self.at
    }
}

fn judge(was: f64, now: f64, advance: f64, clock: f64, interval: f64) {
    viewer_runtime::hour_check::judged();
    let step = now - was;
    if advance > 0.0 && step <= 0.0 {
        viewer_runtime::hour_check::found("the frame's head", format!("held at {was} while playback advanced {advance} hours"));
    }
    if step > advance * (1.0 + PULL) + 1e-6 {
        viewer_runtime::hour_check::found("the frame's head", format!("advanced {step} hours where playback advanced {advance}"));
    }
    if clock - now > interval * BEHIND_MOST + advance + 1e-6 {
        viewer_runtime::hour_check::found(
            "the frame's head",
            format!("stands {} hours behind the clock, past the {} it is allowed", clock - now, interval * BEHIND_MOST),
        );
    }
}

pub fn shown(head: Head, published: Option<i32>) -> Head {
    match published {
        Some(hour) if head.hour > hour || (head.hour == hour && head.fraction > 0.0) => Head { hour, fraction: 0.0 },
        _ => head,
    }
}

fn interval_hours(speed: f64) -> f64 {
    if speed > 0.0 {
        speed * simulation_rate::interval().as_secs_f64()
    } else {
        0.0
    }
}

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

    fn frame(clock: f64, published: f64) -> Frame {
        Frame { clock, published: Some(published as i32), speed: SPEED, moving: true }
    }

    #[test]
    fn a_head_past_the_newest_hour_published_shows_that_hour() {
        let head = Head { hour: 150, fraction: 0.5 };
        assert_eq!(shown(head, Some(148)), Head { hour: 148, fraction: 0.0 });
        assert_eq!(shown(Head { hour: 148, fraction: 0.25 }, Some(148)), Head { hour: 148, fraction: 0.0 });
        assert_eq!(shown(head, Some(160)), head);
        assert_eq!(shown(head, None), head);
    }

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

    #[test]
    fn a_late_publication_slows_the_head_and_never_stops_it() {
        let mut head = FrameHead::default();
        let mut clock = CLOCK;
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

    #[test]
    fn a_stopped_picture_stands_exactly_at_the_clock() {
        let mut head = FrameHead::default();
        let mut clock = CLOCK;
        let published = clock + interval() * 2.2;
        for _ in 0..240 {
            head.take(frame(clock, published));
            clock += SPEED / 240.0;
        }
        assert_ne!(head.hour(), Some(clock), "the head had fallen behind");
        let at = head.take(Frame { moving: false, ..frame(clock, published) });
        assert_eq!((at.hour, at.fraction), (clock as i32, 0.0));
        assert_eq!(head.hour(), Some(clock));
    }

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
        let slow = SPEED / 4.0;
        clock += slow / 240.0;
        let at = head.take(Frame { clock, published: Some((clock + interval() * 2.2) as i32), speed: slow, moving: true });
        let step = f64::from(at.hour) + f64::from(at.fraction) - was;
        assert!((step - slow / 240.0).abs() < 1e-6, "the head advanced {step} at the new speed's {}", slow / 240.0);
    }

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
        let on = head.take(frame(back + SPEED / 240.0, back + interval() * 2.2));
        assert!(f64::from(on.hour) < clock, "{} is past the head it left", on.hour);
    }

    #[test]
    fn nothing_published_leaves_the_head_at_the_clock() {
        let mut head = FrameHead::default();
        let at = head.take(Frame { clock: CLOCK, published: None, speed: SPEED, moving: true });
        assert_eq!(at.hour, CLOCK as i32);
    }

    #[test]
    fn a_quarter_of_an_hour_is_kept_at_sixty_million() {
        let at = split(CLOCK + 0.25);
        assert_eq!((at.hour, at.fraction), (CLOCK as i32, 0.25));
    }
}
