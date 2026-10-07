//! Shared ToolTip painter, clocks and placement (tooltip_controller::update@1402A4080).
use crate::map::{counters::gui_windows, native_gui, sprites::Batch};
use installed_content::defines::Defines;
use std::{
    io,
    time::{Duration, Instant},
};

/// Immediate text, delayed body, shortcut and the bound control rectangle.
type TooltipHit<'a> = (&'a str, Option<&'a str>, Option<&'a str>, [f32; 4]);

/// `NInterface.TOOLTIP_SCREEN_{LEFT,TOP,RIGHT,BOTTOM}_OFFSET_{X,Y}` and the
/// `NGraphics.TOOLTIP_{SHOW,DELAYED,HIDE}_DELAY` seconds `1402A4080` reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TooltipDefines {
    offsets: [i32; 4],
    pub(super) timing: [f32; 3],
}

impl TooltipDefines {
    pub(crate) fn read(defines: &Defines) -> io::Result<Self> {
        let offset = |name: &str| defines.int32("NInterface", name);
        // The game keeps each delay as a float (14331D158, 14331D1E8,
        // 14331D25C) and only compares it (1402A4080): any value is a delay,
        // a negative one included.
        let delay =
            |name: &str| -> io::Result<f32> { Ok(defines.number("NGraphics", name)? as f32) };
        Ok(Self {
            offsets: [
                offset("TOOLTIP_SCREEN_LEFT_OFFSET_X")?,
                offset("TOOLTIP_SCREEN_TOP_OFFSET_Y")?,
                offset("TOOLTIP_SCREEN_RIGHT_OFFSET_X")?,
                offset("TOOLTIP_SCREEN_BOTTOM_OFFSET_Y")?,
            ],
            timing: [
                delay("TOOLTIP_SHOW_DELAY")?,
                delay("TOOLTIP_DELAYED_DELAY")?,
                delay("TOOLTIP_HIDE_DELAY")?,
            ],
        })
    }
}

/// Per-viewer tooltip clocks, corresponding to the controller's presentation
/// clocks in `1402A4080`. A widget rectangle identifies the current binding:
/// refreshing its simulation values does not restart the delayed-body timer.
///
/// The show and hide timers are deadlines on the presentation clock, as the
/// game keeps them (14331730C, 143317308): seconds since the clock started
/// (ReadPresentationClockSeconds 142238C90 accumulates from its start), as a
/// float, with a deadline at or below 0 meaning unset. The viewer's clock
/// starts at its first frame.
#[derive(Default)]
pub(super) struct HoverState {
    province: Option<u32>,
    target: Option<[f32; 4]>,
    entered: Option<Instant>,
    delayed_pending: bool,
    /// The presentation clock's start.
    clock: Option<Instant>,
    /// The show deadline (14331730C); 0 or below is unset.
    show_deadline: f32,
    /// The hide deadline (143317308); 0 or below is unset.
    hide_deadline: f32,
    /// Text waits for the show deadline.
    showing: bool,
    /// The visible tooltip waits for the hide deadline.
    hiding: bool,
    visible: bool,
    text: String,
    /// The next frame's text, kept to reuse its buffer.
    candidate: String,
    point: [f32; 2],
    pub(super) anchor: Option<TooltipPosition>,
}

/// Pointer plus the naval producer's combat-window rectangle, in interface pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TooltipPosition {
    pub(super) point: [f32; 2],
    pub(super) naval_window: Option<[f32; 4]>,
}

impl HoverState {
    /// 140B450E0's province binding changes independently of the map rectangle.
    pub(super) fn bind_province(&mut self, province: Option<u32>) {
        if self.province != province {
            self.province = province;
            self.target = None;
            self.entered = None;
        }
    }
    /// Delays are `[show, delayed-body, hide]` seconds. Call this on every
    /// requested frame, including frames without a hit, so hiding can finish.
    /// An already visible tooltip updates immediately when its binding changes;
    /// the show delay applies only while the CTextBox is hidden in the game.
    pub(super) fn shown(
        &mut self,
        hit: Option<TooltipHit<'_>>,
        point: Option<[f32; 2]>,
        now: Instant,
        timing: [f32; 3],
    ) -> Option<(&str, [f32; 2])> {
        let clock = *self.clock.get_or_insert(now);
        let seconds = now.saturating_duration_since(clock).as_secs_f32();
        if let Some(point) = point {
            self.point = point;
        }
        // 1402A4080 tests the stored byte lengths, then appends those bytes.
        // Spaces and authored leading newlines are part of the tooltip.
        let hit = point.and(hit).filter(|(text, delayed, shortcut, _)| {
            !text.is_empty()
                || delayed.is_some_and(|s| !s.is_empty())
                || shortcut.is_some_and(|s| !s.is_empty())
        });
        let mut candidate = std::mem::take(&mut self.candidate);
        candidate.clear();
        if let Some((text, delayed, shortcut, rect)) = hit {
            if self.target != Some(rect) {
                self.target = Some(rect);
                self.entered = Some(now);
            }
            candidate.push_str(text);
            let delayed = delayed.filter(|text| !text.is_empty());
            self.delayed_pending = delayed.is_some() && !past(self.entered, now, timing[1]);
            if !self.delayed_pending
                && let Some(delayed) = delayed
            {
                if !candidate.is_empty() {
                    candidate.push_str("\n--------------\n");
                }
                candidate.push_str(delayed);
            }
            // 1402A4080 appends the shortcut after the currently admitted body.
            if let Some(shortcut) = shortcut.filter(|text| !text.is_empty()) {
                if !candidate.is_empty() {
                    candidate.push_str("\n--------------\n");
                }
                candidate.push_str(shortcut);
            }
        } else {
            self.target = None;
            self.entered = None;
            self.delayed_pending = false;
        }
        if candidate.is_empty() {
            self.show_deadline = 0.0;
            self.showing = false;
            self.hiding = self.visible;
            if self.visible {
                // 1402A5C0B sets an unset deadline; 1402A5C3B hides once
                // `0 < deadline < now`.
                if self.hide_deadline <= 0.0 {
                    self.hide_deadline = seconds + timing[2];
                }
                if 0.0 < self.hide_deadline && self.hide_deadline < seconds {
                    self.visible = false;
                    self.hiding = false;
                    self.hide_deadline = 0.0;
                    self.text.clear();
                }
            }
            self.candidate = candidate;
        } else {
            self.hide_deadline = 0.0;
            self.hiding = false;
            self.candidate = std::mem::replace(&mut self.text, candidate);
            if !self.visible {
                self.showing = true;
                // 1402A5AF1 sets an unset deadline; 1402A5B1C shows unless
                // `deadline <= 0 || now <= deadline`.
                if self.show_deadline <= 0.0 {
                    self.show_deadline = seconds + timing[0];
                }
                if shows(self.show_deadline, seconds) {
                    self.visible = true;
                    self.showing = false;
                    self.show_deadline = 0.0;
                }
            }
        }
        self.visible.then_some((self.text.as_str(), self.point))
    }

    /// Request another frame at the next transition even with a stationary
    /// pointer. The native comparisons are strict (`deadline < now`), so a
    /// deadline already reached schedules the following clock quantum. An
    /// unset deadline (at or below 0) is set again on each frame, and holds
    /// once the clock passes the negated delay.
    pub(super) fn next_change(&self, now: Instant, timing: [f32; 3]) -> Option<Duration> {
        let seconds = self.clock.map_or(0.0, |clock| {
            now.saturating_duration_since(clock).as_secs_f32()
        });
        let deadline = |pending: bool, deadline: f32, delay: f32| {
            pending.then(|| {
                if deadline > 0.0 {
                    deadline - seconds
                } else {
                    -delay - seconds
                }
            })
        };
        [
            deadline(self.showing, self.show_deadline, timing[0]),
            self.entered
                .filter(|_| self.delayed_pending)
                .map(|start| timing[1] - now.saturating_duration_since(start).as_secs_f32()),
            deadline(self.hiding, self.hide_deadline, timing[2]),
        ]
        .into_iter()
        .flatten()
        .filter_map(|left| {
            // Only the interval left is a duration: none left is the next
            // quantum; a NaN or infinite one never passes (the comparisons
            // are false), so it schedules nothing.
            if !left.is_finite() {
                return None;
            }
            Duration::try_from_secs_f32(left.max(0.0))
                .ok()?
                .checked_add(Duration::from_nanos(1))
        })
        .min()
    }
}

/// 1402A4080's delayed-body test, `delay < now - start`: a negative delay
/// passes at once, a NaN never.
fn past(start: Option<Instant>, now: Instant, delay: f32) -> bool {
    start.is_some_and(|start| delay < now.saturating_duration_since(start).as_secs_f32())
}

/// 1402A4080's show test (1402A5B1C): it skips while `deadline <= 0` or
/// `now <= deadline`, so a NaN deadline shows at once.
fn shows(deadline: f32, now: f32) -> bool {
    use std::cmp::Ordering::{Equal, Less};
    !matches!(deadline.partial_cmp(&0.0), Some(Less | Equal))
        && !matches!(now.partial_cmp(&deadline), Some(Less | Equal))
}

/// Default, unanchored tooltip placement from `1402A4080`, in interface
/// pixels. Producer-specific relative anchors and its above-pointer flag are
/// not used by the land window.
fn position(point: [f32; 2], size: [f32; 2], extent: [f32; 2], offsets: [i32; 4]) -> [f32; 2] {
    let point = point.map(f32::trunc);
    let mut x = (point[0] + 21.0).max(offsets[0] as f32);
    let mut y = (point[1] + 24.0).max(offsets[1] as f32);
    if x > extent[0] - size[0] - offsets[2] as f32 {
        x = point[0] - size[0] - 20.0;
    }
    if y > extent[1] - size[1] - offsets[3] as f32 {
        y = point[1] - size[1] - 20.0;
    }
    [x.max(0.0), y.max(0.0)]
}

/// 1417D47E0 asks 14029FEA0 for CTextBox+0xF8 (the authored maximum
/// width), chooses the free side of the combat window, and anchors at y+250.
fn naval_ship_anchor(window: [f32; 4], pointer_x: f32, width: f32, screen_width: f32) -> [f32; 2] {
    let [x, y, w, _] = window.map(f32::trunc);
    let left = x - width;
    let right = x + w;
    let use_left =
        (pointer_x.trunc() < x + (w / 2.0).trunc() || screen_width <= right + width) && left > 0.0;
    [
        if use_left { left } else { right }.max(0.0),
        (y + 250.0).max(0.0),
    ]
}

fn placed(
    at: TooltipPosition,
    size: [f32; 2],
    extent: [f32; 2],
    offsets: [i32; 4],
    wrap_width: f32,
) -> [f32; 2] {
    let Some(window) = at.naval_window else {
        return position(at.point, size, extent, offsets);
    };
    let mut at = naval_ship_anchor(window, at.point[0], wrap_width, extent[0]);
    // 141994D20 sets an absolute anchor and below flag 1. 1402A5CFF
    // skips pointer offsets; overflowing explicit anchors subtract size+20
    // from the anchor itself (1402A5F1F/1402A5F82).
    for axis in 0..2 {
        at[axis] = at[axis].max(offsets[axis] as f32);
        if at[axis] > extent[axis] - size[axis] - offsets[axis + 2] as f32 {
            at[axis] -= size[axis] + 20.0;
        }
        at[axis] = at[axis].max(0.0);
    }
    at
}

/// 14029FEA0 reads the authored CTextBox+0xF8. CreateTextBox@142247010
/// selects defaultTextBox only when ToolTip is absent, not when its type is wrong.
fn authored_wrap_width(resources: &impl native_gui::Resources) -> Result<f32, native_gui::Refusal> {
    let element = resources
        .gui_type(native_gui::text_sprites::TOOLTIP)
        .or_else(|| resources.gui_type(b"defaultTextBox"))
        .ok_or_else(|| native_gui::Refusal {
            path: "ToolTip".into(),
            kind: native_gui::RefusalKind::NullRead,
            detail: "authored tooltip wrap width is unavailable: ToolTip and defaultTextBox are absent (142247010, 14029FEA0)".into(),
        })?;
    match &element.fields {
        installed_content::gui::native::Fields::TextSprite(text) => Ok(text.max_width as f32),
        _ => Err(native_gui::Refusal {
            path: element.path.clone(),
            kind: native_gui::RefusalKind::TypeConfusion,
            detail: "authored tooltip wrap width is unavailable: tooltip template is not a textBoxType (142247010, 14029FEA0)".into(),
        }),
    }
}

pub(crate) fn draw<R: native_gui::Resources + native_gui::Sheets>(
    resources: &R,
    batch: &mut Batch,
    value: &str,
    point: TooltipPosition,
    extent: [f32; 2],
    defines: &TooltipDefines,
    time: f32,
) -> std::collections::BTreeMap<String, usize> {
    let mut refused = std::collections::BTreeMap::new();
    let wrap_width = match authored_wrap_width(resources) {
        Ok(width) => width,
        Err(why) => {
            gui_windows::count_refused(&mut refused, "ToolTip", &[why]);
            return refused;
        }
    };
    let layout = match native_gui::text_sprites::tooltip(resources, value, time, true) {
        Ok(layout) => layout,
        Err(why) => {
            gui_windows::count_refused(&mut refused, "ToolTip", &why);
            return refused;
        }
    };
    gui_windows::count_noted(&mut refused, "ToolTip", &layout.diagnostics);
    let at = placed(
        point,
        layout.size.map(|v| v as f32),
        extent,
        defines.offsets,
        wrap_width,
    )
    .map(|v| v as i32);
    batch.at_layer(crate::map::sprites::TOOLTIP_LAYER);
    if let Err(why) = native_gui::draw(&layout, at, resources, batch) {
        gui_windows::count_refused(&mut refused, "ToolTip", &[why]);
    }
    refused
}

#[cfg(test)]
mod tests;
