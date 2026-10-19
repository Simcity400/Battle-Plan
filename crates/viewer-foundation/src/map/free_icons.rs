//! Map icons standing in a free eye's world.
//!
//! The map's icons (counters, combat and raid icons, ports, bases, theatre pins and city labels)
//! are laid out in interface pixels around their anchors projected through the camera, as for the
//! game's camera. Each owner tags its quads with the icon's world anchor and the distance its kind
//! is removed at ([`IconAnchor`]); a free eye then shows every icon as standing at its anchor:
//!
//! - an anchor behind the eye, or as far as its kind's cutoff, is not drawn;
//! - the icon keeps its screen size near the eye and shrinks gently with distance
//!   ([`SHRINK_FROM`] to [`SHRINK_TO`] of the eye's object range, down to [`SMALLEST`]), then
//!   fades out over the last [`FADE_SPAN`] of its cutoff;
//! - the whole icon fades when the drawn terrain or water hides its anchor from the eye: one
//!   sight line per anchor ([`TerrainSight`]), never per-pixel clipping. A line clearing the
//!   surface shows the icon whole; one dipping below it fades the icon out over a band that widens
//!   with distance, so it fades rather than pops as the eye moves;
//! - nearer icons draw over farther ones; an anchor's icons keep their own order among
//!   themselves.
//!
//! Hits follow what is drawn: the pointer is taken back into a shown icon's own layout and the
//! nearest icon under it wins. Quads without an anchor (the combat window, tooltips) stay screen
//! interface, unscaled and above every icon.

use std::collections::HashMap;

use super::{
    camera::Camera,
    province_map::HeightField,
    sprites::{Batch, CHILD_SLOTS, Quad},
    terrain_sight::TerrainSight,
};

/// The map icons that number their layer keys together: a layer key names one icon only within
/// its family, as a content may give two families the same priority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IconFamily {
    /// Faction-theatre pins.
    Pins,
    /// Air-base, naval-HQ, facility and naval-base icons, with the ships in their ports.
    Bases,
    Raids,
    /// Capital and victory-point labels.
    Labels,
    /// Unit-stack columns: land, railway-gun and at-sea plates.
    Columns,
    /// Land and naval combat icons and naval result icons.
    Combats,
    /// Naval mission icons.
    Missions,
    /// Resistance and compliance state markers.
    Occupation,
}

/// Where a map icon stands, as its owner tags its quads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconAnchor {
    /// The world position the icon's window is placed at.
    pub world: [f32; 3],
    /// The content's distance cutoff for the icon's kind (for example
    /// `UNITS_ICONS_DISTANCE_CUTOFF`): the icon is gone by this distance from a free eye.
    pub cutoff: f32,
    pub family: IconFamily,
}

impl IconAnchor {
    fn key(&self) -> [u32; 5] {
        [
            self.world[0].to_bits(),
            self.world[1].to_bits(),
            self.world[2].to_bits(),
            self.cutoff.to_bits(),
            self.family as u32,
        ]
    }
}

/// How a free eye shows one icon's quads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Present {
    /// The anchor on screen, in interface pixels: the icon scales about it.
    pub pivot: [f32; 2],
    pub scale: f32,
    /// The whole icon's opacity. Below one the icon draws offscreen and blends in as one image.
    pub alpha: f32,
    /// Draw order among the frame's icons, farthest first; unique to the icon.
    pub rank: u32,
}

impl Present {
    /// `point` in the icon's own layout, before its scale about the pivot.
    pub fn unscale(&self, point: [f32; 2]) -> [f32; 2] {
        [0, 1].map(|axis| self.pivot[axis] + (point[axis] - self.pivot[axis]) / self.scale)
    }

    /// Whether the icon is shown enough to own the pointer.
    pub fn hittable(&self) -> bool {
        self.alpha >= HIT_ALPHA
    }
}

/// Icons are full size within this share of the eye's object range.
pub const SHRINK_FROM: f32 = 0.05;
/// Icons reach their smallest size at this share of the eye's object range.
pub const SHRINK_TO: f32 = 0.6;
/// The smallest share of its screen size a distant icon keeps.
pub const SMALLEST: f32 = 0.6;
/// The share of an icon's cutoff it fades out over.
pub const FADE_SPAN: f32 = 0.2;
/// The sight band at the eye, in world units: a line this far below the surface hides the icon.
pub const SIGHT_BAND: f32 = 0.25;
/// How much the sight band widens per world unit of distance.
pub const SIGHT_BAND_GROWTH: f32 = 0.002;
/// How far above its anchor (or the drawn surface under it, whichever is higher) the sight line
/// aims, and how much of the line's end is left out as the anchor's own ground.
pub const SIGHT_LIFT: f32 = 0.5;
pub const SIGHT_SKIP: f32 = 1.0;
/// Opacity below which an icon no longer owns the pointer.
pub const HIT_ALPHA: f32 = 0.5;
/// Opacity below which an icon is not drawn.
const DRAWN_ALPHA: f32 = 1.0 / 255.0;

/// What a free eye stands on: the drawn terrain and its sight pyramid.
#[derive(Clone, Copy)]
pub struct Ground<'a> {
    pub field: HeightField<'a>,
    pub sight: &'a TerrainSight,
}

/// The size and opacity distance alone gives an icon whose kind is cut off at `cutoff`, for an eye
/// whose objects stand within `range`.
pub fn distance_look(distance: f32, cutoff: f32, range: f32) -> (f32, f32) {
    let shrink = smoothstep(SHRINK_FROM * range, SHRINK_TO * range, distance);
    let scale = 1.0 - (1.0 - SMALLEST) * shrink;
    let end = cutoff.min(range);
    let alpha = 1.0 - smoothstep(end * (1.0 - FADE_SPAN), end, distance);
    (scale, alpha)
}

/// The opacity the sight line from `eye` to `anchor` gives an icon at `distance`.
pub fn sight_alpha(ground: &Ground<'_>, eye: [f32; 3], anchor: [f32; 3], distance: f32) -> f32 {
    let band = SIGHT_BAND + SIGHT_BAND_GROWTH * distance;
    let floor = ground.field.surface_height(anchor[0], anchor[2]);
    let target = [anchor[0], anchor[1].max(floor) + SIGHT_LIFT, anchor[2]];
    let clearance = ground
        .sight
        .clearance(&ground.field, eye, target, SIGHT_SKIP, band);
    smoothstep(-band, 0.0, clearance)
}

fn smoothstep(low: f32, high: f32, value: f32) -> f32 {
    if high <= low {
        return if value < low { 0.0 } else { 1.0 };
    }
    let t = ((value - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The shown icons of one frame, by family and the icon part of their quads' layer keys.
#[derive(Clone, Debug, Default)]
pub struct Looks {
    by_icon: HashMap<(IconFamily, u64), Present>,
}

impl Looks {
    /// How the icon of `family` whose quads carry `layer` is shown; `None` when it is not drawn.
    pub fn of(&self, family: IconFamily, layer: u64) -> Option<&Present> {
        self.by_icon.get(&(family, layer / u64::from(CHILD_SLOTS)))
    }

    /// `point` in the layout of the shown icon of `family` keyed `layer`, with the icon's draw
    /// rank; `None` when that icon is not drawn or too faded to own the pointer.
    pub fn hit(&self, family: IconFamily, layer: u64, point: [f32; 2]) -> Option<([f32; 2], u32)> {
        self.of(family, layer)
            .filter(|present| present.hittable())
            .map(|present| (present.unscale(point), present.rank))
    }

    pub fn is_empty(&self) -> bool {
        self.by_icon.is_empty()
    }

    /// Records that the icon of `family` whose quads carry `layer` shows as `present`.
    pub fn show(&mut self, family: IconFamily, layer: u64, present: Present) {
        self.by_icon
            .insert((family, layer / u64::from(CHILD_SLOTS)), present);
    }
}

/// Places `batch`'s anchored quads for the free eye of `camera` into `out` (emptied first), ranked
/// farthest first and each tagged with its icon's [`Present`], then the unanchored quads as they
/// are. Quads of icons that are not drawn are left out. Returns the shown icons for hit tests.
/// The game's camera has nothing to place: `out` receives `batch` unchanged.
pub fn present(
    camera: &Camera,
    viewport: [u32; 2],
    ui_scale: f32,
    ground: Option<&Ground<'_>>,
    batch: &Batch,
    out: &mut Batch,
) -> Looks {
    out.clear();
    if camera.free().is_none() {
        out.quads.extend_from_slice(&batch.quads);
        return Looks::default();
    }
    let eye = camera.eye();
    let range = camera.object_range();
    // The GUI viewport the owners project their anchors into.
    let extent = super::gui_resources::viewport(viewport, ui_scale);
    // Each distinct anchor once: where it shows and how, or `None` when it does not.
    let mut seen: HashMap<[u32; 5], Option<(Present, f32)>> = HashMap::new();
    let mut shown: Vec<[u32; 5]> = Vec::new();
    for quad in &batch.quads {
        let Some(anchor) = &quad.anchor else {
            continue;
        };
        let key = anchor.key();
        if seen.contains_key(&key) {
            continue;
        }
        let look = look(camera, viewport, extent, ground, eye, range, anchor);
        if look.is_some() {
            shown.push(key);
        }
        seen.insert(key, look);
    }
    // Farthest first; equal distances keep the order the anchors were first met.
    shown.sort_by(|a, b| {
        let distance = |key| seen[key].as_ref().map_or(0.0, |(_, distance)| *distance);
        distance(b).total_cmp(&distance(a))
    });
    for (rank, key) in shown.iter().enumerate() {
        if let Some(Some((present, _))) = seen.get_mut(key) {
            present.rank = rank as u32;
        }
    }
    let mut looks = Looks::default();
    let mut unanchored: Vec<&Quad> = Vec::new();
    for quad in &batch.quads {
        let Some(anchor) = &quad.anchor else {
            unanchored.push(quad);
            continue;
        };
        let Some(Some((present, _))) = seen.get(&anchor.key()) else {
            continue;
        };
        looks.show(anchor.family, quad.layer, *present);
        out.quads.push(Quad {
            present: Some(*present),
            ..quad.clone()
        });
    }
    out.quads.extend(unanchored.into_iter().cloned());
    looks
}

/// How the free eye shows the icon at `anchor`, and its distance; `None` when it is not drawn.
fn look(
    camera: &Camera,
    viewport: [u32; 2],
    extent: [i32; 2],
    ground: Option<&Ground<'_>>,
    eye: [f32; 3],
    range: f32,
    anchor: &IconAnchor,
) -> Option<(Present, f32)> {
    let distance = camera.icon_distance(anchor.world);
    // A distance that is not a number shows nothing either.
    if distance.is_nan() || distance >= anchor.cutoff.min(range) {
        return None;
    }
    let pivot = camera
        .project_map_icon(anchor.world, viewport, extent)?
        .position;
    let (scale, mut alpha) = distance_look(distance, anchor.cutoff, range);
    if let Some(ground) = ground {
        alpha *= sight_alpha(ground, eye, anchor.world, distance);
    }
    (alpha >= DRAWN_ALPHA).then_some((
        Present {
            pivot,
            scale,
            alpha,
            rank: 0,
        },
        distance,
    ))
}

#[cfg(test)]
mod tests;
