//! The map's table frame: the stone band beyond the map's top and bottom edges.
//!
//! Above a camera height of about 2000 the frame is what the camera sees where
//! the map stops. `map/ambient_object.txt` places three entities once each and
//! never moves them:
//!
//! | entity | scale | position | world z it covers |
//! |---|---|---|---|
//! | `frame_border_entity` | 100 | `0 0 2190` | 2050.6 .. 2260.4 |
//! | `frame_border_bottom_entity` | 100 | `0 0 -140` | -209.8 .. 0.0 |
//! | `frame_border_logo_entity` | 300 | `3000 -1 2130` | 2072.9 .. 2187.2 |
//!
//! Each resolves through `gfx/entities/mapitems.asset` to a `pdxmesh` in
//! `gfx/entities/mapitems.gfx` and so to `gfx/models/border_mesh.mesh`,
//! `border_bottom_mesh.mesh` and `hoi4_logo.mesh`, whose materials all name the
//! `PdxMeshBorder` effect of `gfx/FX/pdxmesh.shader` with the `map_border_*` or
//! `map_hoi4_logo_*` texture triple. `mapframe.wgsl` is the port of that
//! effect; its head records what makes the frame unlike the map's own meshes.
//!
//! # Why the frame is exactly as wide as it is
//!
//! `border_mesh` spans 56.32 model units, which at the placement's scale of 100
//! is 5632 world units: the map's own width, once, with no tiling. The two
//! bands then reach 10 units past the camera's own limits, which
//! `CAMERA_OUTSIDE_MAP_DISTANCE_TOP` and `..._BOTTOM` (200 each) put at world z
//! 2248 and -200 — the authored geometry is deliberately a little larger than
//! anything the camera can frame, so its outer edge is never seen. Fitting the
//! camera of `pairD_out40.png` confirms both numbers from the other side: at
//! `CAMERA_MAX_HEIGHT` the fitted centre puts world z 2247 at device row 6.5
//! and z -200 at row 1145.0 of 1152, so the whole-world frame shows the band
//! and stops within a handful of pixels of where the clamp says it must.
//!
//! # Height
//!
//! The bands are modelled around y = 0, not around the water plane, and the
//! ambient-object loader adds the terrain height under the object only when the
//! placement is not `always_visible`; all three of these are, so their y is
//! taken as authored. The top band's own geometry dips to y = -31.9 and rises
//! to y = 14.7 at scale 100, which is what gives the band its lip.
//!
//! # What is not reproduced
//!
//! The placements carry a `rotation` of `0 0 0`, so this pass does not
//! interpret the file's rotation units at all; a mod that rotated an ambient
//! object would be drawn unrotated. The frame is the only ambient object drawn:
//! the other nine blocks (`ambient_wind`, `ambient_water`, `chinese_wall` and
//! the rest) are close-zoom scenery whose entities are gated off at the heights
//! this pass exists for.

use std::io;

use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use installed_content::{
    ResolvedLayers, ambient,
    image::dds,
    mesh::{self, Descriptors},
};

use super::{
    scene::{DEPTH_FORMAT, Targets, module, sampler_entry, texture_entry},
    textures::{Bound, ColorSpace, Samplers, upload},
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

// ------------------------------------------------------------------ constants

/// The descriptor files that declare the frame's entities and meshes.
pub const MAPITEMS: [&str; 2] = ["gfx/entities/mapitems.gfx", "gfx/entities/mapitems.asset"];

/// The `map/ambient_object.txt` entities this pass draws, in draw order.
///
/// The logo is listed last so it wins the depth test against the band it sits
/// on, which is also the order the file declares them in.
pub const FRAME_ENTITIES: [&str; 3] = [
    "frame_border_entity",
    "frame_border_bottom_entity",
    "frame_border_logo_entity",
];

/// Bytes one [`FrameVertex`] occupies.
const VERTEX_STRIDE: u64 = 48;
/// Bytes the shader reads from a wrap copy's uniform slot.
const SLOT_SIZE: u64 = 16;
/// Bytes one slot occupies, rounded up to wgpu's default uniform offset
/// alignment so every slot can be bound at a static offset.
const SLOT_STRIDE: u64 = 256;
/// World-wrap copies, as the border and tree passes count them.
const COPIES: usize = 3;

// --------------------------------------------------------------------- assets

/// One placed frame mesh and the textures its material names.
pub struct FramePiece {
    /// The `map/ambient_object.txt` entity name.
    pub entity: String,
    /// The `.mesh` the entity resolved to.
    pub path: String,
    /// `ambient object scale * entity scale * pdxmesh scale`.
    pub scale: f32,
    /// The placement's world position.
    pub position: [f32; 3],
    pub mesh: mesh::Mesh,
    pub diffuse: dds::Texture,
    pub normal: dds::Texture,
    pub specular: dds::Texture,
}

/// Everything the map-frame pass reads from installed content.
pub struct FrameAssets {
    pub pieces: Vec<FramePiece>,
}

impl FrameAssets {
    /// Reads the frame's placements, meshes and textures.
    ///
    /// `map/ambient_object.txt` is covered content and is hash-verified;
    /// `gfx/entities` and `gfx/models` are outside the coverage policy
    /// ([archived rendering notes](#) section 4), so the descriptors, the meshes and their
    /// textures go through [`ResolvedLayers::read_uncovered`] and are not
    /// verified. The caller states that limitation once.
    ///
    /// A frame piece whose entity, mesh or texture will not read is reported as
    /// a warning and dropped rather than refused: a missing band must not stop
    /// a replay opening, and every other piece still draws.
    pub fn load(layers: &ResolvedLayers, warnings: &mut Vec<String>) -> io::Result<Self> {
        let placements = ambient::read(layers)?;
        let descriptors = mesh::read_descriptors(layers, &MAPITEMS)?;
        let mut pieces = Vec::new();
        let mut missing = Vec::new();
        for entity in FRAME_ENTITIES {
            let Some(placement) = placements.iter().find(|block| block.entity == entity) else {
                missing.push(format!("{entity} (not placed in {})", ambient::PATH));
                continue;
            };
            for object in &placement.objects {
                match load_piece(
                    layers,
                    &descriptors,
                    entity,
                    placement.scale,
                    object.position,
                ) {
                    Ok(piece) => pieces.push(piece),
                    Err(error) => missing.push(format!("{entity} ({error})")),
                }
            }
        }
        if !missing.is_empty() {
            warnings.push(format!(
                "The map's table frame is incomplete: {}. The map's top or bottom edge is drawn \
                 bare where the band is missing.",
                missing.join(", ")
            ));
        }
        Ok(Self { pieces })
    }
}

fn load_piece(
    layers: &ResolvedLayers,
    descriptors: &Descriptors,
    entity: &str,
    object_scale: f32,
    position: [f32; 3],
) -> io::Result<FramePiece> {
    let resolved = descriptors.resolve(entity).ok_or_else(|| {
        invalid(format!(
            "{} declare no entity or pdxmesh \"{entity}\"",
            MAPITEMS.join(" and ")
        ))
    })?;
    let path = resolved.definition.file.clone();
    let decoded = mesh::read(&layers.read_uncovered(&path)?)
        .map_err(|error| invalid(format!("{path}: {error}")))?;
    let material = decoded
        .shapes
        .iter()
        .flat_map(|shape| shape.meshes.iter())
        .find_map(|sub| sub.material.as_ref())
        .ok_or_else(|| invalid(format!("{path} declares no [material]")))?;
    // A material's texture names are relative to the `.mesh`'s own directory
    // (R3 section 1.4).
    let directory = path.rsplit_once('/').map_or("", |(head, _)| head);
    let texture = |file: &str| -> io::Result<dds::Texture> {
        if file.is_empty() {
            return Err(invalid(format!("{path} names no texture")));
        }
        let full = format!("{directory}/{file}");
        dds::decode(&layers.read_uncovered(&full)?)
            .map_err(|error| invalid(format!("{full}: {error}")))
    };
    Ok(FramePiece {
        diffuse: texture(&material.diffuse)?,
        normal: texture(&material.normal)?,
        specular: texture(&material.specular)?,
        entity: entity.to_owned(),
        path,
        scale: object_scale * resolved.scale,
        position,
        mesh: decoded,
    })
}

// ------------------------------------------------------------------- geometry

/// One frame vertex, in the order `VS_INPUT_PDXMESHSTANDARD` declares.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct FrameVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tangent: [f32; 4],
    pub uv0: [f32; 2],
}

/// The frame's vertices and indices, already in world space.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameGeometry {
    pub vertices: Vec<FrameVertex>,
    pub indices: Vec<u32>,
    /// One index range per piece, in [`FrameAssets::pieces`] order.
    pub ranges: Vec<std::ops::Range<u32>>,
    /// The world `z` the geometry spans, for the module's own record.
    pub z_span: [f32; 2],
}

/// Bakes each piece's LOD 0 into world space.
///
/// The world matrix is a uniform scale and a translation, so the positions are
/// scaled and offset here and the normals and tangents pass through unchanged;
/// `mapframe.wgsl` then only adds the wrap copy's x offset. LOD 0 is the only
/// level used: `frame_border_mesh` declares `cull_distance = 99999.0` and the
/// placement is `always_visible`, so the engine never drops the frame to a
/// coarser level however far the camera is.
pub fn geometry(assets: &FrameAssets) -> FrameGeometry {
    let mut out = FrameGeometry {
        z_span: [f32::MAX, f32::MIN],
        ..Default::default()
    };
    for piece in &assets.pieces {
        let start = out.indices.len() as u32;
        for shape in piece.mesh.lods().first().into_iter().flatten() {
            for sub in &shape.meshes {
                if !sub.is_drawable() {
                    continue;
                }
                let base = out.vertices.len() as u32;
                for index in 0..sub.vertex_count() {
                    let p = sub.positions[index];
                    let position = [
                        p[0] * piece.scale + piece.position[0],
                        p[1] * piece.scale + piece.position[1],
                        p[2] * piece.scale + piece.position[2],
                    ];
                    out.z_span[0] = out.z_span[0].min(position[2]);
                    out.z_span[1] = out.z_span[1].max(position[2]);
                    out.vertices.push(FrameVertex {
                        position,
                        normal: sub.normals[index],
                        tangent: sub.tangents[index],
                        uv0: sub.uv0[index],
                    });
                }
                out.indices
                    .extend(sub.indices.iter().map(|index| base + index));
            }
        }
        out.ranges.push(start..out.indices.len() as u32);
    }
    if out.vertices.is_empty() {
        out.z_span = [0.0, 0.0];
    }
    out
}

// ----------------------------------------------------------------------- pass

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct Constants {
    world_offset_x: f32,
    padding: [f32; 3],
}

/// What a frame needs beyond the globals.
pub struct MapFrameFrame<'a> {
    /// The visible world-wrap x offsets, as `border_pass::wrap_offsets` gives
    /// them.
    pub offsets: &'a [f32],
}

/// The map-frame pass.
pub struct MapFramePass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    constants: wgpu::Buffer,
    /// One texture triple per piece, in [`FrameAssets::pieces`] order.
    textures: Vec<[Bound; 3]>,
    /// `piece * COPIES + copy`.
    groups: Vec<wgpu::BindGroup>,
    vertices: Option<wgpu::Buffer>,
    indices: Option<wgpu::Buffer>,
    ranges: Vec<std::ops::Range<u32>>,
}

impl MapFramePass {
    /// Builds the pipeline and uploads the frame's textures.
    pub fn new(
        device: &wgpu::Device,
        samples: u32,
        queue: &wgpu::Queue,
        globals: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
        assets: &FrameAssets,
    ) -> io::Result<Self> {
        let mut entries = (0..3)
            .map(|binding| {
                texture_entry(
                    binding,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::TextureViewDimension::D2,
                )
            })
            .collect::<Vec<_>>();
        entries.push(sampler_entry(3));
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 4,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(SLOT_SIZE),
            },
            count: None,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map frame inputs"),
            entries: &entries,
        });
        let module = module(device, "map frame", include_str!("mapframe.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("map frame"),
            bind_group_layouts: &[Some(globals), Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("map frame"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("map_frame_vertex"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: VERTEX_STRIDE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 12,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 24,
                            shader_location: 2,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 40,
                            shader_location: 3,
                        },
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState {
                // The shipped `PdxMeshBorder` declares no `RasterizerState`,
                // and the meshes are authored with the opposite winding to
                // wgpu's counter-clockwise front face, so culling the back
                // culls the whole band. The bands are closed solids seen only
                // from above, where the depth test already picks the top
                // surface, so nothing is left for a cull to do.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("map_frame_fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // `BlendState { BlendEnable = no  AlphaTest = yes }`: the
                    // frame replaces what is under it, and the alpha it writes
                    // is the shader's own zero.
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let constants = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("map frame wrap offsets"),
            size: SLOT_STRIDE * COPIES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut textures = Vec::new();
        for piece in &assets.pieces {
            textures.push([
                upload(
                    device,
                    queue,
                    &format!("{} diffuse", piece.entity),
                    &piece.diffuse,
                    ColorSpace::Srgb,
                )?,
                upload(
                    device,
                    queue,
                    &format!("{} normal", piece.entity),
                    &piece.normal,
                    ColorSpace::Linear,
                )?,
                upload(
                    device,
                    queue,
                    &format!("{} specular", piece.entity),
                    &piece.specular,
                    ColorSpace::Linear,
                )?,
            ]);
        }
        Ok(Self {
            pipeline,
            layout,
            constants,
            textures,
            groups: Vec::new(),
            vertices: None,
            indices: None,
            ranges: Vec::new(),
        })
    }

    /// Rebuilds the bind groups. The pass samples no scene texture, so this is
    /// needed once, after [`Self::new`].
    pub fn rebind(&mut self, device: &wgpu::Device, samplers: &Samplers) {
        let mut groups = Vec::with_capacity(self.textures.len() * COPIES);
        for piece in &self.textures {
            for copy in 0..COPIES {
                groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("map frame inputs"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&piece[0].view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&piece[1].view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&piece[2].view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::Sampler(&samplers.linear_wrap),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &self.constants,
                                offset: SLOT_STRIDE * copy as u64,
                                size: wgpu::BufferSize::new(SLOT_SIZE),
                            }),
                        },
                    ],
                }));
            }
        }
        self.groups = groups;
    }

    /// Uploads the baked world-space geometry.
    pub fn upload(&mut self, device: &wgpu::Device, geometry: FrameGeometry) {
        use wgpu::util::DeviceExt;
        if geometry.vertices.is_empty() || geometry.indices.is_empty() {
            self.vertices = None;
            self.indices = None;
            self.ranges.clear();
            return;
        }
        self.vertices = Some(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("map frame vertices"),
                contents: bytemuck::cast_slice(&geometry.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
        );
        self.indices = Some(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("map frame indices"),
                contents: bytemuck::cast_slice(&geometry.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
        );
        self.ranges = geometry.ranges;
    }

    /// Draws the frame, once per visible world-wrap copy.
    pub fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        targets: &Targets,
        globals: &wgpu::BindGroup,
        frame: MapFrameFrame<'_>,
    ) {
        let (Some(vertices), Some(indices)) = (&self.vertices, &self.indices) else {
            return;
        };
        if self.groups.is_empty() {
            return;
        }
        let copies = frame.offsets.len().min(COPIES);
        let mut bytes = Vec::with_capacity(COPIES * SLOT_STRIDE as usize);
        for copy in 0..COPIES {
            let slot = Constants {
                world_offset_x: frame.offsets.get(copy).copied().unwrap_or(0.0),
                padding: [0.0; 3],
            };
            bytes.extend_from_slice(bytemuck::bytes_of(&slot));
            bytes.resize(bytes.len() + SLOT_STRIDE as usize - SLOT_SIZE as usize, 0);
        }
        queue.write_buffer(&self.constants, 0, &bytes);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("map frame"),
            color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Load))],
            depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Load)),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, globals, &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
        for (piece, range) in self.ranges.iter().enumerate() {
            if range.is_empty() {
                continue;
            }
            for copy in 0..copies {
                pass.set_bind_group(1, &self.groups[piece * COPIES + copy], &[]);
                pass.draw_indexed(range.clone(), 0, 0..1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use installed_content::{
        image::Rgba8Image,
        mesh::{Mesh, Shape, SubMesh},
    };

    /// A one-texel stand-in: [`geometry`] never looks at a piece's textures.
    fn flat() -> dds::Texture {
        dds::Texture::from_rgba8(Rgba8Image {
            width: 1,
            height: 1,
            pixels: vec![255; 4],
        })
    }

    fn piece(entity: &str, scale: f32, position: [f32; 3]) -> FramePiece {
        let sub = SubMesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            normals: vec![[0.0, 1.0, 0.0]; 3],
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; 3],
            uv0: vec![[0.0, 0.0]; 3],
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        FramePiece {
            entity: entity.to_owned(),
            path: format!("gfx/models/{entity}.mesh"),
            scale,
            position,
            mesh: Mesh {
                shapes: vec![Shape {
                    name: entity.to_owned(),
                    lod: None,
                    meshes: vec![sub],
                }],
                ..Default::default()
            },
            diffuse: flat(),
            normal: flat(),
            specular: flat(),
        }
    }

    #[test]
    fn the_placement_scale_and_position_are_baked_into_world_space() {
        let assets = FrameAssets {
            pieces: vec![piece("frame_border_entity", 100.0, [0.0, 0.0, 2190.0])],
        };
        let geometry = geometry(&assets);
        assert_eq!(geometry.vertices.len(), 3);
        assert_eq!(geometry.vertices[0].position, [0.0, 0.0, 2190.0]);
        // 1 model unit at scale 100 is 100 world units, which is what makes the
        // 56.32-unit band exactly one map width across.
        assert_eq!(geometry.vertices[1].position, [100.0, 0.0, 2190.0]);
        assert_eq!(geometry.vertices[2].position, [0.0, 0.0, 2290.0]);
        // A uniform scale leaves the normal and tangent alone.
        assert_eq!(geometry.vertices[0].normal, [0.0, 1.0, 0.0]);
        assert_eq!(geometry.vertices[0].tangent, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(geometry.ranges, vec![0..3]);
        assert_eq!(geometry.z_span, [2190.0, 2290.0]);
    }

    #[test]
    fn every_piece_gets_its_own_index_range() {
        let assets = FrameAssets {
            pieces: vec![
                piece("frame_border_entity", 100.0, [0.0, 0.0, 2190.0]),
                piece("frame_border_bottom_entity", 100.0, [0.0, 0.0, -140.0]),
            ],
        };
        let geometry = geometry(&assets);
        assert_eq!(geometry.ranges, vec![0..3, 3..6]);
        // The second piece's indices are rebased onto its own vertices.
        assert_eq!(&geometry.indices[3..6], &[3, 4, 5]);
        assert_eq!(geometry.z_span, [-140.0, 2290.0]);
    }

    #[test]
    fn a_piece_with_no_drawable_geometry_contributes_an_empty_range() {
        let mut only = piece("frame_border_logo_entity", 300.0, [3000.0, -1.0, 2130.0]);
        only.mesh.shapes[0].meshes[0].normals.clear();
        let geometry = geometry(&FrameAssets { pieces: vec![only] });
        assert!(geometry.vertices.is_empty());
        assert_eq!(geometry.ranges, vec![0..0]);
        assert_eq!(geometry.z_span, [0.0, 0.0]);
    }
}
