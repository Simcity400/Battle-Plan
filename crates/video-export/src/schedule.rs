use crate::AUDIO_SAMPLE_RATE;
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StretchJoin {
    Start,
    Load,
    Break,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedStretch {
    pub first: i32,
    pub last: i32,
    pub join: StretchJoin,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnheldGap {
    pub first: i32,
    pub last: i32,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRequest {
    pub from_hour: i32,
    pub to_hour: i32,
    pub fps: u32,
    pub hours_per_second: f64,
    pub skip_gaps: bool,
    pub cut_card_seconds: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CutNotice {
    pub from_hour: i32,
    pub to_hour: i32,
    pub omitted_hours: u32,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FrameKind {
    Recorded {
        hour: i32,
        fraction: f64,
        reset_continuity: bool,
        cut_banner: Option<CutNotice>,
    },
    GapCard {
        hour: i32,
        gap_first: i32,
        gap_last: i32,
        reason: String,
    },
    SkipCutCard(CutNotice),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScheduledFrame {
    pub frame_index: u64,
    pub kind: FrameKind,
    pub presentation_time: f64,
    pub audio_frames: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadenceInfo {
    pub hours_per_frame: f64,
    pub uneven: bool,
    pub suggested_hours_per_second: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleSummary {
    pub from_hour: i32,
    pub to_hour: i32,
    pub recorded_hours: u64,
    pub missing_hours: u64,
    pub gap_count: usize,
    pub total_frames: u64,
    pub output_seconds: f64,
    pub cadence: CadenceInfo,
}

#[derive(Clone, Debug)]
pub struct FrameSchedule {
    pub fps: u32,
    pub hours_per_second: f64,
    pub summary: ScheduleSummary,
    pub frames: Vec<ScheduledFrame>,
}

pub fn audio_frames_for_index(frame_index: u64, fps: u32) -> usize {
    let fps = u64::from(fps.max(1));
    let rate = u64::from(AUDIO_SAMPLE_RATE);
    let prev = frame_index.saturating_mul(rate) / fps;
    let next = frame_index.saturating_add(1).saturating_mul(rate) / fps;
    (next - prev) as usize
}

pub fn cadence_info(hours_per_second: f64, fps: u32) -> CadenceInfo {
    let fps_f = f64::from(fps.max(1));
    let hpf = hours_per_second / fps_f;
    if hpf >= 1.0 {
        let nearest = hpf.round();
        let uneven = (hpf - nearest).abs() > 1e-4;
        CadenceInfo {
            hours_per_frame: hpf,
            uneven,
            suggested_hours_per_second: uneven.then_some(nearest * fps_f),
        }
    } else if hpf > 0.0 {
        let frames_per_hour = 1.0 / hpf;
        let nearest = frames_per_hour.round().max(1.0);
        let uneven = (frames_per_hour - nearest).abs() > 1e-3;
        CadenceInfo {
            hours_per_frame: hpf,
            uneven,
            suggested_hours_per_second: uneven.then_some(fps_f / nearest),
        }
    } else {
        CadenceInfo {
            hours_per_frame: 0.0,
            uneven: false,
            suggested_hours_per_second: None,
        }
    }
}

pub fn merge_recorded_tracks(tracks: &[Vec<RecordedStretch>]) -> Vec<RecordedStretch> {
    let mut intervals: Vec<(i32, i32, StretchJoin)> = tracks
        .iter()
        .flat_map(|track| track.iter().map(|s| (s.first, s.last, s.join)))
        .collect();
    intervals.sort_unstable_by_key(|&(first, last, _)| (first, last));
    let mut merged: Vec<RecordedStretch> = Vec::new();
    for (first, last, join) in intervals {
        if let Some(prev) = merged.last_mut()
            && i64::from(first) <= i64::from(prev.last) + 1
        {
            if last > prev.last {
                prev.last = last;
            }
        } else {
            merged.push(RecordedStretch {
                first,
                last,
                join: if merged.is_empty() { StretchJoin::Start } else { join },
            });
        }
    }
    merged
}

pub fn build_schedule(
    stretches: &[RecordedStretch],
    unheld: &[UnheldGap],
    request: &ScheduleRequest,
) -> io::Result<FrameSchedule> {
    if request.from_hour > request.to_hour {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Export start hour cannot be after end hour",
        ));
    }
    if !(1..=240).contains(&request.fps) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Unsupported export frame rate: {} fps", request.fps),
        ));
    }
    if !request.hours_per_second.is_finite() || request.hours_per_second <= 0.0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Export speed must be a finite positive number of hours per second",
        ));
    }
    let clipped_stretches: Vec<RecordedStretch> = stretches
        .iter()
        .filter(|s| s.last >= request.from_hour && s.first <= request.to_hour)
        .map(|s| RecordedStretch {
            first: s.first.max(request.from_hour),
            last: s.last.min(request.to_hour),
            join: s.join,
        })
        .collect();
    if clipped_stretches.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "The selected time range holds no recorded hours to export",
        ));
    }
    let recorded_hours: u64 = clipped_stretches
        .iter()
        .map(|s| (i64::from(s.last) - i64::from(s.first) + 1) as u64)
        .sum();
    let total_span_hours = (i64::from(request.to_hour) - i64::from(request.from_hour) + 1) as u64;
    let missing_hours = total_span_hours.saturating_sub(recorded_hours);
    let clipped_gaps: Vec<UnheldGap> = unheld
        .iter()
        .filter(|g| g.last >= request.from_hour && g.first <= request.to_hour)
        .map(|g| UnheldGap {
            first: g.first.max(request.from_hour),
            last: g.last.min(request.to_hour),
            reason: g.reason.clone(),
        })
        .collect();

    let fps_f = f64::from(request.fps);
    let step_hours = request.hours_per_second / fps_f;
    let cut_frames = if request.cut_card_seconds.is_finite() && request.cut_card_seconds > 0.0 {
        (request.cut_card_seconds * fps_f).round().max(1.0) as u32
    } else {
        0
    };

    let mut frames = Vec::new();
    let fps = request.fps;
    fn push_frame(frames: &mut Vec<ScheduledFrame>, fps: u32, kind: FrameKind) {
        let frame_index = frames.len() as u64;
        frames.push(ScheduledFrame {
            frame_index,
            kind,
            presentation_time: frame_index as f64 / f64::from(fps),
            audio_frames: audio_frames_for_index(frame_index, fps),
        });
    }

    let mut gap_count = 0usize;
    if request.skip_gaps {
        for (idx, stretch) in clipped_stretches.iter().enumerate() {
            if idx > 0 {
                let prev = &clipped_stretches[idx - 1];
                let gap_first = prev.last + 1;
                let gap_last = stretch.first - 1;
                if gap_first <= gap_last {
                    gap_count += 1;
                    let omitted = (i64::from(gap_last) - i64::from(gap_first) + 1) as u32;
                    let reason = gap_reason_at(&clipped_gaps, gap_first);
                    let notice = CutNotice {
                        from_hour: prev.last,
                        to_hour: stretch.first,
                        omitted_hours: omitted,
                        reason,
                    };
                    for _ in 0..cut_frames {
                        push_frame(&mut frames, fps, FrameKind::SkipCutCard(notice.clone()));
                    }
                }
            }
            let span = f64::from(stretch.last - stretch.first);
            let count = if span <= 0.0 {
                1u64
            } else {
                (span / step_hours).floor() as u64 + 1
            };
            for local in 0..count {
                let pos = (f64::from(stretch.first) + local as f64 * step_hours)
                    .min(f64::from(stretch.last));
                let whole = pos.floor() as i32;
                let fraction = (pos - f64::from(whole)).clamp(0.0, 1.0 - f64::EPSILON);
                let reset = local == 0;
                push_frame(
                    &mut frames,
                    fps,
                    FrameKind::Recorded {
                        hour: whole,
                        fraction,
                        reset_continuity: reset,
                        cut_banner: None,
                    },
                );
            }
            if let Some(last_frame) = frames.last()
                && let FrameKind::Recorded { hour, .. } = last_frame.kind
                && hour < stretch.last
            {
                push_frame(
                    &mut frames,
                    fps,
                    FrameKind::Recorded {
                        hour: stretch.last,
                        fraction: 0.0,
                        reset_continuity: false,
                        cut_banner: None,
                    },
                );
            }
        }
    } else {
        gap_count = clipped_gaps.len();
        let span = f64::from(request.to_hour - request.from_hour);
        let count = if span <= 0.0 {
            1u64
        } else {
            (span / step_hours).floor() as u64 + 1
        };
        let mut prev_recorded_hour: Option<i32> = None;
        for local in 0..count {
            let pos = (f64::from(request.from_hour) + local as f64 * step_hours)
                .min(f64::from(request.to_hour));
            let whole = pos.floor() as i32;
            let fraction = (pos - f64::from(whole)).clamp(0.0, 1.0 - f64::EPSILON);
            if stretch_at(&clipped_stretches, whole).is_some() {
                let traversed_gap = prev_recorded_hour.and_then(|prev| {
                    find_traversed_gap(&clipped_gaps, prev, whole)
                });
                let traversed_join = prev_recorded_hour.is_none_or(|prev| {
                    has_discontinuity(&clipped_stretches, prev, whole)
                });
                let cut_banner = traversed_gap.map(|g| CutNotice {
                    from_hour: g.first - 1,
                    to_hour: g.last + 1,
                    omitted_hours: (i64::from(g.last) - i64::from(g.first) + 1) as u32,
                    reason: g.reason.clone(),
                });
                push_frame(
                    &mut frames,
                    fps,
                    FrameKind::Recorded {
                        hour: whole,
                        fraction,
                        reset_continuity: traversed_join || cut_banner.is_some(),
                        cut_banner,
                    },
                );
                prev_recorded_hour = Some(whole);
            } else {
                let (gap_first, gap_last, reason) = gap_bounds_at(&clipped_gaps, whole, request.from_hour, request.to_hour);
                push_frame(
                    &mut frames,
                    fps,
                    FrameKind::GapCard {
                        hour: whole,
                        gap_first,
                        gap_last,
                        reason,
                    },
                );
                prev_recorded_hour = None;
            }
        }
        if let Some(last_frame) = frames.last() {
            let last_hour = match &last_frame.kind {
                FrameKind::Recorded { hour, .. } | FrameKind::GapCard { hour, .. } => Some(*hour),
                FrameKind::SkipCutCard(_) => None,
            };
            if last_hour.is_some_and(|h| h < request.to_hour) {
                let whole = request.to_hour;
                if stretch_at(&clipped_stretches, whole).is_some() {
                    let traversed_join = prev_recorded_hour.is_none_or(|prev| {
                        has_discontinuity(&clipped_stretches, prev, whole)
                    });
                    push_frame(
                        &mut frames,
                        fps,
                        FrameKind::Recorded {
                            hour: whole,
                            fraction: 0.0,
                            reset_continuity: traversed_join,
                            cut_banner: None,
                        },
                    );
                } else {
                    let (gap_first, gap_last, reason) =
                        gap_bounds_at(&clipped_gaps, whole, request.from_hour, request.to_hour);
                    push_frame(
                        &mut frames,
                        fps,
                        FrameKind::GapCard {
                            hour: whole,
                            gap_first,
                            gap_last,
                            reason,
                        },
                    );
                }
            }
        }
    }

    let total_frames = frames.len() as u64;
    let summary = ScheduleSummary {
        from_hour: request.from_hour,
        to_hour: request.to_hour,
        recorded_hours,
        missing_hours,
        gap_count,
        total_frames,
        output_seconds: total_frames as f64 / fps_f,
        cadence: cadence_info(request.hours_per_second, request.fps),
    };
    Ok(FrameSchedule {
        fps: request.fps,
        hours_per_second: request.hours_per_second,
        summary,
        frames,
    })
}

fn stretch_at(stretches: &[RecordedStretch], hour: i32) -> Option<&RecordedStretch> {
    let idx = stretches.partition_point(|s| s.last < hour);
    stretches.get(idx).filter(|s| s.first <= hour)
}

fn has_discontinuity(stretches: &[RecordedStretch], prev: i32, next: i32) -> bool {
    if next < prev {
        return true;
    }
    let Some(a) = stretch_at(stretches, prev) else {
        return true;
    };
    let Some(b) = stretch_at(stretches, next) else {
        return true;
    };
    a.first != b.first || a.last != b.last
}

fn find_traversed_gap<'a>(gaps: &'a [UnheldGap], prev: i32, next: i32) -> Option<&'a UnheldGap> {
    gaps.iter().find(|g| g.first > prev && g.last < next)
}

fn gap_reason_at(gaps: &[UnheldGap], hour: i32) -> String {
    gaps.iter()
        .find(|g| g.first <= hour && hour <= g.last)
        .map_or_else(
            || "Not recorded: this play happened while the recorder wasn't watching".to_owned(),
            |g| g.reason.clone(),
        )
}

fn gap_bounds_at(gaps: &[UnheldGap], hour: i32, from: i32, to: i32) -> (i32, i32, String) {
    gaps.iter()
        .find(|g| g.first <= hour && hour <= g.last)
        .map_or_else(
            || (from, to, "Not recorded".to_owned()),
            |g| (g.first, g.last, g.reason.clone()),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_stretches() -> (Vec<RecordedStretch>, Vec<UnheldGap>) {
        (
            vec![
                RecordedStretch { first: 100, last: 199, join: StretchJoin::Start },
                RecordedStretch { first: 200, last: 249, join: StretchJoin::Load },
                RecordedStretch { first: 300, last: 399, join: StretchJoin::Break },
            ],
            vec![UnheldGap {
                first: 250,
                last: 299,
                reason: "Recorder stopped".into(),
            }],
        )
    }

    #[test]
    fn audio_frames_accumulate_without_drift_at_30_and_60_fps() {
        let total60: usize = (0..60).map(|i| audio_frames_for_index(i, 60)).sum();
        assert_eq!(total60, 44_100);
        assert_eq!(audio_frames_for_index(0, 60), 735);
        let total30: usize = (0..30).map(|i| audio_frames_for_index(i, 30)).sum();
        assert_eq!(total30, 44_100);
        assert_eq!(audio_frames_for_index(0, 30), 1_470);
    }

    #[test]
    fn skip_mode_inserts_dated_cut_cards_and_resets_continuity_on_load_and_break() {
        let (stretches, gaps) = sample_stretches();
        let schedule = build_schedule(
            &stretches,
            &gaps,
            &ScheduleRequest {
                from_hour: 195,
                to_hour: 305,
                fps: 60,
                hours_per_second: 60.0,
                skip_gaps: true,
                cut_card_seconds: 0.5,
            },
        )
        .unwrap();
        assert_eq!(schedule.summary.gap_count, 1);
        assert_eq!(schedule.summary.missing_hours, 50);
        let cut_cards = schedule
            .frames
            .iter()
            .filter(|f| matches!(f.kind, FrameKind::SkipCutCard(_)))
            .count();
        assert_eq!(cut_cards, 30);
        let load_frame = schedule
            .frames
            .iter()
            .find(|f| matches!(f.kind, FrameKind::Recorded { hour: 200, .. }))
            .unwrap();
        assert!(matches!(
            load_frame.kind,
            FrameKind::Recorded { reset_continuity: true, .. }
        ));
    }

    #[test]
    fn high_speed_linear_mode_detects_short_traversed_gap_and_resets_continuity() {
        let stretches = vec![
            RecordedStretch { first: 100, last: 149, join: StretchJoin::Start },
            RecordedStretch { first: 155, last: 300, join: StretchJoin::Break },
        ];
        let gaps = vec![UnheldGap { first: 150, last: 154, reason: "Short gap".into() }];
        let schedule = build_schedule(
            &stretches,
            &gaps,
            &ScheduleRequest {
                from_hour: 100,
                to_hour: 300,
                fps: 30,
                hours_per_second: 600.0,
                skip_gaps: false,
                cut_card_seconds: 0.0,
            },
        )
        .unwrap();
        assert!(schedule.frames.iter().any(|f| matches!(
            &f.kind,
            FrameKind::Recorded { reset_continuity: true, cut_banner: Some(cut), .. }
            if cut.omitted_hours == 5
        )));
    }

    #[test]
    fn completely_unrecorded_selection_is_refused_and_single_hour_succeeds() {
        let (stretches, gaps) = sample_stretches();
        let err = build_schedule(
            &stretches,
            &gaps,
            &ScheduleRequest {
                from_hour: 255,
                to_hour: 290,
                fps: 60,
                hours_per_second: 60.0,
                skip_gaps: false,
                cut_card_seconds: 0.5,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("no recorded hours"));

        let one = build_schedule(
            &stretches,
            &gaps,
            &ScheduleRequest {
                from_hour: 150,
                to_hour: 150,
                fps: 60,
                hours_per_second: 24.0,
                skip_gaps: false,
                cut_card_seconds: 0.5,
            },
        )
        .unwrap();
        assert_eq!(one.frames.len(), 1);
    }

    #[test]
    fn cadence_flags_uneven_24_hps_at_60_fps_and_suggests_clean_snap() {
        let info = cadence_info(24.0, 60);
        assert!(info.uneven);
        assert_eq!(info.suggested_hours_per_second, Some(20.0));
        assert!(!cadence_info(30.0, 60).uneven);
        assert!(!cadence_info(60.0, 60).uneven);
    }
}
