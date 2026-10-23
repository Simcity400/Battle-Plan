//! The model a free camera mode's user steers: one entity in its own pass, placed every frame by
//! the mode's controller ([`super::Scene::set_player`]). It draws apart from the unit cutoffs and
//! display switches, on every map copy its own bounds reach inside the view volume however high
//! it stands above the band the map's objects are admitted in. It casts as every caster does,
//! inside the light the game fits to the camera's capped water-plane footprint.

use super::ModelPass;
use crate::map::models::{KeyedInstance, ModelField, ModelInstance, ModelLibrary};
use eframe::wgpu;
use std::io;
use viewer_foundation::map::camera::{Camera, frustum};
use viewer_foundation::map::frame::FrameConstants;

/// The model a free camera mode's user steers: one entity of a content library.
#[derive(Clone)]
pub struct PlayerModel<'a> {
    pub library: &'a ModelLibrary,
    /// The pass label whose entity data the library's figures take: `"unit models"` for a land
    /// figure, `"flying aircraft"` for an aircraft.
    pub label: &'static str,
    /// The entity's slot in `library`.
    pub mesh: usize,
    pub instance: ModelInstance,
}

/// The cull tile of the player's one-instance field.
const PLAYER_TILE: f32 = 256.0;

/// A bounding sphere: centre and radius.
pub type Sphere = ([f32; 3], f32);

/// The player's pass and the storage its frames reuse.
pub(super) struct Player {
    pub pass: ModelPass,
    label: &'static str,
    /// The placement the pass holds; a frame that does not move it changes nothing.
    placed: Option<(usize, ModelInstance)>,
    /// The field and its build scratch, kept between moves.
    field: ModelField,
    scratch: Vec<(usize, ModelInstance)>,
    keyed: Vec<KeyedInstance>,
    /// Its bounding sphere: around its placement, as far as its mesh's farthest point at its
    /// scale ([`crate::map::models::ModelMesh::bound`]).
    pub sphere: Sphere,
    /// This frame's map copies as x offsets ([`copies`]), and each copy's scene constants.
    pub offsets: Vec<f32>,
    pub frames: Vec<FrameConstants>,
}

impl Player {
    /// Places `model` in `slot`, building the pass afresh for a new label. A placement equal to
    /// the last changes nothing; a moved one rebuilds the field in the storage it kept.
    pub(super) fn place(
        slot: &mut Option<Self>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        model: PlayerModel<'_>,
    ) -> io::Result<()> {
        if slot
            .as_ref()
            .is_none_or(|player| player.label != model.label)
        {
            *slot = Some(Self {
                pass: ModelPass::new(model.library, model.label),
                label: model.label,
                placed: None,
                field: ModelField::default(),
                scratch: Vec::with_capacity(1),
                keyed: Vec::with_capacity(1),
                sphere: ([0.0; 3], 0.0),
                offsets: Vec::new(),
                frames: Vec::new(),
            });
        }
        let player = slot.as_mut().expect("built above");
        let placement = (model.mesh, model.instance);
        if player.placed == Some(placement) {
            return Ok(());
        }
        // What the pass must upload depends on the entity alone, not on where it stands.
        let entity_changed = player.placed.is_none_or(|(mesh, _)| mesh != model.mesh);
        player.scratch.push(placement);
        player.field.rebuild(
            model.library,
            &mut player.scratch,
            &mut player.keyed,
            PLAYER_TILE,
        );
        player.pass.replace_field(model.library, &mut player.field);
        if entity_changed {
            player
                .pass
                .make_field_resident(device, queue, model.library)?;
        }
        let bound = model
            .library
            .meshes
            .get(model.mesh)
            .map_or(0.0, |mesh| mesh.bound);
        player.sphere = (model.instance.position, bound * model.instance.scale);
        player.placed = Some(placement);
        Ok(())
    }
}

/// The x offsets of the map copies on which `sphere` reaches inside `camera`'s view volume, into
/// `out`: every copy within the view volume's own x extent (its corners', an oblique view's far
/// corners reaching farther in x than its depth) is tested, whatever the height, so a copy
/// across the seam is drawn wherever it is seen.
pub fn copies(camera: &Camera, viewport: [u32; 2], (centre, radius): Sphere, out: &mut Vec<f32>) {
    out.clear();
    let planes = camera.frustum_planes(viewport);
    let inside = |x: f32| {
        planes.iter().all(|&[a, b, c, d]| {
            let length = (a * a + b * b + c * c).sqrt();
            length > 0.0 && (a * x + b * centre[1] + c * centre[2] + d) / length >= -radius
        })
    };
    let width = camera.map.x();
    if !(width > 0.0 && width.is_finite()) {
        if inside(centre[0]) {
            out.push(0.0);
        }
        return;
    }
    let lens = camera.lens();
    let corners = frustum::corners(camera, viewport, lens.near, lens.far);
    let (west, east) = corners.iter().fold(
        (f32::INFINITY, f32::NEG_INFINITY),
        |(west, east), corner| (west.min(corner[0]), east.max(corner[0])),
    );
    if !(west.is_finite() && east.is_finite()) {
        return;
    }
    let first = ((west - radius - centre[0]) / width).floor() as i32;
    let last = ((east + radius - centre[0]) / width).ceil() as i32;
    for copy in first..=last {
        let offset = copy as f32 * width;
        if inside(centre[0] + offset) {
            out.push(offset);
        }
    }
}

#[cfg(test)]
#[path = "scene_player/tests.rs"]
mod tests;
