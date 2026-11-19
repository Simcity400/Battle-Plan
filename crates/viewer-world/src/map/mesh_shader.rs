//! Mesh input declarations, compact vertices and skin weights.
use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use installed_content_core::{
    pdxshader::Address, pdxshader::program::Filter, pdxshader::program::Sampler,
    pdxshader::program::Stage,
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
    pub fn from_mesh(sub: &installed_content_core::mesh::SubMesh, scale: f32) -> io::Result<Self> {
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
    /// What draws of this pass must agree on for their order among a list's figures to leave
    /// every pixel the same; none where no agreement makes it so ([`Order`]).
    order: Option<Order>,
}

/// What two draws must agree on for no order of them to leave another pixel: the channels they
/// write, the bias their depth is tested at, and the stencil bits they set and the value they
/// set them to. Where they do, the nearer fragment wins whichever was drawn first, the one that
/// loses leaves nothing of itself behind, and the stencil ends as the same bits whether the
/// farther draw wrote them before the nearer one hid it or never passed at all. A pass that
/// could leave another pixel whatever it is drawn beside has none of this: it blends with what
/// is there, keeps a fragment that is not the nearest, reads or accumulates a stencil, or writes
/// colour to more than the one target this test weighs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Order {
    writes: u32,
    bias: [u32; 3],
    stencil: (u32, u32),
}

#[cfg(test)]
impl Order {
    /// An order of these channels and stencil, for a test of what two draws must agree on.
    pub(crate) fn for_tests(writes: u32, stencil: (u32, u32)) -> Self {
        Self { writes, bias: [0; 3], stencil }
    }
}

impl Pass {
    /// `cache` is shared by the caller's parts, so parts with one program and state share
    /// a pipeline and consecutive draws rebind only what changed. `instanced` names the constant
    /// bindings the pass reads one block of per instance ([`gpu::Pass::instanced`]).
    #[allow(clippy::too_many_arguments, reason = "the technique, its geometry and its target")]
    pub fn new(
        cache: &mut gpu::Cache,
        device: &wgpu::Device,
        selection: &shader_pipeline::library::Selection,
        geometry: &Geometry,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
        samples: u32,
        instanced: u32,
    ) -> io::Result<Self> {
        Self::with_mode(
            cache, device, selection, geometry, format, depth, samples, false, instanced,
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
        instanced: u32,
    ) -> io::Result<Self> {
        Self::with_mode(
            cache, device, selection, geometry, format, depth, 1, true, instanced,
        )
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
        instanced: u32,
    ) -> io::Result<Self> {
        // A visual mod's replacement that cannot be drawn here falls back to the recording.
        shader_pipeline::library::with_recorded_selection(selection, |selection| {
            Self::build(
                cache, device, selection, geometry, format, depth, samples, caster, instanced,
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
        instanced: u32,
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
        pass.instanced = instanced;
        let order = order(&pass.targets, pass.depth_stencil.as_ref(), reference);
        let pipeline = cache
            .get_pass(device, selection, &state, &pass)
            .map_err(io::Error::other)?;
        super::shader_constants::scene(&pipeline).map_err(io::Error::other)?;
        Ok(Self {
            pipeline,
            selection: selection.clone(),
            reference,
            order,
        })
    }

    pub(crate) fn reference(&self) -> u32 {
        self.reference
    }

    /// What this pass's draws must agree on for their order to leave the same pixels.
    pub(crate) fn order(&self) -> Option<Order> {
        self.order
    }

    /// Dynamic mesh state; resource bindings select the executable pipeline.
    pub fn begin(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_stencil_reference(self.reference);
        pass.set_blend_constant(wgpu::Color::WHITE);
    }
}

/// What the draws of a pass must agree on for their order to leave every pixel the same, or none
/// where it keeps a fragment that is not the nearest, blends it with what is there, writes a
/// stencil a later draw reads, or writes colour to more than one target.
fn order(
    targets: &[Option<wgpu::ColorTargetState>],
    depth: Option<&wgpu::DepthStencilState>,
    reference: u32,
) -> Option<Order> {
    let depth = depth?;
    let nearest = matches!(
        depth.depth_compare,
        Some(wgpu::CompareFunction::Less) | Some(wgpu::CompareFunction::LessEqual)
    );
    if depth.depth_write_enabled != Some(true) || !nearest || targets.len() > 1 {
        return None;
    }
    let stencil = stencil(&depth.stencil, reference)?;
    let mut writes = 0;
    for target in targets.iter().flatten() {
        if target.blend.is_some() {
            return None;
        }
        writes = target.write_mask.bits();
    }
    Some(Order {
        writes,
        bias: [
            depth.bias.constant as u32,
            depth.bias.slope_scale.to_bits(),
            depth.bias.clamp.to_bits(),
        ],
        stencil,
    })
}

/// The stencil bits a pass's draw sets and the value it sets them to, or none where the order of
/// two draws could leave another stencil: one the pass itself reads, one it counts up or down
/// from what is there, or one a fragment that fails the depth test writes, which the draw after
/// it would hide.
fn stencil(state: &wgpu::StencilState, reference: u32) -> Option<(u32, u32)> {
    use wgpu::StencilOperation as O;
    let kept = |face: &wgpu::StencilFaceState| {
        face.fail_op == O::Keep && face.depth_fail_op == O::Keep && face.pass_op == O::Keep
    };
    if state.write_mask == 0 || (kept(&state.front) && kept(&state.back)) {
        return Some((0, 0));
    }
    let face = state.front;
    if face != state.back
        || face.compare != wgpu::CompareFunction::Always
        || face.fail_op != O::Keep
        || face.depth_fail_op != O::Keep
    {
        return None;
    }
    let value = match face.pass_op {
        O::Replace => reference & state.write_mask,
        O::Zero => 0,
        _ => return None,
    };
    Some((state.write_mask, value))
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

    fn triangle() -> installed_content_core::mesh::SubMesh {
        installed_content_core::mesh::SubMesh {
            positions: vec![[1., 2., 3.], [4., 5., 6.], [7., 8., 9.]],
            normals: vec![[10., 11., 12.]; 3],
            tangents: vec![[20., 21., 22., 23.]; 3],
            uv0: vec![[30., 31.]; 3],
            indices: vec![0, 1, 2],
            ..Default::default()
        }
    }

    /// The depth-stencil state of a pass that keeps the nearest fragment and nothing else.
    fn nearest() -> wgpu::DepthStencilState {
        wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth24PlusStencil8,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState {
                front: wgpu::StencilFaceState::IGNORE,
                back: wgpu::StencilFaceState::IGNORE,
                read_mask: 0xff,
                write_mask: 0,
            },
            bias: Default::default(),
        }
    }

    fn opaque() -> Option<wgpu::ColorTargetState> {
        Some(wgpu::ColorTargetState {
            format: wgpu::TextureFormat::Rgba16Float,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })
    }

    /// Only a pass whose every kept fragment is the nearest one, blended with nothing and
    /// writing no stencil, may have its draws reordered among a list's figures; and two of them
    /// only where they write the same channels at the same bias.
    #[test]
    fn draws_stand_in_any_order_only_where_no_order_of_two_fragments_leaves_another_pixel() {
        let targets = [opaque()];
        let free = order(&targets, Some(&nearest()), 0).expect("the nearest fragment alone");
        assert_eq!(order(&[], Some(&nearest()), 0).map(|o| o.writes), Some(0), "depth alone");
        assert_eq!(order(&targets, None, 0), None, "no depth test: the last fragment wins");
        let blended = [Some(wgpu::ColorTargetState {
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            ..opaque().expect("made above")
        })];
        assert_eq!(order(&blended, Some(&nearest()), 0), None);
        let held = wgpu::DepthStencilState { depth_write_enabled: Some(false), ..nearest() };
        assert_eq!(order(&targets, Some(&held), 0), None, "nothing hides what follows it");
        let equal = wgpu::DepthStencilState {
            depth_compare: Some(wgpu::CompareFunction::Always),
            ..nearest()
        };
        assert_eq!(order(&targets, Some(&equal), 0), None, "every fragment passes");
        // A stencil the pass itself reads, or counts, makes the order of two draws its own.
        let marked = |pass_op, compare| wgpu::DepthStencilState {
            stencil: wgpu::StencilState {
                front: wgpu::StencilFaceState { compare, pass_op, ..wgpu::StencilFaceState::IGNORE },
                back: wgpu::StencilFaceState { compare, pass_op, ..wgpu::StencilFaceState::IGNORE },
                read_mask: 0xff,
                write_mask: 4,
            },
            ..nearest()
        };
        use wgpu::{CompareFunction as C, StencilOperation as O};
        assert_eq!(order(&targets, Some(&marked(O::Replace, C::Equal)), 4), None, "read");
        assert_eq!(order(&targets, Some(&marked(O::IncrementClamp, C::Always)), 4), None, "counted");
        // Two passes stand in either order only where they agree: a channel one leaves is a
        // channel the other's farther fragment keeps, a bias of its own ties them, and a stencil
        // bit it sets is one the other's hidden fragment would have set differently.
        let set = order(&targets, Some(&marked(O::Replace, C::Always)), 4).expect("set, not read");
        assert_eq!(set.stencil, (4, 4), "the bit it sets and what it sets it to");
        assert_eq!(order(&targets, Some(&marked(O::Replace, C::Always)), 12), Some(set), "4 of 12");
        assert_ne!(order(&targets, Some(&marked(O::Replace, C::Always)), 0), Some(set), "cleared");
        assert_ne!(order(&targets, Some(&marked(O::Zero, C::Always)), 4), Some(set));
        assert_ne!(set, free, "another stencil bit than none");
        let partial = [Some(wgpu::ColorTargetState {
            write_mask: wgpu::ColorWrites::RED,
            ..opaque().expect("made above")
        })];
        assert_ne!(order(&partial, Some(&nearest()), 0), Some(free), "another channel written");
        assert_eq!(order(&partial, Some(&nearest()), 0), order(&partial, Some(&nearest()), 0));
        let biased = wgpu::DepthStencilState {
            bias: wgpu::DepthBiasState { constant: 1, slope_scale: 0., clamp: 0. },
            ..nearest()
        };
        assert_ne!(order(&targets, Some(&biased), 0), Some(free), "another bias");
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
        let sub = installed_content_core::mesh::SubMesh {
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
            Geometry::from_mesh(&installed_content_core::mesh::SubMesh { uv1: None, ..sub }, 1.).unwrap();
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let input = |location, semantic: &str, components| VertexInput {
            kind: ScalarKind::Float,
            location,
            semantic: semantic.into(),
            components,
        };
        let shader = |wgsl: &str, inputs| Shader {
            wgsl: wgsl.into(),
            independent_draws: false,
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
                installed_content_core::pdxshader::program::read_with(name, &|path| {
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
        let instance = shader_pipeline::gpu::instance();
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
            0,
        )
        .unwrap();
        (pass, selection, recorded, geometry)
    }
}
