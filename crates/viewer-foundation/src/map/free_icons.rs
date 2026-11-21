
use std::collections::HashMap;

use super::{
    anchored::Head,
    camera::Camera,
    province_map::HeightField,
    sprites::{Batch, CHILD_SLOTS, Quad},
    terrain_sight::TerrainSight,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IconFamily {
    Pins,
    Bases,
    Raids,
    Labels,
    Columns,
    Combats,
    Missions,
    Occupation,
    Intel,
    Resources,
    AdjacencyRules,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub hour: i32,
    pub at: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    places: std::sync::Arc<[Place]>,
    pub width: f32,
}

impl Track {
    pub fn new(places: &[Place], width: f32) -> Self {
        let mut held = places.to_vec();
        for at in (1..held.len()).rev() {
            held[at - 1].at[0] = unwrap_x(width, held[at].at[0], held[at - 1].at[0]);
        }
        Self { places: std::sync::Arc::from(held), width }
    }

    pub fn places(&self) -> &[Place] {
        &self.places
    }

    fn held(&self) -> &[Place] {
        &self.places
    }

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

    pub fn moves(&self) -> bool {
        let held = self.held();
        held.len() > 1 && held.iter().any(|place| place.at != held[0].at)
    }

    pub fn outside(&self, head: Head) -> i8 {
        let held = self.held();
        let over = |place: &Place| (head.hour - place.hour) as f32 + head.fraction;
        match (held.first(), held.last()) {
            (Some(first), _) if over(first) < 0.0 => -1,
            (_, Some(last)) if over(last) > 0.0 => 1,
            _ => 0,
        }
    }

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

    pub fn newest_hour(&self) -> i32 {
        self.held().last().map_or(0, |place| place.hour)
    }
}

fn segment(held: &[Place], head: Head) -> usize {
    held.iter()
        .position(|place| (head.hour - place.hour) as f32 + head.fraction < 0.0)
        .unwrap_or(held.len() - 1)
        .max(1)
}

pub fn unwrap_x(width: f32, reference: f32, x: f32) -> f32 {
    if width <= 0.0 || !reference.is_finite() || !x.is_finite() {
        return x;
    }
    x - width * ((x - reference) / width).round()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconAnchor {
    pub world: [f32; 3],
    pub cutoff: f32,
    pub family: IconFamily,
    pub moves: Option<u32>,
}

impl IconAnchor {
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

    pub fn moving(self, serial: Option<u32>) -> Self {
        Self { moves: serial, ..self }
    }

    pub fn at(&self, track: Option<&Track>, head: Head) -> [f32; 3] {
        track.map_or(self.world, |track| track.at(head))
    }
}

pub fn next_track() -> u32 {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Present {
    pub pivot: [f32; 2],
    pub scale: f32,
    pub alpha: f32,
    pub rank: u32,
}

impl Present {
    pub fn unscale(&self, point: [f32; 2]) -> [f32; 2] {
        [0, 1].map(|axis| self.pivot[axis] + (point[axis] - self.pivot[axis]) / self.scale)
    }

    pub fn hittable(&self) -> bool {
        self.alpha >= HIT_ALPHA
    }
}

pub const SHRINK_FROM: f32 = 0.05;
pub const SHRINK_TO: f32 = 0.6;
pub const SMALLEST: f32 = 0.6;
pub const FADE_SPAN: f32 = 0.2;
pub const SIGHT_BAND: f32 = 0.25;
pub const SIGHT_BAND_GROWTH: f32 = 0.002;
pub const SIGHT_LIFT: f32 = 0.5;
pub const SIGHT_SKIP: f32 = 1.0;
pub const HIT_ALPHA: f32 = 0.5;
const DRAWN_ALPHA: f32 = 1.0 / 255.0;

#[derive(Clone, Copy)]
pub struct Ground<'a> {
    pub field: HeightField<'a>,
    pub sight: &'a TerrainSight,
}

pub fn distance_look(distance: f32, cutoff: f32, range: f32) -> (f32, f32) {
    let shrink = smoothstep(SHRINK_FROM * range, SHRINK_TO * range, distance);
    let scale = 1.0 - (1.0 - SMALLEST) * shrink;
    let end = cutoff.min(range);
    let alpha = 1.0 - smoothstep(end * (1.0 - FADE_SPAN), end, distance);
    (scale, alpha)
}

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

#[derive(Clone, Debug, Default)]
pub struct Looks {
    by_icon: HashMap<(IconFamily, u64), Present>,
}

impl Looks {
    pub fn of(&self, family: IconFamily, layer: u64) -> Option<&Present> {
        self.by_icon.get(&(family, layer / u64::from(CHILD_SLOTS)))
    }

    pub fn hit(&self, family: IconFamily, layer: u64, point: [f32; 2]) -> Option<([f32; 2], u32)> {
        self.of(family, layer)
            .filter(|present| present.hittable())
            .map(|present| (present.unscale(point), present.rank))
    }

    pub fn is_empty(&self) -> bool {
        self.by_icon.is_empty()
    }

    pub fn show(&mut self, family: IconFamily, layer: u64, present: Present) {
        self.by_icon
            .insert((family, layer / u64::from(CHILD_SLOTS)), present);
    }
}

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
    let extent = super::gui_resources::viewport(viewport, ui_scale);
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
