//! Mesh input assembly:14224BB00 declaration,142250C70 vertices,142250A90 skin.
use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use installed_content::pdxshader::{
    Address,
    program::{Filter, Sampler, Stage},
};
use shader_pipeline::{ScalarKind, VertexInput, binding, gpu};
use std::{collections::BTreeMap, io};

/// Imported geometry stays in mesh space; actor transforms belong to b1/b2.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Geometry {
    pub vertices: Vec<f32>,
    pub skin: Vec<SkinVertex>,
    pub indices: Vec<u32>,
    pub uv1: bool,
}

impl Geometry {
    pub fn from_mesh(sub: &installed_content::mesh::SubMesh, scale: f32) -> io::Result<Self> {
        if !sub.is_drawable() {
            return Err(io::Error::other(
                "native mesh requires incomplete-stream input assembly",
            ));
        }
        let uv1 = sub.uv1.as_ref().filter(|v| v.len() == sub.vertex_count());
        let mut vertices =
            Vec::with_capacity(sub.vertex_count() * if uv1.is_some() { 14 } else { 12 });
        let direction = |mut v: [f32; 3]| {
            if v == [0.; 3] {
                v[1] = 0.01;
            }
            v
        };
        for i in 0..sub.vertex_count() {
            vertices.extend(sub.positions[i].map(|v| v * scale));
            vertices.extend(direction(sub.normals[i]));
            let [x, y, z, w] = sub.tangents[i];
            vertices.extend(direction([x, y, z]));
            vertices.push(if w == 0. { 0.01 } else { w });
            vertices.extend(sub.uv0[i]);
            if let Some(uv1) = uv1 {
                vertices.extend(uv1[i]);
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
            uv1: uv1.is_some(),
        })
    }
}

pub struct Pass {
    pub pipeline: std::sync::Arc<gpu::Pipeline>,
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

    ///142264CC0 applies the shadow material handles; its map callback does not
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
        let source = &selection.effect.source;
        let technique = &source.techniques[selection.index];
        let compiled = selection
            .effect
            .compiled(selection.index)
            .map_err(io::Error::other)?;
        let mut state = gpu::State::authored(source, technique).map_err(io::Error::other)?;
        //1422644C0 binds material samplers only for physicalPS0..2.
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
            reference,
        })
    }

    pub fn begin(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline.pipeline);
        pass.set_stencil_reference(self.reference);
        pass.set_blend_constant(wgpu::Color::WHITE);
    }
}

///142250A90 copies only three weights. The content vertex shader derives the fourth.
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

/// Declaration14224BB00. Stream strides describe the uploaded records, not reflection size.
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
        let (stream, offset, format, kind, width) = match input.semantic.as_str() {
            "POSITION" | "POSITION0" => (0, 0, wgpu::VertexFormat::Float32x3, ScalarKind::Float, 3),
            "TEXCOORD" | "TEXCOORD0" => {
                (0, 12, wgpu::VertexFormat::Float32x3, ScalarKind::Float, 3)
            }
            "TEXCOORD1" => (0, 24, wgpu::VertexFormat::Float32x4, ScalarKind::Float, 4),
            "TEXCOORD2" => (0, 40, wgpu::VertexFormat::Float32x2, ScalarKind::Float, 2),
            "TEXCOORD3" if uv1 => (0, 48, wgpu::VertexFormat::Float32x2, ScalarKind::Float, 2),
            "TEXCOORD4" if skinned => (1, 0, wgpu::VertexFormat::Uint8x4, ScalarKind::Uint, 4),
            "TEXCOORD5" if skinned => (1, 4, wgpu::VertexFormat::Float32x3, ScalarKind::Float, 3),
            _ => {
                return Err(io::Error::other(format!(
                    "native mesh declaration cannot supply {input:?}"
                )));
            }
        };
        if input.kind != kind || input.components > width {
            return Err(io::Error::other(format!(
                "native mesh declaration cannot supply {input:?}"
            )));
        }
        layouts[stream].attributes.push(wgpu::VertexAttribute {
            format,
            offset,
            shader_location: input.location,
        });
    }
    Ok(layouts)
}

///140B40890 allocates these physical handles;140B47790 binds them independently
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
mod tests {
    use super::*;
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
}
