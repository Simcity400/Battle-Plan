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
    pub meta: TooltipMetadata,
}

impl Tooltip {
    pub fn new(title: Atom) -> Tooltip {
        Tooltip {
            title,
            rows: Vec::new(),
            meta: TooltipMetadata::default(),
        }
    }

    pub fn composer(title: Atom) -> TooltipComposer {
        TooltipComposer::new(title)
    }

    pub fn domain_composer(title: Atom) -> DomainTooltipComposer {
        DomainTooltipComposer::new(title)
    }

    pub fn with_row(mut self, label: impl Into<String>, value: Atom) -> Tooltip {
        self.rows.push(TooltipRow::new(label, value));
        self
    }

    pub fn with_metadata(mut self, meta: TooltipMetadata) -> Tooltip {
        self.meta = meta;
        self
    }

    pub fn with_tooltip_key(self, key: impl Into<String>) -> Tooltip {
        let meta = self.meta.clone().with_tooltip_key(key);
        self.with_metadata(meta)
    }

    pub fn with_title_key(self, key: impl Into<String>) -> Tooltip {
        let meta = self.meta.clone().with_title_loc_key(key);
        self.with_metadata(meta)
    }

    pub fn with_source_key(self, key: impl Into<String>) -> Tooltip {
        let meta = self.meta.clone().with_source_key(key);
        self.with_metadata(meta)
    }

    pub fn with_row_metadata(
        mut self,
        label: impl Into<String>,
        value: Atom,
        meta: TooltipRowMeta,
    ) -> Tooltip {
        self.rows
            .push(TooltipRow::new(label, value).with_metadata(meta));
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TooltipRow {
    pub label: String,
    pub value: Atom,
    pub meta: TooltipRowMeta,
}

impl TooltipRow {
    pub fn new(label: impl Into<String>, value: Atom) -> TooltipRow {
        TooltipRow {
            label: label.into(),
            value,
            meta: TooltipRowMeta::default(),
        }
    }

    pub fn with_metadata(mut self, meta: TooltipRowMeta) -> TooltipRow {
        self.meta = meta;
        self
    }
}

/// Draw-inert provenance for a tooltip as a whole. These keys let parity tests
/// prove that a hover used the intended the game tooltip/localisation/source path even
/// when the resolved visible strings happen to match a fallback.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TooltipMetadata {
    pub tooltip_key: Option<String>,
    pub title_key: Option<String>,
    pub source_key: Option<String>,
}

impl TooltipMetadata {
    pub fn new() -> TooltipMetadata {
        TooltipMetadata::default()
    }

    pub fn with_tooltip_key(mut self, key: impl Into<String>) -> TooltipMetadata {
        self.tooltip_key = Some(key.into());
        self
    }

    pub fn tooltip_key(self, key: impl Into<String>) -> TooltipMetadata {
        self.with_tooltip_key(key)
    }

    pub fn with_title_key(mut self, key: impl Into<String>) -> TooltipMetadata {
        self.title_key = Some(key.into());
        self
    }

    pub fn with_title_loc_key(self, key: impl Into<String>) -> TooltipMetadata {
        self.with_title_key(key)
    }

    pub fn title_key(self, key: impl Into<String>) -> TooltipMetadata {
        self.with_title_key(key)
    }

    pub fn with_source_key(mut self, key: impl Into<String>) -> TooltipMetadata {
        self.source_key = Some(key.into());
        self
    }

    pub fn source_key(self, key: impl Into<String>) -> TooltipMetadata {
        self.with_source_key(key)
    }

    pub fn is_empty(&self) -> bool {
        self.tooltip_key.is_none() && self.title_key.is_none() && self.source_key.is_none()
    }

    fn digest(&self) -> String {
        format!(
            "tooltip_key={};title_key={};source_key={}",
            opt_meta_string(&self.tooltip_key),
            opt_meta_string(&self.title_key),
            opt_meta_string(&self.source_key)
        )
    }
}

/// Draw-inert provenance for one tooltip row. `label_key` and `value_key` are
/// localisation or scripted-tooltip keys; `source_key` is the domain/snapshot
/// field that supplied the atom or explains why it is unavailable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TooltipRowMeta {
    pub label_key: Option<String>,
    pub value_key: Option<String>,
    pub source_key: Option<String>,
}

impl TooltipRowMeta {
    pub fn new() -> TooltipRowMeta {
        TooltipRowMeta::default()
    }

    pub fn label_source(
        label_key: impl Into<String>,
        source_key: impl Into<String>,
    ) -> TooltipRowMeta {
        TooltipRowMeta::new()
            .label_key(label_key)
            .source_key(source_key)
    }

    pub fn label_key(mut self, key: impl Into<String>) -> TooltipRowMeta {
        self.label_key = Some(key.into());
        self
    }

    pub fn value_key(mut self, key: impl Into<String>) -> TooltipRowMeta {
        self.value_key = Some(key.into());
        self
    }

    pub fn source_key(mut self, key: impl Into<String>) -> TooltipRowMeta {
        self.source_key = Some(key.into());
        self
    }

    pub fn is_empty(&self) -> bool {
        self.label_key.is_none() && self.value_key.is_none() && self.source_key.is_none()
    }

    fn digest(&self) -> String {
        format!(
            "label_key={};value_key={};source_key={}",
            opt_meta_string(&self.label_key),
            opt_meta_string(&self.value_key),
            opt_meta_string(&self.source_key)
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TooltipComposer {
    tooltip: Tooltip,
}

impl TooltipComposer {
    pub fn new(title: Atom) -> TooltipComposer {
        TooltipComposer {
            tooltip: Tooltip::new(title),
        }
    }

    pub fn row(mut self, label: impl Into<String>, value: Atom) -> TooltipComposer {
        self.tooltip.rows.push(TooltipRow::new(label, value));
        self
    }

    pub fn metadata(mut self, meta: TooltipMetadata) -> TooltipComposer {
        self.tooltip.meta = meta;
        self
    }

    pub fn tooltip_key(self, key: impl Into<String>) -> TooltipComposer {
        let meta = self.tooltip.meta.clone().tooltip_key(key);
        self.metadata(meta)
    }

    pub fn title_key(self, key: impl Into<String>) -> TooltipComposer {
        let meta = self.tooltip.meta.clone().title_key(key);
        self.metadata(meta)
    }

    pub fn source_key(self, key: impl Into<String>) -> TooltipComposer {
        let meta = self.tooltip.meta.clone().source_key(key);
        self.metadata(meta)
    }

    pub fn row_metadata(
        mut self,
        label: impl Into<String>,
        value: Atom,
        meta: TooltipRowMeta,
    ) -> TooltipComposer {
        self.tooltip
            .rows
            .push(TooltipRow::new(label, value).with_metadata(meta));
        self
    }

    pub fn exact_row(self, label: impl Into<String>, value: impl Into<String>) -> TooltipComposer {
        self.row(label, Atom::exact(value))
    }

    pub fn derived_row(
        self,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> TooltipComposer {
        self.row(label, Atom::derived(value))
    }

    pub fn unavailable_row(self, label: impl Into<String>, reason: Reason) -> TooltipComposer {
        self.row(label, Atom::unavailable_because(reason))
    }

    pub fn exact_row_with_metadata(
        self,
        label: impl Into<String>,
        value: impl Into<String>,
        meta: TooltipRowMeta,
    ) -> TooltipComposer {
        self.row_metadata(label, Atom::exact(value), meta)
    }

    pub fn derived_row_with_metadata(
        self,
        label: impl Into<String>,
        value: impl Into<String>,
        meta: TooltipRowMeta,
    ) -> TooltipComposer {
        self.row_metadata(label, Atom::derived(value), meta)
    }

    pub fn unavailable_row_with_metadata(
        self,
        label: impl Into<String>,
        reason: Reason,
        meta: TooltipRowMeta,
    ) -> TooltipComposer {
        self.row_metadata(label, Atom::unavailable_because(reason), meta)
    }

    pub fn optional_derived_row(
        self,
        label: impl Into<String>,
        value: Option<impl Into<String>>,
        missing_reason: Reason,
    ) -> TooltipComposer {
        match value {
            Some(value) => self.derived_row(label, value),
            None => self.unavailable_row(label, missing_reason),
        }
    }

    pub fn build(self) -> Tooltip {
        self.tooltip
    }

    pub fn plan(self, anchor: (f32, f32), viewport: Rect) -> TooltipPlan {
        let tooltip = self.build();
        plan_tooltip(&tooltip, anchor, viewport)
    }
}

/// Composer for domain-owned tooltips where each expected field should resolve to a
/// visible row. Missing fields are supplied as unavailable atoms instead of dropping
/// the row, which keeps parity gaps visible at the hover site.
#[derive(Debug, Clone, PartialEq)]
pub struct DomainTooltipComposer {
    composer: TooltipComposer,
}

impl DomainTooltipComposer {
    pub fn new(title: Atom) -> DomainTooltipComposer {
        DomainTooltipComposer {
            composer: TooltipComposer::new(title),
        }
    }

    pub fn row(mut self, label: impl Into<String>, value: Atom) -> DomainTooltipComposer {
        self.composer = self.composer.row(label, value);
        self
    }

    pub fn metadata(mut self, meta: TooltipMetadata) -> DomainTooltipComposer {
        self.composer = self.composer.metadata(meta);
        self
    }

    pub fn tooltip_key(mut self, key: impl Into<String>) -> DomainTooltipComposer {
        self.composer = self.composer.tooltip_key(key);
        self
    }

    pub fn title_key(mut self, key: impl Into<String>) -> DomainTooltipComposer {
        self.composer = self.composer.title_key(key);
        self
    }

    pub fn source_key(mut self, key: impl Into<String>) -> DomainTooltipComposer {
        self.composer = self.composer.source_key(key);
        self
    }

    pub fn row_metadata(
        mut self,
        label: impl Into<String>,
        value: Atom,
        meta: TooltipRowMeta,
    ) -> DomainTooltipComposer {
        self.composer = self.composer.row_metadata(label, value, meta);
        self
    }

    pub fn exact_row(
        self,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> DomainTooltipComposer {
        self.row(label, Atom::exact(value))
    }

    pub fn derived_row(
        self,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> DomainTooltipComposer {
        self.row(label, Atom::derived(value))
    }

    pub fn unavailable_row(
        self,
        label: impl Into<String>,
        reason: Reason,
    ) -> DomainTooltipComposer {
        self.row(label, Atom::unavailable_because(reason))
    }

    pub fn exact_row_with_metadata(
        self,
        label: impl Into<String>,
        value: impl Into<String>,
        meta: TooltipRowMeta,
    ) -> DomainTooltipComposer {
        self.row_metadata(label, Atom::exact(value), meta)
    }

    pub fn derived_row_with_metadata(
        self,
        label: impl Into<String>,
        value: impl Into<String>,
        meta: TooltipRowMeta,
    ) -> DomainTooltipComposer {
        self.row_metadata(label, Atom::derived(value), meta)
    }

    pub fn unavailable_row_with_metadata(
        self,
        label: impl Into<String>,
        reason: Reason,
        meta: TooltipRowMeta,
    ) -> DomainTooltipComposer {
        self.row_metadata(label, Atom::unavailable_because(reason), meta)
    }

    pub fn required_exact_row<T: Into<String>>(
        self,
        label: impl Into<String>,
        value: Option<T>,
        missing_reason: Reason,
    ) -> DomainTooltipComposer {
        self.required_exact_row_with_metadata(
            label,
            value,
            missing_reason,
            TooltipRowMeta::default(),
        )
    }

    pub fn required_exact_row_with_metadata<T: Into<String>>(
        self,
        label: impl Into<String>,
        value: Option<T>,
        missing_reason: Reason,
        meta: TooltipRowMeta,
    ) -> DomainTooltipComposer {
        match value {
            Some(value) => self.exact_row_with_metadata(label, value, meta),
            None => self.unavailable_row_with_metadata(label, missing_reason, meta),
        }
    }

    pub fn required_derived_row<T: Into<String>>(
        self,
        label: impl Into<String>,
        value: Option<T>,
        missing_reason: Reason,
    ) -> DomainTooltipComposer {
        self.required_derived_row_with_metadata(
            label,
            value,
            missing_reason,
            TooltipRowMeta::default(),
        )
    }

    pub fn required_derived_row_with_metadata<T: Into<String>>(
        self,
        label: impl Into<String>,
        value: Option<T>,
        missing_reason: Reason,
        meta: TooltipRowMeta,
    ) -> DomainTooltipComposer {
        match value {
            Some(value) => self.derived_row_with_metadata(label, value, meta),
            None => self.unavailable_row_with_metadata(label, missing_reason, meta),
        }
    }

    pub fn build(self) -> Tooltip {
        self.composer.build()
    }

    pub fn plan(self, anchor: (f32, f32), viewport: Rect) -> TooltipPlan {
        self.composer.plan(anchor, viewport)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TooltipFrame {
    pub contract: &'static str,
    pub sprite: &'static str,
    pub sprite_border: [f32; 2],
    pub offset: f32,
    pub margin: f32,
    pub pad_x: f32,
    pub pad_y: f32,
    pub title_h: f32,
    pub row_h: f32,
    pub min_w: f32,
    pub max_w: f32,
    pub char_w: f32,
    pub shadow_offset: f32,
    pub shadow: [f32; 4],
    pub border: [f32; 4],
    pub trim: [f32; 4],
    pub panel: [f32; 4],
    pub top_rule: [f32; 4],
    pub bottom_rule: [f32; 4],
    pub title_rule: [f32; 4],
    pub badge_fill: [f32; 4],
    pub derived_badge_fill: [f32; 4],
}

impl TooltipFrame {
    pub const fn game() -> TooltipFrame {
        TooltipFrame {
            contract: "game-tooltip-v1",
            sprite: "GFX_mini_tooltip",
            sprite_border: [12.0, 12.0],
            offset: 14.0,
            margin: 6.0,
            pad_x: 10.0,
            pad_y: 8.0,
            title_h: 20.0,
            row_h: 18.0,
            min_w: 180.0,
            max_w: 340.0,
            char_w: 7.0,
            shadow_offset: 3.0,
            shadow: [0.0, 0.0, 0.0, 0.42],
            border: [0.45, 0.43, 0.35, 1.0],
            trim: [0.15, 0.16, 0.18, 1.0],
            panel: [0.07, 0.075, 0.085, 0.96],
            top_rule: [0.70, 0.62, 0.38, 1.0],
            bottom_rule: [0.03, 0.035, 0.04, 1.0],
            title_rule: [0.20, 0.21, 0.23, 1.0],
            badge_fill: [0.18, 0.15, 0.14, 1.0],
            derived_badge_fill: [0.20, 0.18, 0.11, 1.0],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TooltipChrome {
    pub sprite: &'static str,
    pub border: [f32; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct TooltipPlan {
    pub bounds: Rect,
    frame: TooltipFrame,
    metadata_digest: String,
    content_start: usize,
    plan: Plan,
}

impl TooltipPlan {
    pub fn digest(&self) -> String {
        format!(
            "TT {} chrome={}@{:.0},{:.0} {:.2},{:.2},{:.2},{:.2}{}|{}",
            self.frame.contract,
            self.frame.sprite,
            self.frame.sprite_border[0],
            self.frame.sprite_border[1],
            self.bounds.x,
            self.bounds.y,
            self.bounds.w,
            self.bounds.h,
            self.metadata_digest,
            self.plan.digest()
        )
    }

    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    pub fn frame_contract(&self) -> &'static str {
        self.frame.contract
    }

    pub fn metadata_digest(&self) -> &str {
        &self.metadata_digest
    }

    pub fn chrome(&self) -> TooltipChrome {
        TooltipChrome {
            sprite: self.frame.sprite,
            border: self.frame.sprite_border,
        }
    }

    pub fn draw(&self, renderer: &mut Renderer, assets: &DrawAssets) {
        self.plan.execute(renderer, assets, |_, _| false);
    }

    pub fn draw_content(&self, renderer: &mut Renderer, assets: &DrawAssets) {
        let content = Plan {
            cmds: self.plan.cmds[self.content_start..].to_vec(),
        };
        content.execute(renderer, assets, |_, _| false);
    }
}

pub fn plan_tooltip(model: &Tooltip, anchor: (f32, f32), viewport: Rect) -> TooltipPlan {
    plan_tooltip_with_frame(model, anchor, viewport, TooltipFrame::game())
}

pub fn plan_tooltip_with_frame(
    model: &Tooltip,
    anchor: (f32, f32),
    viewport: Rect,
    frame: TooltipFrame,
) -> TooltipPlan {
    let viewport = sane_rect(viewport);
    let natural_w = natural_width(model, frame);
    let max_allowed = (viewport.w - frame.margin * 2.0).max(80.0);
    let w = natural_w.clamp(frame.min_w, frame.max_w).min(max_allowed);
    let h = (frame.pad_y * 2.0 + frame.title_h + model.rows.len() as f32 * frame.row_h).max(38.0);
    let (x, y) = clamped_position(anchor, viewport, w, h, frame);
    let bounds = Rect { x, y, w, h };

    let mut p = Plan::new();
    p.push(Cmd::Rect {
        x: x + frame.shadow_offset,
        y: y + frame.shadow_offset,
        w,
        h,
        tint: frame.shadow,
        clip: None,
    });
    p.push(Cmd::Rect {
        x,
        y,
        w,
        h,
        tint: frame.border,
        clip: None,
    });
    p.push(Cmd::Rect {
        x: x + 1.0,
        y: y + 1.0,
        w: (w - 2.0).max(0.0),
        h: (h - 2.0).max(0.0),
        tint: frame.trim,
        clip: None,
    });
    p.push(Cmd::Rect {
        x: x + 2.0,
        y: y + 2.0,
        w: (w - 4.0).max(0.0),
        h: (h - 4.0).max(0.0),
        tint: frame.panel,
        clip: None,
    });
    p.push(Cmd::Rect {
        x: x + 2.0,
        y: y + 2.0,
        w: (w - 4.0).max(0.0),
        h: 1.0,
        tint: frame.top_rule,
        clip: None,
    });
    p.push(Cmd::Rect {
        x: x + 2.0,
        y: y + h - 3.0,
        w: (w - 4.0).max(0.0),
        h: 1.0,
        tint: frame.bottom_rule,
        clip: None,
    });

    let content_start = p.len();
    p.push(Cmd::Text {
        x: x + frame.pad_x,
        y: y + frame.pad_y,
        text: model.title.as_str().to_string(),
        tint: model.title.tint(TEXT_TINT),
        clip: Some([
            x + frame.pad_x,
            y,
            (w - frame.pad_x * 2.0).max(0.0),
            frame.title_h + frame.pad_y,
        ]),
    });
    p.push(Cmd::Rect {
        x: x + frame.pad_x,
        y: y + frame.pad_y + frame.title_h - 2.0,
        w: (w - frame.pad_x * 2.0).max(0.0),
        h: 1.0,
        tint: frame.title_rule,
        clip: None,
    });

    let label_w = ((w - frame.pad_x * 2.0) * 0.42).clamp(64.0, 130.0);
    let value_x = x + frame.pad_x + label_w;
    let value_w = (w - frame.pad_x * 2.0 - label_w).max(30.0);
    for (i, row) in model.rows.iter().enumerate() {
        let row_y = y + frame.pad_y + frame.title_h + i as f32 * frame.row_h;
        p.push(Cmd::Text {
            x: x + frame.pad_x,
            y: row_y,
            text: row.label.clone(),
            tint: DIM_TINT,
            clip: Some([x + frame.pad_x, row_y - 2.0, label_w - 4.0, frame.row_h]),
        });
        p.push(Cmd::Text {
            x: value_x,
            y: row_y,
            text: row.value.as_str().to_string(),
            tint: row.value.tint(TEXT_TINT),
            clip: Some([value_x, row_y - 2.0, value_w, frame.row_h]),
        });
        if let Some((badge, fill, tint)) = badge_for(&row.value, frame) {
            let bw = badge_width(badge, frame);
            let bx = (x + w - frame.pad_x - bw).max(value_x + 22.0);
            p.push(Cmd::Rect {
                x: bx,
                y: row_y - 1.0,
                w: bw,
                h: 14.0,
                tint: fill,
                clip: Some([
                    x + frame.pad_x,
                    row_y - 2.0,
                    w - frame.pad_x * 2.0,
                    frame.row_h,
                ]),
            });
            p.push(Cmd::Text {
                x: bx + 4.0,
                y: row_y,
                text: badge.to_string(),
                tint,
                clip: Some([bx, row_y - 2.0, bw, frame.row_h]),
            });
        }
    }

    TooltipPlan {
        bounds,
        frame,
        metadata_digest: metadata_digest(model),
        content_start,
        plan: p,
    }
}

fn metadata_digest(model: &Tooltip) -> String {
    let mut parts = Vec::new();
    if !model.meta.is_empty() {
        parts.push(format!("tip{{{}}}", model.meta.digest()));
    }
    for (i, row) in model.rows.iter().enumerate() {
        if !row.meta.is_empty() {
            parts.push(format!("row{i}{{{}}}", row.meta.digest()));
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" meta{{{}}}", parts.join(";"))
    }
}

fn opt_meta_string(value: &Option<String>) -> String {
    value
        .as_deref()
        .map(meta_string)
        .unwrap_or_else(|| "-".to_string())
}

fn meta_string(s: &str) -> String {
    format!("{s:?}")
}

fn natural_width(model: &Tooltip, frame: TooltipFrame) -> f32 {
    let mut chars = model.title.as_str().chars().count();
    for row in &model.rows {
        let badge = badge_for(&row.value, frame)
            .map(|(b, _, _)| b.chars().count() + 3)
            .unwrap_or(0);
        let row_chars = row.label.chars().count() + row.value.as_str().chars().count() + badge + 6;
        chars = chars.max(row_chars);
    }
    frame.pad_x * 2.0 + chars as f32 * frame.char_w
}

fn badge_for(atom: &Atom, frame: TooltipFrame) -> Option<(&'static str, [f32; 4], [f32; 4])> {
    match atom {
        Atom::Unavailable { reason } => Some((
            match reason {
                Reason::Absent => "missing",
                Reason::Null => "null",
                Reason::Blocked => "unfed",
            },
            frame.badge_fill,
            UNAVAIL_TINT,
        )),
        Atom::Text {
            fidelity: Fidelity::Derived,
            ..
        } => Some((
            "derived",
            frame.derived_badge_fill,
            crate::common::DERIVED_TINT,
        )),
        Atom::Text { .. } => None,
    }
}

fn badge_width(text: &str, frame: TooltipFrame) -> f32 {
    text.chars().count() as f32 * frame.char_w + 8.0
}

fn clamped_position(
    anchor: (f32, f32),
    viewport: Rect,
    w: f32,
    h: f32,
    frame: TooltipFrame,
) -> (f32, f32) {
    let right = viewport.x + viewport.w - frame.margin;
    let bottom = viewport.y + viewport.h - frame.margin;
    let left = viewport.x + frame.margin;
    let top = viewport.y + frame.margin;

    let mut x = anchor.0 + frame.offset;
    if x + w > right {
        x = anchor.0 - w - frame.offset;
    }
    let mut y = anchor.1 + frame.offset;
    if y + h > bottom {
        y = anchor.1 - h - frame.offset;
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
        assert_eq!(a.frame_contract(), "game-tooltip-v1");
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

    #[test]
    fn game_frame_contract_emits_ordered_chrome() {
        let frame = TooltipFrame::game();
        let tt = Tooltip::new(Atom::exact("Frame")).with_row("State", Atom::derived("researched"));
        let plan = plan_tooltip(&tt, (20.0, 20.0), vp());
        let cmds = &plan.plan().cmds;
        let chrome = plan.chrome();

        assert_eq!(chrome.sprite, "GFX_mini_tooltip");
        assert_eq!(chrome.border, [12.0, 12.0]);
        match &cmds[0] {
            Cmd::Rect { x, y, tint, .. } => {
                assert_eq!(*tint, frame.shadow);
                assert_eq!(*x, plan.bounds.x + frame.shadow_offset);
                assert_eq!(*y, plan.bounds.y + frame.shadow_offset);
            }
            other => panic!("first tooltip command must be frame shadow: {other:?}"),
        }
        match &cmds[1] {
            Cmd::Rect { tint, .. } => assert_eq!(*tint, frame.border),
            other => panic!("second tooltip command must be outer border: {other:?}"),
        }
        match &cmds[3] {
            Cmd::Rect { tint, .. } => assert_eq!(*tint, frame.panel),
            other => panic!("fourth tooltip command must be panel fill: {other:?}"),
        }
        assert!(
            plan.digest().starts_with("TT game-tooltip-v1"),
            "{}",
            plan.digest()
        );
        assert!(plan.digest().contains("GFX_mini_tooltip@12,12"));
        assert!(matches!(
            &cmds[plan.content_start],
            Cmd::Text { text, .. } if text == "Frame"
        ));
        assert!(plan.digest().contains("derived"), "{}", plan.digest());
    }

    #[test]
    fn composer_preserves_legacy_factual_rows() {
        let legacy = Tooltip::new(Atom::exact("Province #123"))
            .with_row("Province", Atom::exact("#123"))
            .with_row("Owner", Atom::exact("GER"))
            .with_row(
                "Computed supply",
                Atom::unavailable_because(Reason::Blocked),
            );
        let composed = Tooltip::composer(Atom::exact("Province #123"))
            .exact_row("Province", "#123")
            .exact_row("Owner", "GER")
            .unavailable_row("Computed supply", Reason::Blocked)
            .build();

        assert_eq!(legacy, composed);
        assert_eq!(
            plan_tooltip(&legacy, (40.0, 40.0), vp()).digest(),
            plan_tooltip(&composed, (40.0, 40.0), vp()).digest()
        );
    }

    #[test]
    fn composer_keeps_missing_computed_rows_explicit() {
        let tt = Tooltip::composer(Atom::exact("Tech"))
            .exact_row("ID", "tech_a")
            .optional_derived_row("Computed bonus", Option::<String>::None, Reason::Blocked)
            .build();

        assert_eq!(tt.rows.len(), 2);
        assert_eq!(tt.rows[1].label, "Computed bonus");
        assert_eq!(tt.rows[1].value, Atom::unavailable_because(Reason::Blocked));

        let plan = plan_tooltip(&tt, (10.0, 10.0), vp());
        let digest = plan.digest();
        assert!(digest.contains("Computed bonus"), "{digest}");
        assert!(digest.contains(crate::common::NOT_CAPTURED), "{digest}");
        assert!(digest.contains("unfed"), "{digest}");
        assert_eq!(plan.plan().unavailable_text_count(), 1);
        assert!(!digest.contains("\"0\""), "{digest}");
    }

    #[test]
    fn domain_composer_badges_required_rows_instead_of_omitting_them() {
        let tt = Tooltip::domain_composer(Atom::exact("Faction"))
            .exact_row("Country", "GER")
            .required_exact_row("Name", Option::<String>::None, Reason::Absent)
            .required_derived_row("Projected rank", Option::<String>::None, Reason::Blocked)
            .build();

        assert_eq!(tt.rows.len(), 3);
        assert_eq!(tt.rows[1].label, "Name");
        assert_eq!(tt.rows[1].value, Atom::unavailable_because(Reason::Absent));
        assert_eq!(tt.rows[2].label, "Projected rank");
        assert_eq!(tt.rows[2].value, Atom::unavailable_because(Reason::Blocked));

        let plan = plan_tooltip(&tt, (20.0, 20.0), vp());
        let digest = plan.digest();
        assert!(digest.contains("Name"), "{digest}");
        assert!(digest.contains("Projected rank"), "{digest}");
        assert!(digest.contains(crate::common::NOT_CAPTURED), "{digest}");
        assert!(digest.contains("missing"), "{digest}");
        assert!(digest.contains("unfed"), "{digest}");
        assert_eq!(plan.plan().unavailable_text_count(), 2);
        assert!(!digest.contains("\"0\""), "{digest}");
    }

    #[test]
    fn tooltip_metadata_marks_source_keys_without_changing_draw_plan() {
        let plain = Tooltip::composer(Atom::exact("Panzer IV"))
            .exact_row("ID", "medium_tank_chassis")
            .derived_row("Role", "armor")
            .build();
        let tagged = Tooltip::composer(Atom::exact("Panzer IV"))
            .tooltip_key("technology.tooltip")
            .title_key("medium_tank_chassis")
            .source_key("common/technologies/armor.txt:medium_tank_chassis")
            .exact_row_with_metadata(
                "ID",
                "medium_tank_chassis",
                TooltipRowMeta::new()
                    .label_key("TECH_ID")
                    .value_key("medium_tank_chassis")
                    .source_key("tech.id"),
            )
            .derived_row_with_metadata(
                "Role",
                "armor",
                TooltipRowMeta::new()
                    .label_key("TECH_ROLE")
                    .source_key("derived.role"),
            )
            .build();

        let plain_plan = plan_tooltip(&plain, (24.0, 24.0), vp());
        let tagged_plan = plan_tooltip(&tagged, (24.0, 24.0), vp());
        let digest = tagged_plan.digest();

        assert_eq!(plain_plan.plan().digest(), tagged_plan.plan().digest());
        assert_ne!(plain_plan.digest(), tagged_plan.digest());
        assert!(
            digest.contains("tip{tooltip_key=\"technology.tooltip\""),
            "{digest}"
        );
        assert!(
            digest.contains("title_key=\"medium_tank_chassis\""),
            "{digest}"
        );
        assert!(
            digest.contains("source_key=\"common/technologies/armor.txt:medium_tank_chassis\""),
            "{digest}"
        );
        assert!(digest.contains("row0{label_key=\"TECH_ID\""), "{digest}");
        assert!(
            digest.contains("value_key=\"medium_tank_chassis\""),
            "{digest}"
        );
        assert!(digest.contains("row1{label_key=\"TECH_ROLE\""), "{digest}");
        assert!(
            !plain_plan.digest().contains("meta{"),
            "{}",
            plain_plan.digest()
        );
    }

    #[test]
    fn required_row_metadata_survives_missing_atom() {
        let tt = Tooltip::domain_composer(Atom::exact("Decision"))
            .required_exact_row_with_metadata(
                "Effect",
                Option::<String>::None,
                Reason::Blocked,
                TooltipRowMeta::new()
                    .label_key("DECISION_EFFECT")
                    .value_key("decision_effect_expand_industry")
                    .source_key("decision.effect"),
            )
            .build();

        let plan = plan_tooltip(&tt, (20.0, 20.0), vp());
        let digest = plan.digest();

        assert!(
            digest.contains("row0{label_key=\"DECISION_EFFECT\""),
            "{digest}"
        );
        assert!(
            digest.contains("value_key=\"decision_effect_expand_industry\""),
            "{digest}"
        );
        assert!(
            digest.contains("source_key=\"decision.effect\""),
            "{digest}"
        );
        assert!(digest.contains("unfed"), "{digest}");
        assert_eq!(plan.plan().unavailable_text_count(), 1);
        assert!(!digest.contains("\"0\""), "{digest}");
    }

    #[test]
    fn domain_metadata_snapshot_retains_row_label_value_and_source_keys() {
        let mut state_name_meta =
            TooltipRowMeta::label_source("STATE_NAME", "render.map.PickResult.state_name_key");
        state_name_meta = state_name_meta.value_key("STATE_64");

        let tt = Tooltip::domain_composer(Atom::exact("Province #123"))
            .tooltip_key("core.map.hover.province")
            .title_key("PROVINCE")
            .source_key("render.map.PickResult")
            .exact_row_with_metadata(
                "Province",
                "#123",
                TooltipRowMeta::label_source("PROVINCE", "render.map.PickResult.province_id"),
            )
            .exact_row_with_metadata("State name", "STATE_64", state_name_meta)
            .build();

        let plan = plan_tooltip(&tt, (50.0, 60.0), vp());
        let snapshot = r#" meta{tip{tooltip_key="core.map.hover.province";title_key="PROVINCE";source_key="render.map.PickResult"};row0{label_key="PROVINCE";value_key=-;source_key="render.map.PickResult.province_id"};row1{label_key="STATE_NAME";value_key="STATE_64";source_key="render.map.PickResult.state_name_key"}}"#;

        assert_eq!(plan.metadata_digest(), snapshot);
        assert_eq!(
            plan.digest(),
            plan_tooltip(&tt, (50.0, 60.0), vp()).digest(),
            "metadata snapshots must be stable for parity tests"
        );
    }
}
