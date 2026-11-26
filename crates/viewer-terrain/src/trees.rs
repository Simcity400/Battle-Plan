use content_reader_mesh::mesh::MeshProducts as _;
use std::cell::Cell;
use std::cell::RefCell;
use std::sync::Arc;
use std::{collections::BTreeMap, io, ops::Range};
use viewer_gpu::province_admission::admits_box;
use viewer_gpu::textures::load_file;
use viewer_gpu::textures::load_image;

use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use content_reader_mesh::mesh::{self, Descriptors};
use content_reader_image::{IndexedImage, SharedRgba8Image as Rgba8Image, dds, native::Decode};

use viewer_scene_base::shadow::{BUFFER_FORMAT, DEPTH_MAP_FORMAT};
use viewer_gpu::{
    camera::{Camera, MapDefines, WATER_HEIGHT},
    gpu_contract::{DEPTH_FORMAT, Targets, buffer},
    province_map::HeightField,
    textures::{Bound, ColorSpace, data_texture, upload},
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}


pub const SEASON_COLUMNS: usize = 8;
pub fn tree_fade(defines: &MapDefines, height: f32) -> f32 {
    let [near, far] = defines.tree_fade;
    if height > far {
        return 0.0;
    }
    1.0 - (height.max(near) - near) / (far - near)
}

pub fn mapobject_name(index: u8) -> String {
    format!("mapobject_{index}")
}


#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct TreeInstance {
    pub position: [f32; 3],
    pub rotation: f32,
    pub slope: [f32; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct TreeBucket {
    pub palette: u8,
    pub tile: [u32; 2],
    pub pitch: [u32; 2],
    pub range: Range<u32>,
}

impl TreeBucket {
    fn x_span(&self) -> (f32, f32) {
        let low = self.tile[0] * self.pitch[0];
        (low as f32, (low + self.pitch[0]) as f32)
    }

    fn z_span(&self) -> (f32, f32) {
        let low = self.tile[1] * self.pitch[1];
        (low as f32, (low + self.pitch[1]) as f32)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeField {
    pub instances: Vec<TreeInstance>,
    pub buckets: Vec<TreeBucket>,
    pub cell: [f32; 2],
    pub palettes: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct TreeImage<'a> {
    pub width: u32,
    pub height: u32,
    pub indices: &'a [u8],
}

impl TreeImage<'_> {
    pub fn validate(&self) -> io::Result<()> {
        if self.width == 0 || self.height == 0 {
            return Err(invalid("the tree bitmap needs a nonzero width and height"));
        }
        let count = (self.width as usize)
            .checked_mul(self.height as usize)
            .ok_or_else(|| invalid("the tree bitmap's pixel count overflows"))?;
        if self.indices.len() != count {
            return Err(invalid(format!(
                "the tree bitmap holds {} indices for {} x {} pixels",
                self.indices.len(),
                self.width,
                self.height
            )));
        }
        Ok(())
    }
}

pub fn rotation_for(index: u32) -> f32 {
    let counter = 0xae66_136bu32.wrapping_add(index);
    let mut state = 0xd4b5_37a3u32.wrapping_sub(counter.wrapping_mul(0x4ad6_85b3));
    state = (state ^ (state >> 8)).wrapping_add(0x68e3_1da4);
    state ^= state << 8;
    state = state.wrapping_mul(0x1b56_c4e9);
    state ^= state >> 8;
    state = state.wrapping_mul(0x92d6_8ca2);
    state ^= state >> 8;
    ((state & 0x7fff_ffff) % 360) as f32 * (std::f32::consts::PI / 180.0)
}

fn tree_height(heights: &HeightField<'_>, x: f32, z: f32) -> f32 {
    let x = (x + 0.5) - 0.5;
    let z = (z + 0.5) - 0.5;
    let ix = (x as i32).clamp(0, heights.width as i32 - 2) as usize;
    let iz = (z as i32).clamp(0, heights.height as i32 - 2) as usize;
    let fx = x - ix as f32;
    let fz = z - iz as f32;
    let sample = |column: usize, row: usize| {
        f32::from(
            heights.values[(heights.height as usize - 1 - row) * heights.width as usize + column],
        )
    };
    let a = sample(ix, iz);
    let b = sample(ix + 1, iz);
    let c = sample(ix, iz + 1);
    let d = sample(ix + 1, iz + 1);
    let low = (b - a) * fx + a;
    let high = (d - c) * fx + c;
    ((high - low) * fz + low) * 0.1
}

pub fn generate(image: &TreeImage<'_>, heights: &HeightField<'_>) -> io::Result<TreeField> {
    let map = [heights.width as f32, heights.height as f32];
    image.validate()?;
    heights.validate()?;
    if heights.width < 2 || heights.height < 2 {
        return Err(invalid(
            "tree height sampling needs at least a 2 x 2 height field",
        ));
    }
    let tiles = [heights.width >> 8, heights.height >> 8];
    if tiles.contains(&0) {
        return Err(invalid(format!(
            "tree partitions require map dimensions at least 256; got {} x {}",
            heights.width, heights.height
        )));
    }
    let pitch = [heights.width / tiles[0], heights.height / tiles[1]];
    let cell = [map[0] / image.width as f32, map[1] / image.height as f32];
    let half = [cell[0] * 0.5, cell[1] * 0.5];
    let width = image.width as usize;
    let mut by_bucket: BTreeMap<(u8, u32, u32), Vec<TreeInstance>> = BTreeMap::new();
    let mut counter = 0u32;
    for k in 0..image.height as usize {
        let rb = image.height as usize - 1 - k;
        let z = rb as f32 * cell[1];
        for tx in 0..width {
            let palette = image.indices[k * width + tx];
            if palette == 0 {
                continue;
            }
            let mut x = tx as f32 * cell[0];
            let tile_x = (x / pitch[0] as f32) as u32;
            let tile_z = (z / pitch[1] as f32) as u32;
            let linear = tile_z * tiles[0] + tile_x;
            if linear >= tiles[0] * tiles[1] {
                return Err(invalid(format!(
                    "tree palette {palette} pixel ({tx}, {k}) addresses partition {linear} outside the {} x {} native visibility grid",
                    tiles[0], tiles[1]
                )));
            }
            if k % 2 == 1 {
                x += half[0];
            }
            let y = tree_height(heights, x, z);
            let corner = |dx: f32, dz: f32| {
                tree_height(
                    heights,
                    (x + dx).clamp(0.0, map[0] - 1.0),
                    (z + dz).clamp(0.0, map[1] - 1.0),
                )
            };
            let (a, b, c, d) = (
                corner(-half[0], -half[1]),
                corner(half[0], -half[1]),
                corner(-half[0], half[1]),
                corner(half[0], half[1]),
            );
            let instance = TreeInstance {
                position: [x, y, z],
                rotation: rotation_for(counter),
                slope: [
                    ((b - a) + (d - c)) / (cell[0] + cell[0]),
                    ((c - a) + (d - b)) / (cell[1] + cell[1]),
                ],
            };
            counter = counter.wrapping_add(1);
            by_bucket
                .entry((palette, linear / tiles[0], linear % tiles[0]))
                .or_default()
                .push(instance);
        }
    }

    let mut field = TreeField {
        cell,
        ..TreeField::default()
    };
    for ((palette, tile_z, tile_x), instances) in by_bucket {
        let start = field.instances.len() as u32;
        field.instances.extend_from_slice(&instances);
        field.buckets.push(TreeBucket {
            palette,
            tile: [tile_x, tile_z],
            pitch,
            range: start..field.instances.len() as u32,
        });
        if field.palettes.last() != Some(&palette) {
            field.palettes.push(palette);
        }
    }
    field.palettes.sort_unstable();
    field.palettes.dedup();
    Ok(field)
}

#[cfg(test)]
pub fn lod_for_distance(lod_distances: &[f32], distance: f32, shapes: usize) -> usize {
    lod_for_squared_distance(lod_distances, distance * distance, shapes)
}

fn lod_for_squared_distance(lod_distances: &[f32], squared: f32, shapes: usize) -> usize {
    let mut lod = 0;
    for (index, &threshold) in lod_distances.iter().enumerate() {
        if squared < threshold * threshold {
            break;
        }
        lod = index;
    }
    lod.min(shapes.saturating_sub(1))
}

#[cfg(test)]
pub fn bucket_distance(bucket: &TreeBucket, offset: f32, camera: [f32; 3]) -> f32 {
    bucket_distance_squared(bucket, offset, camera).sqrt()
}

fn bucket_distance_squared(bucket: &TreeBucket, offset: f32, camera: [f32; 3]) -> f32 {
    let (low_x, high_x) = bucket.x_span();
    let (low_z, high_z) = bucket.z_span();
    let x = camera[0] - offset;
    let dx = x - x.clamp(low_x, high_x);
    let dy = camera[1] - WATER_HEIGHT;
    let dz = camera[2] - camera[2].clamp(low_z, high_z);
    dy * dy + dx * dx + dz * dz
}

#[cfg(test)]
pub fn bucket_lod(
    bucket: &TreeBucket,
    offset: f32,
    lod_distances: &[f32],
    shapes: usize,
    camera: [f32; 3],
) -> usize {
    lod_for_distance(
        lod_distances,
        bucket_distance(bucket, offset, camera),
        shapes,
    )
}


pub struct TreeMeshAssets {
    pub name: String,
    pub path: String,
    pub lod_distances: Vec<f32>,
    pub lods: Vec<Option<TreeLod>>,
    pub scale: f32,
}

pub struct TreeLod {
    pub geometry: viewer_scene_base::mesh_shader::Geometry,
    pub diffuse: dds::Texture,
    pub normal: dds::Texture,
}

pub struct TreeAssets {
    pub meshes: Vec<(u8, TreeMeshAssets)>,
    pub tint: Rgba8Image,
    pub season: Rgba8Image,
    pub mask: TreeMask,
}

impl TreeAssets {
    pub fn load(
        layers: &ResolvedLayers,
        decode: Decode,
        palettes: &[u8],
        rivers: &IndexedImage,
        warnings: &mut Vec<String>,
    ) -> io::Result<Self> {
        let roster = layers.descriptor_roster()?;
        if !roster.unreadable.is_empty() {
            warnings.push(format!(
                "Tree descriptor roster has {} unreadable files: {}",
                roster.unreadable.len(),
                roster.unreadable.join(", ")
            ));
        }
        let mut meshes = Vec::new();
        let size = [rivers.width, rivers.height];
        let mut materials = viewer_scene_base::mesh_effect::Materials::new(layers, size, decode);
        let below = layers.without_overlays();
        let below_roster = layers.recorded_descriptor_roster()?;
        let recorded = below.as_ref().zip(below_roster.as_deref());
        let mut recorded_materials = None;
        let mut missing = Vec::new();
        for &palette in palettes {
            let kind = layers.artwork_item(
                &format!("tree kind {palette}"),
                recorded.is_some(),
                |from_recording| match recorded {
                    Some((stack, roster)) if from_recording => {
                        let materials = recorded_materials.get_or_insert_with(|| {
                            viewer_scene_base::mesh_effect::Materials::new(stack, size, decode)
                        });
                        load_mesh(stack, decode, &roster.descriptors, palette, materials)
                    }
                    _ => load_mesh(layers, decode, &roster.descriptors, palette, &mut materials),
                },
            );
            match kind {
                Ok(assets) => meshes.push((palette, assets)),
                Err(error) => missing.push(format!("index {palette} ({error})")),
            }
        }
        if !missing.is_empty() {
            warnings.push(format!(
                "{} tree kinds unresolved for map/trees.bmp palette {}; trees of those kinds are not \
                 drawn.",
                missing.len(),
                missing.join(", ")
            ));
        }
        let image = |name: &str| -> io::Result<Rgba8Image> {
            let path = format!("map/terrain/{name}");
            layers.artwork(&path, |stack| {
                load_image(&path, &|request| stack.read_path(request), decode)
            })
        };
        let season = image("Tree_season.bmp")?;
        if season.width as usize != SEASON_COLUMNS {
            warnings.push(format!(
                "map/terrain/Tree_season.bmp is {} columns wide, not the {SEASON_COLUMNS} the                  engine indexes by season; the tree season tint may be taken from the wrong                  column.",
                season.width
            ));
        }
        Ok(Self {
            meshes,
            tint: image("Tree_tint.bmp")?,
            season,
            mask: tree_mask(rivers)?,
        })
    }
}

pub struct TreeMask {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

const RIVER_INDEX_LIMIT: u8 = 254;

pub fn tree_mask(rivers: &IndexedImage) -> io::Result<TreeMask> {
    let (width, height) = (rivers.width as usize, rivers.height as usize);
    if width < 2 || height < 2 || rivers.indices.len() != width * height {
        return Err(invalid(format!(
            "the river bitmap is {width} x {height} with {} indices and no tree mask can be \
             generated from it",
            rivers.indices.len()
        )));
    }
    let (half_width, half_height) = (width / 2, height / 2);
    let mut data = vec![0u8; half_width * half_height];
    for row in 0..height - 1 {
        for column in 0..width - 1 {
            let top = row * width + column;
            let sum = [top, top + 1, top + width, top + width + 1]
                .into_iter()
                .filter(|index| rivers.indices[*index] < RIVER_INDEX_LIMIT)
                .count();
            data[(row >> 1) * half_width + (column >> 1)] = (sum * 255 / 4) as u8;
        }
    }
    Ok(TreeMask {
        width: half_width as u32,
        height: half_height as u32,
        data,
    })
}

fn load_mesh(
    layers: &ResolvedLayers,
    decode: Decode,
    descriptors: &Descriptors,
    palette: u8,
    materials: &mut viewer_scene_base::mesh_effect::Materials,
) -> io::Result<TreeMeshAssets> {
    let name = mapobject_name(palette);
    let definition = tree_definition(descriptors, palette)?;
    let path = definition.file.clone();
    let scale = definition.scale;
    let bytes = layers.read_uncovered(&path)?;
    let decoded = mesh::read(&bytes).map_err(|error| invalid(format!("{path}: {error}")))?;
    let directory = path.rsplit_once('/').map_or("", |(head, _)| head);
    let index = layers.texture_index()?;
    let texture = |file: &str| -> io::Result<dds::Texture> {
        if file.is_empty() {
            return Err(invalid(format!("{path} names no texture")));
        }
        let full = index.resolve(directory, file)?;
        load_file(&full, &|path| layers.read_uncovered_path(path), decode)
    };
    let mut lods = Vec::new();
    for shapes in decoded.lods() {
        let selected = first_tree_record(&shapes, definition, |material, sub| {
            materials.admitted(material, sub)
        })?;
        let lod = selected
            .map(|(sub, material)| {
                Ok::<_, io::Error>(TreeLod {
                    geometry: viewer_scene_base::mesh_shader::Geometry::from_mesh(sub, scale)
                        .map_err(|error| invalid(format!("{path}: {error}")))?,
                    diffuse: texture(&material.diffuse)?,
                    normal: texture(&material.normal)?,
                })
            })
            .transpose()?;
        lods.push(lod);
    }
    Ok(TreeMeshAssets {
        name,
        path,
        scale,
        lod_distances: decoded.lod_distances,
        lods,
    })
}

fn first_tree_record<'a>(
    shapes: &[&'a mesh::Shape],
    definition: &mesh::MeshDefinition,
    mut admitted: impl FnMut(&mesh::Material, &mesh::SubMesh) -> io::Result<bool>,
) -> io::Result<Option<(&'a mesh::SubMesh, mesh::Material)>> {
    for shape in shapes {
        for sub in &shape.meshes {
            if viewer_scene_base::mesh_effect::candidate(sub) {
                let material = definition.material(&shape.name, sub);
                if admitted(&material, sub)? {
                    return Ok(Some((sub, material)));
                }
            }
        }
    }
    Ok(None)
}

fn tree_definition(descriptors: &Descriptors, palette: u8) -> io::Result<&mesh::MeshDefinition> {
    let name = mapobject_name(palette);
    descriptors
        .meshes
        .get(&name)
        .ok_or_else(|| invalid(format!("content declares no pdxmesh \"{name}\"")))
}


#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct TreeVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tangent: [f32; 4],
    pub uv0: [f32; 2],
    pub uv1: [f32; 2],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeGeometry {
    pub vertices: Vec<TreeVertex>,
    pub indices: Vec<u32>,
    pub ranges: BTreeMap<(u8, usize), Range<u32>>,
    pub lods: BTreeMap<u8, (Vec<f32>, usize)>,
}

pub fn geometry(assets: &TreeAssets) -> io::Result<TreeGeometry> {
    let mut out = TreeGeometry::default();
    for (palette, tree) in &assets.meshes {
        out.lods
            .insert(*palette, (tree.lod_distances.clone(), tree.lods.len()));
        for (lod, selected) in tree.lods.iter().enumerate() {
            let start = out.indices.len() as u32;
            if let Some(selected) = selected {
                let packed = selected
                    .geometry
                    .packed(true)
                    .map_err(|error| invalid(format!("{} LOD {lod}: {error}", tree.path)))?;
                let base = out.vertices.len() as u32;
                for record in packed.chunks_exact(14) {
                    out.vertices.push(TreeVertex {
                        position: record[..3].try_into().unwrap(),
                        normal: record[3..6].try_into().unwrap(),
                        tangent: record[6..10].try_into().unwrap(),
                        uv0: record[10..12].try_into().unwrap(),
                        uv1: record[12..14].try_into().unwrap(),
                    });
                }
                out.indices
                    .extend(selected.geometry.indices.iter().map(|index| base + index));
            }
            out.ranges
                .insert((*palette, lod), start..out.indices.len() as u32);
        }
    }
    Ok(out)
}


pub struct ReceiverInputs<'a> {
    pub frames: &'a [&'a [u8]],
    pub shadow_matrix: [[f32; 4]; 4],
    pub textures: [(u32, &'a wgpu::TextureView); 11],
}

pub struct LitInputs<'a> {
    pub fog: bool,
    pub pulses: bool,
    pub retained: Option<&'a RefCell<viewer_scene_base::shader_scene::State>>,
    pub view: usize,
    pub frames: &'a [&'a [u8]],
    pub retained_pixel_b2: viewer_scene_base::shader_constants::WorldPixelConstants,
    pub textures: [(u32, &'a wgpu::TextureView); 9],
    pub high: bool,
}

#[derive(Clone, Copy)]
pub struct TreeFrame<'a> {
    pub camera: Camera,
    pub offsets: &'a [f32],
    pub planes: viewer_gpu::province_admission::Planes,
}

type Batch = (usize, Range<u32>, Range<u32>);

#[derive(Default, PartialEq)]
struct Admitted {
    key: Option<(Camera, viewer_gpu::province_admission::Planes, Vec<f32>)>,
    copies: Vec<Vec<Batch>>,
}

pub struct TreePass {
    compiled_caster: viewer_scene_base::tree_caster::Caster,
    compiled_receiver: super::tree_receiver::Receiver,
    compiled_lit: RefCell<Option<(bool, super::tree_shader::Lit)>>,
    effect: Arc<shader_pipeline::library::Effect>,
    samples: u32,
    format: wgpu::TextureFormat,
    null: Bound,
    tint: Bound,
    season: Bound,
    mask: Bound,
    diffuse: Vec<Bound>,
    normal: Vec<Bound>,
    materials: Vec<(u8, usize)>,
    vertices: Option<wgpu::Buffer>,
    indices: Option<wgpu::Buffer>,
    instances: Option<wgpu::Buffer>,
    instance_bytes: Arc<[u8]>,
    geometry: TreeGeometry,
    buckets: Vec<TreeBucket>,
    admitted: RefCell<Admitted>,
    drawn: Cell<u32>,
    drawn_buckets: Cell<u32>,
    caster: viewer_scene_base::shadow::CasterId,
}

impl TreePass {
    pub fn texel_reads(&self, high: bool, visit: &mut dyn FnMut(&str)) {
        if let Some((held, lit)) = &*self.compiled_lit.borrow()
            && *held == high
            && let Some(reads) = &lit.texel_reads
        {
            visit(reads);
        }
    }

    pub fn new(
        device: &wgpu::Device,
        samples: u32,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        assets: &TreeAssets,
        effect: &Arc<shader_pipeline::library::Effect>,
    ) -> io::Result<Self> {
        let mut diffuse = Vec::new();
        let mut normal = Vec::new();
        let mut materials = Vec::new();
        for (palette, tree) in &assets.meshes {
            for (lod, selected) in tree.lods.iter().enumerate() {
                let Some(selected) = selected else { continue };
                diffuse.push(upload(
                    device,
                    queue,
                    &format!("{} diffuse", tree.name),
                    &selected.diffuse,
                    ColorSpace::Srgb,
                )?);
                normal.push(upload(
                    device,
                    queue,
                    &format!("{} normal", tree.name),
                    &selected.normal,
                    ColorSpace::Linear,
                )?);
                materials.push((*palette, lod));
            }
        }
        Ok(Self {
            compiled_lit: RefCell::new(None),
            effect: effect.clone(),
            samples,
            format,
            compiled_receiver: super::tree_receiver::Receiver::new(
                device,
                effect,
                BUFFER_FORMAT,
                viewer_scene_base::shadow_blur::DEPTH_FORMAT,
            )?,
            compiled_caster: viewer_scene_base::tree_caster::Caster::new(
                device,
                effect,
                DEPTH_MAP_FORMAT,
                DEPTH_FORMAT,
            )?,
            null: data_texture(
                device,
                queue,
                "tree explicit null slots",
                [1, 1],
                wgpu::TextureFormat::Rgba8Unorm,
                &[0; 4],
            )?,
            tint: rgb_texture(device, queue, "TintMap (Tree_tint.bmp)", &assets.tint)?,
            season: rgb_texture(device, queue, "SeasonMap (Tree_season.bmp)", &assets.season)?,
            mask: data_texture(
                device,
                queue,
                "TreeMaskTexture (map/rivers.bmp)",
                [assets.mask.width, assets.mask.height],
                wgpu::TextureFormat::Rgba8Unorm,
                &assets
                    .mask
                    .data
                    .iter()
                    .flat_map(|a| [0, 0, 0, *a])
                    .collect::<Vec<_>>(),
            )?,
            diffuse,
            normal,
            materials,
            vertices: None,
            indices: None,
            instances: None,
            instance_bytes: Default::default(),
            geometry: TreeGeometry::default(),
            buckets: Vec::new(),
            admitted: RefCell::default(),
            drawn: Cell::new(0),
            drawn_buckets: Cell::new(0),
            caster: viewer_scene_base::shadow::CasterId::next(),
        })
    }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        geometry: TreeGeometry,
        field: &TreeField,
    ) -> io::Result<()> {
        if geometry.indices.is_empty() || field.instances.is_empty() {
            self.vertices = None;
            self.indices = None;
            self.instances = None;
            self.geometry = TreeGeometry::default();
            self.buckets = Vec::new();
            self.admitted = RefCell::default();
            return Ok(());
        }
        self.vertices = Some(buffer(
            device,
            "tree canopy vertices",
            wgpu::BufferUsages::VERTEX,
            bytemuck::cast_slice(&geometry.vertices),
        )?);
        self.indices = Some(buffer(
            device,
            "tree canopy indices",
            wgpu::BufferUsages::INDEX,
            bytemuck::cast_slice(&geometry.indices),
        )?);
        self.instance_bytes = bytemuck::cast_slice(&field.instances).into();
        self.instances = Some(buffer(
            device,
            "tree instances",
            wgpu::BufferUsages::VERTEX,
            bytemuck::cast_slice(&field.instances),
        )?);
        self.geometry = geometry;
        self.buckets = field.buckets.clone();
        self.admitted = RefCell::default();
        Ok(())
    }

    pub fn drawn(&self) -> u32 {
        self.drawn.get()
    }

    pub fn drawn_buckets(&self) -> u32 {
        self.drawn_buckets.get()
    }

    pub fn caster_id(&self) -> viewer_scene_base::shadow::CasterId {
        self.caster
    }

    pub fn warm(&self, device: &wgpu::Device, high: bool) {
        let _ = self.compile_lit(device, high);
    }

    fn compile_lit(&self, device: &wgpu::Device, high: bool) -> io::Result<()> {
        let mut cached = self.compiled_lit.borrow_mut();
        if cached.as_ref().is_none_or(|(held, _)| *held != high) {
            *cached = Some((
                high,
                super::tree_shader::Lit::new(
                    device,
                    &self.effect,
                    self.format,
                    DEPTH_FORMAT,
                    self.samples,
                    high,
                )?,
            ));
        }
        Ok(())
    }

    pub fn draw(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        frame: TreeFrame<'_>,
        inputs: LitInputs<'_>,
    ) -> io::Result<()> {
        let device = arena.device();
        if inputs.view == 0 {
            self.drawn.set(0);
            self.drawn_buckets.set(0);
        }
        if frame.camera.height > frame.camera.defines.tree_fade[1] {
            return Ok(());
        }
        let (Some(vertices), Some(indices), Some(instances)) =
            (&self.vertices, &self.indices, &self.instances)
        else {
            return Ok(());
        };
        self.compile_lit(device, inputs.high)?;
        let cached = self.compiled_lit.borrow();
        let compiled = &cached.as_ref().expect("compiled above").1;
        use content_reader_gui::pdxshader::program::Stage;
        use shader_pipeline::binding;
        let mut textures: BTreeMap<_, _> = inputs
            .textures
            .into_iter()
            .map(|(slot, view)| (binding(slot, Stage::Pixel), view.clone()))
            .collect();
        for (slot, view) in [
            (2, &self.tint.view),
            (3, &self.season.view),
            (5, &self.null.view),
            (7, &self.null.view),
            (14, &self.mask.view),
        ] {
            textures.insert(binding(slot, Stage::Pixel), view.clone());
        }
        let clock = compiled.clock.for_styles(inputs.pulses);
        let animated = clock
            .live(inputs.fog)
            .max(clock.pixel_motion(inputs.retained_pixel_b2.time_read_by(&compiled.pipeline)));
        let mut pass =
            viewer_scene_base::shader_scene::surface_pass(encoder, targets, "trees", animated);
        let pass = &mut pass;
        shader_pipeline::gpu::stats::pipeline();
        pass.set_pipeline(&compiled.pipeline.pipeline);
        shader_pipeline::gpu::stats::state();
        pass.set_blend_constant(wgpu::Color::WHITE);
        shader_pipeline::gpu::stats::state();
        pass.set_vertex_buffer(0, vertices.slice(..));
        shader_pipeline::gpu::stats::state();
        pass.set_vertex_buffer(1, instances.slice(..));
        shader_pipeline::gpu::stats::state();
        pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
        let fade = tree_fade(&frame.camera.defines, frame.camera.height);
        {
            let copy = inputs.view;
            let scene = inputs.frames[copy];
            let mut bindings = Vec::new();
            for (diffuse, normal) in self.diffuse.iter().zip(&self.normal) {
                textures.insert(binding(0, Stage::Pixel), diffuse.view.clone());
                textures.insert(binding(1, Stage::Pixel), normal.view.clone());
                bindings.push(compiled.bindings(
                    arena,
                    scene,
                    fade,
                    inputs.retained_pixel_b2,
                    &textures,
                )?);
            }
            let mut previous = None;
            let batches = self.batches(frame, copy);
            for &(slot, ref indices, ref instances) in batches.iter() {
                if let Some(retained) = inputs.retained {
                    textures.insert(binding(0, Stage::Pixel), self.diffuse[slot].view.clone());
                    textures.insert(binding(1, Stage::Pixel), self.normal[slot].view.clone());
                    let mut tracked = retained.borrow_mut();
                    compiled.retain(
                        &mut tracked,
                        scene,
                        fade,
                        inputs.retained_pixel_b2,
                        &textures,
                    );
                    tracked.bind_vertex(
                        1,
                        std::mem::size_of::<TreeInstance>() as u64,
                        self.instance_bytes.clone(),
                    );
                }
                bindings[slot].bind_after(pass, previous);
                previous = Some(&bindings[slot]);
                self.drawn
                    .set(self.drawn.get() + instances.end - instances.start);
                self.drawn_buckets.set(self.drawn_buckets.get() + 1);
                shader_pipeline::gpu::stats::draw();
                pass.draw_indexed(indices.clone(), 0, instances.clone());
            }
        }
        Ok(())
    }

    pub fn draw_casters(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        pass: &mut wgpu::RenderPass<'_>,
        light_matrix: [[f32; 4]; 4],
        frame: TreeFrame<'_>,
    ) -> io::Result<()> {
        let (Some(vertices), Some(indices), Some(instances)) =
            (&self.vertices, &self.indices, &self.instances)
        else {
            return Ok(());
        };
        shader_pipeline::gpu::stats::pipeline();
        pass.set_pipeline(&self.compiled_caster.pipeline.pipeline);
        shader_pipeline::gpu::stats::state();
        pass.set_blend_constant(wgpu::Color::WHITE);
        shader_pipeline::gpu::stats::state();
        pass.set_vertex_buffer(0, vertices.slice(..));
        shader_pipeline::gpu::stats::state();
        pass.set_vertex_buffer(1, instances.slice(..));
        shader_pipeline::gpu::stats::state();
        pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
        for (copy, offset) in frame.offsets.iter().copied().enumerate() {
            let mut matrix = light_matrix;
            for (value, axis) in matrix[3].iter_mut().zip(light_matrix[0]) {
                *value += axis * offset;
            }
            let mut bindings = Vec::new();
            for diffuse in &self.diffuse {
                bindings.push(self.compiled_caster.bindings(
                    arena,
                    matrix,
                    &diffuse.view,
                    &self.mask.view,
                    &self.null.view,
                )?);
            }
            let mut previous = None;
            for &(slot, ref indices, ref instances) in self.batches(frame, copy).iter() {
                bindings[slot].bind_after(pass, previous);
                previous = Some(&bindings[slot]);
                shader_pipeline::gpu::stats::draw();
                pass.draw_indexed(indices.clone(), 0, instances.clone());
            }
        }
        Ok(())
    }

    pub fn receiver_motion(&self, fog: bool) -> viewer_gpu::surface_motion::Motion {
        self.compiled_receiver.clock.live(fog)
    }

    pub fn draw_unlit(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        pass: &mut wgpu::RenderPass<'_>,
        frame: TreeFrame<'_>,
        inputs: ReceiverInputs<'_>,
    ) -> io::Result<()> {
        if frame.camera.height > frame.camera.defines.tree_fade[1] {
            return Ok(());
        }
        let (Some(vertices), Some(indices), Some(instances)) =
            (&self.vertices, &self.indices, &self.instances)
        else {
            return Ok(());
        };
        let fade = tree_fade(&frame.camera.defines, frame.camera.height);
        use content_reader_gui::pdxshader::program::Stage;
        use shader_pipeline::binding;
        let mut textures: BTreeMap<_, _> = inputs
            .textures
            .into_iter()
            .map(|(slot, view)| (binding(slot, Stage::Pixel), view.clone()))
            .collect();
        textures.insert(binding(5, Stage::Pixel), self.null.view.clone());
        textures.insert(binding(7, Stage::Pixel), self.null.view.clone());
        textures.insert(binding(14, Stage::Pixel), self.mask.view.clone());
        shader_pipeline::gpu::stats::pipeline();
        pass.set_pipeline(&self.compiled_receiver.pipeline.pipeline);
        shader_pipeline::gpu::stats::state();
        pass.set_blend_constant(wgpu::Color::WHITE);
        shader_pipeline::gpu::stats::state();
        pass.set_vertex_buffer(0, vertices.slice(..));
        shader_pipeline::gpu::stats::state();
        pass.set_vertex_buffer(1, instances.slice(..));
        shader_pipeline::gpu::stats::state();
        pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
        for (copy, offset) in frame.offsets.iter().copied().enumerate() {
            let mut matrix = inputs.shadow_matrix;
            for (value, axis) in matrix[3].iter_mut().zip(inputs.shadow_matrix[0]) {
                *value += axis * offset;
            }
            let mut bindings = Vec::new();
            for (diffuse, normal) in self.diffuse.iter().zip(&self.normal) {
                textures.insert(binding(0, Stage::Pixel), diffuse.view.clone());
                textures.insert(binding(1, Stage::Pixel), normal.view.clone());
                bindings.push(self.compiled_receiver.bindings(
                    arena,
                    inputs.frames[copy],
                    matrix,
                    fade,
                    &textures,
                )?);
            }
            let mut previous = None;
            for &(slot, ref indices, ref instances) in self.batches(frame, copy).iter() {
                bindings[slot].bind_after(pass, previous);
                previous = Some(&bindings[slot]);
                shader_pipeline::gpu::stats::draw();
                pass.draw_indexed(indices.clone(), 0, instances.clone());
            }
        }
        Ok(())
    }

    fn batches(&self, frame: TreeFrame<'_>, copy: usize) -> std::cell::Ref<'_, [Batch]> {
        let key = (frame.camera, frame.planes, frame.offsets.to_vec());
        if self.admitted.borrow().key.as_ref() != Some(&key) {
            let copies = self.walk(frame);
            *self.admitted.borrow_mut() = Admitted { key: Some(key), copies };
            viewer_runtime::view_record::add("trees", "walks", 1);
            viewer_runtime::view_record::add("trees", "buckets_walked", self.buckets.len() as u64);
        } else if viewer_scene_base::actor_check::every().is_some() {
            let fresh = self.walk(frame);
            if fresh != self.admitted.borrow().copies {
                viewer_scene_base::actor_check::differs("tree buckets", &copy, &"the held walk is not this camera's");
            }
            viewer_scene_base::actor_check::checked("tree buckets", self.buckets.len() as u64);
        }
        viewer_runtime::view_record::add("trees", "batches_read", 1);
        std::cell::Ref::map(self.admitted.borrow(), |admitted| {
            admitted.copies.get(copy).map_or(&[][..], Vec::as_slice)
        })
    }

    fn walk(&self, frame: TreeFrame<'_>) -> Vec<Vec<Batch>> {
        let mut rows: Vec<Vec<super::tree_order::Entry>> = vec![Vec::new(); frame.offsets.len()];
        for (index, bucket) in self.buckets.iter().enumerate() {
            let Some((selected, distance)) = nearest_visible_copy(bucket, frame) else {
                continue;
            };
            rows[selected].push(super::tree_order::Entry {
                palette: bucket.palette,
                distance,
                bucket: index,
            });
        }
        rows.iter_mut()
            .map(|rows| {
                super::tree_order::sort(rows);
                rows.iter()
                    .filter_map(|row| {
                        let bucket = &self.buckets[row.bucket];
                        let (distances, shapes) = self.geometry.lods.get(&bucket.palette)?;
                        let lod = lod_for_squared_distance(distances, row.distance, *shapes);
                        let slot = self
                            .materials
                            .iter()
                            .position(|p| *p == (bucket.palette, lod))?;
                        let range = self.geometry.ranges.get(&(bucket.palette, lod))?;
                        (!range.is_empty()).then(|| (slot, range.clone(), bucket.range.clone()))
                    })
                    .collect()
            })
            .collect()
    }
}

const TILE_CEILING: f32 = 25.5;

fn nearest_visible_copy(bucket: &TreeBucket, frame: TreeFrame<'_>) -> Option<(usize, f32)> {
    let mut nearest = None;
    let mut minimum = f32::MAX;
    for (copy, offset) in frame.offsets.iter().copied().enumerate() {
        if visible(bucket, offset, &frame.planes) {
            let distance = bucket_distance_squared(bucket, offset, frame.camera.eye());
            if distance < minimum {
                minimum = distance;
                nearest = Some((copy, distance));
            }
        }
    }
    nearest
}

fn visible(
    bucket: &TreeBucket,
    offset: f32,
    planes: &viewer_gpu::province_admission::Planes,
) -> bool {
    let (low_x, high_x) = bucket.x_span();
    let (low_z, high_z) = bucket.z_span();
    admits_box(
        [[low_x, 0.0, low_z], [high_x, TILE_CEILING, high_z]],
        offset,
        planes,
    )
}

fn rgb_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    image: &Rgba8Image,
) -> io::Result<Bound> {
    data_texture(
        device,
        queue,
        label,
        [image.width, image.height],
        wgpu::TextureFormat::Rgba8Unorm,
        &image.pixels,
    )
}

#[cfg(test)]
mod tests;
