//! N6 — the NAVAL-COMBAT widget: a sea-zone combat bubble (on the map) + a stacked
//! combat PANEL. Consumers: naval_combat, naval_combat_results.
//!
//! Codex N6 hardening: the panel is a GENERIC [`RosterPanel`] ({ sides, each with rows
//! of labelled [`Atom`] cells }) — it knows NOTHING naval-specific, so the same panel
//! renders a land roster, a naval roster, or an air roster; the consumer supplies the
//! rows. Every cell is an [`Atom`] (a blocked detail is `—`, never a fabricated 0). An
//! EMPTY roster / missing side label / zero participants draws an explicit empty/unknown
//! state, never fake zeros. The sea-zone bubble REUSES N8's centroid + map camera
//! (so combat bubbles and map icons share one map-layer primitive) — and this widget
//! does NOT depend on `combat_panel` (the additive build guard).
//!
//! ## Usage
//! ```ignore
//! use widgets::naval_combat::*;
//! use widgets::common::Atom;
//! let panel = RosterPanel::new(Atom::exact("Battle of the Denmark Strait"))
//!     .side(SideColumn::new(Atom::exact("ATTACKER"), [0.86,0.36,0.30,1.0])
//!         .row("Capital ships", Atom::exact("3"))
//!         .row("Screens",       Atom::unavailable()))
//!     .side(SideColumn::new(Atom::unavailable(), [0.34,0.58,0.86,1.0]) // unknown side
//!         .row("Capital ships", Atom::exact("2")));
//! // let laid = panel.layout(area);
//! // draw_panel(renderer, &laid, &assets);
//! ```

use render::Renderer;

use crate::common::{Atom, DrawAssets, AssetRef, DIM_TINT, TEXT_TINT, UNAVAIL_TINT};
use crate::map_overlay::{IconAnchor, MapCamera, MapIcon, ProvinceCentroids};
use crate::plan::{Cmd, Plan};

/// One labelled cell in a side column: an identity + an [`Atom`] value.
#[derive(Debug, Clone, PartialEq)]
pub struct RosterRow {
    pub name: String,
    pub value: Atom,
}

/// One side's column: a header [`Atom`] (the side's tag/name — unavailable when the
/// side is unknown) + an accent colour + its rows. An empty `rows` is a valid
/// "unknown roster" state (drawn with a badge, not fabricated).
#[derive(Debug, Clone, PartialEq)]
pub struct SideColumn {
    pub header: Atom,
    pub accent: [f32; 4],
    pub rows: Vec<RosterRow>,
}

impl SideColumn {
    pub fn new(header: Atom, accent: [f32; 4]) -> SideColumn {
        SideColumn { header, accent, rows: Vec::new() }
    }
    /// Add a labelled row (builder-chained).
    pub fn row(mut self, name: impl Into<String>, value: Atom) -> SideColumn {
        self.rows.push(RosterRow { name: name.into(), value });
        self
    }
}

/// The generic roster panel: a title + N side columns. Naval/land/air agnostic.
#[derive(Debug, Clone, PartialEq)]
pub struct RosterPanel {
    pub title: Atom,
    pub sides: Vec<SideColumn>,
}

impl RosterPanel {
    pub fn new(title: Atom) -> RosterPanel {
        RosterPanel { title, sides: Vec::new() }
    }
    pub fn side(mut self, s: SideColumn) -> RosterPanel {
        self.sides.push(s);
        self
    }

    /// Lay the panel out within `area`: a header strip + the side columns split evenly.
    pub fn layout(&self, area: gui_runtime::Rect) -> LaidPanel {
        let header_h = 28.0;
        let pad = 10.0;
        let n = self.sides.len().max(1);
        let body = gui_runtime::Rect { x: area.x + pad, y: area.y + header_h + pad, w: (area.w - 2.0 * pad).max(0.0), h: (area.h - header_h - 2.0 * pad).max(0.0) };
        let gap = 8.0;
        let col_w = ((body.w - (n as f32 - 1.0) * gap) / n as f32).max(0.0);
        let mut cols = Vec::with_capacity(self.sides.len());
        for i in 0..self.sides.len() {
            cols.push(gui_runtime::Rect { x: body.x + i as f32 * (col_w + gap), y: body.y, w: col_w, h: body.h });
        }
        LaidPanel { area, header: gui_runtime::Rect { x: area.x, y: area.y, w: area.w, h: header_h }, title: self.title.clone(), sides: self.sides.clone(), columns: cols }
    }

    /// Per-side (exact, derived, unavailable) cell counts for the accounting test.
    pub fn cell_counts(&self) -> (usize, usize, usize) {
        let (mut e, mut d, mut u) = (0, 0, 0);
        for s in &self.sides {
            for r in &s.rows {
                match r.value.fidelity() {
                    crate::common::Fidelity::Exact => e += 1,
                    crate::common::Fidelity::Derived => d += 1,
                    crate::common::Fidelity::Unavailable => u += 1,
                }
            }
        }
        (e, d, u)
    }
}

/// A laid-out panel: positions for the header + each side column.
#[derive(Debug, Clone, PartialEq)]
pub struct LaidPanel {
    pub area: gui_runtime::Rect,
    pub header: gui_runtime::Rect,
    pub title: Atom,
    pub sides: Vec<SideColumn>,
    pub columns: Vec<gui_runtime::Rect>,
}

const PANEL_BG: [f32; 4] = [0.10, 0.11, 0.14, 0.97];
const HEADER_BG: [f32; 4] = [0.16, 0.18, 0.23, 1.0];
const COL_BG: [f32; 4] = [0.08, 0.085, 0.11, 1.0];

/// Build the render plan for a laid-out roster panel. An empty side (no rows) draws an
/// explicit "no roster — unavailable" badge, never fabricated zero rows.
pub fn plan_panel(panel: &LaidPanel) -> Plan {
    let mut p = Plan::new();
    let a = panel.area;
    let clip = Some([a.x, a.y, a.w, a.h]);
    p.push(Cmd::Rect { x: a.x, y: a.y, w: a.w, h: a.h, tint: PANEL_BG, clip });
    p.push(Cmd::Rect { x: panel.header.x, y: panel.header.y, w: panel.header.w, h: panel.header.h, tint: HEADER_BG, clip });
    p.push(Cmd::Text { x: panel.header.x + 10.0, y: panel.header.y + 7.0, text: panel.title.as_str().into(), tint: panel.title.tint(TEXT_TINT), clip });

    for (i, col) in panel.columns.iter().enumerate() {
        let side = match panel.sides.get(i) {
            Some(s) => s,
            None => continue,
        };
        let cclip = Some([col.x, col.y, col.w, col.h]);
        p.push(Cmd::Rect { x: col.x, y: col.y, w: col.w, h: col.h, tint: COL_BG, clip });
        // accent header bar + side name (a `—` for an unknown side).
        p.push(Cmd::Rect { x: col.x, y: col.y, w: col.w, h: 20.0, tint: side.accent, clip: cclip });
        p.push(Cmd::Text { x: col.x + 6.0, y: col.y + 3.0, text: side.header.as_str().into(), tint: side.header.tint([0.06, 0.06, 0.08, 1.0]), clip: cclip });

        let mut ry = col.y + 28.0;
        if side.rows.is_empty() {
            // an explicit unknown-roster badge (never fabricated zero rows).
            p.push(Cmd::Text { x: col.x + 6.0, y: ry, text: format!("{} no roster", crate::common::NOT_CAPTURED), tint: UNAVAIL_TINT, clip: cclip });
        } else {
            for r in &side.rows {
                p.push(Cmd::Text { x: col.x + 6.0, y: ry, text: r.name.clone(), tint: DIM_TINT, clip: cclip });
                p.push(Cmd::Text { x: col.x + col.w * 0.55, y: ry, text: r.value.as_str().into(), tint: r.value.tint(TEXT_TINT), clip: cclip });
                ry += 18.0;
            }
        }
    }
    p
}

/// Draw the roster panel (plan + execute). READ-ONLY.
pub fn draw_panel(r: &mut Renderer, panel: &LaidPanel, assets: &DrawAssets) {
    plan_panel(panel).execute(r, assets, |_, _| false);
}

// ===========================================================================
// The SEA-ZONE COMBAT BUBBLE — reuses N8's centroid + map camera (Codex N6: the
// bubble is a map-layer primitive shared with N8, not a duplicate of combat_panel).
// ===========================================================================

/// A naval combat to mark on the map: an id + the sea-zone province + a size hint (the
/// total engaged-ship count → bubble radius) + the controlling tag (for tinting; an
/// unknown controller uses the neutral contested style).
#[derive(Debug, Clone, PartialEq)]
pub struct NavalCombat {
    pub id: String,
    pub sea_province: u32,
    /// Total engaged refs (both sides) → bubble size. A measured 0 is honest (tiny dot).
    pub engaged: u32,
    /// The side-controlling tag for tinting (None → neutral contested, never fabricated).
    pub controller_tag: Option<String>,
}

const CONTESTED_C: [f32; 4] = [0.80, 0.70, 0.25, 1.0];
const NAVAL_C: [f32; 4] = [0.34, 0.58, 0.86, 1.0];

/// Lay out naval combat bubbles as N8 [`MapIcon`]s at their sea-zone centroids. A combat
/// whose sea province is absent from the raster is SKIPPED by N8's layout (no fabricated
/// position). The bubble is a coloured marker (`AssetRef::None` + a tint), sized by the
/// engaged count; tinted by the controller (or contested).
pub fn naval_bubble_icons(combats: &[NavalCombat]) -> Vec<MapIcon> {
    combats
        .iter()
        .map(|c| {
            let radius = (8.0 + (c.engaged as f32).sqrt() * 3.0).clamp(8.0, 28.0);
            let tint = if c.controller_tag.is_some() { NAVAL_C } else { CONTESTED_C };
            MapIcon {
                id: c.id.clone(),
                anchor: IconAnchor::Province(c.sea_province),
                sprite: AssetRef::None, // a coloured disc marker
                tint,
                label: None,
                priority: 0,
                size: radius * 2.0,
            }
        })
        .collect()
}

/// Convenience: lay out + return the placed naval bubbles via N8's machinery, so a
/// caller gets the same skip-diagnostics + pick behaviour as the map icon layer.
pub fn layout_naval_bubbles<'a>(
    icons: &'a [MapIcon],
    centroids: &ProvinceCentroids,
    cam: &MapCamera,
) -> crate::map_overlay::IconLayout<'a> {
    crate::map_overlay::layout_icons(icons, centroids, cam)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel() -> RosterPanel {
        RosterPanel::new(Atom::exact("Denmark Strait"))
            .side(
                SideColumn::new(Atom::exact("ATTACKER"), [0.86, 0.36, 0.30, 1.0])
                    .row("Capital ships", Atom::exact("3"))
                    .row("Screens", Atom::unavailable()),
            )
            .side(SideColumn::new(Atom::unavailable(), [0.34, 0.58, 0.86, 1.0]))
    }

    #[test]
    fn cell_counts_track_fidelity() {
        // attacker: 1 exact + 1 unavailable; defender: no rows.
        assert_eq!(panel().cell_counts(), (1, 0, 1));
    }

    #[test]
    fn empty_side_draws_no_roster_badge_not_fake_zeros() {
        let laid = panel().layout(gui_runtime::Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 });
        let p = plan_panel(&laid);
        // the empty defender side + the unavailable "Screens" cell + the unknown header
        // -> multiple "—" glyphs; no fabricated "0".
        assert!(p.unavailable_text_count() >= 2);
        assert!(p.cmds.iter().any(|c| matches!(c, Cmd::Text { text, .. } if text.contains("no roster"))));
        // the measured "3" survives (a real value).
        assert!(p.cmds.iter().any(|c| matches!(c, Cmd::Text { text, .. } if text == "3")));
    }

    #[test]
    fn empty_panel_no_panic() {
        let laid = RosterPanel::new(Atom::unavailable()).layout(gui_runtime::Rect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 });
        let p = plan_panel(&laid);
        assert!(!p.is_empty()); // at least the panel bg + header + title glyph
    }

    #[test]
    fn naval_bubble_skips_absent_sea_zone() {
        let ids = vec![5, 5, 0, 0];
        let centroids = ProvinceCentroids::from_raster_ids(&ids, 2);
        let cam = MapCamera::new((0.0, 0.0), 1.0, (0.0, 0.0));
        let combats = vec![
            NavalCombat { id: "a".into(), sea_province: 5, engaged: 9, controller_tag: Some("ENG".into()) },
            NavalCombat { id: "b".into(), sea_province: 404, engaged: 4, controller_tag: None },
        ];
        let icons = naval_bubble_icons(&combats);
        let laid = layout_naval_bubbles(&icons, &centroids, &cam);
        assert_eq!(laid.placed.len(), 1); // sea-zone 5 placed; 404 skipped
        assert_eq!(laid.skipped, vec![404]);
        assert_eq!(laid.pick(0.5, 0.5).as_deref(), Some("a"));
    }

    #[test]
    fn bubble_tint_neutral_when_controller_unknown() {
        let combats = vec![NavalCombat { id: "x".into(), sea_province: 1, engaged: 0, controller_tag: None }];
        let icons = naval_bubble_icons(&combats);
        assert_eq!(icons[0].tint, CONTESTED_C); // never a fabricated owner colour
    }
}
