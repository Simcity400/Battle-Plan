use viewer_gpu::icon_tracks::Head;
use std::collections::HashMap;

use viewer_gpu::{camera::Camera, icon_tracks::{IconAnchor, IconFamily, Track}};
use super::sprites::{Batch, CHILD_SLOTS, Quad};

pub const PAD: f32 = 64.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    NotAMove,
    PastThePad,
    Unprojected,
}

impl Refused {
    pub fn why(self) -> &'static str {
        match self {
            Self::NotAMove => "the camera did more than look at another point of the map",
            Self::PastThePad => "the camera moved further than the placed view's pad",
            Self::Unprojected => "an anchor does not project at both cameras",
        }
    }
}

#[derive(Debug, Default)]
pub struct Shifts {
    applied: HashMap<[u32; 6], [i32; 2]>,
    by_icon: HashMap<(IconFamily, u64), [i32; 2]>,
    largest: f32,
    judged: Vec<[i32; 2]>,
    found: Vec<String>,
    painting: u64,
}

impl Shifts {
    pub fn is_empty(&self) -> bool {
        self.applied.is_empty()
    }

    pub fn largest(&self) -> f32 {
        self.largest
    }

    pub fn of(&self, family: IconFamily, layer: u64) -> Option<[i32; 2]> {
        self.by_icon
            .get(&(family, layer / u64::from(CHILD_SLOTS)))
            .copied()
    }

    pub fn clear(&mut self) {
        self.applied.clear();
        self.by_icon.clear();
        self.largest = 0.0;
        self.judged.clear();
        self.painting = 0;
    }

    pub fn found(&self) -> &[String] {
        &self.found
    }

    #[allow(clippy::too_many_arguments, reason = "the two cameras, the frame's head and the view")]
    pub fn move_to(
        &mut self,
        batch: &mut Batch,
        placed: &Camera,
        camera: &Camera,
        viewport: [u32; 2],
        extent: [i32; 2],
        at: Head,
        judge: bool,
    ) -> Result<Moved, Refused> {
        if !moved_over_the_map(placed, camera) {
            return Err(Refused::NotAMove);
        }
        if self.painting != batch.painting() {
            self.clear();
            self.painting = batch.painting();
        }
        if judge {
            self.found.clear();
        }
        let mut wanted: HashMap<[u32; 6], [i32; 2]> = HashMap::with_capacity(self.applied.len());
        let mut largest = 0.0f32;
        let mut moving = 0;
        let mut still = 0;
        let mut step_max = 0;
        let mut steps = [0u64; 4];
        let mut travel_milli = 0u64;
        let (mut tracked, mut before_samples, mut past_samples) = (0u64, 0u64, 0u64);
        for quad in &batch.quads {
            let Some(anchor) = quad.anchor else {
                continue;
            };
            let key = anchor.key();
            if wanted.contains_key(&key) {
                continue;
            }
            let (step, by_the_camera) = step(placed, camera, &anchor, batch.track_of(&anchor), at, viewport, extent)?;
            largest = largest.max(by_the_camera[0].hypot(by_the_camera[1]));
            if largest > PAD {
                return Err(Refused::PastThePad);
            }
            if step != [0.0; 2] {
                moving += 1;
            }
            let whole = [whole(step[0]), whole(step[1])];
            let before = self.applied.get(&key).copied().unwrap_or([0; 2]);
            let moved_by = (whole[0] - before[0]).abs().max((whole[1] - before[1]).abs());
            step_max = step_max.max(moved_by);
            if anchor.moves.is_some() {
                tracked += 1;
                if whole == before && step != [0.0; 2] {
                    still += 1;
                }
                let band = match moved_by {
                    0 if step != [0.0; 2] => Some(0),
                    0 => None,
                    1 => Some(1),
                    2..=4 => Some(2),
                    _ => Some(3),
                };
                if let Some(band) = band {
                    steps[band] += 1;
                }
                travel_milli += (((whole[0] - before[0]) as f32).hypot((whole[1] - before[1]) as f32) * 1000.0) as u64;
                match batch.track_of(&anchor).map_or(0, |track| track.outside(at)) {
                    -1 => before_samples += 1,
                    1 => past_samples += 1,
                    _ => {}
                }
            }
            if judge && let Some(axis) = [0, 1].into_iter().find(|&axis| (whole[axis] as f32 - step[axis]).abs() > 1.0) {
                self.found.push(format!(
                    "an anchor of {:?} is offset by {} pixels on axis {axis} where this frame's camera moves it {}",
                    anchor.family, whole[axis], step[axis]
                ));
            }
            wanted.insert(key, whole);
        }
        self.by_icon.clear();
        if judge {
            self.judged.resize(batch.quads.len(), [0; 2]);
        }
        let mut quads_moved = 0;
        for (at, quad) in batch.quads.iter_mut().enumerate() {
            let Some(anchor) = quad.anchor else {
                continue;
            };
            let key = anchor.key();
            let to = wanted.get(&key).copied().unwrap_or([0; 2]);
            let from = self.applied.get(&key).copied().unwrap_or([0; 2]);
            let step = [to[0] - from[0], to[1] - from[1]];
            if step != [0; 2] {
                shift(quad, step);
                quads_moved += 1;
            }
            if judge && let Some(carried) = self.judged.get_mut(at) {
                carried[0] += step[0];
                carried[1] += step[1];
                if *carried != to {
                    self.found.push(format!(
                        "quad {at} carries {carried:?} where its anchor stands {to:?} from its placing"
                    ));
                }
            }
            self.by_icon
                .insert((anchor.family, quad.layer / u64::from(CHILD_SLOTS)), to);
        }
        let mut stepped: Vec<[i32; 2]> = Vec::new();
        for window in batch.windows() {
            let key = batch.quads[window.quads()]
                .iter()
                .find_map(|quad| quad.anchor.map(|anchor| anchor.key()));
            let to = key.and_then(|key| wanted.get(&key).copied()).unwrap_or([0; 2]);
            let from = key
                .and_then(|key| self.applied.get(&key).copied())
                .unwrap_or([0; 2]);
            stepped.push([to[0] - from[0], to[1] - from[1]]);
        }
        for (window, step) in batch.standing_windows().iter_mut().zip(stepped) {
            window.shift[0] += step[0];
            window.shift[1] += step[1];
        }
        let anchors_moved = wanted
            .iter()
            .filter(|(key, to)| self.applied.get(*key).copied().unwrap_or([0; 2]) != **to)
            .count();
        let moved = Moved {
            anchors: wanted.len(),
            anchors_moved,
            movers: moving,
            movers_still: still,
            step_max,
            quads: quads_moved,
            steps,
            travel_milli,
            tracked,
            before_samples,
            past_samples,
        };
        self.applied = wanted;
        self.largest = largest;
        Ok(moved)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn of_icons(icons: impl IntoIterator<Item = (IconFamily, u64, [i32; 2])>) -> Self {
        let mut shifts = Self::default();
        for (family, layer, shift) in icons {
            shifts
                .by_icon
                .insert((family, layer / u64::from(CHILD_SLOTS)), shift);
            shifts.applied.insert([layer as u32; 6], shift);
        }
        shifts
    }

    pub fn each(&self) -> impl Iterator<Item = ([u32; 6], [i32; 2])> + '_ {
        self.applied.iter().map(|(key, shift)| (*key, *shift))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Moved {
    pub anchors: usize,
    pub anchors_moved: usize,
    pub movers: usize,
    pub movers_still: usize,
    pub step_max: i32,
    pub quads: usize,
    pub steps: [u64; 4],
    pub travel_milli: u64,
    pub tracked: u64,
    pub before_samples: u64,
    pub past_samples: u64,
}

impl Moved {
    pub fn named(&self) -> [(&'static str, u64); 8] {
        [
            ("movers_tracked", self.tracked),
            ("movers_stood", self.steps[0]),
            ("movers_1px", self.steps[1]),
            ("movers_2to4px", self.steps[2]),
            ("movers_over4px", self.steps[3]),
            ("movers_travel_milli", self.travel_milli),
            ("movers_before_samples", self.before_samples),
            ("movers_past_samples", self.past_samples),
        ]
    }
}

fn moved_over_the_map(placed: &Camera, camera: &Camera) -> bool {
    placed.free().is_none()
        && camera.free().is_none()
        && *placed
            == Camera {
                center: placed.center,
                ..*camera
            }
}

fn step(
    placed: &Camera,
    camera: &Camera,
    anchor: &IconAnchor,
    track: Option<&Track>,
    at: Head,
    viewport: [u32; 2],
    extent: [i32; 2],
) -> Result<([f32; 2], [f32; 2]), Refused> {
    let project = |camera: &Camera, world: [f32; 3]| {
        camera
            .project_map_icon(world, viewport, extent)
            .map(|projection| projection.position)
            .ok_or(Refused::Unprojected)
    };
    let was = project(placed, anchor.world)?;
    let standing = project(camera, anchor.world)?;
    let now = match track {
        Some(_) => project(camera, anchor.at(track, at))?,
        None => standing,
    };
    let step = [0, 1].map(|axis| now[axis] - was[axis]);
    let by_the_camera = [0, 1].map(|axis| standing[axis] - was[axis]);
    if step.iter().chain(&by_the_camera).any(|value| !value.is_finite()) {
        return Err(Refused::Unprojected);
    }
    Ok((step, by_the_camera))
}

fn whole(value: f32) -> i32 {
    let value = if value >= 0.0 { value + 0.5 } else { value - 0.5 };
    if value.is_finite() && (-2_147_483_648.0..2_147_483_648.0).contains(&value) {
        value as i32
    } else {
        0
    }
}

pub fn shift(quad: &mut Quad, step: [i32; 2]) {
    let by = step.map(|value| value as f32);
    quad.rect[0] += by[0];
    quad.rect[1] += by[1];
    let Some(exact) = quad.exact.as_mut() else {
        return;
    };
    for corner in &mut exact.corners {
        corner[0] += by[0];
        corner[1] += by[1];
    }
    if let Some(scissor) = exact.scissor.as_mut() {
        scissor[0] += step[0];
        scissor[1] += step[1];
        scissor[2] += step[0];
        scissor[3] += step[1];
    }
    if let Some(native) = exact.native.as_mut() {
        shift_world(&mut std::sync::Arc::make_mut(native).constants, by);
    }
}

fn shift_world(constants: &mut [f32], by: [f32; 2]) {
    constants[12] += by[0];
    constants[13] -= by[1];
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewer_gpu::{camera::{MapDefines, MapSize, Placement}, icon_tracks::Place};
    use crate::sprites::{WindowKey, WindowPool, WindowStamp};

    const VIEWPORT: [u32; 2] = [1600, 900];
    const EXTENT: [i32; 2] = [1600, 900];
    const WORLD: [f32; 3] = [1000.0, 10.0, 500.0];

    fn looking_at(x: f32) -> Camera {
        Placement::map([x, 500.0], 110.0).on(MapSize::TEST, MapDefines::test())
    }

    fn paint(batch: &mut Batch, rect: [f32; 4]) {
        paint_standing(batch, rect, [0; 2]);
    }

    fn paint_standing(batch: &mut Batch, rect: [f32; 4], from: [i32; 2]) {
        batch.clear();
        batch.at_window_standing(
            WindowKey::new(WindowPool::Base, 0, 1_u64),
            WindowStamp { painting: 1, taken: 1 },
            from,
        );
        batch.quads.push(Quad {
            texture: 0,
            rect,
            uv: [0.0; 4],
            color: [1.0; 4],
            rotation: None,
            layer: 0,
            shade: 0.0,
            exact: None,
            anchor: Some(IconAnchor {
                world: WORLD,
                cutoff: f32::INFINITY,
                family: IconFamily::Columns,
                moves: None,
            }),
            present: None,
        });
        batch.no_window();
    }

    #[test]
    fn a_batch_painted_again_is_moved_from_its_own_places_and_not_the_painting_before_it() {
        let placed = looking_at(1000.0);
        let mut shifts = Shifts::default();
        let mut batch = Batch::default();
        paint(&mut batch, [100.0, 200.0, 4.0, 4.0]);
        let moved = looking_at(1000.5);
        shifts
            .move_to(&mut batch, &placed, &moved, VIEWPORT, EXTENT, Head::default(), true)
            .expect("the camera only looked elsewhere");
        assert!(shifts.found().is_empty(), "{:?}", shifts.found());
        let first = batch.quads[0].rect[0];
        assert_ne!(first, 100.0, "the anchor moved whole pixels");
        paint(&mut batch, [100.0, 200.0, 4.0, 4.0]);
        let back = looking_at(999.5);
        let step = shifts
            .move_to(&mut batch, &placed, &back, VIEWPORT, EXTENT, Head::default(), true)
            .expect("the camera only looked elsewhere");
        assert!(shifts.found().is_empty(), "{:?}", shifts.found());
        assert_eq!(step.anchors, 1);
        let (_, offset) = shifts.each().next().expect("the one anchor");
        assert_eq!(
            [batch.quads[0].rect[0] - 100.0, batch.quads[0].rect[1] - 200.0],
            [offset[0] as f32, offset[1] as f32]
        );
    }

    #[test]
    fn a_window_the_painting_placed_elsewhere_carries_that_offset_and_the_cameras_too() {
        let placed = looking_at(1000.0);
        let mut shifts = Shifts::default();
        let mut batch = Batch::default();
        paint_standing(&mut batch, [100.0, 200.0, 4.0, 4.0], [7, -3]);
        assert_eq!(batch.windows().next().expect("the window").shift, [7, -3]);
        let moved = looking_at(1000.5);
        shifts
            .move_to(&mut batch, &placed, &moved, VIEWPORT, EXTENT, Head::default(), true)
            .expect("the camera only looked elsewhere");
        assert!(shifts.found().is_empty(), "{:?}", shifts.found());
        let (_, by_the_camera) = shifts.each().next().expect("the one anchor");
        assert_ne!(by_the_camera, [0; 2], "the anchor moved whole pixels");
        assert_eq!(
            batch.windows().next().expect("the window").shift,
            [7 + by_the_camera[0], -3 + by_the_camera[1]],
            "the painting's own offset and the camera's"
        );
        let further = looking_at(1001.0);
        shifts
            .move_to(&mut batch, &placed, &further, VIEWPORT, EXTENT, Head::default(), true)
            .expect("the camera only looked elsewhere");
        let (_, further_by_the_camera) = shifts.each().next().expect("the one anchor");
        assert_eq!(
            batch.windows().next().expect("the window").shift,
            [7 + further_by_the_camera[0], -3 + further_by_the_camera[1]]
        );
    }

    #[test]
    fn a_painting_stands_where_this_frames_camera_projects_its_anchor() {
        let placed = looking_at(1000.0);
        let now = looking_at(1000.5);
        let project = |camera: &Camera| {
            camera
                .project_map_icon(WORLD, VIEWPORT, EXTENT)
                .expect("the anchor projects")
                .position
        };
        let by_the_camera = [0, 1].map(|axis| whole(project(&now)[axis] - project(&placed)[axis]));
        assert_ne!(by_the_camera, [0, 0], "the camera moved the anchor whole pixels");
        let mut shifts = Shifts::default();
        let mut batch = Batch::default();
        paint(&mut batch, [100.0, 200.0, 4.0, 4.0]);
        shifts
            .move_to(&mut batch, &placed, &now, VIEWPORT, EXTENT, Head::default(), true)
            .expect("the camera only looked elsewhere");
        let offset = [batch.quads[0].rect[0] - 100.0, batch.quads[0].rect[1] - 200.0];
        assert_eq!(offset, by_the_camera.map(|step| step as f32));
        paint(&mut batch, [100.0, 200.0, 4.0, 4.0]);
        shifts
            .move_to(&mut batch, &placed, &now, VIEWPORT, EXTENT, Head::default(), true)
            .expect("the camera only looked elsewhere");
        assert!(shifts.found().is_empty(), "{:?}", shifts.found());
        assert_eq!(
            [batch.quads[0].rect[0] - 100.0, batch.quads[0].rect[1] - 200.0],
            by_the_camera.map(|step| step as f32),
            "the painting stands where this frame's camera projects the anchor"
        );
        let mut taken_for_this_camera = Shifts::default();
        let mut batch = Batch::default();
        paint(&mut batch, [100.0, 200.0, 4.0, 4.0]);
        taken_for_this_camera
            .move_to(&mut batch, &now, &now, VIEWPORT, EXTENT, Head::default(), true)
            .expect("the camera only looked elsewhere");
        assert_eq!([batch.quads[0].rect[0] - 100.0, batch.quads[0].rect[1] - 200.0], [0.0, 0.0]);
    }

    #[test]
    fn a_painting_is_placed_at_the_frames_own_head_before_it_is_drawn() {
        let camera = looking_at(1000.0);
        let mut shifts = Shifts::default();
        let mut batch = Batch::default();
        paint(&mut batch, [100.0, 200.0, 4.0, 4.0]);
        let track = Track::new(&[Place { hour: 10, at: [WORLD[0] - 8.0, WORLD[1], WORLD[2]] }, Place { hour: 14, at: WORLD }], 0.0);
        let serial = batch.add_track(track);
        for quad in &mut batch.quads {
            quad.anchor = quad.anchor.map(|anchor| anchor.moving(Some(serial)));
        }
        let at = Head { hour: 12, fraction: 0.0 };
        let moved = shifts
            .move_to(&mut batch, &camera, &camera, VIEWPORT, EXTENT, at, true)
            .expect("the painting's own camera");
        assert!(shifts.found().is_empty(), "{:?}", shifts.found());
        assert_eq!(moved.movers, 1, "the one mover is placed at this frame's head");
        let placed = batch.quads[0].rect[0];
        assert_ne!(placed, 100.0, "two hours behind the painting's own hour moved it whole pixels");
        let next = Head { hour: 13, fraction: 0.0 };
        shifts
            .move_to(&mut batch, &camera, &camera, VIEWPORT, EXTENT, next, true)
            .expect("the painting's own camera");
        assert!(shifts.found().is_empty(), "{:?}", shifts.found());
        let then = batch.quads[0].rect[0];
        assert!(then > placed, "the mover advanced with the head: {placed} then {then}");
    }

    #[test]
    fn a_whole_pixel_is_rounded_away_from_zero_as_the_games_own_placing_rounds_it() {
        assert_eq!(
            [whole(0.4), whole(0.5), whole(-0.4), whole(-0.5), whole(1.6), whole(-1.6)],
            [0, 1, 0, -1, 2, -2]
        );
        assert_eq!([whole(f32::NAN), whole(f32::INFINITY)], [0, 0]);
    }

    #[test]
    fn a_moved_quad_carries_its_rectangle_corners_and_scissor_with_it() {
        use crate::{native_effects::Blend, sprites::Exact};
        let mut quad = Quad {
            texture: 0,
            rect: [10.0, 20.0, 4.0, 4.0],
            uv: [0.0; 4],
            color: [1.0; 4],
            rotation: None,
            layer: 0,
            shade: 0.0,
            exact: Some(Exact {
                native: None,
                corners: [[10.0, 20.0], [14.0, 20.0], [14.0, 24.0], [10.0, 24.0]],
                uv: [[0.0; 2]; 4],
                scissor: Some([10, 20, 14, 24]),
                technique: 0,
                blend: Blend::Off,
                samplers: 0,
                lod_bias: [0.0; 3],
                second: None,
                masking: None,
                offsets: [0.0; 4],
                inputs: [0.0; 4],
                second_color: [0.0; 4],
            }),
            anchor: None,
            present: None,
        };
        shift(&mut quad, [3, -2]);
        let exact = quad.exact.as_ref().unwrap();
        assert_eq!(quad.rect, [13.0, 18.0, 4.0, 4.0]);
        assert_eq!(exact.corners, [[13.0, 18.0], [17.0, 18.0], [17.0, 22.0], [13.0, 22.0]]);
        assert_eq!(exact.scissor, Some([13, 18, 17, 22]));
    }

}
