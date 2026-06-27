//! N8 — the MAP-OVERLAY ICON LAYER: icons placed at province centroids (or an explicit
//! world / screen anchor), projected through the live map camera. Consumers: mapicons,
//! divisions-on-map, VP counters.
//!
//! Codex N8 hardening:
//!   * an icon's anchor is an [`IconAnchor`] — `Province(id)` (centroid), `World{x,y}`,
//!     or `Screen{x,y}` (a fixed HUD label) — so non-province-anchored icons work too;
//!   * an icon's sprite is an [`AssetRef`] (no-icon-by-design vs missing-asset vs
//!     blocked stay distinct; a text-only marker uses `AssetRef::None` + a label);
//!   * a province ABSENT from the raster yields NO placed icon, BUT the layout exposes
//!     the skipped ids as a diagnostic ([`IconLayout::skipped`]) — never a fabricated
//!     position;
//!   * labels are [`Atom`]s.
//!
//! The province-centroid math + the map camera are OWNED here (copied SHAPE from
//! `combat_panel`, NOT imported — the additive build guard).
//!
//! ## Usage
//! ```ignore
//! use widgets::map_overlay::*;
//! use widgets::common::{Atom, AssetRef};
//! let centroids = ProvinceCentroids::from_raster_ids(&ids, width);
//! let cam = MapCamera::new((0.0,0.0), 2.0, (0.0,0.0));
//! let icons = vec![
//!     MapIcon::province(64, AssetRef::Named("GFX_vp".into())).label(Atom::exact("12")),
//!     MapIcon::world(100.0, 50.0, AssetRef::None).label(Atom::exact("HQ")),
//! ];
//! let laid = layout_icons(&icons, &centroids, &cam);
//! // draw_icons(renderer, &laid, &assets, resolve_sprite);
//! // if let Some(i) = laid.pick(mx, my) { ... }
//! ```

use std::collections::HashMap;

use render::Renderer;

use crate::common::{AssetRef, Atom, DrawAssets, TEXT_TINT};
use crate::plan::{Cmd, Plan, SpriteDraw};

/// Centroid (mean pixel) of each province id in a raster. Computed once per raster,
/// reused every frame. (Copied SHAPE from `combat_panel::ProvinceCentroids`, owned here.)
#[derive(Debug, Clone, Default)]
pub struct ProvinceCentroids {
    centroid: HashMap<u32, (f32, f32)>,
}

impl ProvinceCentroids {
    /// One linear pass over the id raster accumulating per-id pixel sums. `ids[i]` is the
    /// province id at pixel `(i % width, i / width)`; id 0 = no province (skipped).
    pub fn from_raster_ids(ids: &[u32], width: usize) -> ProvinceCentroids {
        let mut acc: HashMap<u32, (f64, f64, u64)> = HashMap::new();
        let w = width.max(1);
        for (i, &id) in ids.iter().enumerate() {
            if id == 0 {
                continue;
            }
            let x = (i % w) as f64;
            let y = (i / w) as f64;
            let e = acc.entry(id).or_insert((0.0, 0.0, 0));
            e.0 += x;
            e.1 += y;
            e.2 += 1;
        }
        let centroid = acc
            .into_iter()
            .map(|(id, (sx, sy, n))| {
                let n = n.max(1) as f64;
                (id, ((sx / n) as f32, (sy / n) as f32))
            })
            .collect();
        ProvinceCentroids { centroid }
    }
    /// Build directly from a precomputed id→(cx,cy) map (when the centroids are already
    /// available from the map artifacts).
    pub fn from_map(centroid: HashMap<u32, (f32, f32)>) -> ProvinceCentroids {
        ProvinceCentroids { centroid }
    }
    /// Map-pixel centroid of a province; `None` if it has no pixels (no icon placed).
    pub fn get(&self, province: u32) -> Option<(f32, f32)> {
        self.centroid.get(&province).copied()
    }
    pub fn len(&self) -> usize {
        self.centroid.len()
    }
    pub fn is_empty(&self) -> bool {
        self.centroid.is_empty()
    }
}

/// The map camera: a map-pixel → screen-pixel affine (pan + uniform zoom). Matches the
/// political map's transform so icons track the same frame. (Owned copy of the
/// `combat_panel::MapCamera` shape.)
#[derive(Debug, Clone, Copy)]
pub struct MapCamera {
    pub pan: (f32, f32),
    pub zoom: f32,
    pub offset: (f32, f32),
}

impl MapCamera {
    pub fn new(pan: (f32, f32), zoom: f32, offset: (f32, f32)) -> MapCamera {
        MapCamera { pan, zoom, offset }
    }
    /// Project a MAP pixel to a SCREEN pixel. A degenerate `zoom` from the public field
    /// (non-finite, zero, or negative) is coerced to 1.0 so the projection is always
    /// finite + non-collapsing/non-inverting (Codex: the frozen struct must make
    /// degenerate input safe, including a finite `0.0`/negative zoom). The layout step
    /// then also drops any non-finite result, so a poisoned camera yields no placed icon.
    pub fn to_screen(&self, mx: f32, my: f32) -> (f32, f32) {
        let z = if self.zoom.is_finite() && self.zoom > 0.0 {
            self.zoom
        } else {
            1.0
        };
        (
            (mx - self.pan.0) * z + self.offset.0,
            (my - self.pan.1) * z + self.offset.1,
        )
    }
}

/// Where an icon is anchored (Codex N8: not province-only).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IconAnchor {
    /// At a province's centroid (projected through the map camera).
    Province(u32),
    /// At an explicit MAP-pixel position (projected through the camera).
    World { x: f32, y: f32 },
    /// At an explicit SCREEN-pixel position (a fixed HUD label; camera-independent).
    Screen { x: f32, y: f32 },
}

/// One overlay icon: an anchor + a sprite [`AssetRef`] + a tint + an optional [`Atom`]
/// label + a priority (higher = drawn on top + wins a pick on overlap).
#[derive(Debug, Clone, PartialEq)]
pub struct MapIcon {
    pub id: String,
    pub anchor: IconAnchor,
    pub sprite: AssetRef,
    pub tint: [f32; 4],
    pub label: Option<Atom>,
    pub priority: i32,
    /// Icon size in screen px (centred on the anchor).
    pub size: f32,
}

impl MapIcon {
    pub fn province(province: u32, sprite: AssetRef) -> MapIcon {
        Self::at(IconAnchor::Province(province), sprite)
    }
    pub fn world(x: f32, y: f32, sprite: AssetRef) -> MapIcon {
        Self::at(IconAnchor::World { x, y }, sprite)
    }
    pub fn screen(x: f32, y: f32, sprite: AssetRef) -> MapIcon {
        Self::at(IconAnchor::Screen { x, y }, sprite)
    }
    fn at(anchor: IconAnchor, sprite: AssetRef) -> MapIcon {
        MapIcon {
            id: String::new(),
            anchor,
            sprite,
            tint: [1.0; 4],
            label: None,
            priority: 0,
            size: 18.0,
        }
    }
    pub fn id(mut self, id: impl Into<String>) -> MapIcon {
        self.id = id.into();
        self
    }
    pub fn label(mut self, label: Atom) -> MapIcon {
        self.label = Some(label);
        self
    }
    pub fn tint(mut self, tint: [f32; 4]) -> MapIcon {
        self.tint = tint;
        self
    }
    pub fn priority(mut self, p: i32) -> MapIcon {
        self.priority = p;
        self
    }
    pub fn size(mut self, s: f32) -> MapIcon {
        self.size = s;
        self
    }
}

/// A placed icon: its screen position + the source icon index.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedIcon {
    pub index: usize,
    pub sx: f32,
    pub sy: f32,
}

/// The result of laying out icons: the placed ones (sorted by priority ascending, so a
/// higher priority draws last/on-top) + the skipped diagnostics.
#[derive(Debug, Clone, Default)]
pub struct IconLayout<'a> {
    pub placed: Vec<PlacedIcon>,
    /// Province-anchored icons whose province had NO centroid (no fabricated position).
    pub skipped: Vec<u32>,
    icons: &'a [MapIcon],
}

impl<'a> IconLayout<'a> {
    /// Hit-test a screen point against the placed icons. Front-most (highest priority,
    /// then last) wins; returns the icon id (its `id`, or its index if unnamed). `None`
    /// off every icon.
    pub fn pick(&self, x: f32, y: f32) -> Option<String> {
        let mut best: Option<(f32, &PlacedIcon)> = None;
        for pl in &self.placed {
            let icon = &self.icons[pl.index];
            let half = (icon.size * 0.5).max(2.0);
            if (x - pl.sx).abs() <= half && (y - pl.sy).abs() <= half {
                let d2 = (x - pl.sx).powi(2) + (y - pl.sy).powi(2);
                // later in `placed` (higher priority) wins ties via <= replacement.
                if best.map(|(bd, _)| d2 <= bd).unwrap_or(true) {
                    best = Some((d2, pl));
                }
            }
        }
        best.map(|(_, pl)| {
            let icon = &self.icons[pl.index];
            if icon.id.is_empty() {
                pl.index.to_string()
            } else {
                icon.id.clone()
            }
        })
    }
}

/// Lay icons out: project each anchor to a screen pixel (a Province with no centroid is
/// SKIPPED + recorded; a non-finite World/Screen is skipped). Placed icons are sorted by
/// priority ascending (stable within a priority) so the draw/pick order is correct.
pub fn layout_icons<'a>(
    icons: &'a [MapIcon],
    centroids: &ProvinceCentroids,
    cam: &MapCamera,
) -> IconLayout<'a> {
    let mut placed = Vec::new();
    let mut skipped = Vec::new();
    for (i, icon) in icons.iter().enumerate() {
        let pos = match icon.anchor {
            IconAnchor::Province(pid) => match centroids.get(pid) {
                Some((cx, cy)) => Some(cam.to_screen(cx, cy)),
                None => {
                    skipped.push(pid);
                    None
                }
            },
            IconAnchor::World { x, y } => {
                if x.is_finite() && y.is_finite() {
                    Some(cam.to_screen(x, y))
                } else {
                    None
                }
            }
            IconAnchor::Screen { x, y } => {
                if x.is_finite() && y.is_finite() {
                    Some((x, y))
                } else {
                    None
                }
            }
        };
        if let Some((sx, sy)) = pos {
            if sx.is_finite() && sy.is_finite() {
                placed.push(PlacedIcon { index: i, sx, sy });
            }
        }
    }
    // stable sort by priority ascending (preserves source order within a priority).
    placed.sort_by_key(|pl| icons[pl.index].priority);
    IconLayout {
        placed,
        skipped,
        icons,
    }
}

/// Build the render plan for the laid-out icons. Each placed icon draws its sprite (a
/// `Named` resolves → placeholder on miss; an `Unavailable`/`None`-with-label draws a
/// marker disc) + its optional label. The plan is built FROM the layout's `icons`
/// borrow, so it carries the source icon data.
pub fn plan_icons(layout: &IconLayout, clip: Option<[f32; 4]>) -> Plan {
    let mut p = Plan::new();
    for pl in &layout.placed {
        let icon = &layout.icons[pl.index];
        let half = icon.size * 0.5;
        match &icon.sprite {
            AssetRef::Named(name) => {
                p.push(Cmd::Sprite {
                    x: pl.sx - half,
                    y: pl.sy - half,
                    w: icon.size,
                    h: icon.size,
                    name: Some(name.clone()),
                    tint: icon.tint,
                    clip,
                });
            }
            AssetRef::Unavailable => {
                // a visible no-data marker (a hollow disc), distinct from a real icon.
                p.push(Cmd::Disc {
                    cx: pl.sx,
                    cy: pl.sy,
                    r: half.max(3.0),
                    tint: crate::common::UNAVAIL_TINT,
                    clip,
                });
            }
            AssetRef::None => {
                // a no-sprite icon is a COLOURED MARKER (a combat bubble, a VP dot): always
                // draw the tinted disc — the tint carries the meaning, the label is optional
                // decoration. (Codex: drawing only when a label exists left naval bubbles
                // — which have no label — invisible.)
                p.push(Cmd::Disc {
                    cx: pl.sx,
                    cy: pl.sy,
                    r: half.max(3.0),
                    tint: icon.tint,
                    clip,
                });
            }
        }
        if let Some(lbl) = &icon.label {
            p.push(Cmd::Text {
                x: pl.sx + half + 2.0,
                y: pl.sy - 6.0,
                text: lbl.as_str().into(),
                tint: lbl.tint(TEXT_TINT),
                clip,
            });
        }
    }
    p
}

/// Draw the icon layer (plan + execute). `resolve` resolves an icon sprite (miss →
/// placeholder).
pub fn draw_icons(
    r: &mut Renderer,
    layout: &IconLayout,
    clip: Option<[f32; 4]>,
    assets: &DrawAssets,
    resolve: impl FnMut(&mut Renderer, SpriteDraw) -> bool,
) {
    plan_icons(layout, clip).execute(r, assets, resolve);
}

#[cfg(test)]
mod tests {
    use super::*;

    // a 4x2 raster: left half province 7, right half province 9.
    fn centroids() -> ProvinceCentroids {
        let ids = vec![7, 7, 9, 9, 7, 7, 9, 9];
        ProvinceCentroids::from_raster_ids(&ids, 4)
    }

    #[test]
    fn centroid_is_pixel_mean() {
        let c = centroids();
        assert_eq!(c.get(7), Some((0.5, 0.5)));
        assert_eq!(c.get(9), Some((2.5, 0.5)));
        assert_eq!(c.get(123), None);
    }

    #[test]
    fn province_absent_from_raster_is_skipped_not_fabricated() {
        let cam = MapCamera::new((0.0, 0.0), 1.0, (0.0, 0.0));
        let icons = vec![
            MapIcon::province(7, AssetRef::Named("GFX_a".into())),
            MapIcon::province(99999, AssetRef::Named("GFX_b".into())),
        ];
        let laid = layout_icons(&icons, &centroids(), &cam);
        assert_eq!(laid.placed.len(), 1); // province 7 placed; 99999 skipped
        assert_eq!(laid.skipped, vec![99999]);
    }

    #[test]
    fn world_and_screen_anchors_place_without_centroid() {
        let cam = MapCamera::new((0.0, 0.0), 2.0, (10.0, 5.0));
        let icons = vec![
            MapIcon::world(3.0, 4.0, AssetRef::None).label(Atom::exact("W")),
            MapIcon::screen(100.0, 50.0, AssetRef::None).label(Atom::exact("S")),
        ];
        let laid = layout_icons(&icons, &ProvinceCentroids::default(), &cam);
        assert_eq!(laid.placed.len(), 2);
        // world: (3,4)*2 + (10,5) = (16,13).
        assert_eq!((laid.placed[0].sx, laid.placed[0].sy), (16.0, 13.0));
        // screen anchor is camera-independent.
        assert_eq!((laid.placed[1].sx, laid.placed[1].sy), (100.0, 50.0));
    }

    #[test]
    fn priority_orders_draw_and_pick() {
        let cam = MapCamera::new((0.0, 0.0), 1.0, (0.0, 0.0));
        let icons = vec![
            MapIcon::screen(50.0, 50.0, AssetRef::None)
                .id("low")
                .label(Atom::exact("L"))
                .priority(0)
                .size(20.0),
            MapIcon::screen(50.0, 50.0, AssetRef::None)
                .id("high")
                .label(Atom::exact("H"))
                .priority(5)
                .size(20.0),
        ];
        let laid = layout_icons(&icons, &ProvinceCentroids::default(), &cam);
        // higher priority is last in `placed` (drawn on top) + wins the overlapping pick.
        assert_eq!(laid.placed.last().unwrap().index, 1);
        assert_eq!(laid.pick(50.0, 50.0).as_deref(), Some("high"));
    }

    #[test]
    fn empty_inputs_no_panic() {
        let cam = MapCamera::new((0.0, 0.0), 1.0, (0.0, 0.0));
        let laid = layout_icons(&[], &ProvinceCentroids::default(), &cam);
        assert!(laid.placed.is_empty());
        assert_eq!(laid.pick(0.0, 0.0), None);
        assert!(plan_icons(&laid, None).is_empty());
    }

    #[test]
    fn unavailable_sprite_draws_marker_not_nothing() {
        let cam = MapCamera::new((0.0, 0.0), 1.0, (0.0, 0.0));
        let icons = vec![MapIcon::screen(10.0, 10.0, AssetRef::Unavailable)];
        let laid = layout_icons(&icons, &ProvinceCentroids::default(), &cam);
        let p = plan_icons(&laid, None);
        assert!(p.cmds.iter().any(|c| matches!(c, Cmd::Disc { .. })));
    }

    #[test]
    fn degenerate_camera_zoom_is_safe() {
        // a zero / negative / NaN zoom must not collapse or invert the projection.
        for z in [0.0_f32, -2.0, f32::NAN, f32::INFINITY] {
            let cam = MapCamera::new((0.0, 0.0), z, (10.0, 5.0));
            let (sx, sy) = cam.to_screen(3.0, 4.0);
            assert!(sx.is_finite() && sy.is_finite());
            // with the coerced unit zoom: (3,4)*1 + (10,5) = (13,9) — not collapsed to the offset.
            assert_eq!((sx, sy), (13.0, 9.0));
        }
    }

    #[test]
    fn label_less_none_marker_still_draws_a_disc() {
        // a coloured marker (a combat bubble / VP dot) has no sprite AND no label — it
        // must STILL draw its tinted disc (Codex: was invisible before the fix).
        let cam = MapCamera::new((0.0, 0.0), 1.0, (0.0, 0.0));
        let icons = vec![MapIcon::screen(10.0, 10.0, AssetRef::None).tint([0.8, 0.7, 0.25, 1.0])];
        let laid = layout_icons(&icons, &ProvinceCentroids::default(), &cam);
        let p = plan_icons(&laid, None);
        assert_eq!(
            p.cmds
                .iter()
                .filter(|c| matches!(c, Cmd::Disc { .. }))
                .count(),
            1
        );
    }
}
