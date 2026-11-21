
use std::{
    cell::RefCell,
    sync::atomic::{AtomicU64, Ordering::Relaxed},
    time::{Duration, Instant},
};

pub const BOUND: u32 = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mark {
    Actors,
}

impl Mark {
    const ALL: [Self; 1] = [Self::Actors];

    fn at(self) -> usize {
        self as usize
    }
}

struct Learned {
    rest: [f64; Mark::ALL.len()],
}

struct Frame {
    began: Instant,
    interval: Duration,
    keep: bool,
    reached: [Option<Duration>; Mark::ALL.len()],
}

thread_local! {
    static LEARNED: RefCell<Learned> = const { RefCell::new(Learned { rest: [0.0; Mark::ALL.len()] }) };
    static FRAME: RefCell<Option<Frame>> = const { RefCell::new(None) };
}

const FOLLOW: f64 = 0.1;

#[must_use = "the budget stands while this is held"]
pub fn begin(keep: bool) -> Budgeted {
    begin_at(Instant::now(), keep);
    Budgeted
}

pub struct Budgeted;

impl Drop for Budgeted {
    fn drop(&mut self) {
        finish();
    }
}

fn begin_at(began: Instant, keep: bool) {
    let interval = Duration::from_secs_f64(1.0 / super::display_refresh::rate());
    FRAME.with_borrow_mut(|frame| {
        *frame = Some(Frame { began, interval, keep, reached: [None; Mark::ALL.len()] });
    });
}

fn finish() {
    finish_at(Instant::now());
}

fn finish_at(now: Instant) {
    let Some(frame) = FRAME.with_borrow_mut(Option::take) else {
        return;
    };
    let whole = now.saturating_duration_since(frame.began).as_micros() as f64;
    LEARNED.with_borrow_mut(|learned| {
        for mark in Mark::ALL {
            if let Some(at) = frame.reached[mark.at()] {
                let rest = (whole - at.as_micros() as f64).max(0.0);
                let held = &mut learned.rest[mark.at()];
                *held += FOLLOW * (rest - *held);
            }
        }
    });
}

pub fn room(mark: Mark) -> Option<Duration> {
    room_at(mark, Instant::now())
}

fn room_at(mark: Mark, now: Instant) -> Option<Duration> {
    FRAME.with_borrow_mut(|frame| {
        let frame = frame.as_mut()?;
        let spent = now.saturating_duration_since(frame.began);
        frame.reached[mark.at()].get_or_insert(spent);
        if !frame.keep {
            return None;
        }
        let rest = LEARNED.with_borrow(|learned| learned.rest[mark.at()]);
        let left = frame.interval.as_micros() as f64 - spent.as_micros() as f64 - rest;
        Some(Duration::from_micros(left.max(0.0) as u64))
    })
}

pub fn spent() -> Option<Duration> {
    FRAME.with_borrow(|frame| frame.as_ref().map(|frame| frame.began.elapsed()))
}

pub fn deadline(mark: Mark) -> Option<Instant> {
    room(mark).map(|room| Instant::now() + room)
}

pub mod check {
    use super::{BOUND, AtomicU64, Relaxed};

    static CHECKED: AtomicU64 = AtomicU64::new(0);
    static DIFFERING: AtomicU64 = AtomicU64::new(0);
    static CARRIED: AtomicU64 = AtomicU64::new(0);
    static LONGEST: AtomicU64 = AtomicU64::new(0);

    pub fn on() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| {
            std::env::var_os("BATTLEPLAN_VIEWER_CHECK_BUDGET").or_else(|| std::env::var_os("HOI4_VIEWER_CHECK_BUDGET")).is_some_and(|value| !value.is_empty() && value != "0")
        })
    }

    pub fn carrying(part: &'static str, pieces: usize, frames: u32) {
        if !on() {
            return;
        }
        CHECKED.fetch_add(1, Relaxed);
        if pieces > 0 {
            CARRIED.fetch_add(1, Relaxed);
            LONGEST.fetch_max(u64::from(frames), Relaxed);
        }
        if frames > BOUND {
            found(part, format!("{pieces} pieces held {frames} frames, past the bound of {BOUND}"));
        }
    }

    pub fn quiet(part: &'static str, pieces: usize) {
        if !on() {
            return;
        }
        CHECKED.fetch_add(1, Relaxed);
        if pieces > 0 {
            found(part, format!("{pieces} pieces still pending where the hours have stopped"));
        }
    }

    pub fn found(part: &str, why: impl std::fmt::Display) {
        DIFFERING.fetch_add(1, Relaxed);
        eprintln!("BATTLEPLAN_VIEWER_CHECK_BUDGET: {part}: {why}");
    }

    pub fn counted() -> (u64, u64, u64, u64) {
        (CHECKED.load(Relaxed), DIFFERING.load(Relaxed), CARRIED.load(Relaxed), LONGEST.load(Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn began(spent: u64) -> Instant {
        Instant::now() - Duration::from_micros(spent)
    }

    #[test]
    fn a_frame_that_must_be_whole_keeps_no_budget_and_a_budgeted_frames_room_falls_as_it_spends() {
        super::super::display_refresh::pace_at(120.0);
        begin_at(began(0), false);
        assert_eq!(room(Mark::Actors), None, "a frame that must be whole keeps no budget");
        assert_eq!(deadline(Mark::Actors), None);
        finish();
        assert_eq!(room(Mark::Actors), None, "no frame has begun");
        let start = began(0);
        begin_at(start, true);
        let early = room_at(Mark::Actors, start).expect("a budgeted frame has room");
        let late = room_at(Mark::Actors, start + Duration::from_micros(2_000)).expect("room");
        assert!(early <= Duration::from_micros(8_334), "the room is at most the display's interval");
        assert_eq!(early - late, Duration::from_micros(2_000), "the room falls by what the frame spent");
        finish();
    }

    #[test]
    fn a_mark_learns_what_the_frames_before_spent_past_it_and_closes_when_they_fill_the_interval() {
        super::super::display_refresh::pace_at(120.0);
        for _ in 0..20 {
            let start = began(0);
            begin_at(start, true);
            room_at(Mark::Actors, start);
            finish_at(start + Duration::from_micros(4_000));
        }
        let start = began(0);
        begin_at(start, true);
        let room = room_at(Mark::Actors, start).expect("room");
        assert!(
            room > Duration::from_micros(4_500) && room < Duration::from_micros(5_000),
            "a frame that reaches the mark at once keeps the interval less what follows: {room:?}"
        );
        assert_eq!(
            room_at(Mark::Actors, start + Duration::from_micros(5_000)),
            Some(Duration::ZERO),
            "and a frame already five milliseconds in has nothing left past that mark"
        );
        finish();
    }

    #[test]
    fn the_judge_names_a_piece_held_past_the_bound() {
        assert!(!check::on(), "the judge is off unless BATTLEPLAN_VIEWER_CHECK_BUDGET is set");
        let (checks, differing, _, _) = check::counted();
        check::carrying("a part", 9, BOUND + 1);
        check::quiet("a part", 3);
        assert_eq!(check::counted(), (checks, differing, 0, 0), "an off judge reads nothing");
    }
}
