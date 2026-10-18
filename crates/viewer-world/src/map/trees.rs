//! Tree instances generated from the resolved tree bitmap and content meshes.
//!
//! Each nonzero bitmap pixel supplies a canopy instance. Placement retains
//! palette, terrain height/slopes, integer-degree rotation and generator order.
//! Descriptor scale is baked into mesh positions. Camera distance to each map
//! partition selects authored LOD; expanded side planes control admission.
//!
//! A generated half-resolution river mask keeps overhanging canopy off river
//! pixels. Source bytes below 254 are marked, rows are flipped, and two-by-two
//! blocks are averaged with integer truncation. Lit and shadow passes bind this
//! mask at texture slot 14.
//!
//! Tree techniques compile from the recorded content stack. Their artwork,
//! shading and animation come from installed content. The scene owns draw
//! order, receiver/caster gates and post-processing.
use std::cell::Cell;
use std::cell::RefCell;
use std::sync::Arc;
use std::{collections::BTreeMap, io, ops::Range};
use viewer_foundation::map::province_admission::admits_box;
use viewer_foundation::map::textures::load_file;
use viewer_foundation::map::textures::load_image;

use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use installed_content_core::{ResolvedLayers, mesh, mesh::Descriptors};
use installed_content_image::{IndexedImage, Rgba8Image, dds, native::Decode};

use {
    super::shadow::{BUFFER_FORMAT, DEPTH_MAP_FORMAT},
    viewer_foundation::map::{
        camera::{Camera, MapDefines, WATER_HEIGHT},
        gpu_contract::{DEPTH_FORMAT, Targets, buffer},
        province_map::HeightField,
        textures::{Bound, ColorSpace, data_texture, upload},
    },
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

// ------------------------------------------------------------------ constants

/// Columns of `map/terrain/Tree_season.bmp`, one per `tree_*` season block.
pub const SEASON_COLUMNS: usize = 8;
/// `vTreeFade` at a camera height.
///
/// The whole layer is blended at this alpha, so every pixel it contributes
/// scales with it; above the content's `TREE_FADE_FAR` the engine returns before issuing a
/// draw, which [`TreePass::draw`] reproduces by drawing nothing.
pub fn tree_fade(defines: &MapDefines, height: f32) -> f32 {
    let [near, far] = defines.tree_fade;
    if height > far {
        return 0.0;
    }
    1.0 - (height.max(near) - near) / (far - near)
}

/// The pdxmesh key a `map/trees.bmp` palette index names.
///
/// Append decimal palette index to the mapobject_ registry prefix.
pub fn mapobject_name(index: u8) -> String {
    format!("mapobject_{index}")
}

// --------------------------------------------------------- instance generation

/// One canopy instance, in the engine's own 24-byte record order: world
/// position, rotation about Y in radians, and the terrain slope the tile is
/// sheared onto.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct TreeInstance {
    pub position: [f32; 3],
    pub rotation: f32,
    pub slope: [f32; 2],
}

/// One partition of one palette index: the engine's frustum-cull and LOD granularity.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeBucket {
    /// The `map/trees.bmp` palette index, which names the mesh.
    pub palette: u8,
    /// Tile column and row of the `MAP_SIZE >> 8` grid.
    pub tile: [u32; 2],
    /// Integer `dimension / (dimension >> 8)` on each axis.
    pub pitch: [u32; 2],
    /// The bucket's slice of [`TreeField::instances`].
    pub range: Range<u32>,
}

impl TreeBucket {
    /// The tile's world-space extent in x, before any wrap offset.
    fn x_span(&self) -> (f32, f32) {
        let low = self.tile[0] * self.pitch[0];
        (low as f32, (low + self.pitch[0]) as f32)
    }

    /// The tile's world-space extent in z.
    fn z_span(&self) -> (f32, f32) {
        let low = self.tile[1] * self.pitch[1];
        (low as f32, (low + self.pitch[1]) as f32)
    }
}

/// Every instance on the map, ordered so that each bucket is contiguous.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeField {
    pub instances: Vec<TreeInstance>,
    pub buckets: Vec<TreeBucket>,
    /// Independent world units per tree-bitmap pixel on x and z.
    pub cell: [f32; 2],
    /// Palette indices that carry at least one instance, ascending.
    pub palettes: Vec<u8>,
}

/// `map/trees.bmp` as the loader returns it: one palette index per pixel, rows
/// top-left first.
#[derive(Clone, Copy, Debug)]
pub struct TreeImage<'a> {
    pub width: u32,
    pub height: u32,
    pub indices: &'a [u8],
}

impl TreeImage<'_> {
    /// Checks the index count matches the declared dimensions.
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

/// Integer-degree tree rotation.
///
/// Reset seed per bitmap load. Advance only for nonzero pixels in north-to-south, west-to-east order.
/// Clear the sign bit before modulo 360 and convert degrees to radians in f32. Retain rotation for
/// shading.
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

/// called with `(x + .5, z + .5, false)` by the tree
/// generator: subtract the half texel, clamp the integer base to size - 2,
/// bilinearly interpolate bytes, then multiply by 0.1. Keep that operation
/// order, including the fractional coordinate relative to the clamped base.
/// The bitmap loader returns north-first rows; the native sampler uses world z.
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

/// Generates every instance of `map/trees.bmp`.
///
/// For **every** non-zero pixel — there is no density roll and no sub-pixel
/// jitter — the engine places one instance at the pixel's world position, with
/// a half-cell brick offset on alternate rows, the terrain height under the
/// cell centre, and the slope from the four corner heights. Rows are visited
/// from the north; the stored row index runs the other way, from the southern
/// edge. The north-first loop counter drives the brick offset.
///
/// Errors are named `InvalidData` for a bitmap or height field whose sample
/// count does not match its dimensions.
pub fn generate(image: &TreeImage<'_>, heights: &HeightField<'_>) -> io::Result<TreeField> {
    // The heightmap is map-sized (`MapAssets::load` checks it), so it gives
    // `MAP_SIZE`.
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
        // `rb = treeH - 1 - k` is the stored bottom-up row, and world z is
        // `rb * cell`; the loader already flipped the rows, so the stored row
        // `rb` is the top-down row `k`.
        let rb = image.height as usize - 1 - k;
        let z = rb as f32 * cell[1];
        for tx in 0..width {
            let palette = image.indices[k * width + tx];
            if palette == 0 {
                continue;
            }
            let mut x = tx as f32 * cell[0];
            // Tile assignment precedes the half-cell offset.
            let tile_x = (x / pitch[0] as f32) as u32;
            let tile_z = (z / pitch[1] as f32) as u32;
            // Native addresses partitions linearly; an x remainder can enter the next row.
            let linear = tile_z * tiles[0] + tile_x;
            if linear >= tiles[0] * tiles[1] {
                return Err(invalid(format!(
                    "tree palette {palette} pixel ({tx}, {k}) addresses partition {linear} outside the {} x {} native visibility grid",
                    tiles[0], tiles[1]
                )));
            }
            // Test parity with the north-first loop counter rather than the stored bottom-up row.
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

/// Test convenience for a plain camera distance; runtime keeps the native squared value.
#[cfg(test)]
pub fn lod_for_distance(lod_distances: &[f32], distance: f32, shapes: usize) -> usize {
    lod_for_squared_distance(lod_distances, distance * distance, shapes)
}

/// Square authored thresholds, stop at the first threshold greater than cached squared distance,
/// and clamp to the shape count.
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

/// Tile LOD uses squared distance from the eye to the nearest point of the tile's water-plane
/// rectangle. Horizontal components are zero while the eye lies inside the tile. Water-plane
/// height supplies the vertical component.
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
    // Evaluate height, x, then z, retaining squared distance.
    dy * dy + dx * dx + dz * dz
}

/// The LOD a bucket's tile draws at, from [`bucket_distance`].
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

// --------------------------------------------------------------------- assets

/// One palette index's mesh and its two textures.
pub struct TreeMeshAssets {
    /// The pdxmesh key, `mapobject_<index>`.
    pub name: String,
    /// The `.mesh` path the descriptor named.
    pub path: String,
    pub lod_distances: Vec<f32>,
    /// First admitted native material/geometry record of each LOD.
    pub lods: Vec<Option<TreeLod>>,
    /// Descriptor scale already baked into Geometry; retained for diagnostics.
    pub scale: f32,
}

pub struct TreeLod {
    pub geometry: super::mesh_shader::Geometry,
    pub diffuse: dds::Texture,
    pub normal: dds::Texture,
}

/// Everything the tree pass reads from installed content.
pub struct TreeAssets {
    /// One entry per palette index that carries an instance, ascending.
    pub meshes: Vec<(u8, TreeMeshAssets)>,
    /// `map/terrain/Tree_tint.bmp`, 16 x 16 24-bit.
    pub tint: Rgba8Image,
    /// `map/terrain/Tree_season.bmp`, 8 x 16 24-bit.
    pub season: Rgba8Image,
    /// `TreeMaskTexture`, generated from `map/rivers.bmp`; see [`tree_mask`].
    pub mask: TreeMask,
}

impl TreeAssets {
    /// Reads the descriptors, meshes and textures for the palette indices in
    /// use, appending any limitation to `warnings`.
    ///
    /// `map/terrain/Tree_*.bmp` is covered content and
    /// are hash-verified; `gfx/entities` and `gfx/models` are outside the
    /// coverage policy , so the descriptor, the
    /// meshes and their textures go through
    /// [`ResolvedLayers::read_uncovered`] and are not verified. The caller
    /// states that limitation once.
    ///
    /// A palette index the descriptor does not declare, or whose mesh or
    /// texture will not decode, is reported as a warning and dropped rather
    /// than refused: one unusable tree kind must not stop a replay opening.
    pub fn load(
        layers: &ResolvedLayers,
        decode: Decode,
        palettes: &[u8],
        rivers: &IndexedImage,
        warnings: &mut Vec<String>,
    ) -> io::Result<Self> {
        // Resolve tree definitions through the complete loaded descriptor roster.
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
        let mut materials = super::mesh_effect::Materials::new(layers, size, decode);
        // A tree kind the visual mods break is drawn from the recorded layers below them, its
        // mesh, textures and admitted materials theirs, named; one neither gives is refused by
        // name for the map's fallback.
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
                            super::mesh_effect::Materials::new(stack, size, decode)
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
        // Load both bitmaps as texture files. Their colours are artwork, read through the
        // presentation layers; where trees stand is recorded geometry.
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

/// TreeMaskTexture, the generated river mask that clips canopy
/// against, generated the way the engine generates it.
pub struct TreeMask {
    /// Half the river bitmap in each direction: 2816 x 1024 on shipped content.
    pub width: u32,
    pub height: u32,
    /// One byte per texel, rows top-down like the bitmap it came from.
    pub data: Vec<u8>,
}

/// Not a river: `map/rivers.bmp` indices 254 (land) and 255 (sea) are the two
/// bulk classes and everything below them is a river pixel (`map::rivers`).
const RIVER_INDEX_LIMIT: u8 = 254;

/// Builds [`TreeMask`] from `map/rivers.bmp`.
///
/// Generate the tree mask from river bytes below 254. Mark a full-resolution byte image, flip rows
/// and average two-by-two blocks into half resolution with integer truncation. Block offset follows
/// the loop below.
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
    // The filter walks EVERY source texel, not every second one: it writes the
    // 2 x 2 window at (y, x) into the destination texel (y >> 1, x >> 1), so the
    // four writes to a texel overwrite one another and the last one --- the
    // window at (2m + 1, 2n + 1) --- is the one that stays. A texel therefore
    // covers rows 2m+1..2m+2 and columns 2n+1..2n+2, one short of the aligned
    // quad. The native reads one row and column past the end on its last pass;
    // this clamps instead of following it out of the buffer.
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
    materials: &mut super::mesh_effect::Materials,
) -> io::Result<TreeMeshAssets> {
    let name = mapobject_name(palette);
    let definition = tree_definition(descriptors, palette)?;
    let path = definition.file.clone();
    let scale = definition.scale;
    let bytes = layers.read_uncovered(&path)?;
    let decoded = mesh::read(&bytes).map_err(|error| invalid(format!("{path}: {error}")))?;
    // Resolve the global texture index before the mesh directory.
    let directory = path.rsplit_once('/').map_or("", |(head, _)| head);
    let index = layers.texture_index()?;
    let texture = |file: &str| -> io::Result<dds::Texture> {
        if file.is_empty() {
            return Err(invalid(format!("{path} names no texture")));
        }
        let full = index.resolve(directory, file);
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
                    geometry: super::mesh_shader::Geometry::from_mesh(sub, scale)
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

/// Visit admitted records in source order and keep the first record of the selected LOD before
/// loading textures.
fn first_tree_record<'a>(
    shapes: &[&'a mesh::Shape],
    definition: &mesh::MeshDefinition,
    mut admitted: impl FnMut(&mesh::Material, &mesh::SubMesh) -> io::Result<bool>,
) -> io::Result<Option<(&'a mesh::SubMesh, mesh::Material)>> {
    for shape in shapes {
        for sub in &shape.meshes {
            if super::mesh_effect::candidate(sub) {
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
    // Do not consult the entity registry or its scale/clone rules.
    descriptors
        .meshes
        .get(&name)
        .ok_or_else(|| invalid(format!("content declares no pdxmesh \"{name}\"")))
}

// ------------------------------------------------------------------- geometry

/// One canopy vertex, in the order `VS_INPUT_INSTANCE` declares
/// (`tree.shader`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct TreeVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// `ta`: tangent in `xyz`, bitangent handedness in `w`.
    pub tangent: [f32; 4],
    pub uv0: [f32; 2],
    /// `u1`: the tint row and the height jitter. A mesh LOD that declares none
    /// fetches the next packed vertex's position.xy, or zero past the allocation.
    pub uv1: [f32; 2],
}

/// The uploaded geometry: every LOD of every tree mesh in one buffer pair, with
/// the index range each `(palette, lod)` draws.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeGeometry {
    pub vertices: Vec<TreeVertex>,
    pub indices: Vec<u32>,
    /// `(palette, lod)` to its slice of `indices`. Indices are absolute, so a
    /// draw needs no base vertex.
    pub ranges: BTreeMap<(u8, usize), Range<u32>>,
    /// `loddist` per palette index, and the number of shapes it declares.
    pub lods: BTreeMap<u8, (Vec<f32>, usize)>,
}

/// Flattens the first admitted record of every tree LOD into one buffer pair.
/// Positions already carry the descriptor scale.
pub fn geometry(assets: &TreeAssets) -> io::Result<TreeGeometry> {
    let mut out = TreeGeometry::default();
    for (palette, tree) in &assets.meshes {
        out.lods
            .insert(*palette, (tree.lod_distances.clone(), tree.lods.len()));
        for (lod, selected) in tree.lods.iter().enumerate() {
            let start = out.indices.len() as u32;
            if let Some(selected) = selected {
                // The declaration always includes UV1 at byte 48.
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

// ------------------------------------------------------------------- the pass

/// Scene-owned physical slots for the receiver's shared t2..15 bind.
pub struct ReceiverInputs<'a> {
    pub frames: &'a [&'a [u8]],
    pub shadow_matrix: [[f32; 4]; 4],
    pub textures: [(u32, &'a wgpu::TextureView); 11],
}

/// Scene-owned slots; the tree pass supplies diffuse, normal, tint, season and mask.
pub struct LitInputs<'a> {
    pub retained: Option<&'a RefCell<super::shader_scene::State>>,
    pub view: usize,
    pub frames: &'a [&'a [u8]],
    pub retained_pixel_b2: super::shader_constants::WorldPixelConstants,
    pub textures: [(u32, &'a wgpu::TextureView); 9],
    pub high: bool,
}

/// What one frame of the pass needs beyond its own geometry.
#[derive(Clone, Copy)]
pub struct TreeFrame<'a> {
    pub camera: Camera,
    /// World x translations of the wrap copies to submit.
    pub offsets: &'a [f32],
    /// All tree passes use the camera's expanded, outside-positive sides.
    pub planes: viewer_foundation::map::province_admission::Planes,
}

/// The GPU tree pass.
pub struct TreePass {
    compiled_caster: super::tree_caster::Caster,
    compiled_receiver: super::tree_receiver::Receiver,
    compiled_lit: RefCell<Option<(bool, super::tree_shader::Lit)>>,
    effect: Arc<shader_pipeline::library::Effect>,
    samples: u32,
    format: wgpu::TextureFormat,
    null: Bound,
    tint: Bound,
    season: Bound,
    /// `TreeMaskTexture`: the river mask this pass generates and owns, because
    /// no other pass binds it.
    mask: Bound,
    /// One per admitted (palette, LOD), in `materials` order.
    diffuse: Vec<Bound>,
    normal: Vec<Bound>,
    /// The (palette, LOD) of each texture pair.
    materials: Vec<(u8, usize)>,
    vertices: Option<wgpu::Buffer>,
    indices: Option<wgpu::Buffer>,
    instances: Option<wgpu::Buffer>,
    instance_bytes: Arc<[u8]>,
    geometry: TreeGeometry,
    buckets: Vec<TreeBucket>,
    /// Instances the last frame drew, for reporting.
    drawn: Cell<u32>,
    drawn_buckets: Cell<u32>,
}

impl TreePass {
    /// Builds the pipeline and uploads the tree textures.
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
                super::shadow_blur::DEPTH_FORMAT,
            )?,
            compiled_caster: super::tree_caster::Caster::new(
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
            // Native A8 sampling: wgpu has no A8Unorm, so preserve RGBA(0,0,0,a).
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
            drawn: Cell::new(0),
            drawn_buckets: Cell::new(0),
        })
    }

    /// Uploads the canopy geometry and the instance field. Called once per
    /// document: both depend only on content.
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
        Ok(())
    }

    /// Instances the last frame drew, for reporting.
    pub fn drawn(&self) -> u32 {
        self.drawn.get()
    }

    /// Buckets the last frame drew: one `draw_indexed` each, per wrap copy.
    pub fn drawn_buckets(&self) -> u32 {
        self.drawn_buckets.get()
    }

    /// Blends the canopy over the scene, once per visible world-wrap copy.
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
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("trees"),
            color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Load))],
            depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Load)),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let (Some(vertices), Some(indices), Some(instances)) =
            (&self.vertices, &self.indices, &self.instances)
        else {
            return Ok(());
        };
        let mut cached = self.compiled_lit.borrow_mut();
        if cached.as_ref().is_none_or(|(high, _)| *high != inputs.high) {
            *cached = Some((
                inputs.high,
                super::tree_shader::Lit::new(
                    device,
                    &self.effect,
                    self.format,
                    DEPTH_FORMAT,
                    self.samples,
                    inputs.high,
                )?,
            ));
        }
        let compiled = &cached.as_ref().expect("created above").1;
        use installed_content_core::pdxshader::program::Stage;
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
        pass.set_pipeline(&compiled.pipeline.pipeline);
        pass.set_blend_constant(wgpu::Color::WHITE);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_vertex_buffer(1, instances.slice(..));
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
            for (slot, indices, instances) in self.batches(frame, copy) {
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
                bindings[slot].bind(&mut pass);
                self.drawn
                    .set(self.drawn.get() + instances.end - instances.start);
                self.drawn_buckets.set(self.drawn_buckets.get() + 1);
                pass.draw_indexed(indices, 0, instances);
            }
        }
        Ok(())
    }

    /// Submit `treeshadow` into the light's depth map using the same camera tile cache as lit trees and
    /// receivers. There is no separate light-frustum cull.
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
        // There is no lit-tree fade gate. Caller shadow admission remains in scene.rs.
        pass.set_pipeline(&self.compiled_caster.pipeline.pipeline);
        pass.set_blend_constant(wgpu::Color::WHITE);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_vertex_buffer(1, instances.slice(..));
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
            for (slot, indices, instances) in self.batches(frame, copy) {
                bindings[slot].bind(pass);
                pass.draw_indexed(indices, 0, instances);
            }
        }
        Ok(())
    }

    /// `treeunlit` into the screen-space shadow buffer.
    pub fn draw_unlit(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        pass: &mut wgpu::RenderPass<'_>,
        frame: TreeFrame<'_>,
        inputs: ReceiverInputs<'_>,
    ) -> io::Result<()> {
        // Return above far, not at equality; alpha-zero still writes depth.
        if frame.camera.height > frame.camera.defines.tree_fade[1] {
            return Ok(());
        }
        let (Some(vertices), Some(indices), Some(instances)) =
            (&self.vertices, &self.indices, &self.instances)
        else {
            return Ok(());
        };
        let fade = tree_fade(&frame.camera.defines, frame.camera.height);
        use installed_content_core::pdxshader::program::Stage;
        use shader_pipeline::binding;
        let mut textures: BTreeMap<_, _> = inputs
            .textures
            .into_iter()
            .map(|(slot, view)| (binding(slot, Stage::Pixel), view.clone()))
            .collect();
        textures.insert(binding(5, Stage::Pixel), self.null.view.clone());
        textures.insert(binding(7, Stage::Pixel), self.null.view.clone());
        textures.insert(binding(14, Stage::Pixel), self.mask.view.clone());
        pass.set_pipeline(&self.compiled_receiver.pipeline.pipeline);
        pass.set_blend_constant(wgpu::Color::WHITE);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_vertex_buffer(1, instances.slice(..));
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
            for (slot, indices, instances) in self.batches(frame, copy) {
                bindings[slot].bind(pass);
                pass.draw_indexed(indices, 0, instances);
            }
        }
        Ok(())
    }

    fn batches<'a>(
        &'a self,
        frame: TreeFrame<'a>,
        copy: usize,
    ) -> impl Iterator<Item = (usize, Range<u32>, Range<u32>)> + 'a {
        // Collect in generator order, sort, then resolve model and LOD.
        let mut rows: Vec<_> = self
            .buckets
            .iter()
            .enumerate()
            .filter_map(|(index, bucket)| {
                let (selected, distance) = nearest_visible_copy(bucket, frame)?;
                (selected == copy).then_some(super::tree_order::Entry {
                    palette: bucket.palette,
                    distance,
                    bucket: index,
                })
            })
            .collect();
        super::tree_order::sort(&mut rows);
        rows.into_iter().filter_map(move |row| {
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
    }
}

/// Upper bound of the terrain-height byte range scaled to world units.
const TILE_CEILING: f32 = 25.5;

/// Replace the wrap tag only on a strictly smaller distance.
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

/// The four side planes only; native admission has no additional ground rectangle.
fn visible(
    bucket: &TreeBucket,
    offset: f32,
    planes: &viewer_foundation::map::province_admission::Planes,
) -> bool {
    let (low_x, high_x) = bucket.x_span();
    let (low_z, high_z) = bucket.z_span();
    admits_box(
        [[low_x, 0.0, low_z], [high_x, TILE_CEILING, high_z]],
        offset,
        planes,
    )
}

/// Uploads a 24-bit BMP the loader expanded to RGBA.
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
