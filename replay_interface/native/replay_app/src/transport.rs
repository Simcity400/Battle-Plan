
use crate::clausewitz::Date;
use replay_core::{Leaf, SessionMetadataView};
use std::error::Error;
use std::fmt;

pub const SPEED_STEPS_HOURS_PER_SEC: [f64; 5] = [1.0, 2.0, 4.0, 12.0, 48.0];

pub const DEFAULT_SPEED_STEP: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateTick {
    pub index: usize,
    pub hour: i64,
    pub label: String,
}

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
    Unknown,
    Known(Option<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordedClockState {
    pub speed_raw: u8,
    pub paused: bool,
    pub tick_progress_f32_bits: Option<u32>,
    pub transition_sequence: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Transport {
    hours: Vec<i64>,
    dates: Vec<DateSlot>,
    index: usize,
    playing: bool,
    speed_step: usize,
    accum: f64,
    recorded_clock: Option<RecordedClockState>,
}

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


    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
        self.accum = 0.0;
    }
    pub fn toggle_play(&mut self) {
        self.set_playing(!self.playing);
    }


    pub fn set_speed_step(&mut self, step: usize) {
        self.speed_step = step.min(SPEED_STEPS_HOURS_PER_SEC.len() - 1);
    }
    pub fn speed_up(&mut self) {
        self.set_speed_step(self.speed_step + 1);
    }
    pub fn speed_down(&mut self) {
        self.speed_step = self.speed_step.saturating_sub(1);
    }


    pub fn step_forward(&mut self) -> usize {
        self.seek_to_index(self.index + 1)
    }
    pub fn step_backward(&mut self) -> usize {
        self.seek_to_index(self.index.saturating_sub(1))
    }

    pub fn seek_to_index(&mut self, index: usize) -> usize {
        self.index = index.min(self.hours.len() - 1);
        self.accum = 0.0;
        self.index
    }

    pub fn seek_to_hour(&mut self, hour: i64) -> usize {
        let index = match self.hours.binary_search(&hour) {
            Ok(i) => i,
            Err(0) => 0,
            Err(i) => i - 1,
        };
        self.seek_to_index(index)
    }

    pub fn seek_to_fraction(&mut self, fraction: f64) -> usize {
        if self.hours.len() < 2 {
            return self.seek_to_index(0);
        }
        let scaled = fraction.clamp(0.0, 1.0) * (self.hours.len() - 1) as f64;
        self.seek_to_index(scaled.round() as usize)
    }

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
        assert_eq!(SPEED_STEPS_HOURS_PER_SEC, [1.0, 2.0, 4.0, 12.0, 48.0]);
        let mut t = Transport::without_dates(vec![0, 1, 2]).unwrap();
        assert_eq!(t.speed_step(), DEFAULT_SPEED_STEP);
        assert_eq!(t.speed_hours_per_sec(), 4.0);
        t.speed_down();
        t.speed_down();
        t.speed_down();
        assert_eq!(t.speed_step(), 0);
        assert_eq!(t.speed_hours_per_sec(), 1.0);
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
        t.set_speed_step(2);
        assert!(!t.advance(0.1));
        t.toggle_play();
        assert!(!t.is_playing());
        t.toggle_play();
        assert!(t.is_playing());
        assert!(!t.advance(0.1));
    }

    #[test]
    fn advance_steps_and_autopauses_at_end() {
        let mut t = Transport::without_dates(vec![0, 1, 2]).unwrap();
        t.set_speed_step(2);
        t.set_playing(true);
        assert!(t.advance(1.0));
        assert_eq!(t.current_index(), 2);
        assert!(!t.is_playing(), "auto-paused at last hour");
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
        assert_eq!(t.step_backward(), 0);
        assert_eq!(t.step_forward(), 1);
        assert_eq!(t.step_forward(), 2);
        assert_eq!(t.step_forward(), 2);
        assert_eq!(t.current_hour(), 30);
    }

    #[test]
    fn seek_to_hour_is_at_or_before_and_clamped() {
        let mut t = Transport::without_dates(vec![100, 200, 300]).unwrap();
        assert_eq!(t.seek_to_hour(50), 0);
        assert_eq!(t.seek_to_hour(200), 1);
        assert_eq!(t.seek_to_hour(250), 1);
        assert_eq!(t.seek_to_hour(9999), 2);
    }

    #[test]
    fn seek_to_fraction_index_spaced() {
        let mut t = Transport::without_dates((0..11).collect()).unwrap();
        assert_eq!(t.seek_to_fraction(0.0), 0);
        assert_eq!(t.seek_to_fraction(0.5), 5);
        assert_eq!(t.seek_to_fraction(1.0), 10);
        assert_eq!(t.seek_to_fraction(-1.0), 0);
        assert_eq!(t.seek_to_fraction(2.0), 10);
    }

    #[test]
    fn jump_to_date_resolves_at_or_before_and_clamps() {
        let t0 = "1936.1.1.0";
        let t1 = "1936.1.1.12";
        let t2 = "1936.1.2.0";
        let mut t = dated(&[0, 12, 24], &[t0, t1, t2]);
        assert_eq!(t.jump_to_date("1936.1.1.12").unwrap(), 1);
        assert_eq!(t.jump_to_date("1936.1.1.6").unwrap(), 0);
        assert_eq!(t.jump_to_date("1935.12.31").unwrap(), 0);
        assert_eq!(t.jump_to_date("1990.1.1").unwrap(), 2);
        assert_eq!(t.jump_to_date("1936.1.2").unwrap(), 2);
        assert!(t.jump_to_date("not-a-date").is_err());
    }

    #[test]
    fn date_ticks_one_per_calendar_day() {
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
