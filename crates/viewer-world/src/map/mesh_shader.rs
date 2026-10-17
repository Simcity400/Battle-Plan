//! Mesh input declarations, compact vertices and skin weights.
use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use installed_content::pdxshader::{
    Address,
    program::{Filter, Sampler, Stage},
};
use shader_pipeline::{
    ScalarKind, VertexInput, binding, gpu,
    vertices::{NativeBuffer, pack_native},
};
use std::sync::Arc;
use std::{collections::BTreeMap, io};

/// Imported geometry stays in mesh space; actor transforms belong to b1/b2.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Geometry {
    /// the game's compact native allocation.
    pub vertices: Vec<f32>,
    pub skin: Vec<SkinVertex>,
    pub indices: Vec<u32>,
    pub uv1: bool,
    pub stride: u64,
}

impl Geometry {
    pub fn from_mesh(sub: &installed_content::mesh::SubMesh, scale: f32) -> io::Result<Self> {
        let count = sub.vertex_count();
        if count == 0 {
            return Err(io::Error::other("native mesh has no positions"));
        }
        // The game packs correctly sized p,n,ta,u0,u1,u2,u3 without holes.
        let normals = (sub.normals.len() == count).then_some(&sub.normals);
        let tangents = (sub.tangents.len() == count).then_some(&sub.tangents);
        let uv0 = (sub.uv0.len() == count).then_some(&sub.uv0);
        let coordinates = [&sub.uv1, &sub.uv2, &sub.uv3]
            .map(|values| values.as_ref().filter(|v| v.len() == count));
        let width = 3
            + usize::from(normals.is_some()) * 3
            + usize::from(tangents.is_some()) * 4
            + usize::from(uv0.is_some()) * 2
            + coordinates.iter().filter(|v| v.is_some()).count() * 2;
        let mut vertices = Vec::with_capacity(count * width);
        // The game writes bit pattern 0x3C23D70A for both repairs, including -0.
        let direction = |mut v: [f32; 3]| {
            if v == [0.; 3] {
                v[1] = 0.01;
            }
            v
        };
        for i in 0..count {
            vertices.extend(sub.positions[i].map(|v| v * scale));
            if let Some(normals) = normals {
                vertices.extend(direction(normals[i]));
            }
            if let Some(tangents) = tangents {
                let [x, y, z, w] = tangents[i];
                vertices.extend(direction([x, y, z]));
                vertices.push(if w == 0. { 0.01 } else { w });
            }
            if let Some(uv0) = uv0 {
                vertices.extend(uv0[i]);
            }
            for values in coordinates.iter().flatten() {
                vertices.extend(values[i]);
            }
        }
        let skin = sub.skin.as_ref().map_or_else(Vec::new, |skin| {
            skin.joints
                .iter()
                .zip(&skin.weights)
                .map(|(i, w)| SkinVertex::from_authored(*i, *w))
                .collect()
        });
        Ok(Self {
            vertices,
            skin,
            indices: sub.indices.clone(),
            // The game clears the UV1 bit only when u1 is absent, not wrong-sized.
            uv1: sub.uv1.is_some(),
            stride: (width * 4) as u64,
        })
    }

    /// Fixed vertex fetches, always including UV1 for trees.
    pub fn packed(&self, uv1: bool) -> io::Result<Vec<f32>> {
        let bytes = bytemuck::cast_slice(&self.vertices);
        let known = 0..bytes.len() as u64;
        let buffers = BTreeMap::from([(
            0,
            Some(NativeBuffer {
                bytes,
                size: known.end,
                offset: 0,
                known: std::slice::from_ref(&known),
            }),
        )]);
        let count = u32::try_from(bytes.len() as u64 / self.stride).map_err(io::Error::other)?;
        let packed = pack_native(&[declaration(uv1, self.stride)], &buffers, count, 1)
            .map_err(io::Error::other)?;
        Ok(packed.bytes[0]
            .chunks_exact(4)
            .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
            .collect())
    }
}

pub struct Pass {
    pub pipeline: Arc<gpu::Pipeline>,
    /// The technique the pipeline was built from: the recorded effect's when a visual mod's
    /// replacement fell back, so its vertex inputs are the ones drawn.
    pub selection: shader_pipeline::library::Selection,
    reference: u32,
}

impl Pass {
    /// `cache` is shared by the caller's parts, so parts with one program and state share
    /// a pipeline and consecutive draws rebind only what changed.
    pub fn new(
        cache: &mut gpu::Cache,
        device: &wgpu::Device,
        selection: &shader_pipeline::library::Selection,
        geometry: &Geometry,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
        samples: u32,
    ) -> io::Result<Self> {
        Self::with_mode(
            cache, device, selection, geometry, format, depth, samples, false,
        )
    }

    /// The game applies the shadow material handles; its map callback does not
    ///bind the lit map samplers, and the material can update only PS0.
    pub fn caster(
        cache: &mut gpu::Cache,
        device: &wgpu::Device,
        selection: &shader_pipeline::library::Selection,
        geometry: &Geometry,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
    ) -> io::Result<Self> {
        Self::with_mode(cache, device, selection, geometry, format, depth, 1, true)
    }

    #[allow(clippy::too_many_arguments)]
    fn with_mode(
        cache: &mut gpu::Cache,
        device: &wgpu::Device,
        selection: &shader_pipeline::library::Selection,
        geometry: &Geometry,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
        samples: u32,
        caster: bool,
    ) -> io::Result<Self> {
        // A visual mod's replacement that cannot be drawn here falls back to the recording.
        shader_pipeline::library::with_recorded_selection(selection, |selection| {
            Self::build(
                cache, device, selection, geometry, format, depth, samples, caster,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        cache: &mut gpu::Cache,
        device: &wgpu::Device,
        selection: &shader_pipeline::library::Selection,
        geometry: &Geometry,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
        samples: u32,
        caster: bool,
    ) -> io::Result<Self> {
        let source = &selection.effect.source;
        let technique = &source.techniques[selection.index];
        let compiled = selection
            .effect
            .compiled(selection.index)
            .map_err(io::Error::other)?;
        let mut state = gpu::State::authored(source, technique).map_err(io::Error::other)?;
        // The game binds material samplers only for physicalPS0..2.
        state.samplers.retain(|key, _| {
            (0..if caster { 1 } else { 3 }).any(|i| *key == binding(i, Stage::Pixel))
        });
        if !caster {
            state.samplers.extend(map_samplers());
        }
        let mut pass = gpu::Pass::packed(&compiled, &state, format, samples);
        pass.vertices = vertex_layouts(
            &compiled.vertex.inputs,
            geometry.uv1,
            !geometry.skin.is_empty(),
        )?;
        let raster = source.rasterizer(technique);
        pass.primitive = gpu::primitive(&raster, wgpu::PrimitiveTopology::TriangleList).0;
        let (depth, reference) =
            gpu::depth_stencil(&source.depth_stencil(technique), &raster, depth);
        pass.depth_stencil = Some(depth);
        let pipeline = cache
            .get_pass(device, selection, &state, &pass)
            .map_err(io::Error::other)?;
        super::shader_constants::scene(&pipeline).map_err(io::Error::other)?;
        Ok(Self {
            pipeline,
            selection: selection.clone(),
            reference,
        })
    }

    pub fn begin(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline.pipeline);
        pass.set_stencil_reference(self.reference);
        pass.set_blend_constant(wgpu::Color::WHITE);
    }
}

/// The game copies only three weights. The content vertex shader derives the fourth.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct SkinVertex {
    pub indices: [u8; 4],
    pub weights: [f32; 3],
}

impl SkinVertex {
    pub fn from_authored(indices: [i32; 4], weights: [f32; 4]) -> Self {
        Self {
            indices: indices.map(|index| if index < 0 { 0 } else { index as u8 }),
            weights: std::array::from_fn(|i| if indices[i] < 0 { 0.0 } else { weights[i] }),
        }
    }
}

// Trees use the same fields with unconditional UV1.
const MESH_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x2, 4 => Float32x2
];
const SKIN_ATTRIBUTES: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![5 => Uint8x4, 6 => Float32x3];

fn declaration(uv1: bool, stride: u64) -> gpu::VertexLayout {
    gpu::VertexLayout {
        stride,
        step: wgpu::VertexStepMode::Vertex,
        attributes: MESH_ATTRIBUTES[..if uv1 { 5 } else { 4 }].to_vec(),
    }
}

/// Omitted UV1 or skin inputs prevent input-layout creation.
pub fn vertex_layouts(
    inputs: &[VertexInput],
    uv1: bool,
    skinned: bool,
) -> io::Result<Vec<gpu::VertexLayout>> {
    let mut layouts = vec![gpu::VertexLayout {
        stride: if uv1 { 56 } else { 48 },
        step: wgpu::VertexStepMode::Vertex,
        attributes: Vec::new(),
    }];
    if skinned {
        layouts.push(gpu::VertexLayout {
            stride: 16,
            step: wgpu::VertexStepMode::Vertex,
            attributes: Vec::new(),
        });
    }
    for input in inputs {
        let (stream, attribute) = match input.semantic.as_str() {
            "POSITION" | "POSITION0" => (0, MESH_ATTRIBUTES[0]),
            "TEXCOORD" | "TEXCOORD0" => (0, MESH_ATTRIBUTES[1]),
            "TEXCOORD1" => (0, MESH_ATTRIBUTES[2]),
            "TEXCOORD2" => (0, MESH_ATTRIBUTES[3]),
            "TEXCOORD3" if uv1 => (0, MESH_ATTRIBUTES[4]),
            "TEXCOORD4" if skinned => (1, SKIN_ATTRIBUTES[0]),
            "TEXCOORD5" if skinned => (1, SKIN_ATTRIBUTES[1]),
            _ => {
                return Err(io::Error::other(format!(
                    "native mesh declaration cannot supply {input:?}"
                )));
            }
        };
        let (kind, width) = if attribute.format == wgpu::VertexFormat::Uint8x4 {
            (ScalarKind::Uint, 4)
        } else {
            (ScalarKind::Float, attribute.format.size() / 4)
        };
        if input.kind != kind || u64::from(input.components) > width {
            return Err(io::Error::other(format!(
                "native mesh declaration cannot supply {input:?}"
            )));
        }
        layouts[stream].attributes.push(wgpu::VertexAttribute {
            shader_location: input.location,
            ..attribute
        });
    }
    Ok(layouts)
}

/// Allocate these physical handles and bind them independently
/// of the material's authored sampler vector, shared by map meshes and particles.
pub fn map_samplers() -> BTreeMap<u32, Sampler> {
    (4..14)
        .map(|index| {
            let filter = if matches!(index, 8 | 9) {
                Filter::Point
            } else {
                Filter::Linear
            };
            let address = if matches!(index, 5 | 8..=11) {
                Address::Clamp
            } else {
                Address::Wrap
            };
            (
                binding(index, Stage::Pixel),
                Sampler {
                    name: format!("map callback slot{index}"),
                    index,
                    cube: index == 7,
                    mag: filter,
                    min: filter,
                    mip: filter,
                    u: address,
                    v: address,
                    w: Address::Wrap,
                    lod_bias: 0.0,
                    min_lod: 0,
                    max_lod: 1024,
                },
            )
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn triangle() -> installed_content::mesh::SubMesh {
        installed_content::mesh::SubMesh {
            positions: vec![[1., 2., 3.], [4., 5., 6.], [7., 8., 9.]],
            normals: vec![[10., 11., 12.]; 3],
            tangents: vec![[20., 21., 22., 23.]; 3],
            uv0: vec![[30., 31.]; 3],
            indices: vec![0, 1, 2],
            ..Default::default()
        }
    }

    #[test]
    fn absent_and_wrong_sized_streams_are_omitted_from_the_stride() {
        for count in [0, 1] {
            let mut sub = triangle();
            sub.normals.truncate(count);
            sub.tangents.truncate(count);
            sub.uv0.truncate(count);
            let geometry = Geometry::from_mesh(&sub, 1.).unwrap();
            assert_eq!(geometry.stride, 12);
            assert_eq!(geometry.vertices, sub.positions.concat());
        }
    }

    #[test]
    fn fixed_fetches_alias_next_records_and_zero_whole_elements_past_the_end() {
        let mut sub = triangle();
        sub.normals.clear();
        let geometry = Geometry::from_mesh(&sub, 1.).unwrap();
        assert_eq!(geometry.stride, 36);
        let packed = geometry.packed(false).unwrap();
        assert_eq!(&packed[3..12], &[20., 21., 22., 23., 30., 31., 4., 5., 6.]);
        assert_eq!(&packed[30..36], &[0.; 6]);
    }

    #[test]
    fn absent_uv1_and_skin_omit_their_declaration_elements() {
        let geometry = Geometry::from_mesh(&triangle(), 1.).unwrap();
        assert!(!geometry.uv1);
        assert!(geometry.skin.is_empty());
        for semantic in ["TEXCOORD3", "TEXCOORD4", "TEXCOORD5"] {
            let input = VertexInput {
                semantic: semantic.into(),
                location: 0,
                components: 2,
                kind: ScalarKind::Float,
            };
            let error = vertex_layouts(&[input], geometry.uv1, false).unwrap_err();
            assert!(error.to_string().contains(semantic));
        }
        assert_eq!(vertex_layouts(&[], false, false).unwrap().len(), 1);
    }

    #[test]
    fn wrong_sized_uv1_keeps_its_declaration_and_fetches_extra_coordinates() {
        let mut sub = triangle();
        sub.uv1 = Some(Vec::new());
        sub.uv2 = Some(vec![[40., 41.]; 3]);
        sub.uv3 = Some(vec![[50., 51.]; 3]);
        let geometry = Geometry::from_mesh(&sub, 1.).unwrap();
        assert!(geometry.uv1);
        assert_eq!(geometry.stride, 64);
        assert_eq!(&geometry.vertices[12..16], &[40., 41., 50., 51.]);
        assert_eq!(&geometry.packed(geometry.uv1).unwrap()[12..14], &[40., 41.]);
    }

    #[test]
    fn imported_geometry_preserves_local_streams_and_native_repairs() {
        let sub = installed_content::mesh::SubMesh {
            positions: vec![[1., -2., 3.], [4., 5., 6.]],
            normals: vec![[0., -0., 0.], [1., 2., 3.]],
            tangents: vec![[0., 0., 0., -0.], [2., 3., 4., -1.]],
            uv0: vec![[0.25, 0.5], [0.75, 1.]],
            uv1: Some(vec![[0.125, 0.875]; 2]),
            indices: vec![0, 1, 0],
            ..Default::default()
        };
        let geometry = Geometry::from_mesh(&sub, 0.5).unwrap();
        assert_eq!(
            geometry.vertices,
            [
                0.5, -1., 1.5, 0., 0.01, 0., 0., 0.01, 0., 0.01, 0.25, 0.5, 0.125, 0.875, 2., 2.5,
                3., 1., 2., 3., 2., 3., 4., -1., 0.75, 1., 0.125, 0.875,
            ]
        );
        assert_eq!(geometry.indices, sub.indices);
        assert!(geometry.skin.is_empty());
        let without_uv1 =
            Geometry::from_mesh(&installed_content::mesh::SubMesh { uv1: None, ..sub }, 1.).unwrap();
        assert_eq!(without_uv1.vertices.len(), 24);
        assert!(!without_uv1.uv1);
    }

    #[test]
    fn native_skin_preserves_slots_truncates_indices_and_ignores_fourth_weight() {
        let vertex = SkinVertex::from_authored([-9, 257, 2, -1], [0.7, 0.25, -0.125, 99.]);
        assert_eq!(vertex.indices, [0, 1, 2, 0]);
        assert_eq!(vertex.weights, [0., 0.25, -0.125]);
        assert_eq!(std::mem::size_of::<SkinVertex>(), 16);
        assert_eq!(&bytemuck::bytes_of(&vertex)[..4], &[0, 1, 2, 0]);
    }
    #[test]
    fn physical_map_samplers_do_not_follow_an_effects_declarations() {
        let samplers = map_samplers();
        assert_eq!(samplers.len(), 10);
        let cube = &samplers[&binding(7, Stage::Pixel)];
        assert_eq!(cube.u, Address::Wrap);
        assert_eq!(cube.min, Filter::Linear);
        assert_eq!(cube.max_lod, 1024);
        for index in [8, 9] {
            let sampler = &samplers[&binding(index, Stage::Pixel)];
            assert_eq!(sampler.min, Filter::Point);
            assert_eq!(sampler.u, Address::Clamp);
        }
        assert!(!samplers.contains_key(&binding(4, Stage::Vertex)));
    }

    /// A mesh pass asked for a visual mod's replacement that cannot be laid out (it reads
    /// COLOR0, which the native mesh declaration cannot supply), built: with the replacement's
    /// selection, the recorded effect (whose technique blends, unlike the replacement's) and
    /// the geometry.
    pub(crate) fn fallen_back_pass() -> (
        Pass,
        shader_pipeline::library::Selection,
        Arc<shader_pipeline::library::Effect>,
        Geometry,
    ) {
        use shader_pipeline::{Compiled, Shader, library::Effect};
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let input = |location, semantic: &str, components| VertexInput {
            kind: ScalarKind::Float,
            location,
            semantic: semantic.into(),
            components,
        };
        let shader = |wgsl: &str, inputs| Shader {
            wgsl: wgsl.into(),
            buffers: vec![],
            textures: vec![],
            samplers: vec![],
            sampling_pairs: vec![],
            inputs,
            clock: Default::default(),
        };
        let pixel = shader(
            "@fragment fn main() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }",
            vec![],
        );
        let program = |name: &'static [u8], text: &'static [u8]| {
            Arc::new(
                installed_content::pdxshader::program::read_with(name, &|path| {
                    Some(if path == name {
                        text.to_vec()
                    } else {
                        Vec::new()
                    })
                })
                .unwrap(),
            )
        };
        let recorded = Effect::from_compiled(
            program(
                b"recorded.shader",
                b"BlendState BlendState { BlendEnable = yes SourceBlend = src_alpha DestBlend = inv_src_alpha } Effect Up { }",
            ),
            Vec::new(),
            vec![Ok(Arc::new(Compiled {
                vertex: shader(
                    "@vertex fn main(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>) -> @builtin(position) vec4<f32> { return vec4<f32>(p.xy + uv, p.z, 1.0); }",
                    vec![input(0, "POSITION", 3), input(1, "TEXCOORD0", 2)],
                ),
                pixel: pixel.clone(),
            }))],
        );
        let replacement = Effect::replacing(
            program(b"mod.shader", b"Effect Up { }"),
            vec![Ok(Arc::new(Compiled {
                vertex: shader(
                    "@vertex fn main(@location(0) p: vec3<f32>, @location(1) c: vec4<f32>) -> @builtin(position) vec4<f32> { return vec4<f32>(p, 1.0) * c; }",
                    vec![input(0, "POSITION", 3), input(1, "COLOR0", 4)],
                ),
                pixel,
            }))],
            recorded.clone(),
        );
        let selection = replacement.select("Up").unwrap().unwrap();
        let geometry = Geometry::from_mesh(&triangle(), 1.0).unwrap();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, _queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let pass = Pass::new(
            &mut gpu::Cache::default(),
            &device,
            &selection,
            &geometry,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Depth24PlusStencil8,
            1,
        )
        .unwrap();
        (pass, selection, recorded, geometry)
    }

    /// A mesh pass whose visual mod replacement cannot be laid out is built from the recorded
    /// technique, and keeps it, so what the part draws with is the recorded one's.
    #[test]
    fn a_mesh_pass_keeps_the_recorded_technique_it_fell_back_to() {
        let (pass, asked, recorded, _) = fallen_back_pass();
        let inputs = |selection: &shader_pipeline::library::Selection| {
            selection
                .effect
                .compiled(selection.index)
                .unwrap()
                .vertex
                .inputs
                .clone()
        };
        assert!(vertex_layouts(&inputs(&asked), false, false).is_err());
        assert!(Arc::ptr_eq(&pass.selection.effect, &recorded));
        let layouts = vertex_layouts(&inputs(&pass.selection), false, false).unwrap();
        assert_eq!(layouts[0].attributes.len(), 2);
    }
}
