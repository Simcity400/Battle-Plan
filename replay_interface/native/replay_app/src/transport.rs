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
use replay_core::{Leaf, SessionMetadataView};
use std::error::Error;
use std::fmt;

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

/// A fail-closed date-navigation error. Recorded-date lookup failures are
/// distinct from malformed user input so callers never turn unavailable index
/// metadata into a guessed seek.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateNavigationError {
    InvalidTarget { target: String },
    Lookup { hour: i64, detail: String },
}

impl fmt::Display for DateNavigationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DateNavigationError::InvalidTarget { target } => {
                write!(f, "jump-to-date: {target:?} is not a Y.M.D[.H] date")
            }
            DateNavigationError::Lookup { hour, detail } => {
                write!(f, "date index unavailable at hour {hour}: {detail}")
            }
        }
    }
}

impl Error for DateNavigationError {}

#[derive(Debug, Clone)]
enum DateSlot {
    /// The authenticated date index has not been consulted for this hour.
    Unknown,
    /// The index was consulted successfully. `None` is an authenticated
    /// dateless frame, not an unavailable lookup.
    Known(Option<String>),
}

/// Exact recorded engine clock state at the selected replay hour.
///
/// The engine speed slot is preserved as recorded state, but it does not select
/// a product wall-clock rate: the game simulation throughput is machine-dependent
/// and the recording contains no authoritative seconds-per-hour law. Recorded
/// pause is likewise separate from the viewer's play/pause control because a
/// paused capture contains no authoritative wall duration to wait before the
/// next recorded hour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordedClockState {
    pub speed_raw: u8,
    pub paused: bool,
    pub tick_progress_f32_bits: Option<u32>,
    pub transition_sequence: Option<u64>,
}

/// Transport state over an ordered recorded-hour spine.
#[derive(Debug, Clone)]
pub struct Transport {
    /// Strictly increasing recorded hour indexes (the scrub authority).
    hours: Vec<i64>,
    /// Lazy per-hour date-index cache, parallel to `hours`. Product startup
    /// leaves every slot `Unknown`; user-invoked date navigation fills only
    /// the portion it needs without loading snapshot bodies.
    dates: Vec<DateSlot>,
    index: usize,
    playing: bool,
    speed_step: usize,
    /// Sub-hour playback accumulator (game-hours), reset on any explicit seek.
    accum: f64,
    /// X12/D57 clock state consumed from the currently selected typed frame.
    /// `None` is the fail-closed legacy/unavailable state.
    recorded_clock: Option<RecordedClockState>,
}

/// Constant-size Present transaction snapshot. Lazy date-cache observations
/// are immutable input knowledge and may be retained; only viewer-visible
/// transport selection/cadence state rolls back when a frame is undisplayed.
#[derive(Debug, Clone, Copy)]
pub struct TransportPresentationCheckpoint {
    index: usize,
    playing: bool,
    speed_step: usize,
    accum: f64,
    recorded_clock: Option<RecordedClockState>,
}

impl Transport {
    pub fn presentation_checkpoint(&self) -> TransportPresentationCheckpoint {
        TransportPresentationCheckpoint {
            index: self.index,
            playing: self.playing,
            speed_step: self.speed_step,
            accum: self.accum,
            recorded_clock: self.recorded_clock,
        }
    }

    pub fn restore_presentation_checkpoint(
        &mut self,
        checkpoint: TransportPresentationCheckpoint,
    ) {
        self.index = checkpoint.index;
        self.playing = checkpoint.playing;
        self.speed_step = checkpoint.speed_step;
        self.accum = checkpoint.accum;
        self.recorded_clock = checkpoint.recorded_clock;
    }

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
            dates: dates.into_iter().map(DateSlot::Known).collect(),
            index: 0,
            playing: false,
            speed_step: DEFAULT_SPEED_STEP,
            accum: 0.0,
            recorded_clock: None,
        })
    }

    /// Build over an authenticated hour index without enumerating its date
    /// metadata. Calls to the `*_with` date-navigation methods resolve and
    /// cache dates through the supplied index lookup only when requested.
    pub fn with_lazy_dates(hours: Vec<i64>) -> Result<Transport, String> {
        if hours.is_empty() {
            return Err("transport: recording has no hours".to_string());
        }
        let date_count = hours.len();
        Ok(Transport {
            hours,
            dates: vec![DateSlot::Unknown; date_count],
            index: 0,
            playing: false,
            speed_step: DEFAULT_SPEED_STEP,
            accum: 0.0,
            recorded_clock: None,
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
    pub fn current_date(&self) -> std::result::Result<Option<&str>, DateNavigationError> {
        match &self.dates[self.index] {
            DateSlot::Known(date) => Ok(date.as_deref()),
            DateSlot::Unknown => Err(DateNavigationError::Lookup {
                hour: self.current_hour(),
                detail: "date index has not been resolved".to_string(),
            }),
        }
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
    pub fn recorded_clock(&self) -> Option<RecordedClockState> {
        self.recorded_clock
    }

    /// Consume the typed D57/X12 tail for the currently displayed frame.
    ///
    /// A complete X12 history is cumulative, so its last row is the exact
    /// speed/pause/tick state at the snapshot boundary. Legacy hourly anchors
    /// also work, but carry no sub-hour tick/sequence. Invalid or unavailable
    /// leaves clear the recorded state and never synthesize a clock.
    pub fn apply_recorded_session_timing(&mut self, session: &SessionMetadataView) -> bool {
        let Some(events) = session.speed_events.measured() else {
            self.recorded_clock = None;
            return false;
        };
        let Some(event) = events.last() else {
            self.recorded_clock = None;
            return false;
        };
        let (Leaf::Measured(speed), Leaf::Measured(paused)) = (&event.speed, &event.paused) else {
            self.recorded_clock = None;
            return false;
        };
        let Ok(speed_raw) = u8::try_from(*speed) else {
            self.recorded_clock = None;
            return false;
        };
        if speed_raw as usize >= SPEED_STEPS_HOURS_PER_SEC.len() {
            self.recorded_clock = None;
            return false;
        }
        let tick_progress_f32_bits = match &event.tick_progress_f32_bits {
            Leaf::Measured(bits) => Some(*bits),
            Leaf::Absent => None,
            Leaf::Unobtained => {
                self.recorded_clock = None;
                return false;
            }
        };
        let transition_sequence = match &event.transition_sequence {
            Leaf::Measured(sequence) => Some(*sequence),
            Leaf::Absent => None,
            Leaf::Unobtained => {
                self.recorded_clock = None;
                return false;
            }
        };
        self.recorded_clock = Some(RecordedClockState {
            speed_raw,
            paused: *paused,
            tick_progress_f32_bits,
            transition_sequence,
        });
        true
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
    pub fn jump_to_date(&mut self, date: &str) -> std::result::Result<usize, DateNavigationError> {
        let target = Date::parse(date).ok_or_else(|| DateNavigationError::InvalidTarget {
            target: date.to_string(),
        })?;
        let mut found: Option<usize> = None;
        for (i, d) in self.dates.iter().enumerate() {
            let DateSlot::Known(d) = d else {
                return Err(DateNavigationError::Lookup {
                    hour: self.hours[i],
                    detail: "date index has not been resolved; use jump_to_date_with".to_string(),
                });
            };
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

    fn parsed_date_at_with<F>(
        &mut self,
        index: usize,
        lookup: &mut F,
    ) -> std::result::Result<Option<Date>, DateNavigationError>
    where
        F: FnMut(i64) -> std::result::Result<Option<String>, String>,
    {
        let hour = self.hours[index];
        if matches!(self.dates[index], DateSlot::Unknown) {
            let date =
                lookup(hour).map_err(|detail| DateNavigationError::Lookup { hour, detail })?;
            self.dates[index] = DateSlot::Known(date);
        }
        match &self.dates[index] {
            DateSlot::Known(date) => Ok(date.as_deref().and_then(Date::parse)),
            DateSlot::Unknown => unreachable!("successful lookup must populate the date cache"),
        }
    }

    /// Lazy/index-backed jump-to-date. The supplied lookup must expose only
    /// authenticated index metadata; snapshot-body decoding does not belong in
    /// this seam. Successful lookups are cached, while a lookup error leaves
    /// the slot unresolved and the current selection unchanged.
    pub fn jump_to_date_with<F>(
        &mut self,
        date: &str,
        mut lookup: F,
    ) -> std::result::Result<usize, DateNavigationError>
    where
        F: FnMut(i64) -> std::result::Result<Option<String>, String>,
    {
        let target = Date::parse(date).ok_or_else(|| DateNavigationError::InvalidTarget {
            target: date.to_string(),
        })?;
        let mut found: Option<usize> = None;
        for index in 0..self.dates.len() {
            let Some(parsed) = self.parsed_date_at_with(index, &mut lookup)? else {
                continue;
            };
            if parsed <= target {
                found = Some(index);
            } else {
                break;
            }
        }
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
    pub fn date_ticks(&self) -> std::result::Result<Vec<DateTick>, DateNavigationError> {
        let mut ticks = Vec::new();
        let mut last_day: Option<(i64, u32, u32)> = None;
        for (i, d) in self.dates.iter().enumerate() {
            let DateSlot::Known(d) = d else {
                return Err(DateNavigationError::Lookup {
                    hour: self.hours[i],
                    detail: "date index has not been resolved; use date_ticks_with".to_string(),
                });
            };
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
        Ok(ticks)
    }

    /// Resolve the day-boundary tick spine on demand through the authenticated
    /// date index. App startup deliberately does not call this method; the
    /// first explicit day-navigation command resolves it and later commands
    /// reuse the cache.
    pub fn date_ticks_with<F>(
        &mut self,
        mut lookup: F,
    ) -> std::result::Result<Vec<DateTick>, DateNavigationError>
    where
        F: FnMut(i64) -> std::result::Result<Option<String>, String>,
    {
        let mut ticks = Vec::new();
        let mut last_day: Option<(i64, u32, u32)> = None;
        for index in 0..self.dates.len() {
            let Some(parsed) = self.parsed_date_at_with(index, &mut lookup)? else {
                continue;
            };
            let day = parsed.day_key();
            if last_day != Some(day) {
                ticks.push(DateTick {
                    index,
                    hour: self.hours[index],
                    label: parsed.day_label(),
                });
                last_day = Some(day);
            }
        }
        Ok(ticks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn session_with_speed_events(
        events: Leaf<Vec<replay_core::SpeedEvent>>,
    ) -> SessionMetadataView {
        SessionMetadataView {
            status: replay_core::ViewStatus::Present,
            player_countries: Leaf::Absent,
            speed_events: events,
            active_dlcs: Leaf::Absent,
            localisation_language: Leaf::Absent,
            display_variant: Leaf::Absent,
        }
    }

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
    fn x12_tail_preserves_exact_clock_without_inventing_wall_time() {
        let mut transport = Transport::without_dates(vec![100, 101]).unwrap();
        let session = session_with_speed_events(Leaf::Measured(vec![
            replay_core::SpeedEvent {
                hour_index: Leaf::Measured(100),
                speed: Leaf::Measured(1),
                paused: Leaf::Measured(false),
                transition_sequence: Leaf::Measured(0),
                tick_progress_f32_bits: Leaf::Absent,
                source: Leaf::Measured("pre_action_baseline".into()),
            },
            replay_core::SpeedEvent {
                hour_index: Leaf::Measured(100),
                speed: Leaf::Measured(4),
                paused: Leaf::Measured(true),
                transition_sequence: Leaf::Measured(7),
                tick_progress_f32_bits: Leaf::Measured(0x3f00_0000),
                source: Leaf::Measured("pause_toggle".into()),
            },
        ]));

        assert!(transport.apply_recorded_session_timing(&session));
        assert_eq!(transport.speed_step(), DEFAULT_SPEED_STEP);
        assert_eq!(transport.speed_hours_per_sec(), 4.0);
        assert_eq!(
            transport.recorded_clock(),
            Some(RecordedClockState {
                speed_raw: 4,
                paused: true,
                tick_progress_f32_bits: Some(0x3f00_0000),
                transition_sequence: Some(7),
            })
        );
    }

    #[test]
    fn unavailable_x12_clock_clears_recorded_state_without_inventing_a_speed() {
        let mut transport = Transport::without_dates(vec![100, 101]).unwrap();
        transport.set_speed_step(1);
        let measured = session_with_speed_events(Leaf::Measured(vec![replay_core::SpeedEvent {
            hour_index: Leaf::Measured(100),
            speed: Leaf::Measured(3),
            paused: Leaf::Measured(false),
            transition_sequence: Leaf::Absent,
            tick_progress_f32_bits: Leaf::Absent,
            source: Leaf::Absent,
        }]));
        assert!(transport.apply_recorded_session_timing(&measured));
        assert!(transport.recorded_clock().is_some());
        assert_eq!(transport.speed_step(), 1);

        let unavailable = session_with_speed_events(Leaf::Unobtained);
        assert!(!transport.apply_recorded_session_timing(&unavailable));
        assert_eq!(transport.recorded_clock(), None);
        assert_eq!(transport.speed_step(), 1);
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
    fn highest_speed_advance_reports_one_exact_skipped_hour_destination() {
        let hours = (0_i64..=100).collect::<Vec<_>>();
        let mut transport = Transport::without_dates(hours).unwrap();
        transport.set_speed_step(SPEED_STEPS_HOURS_PER_SEC.len() - 1);
        transport.set_playing(true);
        assert!(transport.advance(1.0));
        assert_eq!(transport.current_index(), 48);
        assert_eq!(transport.current_hour(), 48);
        assert!(transport.is_playing());
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
        let ticks = t.date_ticks().unwrap();
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
        assert!(t.date_ticks().unwrap().is_empty());
        // Jump-to-date on a dateless recording clamps to the first hour.
        let mut t = t;
        assert_eq!(t.jump_to_date("1936.1.1").unwrap(), 0);
    }

    #[test]
    fn lazy_date_index_is_unqueried_until_navigation_and_then_cached() {
        let calls = Cell::new(0usize);
        let mut transport = Transport::with_lazy_dates(vec![0, 1, 24, 25, 48]).unwrap();
        assert_eq!(calls.get(), 0, "construction must not enumerate dates");

        let ticks = transport
            .date_ticks_with(|hour| {
                calls.set(calls.get() + 1);
                let day = hour / 24 + 1;
                let clock_hour = hour % 24;
                Ok(Some(format!("1936.1.{day}.{clock_hour}")))
            })
            .expect("authenticated date index");
        assert_eq!(calls.get(), 5);
        assert_eq!(
            ticks.iter().map(|tick| tick.index).collect::<Vec<_>>(),
            [0, 2, 4]
        );

        let second = transport
            .date_ticks_with(|_| {
                calls.set(calls.get() + 1);
                Err("cached dates must not be reloaded".to_string())
            })
            .expect("cached date ticks");
        assert_eq!(second, ticks);
        assert_eq!(calls.get(), 5);
    }

    #[test]
    fn lazy_jump_preserves_behavior_and_lookup_errors_do_not_seek() {
        let mut transport = Transport::with_lazy_dates(vec![0, 12, 24]).unwrap();
        transport.seek_to_index(1);
        let error = transport
            .jump_to_date_with("1936.1.2", |hour| {
                if hour == 12 {
                    Err("authenticated index read failed".to_string())
                } else {
                    Ok(Some(format!("1936.1.1.{hour}")))
                }
            })
            .expect_err("lookup failure must fail closed");
        assert!(matches!(
            error,
            DateNavigationError::Lookup { hour: 12, .. }
        ));
        assert_eq!(transport.current_index(), 1, "failed lookup must not seek");

        assert_eq!(
            transport
                .jump_to_date_with("1936.1.2", |hour| {
                    Ok(Some(
                        match hour {
                            0 => "1936.1.1.0",
                            12 => "1936.1.1.12",
                            24 => "1936.1.2.0",
                            _ => unreachable!(),
                        }
                        .to_string(),
                    ))
                })
                .expect("retry authenticated index"),
            2
        );
    }
}
