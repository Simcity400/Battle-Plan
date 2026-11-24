use super::distribution;
use serde_json::{Value, json};

pub(crate) fn due_us(index: u64, refresh_hz: f64) -> f64 {
    index as f64 * 1e6 / refresh_hz
}

pub(crate) fn late_start(index: u64, due_us: f64, reached_us: f64) -> bool {
    index > 0 && reached_us > due_us
}

pub(crate) fn head(from: i32, speed: f64, elapsed_s: f64, last: i32) -> f64 {
    (f64::from(from) + elapsed_s * speed).min(f64::from(last))
}

pub(crate) fn seek_back_end(back: i32, speed: f64, seconds: f64, last: i32) -> f64 {
    (f64::from(back) + seconds * speed).min(f64::from(last))
}

pub(crate) fn split(head: f64) -> (i32, f64) {
    let hour = head.floor();
    (hour as i32, head - hour)
}

pub(crate) fn new_hour(previous: Option<i32>, drawn: Option<i32>) -> bool {
    drawn.is_some() && drawn != previous
}

#[derive(Clone, Debug)]
pub(crate) struct Frame {
    pub due_us: f64,
    pub start_us: f64,
    pub work_us: f64,
    pub gpu_wait_us: f64,
    pub interval_us: Option<f64>,
    pub acquire_us: Option<f64>,
    pub thread_us: Option<f64>,
    pub head_hour: f64,
    pub drawn_hour: Option<i32>,
    pub oldest_hour: Option<i32>,
    pub overlay_hour: Option<i32>,
    pub made: Option<u64>,
    pub late_start: bool,
    pub new_hour: bool,
}

impl Frame {
    pub(crate) fn lag_hours(&self) -> Option<f64> {
        self.oldest_hour
            .map(|oldest| self.head_hour - f64::from(oldest))
    }

    pub(crate) fn clock_lag_hours(&self) -> Option<f64> {
        self.drawn_hour
            .map(|drawn| self.head_hour - f64::from(drawn))
    }

    pub(crate) fn late_ms(&self, speed: f64) -> Option<f64> {
        self.lag_hours().filter(|_| speed > 0.0).map(|hours| hours / speed * 1e3)
    }

    pub(crate) fn overlay_behind_hours(&self) -> Option<f64> {
        self.drawn_hour
            .zip(self.overlay_hour)
            .map(|(drawn, overlay)| f64::from(drawn) - f64::from(overlay))
    }
}

pub(crate) fn summary(frames: &[Frame], refresh_hz: f64, end_us: f64, speed: f64) -> Value {
    let interval = 1e6 / refresh_hz;
    let warmup = frames
        .iter()
        .position(|frame| frame.drawn_hour.is_some())
        .unwrap_or(frames.len());
    let measured = &frames[warmup..];
    let warmup_ms = measured.first().map(|frame| frame.start_us / 1e3);
    let thread = measured
        .iter()
        .map(|frame| frame.thread_us)
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();
    let late_starts = (warmup.max(1)..frames.len()).filter(|&k| frames[k].late_start);
    let unexplained = late_starts
        .clone()
        .filter(|&k| {
            let before = &frames[k - 1];
            before.start_us + before.work_us + before.gpu_wait_us <= frames[k].due_us
        })
        .count();
    let behind: Vec<f64> = measured.iter().filter_map(Frame::overlay_behind_hours).collect();
    let behind_ms = if speed > 0.0 { behind.iter().map(|hours| hours / speed * 1e3).collect() } else { Vec::new() };
    let overlay_moved = measured
        .windows(2)
        .filter(|pair| pair[0].overlay_hour.is_some() && pair[1].overlay_hour.is_some())
        .filter(|pair| pair[0].overlay_hour != pair[1].overlay_hour)
        .count();
    let mut publications: Vec<(u64, Option<i32>)> = Vec::new();
    for frame in measured.iter() {
        match frame.made {
            Some(made) if publications.last().is_none_or(|(last, _)| *last != made) => publications.push((made, frame.oldest_hour)),
            _ => {}
        }
    }
    let with_a_new_hour = publications
        .windows(2)
        .filter(|pair| pair[1].1.is_some() && pair[1].1 != pair[0].1)
        .count();
    let mut shown = measured
        .iter()
        .filter_map(|frame| frame.drawn_hour)
        .collect::<Vec<_>>();
    shown.sort_unstable();
    shown.dedup();
    let mut longest = None::<f64>;
    let mut since = None::<f64>;
    for frame in measured.iter().filter(|frame| frame.new_hour) {
        if let Some(since) = since {
            longest = Some(longest.unwrap_or(0.0).max(frame.start_us - since));
        }
        since = Some(frame.start_us);
    }
    if let Some(since) = since {
        longest = Some(longest.unwrap_or(0.0).max(end_us - since));
    }
    let seconds = warmup_ms.map(|ms| (end_us - ms * 1e3) / 1e6);
    json!({
        "frames": frames.len(),
        "seconds": end_us / 1e6,
        "warmup_frames": warmup,
        "warmup_ms": warmup_ms,
        "measured_frames": measured.len(),
        "measured_seconds": seconds,
        "work_us": distribution(measured.iter().map(|frame| frame.work_us).collect()),
        "thread_us": distribution(thread),
        "gpu_wait_us": distribution(measured.iter().map(|frame| frame.gpu_wait_us).collect()),
        "interval_us": distribution(measured.iter().filter_map(|frame| frame.interval_us).collect()),
        "acquire_us": distribution(measured.iter().filter_map(|frame| frame.acquire_us).collect()),
        "intervals_over_one_and_a_half_frames": measured.iter().filter_map(|frame| frame.interval_us).filter(|us| *us > interval * 1.5).count(),
        "late_frames": measured.iter().filter(|frame| frame.work_us > interval).count(),
        "late_starts": late_starts.count(),
        "late_starts_unexplained": unexplained,
        "speed_hours_per_second": speed,
        "lag_hours": distribution(measured.iter().filter_map(Frame::lag_hours).collect()),
        "late_ms": distribution(measured.iter().filter_map(|frame| frame.late_ms(speed)).collect()),
        "publications": publications.len(),
        "publications_per_second": seconds.filter(|s| *s > 0.0).map(|s| publications.len() as f64 / s),
        "publications_with_a_new_hour": with_a_new_hour,
        "publications_with_a_new_hour_per_second": seconds.filter(|s| *s > 0.0).map(|s| with_a_new_hour as f64 / s),
        "head_behind_the_clock_hours": distribution(measured.iter().filter_map(Frame::clock_lag_hours).collect()),
        "overlay_behind_hours": distribution(behind),
        "overlay_behind_ms": distribution(behind_ms),
        "overlay_new_hour_frames": overlay_moved,
        "drawn_past_head_frames": measured.iter().filter(|frame| frame.lag_hours().is_some_and(|lag| lag < 0.0)).count(),
        "shown_hours": shown.len(),
        "new_hour_frames": measured.iter().filter(|frame| frame.new_hour).count(),
        "shown_hours_per_second": seconds.filter(|s| *s > 0.0).map(|s| shown.len() as f64 / s),
        "longest_without_a_new_hour_ms": longest.map(|us| us / 1e3),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(start_us: f64, work_us: f64, drawn: Option<i32>, head: f64) -> Frame {
        Frame {
            due_us: start_us,
            start_us,
            work_us,
            gpu_wait_us: 0.0,
            interval_us: None,
            acquire_us: None,
            thread_us: Some(work_us / 2.0),
            head_hour: head,
            drawn_hour: drawn,
            oldest_hour: drawn,
            overlay_hour: drawn,
            made: drawn.map(|hour| hour as u64),
            late_start: false,
            new_hour: false,
        }
    }

    fn mark_new(frames: &mut [Frame]) {
        let mut previous = None;
        for frame in frames {
            frame.new_hour = new_hour(previous, frame.drawn_hour);
            previous = frame.drawn_hour.or(previous);
        }
    }

    #[test]
    fn frames_fall_due_on_the_refresh_grid_and_only_a_frame_reached_after_its_due_time_starts_late()
    {
        assert_eq!(due_us(0, 60.0), 0.0);
        assert!((due_us(60, 60.0) - 1e6).abs() < 1e-6);
        assert!((due_us(3, 50.0) - 60_000.0).abs() < 1e-6);
        assert!(!late_start(0, 0.0, 5.0));
        assert!(!late_start(2, 33_333.3, 33_000.0));
        assert!(!late_start(2, 33_333.3, 33_333.3));
        assert!(late_start(2, 33_333.3, 40_000.0));
    }

    #[test]
    fn a_run_sought_back_near_the_branch_end_ends_past_its_seek_where_the_head_can_reach() {
        for (back, speed, ends) in [(990, 60.0, 1000.0), (999, 0.75, 1000.0), (900, 2.5, 905.0), (900, 0.3, 900.6), (998, 0.75, 999.5)] {
            let target = seek_back_end(back, speed, 2.0, 1000);
            assert!((target - ends).abs() < 1e-9, "{back} at {speed}: {target}");
            assert!(target > f64::from(back), "{back} at {speed}: the run plays on past its seek");
            let frames = (f64::from(1000 - back) / speed * 60.0).ceil() as u32;
            for whole in [false, true] {
                let reached = (0..=frames).map(|frame| head(back, speed, f64::from(frame) / 60.0, 1000)).map(|head| if whole { head.floor() } else { head });
                assert!(reached.clone().any(|head| head >= target), "{back} at {speed}, whole hours {whole}: never reaches {target}");
                assert!(reached.take(1).all(|head| head < target), "{back} at {speed}: the seek's own frame is shown");
            }
        }
    }

    #[test]
    fn the_summary_leaves_out_warmup_and_counts_lag_shown_hours_and_late_frames() {
        let mut frames = vec![
            frame(0.0, 40_000.0, None, 100.0),
            frame(16_667.0, 5_000.0, None, 124.0),
            frame(33_333.0, 30_000.0, Some(110), 148.0),
            frame(63_333.0, 5_000.0, Some(110), 192.0),
            frame(66_667.0, 5_000.0, Some(150), 196.0),
            frame(83_333.0, 5_000.0, Some(190), 220.0),
        ];
        frames[3].due_us = 50_000.0;
        frames[3].late_start = true;
        frames[4].due_us = 70_000.0;
        frames[4].start_us = 72_000.0;
        frames[4].late_start = true;
        mark_new(&mut frames);
        let report = summary(&frames, 60.0, 100_000.0, 1.0);
        assert_eq!(report["frames"], 6);
        assert_eq!(report["warmup_frames"], 2);
        assert_eq!(report["warmup_ms"], 33.333);
        assert_eq!(report["measured_frames"], 4);
        assert_eq!(report["late_frames"], 1);
        assert_eq!(report["late_starts"], 2);
        assert_eq!(report["late_starts_unexplained"], 1);
        assert_eq!(report["shown_hours"], 3);
        assert_eq!(report["new_hour_frames"], 3);
        assert_eq!(report["publications"], 3, "the frames drew three publications, one of them twice");
        assert_eq!(report["publications_with_a_new_hour"], 2);
        assert_eq!(report["late_ms"]["max"], 82_000.0, "the lag in milliseconds of playback at one hour a second");
        assert_eq!(report["drawn_past_head_frames"], 0);
        assert_eq!(report["lag_hours"]["max"], 82.0);
        assert_eq!(report["lag_hours"]["p50"], 38.0);
        assert_eq!(report["work_us"]["max"], 30_000.0);
        assert_eq!(report["thread_us"]["max"], 15_000.0);
        assert!((report["longest_without_a_new_hour_ms"].as_f64().unwrap() - 38.667).abs() < 1e-9);
        let per_second = report["shown_hours_per_second"].as_f64().unwrap();
        assert!((per_second - 3.0 / 0.066_667).abs() < 1e-6);
    }

    #[test]
    fn the_lag_counts_the_layers_and_the_overlays_hours_behind_them_are_reported_apart() {
        let mut frames = vec![frame(0.0, 1.0, Some(100), 110.0), frame(16_667.0, 1.0, Some(110), 120.0)];
        frames[0].oldest_hour = Some(96);
        frames[1].oldest_hour = Some(101);
        frames[0].overlay_hour = Some(40);
        frames[1].overlay_hour = Some(50);
        mark_new(&mut frames);
        let report = summary(&frames, 60.0, 33_333.0, 24.0);
        assert_eq!(report["lag_hours"]["max"], 19.0, "the overlay's hour is no part of the lag");
        assert_eq!(report["head_behind_the_clock_hours"]["max"], 10.0);
        assert_eq!(report["overlay_behind_hours"]["max"], 60.0);
        assert_eq!(report["overlay_behind_ms"]["max"], 2500.0);
        assert_eq!(report["overlay_new_hour_frames"], 1);
    }

    #[test]
    fn a_frame_without_its_window_thread_time_withholds_the_thread_distribution() {
        let mut frames = vec![frame(0.0, 1.0, Some(1), 1.0), frame(1.0, 1.0, Some(1), 1.0)];
        frames[1].thread_us = None;
        mark_new(&mut frames);
        assert_eq!(summary(&frames, 60.0, 2.0, 1.0)["thread_us"]["p50"], Value::Null);
    }

    #[test]
    fn an_empty_run_reports_nothing_measured() {
        let report = summary(&[], 60.0, 0.0, 1.0);
        assert_eq!(report["frames"], 0);
        assert_eq!(report["warmup_frames"], 0);
        assert_eq!(report["warmup_ms"], Value::Null);
        assert_eq!(report["work_us"]["p50"], Value::Null);
        assert_eq!(report["lag_hours"]["max"], Value::Null);
        assert_eq!(report["shown_hours"], 0);
        assert_eq!(report["shown_hours_per_second"], Value::Null);
        assert_eq!(report["longest_without_a_new_hour_ms"], Value::Null);
    }

    #[test]
    fn a_run_that_never_draws_an_hour_is_all_warmup() {
        let mut frames = (0..4)
            .map(|k| {
                frame(
                    f64::from(k) * 16_667.0,
                    50_000.0,
                    None,
                    100.0 + f64::from(k) * 24.0,
                )
            })
            .collect::<Vec<_>>();
        frames[2].late_start = true;
        mark_new(&mut frames);
        let report = summary(&frames, 60.0, 70_000.0, 1.0);
        assert_eq!(report["frames"], 4);
        assert_eq!(report["warmup_frames"], 4);
        assert_eq!(report["measured_frames"], 0);
        assert_eq!(report["warmup_ms"], Value::Null);
        assert_eq!(report["late_frames"], 0);
        assert_eq!(report["late_starts"], 0);
        assert_eq!(report["work_us"]["max"], Value::Null);
        assert_eq!(report["lag_hours"]["p50"], Value::Null);
        assert_eq!(report["shown_hours"], 0);
        assert_eq!(report["new_hour_frames"], 0);
        assert_eq!(report["shown_hours_per_second"], Value::Null);
        assert_eq!(report["longest_without_a_new_hour_ms"], Value::Null);
    }
}
