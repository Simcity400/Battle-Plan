
use super::ModelPass;
use viewer_models::{KeyedInstance, ModelField, ModelInstance, ModelLibrary};
use eframe::wgpu;
use std::io;
use viewer_gpu::camera::{Camera, frustum};
use viewer_gpu::frame::FrameConstants;

#[derive(Clone)]
pub struct PlayerModel<'a> {
    pub library: &'a ModelLibrary,
    pub label: &'static str,
    pub mesh: usize,
    pub instance: ModelInstance,
}

const PLAYER_TILE: f32 = 256.0;

pub type Sphere = ([f32; 3], f32);

pub(super) struct Player {
    pub pass: ModelPass,
    label: &'static str,
    placed: Option<(usize, ModelInstance)>,
    field: ModelField,
    scratch: Vec<(usize, ModelInstance)>,
    keyed: Vec<KeyedInstance>,
    pub sphere: Sphere,
    pub offsets: Vec<f32>,
    pub frames: Vec<FrameConstants>,
}

impl Player {
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
            let mut pass = ModelPass::new(model.library, model.label);
            pass.drawn_directly();
            *slot = Some(Self {
                pass,
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
                .make_field_resident(device, queue)?;
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
