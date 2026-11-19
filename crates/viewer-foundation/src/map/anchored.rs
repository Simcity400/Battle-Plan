//! Overlay windows anchored in the world.
//!
//! A window's quads are made for its content, relative to the anchor it stands on. A frame whose
//! camera alone moved does not lay the overlay out, paint it or sort it again: it projects each
//! anchor at its own camera and offsets the windows standing on it, which is one loop over the
//! anchors and a small write over the quads that actually moved.
//!
//! The offset is whole interface pixels, as the game's own placing is ([`super::icon_windows`]),
//! so nothing is drawn off the pixel grid. It is measured from the camera the overlay was placed
//! at, never from the frame before, so a long move accumulates no error; what it leaves against a
//! whole re-layout is the half pixel the placing's own rounding held, bounded at one pixel on each
//! axis and gone at the next placing (every publication, every frame the camera stands still, and
//! whenever the camera has moved further than [`PAD`]).

use std::collections::HashMap;

use super::{
    camera::Camera,
    free_icons::{IconAnchor, IconFamily, Track},
    sprites::{Batch, CHILD_SLOTS, Quad},
};

/// How far, in interface pixels, the camera may move from the one the overlay was placed at before
/// the frame places it again: a window outside the placed view reaches the view only after moving
/// this far, so no icon an hour holds can be missing from the picture for longer.
pub const PAD: f32 = 64.0;

/// Why a frame's camera could not be answered by moving the windows it already has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The camera is not the placed one looking at another point of the map: its height, lens or
    /// pose changed, or the eye is free.
    NotAMove,
    /// The camera has moved further than [`PAD`] since the overlay was placed.
    PastThePad,
    /// An anchor does not project at one of the two cameras.
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

/// Where each anchor the overlay's quads stand on is now, against where it was when the overlay
/// was placed, in whole interface pixels; and the same by icon, for the hit shapes.
#[derive(Debug, Default)]
pub struct Shifts {
    /// The shift written into the batch, by [`IconAnchor::key`].
    applied: HashMap<[u32; 6], [i32; 2]>,
    /// The same shift by the icon its quads' layer keys name, which is how a hit shape finds it.
    by_icon: HashMap<(IconFamily, u64), [i32; 2]>,
    /// The largest move any anchor has made since the overlay was placed.
    largest: f32,
    /// Under the judge: the shift written into each quad so far, by its place in the batch, and
    /// what the last move found wrong.
    judged: Vec<[i32; 2]>,
    found: Vec<String>,
    /// The painting ([`Batch::painting`]) these shifts were written into. A batch painted again
    /// carries nothing: its quads stand where its own painting put them.
    painting: u64,
}

impl Shifts {
    pub fn is_empty(&self) -> bool {
        self.applied.is_empty()
    }

    /// The largest distance an anchor has moved since the overlay was placed, in interface pixels.
    pub fn largest(&self) -> f32 {
        self.largest
    }

    /// How the icon of `family` whose quads carry `layer` stands against where it was placed.
    pub fn of(&self, family: IconFamily, layer: u64) -> Option<[i32; 2]> {
        self.by_icon
            .get(&(family, layer / u64::from(CHILD_SLOTS)))
            .copied()
    }

    /// Forgets every shift: the overlay was placed again, so its quads stand where its anchors do.
    pub fn clear(&mut self) {
        self.applied.clear();
        self.by_icon.clear();
        self.largest = 0.0;
        self.judged.clear();
        self.painting = 0;
    }

    /// What the judge (`BATTLE_PLAN_VIEWER_CHECK_ANCHOR`) found wrong with the last move: for every quad
    /// of the overlay, the whole offset it carries is its own anchor's, and that offset is within
    /// one pixel of where this frame's camera projects that anchor. A quad left behind, moved
    /// twice, or moved by another anchor's motion is named here.
    pub fn found(&self) -> &[String] {
        &self.found
    }

    /// Moves `batch`'s anchored windows from the places `placed` gives their anchors to the places
    /// `camera` gives them. Only the quads whose anchor moved since the last call are written.
    /// On a refusal the batch is left exactly as it was and the caller places the overlay afresh.
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
        // A publication paints the batch again at the places its own camera gives. Those quads
        // carry nothing, so the offsets written into the painting before them are not theirs: a
        // move that took them for this painting's would leave a quad a pixel from its anchor
        // under a camera that keeps moving, and the error would accumulate.
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
            // How far this frame moves the window the last one placed, and a mover the hours moved
            // that lands on the same whole pixel: what a watcher sees of the replay advancing.
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
        // A window whose quads were moved draws its painting's quads elsewhere: the upload keeps
        // a window's quads by its key and its stamp alone, so where it stands is part of it. The
        // step is added to where the window already stood, which is where the composition placed
        // the quads it kept ([`Batch::at_window_standing`]).
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

    /// Shifts of named icons, for a test of what moves with them.
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

    /// Every anchor's shift, as the judge reads them.
    pub fn each(&self) -> impl Iterator<Item = ([u32; 6], [i32; 2])> + '_ {
        self.applied.iter().map(|(key, shift)| (*key, *shift))
    }
}

/// What one move wrote.
#[derive(Clone, Copy, Debug, Default)]
pub struct Moved {
    pub anchors: usize,
    pub anchors_moved: usize,
    /// Anchors standing somewhere other than their placed point for this head: the movers this
    /// frame placed at its own hour.
    pub movers: usize,
    /// Of those, the ones this frame leaves on the whole pixel the frame before put them on: the
    /// hours moved them less than half a pixel.
    pub movers_still: usize,
    /// The furthest any window moved from where the frame before placed it, in whole interface
    /// pixels on one axis.
    pub step_max: i32,
    pub quads: usize,
    /// What the movers in view travelled on this frame, as a watcher sees it: how many stood on
    /// the pixel the frame before put them on while their hours moved them, moved one pixel,
    /// two to four, and more than four.
    pub steps: [u64; 4],
    /// The pixels the movers travelled on this frame, summed, in thousandths: a run's total
    /// travel, which a sawtooth multiplies and even motion leaves equal to the hours' own.
    pub travel_milli: u64,
    /// Anchors carrying a track at all: what the movers' counts here are a share of.
    pub tracked: u64,
    /// Movers whose frame's head stands before the first hour their track holds a place for, and
    /// past its last: a frame the publication gives nothing to move them on, which holds them at
    /// an end.
    pub before_samples: u64,
    pub past_samples: u64,
}

impl Moved {
    /// What a watcher saw of the movers on this frame, under the names the record carries them by.
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

/// Whether `camera` is `placed` looking at another point of the map: the same eye height, lens and
/// pose, and neither a free eye, whose icons are placed around their own pivot every frame
/// ([`super::free_icons`]).
fn moved_over_the_map(placed: &Camera, camera: &Camera) -> bool {
    placed.free().is_none()
        && camera.free().is_none()
        && *placed
            == Camera {
                center: placed.center,
                ..*camera
            }
}

/// Where the frame stands: the hour of the replay it shows, as an integer hour and the fraction
/// into it. The hour is never held as an `f32`: a replay hour is about sixty million, where
/// neighbouring `f32` values are four hours apart, so a fractional head cast to one is lost and a
/// mover could only stand on every fourth hour.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Head {
    pub hour: i32,
    /// How far into `hour` the frame stands, in [0, 1).
    pub fraction: f32,
}

/// Where `anchor` stands at `camera` and this frame's head against where it stood at `placed` and
/// the head its publication placed it at, in interface pixels; and of that, how much the camera
/// alone moved it, which is what the pad bounds (a mover may travel as far as its hours take it).
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

/// The game's rounding of a placed point ([`super::icon_windows::point`]): away from zero at the
/// half pixel, so a window never lands off the pixel grid.
fn whole(value: f32) -> i32 {
    let value = if value >= 0.0 { value + 0.5 } else { value - 0.5 };
    if value.is_finite() && (-2_147_483_648.0..2_147_483_648.0).contains(&value) {
        value as i32
    } else {
        0
    }
}

/// Moves one quad by `step` whole interface pixels: its rectangle, its exact corners and scissor,
/// and the translation of the world matrix the game's own GUI shaders read, whose vertices are the
/// window's own local geometry ([`super::native_shader::draw::Quad::vertices`]).
///
/// A window drawn at an origin is its own drawing at any other whole-pixel origin moved this way:
/// what a draw puts at the origin is this translation and nothing else, which is why a
/// composition can keep the quads of a window it placed elsewhere
/// ([`super::icon_windows::Painted::paint`]).
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

/// Moves a native draw's world matrix by `by` interface pixels. The matrix is column-major and the
/// GUI's y runs down where the matrix's runs up ([`super::native_shader::draw::emit`]), so the
/// translation's y takes the step negated.
fn shift_world(constants: &mut [f32], by: [f32; 2]) {
    constants[12] += by[0];
    constants[13] -= by[1];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{
        camera::{MapDefines, MapSize, Placement},
        free_icons::Place,
        sprites::{WindowKey, WindowPool, WindowStamp},
    };

    const VIEWPORT: [u32; 2] = [1600, 900];
    const EXTENT: [i32; 2] = [1600, 900];
    const WORLD: [f32; 3] = [1000.0, 10.0, 500.0];

    fn looking_at(x: f32) -> Camera {
        Placement::map([x, 500.0], 110.0).on(MapSize::TEST, MapDefines::test())
    }

    /// One window of one anchored quad standing at `rect`, painted afresh into `batch`.
    fn paint(batch: &mut Batch, rect: [f32; 4]) {
        paint_standing(batch, rect, [0; 2]);
    }

    /// The same, for a window the painting kept and placed `from` pixels from where it was drawn.
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
            rotation: 0.0,
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

    /// A publication paints the batch again, at the places its own camera gives, and the quads it
    /// hands over carry no offset. The shifts of the painting before it are not theirs: a batch
    /// painted again is moved from its own places, or a quad stands a pixel from its anchor under
    /// a camera that keeps moving while the replay plays, and the error accumulates.
    #[test]
    fn a_batch_painted_again_is_moved_from_its_own_places_and_not_the_painting_before_it() {
        let placed = looking_at(1000.0);
        let mut shifts = Shifts::default();
        let mut batch = Batch::default();
        paint(&mut batch, [100.0, 200.0, 4.0, 4.0]);
        // Far enough for the anchor to move whole pixels, and back again.
        let moved = looking_at(1000.5);
        shifts
            .move_to(&mut batch, &placed, &moved, VIEWPORT, EXTENT, Head::default(), true)
            .expect("the camera only looked elsewhere");
        assert!(shifts.found().is_empty(), "{:?}", shifts.found());
        let first = batch.quads[0].rect[0];
        assert_ne!(first, 100.0, "the anchor moved whole pixels");
        // The publication's own painting: the same window at its own places, carrying nothing.
        paint(&mut batch, [100.0, 200.0, 4.0, 4.0]);
        let back = looking_at(999.5);
        let step = shifts
            .move_to(&mut batch, &placed, &back, VIEWPORT, EXTENT, Head::default(), true)
            .expect("the camera only looked elsewhere");
        assert!(shifts.found().is_empty(), "{:?}", shifts.found());
        assert_eq!(step.anchors, 1);
        // What the one quad carries is exactly what its anchor carries from the placing.
        let (_, offset) = shifts.each().next().expect("the one anchor");
        assert_eq!(
            [batch.quads[0].rect[0] - 100.0, batch.quads[0].rect[1] - 200.0],
            [offset[0] as f32, offset[1] as f32]
        );
    }

    /// A window the painting kept and placed elsewhere already stands away from where its
    /// painting drew it, and the camera's own move is added to that: what the batch records is the
    /// whole offset of the quads from the drawing, which is what the upload keeps a window by.
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
        // A second move of the same painting adds its own step and no more.
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

    /// A painting draws the layout it was made from where that layout was placed, and this frame's
    /// camera has moved on since (a frame that answered its camera by moving the windows lays
    /// nothing out, and the pools can still ask it to paint). So the painting is moved from the
    /// camera its layout stands at, and every quad then stands where this frame's camera projects
    /// its anchor. Taken for the frame's own camera instead, every window stays short by what the
    /// camera moved between the placing and the painting, for as long as that layout stands.
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
        // The pools ask for a painting of the same layout: it is painted at the layout's own
        // places again, and moved from the camera that layout stands at, not from this frame's.
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
        // Measured from this frame's camera, as if the painting stood there, the window is left at
        // the layout's own place: short by the whole of the camera's move.
        let mut taken_for_this_camera = Shifts::default();
        let mut batch = Batch::default();
        paint(&mut batch, [100.0, 200.0, 4.0, 4.0]);
        taken_for_this_camera
            .move_to(&mut batch, &now, &now, VIEWPORT, EXTENT, Head::default(), true)
            .expect("the camera only looked elsewhere");
        assert_eq!([batch.quads[0].rect[0] - 100.0, batch.quads[0].rect[1] - 200.0], [0.0, 0.0]);
    }

    /// A publication paints its movers where the hour it was laid out for puts them, and the frame
    /// that draws that painting shows an earlier hour (a publication is prepared for the head of
    /// the last frame that draws it). So the painting is placed at the frame's own head before it
    /// is drawn: the mover stands between the two places its publication carries, at that head,
    /// and the frame after it moves on toward this publication's place rather than back.
    #[test]
    fn a_painting_is_placed_at_the_frames_own_head_before_it_is_drawn() {
        let camera = looking_at(1000.0);
        let mut shifts = Shifts::default();
        let mut batch = Batch::default();
        paint(&mut batch, [100.0, 200.0, 4.0, 4.0]);
        // The publication placed the mover at WORLD and carries where the publication before it
        // read the stack: eight world units west, four hours earlier.
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
        // The frame after it carries the mover on from where this frame placed it, by the hours
        // between the two heads alone.
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

    /// A window painted at one origin and the same window painted at another differ by exactly the
    /// origin, in the rectangle, the corners, the scissor and the world matrix's translation
    /// ([`super::super::native_shader::draw::emit`] adds the origin to each): a move writes that
    /// difference instead of painting again.
    #[test]
    fn a_moved_quad_carries_its_rectangle_corners_and_scissor_with_it() {
        use crate::map::{native_effects::Blend, sprites::Exact};
        let mut quad = Quad {
            texture: 0,
            rect: [10.0, 20.0, 4.0, 4.0],
            uv: [0.0; 4],
            color: [1.0; 4],
            rotation: 0.0,
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

    #[test]
    fn a_moved_world_matrix_translates_the_games_own_gui_draw_by_the_same_pixels() {
        let mut constants = vec![0.0; 24];
        (constants[12], constants[13]) = (10.0, -20.0);
        shift_world(&mut constants, [3.0, -2.0]);
        assert_eq!([constants[12], constants[13]], [13.0, -18.0]);
    }
}
