use super::distribution;
use serde_json::{Value, json};

/// Microseconds after the first frame's start at which frame `index` is due.
pub(crate) fn due_us(index: u64, refresh_hz: f64) -> f64 {
    index as f64 * 1e6 / refresh_hz
}

/// A frame reached after its due time starts at once, late; the first frame's start is the clock's zero.
pub(crate) fn late_start(index: u64, due_us: f64, reached_us: f64) -> bool {
    index > 0 && reached_us > due_us
}

/// The head `elapsed_s` seconds after `from` at `speed` hours a second, held at the branch's last hour.
pub(crate) fn head(from: i32, speed: f64, elapsed_s: f64, last: i32) -> f64 {
    (f64::from(from) + elapsed_s * speed).min(f64::from(last))
}

/// The head's whole hour and the fraction past it.
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
    pub thread_us: Option<f64>,
    pub head_hour: f64,
    /// The hour the window clock shows drawn.
    pub drawn_hour: Option<i32>,
    /// The oldest hour on screen, every layer and the overlay counted; with no map of layers, the
    /// drawn hour.
    pub oldest_hour: Option<i32>,
    pub late_start: bool,
    pub new_hour: bool,
}

impl Frame {
    /// The head less the oldest hour on screen.
    pub(crate) fn lag_hours(&self) -> Option<f64> {
        self.oldest_hour
            .map(|oldest| self.head_hour - f64::from(oldest))
    }

    /// The head less the window clock's drawn hour, which leaves out the layers whose product is
    /// made on a worker from rows at the head, and the overlay.
    pub(crate) fn clock_lag_hours(&self) -> Option<f64> {
        self.drawn_hour
            .map(|drawn| self.head_hour - f64::from(drawn))
    }
}

/// The paced run over the frames after warm-up (those before the first drawn hour). `end_us`
/// is when the head reached the run's end.
pub(crate) fn summary(frames: &[Frame], refresh_hz: f64, end_us: f64) -> Value {
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
    // A late start the frame before explains by its own work and GPU wait; the rest is the harness's.
    let unexplained = late_starts
        .clone()
        .filter(|&k| {
            let before = &frames[k - 1];
            before.start_us + before.work_us + before.gpu_wait_us <= frames[k].due_us
        })
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
        "late_frames": measured.iter().filter(|frame| frame.work_us > interval).count(),
        "late_starts": late_starts.count(),
        "late_starts_unexplained": unexplained,
        "lag_hours": distribution(measured.iter().filter_map(Frame::lag_hours).collect()),
        "lag_hours_without_cadence_layers_or_overlay": distribution(measured.iter().filter_map(Frame::clock_lag_hours).collect()),
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
            thread_us: Some(work_us / 2.0),
            head_hour: head,
            drawn_hour: drawn,
            oldest_hour: drawn,
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
    fn the_head_runs_at_the_speed_from_its_start_and_stops_at_the_branch_end() {
        assert_eq!(head(100, 1440.0, 0.0, 10_000), 100.0);
        assert_eq!(head(100, 1440.0, 0.5, 10_000), 820.0);
        assert_eq!(head(100, 24.0, 1.0 / 48.0, 10_000), 100.5);
        assert_eq!(head(100, 1440.0, 60.0, 10_000), 10_000.0);
        assert_eq!(split(820.25), (820, 0.25));
        assert_eq!(split(60_828_491.0), (60_828_491, 0.0));
    }

    #[test]
    fn a_new_hour_is_a_drawn_hour_that_differs_from_the_one_before() {
        assert!(!new_hour(None, None));
        assert!(new_hour(None, Some(5)));
        assert!(!new_hour(Some(5), Some(5)));
        assert!(new_hour(Some(5), Some(9)));
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
        // Frame 3 started late because frame 2 ran past its due time.
        frames[3].due_us = 50_000.0;
        frames[3].late_start = true;
        // Frame 4 started late though frame 3's work ended before its due time: the harness's own.
        frames[4].due_us = 70_000.0;
        frames[4].start_us = 72_000.0;
        frames[4].late_start = true;
        mark_new(&mut frames);
        let report = summary(&frames, 60.0, 100_000.0);
        assert_eq!(report["frames"], 6);
        assert_eq!(report["warmup_frames"], 2);
        assert_eq!(report["warmup_ms"], 33.333);
        assert_eq!(report["measured_frames"], 4);
        assert_eq!(report["late_frames"], 1);
        assert_eq!(report["late_starts"], 2);
        assert_eq!(report["late_starts_unexplained"], 1);
        assert_eq!(report["shown_hours"], 3);
        assert_eq!(report["new_hour_frames"], 3);
        assert_eq!(report["drawn_past_head_frames"], 0);
        assert_eq!(report["lag_hours"]["max"], 82.0);
        assert_eq!(report["lag_hours"]["p50"], 38.0);
        assert_eq!(report["work_us"]["max"], 30_000.0);
        assert_eq!(report["thread_us"]["max"], 15_000.0);
        // 110 at 33.3 ms, 150 at 72, 190 at 83.3, the run's end at 100.
        assert!((report["longest_without_a_new_hour_ms"].as_f64().unwrap() - 38.667).abs() < 1e-9);
        let per_second = report["shown_hours_per_second"].as_f64().unwrap();
        assert!((per_second - 3.0 / 0.066_667).abs() < 1e-6);
    }

    /// The lag is the head less the oldest hour on screen, every layer and the overlay counted; the
    /// window clock's lag, which leaves them out, is kept beside it.
    #[test]
    fn the_lag_counts_the_oldest_hour_on_screen_and_keeps_the_window_clocks_beside_it() {
        let mut frames = vec![frame(0.0, 1.0, Some(100), 110.0), frame(16_667.0, 1.0, Some(110), 120.0)];
        frames[0].oldest_hour = Some(96);
        frames[1].oldest_hour = Some(101);
        mark_new(&mut frames);
        let report = summary(&frames, 60.0, 33_333.0);
        assert_eq!(report["lag_hours"]["max"], 19.0);
        assert_eq!(report["lag_hours_without_cadence_layers_or_overlay"]["max"], 10.0);
    }

    #[test]
    fn a_frame_without_its_window_thread_time_withholds_the_thread_distribution() {
        let mut frames = vec![frame(0.0, 1.0, Some(1), 1.0), frame(1.0, 1.0, Some(1), 1.0)];
        frames[1].thread_us = None;
        mark_new(&mut frames);
        assert_eq!(summary(&frames, 60.0, 2.0)["thread_us"]["p50"], Value::Null);
    }

    #[test]
    fn an_empty_run_reports_nothing_measured() {
        let report = summary(&[], 60.0, 0.0);
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
        let report = summary(&frames, 60.0, 70_000.0);
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
