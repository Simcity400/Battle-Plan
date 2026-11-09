//! Exact upload inputs. Float equality alone would merge signed zero in native shader inputs.
use super::{Batch, Exact, Quad};
use crate::map::native_shader::draw;
use std::sync::Arc;

/// The complete drawn camera and viewport, compared by field bits rather than Rust padding.
/// This is only a hint to decline retention; the full upload key still decides every reuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CameraFrame {
    values: [u32; 81],
    free: Option<[u32; 6]>,
}

impl CameraFrame {
    pub fn new(camera: crate::map::camera::Camera, size: [u32; 2]) -> Self {
        use crate::map::{
            camera::{Camera, Free, Lens, MapDefines, MapSize, Pose},
            gradient_math::FieldDefines,
        };
        // Exhaustive patterns make a new camera operand a compile error until it is included.
        let Camera {
            center,
            height,
            map,
            defines,
            pose,
        } = camera;
        let MapSize {
            width,
            height: map_height,
        } = map;
        let MapDefines {
            tilt_lead,
            heights,
            outside,
            sun,
            moon,
            second_latitude,
            gradient_borders,
            shadows,
            fog,
            tree_fade,
            ambient,
            sun_diffuse,
            moon_diffuse,
            raid_icons,
            raid_icons_raid_mode,
            sun_specular,
            cubemap,
            country_names_cutoff,
            gradient_field,
            border_width,
            border_fades,
            sprawl_cutoff,
            map_objects_cutoff,
            railway_cutoff,
            railway_cutoff_speed,
        } = defines;
        let FieldDefines {
            center: field_center,
            country_thickness,
            country_area,
            strategic_thickness,
            optimization_range,
            navy_neutral,
            mode_thickness_max,
        } = gradient_field;
        let free = match pose {
            Pose::Map => None,
            Pose::Free(free) => {
                let Free {
                    yaw,
                    pitch,
                    roll,
                    lens,
                } = free;
                let Lens { fov_y, near, far } = lens;
                Some([yaw, pitch, roll, fov_y, near, far].map(f32::to_bits))
            }
        };
        Self {
            values: [
                center[0].to_bits(),
                center[1].to_bits(),
                height.to_bits(),
                width,
                map_height,
                size[0],
                size[1],
                tilt_lead.to_bits(),
                heights[0].to_bits(),
                heights[1].to_bits(),
                outside[0].to_bits(),
                outside[1].to_bits(),
                sun[0].to_bits(),
                sun[1].to_bits(),
                sun[2].to_bits(),
                moon[0].to_bits(),
                moon[1].to_bits(),
                second_latitude[0].to_bits(),
                second_latitude[1].to_bits(),
                gradient_borders[0].to_bits(),
                gradient_borders[1].to_bits(),
                gradient_borders[2].to_bits(),
                gradient_borders[3].to_bits(),
                shadows[0].to_bits(),
                shadows[1].to_bits(),
                fog[0].to_bits(),
                fog[1].to_bits(),
                tree_fade[0].to_bits(),
                tree_fade[1].to_bits(),
                ambient[0][0].to_bits(),
                ambient[0][1].to_bits(),
                ambient[0][2].to_bits(),
                ambient[1][0].to_bits(),
                ambient[1][1].to_bits(),
                ambient[1][2].to_bits(),
                ambient[2][0].to_bits(),
                ambient[2][1].to_bits(),
                ambient[2][2].to_bits(),
                ambient[3][0].to_bits(),
                ambient[3][1].to_bits(),
                ambient[3][2].to_bits(),
                ambient[4][0].to_bits(),
                ambient[4][1].to_bits(),
                ambient[4][2].to_bits(),
                ambient[5][0].to_bits(),
                ambient[5][1].to_bits(),
                ambient[5][2].to_bits(),
                sun_diffuse[0].to_bits(),
                sun_diffuse[1].to_bits(),
                sun_diffuse[2].to_bits(),
                sun_diffuse[3].to_bits(),
                moon_diffuse[0].to_bits(),
                moon_diffuse[1].to_bits(),
                moon_diffuse[2].to_bits(),
                moon_diffuse[3].to_bits(),
                raid_icons[0].to_bits(),
                raid_icons[1].to_bits(),
                raid_icons_raid_mode.to_bits(),
                sun_specular.to_bits(),
                cubemap.to_bits(),
                country_names_cutoff.to_bits(),
                field_center.to_bits(),
                country_thickness[0].to_bits(),
                country_thickness[1].to_bits(),
                country_area[0].to_bits(),
                country_area[1].to_bits(),
                strategic_thickness.to_bits(),
                optimization_range.to_bits(),
                u32::from(navy_neutral[0]),
                u32::from(navy_neutral[1]),
                u32::from(navy_neutral[2]),
                mode_thickness_max.to_bits(),
                border_width.to_bits(),
                border_fades[0][0].to_bits(),
                border_fades[0][1].to_bits(),
                border_fades[1][0].to_bits(),
                border_fades[1][1].to_bits(),
                sprawl_cutoff.to_bits(),
                map_objects_cutoff.to_bits(),
                railway_cutoff.to_bits(),
                railway_cutoff_speed.to_bits(),
            ],
            free,
        }
    }
}

pub(super) struct Upload {
    quads: Vec<Quad>,
    size: [u32; 2],
    scale: u32,
    artwork: Option<u64>,
}

impl Upload {
    pub(super) fn new(batch: &Batch, size: [u32; 2], scale: f32, artwork: Option<u64>) -> Self {
        let mut upload = Self {
            quads: Vec::new(),
            size,
            scale: scale.to_bits(),
            artwork,
        };
        upload.update(batch, size, scale, artwork);
        upload
    }

    /// Keep the snapshot's storage on a miss: a moving camera can change every quad.
    pub(super) fn update(
        &mut self,
        batch: &Batch,
        size: [u32; 2],
        scale: f32,
        artwork: Option<u64>,
    ) {
        self.quads.clone_from(&batch.quads);
        self.size = size;
        self.scale = scale.to_bits();
        self.artwork = artwork;
    }

    pub(super) fn matches(
        &self,
        batch: &Batch,
        size: [u32; 2],
        scale: f32,
        artwork: Option<u64>,
    ) -> bool {
        self.size == size
            && self.scale == scale.to_bits()
            && self.artwork == artwork
            && self.quads.len() == batch.quads.len()
            && self
                .quads
                .iter()
                .zip(&batch.quads)
                .all(|(a, b)| same_quad(a, b))
    }
}

#[inline]
fn floats(a: &[f32], b: &[f32]) -> bool {
    let a: &[u8] = bytemuck::cast_slice(a);
    a == bytemuck::cast_slice::<_, u8>(b)
}

fn same_quad(a: &Quad, b: &Quad) -> bool {
    a.texture == b.texture
        && a.layer == b.layer
        && floats(&a.rect, &b.rect)
        && floats(&a.uv, &b.uv)
        && floats(&a.color, &b.color)
        && a.rotation.to_bits() == b.rotation.to_bits()
        && a.shade.to_bits() == b.shade.to_bits()
        && match (a.anchor, b.anchor) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                a.family == b.family
                    && floats(&a.world, &b.world)
                    && a.cutoff.to_bits() == b.cutoff.to_bits()
            }
            _ => false,
        }
        && match (a.present, b.present) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                a.rank == b.rank
                    && floats(&a.pivot, &b.pivot)
                    && a.scale.to_bits() == b.scale.to_bits()
                    && a.alpha.to_bits() == b.alpha.to_bits()
            }
            _ => false,
        }
        && match (&a.exact, &b.exact) {
            (None, None) => true,
            (Some(a), Some(b)) => same_exact(a, b),
            _ => false,
        }
}

fn same_exact(a: &Exact, b: &Exact) -> bool {
    a.scissor == b.scissor
        && a.technique == b.technique
        && a.blend == b.blend
        && a.samplers == b.samplers
        && a.second == b.second
        && a.masking == b.masking
        && floats(
            bytemuck::cast_slice(&a.corners),
            bytemuck::cast_slice(&b.corners),
        )
        && floats(bytemuck::cast_slice(&a.uv), bytemuck::cast_slice(&b.uv))
        && floats(&a.lod_bias, &b.lod_bias)
        && floats(&a.offsets, &b.offsets)
        && floats(&a.inputs, &b.inputs)
        && floats(&a.second_color, &b.second_color)
        && match (&a.native, &b.native) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b) || same_native(a, b),
            _ => false,
        }
}

fn same_native(a: &draw::Quad, b: &draw::Quad) -> bool {
    a.stride == b.stride
        && a.known == b.known
        && a.pixel_constants == b.pixel_constants
        && a.font_viewport == b.font_viewport
        && a.topology == b.topology
        && a.default_blend == b.default_blend
        && a.declaration == b.declaration
        && floats(&a.constants, &b.constants)
        && a.effect.technique == b.effect.technique
        && (Arc::ptr_eq(&a.effect.asset, &b.effect.asset) || a.effect.asset == b.effect.asset)
        && a.textures == b.textures
        && a.extra_constants.len() == b.extra_constants.len()
        && a.extra_constants
            .iter()
            .zip(&b.extra_constants)
            .all(|(a, b)| a.0 == b.0 && a.2 == b.2 && floats(&a.1, &b.1))
        && a.path == b.path
        && a.vertices == b.vertices
        && match (&a.bake, &b.bake) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                Arc::ptr_eq(a, b)
                    || (a.size == b.size
                        && a.draws.len() == b.draws.len()
                        && a.draws.iter().zip(&b.draws).all(|(a, b)| same_native(a, b)))
            }
            _ => false,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> crate::map::camera::Camera {
        use crate::map::camera::{Camera, MapDefines, MapSize, Pose};
        Camera {
            center: [0.0, 10.0],
            height: 20.0,
            map: MapSize::TEST,
            defines: MapDefines::test(),
            pose: Pose::Map,
        }
    }

    #[test]
    fn sprite_camera_frame_covers_every_camera_operand_and_viewport() {
        let mut camera = camera();
        let mut count = 0;
        for value in camera
            .center
            .iter_mut()
            .chain(std::iter::once(&mut camera.height))
            .chain(std::iter::once(&mut camera.defines.tilt_lead))
            .chain(camera.defines.heights.iter_mut())
            .chain(camera.defines.outside.iter_mut())
            .chain(camera.defines.sun.iter_mut())
            .chain(camera.defines.moon.iter_mut())
            .chain(camera.defines.second_latitude.iter_mut())
            .chain(camera.defines.gradient_borders.iter_mut())
            .chain(camera.defines.shadows.iter_mut())
            .chain(camera.defines.fog.iter_mut())
            .chain(camera.defines.tree_fade.iter_mut())
            .chain(camera.defines.ambient.iter_mut().flatten())
            .chain(camera.defines.sun_diffuse.iter_mut())
            .chain(camera.defines.moon_diffuse.iter_mut())
            .chain(camera.defines.raid_icons.iter_mut())
            .chain(std::iter::once(&mut camera.defines.raid_icons_raid_mode))
            .chain(std::iter::once(&mut camera.defines.sun_specular))
            .chain(std::iter::once(&mut camera.defines.cubemap))
            .chain(std::iter::once(&mut camera.defines.country_names_cutoff))
            .chain(std::iter::once(&mut camera.defines.border_width))
            .chain(camera.defines.border_fades.iter_mut().flatten())
            .chain(std::iter::once(&mut camera.defines.sprawl_cutoff))
            .chain(std::iter::once(&mut camera.defines.map_objects_cutoff))
            .chain(std::iter::once(&mut camera.defines.railway_cutoff))
            .chain(std::iter::once(&mut camera.defines.railway_cutoff_speed))
            .chain(std::iter::once(&mut camera.defines.gradient_field.center))
            .chain(camera.defines.gradient_field.country_thickness.iter_mut())
            .chain(camera.defines.gradient_field.country_area.iter_mut())
            .chain(std::iter::once(
                &mut camera.defines.gradient_field.strategic_thickness,
            ))
            .chain(std::iter::once(
                &mut camera.defines.gradient_field.optimization_range,
            ))
            .chain(std::iter::once(
                &mut camera.defines.gradient_field.mode_thickness_max,
            ))
        {
            count += 1;
            *value = f32::from_bits(0x7fc0_0000 + count);
        }
        let key = CameraFrame::new(camera, [123, 456]);
        let mut observed: Vec<_> = key
            .values
            .into_iter()
            .filter(|value| *value > 0x7fc0_0000 && *value <= 0x7fc0_0000 + count)
            .collect();
        observed.sort_unstable();
        assert_eq!(
            observed,
            (1..=count).map(|n| 0x7fc0_0000 + n).collect::<Vec<_>>()
        );
        assert_eq!(key, CameraFrame::new(camera, [123, 456]));
        assert_ne!(key, CameraFrame::new(camera, [124, 456]));
        assert_ne!(key, CameraFrame::new(camera, [123, 457]));
        let mut changed = camera;
        changed.map.width += 1;
        assert_ne!(key, CameraFrame::new(changed, [123, 456]));
        changed = camera;
        changed.map.height += 1;
        assert_ne!(key, CameraFrame::new(changed, [123, 456]));
        for index in 0..3 {
            changed = camera;
            changed.defines.gradient_field.navy_neutral[index] ^= 1;
            assert_ne!(key, CameraFrame::new(changed, [123, 456]));
        }
    }

    #[test]
    fn sprite_camera_frame_keeps_zero_nan_and_every_free_pose_bit() {
        use crate::map::camera::{Free, Lens, Pose};
        let mut camera = camera();
        let map = CameraFrame::new(camera, [16, 16]);
        camera.center[0] = -0.0;
        assert_ne!(map, CameraFrame::new(camera, [16, 16]));
        camera.center[0] = f32::from_bits(0x7fc0_0001);
        let nan = CameraFrame::new(camera, [16, 16]);
        assert_eq!(nan, CameraFrame::new(camera, [16, 16]));
        camera.center[0] = f32::from_bits(0x7fc0_0002);
        assert_ne!(nan, CameraFrame::new(camera, [16, 16]));
        camera.pose = Pose::Free(Free::new(0.0, 0.0, Lens::MAP));
        let free = CameraFrame::new(camera, [16, 16]);
        assert_ne!(free, map);
        for index in 0..6 {
            let mut changed = camera;
            let Pose::Free(pose) = &mut changed.pose else {
                unreachable!()
            };
            let value = match index {
                0 => &mut pose.yaw,
                1 => &mut pose.pitch,
                2 => &mut pose.roll,
                3 => &mut pose.lens.fov_y,
                4 => &mut pose.lens.near,
                _ => &mut pose.lens.far,
            };
            *value = f32::from_bits(value.to_bits() ^ 1);
            assert_ne!(
                free,
                CameraFrame::new(changed, [16, 16]),
                "pose operand {index}"
            );
        }
    }

    #[test]
    fn retained_upload_float_inputs_include_every_component_and_nan_bit() {
        fn values<const N: usize>(next: &mut u32) -> [f32; N] {
            std::array::from_fn(|_| {
                let value = f32::from_bits(*next);
                *next += 1;
                value
            })
        }
        // Distinct NaN payloads ensure neither float equality nor an omitted field can pass.
        let mut next = 0x7fc0_0001;
        let quad = Quad {
            texture: 0,
            layer: 0,
            rect: values(&mut next),
            uv: values(&mut next),
            color: values(&mut next),
            rotation: values::<1>(&mut next)[0],
            shade: values::<1>(&mut next)[0],
            exact: None,
            anchor: None,
            present: None,
        };
        assert!(same_quad(&quad, &quad.clone()));
        for index in 0..14 {
            let mut changed = quad.clone();
            let value = changed
                .rect
                .iter_mut()
                .chain(&mut changed.uv)
                .chain(&mut changed.color)
                .chain(std::iter::once(&mut changed.rotation))
                .chain(std::iter::once(&mut changed.shade))
                .nth(index)
                .unwrap();
            *value = f32::from_bits(value.to_bits() ^ 1);
            assert!(!same_quad(&quad, &changed), "quad component {index}");
        }
        let exact = Exact {
            native: None,
            corners: std::array::from_fn(|_| values(&mut next)),
            uv: std::array::from_fn(|_| values(&mut next)),
            lod_bias: values(&mut next),
            offsets: values(&mut next),
            inputs: values(&mut next),
            second_color: values(&mut next),
            scissor: None,
            technique: 0,
            blend: crate::map::native_effects::Blend::Off,
            samplers: 0,
            second: None,
            masking: None,
        };
        assert!(same_exact(&exact, &exact.clone()));
        for index in 0..31 {
            let mut changed = exact.clone();
            let value = changed
                .corners
                .iter_mut()
                .flatten()
                .chain(changed.uv.iter_mut().flatten())
                .chain(&mut changed.lod_bias)
                .chain(&mut changed.offsets)
                .chain(&mut changed.inputs)
                .chain(&mut changed.second_color)
                .nth(index)
                .unwrap();
            *value = f32::from_bits(value.to_bits() ^ 1);
            assert!(!same_exact(&exact, &changed), "exact component {index}");
        }
    }

    #[test]
    fn retained_upload_inputs_compare_native_and_nested_bake_float_bits() {
        let program =
            installed_content_core::pdxshader::program::read_with(b"absent.lua", &|_| None).unwrap();
        let a = draw::Quad {
            source_draws: 1,
            path: "synthetic".into(),
            effect: draw::Effect {
                asset: crate::map::native_shader::library::Effect::from_compiled(
                    Arc::new(program),
                    vec![],
                    vec![],
                ),
                technique: "Up".into(),
            },
            declaration: crate::map::native_shader::vertices::Declaration::Sprite,
            vertices: vec![0; 20],
            stride: 20,
            constants: vec![0.; 16],
            known: 64,
            pixel_constants: true,
            textures: vec![(0, Some(1))],
            extra_constants: vec![(1, vec![0.; 4], std::iter::once(0..16).collect())],
            font_viewport: None,
            topology: eframe::wgpu::PrimitiveTopology::TriangleStrip,
            default_blend: false,
            bake: None,
        };
        assert!(same_native(&a, &a.clone()));
        let mut b = a.clone();
        b.constants[0] = -0.;
        assert!(!same_native(&a, &b));
        b = a.clone();
        b.extra_constants[0].1[0] = -0.;
        assert!(!same_native(&a, &b));
        b = a.clone();
        b.textures[0].1 = None;
        assert!(!same_native(&a, &b));
        b = a.clone();
        b.effect.technique = "Down".into();
        assert!(!same_native(&a, &b));
        let mut a = a;
        a.bake = Some(Arc::new(draw::Bake {
            size: [4, 4],
            draws: vec![a.clone()],
        }));
        let mut b = a.clone();
        assert!(same_native(&a, &b));
        Arc::make_mut(b.bake.as_mut().unwrap()).draws[0].constants[0] = -0.;
        assert!(!same_native(&a, &b));
    }

    #[test]
    fn retained_upload_misses_reuse_snapshot_storage_and_replace_every_input() {
        let mut batch = Batch::default();
        batch.scaled(
            &super::super::SpriteSheet {
                name: "synthetic".into(),
                texture: 0,
                frames: 1,
                size: [4, 4],
            },
            0,
            [0., 0.],
            [4., 4.],
            [1.; 4],
        );
        let mut kept = Upload::new(&batch, [16, 16], 1., Some(3));
        let storage = kept.quads.as_ptr();
        let capacity = kept.quads.capacity();
        batch.quads[0].rect[0] = -0.;
        assert!(!kept.matches(&batch, [17, 18], 2., Some(4)));
        kept.update(&batch, [17, 18], 2., Some(4));
        assert!(kept.matches(&batch, [17, 18], 2., Some(4)));
        assert_eq!(kept.quads.as_ptr(), storage);
        assert_eq!(kept.quads.capacity(), capacity);
        batch.clear();
        kept.update(&batch, [17, 18], 2., Some(4));
        assert!(kept.matches(&batch, [17, 18], 2., Some(4)));
        assert_eq!(kept.quads.capacity(), capacity);
    }

    #[test]
    fn retained_upload_inputs_preserve_float_bits_order_and_output_parameters() {
        let mut batch = Batch::default();
        batch.scaled(
            &super::super::SpriteSheet {
                name: "synthetic".into(),
                texture: 0,
                frames: 1,
                size: [4, 4],
            },
            0,
            [0., 0.],
            [4., 4.],
            [1.; 4],
        );
        let kept = Upload::new(&batch, [16, 16], 1., Some(3));
        assert!(kept.matches(&batch, [16, 16], 1., Some(3)));
        assert!(!kept.matches(&batch, [17, 16], 1., Some(3)));
        assert!(!kept.matches(&batch, [16, 16], 2., Some(3)));
        assert!(!kept.matches(&batch, [16, 16], 1., Some(4)));
        batch.quads[0].rect[0] = -0.;
        assert!(!kept.matches(&batch, [16, 16], 1., Some(3)));
        batch.quads[0].rect[0] = 0.;
        batch.quads[0].layer += 1;
        assert!(!kept.matches(&batch, [16, 16], 1., Some(3)));
        batch.quads[0].layer -= 1;
        batch.quads.push(batch.quads[0].clone());
        assert!(!kept.matches(&batch, [16, 16], 1., Some(3)));
    }
}
