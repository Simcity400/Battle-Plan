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
        crossed_gap: Option<CutNotice>,
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

#[derive(Clone, Debug, PartialEq)]
pub struct PreviewFrame {
    pub frame: ScheduledFrame,
    pub seed_frames: Vec<ScheduledFrame>,
}

#[derive(Clone, Debug)]
pub struct FrameSchedule {
    pub fps: u32,
    pub hours_per_second: f64,
    pub skip_gaps: bool,
    pub cut_frames: u32,
    pub summary: ScheduleSummary,
    stretches: Vec<RecordedStretch>,
    gaps: Vec<UnheldGap>,
    single_fraction: Option<f64>,
}

impl FrameSchedule {
    pub fn total_frames(&self) -> u64 {
        self.summary.total_frames
    }

    pub fn first_recorded(&self) -> Option<(i32, f64)> {
        if let Some(frac) = self.single_fraction {
            return Some((self.summary.from_hour, frac));
        }
        if self.stretches.is_empty() {
            return None;
        }
        if self.skip_gaps {
            return Some((self.stretches[0].first, 0.0));
        }
        let step_hours = self.hours_per_second / f64::from(self.fps.max(1));
        let from = self.summary.from_hour;
        let to = self.summary.to_hour;
        let total_steps = steps_in_span(from, to, step_hours).ok()?;
        for stretch in &self.stretches {
            let start_delta = (f64::from(stretch.first) - f64::from(from)).max(0.0);
            let mut k = if start_delta == 0.0 {
                0u64
            } else {
                (start_delta / step_hours).floor() as u64
            };
            while k < total_steps {
                let pos = (f64::from(from) + k as f64 * step_hours).min(f64::from(to));
                let whole = pos.floor() as i32;
                if whole > stretch.last {
                    break;
                }
                if whole >= stretch.first {
                    let fraction = (pos - f64::from(whole)).clamp(0.0, 1.0 - f64::EPSILON);
                    return Some((whole, fraction));
                }
                k += 1;
            }
        }
        let last_step_hour = if total_steps > 0 {
            let last_pos = (f64::from(from) + (total_steps - 1) as f64 * step_hours).min(f64::from(to));
            last_pos.floor() as i32
        } else {
            from
        };
        if last_step_hour < to && stretch_at(&self.stretches, to).is_some() {
            return Some((to, 0.0));
        }
        None
    }

    pub fn single_preview(hour: i32, fraction: f64, fps: u32, hours_per_second: f64) -> Self {
        let safe_fps = fps.max(1);
        let safe_hps = if hours_per_second.is_finite() && hours_per_second > 0.0 {
            hours_per_second
        } else {
            1.0
        };
        let clamped_frac = if fraction.is_finite() {
            fraction.clamp(0.0, 1.0 - f64::EPSILON)
        } else {
            0.0
        };
        let summary = ScheduleSummary {
            from_hour: hour,
            to_hour: hour,
            recorded_hours: 1,
            missing_hours: 0,
            gap_count: 0,
            total_frames: 1,
            output_seconds: 1.0 / f64::from(safe_fps),
            cadence: cadence_info(safe_hps, safe_fps),
        };
        Self {
            fps: safe_fps,
            hours_per_second: safe_hps,
            skip_gaps: false,
            cut_frames: 0,
            summary,
            stretches: vec![RecordedStretch {
                first: hour,
                last: hour,
                join: StretchJoin::Start,
            }],
            gaps: Vec::new(),
            single_fraction: Some(clamped_frac),
        }
    }

    pub fn first_recorded_covered(&self, covered: &[RecordedStretch]) -> Option<(i32, f64)> {
        if self.stretches.is_empty() || covered.is_empty() {
            return None;
        }
        if let Some(frac) = self.single_fraction {
            let h = self.summary.from_hour;
            if covered.iter().any(|c| c.first <= h && h <= c.last) {
                return Some((h, frac));
            }
            return None;
        }
        let step_hours = self.hours_per_second / f64::from(self.fps.max(1));
        if self.skip_gaps {
            for stretch in &self.stretches {
                let total_steps = steps_in_span(stretch.first, stretch.last, step_hours).ok()?;
                for cov in covered {
                    let lo = stretch.first.max(cov.first);
                    let hi = stretch.last.min(cov.last);
                    if lo > hi {
                        continue;
                    }
                    let start_delta = (f64::from(lo) - f64::from(stretch.first)).max(0.0);
                    let mut k = if start_delta == 0.0 {
                        0u64
                    } else {
                        (start_delta / step_hours).floor() as u64
                    };
                    while k < total_steps {
                        let pos = (f64::from(stretch.first) + k as f64 * step_hours)
                            .min(f64::from(stretch.last));
                        let whole = pos.floor() as i32;
                        if whole > hi {
                            break;
                        }
                        if whole >= lo {
                            let fraction = (pos - f64::from(whole)).clamp(0.0, 1.0 - f64::EPSILON);
                            return Some((whole, fraction));
                        }
                        k += 1;
                    }
                    let last_step_hour = if total_steps > 0 {
                        let last_pos = (f64::from(stretch.first)
                            + (total_steps - 1) as f64 * step_hours)
                            .min(f64::from(stretch.last));
                        last_pos.floor() as i32
                    } else {
                        stretch.first
                    };
                    if last_step_hour < stretch.last && stretch.last >= lo && stretch.last <= hi {
                        return Some((stretch.last, 0.0));
                    }
                }
            }
            return None;
        }
        let from = self.summary.from_hour;
        let to = self.summary.to_hour;
        let total_steps = steps_in_span(from, to, step_hours).ok()?;
        for stretch in &self.stretches {
            for cov in covered {
                let lo = stretch.first.max(cov.first);
                let hi = stretch.last.min(cov.last);
                if lo > hi {
                    continue;
                }
                let start_delta = (f64::from(lo) - f64::from(from)).max(0.0);
                let mut k = if start_delta == 0.0 {
                    0u64
                } else {
                    (start_delta / step_hours).floor() as u64
                };
                while k < total_steps {
                    let pos = (f64::from(from) + k as f64 * step_hours).min(f64::from(to));
                    let whole = pos.floor() as i32;
                    if whole > hi {
                        break;
                    }
                    if whole >= lo {
                        let fraction = (pos - f64::from(whole)).clamp(0.0, 1.0 - f64::EPSILON);
                        return Some((whole, fraction));
                    }
                    k += 1;
                }
                let last_step_hour = if total_steps > 0 {
                    let last_pos =
                        (f64::from(from) + (total_steps - 1) as f64 * step_hours).min(f64::from(to));
                    last_pos.floor() as i32
                } else {
                    from
                };
                if last_step_hour < to && to >= lo && to <= hi {
                    return Some((to, 0.0));
                }
            }
        }
        None
    }

    pub fn preview_frame(&self, target_hour: i32) -> PreviewFrame {
        let step_hours = self.hours_per_second / f64::from(self.fps.max(1));
        let target = target_hour.clamp(self.summary.from_hour, self.summary.to_hour);
        let mk_frame = |frame_index: u64, kind: FrameKind| ScheduledFrame {
            frame_index,
            presentation_time: frame_index as f64 / f64::from(self.fps.max(1)),
            audio_frames: audio_frames_for_index(frame_index, self.fps),
            kind,
        };
        if self.skip_gaps {
            let mut base_index: u64 = 0;
            for (idx, stretch) in self.stretches.iter().enumerate() {
                if idx > 0 {
                    let prev = &self.stretches[idx - 1];
                    let gap_first = i64::from(prev.last) + 1;
                    let gap_last = i64::from(stretch.first) - 1;
                    if gap_first <= gap_last && target > prev.last && target < stretch.first {
                        if self.cut_frames > 0 {
                            let omitted =
                                (gap_last - gap_first + 1).clamp(0, i64::from(u32::MAX)) as u32;
                            let reason = gap_reason_at(&self.gaps, gap_first as i32);
                            return PreviewFrame {
                                frame: mk_frame(
                                    base_index,
                                    FrameKind::SkipCutCard(CutNotice {
                                        from_hour: prev.last,
                                        to_hour: stretch.first,
                                        omitted_hours: omitted,
                                        reason,
                                    }),
                                ),
                                seed_frames: Vec::new(),
                            };
                        }
                        return PreviewFrame {
                            frame: mk_frame(
                                base_index,
                                FrameKind::Recorded {
                                    hour: stretch.first,
                                    fraction: 0.0,
                                    reset_continuity: true,
                                    cut_banner: None,
                                },
                            ),
                            seed_frames: Vec::new(),
                        };
                    }
                    if gap_first <= gap_last {
                        base_index += u64::from(self.cut_frames);
                    }
                }
                let span_total = span_frame_count(stretch.first, stretch.last, step_hours).unwrap_or(1);
                if target <= stretch.last || idx + 1 == self.stretches.len() {
                    let eff_target = target.clamp(stretch.first, stretch.last);
                    let (local, whole, fraction) =
                        select_span_sample(stretch.first, stretch.last, eff_target, step_hours);
                    let frame_index = base_index + local;
                    let reset_continuity = local == 0;
                    let seed_frames = if reset_continuity {
                        Vec::new()
                    } else {
                        build_ease_seeds(
                            base_index,
                            stretch.first,
                            stretch.last,
                            local,
                            whole,
                            step_hours,
                            &self.stretches,
                            self.fps,
                        )
                    };
                    return PreviewFrame {
                        frame: mk_frame(
                            frame_index,
                            FrameKind::Recorded {
                                hour: whole,
                                fraction,
                                reset_continuity,
                                cut_banner: None,
                            },
                        ),
                        seed_frames,
                    };
                }
                base_index += span_total;
            }
            return PreviewFrame {
                frame: mk_frame(
                    0,
                    FrameKind::Recorded {
                        hour: target,
                        fraction: 0.0,
                        reset_continuity: true,
                        cut_banner: None,
                    },
                ),
                seed_frames: Vec::new(),
            };
        }
        let from = self.summary.from_hour;
        let to = self.summary.to_hour;
        let (local, whole, fraction) = select_span_sample(from, to, target, step_hours);
        let prev_whole = if local > 0 {
            let (pw, _) = eval_span_step(from, to, local - 1, step_hours);
            Some(pw)
        } else {
            None
        };
        if stretch_at(&self.stretches, whole).is_some() {
            let cut_banner = prev_whole
                .and_then(|prev| traversed_gap_notice(&self.gaps, prev, whole, &[]));
            let reset_continuity = prev_whole
                .is_none_or(|prev| has_discontinuity(&self.stretches, prev, whole));
            let seed_frames = if reset_continuity {
                Vec::new()
            } else {
                build_ease_seeds(
                    0,
                    from,
                    to,
                    local,
                    whole,
                    step_hours,
                    &self.stretches,
                    self.fps,
                )
            };
            PreviewFrame {
                frame: mk_frame(
                    local,
                    FrameKind::Recorded {
                        hour: whole,
                        fraction,
                        reset_continuity,
                        cut_banner,
                    },
                ),
                seed_frames,
            }
        } else {
            let (gap_first, gap_last, reason) = gap_bounds_at(&self.gaps, whole, from, to);
            let crossed_gap = prev_whole
                .and_then(|prev| traversed_gap_notice(&self.gaps, prev, whole, &[gap_first]));
            PreviewFrame {
                frame: mk_frame(
                    local,
                    FrameKind::GapCard {
                        hour: whole,
                        gap_first,
                        gap_last,
                        reason,
                        crossed_gap,
                    },
                ),
                seed_frames: Vec::new(),
            }
        }
    }

    pub fn cursor(&self) -> FrameCursor<'_> {
        FrameCursor::new(self)
    }

    pub fn into_cursor(self) -> OwnedFrameCursor {
        OwnedFrameCursor::new(self)
    }

    pub fn shifted(&self, axis_offset: i32) -> Self {
        let mut next = self.clone();
        next.summary.from_hour = next.summary.from_hour.saturating_add(axis_offset);
        next.summary.to_hour = next.summary.to_hour.saturating_add(axis_offset);
        for stretch in &mut next.stretches {
            stretch.first = stretch.first.saturating_add(axis_offset);
            stretch.last = stretch.last.saturating_add(axis_offset);
        }
        for gap in &mut next.gaps {
            gap.first = gap.first.saturating_add(axis_offset);
            gap.last = gap.last.saturating_add(axis_offset);
        }
        next
    }

    #[cfg(test)]
    pub fn collect_frames(&self) -> Vec<ScheduledFrame> {
        self.cursor().collect()
    }
}

#[derive(Clone, Debug)]
enum CursorPhase {
    Single {
        hour: i32,
        fraction: f64,
    },
    Skip {
        stretch_idx: usize,
        cut_remaining: u32,
        pending_cut: Option<CutNotice>,
        local_idx: u64,
        stretch_count: u64,
        last_emitted_hour: Option<i32>,
        appending_end: bool,
    },
    Linear {
        local_idx: u64,
        total_steps: u64,
        last_emitted_hour: Option<i32>,
        prev_sampled_hour: Option<i32>,
        prev_recorded_hour: Option<i32>,
        disclosed_gaps: Vec<i32>,
        endpoint_done: bool,
    },
    Done,
}

#[derive(Clone, Debug)]
pub struct OwnedFrameCursor {
    schedule: FrameSchedule,
    step_hours: f64,
    frame_index: u64,
    phase: CursorPhase,
}

impl OwnedFrameCursor {
    fn new(schedule: FrameSchedule) -> Self {
        let step_hours = schedule.hours_per_second / f64::from(schedule.fps.max(1));
        let phase = initial_phase(&schedule, step_hours);
        Self {
            schedule,
            step_hours,
            frame_index: 0,
            phase,
        }
    }

    pub fn schedule(&self) -> &FrameSchedule {
        &self.schedule
    }
}

impl Iterator for OwnedFrameCursor {
    type Item = ScheduledFrame;

    fn next(&mut self) -> Option<Self::Item> {
        advance_cursor(
            &self.schedule,
            self.step_hours,
            &mut self.frame_index,
            &mut self.phase,
        )
    }
}

fn initial_phase(schedule: &FrameSchedule, step_hours: f64) -> CursorPhase {
    if let Some(fraction) = schedule.single_fraction {
        CursorPhase::Single {
            hour: schedule.summary.from_hour,
            fraction,
        }
    } else if schedule.stretches.is_empty() {
        CursorPhase::Done
    } else if schedule.skip_gaps {
        let first = &schedule.stretches[0];
        let stretch_count = steps_in_span(first.first, first.last, step_hours).unwrap_or(1);
        CursorPhase::Skip {
            stretch_idx: 0,
            cut_remaining: 0,
            pending_cut: None,
            local_idx: 0,
            stretch_count,
            last_emitted_hour: None,
            appending_end: false,
        }
    } else {
        let total_steps = steps_in_span(
            schedule.summary.from_hour,
            schedule.summary.to_hour,
            step_hours,
        )
        .unwrap_or(1);
        CursorPhase::Linear {
            local_idx: 0,
            total_steps,
            last_emitted_hour: None,
            prev_sampled_hour: None,
            prev_recorded_hour: None,
            disclosed_gaps: Vec::new(),
            endpoint_done: false,
        }
    }
}

fn make_frame(frame_index: &mut u64, fps: u32, kind: FrameKind) -> ScheduledFrame {
    let idx = *frame_index;
    *frame_index = frame_index.saturating_add(1);
    ScheduledFrame {
        frame_index: idx,
        kind,
        presentation_time: idx as f64 / f64::from(fps),
        audio_frames: audio_frames_for_index(idx, fps),
    }
}

fn emit_linear_sample(
    schedule: &FrameSchedule,
    whole: i32,
    fraction: f64,
    prev_sampled_hour: &mut Option<i32>,
    prev_recorded_hour: &mut Option<i32>,
    disclosed_gaps: &mut Vec<i32>,
) -> FrameKind {
    let prior_sample = *prev_sampled_hour;
    *prev_sampled_hour = Some(whole);
    if stretch_at(&schedule.stretches, whole).is_some() {
        let cut_banner = prior_sample
            .and_then(|prev| traversed_gap_notice(&schedule.gaps, prev, whole, disclosed_gaps));
        let traversed_join = prev_recorded_hour
            .is_none_or(|prev| has_discontinuity(&schedule.stretches, prev, whole));
        *prev_recorded_hour = Some(whole);
        disclosed_gaps.clear();
        FrameKind::Recorded {
            hour: whole,
            fraction,
            reset_continuity: traversed_join || cut_banner.is_some(),
            cut_banner,
        }
    } else {
        let (gap_first, gap_last, reason) = gap_bounds_at(
            &schedule.gaps,
            whole,
            schedule.summary.from_hour,
            schedule.summary.to_hour,
        );
        let crossed_gap = prior_sample
            .and_then(|prev| traversed_gap_notice(&schedule.gaps, prev, gap_first, disclosed_gaps));
        if crossed_gap.is_some()
            && let Some(prev) = prior_sample
        {
            for gap in &schedule.gaps {
                if gap.first > prev && gap.last < gap_first && !disclosed_gaps.contains(&gap.first) {
                    disclosed_gaps.push(gap.first);
                }
            }
        }
        if !disclosed_gaps.contains(&gap_first) {
            disclosed_gaps.push(gap_first);
        }
        FrameKind::GapCard {
            hour: whole,
            gap_first,
            gap_last,
            reason,
            crossed_gap,
        }
    }
}

fn advance_cursor(
    schedule: &FrameSchedule,
    step_hours: f64,
    frame_index: &mut u64,
    phase: &mut CursorPhase,
) -> Option<ScheduledFrame> {
    loop {
        match phase {
            CursorPhase::Done => return None,
            CursorPhase::Single { hour, fraction } => {
                let h = *hour;
                let f = *fraction;
                *phase = CursorPhase::Done;
                return Some(make_frame(
                    frame_index,
                    schedule.fps,
                    FrameKind::Recorded {
                        hour: h,
                        fraction: f,
                        reset_continuity: true,
                        cut_banner: None,
                    },
                ));
            }
            CursorPhase::Skip {
                stretch_idx,
                cut_remaining,
                pending_cut,
                local_idx,
                stretch_count,
                last_emitted_hour,
                appending_end,
            } => {
                if *cut_remaining > 0 {
                    *cut_remaining -= 1;
                    let notice = pending_cut.clone().expect("pending cut set with counter");
                    if *cut_remaining == 0 {
                        *pending_cut = None;
                    }
                    return Some(make_frame(frame_index, schedule.fps, FrameKind::SkipCutCard(notice)));
                }
                let Some(stretch) = schedule.stretches.get(*stretch_idx) else {
                    *phase = CursorPhase::Done;
                    return None;
                };
                if *local_idx < *stretch_count {
                    let local = *local_idx;
                    *local_idx += 1;
                    let pos = (f64::from(stretch.first) + local as f64 * step_hours)
                        .min(f64::from(stretch.last));
                    let whole = pos.floor() as i32;
                    let fraction = (pos - f64::from(whole)).clamp(0.0, 1.0 - f64::EPSILON);
                    *last_emitted_hour = Some(whole);
                    return Some(make_frame(
                        frame_index,
                        schedule.fps,
                        FrameKind::Recorded {
                            hour: whole,
                            fraction,
                            reset_continuity: local == 0,
                            cut_banner: None,
                        },
                    ));
                }
                if !*appending_end && last_emitted_hour.is_some_and(|h| h < stretch.last) {
                    *appending_end = true;
                    *last_emitted_hour = Some(stretch.last);
                    return Some(make_frame(
                        frame_index,
                        schedule.fps,
                        FrameKind::Recorded {
                            hour: stretch.last,
                            fraction: 0.0,
                            reset_continuity: false,
                            cut_banner: None,
                        },
                    ));
                }
                let prev_last = stretch.last;
                *stretch_idx += 1;
                let Some(next_stretch) = schedule.stretches.get(*stretch_idx) else {
                    *phase = CursorPhase::Done;
                    return None;
                };
                *local_idx = 0;
                *stretch_count =
                    steps_in_span(next_stretch.first, next_stretch.last, step_hours).unwrap_or(1);
                *last_emitted_hour = None;
                *appending_end = false;
                let gap_first = i64::from(prev_last) + 1;
                let gap_last = i64::from(next_stretch.first) - 1;
                if gap_first <= gap_last && schedule.cut_frames > 0 {
                    let omitted = (gap_last - gap_first + 1).min(i64::from(u32::MAX)) as u32;
                    let reason = gap_reason_at(&schedule.gaps, gap_first as i32);
                    *pending_cut = Some(CutNotice {
                        from_hour: prev_last,
                        to_hour: next_stretch.first,
                        omitted_hours: omitted,
                        reason,
                    });
                    *cut_remaining = schedule.cut_frames;
                }
            }
            CursorPhase::Linear {
                local_idx,
                total_steps,
                last_emitted_hour,
                prev_sampled_hour,
                prev_recorded_hour,
                disclosed_gaps,
                endpoint_done,
            } => {
                if *local_idx < *total_steps {
                    let local = *local_idx;
                    *local_idx += 1;
                    let pos = (f64::from(schedule.summary.from_hour)
                        + local as f64 * step_hours)
                        .min(f64::from(schedule.summary.to_hour));
                    let whole = pos.floor() as i32;
                    let fraction = (pos - f64::from(whole)).clamp(0.0, 1.0 - f64::EPSILON);
                    *last_emitted_hour = Some(whole);
                    let kind = emit_linear_sample(
                        schedule,
                        whole,
                        fraction,
                        prev_sampled_hour,
                        prev_recorded_hour,
                        disclosed_gaps,
                    );
                    return Some(make_frame(frame_index, schedule.fps, kind));
                }
                if !*endpoint_done
                    && last_emitted_hour.is_some_and(|h| h < schedule.summary.to_hour)
                {
                    *endpoint_done = true;
                    let whole = schedule.summary.to_hour;
                    *last_emitted_hour = Some(whole);
                    let kind = emit_linear_sample(
                        schedule,
                        whole,
                        0.0,
                        prev_sampled_hour,
                        prev_recorded_hour,
                        disclosed_gaps,
                    );
                    return Some(make_frame(frame_index, schedule.fps, kind));
                }
                *phase = CursorPhase::Done;
                return None;
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct FrameCursor<'a> {
    schedule: &'a FrameSchedule,
    step_hours: f64,
    frame_index: u64,
    phase: CursorPhase,
}

impl<'a> FrameCursor<'a> {
    fn new(schedule: &'a FrameSchedule) -> Self {
        let step_hours = schedule.hours_per_second / f64::from(schedule.fps.max(1));
        let phase = initial_phase(schedule, step_hours);
        Self {
            schedule,
            step_hours,
            frame_index: 0,
            phase,
        }
    }
}

impl Iterator for FrameCursor<'_> {
    type Item = ScheduledFrame;

    fn next(&mut self) -> Option<Self::Item> {
        advance_cursor(
            self.schedule,
            self.step_hours,
            &mut self.frame_index,
            &mut self.phase,
        )
    }
}

fn steps_in_span(first: i32, last: i32, step_hours: f64) -> io::Result<u64> {
    let span = i64::from(last) - i64::from(first);
    if span <= 0 {
        return Ok(1);
    }
    let raw = (span as f64 / step_hours).floor();
    if !raw.is_finite() || raw < 0.0 || raw >= (u64::MAX - 2) as f64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Export duration exceeds supported frame count",
        ));
    }
    Ok(raw as u64 + 1)
}

fn span_frame_count(first: i32, last: i32, step_hours: f64) -> io::Result<u64> {
    let count = steps_in_span(first, last, step_hours)?;
    let last_pos = (f64::from(first) + (count - 1) as f64 * step_hours).min(f64::from(last));
    let last_whole = last_pos.floor() as i32;
    let extra = u64::from(last_whole < last);
    count.checked_add(extra).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Export frame count overflowed",
        )
    })
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
    let mut intervals: Vec<(i32, i32)> = Vec::new();
    let mut load_starts = std::collections::BTreeSet::new();
    for track in tracks {
        for s in track {
            if s.first > s.last {
                continue;
            }
            intervals.push((s.first, s.last));
            if s.join == StretchJoin::Load {
                load_starts.insert(s.first);
            }
        }
    }
    intervals.sort_unstable_by_key(|&(first, last)| (first, last));
    let mut spans: Vec<(i32, i32)> = Vec::new();
    for (first, last) in intervals {
        if let Some(prev) = spans.last_mut()
            && i64::from(first) <= i64::from(prev.1) + 1
        {
            if last > prev.1 {
                prev.1 = last;
            }
            continue;
        }
        spans.push((first, last));
    }
    let mut merged: Vec<RecordedStretch> = Vec::new();
    for (span_first, span_last) in spans {
        let mut cursor = span_first;
        let mut join = if merged.is_empty() {
            StretchJoin::Start
        } else if load_starts.contains(&span_first) {
            StretchJoin::Load
        } else {
            StretchJoin::Break
        };
        for &cut in load_starts.range((
            std::ops::Bound::Excluded(span_first),
            std::ops::Bound::Included(span_last),
        )) {
            if cut > cursor {
                merged.push(RecordedStretch {
                    first: cursor,
                    last: cut - 1,
                    join,
                });
                cursor = cut;
                join = StretchJoin::Load;
            }
        }
        merged.push(RecordedStretch {
            first: cursor,
            last: span_last,
            join,
        });
    }
    merged
}

pub fn merge_unheld_gaps(
    merged_stretches: &[RecordedStretch],
    tracks_unheld: &[Vec<UnheldGap>],
    from_hour: i32,
    to_hour: i32,
) -> Vec<UnheldGap> {
    let lo = from_hour.min(to_hour);
    let hi = from_hour.max(to_hour);
    let reason_for = |first: i32, last: i32| -> String {
        tracks_unheld
            .iter()
            .flat_map(|gaps| gaps.iter())
            .find(|g| g.first <= last && g.last >= first)
            .map_or_else(
                || "Not recorded: this play happened while the recorder wasn't watching".to_owned(),
                |g| g.reason.clone(),
            )
    };
    let mut out = Vec::new();
    let mut cursor = i64::from(lo);
    let end = i64::from(hi);
    for stretch in merged_stretches {
        let s_first = i64::from(stretch.first).max(i64::from(lo));
        let s_last = i64::from(stretch.last).min(end);
        if s_first > s_last {
            continue;
        }
        if cursor < s_first {
            let first = cursor as i32;
            let last = (s_first - 1) as i32;
            out.push(UnheldGap {
                first,
                last,
                reason: reason_for(first, last),
            });
        }
        if s_last + 1 > cursor {
            cursor = s_last + 1;
        }
    }
    if cursor <= end {
        let first = cursor as i32;
        let last = end as i32;
        out.push(UnheldGap {
            first,
            last,
            reason: reason_for(first, last),
        });
    }
    out
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

    let mut gap_count = 0usize;
    let total_frames: u64 = if request.skip_gaps {
        let mut total = 0u64;
        for (idx, stretch) in clipped_stretches.iter().enumerate() {
            if idx > 0 {
                let prev = &clipped_stretches[idx - 1];
                let gap_first = i64::from(prev.last) + 1;
                let gap_last = i64::from(stretch.first) - 1;
                if gap_first <= gap_last {
                    gap_count += 1;
                    total = total.checked_add(u64::from(cut_frames)).ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "Export frame count overflowed",
                        )
                    })?;
                }
            }
            let stretch_frames = span_frame_count(stretch.first, stretch.last, step_hours)?;
            total = total.checked_add(stretch_frames).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Export frame count overflowed",
                )
            })?;
        }
        total
    } else {
        gap_count = clipped_gaps.len();
        span_frame_count(request.from_hour, request.to_hour, step_hours)?
    };

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
        skip_gaps: request.skip_gaps,
        cut_frames,
        summary,
        stretches: clipped_stretches,
        gaps: clipped_gaps,
        single_fraction: None,
    })
}

fn eval_span_step(first: i32, last: i32, local_idx: u64, step_hours: f64) -> (i32, f64) {
    let total_steps = steps_in_span(first, last, step_hours).unwrap_or(1);
    if local_idx >= total_steps {
        return (last, 0.0);
    }
    let pos = (f64::from(first) + local_idx as f64 * step_hours).min(f64::from(last));
    let whole = pos.floor() as i32;
    (whole, (pos - f64::from(whole)).clamp(0.0, 1.0 - f64::EPSILON))
}

fn select_span_sample(first: i32, last: i32, target: i32, step_hours: f64) -> (u64, i32, f64) {
    let clamped = target.clamp(first, last);
    let span = f64::from(last) - f64::from(first);
    if span <= 0.0 || step_hours <= 0.0 {
        return (0, first, 0.0);
    }
    let total_steps = steps_in_span(first, last, step_hours).unwrap_or(1);
    let last_step_hour = if total_steps > 0 {
        eval_span_step(first, last, total_steps - 1, step_hours).0
    } else {
        first
    };
    if clamped == last && last_step_hour < last {
        return (total_steps, last, 0.0);
    }
    let delta = (f64::from(clamped) - f64::from(first)).max(0.0);
    let k_ceil = (delta / step_hours).ceil().max(0.0) as u64;
    if k_ceil < total_steps {
        let (w, f) = eval_span_step(first, last, k_ceil, step_hours);
        if w == clamped {
            return (k_ceil, w, f);
        }
    }
    let k_floor = (delta / step_hours)
        .floor()
        .max(0.0)
        .min((total_steps.saturating_sub(1)) as f64) as u64;
    let (w, f) = eval_span_step(first, last, k_floor, step_hours);
    (k_floor, w, f)
}

#[expect(
    clippy::too_many_arguments,
    reason = "computes exact prior ease seed frames from span geometry and schedule parameters"
)]
fn build_ease_seeds(
    base_index: u64,
    span_first: i32,
    span_last: i32,
    selected_local: u64,
    selected_hour: i32,
    step_hours: f64,
    stretches: &[RecordedStretch],
    fps: u32,
) -> Vec<ScheduledFrame> {
    if selected_local == 0 || step_hours <= 0.0 {
        return Vec::new();
    }
    let delta = (f64::from(selected_hour) - f64::from(span_first)).max(0.0);
    let change_local = (delta / step_hours).ceil().max(0.0) as u64;
    let change_local = change_local.min(selected_local);
    if change_local == 0 {
        return Vec::new();
    }
    let prior_local = change_local - 1;
    let (prior_hour, prior_frac) = eval_span_step(span_first, span_last, prior_local, step_hours);
    if has_discontinuity(stretches, prior_hour, selected_hour) {
        return Vec::new();
    }
    let safe_fps = fps.max(1);
    let mk = |idx: u64, hour: i32, fraction: f64, reset_continuity: bool| ScheduledFrame {
        frame_index: idx,
        presentation_time: idx as f64 / f64::from(safe_fps),
        audio_frames: audio_frames_for_index(idx, safe_fps),
        kind: FrameKind::Recorded {
            hour,
            fraction,
            reset_continuity,
            cut_banner: None,
        },
    };
    let mut seeds = Vec::with_capacity(2);
    seeds.push(mk(base_index + prior_local, prior_hour, prior_frac, true));
    if change_local < selected_local {
        let (ch_hour, ch_frac) = eval_span_step(span_first, span_last, change_local, step_hours);
        if ch_hour == selected_hour {
            seeds.push(mk(base_index + change_local, ch_hour, ch_frac, false));
        }
    }
    seeds
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

fn traversed_gap_notice(
    gaps: &[UnheldGap],
    prev: i32,
    next: i32,
    disclosed_gaps: &[i32],
) -> Option<CutNotice> {
    let mut first_gap: Option<&UnheldGap> = None;
    let mut last_gap: Option<&UnheldGap> = None;
    let mut omitted: u64 = 0;
    for gap in gaps {
        if gap.first > prev && gap.last < next && !disclosed_gaps.contains(&gap.first) {
            if first_gap.is_none() {
                first_gap = Some(gap);
            }
            last_gap = Some(gap);
            omitted = omitted.saturating_add((i64::from(gap.last) - i64::from(gap.first) + 1) as u64);
        }
    }
    let first = first_gap?;
    let last = last_gap?;
    Some(CutNotice {
        from_hour: first.first.saturating_sub(1),
        to_hour: last.last.saturating_add(1),
        omitted_hours: omitted.min(u64::from(u32::MAX)) as u32,
        reason: first.reason.clone(),
    })
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
        let frames = schedule.collect_frames();
        assert_eq!(frames.len() as u64, schedule.summary.total_frames);
        let cut_cards = frames
            .iter()
            .filter(|f| matches!(f.kind, FrameKind::SkipCutCard(_)))
            .count();
        assert_eq!(cut_cards, 30);
        let load_frame = frames
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
        let frames = schedule.collect_frames();
        assert_eq!(frames.len() as u64, schedule.summary.total_frames);
        assert!(frames.iter().any(|f| matches!(
            &f.kind,
            FrameKind::Recorded { reset_continuity: true, cut_banner: Some(cut), .. }
            if cut.omitted_hours == 5
        )));
    }

    #[test]
    fn endpoint_and_multi_gap_steps_report_all_omitted_hours_without_upfront_allocation() {
        let stretches = vec![
            RecordedStretch { first: 100, last: 110, join: StretchJoin::Start },
            RecordedStretch { first: 115, last: 115, join: StretchJoin::Break },
        ];
        let gaps = vec![UnheldGap { first: 111, last: 114, reason: "Tail gap".into() }];
        let schedule = build_schedule(
            &stretches,
            &gaps,
            &ScheduleRequest {
                from_hour: 100,
                to_hour: 115,
                fps: 30,
                hours_per_second: 300.0,
                skip_gaps: false,
                cut_card_seconds: 0.0,
            },
        )
        .unwrap();
        let frames = schedule.collect_frames();
        assert_eq!(frames.len() as u64, schedule.summary.total_frames);
        assert!(matches!(
            &frames.last().unwrap().kind,
            FrameKind::Recorded { hour: 115, reset_continuity: true, cut_banner: Some(cut), .. }
            if cut.omitted_hours == 4
        ));

        let huge = build_schedule(
            &[RecordedStretch { first: 0, last: 60_000, join: StretchJoin::Start }],
            &[],
            &ScheduleRequest {
                from_hour: 0,
                to_hour: 60_000,
                fps: 60,
                hours_per_second: 0.1,
                skip_gaps: false,
                cut_card_seconds: 0.0,
            },
        )
        .unwrap();
        assert_eq!(huge.summary.total_frames, 36_000_001);

        let merged = merge_recorded_tracks(&[vec![
            RecordedStretch { first: 100, last: 199, join: StretchJoin::Start },
            RecordedStretch { first: 200, last: 249, join: StretchJoin::Load },
        ]]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[1].join, StretchJoin::Load);

        let track_a = vec![
            RecordedStretch { first: 0, last: 50, join: StretchJoin::Start },
            RecordedStretch { first: 100, last: 200, join: StretchJoin::Break },
        ];
        let track_b = vec![RecordedStretch { first: 40, last: 120, join: StretchJoin::Start }];
        let union = merge_recorded_tracks(&[track_a, track_b]);
        let union_gaps = merge_unheld_gaps(
            &union,
            &[
                vec![UnheldGap { first: 51, last: 99, reason: "track A gap".into() }],
                vec![],
            ],
            0,
            200,
        );
        assert_eq!(union.len(), 1);
        assert!(union_gaps.is_empty());
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
        assert_eq!(one.collect_frames().len(), 1);
    }

    #[test]
    fn linear_sampling_discloses_crossed_gap_even_when_intermediate_sample_hits_later_gap() {
        let stretches = vec![
            RecordedStretch { first: 100, last: 101, join: StretchJoin::Start },
            RecordedStretch { first: 104, last: 108, join: StretchJoin::Break },
            RecordedStretch { first: 112, last: 120, join: StretchJoin::Break },
        ];
        let gaps = vec![
            UnheldGap { first: 102, last: 103, reason: "first gap".into() },
            UnheldGap { first: 109, last: 111, reason: "second gap".into() },
        ];
        let schedule = build_schedule(
            &stretches,
            &gaps,
            &ScheduleRequest {
                from_hour: 100,
                to_hour: 120,
                fps: 1,
                hours_per_second: 10.0,
                skip_gaps: false,
                cut_card_seconds: 0.0,
            },
        )
        .unwrap();
        let frames = schedule.collect_frames();
        assert_eq!(frames.len(), 3);
        assert!(matches!(frames[0].kind, FrameKind::Recorded { hour: 100, .. }));
        assert!(matches!(
            &frames[1].kind,
            FrameKind::GapCard { hour: 110, gap_first: 109, gap_last: 111, crossed_gap: Some(cut), .. }
            if cut.omitted_hours == 2 && cut.from_hour == 101 && cut.to_hour == 104
        ));
        assert!(matches!(
            &frames[2].kind,
            FrameKind::Recorded { hour: 120, reset_continuity: true, cut_banner: None, .. }
        ));

        let terminal_gap = build_schedule(
            &stretches,
            &gaps,
            &ScheduleRequest {
                from_hour: 100,
                to_hour: 110,
                fps: 1,
                hours_per_second: 10.0,
                skip_gaps: false,
                cut_card_seconds: 0.0,
            },
        )
        .unwrap();
        let terminal_frames = terminal_gap.collect_frames();
        assert_eq!(terminal_frames.len(), 2);
        assert!(matches!(
            &terminal_frames[1].kind,
            FrameKind::GapCard { hour: 110, gap_first: 109, gap_last: 110, crossed_gap: Some(cut), .. }
            if cut.omitted_hours == 2 && cut.from_hour == 101 && cut.to_hour == 104
        ));

        let leading_gap = build_schedule(
            &[
                RecordedStretch { first: 102, last: 103, join: StretchJoin::Start },
                RecordedStretch { first: 106, last: 110, join: StretchJoin::Break },
            ],
            &[
                UnheldGap { first: 100, last: 101, reason: "leading gap".into() },
                UnheldGap { first: 104, last: 105, reason: "mid gap".into() },
            ],
            &ScheduleRequest {
                from_hour: 100,
                to_hour: 110,
                fps: 1,
                hours_per_second: 10.0,
                skip_gaps: false,
                cut_card_seconds: 0.0,
            },
        )
        .unwrap();
        let leading_frames = leading_gap.collect_frames();
        assert_eq!(leading_frames.len(), 2);
        assert!(matches!(
            &leading_frames[0].kind,
            FrameKind::GapCard { hour: 100, gap_first: 100, gap_last: 101, crossed_gap: None, .. }
        ));
        assert!(matches!(
            &leading_frames[1].kind,
            FrameKind::Recorded { hour: 110, reset_continuity: true, cut_banner: Some(cut), .. }
            if cut.omitted_hours == 2 && cut.from_hour == 103 && cut.to_hour == 106
        ));
    }

    #[test]
    fn first_recorded_skips_huge_leading_gap_directly() {
        let schedule = build_schedule(
            &[RecordedStretch { first: 10_000, last: 10_010, join: StretchJoin::Start }],
            &[UnheldGap { first: 0, last: 9_999, reason: "leading gap".into() }],
            &ScheduleRequest {
                from_hour: 0,
                to_hour: 10_010,
                fps: 60,
                hours_per_second: 0.1,
                skip_gaps: false,
                cut_card_seconds: 0.0,
            },
        )
        .unwrap();
        assert_eq!(schedule.first_recorded(), Some((10_000, 0.0)));
    }

    #[test]
    fn cadence_flags_uneven_24_hps_at_60_fps_and_suggests_clean_snap() {
        let info = cadence_info(24.0, 60);
        assert!(info.uneven);
        assert_eq!(info.suggested_hours_per_second, Some(20.0));
        assert!(!cadence_info(30.0, 60).uneven);
        assert!(!cadence_info(60.0, 60).uneven);
    }

    #[test]
    fn merge_recorded_tracks_preserves_load_boundary_across_overlapping_track() {
        let track_a = vec![
            RecordedStretch { first: 0, last: 99, join: StretchJoin::Start },
            RecordedStretch { first: 100, last: 200, join: StretchJoin::Load },
        ];
        let track_b = vec![RecordedStretch { first: 0, last: 200, join: StretchJoin::Start }];
        let merged = merge_recorded_tracks(&[track_a, track_b]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0], RecordedStretch { first: 0, last: 99, join: StretchJoin::Start });
        assert_eq!(merged[1], RecordedStretch { first: 100, last: 200, join: StretchJoin::Load });
        let gaps = merge_unheld_gaps(&merged, &[], 0, 200);
        assert!(gaps.is_empty());
        let schedule = build_schedule(
            &merged,
            &gaps,
            &ScheduleRequest {
                from_hour: 99,
                to_hour: 100,
                fps: 1,
                hours_per_second: 1.0,
                skip_gaps: false,
                cut_card_seconds: 0.0,
            },
        )
        .unwrap();
        let frames = schedule.collect_frames();
        assert_eq!(frames.len(), 2);
        assert!(matches!(
            frames[1].kind,
            FrameKind::Recorded { hour: 100, reset_continuity: true, .. }
        ));
    }

    #[test]
    fn merge_unheld_gaps_computes_leading_and_trailing_end_gaps() {
        let merged = vec![RecordedStretch { first: 10, last: 90, join: StretchJoin::Start }];
        let gaps = merge_unheld_gaps(
            &merged,
            &[vec![
                UnheldGap { first: 0, last: 9, reason: "before start".into() },
                UnheldGap { first: 91, last: 100, reason: "after end".into() },
            ]],
            0,
            100,
        );
        assert_eq!(gaps.len(), 2);
        assert_eq!((gaps[0].first, gaps[0].last, gaps[0].reason.as_str()), (0, 9, "before start"));
        assert_eq!((gaps[1].first, gaps[1].last, gaps[1].reason.as_str()), (91, 100, "after end"));
    }

    #[test]
    fn preview_frame_matches_linear_fractional_cadence_and_skip_gaps() {
        let (stretches, gaps) = sample_stretches();
        let linear = build_schedule(
            &stretches,
            &gaps,
            &ScheduleRequest {
                from_hour: 100,
                to_hour: 399,
                fps: 4,
                hours_per_second: 1.0,
                skip_gaps: false,
                cut_card_seconds: 1.0,
            },
        )
        .unwrap();
        assert!(matches!(
            linear.preview_frame(270).frame.kind,
            FrameKind::GapCard { hour: 270, gap_first: 250, gap_last: 299, .. }
        ));
        let recorded = linear.preview_frame(110);
        assert!(matches!(
            recorded.frame.kind,
            FrameKind::Recorded { hour: 110, fraction, reset_continuity: false, .. } if fraction.abs() < 1e-6
        ));
        assert!(!recorded.seed_frames.is_empty());

        let frac_gap = build_schedule(
            &[
                RecordedStretch { first: 0, last: 0, join: StretchJoin::Start },
                RecordedStretch { first: 2, last: 5, join: StretchJoin::Break },
            ],
            &[UnheldGap { first: 1, last: 1, reason: "hour 1 missing".into() }],
            &ScheduleRequest {
                from_hour: 0,
                to_hour: 5,
                fps: 60,
                hours_per_second: 24.0,
                skip_gaps: false,
                cut_card_seconds: 1.0,
            },
        )
        .unwrap();
        let frac_gap_preview = frac_gap.preview_frame(1);
        assert_eq!(frac_gap_preview.frame.frame_index, 3);
        assert!(matches!(
            frac_gap_preview.frame.kind,
            FrameKind::GapCard { hour: 1, gap_first: 1, gap_last: 1, .. }
        ));

        let frac_rec = build_schedule(
            &[RecordedStretch { first: 0, last: 5, join: StretchJoin::Start }],
            &[],
            &ScheduleRequest {
                from_hour: 0,
                to_hour: 5,
                fps: 60,
                hours_per_second: 24.0,
                skip_gaps: false,
                cut_card_seconds: 1.0,
            },
        )
        .unwrap();
        let frac_rec_preview = frac_rec.preview_frame(1);
        assert_eq!(frac_rec_preview.frame.frame_index, 3);
        assert!(matches!(
            frac_rec_preview.frame.kind,
            FrameKind::Recorded { hour: 1, fraction, reset_continuity: false, .. }
            if (fraction - 0.2).abs() < 1e-6
        ));
        assert_eq!(frac_rec_preview.seed_frames.len(), 1);
        assert!(matches!(
            frac_rec_preview.seed_frames[0].kind,
            FrameKind::Recorded { hour: 0, .. }
        ));

        let skip_with_card = build_schedule(
            &stretches,
            &gaps,
            &ScheduleRequest {
                from_hour: 100,
                to_hour: 399,
                fps: 30,
                hours_per_second: 30.0,
                skip_gaps: true,
                cut_card_seconds: 1.0,
            },
        )
        .unwrap();
        assert!(matches!(
            skip_with_card.preview_frame(270).frame.kind,
            FrameKind::SkipCutCard(CutNotice { from_hour: 249, to_hour: 300, omitted_hours: 50, .. })
        ));

        let skip_zero_card = build_schedule(
            &stretches,
            &gaps,
            &ScheduleRequest {
                from_hour: 100,
                to_hour: 399,
                fps: 30,
                hours_per_second: 30.0,
                skip_gaps: true,
                cut_card_seconds: 0.0,
            },
        )
        .unwrap();
        assert!(matches!(
            skip_zero_card.preview_frame(270).frame.kind,
            FrameKind::Recorded { hour: 300, fraction, .. } if fraction.abs() < 1e-6
        ));
    }

    #[test]
    fn first_recorded_covered_finds_first_scheduled_position_inside_map_coverage() {
        let page_schedule = build_schedule(
            &[RecordedStretch { first: 50, last: 200, join: StretchJoin::Start }],
            &[],
            &ScheduleRequest {
                from_hour: 50,
                to_hour: 200,
                fps: 60,
                hours_per_second: 24.0,
                skip_gaps: false,
                cut_card_seconds: 0.0,
            },
        )
        .unwrap();
        let map_stretches = vec![
            RecordedStretch { first: 0, last: 10, join: StretchJoin::Start },
            RecordedStretch { first: 100, last: 200, join: StretchJoin::Break },
        ];
        let first = page_schedule.first_recorded_covered(&map_stretches).unwrap();
        assert_eq!(first.0, 100);
        assert!(first.1.abs() < 1e-6);
    }
}
