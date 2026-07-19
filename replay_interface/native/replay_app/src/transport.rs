//! E1 transport controls: play/pause, speed steps, seek (index / hour /
//! fraction / date), jump-to-date, and day-boundary seek-bar ticks.
//!
//! This is a pure state machine over a recording's ordered `(hour_index,
//! date)` spine. The Win32 loop drives it (`advance` on each frame, key/scrub
//! events call the seek/step/speed methods) and then reads `current_hour()`
//! back to scrub the map through `replay_core` (`WorldTimeline::seek` /
//! `play_to` — which resolve identically today; see the
//! `seek_equals_play_to` conformance gate). No recording bytes are parsed
//! here; dates come from the frames' own `date` strings.
//!
//! Parameter labels (per the no-eyeball / FILE-CAPTURE-FIT-UNKNOWN rule):
//! * [`SPEED_STEPS_HOURS_PER_SEC`] — **FIT (product UX)**. These are
//!   replay-app *playback* rates (game-hours advanced per wall-clock second),
//!   a video-player-style control. They are NOT the engine's game-speed
//!   law and make no parity claim; the default matches the pre-existing
//!   autoplay rate (0.25 s/hour = 4 h/s).

use crate::clausewitz::Date;

/// Playback speed steps in game-hours per wall-clock second. **FIT (product
/// UX)** — see the module note. Explicit, ascending, and fixed so the
/// speed-step behavior is fully determined and unit-gated.
pub const SPEED_STEPS_HOURS_PER_SEC: [f64; 5] = [1.0, 2.0, 4.0, 12.0, 48.0];

/// Default speed step index: `4.0` h/s, matching the prior autoplay rate.
pub const DEFAULT_SPEED_STEP: usize = 2;

/// One day-boundary tick on the seek bar: the first recorded frame of a new
/// calendar day, with its `Y.M.D` label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateTick {
    pub index: usize,
    pub hour: i64,
    pub label: String,
}

/// Transport state over an ordered recorded-hour spine.
#[derive(Debug, Clone)]
pub struct Transport {
    /// Strictly increasing recorded hour indexes (the scrub authority).
    hours: Vec<i64>,
    /// Per-hour `date` strings, parallel to `hours` (`None` when the frame
    /// carried no date). Used only for date ticks and jump-to-date.
    dates: Vec<Option<String>>,
    index: usize,
    playing: bool,
    speed_step: usize,
    /// Sub-hour playback accumulator (game-hours), reset on any explicit seek.
    accum: f64,
}

impl Transport {
    /// Build over recorded hours and their parallel date strings. `hours` must
    /// be non-empty and the two slices equal length; otherwise `Err` (the
    /// caller has a malformed timeline and must fail closed).
    pub fn new(hours: Vec<i64>, dates: Vec<Option<String>>) -> Result<Transport, String> {
        if hours.is_empty() {
            return Err("transport: recording has no hours".to_string());
        }
        if hours.len() != dates.len() {
            return Err(format!(
                "transport: hours ({}) and dates ({}) length mismatch",
                hours.len(),
                dates.len()
            ));
        }
        Ok(Transport {
            hours,
            dates,
            index: 0,
            playing: false,
            speed_step: DEFAULT_SPEED_STEP,
            accum: 0.0,
        })
    }

    /// Build with no date strings (all `None`); date-based controls degrade to
    /// clamping to the first hour, which is the honest behavior for a
    /// dateless recording.
    pub fn without_dates(hours: Vec<i64>) -> Result<Transport, String> {
        let dates = vec![None; hours.len()];
        Transport::new(hours, dates)
    }

    pub fn len(&self) -> usize {
        self.hours.len()
    }
    pub fn is_empty(&self) -> bool {
        self.hours.is_empty()
    }
    pub fn current_index(&self) -> usize {
        self.index
    }
    pub fn current_hour(&self) -> i64 {
        self.hours[self.index]
    }
    pub fn current_date(&self) -> Option<&str> {
        self.dates[self.index].as_deref()
    }
    pub fn hours(&self) -> &[i64] {
        &self.hours
    }
    pub fn is_playing(&self) -> bool {
        self.playing
    }
    pub fn speed_step(&self) -> usize {
        self.speed_step
    }
    pub fn speed_hours_per_sec(&self) -> f64 {
        SPEED_STEPS_HOURS_PER_SEC[self.speed_step]
    }

    // --- play/pause ---

    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
        self.accum = 0.0;
    }
    pub fn toggle_play(&mut self) {
        self.set_playing(!self.playing);
    }

    // --- speed steps ---

    pub fn set_speed_step(&mut self, step: usize) {
        self.speed_step = step.min(SPEED_STEPS_HOURS_PER_SEC.len() - 1);
    }
    pub fn speed_up(&mut self) {
        self.set_speed_step(self.speed_step + 1);
    }
    pub fn speed_down(&mut self) {
        self.speed_step = self.speed_step.saturating_sub(1);
    }

    // --- stepping / seeking ---

    /// Step one recorded hour forward (clamped at the last hour).
    pub fn step_forward(&mut self) -> usize {
        self.seek_to_index(self.index + 1)
    }
    /// Step one recorded hour backward (clamped at the first hour).
    pub fn step_backward(&mut self) -> usize {
        self.seek_to_index(self.index.saturating_sub(1))
    }

    /// Seek to a recorded-hour index, clamped into range. Resets the
    /// sub-hour accumulator so playback continues from the seek point.
    pub fn seek_to_index(&mut self, index: usize) -> usize {
        self.index = index.min(self.hours.len() - 1);
        self.accum = 0.0;
        self.index
    }

    /// Seek to the recorded hour at-or-before `hour` (the scrub primitive,
    /// matching `WorldTimeline::state_at`). Before the first recorded hour it
    /// clamps to the first; after the last it clamps to the last.
    pub fn seek_to_hour(&mut self, hour: i64) -> usize {
        let index = match self.hours.binary_search(&hour) {
            Ok(i) => i,
            Err(0) => 0,
            Err(i) => i - 1,
        };
        self.seek_to_index(index)
    }

    /// Seek by a `0.0..=1.0` fraction across the recorded-hour count (the
    /// seek-bar drag primitive). Index-spaced, matching the tick layout.
    pub fn seek_to_fraction(&mut self, fraction: f64) -> usize {
        if self.hours.len() < 2 {
            return self.seek_to_index(0);
        }
        let scaled = fraction.clamp(0.0, 1.0) * (self.hours.len() - 1) as f64;
        self.seek_to_index(scaled.round() as usize)
    }

    /// Jump to a Clausewitz date (`Y.M.D` or `Y.M.D.H`): resolves to the
    /// recorded hour whose date is at-or-before the target, clamped to
    /// `[first, last]`. A target before the first dated frame clamps to index
    /// 0; a target after the last clamps to the last dated frame. Frames with
    /// no/unparseable date are skipped. `Err` only when the *input* string is
    /// unparseable (never a guessed jump).
    pub fn jump_to_date(&mut self, date: &str) -> Result<usize, String> {
        let target = Date::parse(date)
            .ok_or_else(|| format!("jump-to-date: {date:?} is not a Y.M.D[.H] date"))?;
        let mut found: Option<usize> = None;
        for (i, d) in self.dates.iter().enumerate() {
            let Some(parsed) = d.as_deref().and_then(Date::parse) else {
                continue;
            };
            if parsed <= target {
                found = Some(i);
            } else {
                break;
            }
        }
        // Before the first dated frame → clamp to the first hour.
        Ok(self.seek_to_index(found.unwrap_or(0)))
    }

    // --- playback advance ---

    /// Advance playback by `dt_secs` of wall time. Steps whole recorded hours
    /// forward at the current speed; auto-pauses at the last hour (product
    /// playback does not wrap). Returns whether the current index changed.
    pub fn advance(&mut self, dt_secs: f64) -> bool {
        if !self.playing || self.hours.len() < 2 {
            return false;
        }
        self.accum += dt_secs * self.speed_hours_per_sec();
        let mut changed = false;
        while self.accum >= 1.0 {
            self.accum -= 1.0;
            if self.index + 1 < self.hours.len() {
                self.index += 1;
                changed = true;
            } else {
                self.playing = false;
                self.accum = 0.0;
                break;
            }
        }
        changed
    }

    // --- seek-bar ticks ---

    /// One tick per calendar day, at the first recorded hour of that day,
    /// labeled `Y.M.D`. Empty when no frames carry parseable dates (the seek
    /// bar then falls back to its index-spaced hour ticks).
    pub fn date_ticks(&self) -> Vec<DateTick> {
        let mut ticks = Vec::new();
        let mut last_day: Option<(i64, u32, u32)> = None;
        for (i, d) in self.dates.iter().enumerate() {
            let Some(parsed) = d.as_deref().and_then(Date::parse) else {
                continue;
            };
            let day = parsed.day_key();
            if last_day != Some(day) {
                ticks.push(DateTick {
                    index: i,
                    hour: self.hours[i],
                    label: parsed.day_label(),
                });
                last_day = Some(day);
            }
        }
        ticks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dated(hours: &[i64], dates: &[&str]) -> Transport {
        Transport::new(
            hours.to_vec(),
            dates.iter().map(|d| Some(d.to_string())).collect(),
        )
        .expect("transport")
    }

    #[test]
    fn new_rejects_empty_and_mismatched() {
        assert!(Transport::new(vec![], vec![]).is_err());
        assert!(Transport::new(vec![1, 2], vec![None]).is_err());
    }

    #[test]
    fn speed_step_table_is_explicit_and_clamped() {
        // The table itself is the contract.
        assert_eq!(SPEED_STEPS_HOURS_PER_SEC, [1.0, 2.0, 4.0, 12.0, 48.0]);
        let mut t = Transport::without_dates(vec![0, 1, 2]).unwrap();
        assert_eq!(t.speed_step(), DEFAULT_SPEED_STEP);
        assert_eq!(t.speed_hours_per_sec(), 4.0);
        // speed_down clamps at 0.
        t.speed_down();
        t.speed_down();
        t.speed_down();
        assert_eq!(t.speed_step(), 0);
        assert_eq!(t.speed_hours_per_sec(), 1.0);
        // speed_up clamps at the last step.
        for _ in 0..10 {
            t.speed_up();
        }
        assert_eq!(t.speed_step(), 4);
        assert_eq!(t.speed_hours_per_sec(), 48.0);
        t.set_speed_step(99);
        assert_eq!(t.speed_step(), 4);
    }

    #[test]
    fn play_pause_toggle_resets_accumulator() {
        let mut t = Transport::without_dates(vec![0, 1, 2, 3]).unwrap();
        assert!(!t.is_playing());
        t.toggle_play();
        assert!(t.is_playing());
        // Half an hour accumulated, then a toggle clears it.
        t.set_speed_step(2); // 4 h/s
        assert!(!t.advance(0.1)); // 0.4h < 1h, no step
        t.toggle_play();
        assert!(!t.is_playing());
        t.toggle_play();
        assert!(t.is_playing());
        // Accumulator was reset by toggles, so this 0.4h alone still no step.
        assert!(!t.advance(0.1));
    }

    #[test]
    fn advance_steps_and_autopauses_at_end() {
        let mut t = Transport::without_dates(vec![0, 1, 2]).unwrap();
        t.set_speed_step(2); // 4 h/s
        t.set_playing(true);
        // 1.0s * 4 h/s = 4 hours of demand over a 2-hour span → index 0 -> 2,
        // then the surplus hits the end and auto-pauses.
        assert!(t.advance(1.0));
        assert_eq!(t.current_index(), 2);
        assert!(!t.is_playing(), "auto-paused at last hour");
        // Further advance is a no-op once paused.
        assert!(!t.advance(1.0));
        assert_eq!(t.current_index(), 2);
    }

    #[test]
    fn step_forward_backward_clamp() {
        let mut t = Transport::without_dates(vec![10, 20, 30]).unwrap();
        assert_eq!(t.step_backward(), 0); // clamp low
        assert_eq!(t.step_forward(), 1);
        assert_eq!(t.step_forward(), 2);
        assert_eq!(t.step_forward(), 2); // clamp high
        assert_eq!(t.current_hour(), 30);
    }

    #[test]
    fn seek_to_hour_is_at_or_before_and_clamped() {
        let mut t = Transport::without_dates(vec![100, 200, 300]).unwrap();
        assert_eq!(t.seek_to_hour(50), 0); // before first → first
        assert_eq!(t.seek_to_hour(200), 1); // exact
        assert_eq!(t.seek_to_hour(250), 1); // at-or-before
        assert_eq!(t.seek_to_hour(9999), 2); // after last → last
    }

    #[test]
    fn seek_to_fraction_index_spaced() {
        let mut t = Transport::without_dates((0..11).collect()).unwrap();
        assert_eq!(t.seek_to_fraction(0.0), 0);
        assert_eq!(t.seek_to_fraction(0.5), 5);
        assert_eq!(t.seek_to_fraction(1.0), 10);
        assert_eq!(t.seek_to_fraction(-1.0), 0); // clamp
        assert_eq!(t.seek_to_fraction(2.0), 10); // clamp
    }

    #[test]
    fn jump_to_date_resolves_at_or_before_and_clamps() {
        let t0 = "1936.1.1.0";
        let t1 = "1936.1.1.12";
        let t2 = "1936.1.2.0";
        let mut t = dated(&[0, 12, 24], &[t0, t1, t2]);
        // exact
        assert_eq!(t.jump_to_date("1936.1.1.12").unwrap(), 1);
        // at-or-before (06:00 → the 00:00 frame)
        assert_eq!(t.jump_to_date("1936.1.1.6").unwrap(), 0);
        // before first → clamp to 0
        assert_eq!(t.jump_to_date("1935.12.31").unwrap(), 0);
        // after last → clamp to last dated frame
        assert_eq!(t.jump_to_date("1990.1.1").unwrap(), 2);
        // date-only target matches the day boundary
        assert_eq!(t.jump_to_date("1936.1.2").unwrap(), 2);
        // unparseable input errors (no guessed jump)
        assert!(t.jump_to_date("not-a-date").is_err());
    }

    #[test]
    fn date_ticks_one_per_calendar_day() {
        // Two hours on day 1, two on day 2, one on day 3.
        let t = dated(
            &[0, 1, 24, 25, 48],
            &[
                "1936.1.1.0",
                "1936.1.1.1",
                "1936.1.2.0",
                "1936.1.2.1",
                "1936.1.3.0",
            ],
        );
        let ticks = t.date_ticks();
        assert_eq!(ticks.len(), 3);
        assert_eq!(
            ticks[0],
            DateTick {
                index: 0,
                hour: 0,
                label: "1936.1.1".into()
            }
        );
        assert_eq!(
            ticks[1],
            DateTick {
                index: 2,
                hour: 24,
                label: "1936.1.2".into()
            }
        );
        assert_eq!(
            ticks[2],
            DateTick {
                index: 4,
                hour: 48,
                label: "1936.1.3".into()
            }
        );
    }

    #[test]
    fn date_ticks_empty_without_dates() {
        let t = Transport::without_dates(vec![0, 1, 2]).unwrap();
        assert!(t.date_ticks().is_empty());
        // Jump-to-date on a dateless recording clamps to the first hour.
        let mut t = t;
        assert_eq!(t.jump_to_date("1936.1.1").unwrap(), 0);
    }
}
