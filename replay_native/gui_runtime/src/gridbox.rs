//! GRIDBOX ROW-FACTORY + SCROLL VIEWPORT + SCROLLBAR (M5 foundations 3a).
//!
//! A `gridBoxType` in Clausewitz is NOT a static container: it INSTANTIATES one
//! copy of a named ENTRY-TEMPLATE window per bound array element, tiling the
//! instances on a grid (`slotsize` cells, `format` = flow direction), and clips
//! the whole thing to the gridbox's own box. When the instances overflow that box
//! a vertical scrollbar drives a scroll offset that shifts the rows up and the
//! clip rejects what falls outside the viewport.
//!
//! This module is the first real special-widget behaviour in the runtime. It is
//! deliberately generic: it takes a TEMPLATE [`Node`] (one of the parsed entry
//! windows, e.g. `state_building_entry`) and a count `n`, and returns `n` laid-out
//! clones positioned at their grid cells, each pre-offset by the scroll position
//! and carrying the viewport clip — exactly what the draw/hit-test passes already
//! consume (they honour `node.clip`). The caller fills each clone's bound values
//! (level text, building icon frame) BEFORE or AFTER instantiation via the row's
//! element names; M5's bind layer mutates the clones.
//!
//! Codex-required correctness (refuted/confirmed below in tests):
//!   * one instance PER element, at the correct per-row grid cell,
//!   * the viewport CLIP rejects rows scrolled out of the box,
//!   * the SCROLL OFFSET shifts every row by the same pixel delta,
//!   * the scrollbar thumb size/þposition reflects viewport/content ratio.

use crate::layout::{layout, ContentSize};
use crate::scene::{ElementKind, Node, Rect};

/// Flow direction of a gridbox's instances (from the gridbox `format`/`text`
/// block). the game state-view building grids flow LEFT-TO-RIGHT then wrap DOWN.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridFlow {
    /// fill a row left→right, then move to the next row (UPPER_LEFT default).
    RowMajor,
    /// fill a column top→bottom, then move to the next column.
    ColumnMajor,
}

/// A laid-out gridbox: the visible viewport rect, the instantiated+clipped row
/// clones, the full content height, and the scrollbar geometry. The rows are
/// ready for the existing draw/hit-test passes (they carry `rect` + `clip`).
#[derive(Debug, Clone)]
pub struct GridInstances {
    /// The clipped viewport rect (the gridbox's own box on screen).
    pub viewport: Rect,
    /// One laid-out clone per bound element, scroll-shifted + viewport-clipped.
    pub rows: Vec<Node>,
    /// Total laid-out content height (all rows, unclipped) — drives overflow.
    pub content_height: f32,
    /// True when content_height exceeds the viewport (a scrollbar is needed).
    pub overflowing: bool,
    /// The scrollbar track + thumb rects (None when not overflowing).
    pub scrollbar: Option<ScrollBar>,
}

/// A vertical scrollbar's geometry for a scrollable viewport.
#[derive(Debug, Clone, Copy)]
pub struct ScrollBar {
    /// The full track rect (right edge of the viewport).
    pub track: Rect,
    /// The thumb rect (sized by viewport/content, positioned by scroll offset).
    pub thumb: Rect,
}

impl ScrollBar {
    /// Max scrollable pixels (content beyond the viewport).
    pub fn max_scroll(viewport_h: f32, content_h: f32) -> f32 {
        (content_h - viewport_h).max(0.0)
    }
}

/// The per-grid layout parameters resolved from the parsed gridbox node.
#[derive(Debug, Clone, Copy)]
pub struct GridSpec {
    /// cell width / height in px (the `slotsize`, or the template's own size).
    pub cell_w: f32,
    pub cell_h: f32,
    /// number of columns that fit across the viewport (>=1).
    pub columns: usize,
    pub flow: GridFlow,
    /// width of the vertical scrollbar gutter (px), reserved on the right.
    pub scrollbar_w: f32,
}

impl GridSpec {
    /// Derive a grid spec from a parsed gridbox node + its entry template, using
    /// the template's size as the cell and the gridbox box width for the column
    /// count. `scrollbar_w` is reserved on the right when overflowing.
    pub fn from_gridbox(gridbox: &Node, template: &Node, scrollbar_w: f32) -> GridSpec {
        // cell size = the template's declared size (state_building_entry = 140x70).
        let (cell_w, cell_h) = template_cell_size(template);
        // viewport width drives the column count. The gridbox carries an explicit
        // size; if absent, fall back to its laid-out rect.
        let vp_w = node_box_w(gridbox).max(cell_w);
        let usable = (vp_w - scrollbar_w).max(cell_w);
        let columns = ((usable / cell_w).floor() as usize).max(1);
        GridSpec {
            cell_w,
            cell_h,
            columns,
            flow: GridFlow::RowMajor,
            scrollbar_w,
        }
    }
}

/// The declared cell size of an entry template (its `size_w`/`size_h` px, else a
/// small default so a sizeless template still tiles).
fn template_cell_size(template: &Node) -> (f32, f32) {
    let w = scalar_px(template.size_w.as_ref()).unwrap_or(140.0);
    let h = scalar_px(template.size_h.as_ref()).unwrap_or(70.0);
    (w, h)
}

/// The box width of a node from its raw size scalar (px only; percent handled by
/// the caller's already-laid-out rect when present).
fn node_box_w(node: &Node) -> f32 {
    if node.rect.w > 0.0 {
        node.rect.w
    } else {
        scalar_px(node.size_w.as_ref()).unwrap_or(0.0)
    }
}

fn scalar_px(s: Option<&crate::model::Scalar>) -> Option<f32> {
    match s {
        Some(crate::model::Scalar::Int(i)) if *i > 0 => Some(*i as f32),
        Some(crate::model::Scalar::Float(f)) if *f > 0.0 => Some(*f as f32),
        Some(crate::model::Scalar::Str(t)) => t.trim().parse::<f32>().ok().filter(|v| *v > 0.0),
        _ => None,
    }
}

/// INSTANTIATE the row-factory: produce `count` laid-out clones of `template`,
/// tiled on the grid inside `viewport`, scrolled by `scroll_y` pixels, and clipped
/// to the viewport. `content` resolves sprite/text content sizes (same trait the
/// layout pass uses). This is the heart of the gridbox special-widget.
///
/// The returned rows carry absolute `rect`s (already shifted by `-scroll_y`) and a
/// `clip` set to the viewport, so the EXISTING draw + hit-test passes render and
/// hit-test them correctly with no special-casing downstream.
pub fn instantiate<C: ContentSize>(
    template: &Node,
    count: usize,
    viewport: Rect,
    spec: &GridSpec,
    scroll_y: f32,
    content: &C,
) -> GridInstances {
    let mut rows: Vec<Node> = Vec::with_capacity(count);

    // Sanitize the column count: `GridSpec` is public + constructible, so a
    // degenerate `columns: 0` must never reach the `i % columns` / `i / columns`
    // below (divide-by-zero PANIC). `from_gridbox` already guarantees >=1, but a
    // hand-built spec might not (Codex).
    let columns = spec.columns.max(1);

    // total content height = number of grid ROWS * cell height. `div_ceil` avoids
    // the `count + columns - 1` add that could overflow for a hostile huge column
    // count (Codex).
    let grid_rows = count.div_ceil(columns);
    let content_height = grid_rows as f32 * spec.cell_h;
    // clamp scroll into range; `overflowing` is defined CONSISTENTLY with
    // `max_scroll`: a scrollbar exists iff there is a positive scrollable range
    // (Codex (a): the prior `+0.5` epsilon made `overflowing` and `max_scroll`
    // disagree for sub-0.5px content). Now both derive from the same `max_scroll`.
    let max_scroll = (content_height - viewport.h).max(0.0);
    let overflowing = max_scroll > 0.0;
    let scroll_y = scroll_y.clamp(0.0, max_scroll);

    for i in 0..count {
        // RowMajor is the validated Clausewitz flow (state-view building grids:
        // fill a row left→right, wrap down). ColumnMajor is declared in the API for
        // completeness but is NOT yet validated against a real column-flow .gui
        // (Codex (a)); until a real case exists it deliberately falls back to the
        // proven row-major placement rather than ship guessed column math.
        let (col, grow) = (i % columns, i / columns);
        // the cell's top-left WITHIN the viewport, pre-scroll.
        let cell_x = viewport.x + col as f32 * spec.cell_w;
        let cell_y = viewport.y + grow as f32 * spec.cell_h - scroll_y;
        let cell = Rect {
            x: cell_x,
            y: cell_y,
            w: spec.cell_w,
            h: spec.cell_h,
        };
        // clone the template and lay it out into the cell rect; propagate the
        // viewport clip so off-viewport rows are rejected by draw + hit-test.
        let mut clone = template.clone();
        // the template's own position is the cell origin (0,0 within the cell).
        clone.pos = (0.0, 0.0);
        layout(&mut clone, cell, Some(viewport), content);
        rows.push(clone);
    }

    let scrollbar = if overflowing {
        let track = Rect {
            x: viewport.x + viewport.w - spec.scrollbar_w,
            y: viewport.y,
            w: spec.scrollbar_w,
            h: viewport.h,
        };
        // thumb height ∝ visible fraction; thumb position ∝ scroll fraction. The
        // 16px MINIMUM must never exceed the track height (a very short viewport vs
        // tall content), or the thumb escaped the track at max scroll (Codex). Clamp
        // thumb_h to the track, and clamp thumb_y so the thumb stays fully inside.
        let visible_frac = (viewport.h / content_height).clamp(0.05, 1.0);
        let thumb_h = (track.h * visible_frac).max(16.0).min(track.h);
        let scroll_frac = if max_scroll > 0.0 {
            scroll_y / max_scroll
        } else {
            0.0
        };
        // (track.h - thumb_h) is now guaranteed >= 0, so the thumb cannot run past
        // either end of the track.
        let thumb_y = track.y + (track.h - thumb_h).max(0.0) * scroll_frac.clamp(0.0, 1.0);
        let thumb = Rect {
            x: track.x,
            y: thumb_y,
            w: spec.scrollbar_w,
            h: thumb_h,
        };
        Some(ScrollBar { track, thumb })
    } else {
        None
    };

    GridInstances {
        viewport,
        rows,
        content_height,
        overflowing,
        scrollbar,
    }
}

/// Find a named child within ONE laid-out row clone (depth-first). Used by the
/// bind layer to set a row's `level` text or `building_picture` icon frame.
pub fn row_child_mut<'a>(row: &'a mut Node, name: &str) -> Option<&'a mut Node> {
    if row.name == name {
        return Some(row);
    }
    for c in row.children.iter_mut() {
        if let Some(found) = row_child_mut(c, name) {
            return Some(found);
        }
    }
    None
}

/// Set the icon FRAME of a named child icon/button in a row (the building/resource
/// sprite frame). Returns true if found and updated.
pub fn set_row_icon_frame(row: &mut Node, name: &str, frame: u32) -> bool {
    if let Some(n) = row_child_mut(row, name) {
        match &mut n.kind {
            ElementKind::Icon { frame: f, .. } => {
                *f = frame;
                return true;
            }
            ElementKind::Button { .. } => {
                // a buttonType with a strip sprite — promote it to a framed icon so
                // the draw pass renders the chosen frame (building_picture is a
                // buttonType in the template but is really a framed sprite).
                if let ElementKind::Button {
                    sprite: Some(s), ..
                } = &n.kind
                {
                    let sprite = s.clone();
                    n.kind = ElementKind::Icon { sprite, frame };
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Anchor, ElementKind, Rect};

    struct SizedContent;
    impl ContentSize for SizedContent {
        fn content_size(&self, node: &Node) -> Option<(f32, f32)> {
            // give text/icon a fixed size so children lay out non-zero.
            match &node.kind {
                ElementKind::Text(_) => Some((40.0, 18.0)),
                ElementKind::Icon { .. } | ElementKind::Button { .. } => Some((32.0, 32.0)),
                _ => None,
            }
        }
    }

    /// Build a 140x70 entry template with a `level` text child (like the real
    /// `state_building_entry`).
    fn template() -> Node {
        let mut level = Node {
            name: "level".into(),
            raw_kind: "instantTextBoxType".into(),
            tooltip: None,
            kind: ElementKind::Text(crate::scene::TextSpec {
                text: "0".into(),
                raw: "0".into(),
                font: "f".into(),
                align: crate::scene::TextAlign::Center,
                max_width: Some(60.0),
                max_height: Some(20.0),
            }),
            pos: (5.0, 54.0),
            size_w: None,
            size_h: None,
            orientation: Anchor::UpperLeft,
            origo: Anchor::UpperLeft,
            clipping: false,
            alwaystransparent: false,
            rect: Rect::default(),
            clip: None,
            children: vec![],
        };
        level.rect = Rect::default();
        Node {
            name: "state_building_entry".into(),
            raw_kind: "containerWindowType".into(),
            tooltip: None,
            kind: ElementKind::Container {
                background: Some("GFX_entry_bg".into()),
            },
            pos: (0.0, 0.0),
            size_w: Some(crate::model::Scalar::Int(140)),
            size_h: Some(crate::model::Scalar::Int(70)),
            orientation: Anchor::UpperLeft,
            origo: Anchor::UpperLeft,
            clipping: false,
            alwaystransparent: false,
            rect: Rect::default(),
            clip: None,
            children: vec![level],
        }
    }

    fn gridbox_node(w: f32, h: f32) -> Node {
        let mut g = template();
        g.name = "state_building_entries".into();
        g.kind = ElementKind::Container { background: None };
        g.size_w = Some(crate::model::Scalar::Int(w as i64));
        g.size_h = Some(crate::model::Scalar::Int(h as i64));
        g.rect = Rect {
            x: 153.0,
            y: 45.0,
            w,
            h,
        };
        g.children.clear();
        g
    }

    #[test]
    fn one_instance_per_element_on_the_grid() {
        let tmpl = template();
        let gb = gridbox_node(480.0, 105.0); // 480/140 = 3 columns
        let spec = GridSpec::from_gridbox(&gb, &tmpl, 0.0);
        assert_eq!(spec.columns, 3);
        let vp = gb.rect;
        let inst = instantiate(&tmpl, 7, vp, &spec, 0.0, &SizedContent);
        // exactly one clone per element.
        assert_eq!(inst.rows.len(), 7);
        // row 0 at column 0; row 3 wraps to grid-row 1 column 0 (below).
        assert_eq!(inst.rows[0].rect.x, vp.x);
        assert_eq!(inst.rows[0].rect.y, vp.y);
        assert_eq!(inst.rows[1].rect.x, vp.x + 140.0);
        assert_eq!(inst.rows[3].rect.x, vp.x); // wrapped
        assert_eq!(inst.rows[3].rect.y, vp.y + 70.0); // second grid-row
                                                      // 7 elements / 3 cols = 3 grid-rows * 70 = 210 content height > 105 vp.
        assert_eq!(inst.content_height, 210.0);
        assert!(inst.overflowing);
        assert!(inst.scrollbar.is_some());
    }

    #[test]
    fn scroll_offset_shifts_every_row_and_clip_rejects_offscreen() {
        let tmpl = template();
        let gb = gridbox_node(480.0, 105.0);
        let spec = GridSpec::from_gridbox(&gb, &tmpl, 0.0);
        let vp = gb.rect;
        // scroll down by one grid-row (70px).
        let inst = instantiate(&tmpl, 7, vp, &spec, 70.0, &SizedContent);
        // row 0 is now 70px above its un-scrolled position → above the viewport.
        assert_eq!(inst.rows[0].rect.y, vp.y - 70.0);
        // its clip is the viewport; a hit-test inside the row's (now offscreen)
        // rect must be rejected because the clip excludes y < vp.y.
        let r0 = &inst.rows[0];
        assert_eq!(r0.clip, Some(vp));
        // the child 'level' clip is also the viewport (propagated).
        let child = &r0.children[0];
        assert_eq!(child.clip, Some(vp));
        // row 3 (grid-row 1) was at vp.y+70; after -70 scroll it's at vp.y (top).
        assert_eq!(inst.rows[3].rect.y, vp.y);
    }

    #[test]
    fn scrollbar_thumb_reflects_ratio_and_position() {
        let tmpl = template();
        let gb = gridbox_node(480.0, 105.0);
        let spec = GridSpec::from_gridbox(&gb, &tmpl, 12.0);
        // with the 12px gutter, usable = 468 → still 3 cols (468/140=3).
        assert_eq!(spec.columns, 3);
        let vp = gb.rect;
        let top = instantiate(&tmpl, 9, vp, &spec, 0.0, &SizedContent);
        // 9/3 = 3 grid-rows * 70 = 210 content; viewport 105 → max scroll 105.
        let sb = top.scrollbar.unwrap();
        // thumb at top when scroll=0.
        assert_eq!(sb.thumb.y, vp.y);
        // thumb height ∝ visible fraction 105/210 = 0.5 → 52.5px.
        assert!((sb.thumb.h - 52.5).abs() < 0.01);
        // scroll to the bottom (105px) → thumb at the bottom of the track.
        let bot = instantiate(&tmpl, 9, vp, &spec, 105.0, &SizedContent);
        let sb2 = bot.scrollbar.unwrap();
        assert!((sb2.thumb.y - (vp.y + vp.h - sb2.thumb.h)).abs() < 0.01);
        // the track sits in the right gutter.
        assert!((sb.track.x - (vp.x + vp.w - 12.0)).abs() < 0.01);
    }

    #[test]
    fn zero_columns_does_not_panic() {
        // BUG (Codex): `GridSpec` is public + constructible with `columns: 0`; the
        // instantiate loop did `i % spec.columns` -> divide-by-zero PANIC for any
        // count > 0. A degenerate spec must be sanitized to >=1 column, never crash.
        let tmpl = template();
        let vp = Rect {
            x: 0.0,
            y: 0.0,
            w: 480.0,
            h: 105.0,
        };
        let bad = GridSpec {
            cell_w: 140.0,
            cell_h: 70.0,
            columns: 0, // hostile / degenerate
            flow: GridFlow::RowMajor,
            scrollbar_w: 0.0,
        };
        let inst = instantiate(&tmpl, 5, vp, &bad, 0.0, &SizedContent);
        assert_eq!(
            inst.rows.len(),
            5,
            "must still produce one row per element, no panic"
        );
    }

    #[test]
    fn scrollbar_thumb_never_escapes_track() {
        // BUG (Codex): a tiny viewport vs the 16px thumb minimum let the thumb run
        // past the track at max scroll (thumb_y < track.y, or thumb bottom > track
        // bottom). The thumb must stay fully inside the track for ALL scroll values.
        let tmpl = template();
        // A very SHORT viewport (height 10) vs a much taller content (2 cells = 140)
        // so the naive 16px-minimum thumb would be TALLER than the 10px track — the
        // exact case where the thumb escaped (Codex). The fix clamps thumb_h to the
        // track height and pins thumb_y inside the track.
        let mut gb = gridbox_node(140.0, 10.0); // 1 column, viewport height 10
        gb.rect.h = 10.0;
        let spec = GridSpec::from_gridbox(&gb, &tmpl, 4.0);
        let vp = gb.rect;
        let max = ScrollBar::max_scroll(vp.h, 2.0 * 70.0); // content 140, vp 10
        for sy in [0.0, max * 0.5, max] {
            let inst = instantiate(&tmpl, 2, vp, &spec, sy, &SizedContent);
            let sb = inst.scrollbar.expect("overflowing -> scrollbar");
            assert!(
                sb.thumb.h <= sb.track.h + 0.01,
                "thumb taller than track: {sb:?}"
            );
            assert!(
                sb.thumb.y >= sb.track.y - 0.01,
                "thumb above track top: {sb:?}"
            );
            assert!(
                sb.thumb.y + sb.thumb.h <= sb.track.y + sb.track.h + 0.01,
                "thumb bottom past track bottom: {sb:?}"
            );
        }
    }

    #[test]
    fn no_overflow_when_content_fits() {
        let tmpl = template();
        let gb = gridbox_node(480.0, 105.0);
        let spec = GridSpec::from_gridbox(&gb, &tmpl, 0.0);
        let vp = gb.rect;
        // only 3 elements = 1 grid-row = 70px <= 105px viewport.
        let inst = instantiate(&tmpl, 3, vp, &spec, 0.0, &SizedContent);
        assert!(!inst.overflowing);
        assert!(inst.scrollbar.is_none());
    }
}
