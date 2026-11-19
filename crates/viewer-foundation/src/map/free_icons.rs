//! Map icons standing in a free eye's world.
//!
//! Owners tag quads with world anchors and distance cutoffs for free-camera placement:
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
    anchored::Head,
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
    /// The Operatives mode's intel containers.
    Intel,
    /// The resources mode's state resource icons.
    Resources,
    /// Strait and canal (adjacency-rule) icons.
    AdjacencyRules,
}

/// Where the recording's own rows put a mover at one hour: the place a publication's layout read
/// for it, never a place computed from where it might go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    /// The replay hour of the rows this place was read from. An integer: a replay hour is about
    /// sixty million, where neighbouring `f32` values are four hours apart, so an hour is never
    /// held or compared as one ([`Head`]).
    pub hour: i32,
    pub at: [f32; 3],
}

/// Where a mover stood at each hour the recording moved it, oldest first: each place is the one
/// the recording's own rows gave at that hour. Nothing is extrapolated from a route or an estimate
/// of the hours to come (a route and its recorded estimate, measured against the next publication
/// at the top speed, ended 14.87 world units — 227 pixels — apart, so a mover placed along them
/// slid out and was snapped back by every publication).
///
/// It holds a place for every hour the recording wrote the mover's province at over the hours the
/// frames of its publication can stand at, and no more: the count is the recording's own motion
/// and never a cap of ours.
///
/// The display frame evaluates it at that frame's own head, between the two places that bracket
/// it ([`crate::map::anchored`]), and offsets the windows standing on the anchor. So a counter is
/// placed at the frame's hour on every display frame, travels exactly as far as the hours carried
/// it, and nothing is laid out, painted or sorted for its motion.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    /// The places, oldest first, each unwrapped against the one after it. Shared: a layout reads a
    /// mover's places once, and the batch, the groups and the history then all stand on those.
    places: std::sync::Arc<[Place]>,
    /// The map's width, which an interpolated place is folded onto once; zero for no wrap. The
    /// places themselves are unwrapped against each other, so a mover crossing the map's seam
    /// travels the two units across it and not half the world ([`Track::new`]).
    pub width: f32,
}

impl Track {
    /// A track over `places`, oldest first, each unwrapped against the one after it so the straight
    /// line between two neighbours is the short way round the map; the newest stands as given.
    pub fn new(places: &[Place], width: f32) -> Self {
        let mut held = places.to_vec();
        for at in (1..held.len()).rev() {
            held[at - 1].at[0] = unwrap_x(width, held[at].at[0], held[at - 1].at[0]);
        }
        Self { places: std::sync::Arc::from(held), width }
    }

    /// The places it holds, oldest first.
    pub fn places(&self) -> &[Place] {
        &self.places
    }

    fn held(&self) -> &[Place] {
        &self.places
    }

    /// Where the mover stands at `head`: between the two places that bracket it, held at the
    /// oldest or the newest outside them, and folded onto the map once. The hours are differenced
    /// as integers before anything is cast, so a fractional head is not lost in an absolute replay
    /// hour.
    pub fn at(&self, head: Head) -> [f32; 3] {
        let held = self.held();
        let (Some(first), Some(last)) = (held.first(), held.last()) else {
            return [0.0; 3];
        };
        let over = |place: &Place| (head.hour - place.hour) as f32 + head.fraction;
        let mut at = if over(first) <= 0.0 {
            first.at
        } else if over(last) >= 0.0 {
            last.at
        } else {
            let to = segment(held, head);
            let (from, to) = (&held[to - 1], &held[to]);
            let hours = (to.hour - from.hour) as f32;
            let into = if hours > 0.0 { (over(from) / hours).clamp(0.0, 1.0) } else { 1.0 };
            [0, 1, 2].map(|axis| from.at[axis] + (to.at[axis] - from.at[axis]) * into)
        };
        if self.width > 0.0 {
            at[0] = at[0].rem_euclid(self.width);
        }
        at
    }

    /// Whether the places are different places: a mover the recording moved between them.
    pub fn moves(&self) -> bool {
        let held = self.held();
        held.len() > 1 && held.iter().any(|place| place.at != held[0].at)
    }

    /// Where `head` stands against the hours this track holds places for: `-1` before the oldest,
    /// `0` between them, `1` past the newest. A frame outside them is a frame the publications
    /// gave nothing to move its movers on, so it holds them at an end.
    pub fn outside(&self, head: Head) -> i8 {
        let held = self.held();
        let over = |place: &Place| (head.hour - place.hour) as f32 + head.fraction;
        match (held.first(), held.last()) {
            (Some(first), _) if over(first) < 0.0 => -1,
            (_, Some(last)) if over(last) > 0.0 => 1,
            _ => 0,
        }
    }

    /// Whether the two places `head` stands between are one point, or it stands outside them all:
    /// the recording left the mover where it was over those hours, so a frame that leaves it
    /// standing is right to.
    pub fn held_still(&self, head: Head) -> bool {
        let held = self.held();
        let (Some(first), Some(last)) = (held.first(), held.last()) else {
            return true;
        };
        let over = |place: &Place| (head.hour - place.hour) as f32 + head.fraction;
        if over(first) <= 0.0 || over(last) >= 0.0 {
            return true;
        }
        let to = segment(held, head);
        held[to - 1].at == held[to].at
    }

    /// The hour of the newest place: the publication's own.
    pub fn newest_hour(&self) -> i32 {
        self.held().last().map_or(0, |place| place.hour)
    }
}

/// The newer place of the pair of `held` that brackets `head`, clamped to the track's own ends, so
/// a head outside the places takes the stretch nearest it. `held` holds at least two places.
fn segment(held: &[Place], head: Head) -> usize {
    held.iter()
        .position(|place| (head.hour - place.hour) as f32 + head.fraction < 0.0)
        .unwrap_or(held.len() - 1)
        .max(1)
}

/// Shifts `x` by whole map widths until it is within half a width of `reference`.
pub fn unwrap_x(width: f32, reference: f32, x: f32) -> f32 {
    if width <= 0.0 || !reference.is_finite() || !x.is_finite() {
        return x;
    }
    x - width * ((x - reference) / width).round()
}

/// Where a map icon stands, as its owner tags its quads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconAnchor {
    /// The world position the icon's window is placed at: where its publication put it.
    pub world: [f32; 3],
    /// The content's distance cutoff for the icon's kind (for example
    /// `UNITS_ICONS_DISTANCE_CUTOFF`): the icon is gone by this distance from a free eye.
    pub cutoff: f32,
    pub family: IconFamily,
    /// The track the batch carries for this anchor ([`crate::map::sprites::Batch::track`]); none
    /// for an icon that stands where its hour put it. Unique to the track, so two icons placed at
    /// one point and moving apart are never moved as one.
    pub moves: Option<u32>,
}

impl IconAnchor {
    /// What makes the anchor the same anchor: every operand, by bits, its track among them.
    pub fn key(&self) -> [u32; 6] {
        [
            self.world[0].to_bits(),
            self.world[1].to_bits(),
            self.world[2].to_bits(),
            self.cutoff.to_bits(),
            self.family as u32,
            self.moves.map_or(u32::MAX, |serial| serial),
        ]
    }

    /// The anchor carrying the track `serial` names.
    pub fn moving(self, serial: Option<u32>) -> Self {
        Self { moves: serial, ..self }
    }

    /// Where the anchor stands at the frame's own head, given the track the batch carries for it:
    /// its placed point when it has none.
    pub fn at(&self, track: Option<&Track>, head: Head) -> [f32; 3] {
        track.map_or(self.world, |track| track.at(head))
    }
}

/// A fresh serial for every track a publication carries, so no two are ever confused.
pub fn next_track() -> u32 {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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
/// A free eye's icons, placed around their own pivots, at the frame's own head: a mover stands
/// where its track puts it there too, which the map camera's own path does by offsetting the
/// windows ([`crate::map::anchored`]).
#[allow(clippy::too_many_arguments, reason = "the camera, the view, the ground and the frame's head")]
pub fn present(
    camera: &Camera,
    viewport: [u32; 2],
    ui_scale: f32,
    ground: Option<&Ground<'_>>,
    batch: &Batch,
    at: Head,
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
    let mut seen: HashMap<[u32; 6], Option<(Present, f32)>> = HashMap::new();
    let mut shown: Vec<[u32; 6]> = Vec::new();
    for quad in &batch.quads {
        let Some(anchor) = &quad.anchor else {
            continue;
        };
        let key = anchor.key();
        if seen.contains_key(&key) {
            continue;
        }
        let look = look(camera, viewport, extent, ground, eye, range, anchor, anchor.at(batch.track_of(anchor), at));
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
#[allow(clippy::too_many_arguments, reason = "the camera, the view, the ground and where the icon stands")]
fn look(
    camera: &Camera,
    viewport: [u32; 2],
    extent: [i32; 2],
    ground: Option<&Ground<'_>>,
    eye: [f32; 3],
    range: f32,
    anchor: &IconAnchor,
    world: [f32; 3],
) -> Option<(Present, f32)> {
    let distance = camera.icon_distance(world);
    // A distance that is not a number shows nothing either.
    if distance.is_nan() || distance >= anchor.cutoff.min(range) {
        return None;
    }
    let pivot = camera.project_map_icon(world, viewport, extent)?.position;
    let (scale, mut alpha) = distance_look(distance, anchor.cutoff, range);
    if let Some(ground) = ground {
        alpha *= sight_alpha(ground, eye, world, distance);
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
