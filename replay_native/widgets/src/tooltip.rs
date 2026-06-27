//! Generic passive tooltip primitive.
//!
//! Tooltips are render plans only: the caller owns hover/input state and hands this
//! module factual rows. Values use [`Atom`], so unavailable fields render the
//! not-captured glyph plus a small reason badge rather than a fabricated value.

use gui_runtime::Rect;
use render::Renderer;

use crate::common::{Atom, DrawAssets, Fidelity, Reason, DIM_TINT, TEXT_TINT, UNAVAIL_TINT};
use crate::plan::{Cmd, Plan};

#[derive(Debug, Clone, PartialEq)]
pub struct Tooltip {
    pub title: Atom,
    pub rows: Vec<TooltipRow>,
}

impl Tooltip {
    pub fn new(title: Atom) -> Tooltip {
        Tooltip {
            title,
            rows: Vec::new(),
        }
    }

    pub fn with_row(mut self, label: impl Into<String>, value: Atom) -> Tooltip {
        self.rows.push(TooltipRow {
            label: label.into(),
            value,
        });
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TooltipRow {
    pub label: String,
    pub value: Atom,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TooltipPlan {
    pub bounds: Rect,
    plan: Plan,
}

impl TooltipPlan {
    pub fn digest(&self) -> String {
        format!(
            "TT {:.2},{:.2},{:.2},{:.2}|{}",
            self.bounds.x,
            self.bounds.y,
            self.bounds.w,
            self.bounds.h,
            self.plan.digest()
        )
    }

    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    pub fn draw(&self, renderer: &mut Renderer, assets: &DrawAssets) {
        self.plan.execute(renderer, assets, |_, _| false);
    }
}

const OFFSET: f32 = 14.0;
const MARGIN: f32 = 6.0;
const PAD_X: f32 = 10.0;
const PAD_Y: f32 = 8.0;
const TITLE_H: f32 = 20.0;
const ROW_H: f32 = 18.0;
const MIN_W: f32 = 180.0;
const MAX_W: f32 = 340.0;
const CHAR_W: f32 = 7.0;
const PANEL: [f32; 4] = [0.07, 0.075, 0.085, 0.96];
const BORDER: [f32; 4] = [0.40, 0.42, 0.45, 1.0];
const RULE: [f32; 4] = [0.20, 0.21, 0.23, 1.0];
const BADGE_FILL: [f32; 4] = [0.18, 0.15, 0.14, 1.0];
const DERIVED_BADGE_FILL: [f32; 4] = [0.20, 0.18, 0.11, 1.0];

pub fn plan_tooltip(model: &Tooltip, anchor: (f32, f32), viewport: Rect) -> TooltipPlan {
    let viewport = sane_rect(viewport);
    let natural_w = natural_width(model);
    let max_allowed = (viewport.w - MARGIN * 2.0).max(80.0);
    let w = natural_w.clamp(MIN_W, MAX_W).min(max_allowed);
    let h = (PAD_Y * 2.0 + TITLE_H + model.rows.len() as f32 * ROW_H).max(38.0);
    let (x, y) = clamped_position(anchor, viewport, w, h);
    let bounds = Rect { x, y, w, h };

    let mut p = Plan::new();
    p.push(Cmd::Rect {
        x,
        y,
        w,
        h,
        tint: BORDER,
        clip: None,
    });
    p.push(Cmd::Rect {
        x: x + 1.0,
        y: y + 1.0,
        w: (w - 2.0).max(0.0),
        h: (h - 2.0).max(0.0),
        tint: PANEL,
        clip: None,
    });

    p.push(Cmd::Text {
        x: x + PAD_X,
        y: y + PAD_Y,
        text: model.title.as_str().to_string(),
        tint: model.title.tint(TEXT_TINT),
        clip: Some([x + PAD_X, y, (w - PAD_X * 2.0).max(0.0), TITLE_H + PAD_Y]),
    });
    p.push(Cmd::Rect {
        x: x + PAD_X,
        y: y + PAD_Y + TITLE_H - 2.0,
        w: (w - PAD_X * 2.0).max(0.0),
        h: 1.0,
        tint: RULE,
        clip: None,
    });

    let label_w = ((w - PAD_X * 2.0) * 0.42).clamp(64.0, 130.0);
    let value_x = x + PAD_X + label_w;
    let value_w = (w - PAD_X * 2.0 - label_w).max(30.0);
    for (i, row) in model.rows.iter().enumerate() {
        let row_y = y + PAD_Y + TITLE_H + i as f32 * ROW_H;
        p.push(Cmd::Text {
            x: x + PAD_X,
            y: row_y,
            text: row.label.clone(),
            tint: DIM_TINT,
            clip: Some([x + PAD_X, row_y - 2.0, label_w - 4.0, ROW_H]),
        });
        p.push(Cmd::Text {
            x: value_x,
            y: row_y,
            text: row.value.as_str().to_string(),
            tint: row.value.tint(TEXT_TINT),
            clip: Some([value_x, row_y - 2.0, value_w, ROW_H]),
        });
        if let Some((badge, fill, tint)) = badge_for(&row.value) {
            let bw = badge_width(badge);
            let bx = (x + w - PAD_X - bw).max(value_x + 22.0);
            p.push(Cmd::Rect {
                x: bx,
                y: row_y - 1.0,
                w: bw,
                h: 14.0,
                tint: fill,
                clip: Some([x + PAD_X, row_y - 2.0, w - PAD_X * 2.0, ROW_H]),
            });
            p.push(Cmd::Text {
                x: bx + 4.0,
                y: row_y,
                text: badge.to_string(),
                tint,
                clip: Some([bx, row_y - 2.0, bw, ROW_H]),
            });
        }
    }

    TooltipPlan { bounds, plan: p }
}

fn natural_width(model: &Tooltip) -> f32 {
    let mut chars = model.title.as_str().chars().count();
    for row in &model.rows {
        let badge = badge_for(&row.value)
            .map(|(b, _, _)| b.chars().count() + 3)
            .unwrap_or(0);
        let row_chars = row.label.chars().count() + row.value.as_str().chars().count() + badge + 6;
        chars = chars.max(row_chars);
    }
    PAD_X * 2.0 + chars as f32 * CHAR_W
}

fn badge_for(atom: &Atom) -> Option<(&'static str, [f32; 4], [f32; 4])> {
    match atom {
        Atom::Unavailable { reason } => Some((
            match reason {
                Reason::Absent => "missing",
                Reason::Null => "null",
                Reason::Blocked => "unfed",
            },
            BADGE_FILL,
            UNAVAIL_TINT,
        )),
        Atom::Text {
            fidelity: Fidelity::Derived,
            ..
        } => Some(("derived", DERIVED_BADGE_FILL, crate::common::DERIVED_TINT)),
        Atom::Text { .. } => None,
    }
}

fn badge_width(text: &str) -> f32 {
    text.chars().count() as f32 * CHAR_W + 8.0
}

fn clamped_position(anchor: (f32, f32), viewport: Rect, w: f32, h: f32) -> (f32, f32) {
    let right = viewport.x + viewport.w - MARGIN;
    let bottom = viewport.y + viewport.h - MARGIN;
    let left = viewport.x + MARGIN;
    let top = viewport.y + MARGIN;

    let mut x = anchor.0 + OFFSET;
    if x + w > right {
        x = anchor.0 - w - OFFSET;
    }
    let mut y = anchor.1 + OFFSET;
    if y + h > bottom {
        y = anchor.1 - h - OFFSET;
    }

    let max_x = (right - w).max(left);
    let max_y = (bottom - h).max(top);
    (x.clamp(left, max_x), y.clamp(top, max_y))
}

fn sane_rect(r: Rect) -> Rect {
    let f = |v: f32| if v.is_finite() { v } else { 0.0 };
    Rect {
        x: f(r.x),
        y: f(r.y),
        w: f(r.w).max(0.0),
        h: f(r.h).max(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            w: 320.0,
            h: 180.0,
        }
    }

    #[test]
    fn planner_clamps_to_viewport_and_is_stable() {
        let tt = Tooltip::new(Atom::exact("Long title"))
            .with_row("ID", Atom::exact("tech_a"))
            .with_row("Cost", Atom::unavailable_because(Reason::Null));
        let a = plan_tooltip(&tt, (310.0, 170.0), vp());
        let b = plan_tooltip(&tt, (310.0, 170.0), vp());
        assert_eq!(a.digest(), b.digest());
        assert!(a.bounds.x >= 6.0);
        assert!(a.bounds.y >= 6.0);
        assert!(a.bounds.x + a.bounds.w <= 314.0 + 0.01);
        assert!(a.bounds.y + a.bounds.h <= 174.0 + 0.01);
    }

    #[test]
    fn missing_value_renders_glyph_and_badge_not_zero() {
        let tt = Tooltip::new(Atom::exact("Tech"))
            .with_row("Research cost", Atom::unavailable_because(Reason::Null));
        let plan = plan_tooltip(&tt, (10.0, 10.0), vp());
        let digest = plan.digest();
        assert!(digest.contains("null"), "{digest}");
        assert!(plan.plan().unavailable_text_count() >= 1);
        assert!(!digest.contains("\"0\""), "{digest}");
    }
}
