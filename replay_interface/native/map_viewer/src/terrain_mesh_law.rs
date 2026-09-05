//! Every generated field is either PURE LAW (grid coords, index winding) or

use crate::*;
use std::collections::BTreeMap;

pub const TERRAIN_CHUNK_GRID_SIDE_VERTS: u32 = 65;
pub const TERRAIN_CHUNK_GRID_SIDE_CELLS: u32 = 64;
pub const TERRAIN_CHUNK_GRID_VERTS: u32 =
    TERRAIN_CHUNK_GRID_SIDE_VERTS * TERRAIN_CHUNK_GRID_SIDE_VERTS;
pub const TERRAIN_CHUNK_INDEX_COUNT: u32 =
    TERRAIN_CHUNK_GRID_SIDE_CELLS * TERRAIN_CHUNK_GRID_SIDE_CELLS * 6;
pub const TERRAIN_CHUNK_SLOT0_BYTES: usize = TERRAIN_CHUNK_GRID_VERTS as usize * 8;
pub const TERRAIN_CHUNK_SLOT1_TILE_BYTES: usize = TERRAIN_CHUNK_GRID_VERTS as usize * 4;
pub const TERRAIN_CHUNK_INDEX_BYTES: usize = TERRAIN_CHUNK_INDEX_COUNT as usize * 2;
pub const TERRAIN_CHUNK_CB1_BYTES: usize = 112;

pub const TERRAIN_CHUNK_SLOT0_POSITION_FORMAT: u32 = 14;
pub const TERRAIN_CHUNK_SLOT1_TEXCOORD_FORMAT: u32 = 38;
pub const TERRAIN_CHUNK_INDEX_FORMAT: u32 = 57;
pub const TERRAIN_CHUNK_SLOT0_STRIDE: u32 = 8;
pub const TERRAIN_CHUNK_SLOT1_STRIDE: u32 = 4;

pub fn terrain_chunk_scale_for_tile_size(tile_size: u32) -> Result<f32> {
    if tile_size == 0 || tile_size % TERRAIN_CHUNK_GRID_SIDE_CELLS != 0 {
        return Err(format!(
            "terrain mesh law: tile size {tile_size} is not a positive multiple of {} cells",
            TERRAIN_CHUNK_GRID_SIDE_CELLS
        )
        .into());
    }
    Ok((tile_size / TERRAIN_CHUNK_GRID_SIDE_CELLS) as f32)
}

pub fn terrain_chunk_vertex_edge_class(col: u32, row: u32) -> (i16, i16) {
    let last = TERRAIN_CHUNK_GRID_SIDE_CELLS;
    if row == 0 && col % 2 == 1 {
        (1, 0)
    } else if row == last && col % 2 == 1 {
        (2, 0)
    } else if col == 0 && row % 2 == 1 {
        (3, 1)
    } else if col == last && row % 2 == 1 {
        (4, 1)
    } else {
        (0, 0)
    }
}

pub fn generate_terrain_chunk_slot0() -> Vec<u8> {
    let mut out = Vec::with_capacity(TERRAIN_CHUNK_SLOT0_BYTES);
    for i in 0..TERRAIN_CHUNK_GRID_VERTS {
        let col = i % TERRAIN_CHUNK_GRID_SIDE_VERTS;
        let row = i / TERRAIN_CHUNK_GRID_SIDE_VERTS;
        let (z, w) = terrain_chunk_vertex_edge_class(col, row);
        out.extend_from_slice(&(col as i16).to_le_bytes());
        out.extend_from_slice(&(row as i16).to_le_bytes());
        out.extend_from_slice(&z.to_le_bytes());
        out.extend_from_slice(&w.to_le_bytes());
    }
    debug_assert_eq!(out.len(), TERRAIN_CHUNK_SLOT0_BYTES);
    out
}

pub fn generate_terrain_chunk_index() -> Vec<u8> {
    let mut out = Vec::with_capacity(TERRAIN_CHUNK_INDEX_BYTES);
    let side = TERRAIN_CHUNK_GRID_SIDE_VERTS as u16;
    for cz in 0..TERRAIN_CHUNK_GRID_SIDE_CELLS as u16 {
        for cx in 0..TERRAIN_CHUNK_GRID_SIDE_CELLS as u16 {
            let b = cz * side + cx;
            let tri: [u16; 6] = if (cx + cz) % 2 == 0 {
                [b, b + 1, b + side + 1, b, b + side + 1, b + side]
            } else {
                [b, b + 1, b + side, b + 1, b + side + 1, b + side]
            };
            for idx in tri {
                out.extend_from_slice(&idx.to_le_bytes());
            }
        }
    }
    debug_assert_eq!(out.len(), TERRAIN_CHUNK_INDEX_BYTES);
    out
}

fn sample_heightmap_raw(heightmap: &IndexImage, world_x: i64, world_z: i64) -> u8 {
    let width = heightmap.width as i64;
    let height = heightmap.height as i64;
    let mut bx = world_x % width;
    if bx < 0 {
        bx += width;
    }
    let by = (height - 1 - world_z).clamp(0, height - 1);
    heightmap.pixels[(by * width + bx) as usize]
}

fn terrain_chunk_height_i16(heightmap: &IndexImage, world_x: i64, world_z: i64) -> i16 {
    i16::from(sample_heightmap_raw(heightmap, world_x, world_z)) * 10
}

pub fn generate_terrain_chunk_slot1(
    heightmap: &IndexImage,
    min_x: i64,
    min_z: i64,
    tile_size: u32,
) -> Result<Vec<u8>> {
    let scale = terrain_chunk_scale_for_tile_size(tile_size)? as i64;
    let last = TERRAIN_CHUNK_GRID_SIDE_CELLS;
    let mut out = Vec::with_capacity(TERRAIN_CHUNK_SLOT1_TILE_BYTES);
    let world = |col: u32, row: u32| (min_x + col as i64 * scale, min_z + row as i64 * scale);
    for i in 0..TERRAIN_CHUNK_GRID_VERTS {
        let col = i % TERRAIN_CHUNK_GRID_SIDE_VERTS;
        let row = i / TERRAIN_CHUNK_GRID_SIDE_VERTS;
        let (wx, wz) = world(col, row);
        let primary = terrain_chunk_height_i16(heightmap, wx, wz);
        let alt = if row == 0 && col % 2 == 1 {
            let (nx, nz) = world(col + 1, 0);
            terrain_chunk_height_i16(heightmap, nx, nz)
        } else if row == last && col % 2 == 1 {
            let (nx, nz) = world(col + 1, last);
            terrain_chunk_height_i16(heightmap, nx, nz)
        } else if col == 0 && row % 2 == 1 {
            let (nx, nz) = world(0, row + 1);
            terrain_chunk_height_i16(heightmap, nx, nz)
        } else if col == last && row % 2 == 1 {
            let (nx, nz) = world(last, row + 1);
            terrain_chunk_height_i16(heightmap, nx, nz)
        } else {
            primary
        };
        out.extend_from_slice(&primary.to_le_bytes());
        out.extend_from_slice(&alt.to_le_bytes());
    }
    debug_assert_eq!(out.len(), TERRAIN_CHUNK_SLOT1_TILE_BYTES);
    Ok(out)
}

pub fn generate_terrain_chunk_cb1(
    min_x: f32,
    min_z: f32,
    tile_size: u32,
    detail_edge_flags: [f32; 4],
) -> Result<Vec<u8>> {
    let scale = terrain_chunk_scale_for_tile_size(tile_size)?;
    let mut out = vec![0u8; TERRAIN_CHUNK_CB1_BYTES];
    let write_row = |out: &mut [u8], row: usize, v: [f32; 4]| {
        for (k, value) in v.iter().enumerate() {
            let start = row * 16 + k * 4;
            out[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
    };
    write_row(&mut out, 4, [min_x, min_z, scale, 0.0]);
    write_row(&mut out, 5, detail_edge_flags);
    write_row(&mut out, 6, [0.0, 0.1, scale, 0.0]);
    Ok(out)
}

pub const TERRAIN_CHUNK_HEIGHTMAP_SOURCE: &str = "map/heightmap.bmp";

#[derive(Clone, Debug, PartialEq)]
pub struct TerrainMeshLawChunkComparison {
    pub chunk_id: String,
    pub tile_size: u32,
    pub min_x: f32,
    pub min_z: f32,
    pub slot1_offset: u32,
    pub slot0_identical: bool,
    pub slot1_identical: bool,
    pub index_identical: bool,
    pub cb1_identical: bool,
    pub first_mismatch: Option<(&'static str, usize)>,
}

impl TerrainMeshLawChunkComparison {
    pub fn identical(&self) -> bool {
        self.slot0_identical && self.slot1_identical && self.index_identical && self.cb1_identical
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerrainMeshLawLodTally {
    pub tile_size: u32,
    pub draws_checked: usize,
    pub draws_identical: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TerrainMeshLawCorpusReport {
    pub session_label: String,
    pub draws_checked: usize,
    pub draws_identical: usize,
    pub per_lod: Vec<TerrainMeshLawLodTally>,
    pub comparisons: Vec<TerrainMeshLawChunkComparison>,
}

impl TerrainMeshLawCorpusReport {
    pub fn all_identical(&self) -> bool {
        self.draws_checked > 0 && self.draws_checked == self.draws_identical
    }
}

fn terrain_chunk_draw_vs_cb1<'a>(draw: &'a MapFrameDraw) -> Result<&'a MapFrameBufferBinding> {
    draw.constant_buffers
        .iter()
        .find(|b| b.stage == "VS" && b.slot == TERRAIN_DRAW_CB1_BIND_POINT)
        .ok_or_else(|| "terrain mesh law corpus: chunk draw has no VS terrain CB1".into())
}

fn read_captured_buffer(
    bundle_dir: &std::path::Path,
    file: &str,
    expected_len: usize,
) -> Result<Vec<u8>> {
    let path = bundle_dir.join(file);
    let bytes = std::fs::read(&path)?;
    if bytes.len() != expected_len {
        return Err(format!(
            "terrain mesh law corpus: {} has {} bytes, metadata expects {}",
            path.display(),
            bytes.len(),
            expected_len
        )
        .into());
    }
    Ok(bytes)
}

pub fn terrain_mesh_law_validate_corpus(
    library: &NativeD3d11TerrainChunkLibrary,
    heightmap: &IndexImage,
    session_label: &str,
) -> Result<TerrainMeshLawCorpusReport> {
    let expected_slot0 = generate_terrain_chunk_slot0();
    let expected_index = generate_terrain_chunk_index();
    let bundle_dir = library.bundle_dir();
    let mut comparisons = Vec::new();
    let mut per_lod: BTreeMap<u32, TerrainMeshLawLodTally> = BTreeMap::new();

    for chunk in library.chunk_records() {
        let bounds = chunk.bounds;
        let tile_size = terrain_chunk_world_tile_size(chunk)?;

        let slot0_binding = chunk
            .draw
            .vertex_buffers
            .get("0")
            .ok_or("terrain mesh law corpus: chunk draw has no slot-0 vertex buffer")?;
        let slot0_file = slot0_binding
            .file
            .as_ref()
            .ok_or("terrain mesh law corpus: slot-0 binding has no file")?;
        let captured_slot0 = read_captured_buffer(bundle_dir, slot0_file, slot0_binding.byte_len)?;
        let slot0_identical = captured_slot0 == expected_slot0;

        let index_binding = chunk
            .draw
            .index_buffer
            .as_ref()
            .ok_or("terrain mesh law corpus: chunk draw has no index buffer")?;
        let index_file = index_binding
            .file
            .as_ref()
            .ok_or("terrain mesh law corpus: index binding has no file")?;
        let captured_index = read_captured_buffer(bundle_dir, index_file, index_binding.byte_len)?;
        let index_identical = captured_index == expected_index;

        let slot1_binding = chunk
            .draw
            .vertex_buffers
            .get("1")
            .ok_or("terrain mesh law corpus: chunk draw has no slot-1 vertex buffer")?;
        let slot1_file = slot1_binding
            .file
            .as_ref()
            .ok_or("terrain mesh law corpus: slot-1 binding has no file")?;
        let slot1_offset = slot1_binding
            .offset
            .ok_or("terrain mesh law corpus: slot-1 binding has no offset")?;
        let captured_pool = read_captured_buffer(bundle_dir, slot1_file, slot1_binding.byte_len)?;
        let window_start = slot1_offset as usize;
        let window_end = window_start
            .checked_add(TERRAIN_CHUNK_SLOT1_TILE_BYTES)
            .ok_or("terrain mesh law corpus: slot-1 window overflow")?;
        if window_end > captured_pool.len() {
            return Err(format!(
                "terrain mesh law corpus: chunk {} slot-1 window [{window_start}, {window_end}) exceeds pool length {}",
                chunk.chunk_id,
                captured_pool.len()
            )
            .into());
        }
        let captured_slot1 = &captured_pool[window_start..window_end];
        let expected_slot1 = generate_terrain_chunk_slot1(
            heightmap,
            bounds.min_x as i64,
            bounds.min_z as i64,
            tile_size,
        )?;
        let slot1_identical = captured_slot1 == expected_slot1.as_slice();

        let cb1_binding = terrain_chunk_draw_vs_cb1(&chunk.draw)?;
        let cb1_file = cb1_binding
            .file
            .as_ref()
            .ok_or("terrain mesh law corpus: CB1 binding has no file")?;
        let captured_cb1 = read_captured_buffer(bundle_dir, cb1_file, cb1_binding.byte_len)?;
        let detail_flags = terrain_chunk_cb1_detail_edge_flags_from_bytes(&captured_cb1)?;
        let expected_cb1 =
            generate_terrain_chunk_cb1(bounds.min_x, bounds.min_z, tile_size, detail_flags)?;
        let cb1_identical = captured_cb1 == expected_cb1;

        let first_mismatch = if !slot0_identical {
            first_byte_diff(&captured_slot0, &expected_slot0).map(|i| ("slot0", i))
        } else if !slot1_identical {
            first_byte_diff(captured_slot1, &expected_slot1).map(|i| ("slot1", i))
        } else if !index_identical {
            first_byte_diff(&captured_index, &expected_index).map(|i| ("index", i))
        } else if !cb1_identical {
            first_byte_diff(&captured_cb1, &expected_cb1).map(|i| ("cb1", i))
        } else {
            None
        };

        let comparison = TerrainMeshLawChunkComparison {
            chunk_id: chunk.chunk_id.clone(),
            tile_size,
            min_x: bounds.min_x,
            min_z: bounds.min_z,
            slot1_offset,
            slot0_identical,
            slot1_identical,
            index_identical,
            cb1_identical,
            first_mismatch,
        };
        let tally = per_lod.entry(tile_size).or_insert(TerrainMeshLawLodTally {
            tile_size,
            draws_checked: 0,
            draws_identical: 0,
        });
        tally.draws_checked += 1;
        if comparison.identical() {
            tally.draws_identical += 1;
        }
        comparisons.push(comparison);
    }

    let draws_checked = comparisons.len();
    let draws_identical = comparisons.iter().filter(|c| c.identical()).count();
    Ok(TerrainMeshLawCorpusReport {
        session_label: session_label.to_string(),
        draws_checked,
        draws_identical,
        per_lod: per_lod.into_values().collect(),
        comparisons,
    })
}

pub fn terrain_chunk_pool_grid_dims(
    world_width: u32,
    world_height: u32,
    tile_size: u32,
) -> Result<(u32, u32)> {
    if tile_size == 0 || world_width % tile_size != 0 || world_height % tile_size != 0 {
        return Err(format!(
            "terrain mesh law: world {world_width}x{world_height} is not divisible by tile size {tile_size}"
        )
        .into());
    }
    Ok((world_width / tile_size, world_height / tile_size))
}

pub fn generate_terrain_chunk_slot1_pool(
    heightmap: &IndexImage,
    world_width: u32,
    world_height: u32,
    tile_size: u32,
) -> Result<Vec<u8>> {
    let (tiles_x, tiles_z) = terrain_chunk_pool_grid_dims(world_width, world_height, tile_size)?;
    let mut pool =
        Vec::with_capacity(tiles_x as usize * tiles_z as usize * TERRAIN_CHUNK_SLOT1_TILE_BYTES);
    for tz in 0..tiles_z {
        for tx in 0..tiles_x {
            let min_x = (tx * tile_size) as i64;
            let min_z = (tz * tile_size) as i64;
            let window = generate_terrain_chunk_slot1(heightmap, min_x, min_z, tile_size)?;
            pool.extend_from_slice(&window);
        }
    }
    Ok(pool)
}

/// it). Returns an error if any generated payload's hash does not match the
pub(crate) fn build_terrain_chunk_synthetic_payloads(
    library: &NativeD3d11TerrainChunkLibrary,
    heightmap: &IndexImage,
    world_width: u32,
    world_height: u32,
) -> Result<BTreeMap<String, MapFrameSyntheticPayload>> {
    let mut payloads: BTreeMap<String, MapFrameSyntheticPayload> = BTreeMap::new();
    let slot0 = generate_terrain_chunk_slot0();
    let slot0_hash = fnv1a64_bytes(&slot0);
    let index = generate_terrain_chunk_index();
    let index_hash = fnv1a64_bytes(&index);
    let mut pools: BTreeMap<u32, (Vec<u8>, String)> = BTreeMap::new();

    let verify = |file: &str, want: Option<&str>, got: &str| -> Result<()> {
        if let Some(want) = want {
            if want != got {
                return Err(format!(
                    "--chunks-from-files blocked: generated {file} hash {got} != recorded {want}; capture is outside the proven mesh law"
                )
                .into());
            }
        }
        Ok(())
    };

    for chunk in library.chunk_records() {
        let tile_size = terrain_chunk_world_tile_size(chunk)?;
        if let Some(binding) = chunk.draw.vertex_buffers.get("0") {
            if let Some(file) = binding.file.clone() {
                verify(&file, binding.fnv1a64.as_deref(), &slot0_hash)?;
                payloads
                    .entry(file)
                    .or_insert_with(|| MapFrameSyntheticPayload {
                        bytes: slot0.clone(),
                        fnv1a64: slot0_hash.clone(),
                        provenance: "chunk1 mesh-law slot0 (65x65 int grid)".to_string(),
                    });
            }
        }
        if let Some(binding) = chunk.draw.index_buffer.as_ref() {
            if let Some(file) = binding.file.clone() {
                verify(&file, binding.fnv1a64.as_deref(), &index_hash)?;
                payloads
                    .entry(file)
                    .or_insert_with(|| MapFrameSyntheticPayload {
                        bytes: index.clone(),
                        fnv1a64: index_hash.clone(),
                        provenance: "chunk1 mesh-law index (checkerboard tessellation)".to_string(),
                    });
            }
        }
        if let Some(binding) = chunk.draw.vertex_buffers.get("1") {
            if let Some(file) = binding.file.clone() {
                if !pools.contains_key(&tile_size) {
                    let pool = generate_terrain_chunk_slot1_pool(
                        heightmap,
                        world_width,
                        world_height,
                        tile_size,
                    )?;
                    let hash = fnv1a64_bytes(&pool);
                    pools.insert(tile_size, (pool, hash));
                }
                let (pool, hash) = pools.get(&tile_size).expect("pool present");
                verify(&file, binding.fnv1a64.as_deref(), hash)?;
                if binding.byte_len != pool.len() {
                    return Err(format!(
                        "--chunks-from-files blocked: generated LOD {tile_size} slot1 pool is {} bytes, metadata records {}",
                        pool.len(),
                        binding.byte_len
                    )
                    .into());
                }
                payloads
                    .entry(file)
                    .or_insert_with(|| MapFrameSyntheticPayload {
                        bytes: pool.clone(),
                        fnv1a64: hash.clone(),
                        provenance: format!(
                            "chunk1 mesh-law slot1 pool (LOD {tile_size}, heightmap-derived)"
                        ),
                    });
            }
        }
        let cb1_binding = terrain_chunk_draw_vs_cb1(&chunk.draw)?;
        if let Some(file) = cb1_binding.file.clone() {
            let captured_cb1 =
                read_captured_buffer(library.bundle_dir(), &file, cb1_binding.byte_len)?;
            let detail_flags = terrain_chunk_cb1_detail_edge_flags_from_bytes(&captured_cb1)?;
            let cb1 = generate_terrain_chunk_cb1(
                chunk.bounds.min_x,
                chunk.bounds.min_z,
                tile_size,
                detail_flags,
            )?;
            let hash = fnv1a64_bytes(&cb1);
            verify(&file, cb1_binding.fnv1a64.as_deref(), &hash)?;
            payloads
                .entry(file)
                .or_insert_with(|| MapFrameSyntheticPayload {
                    bytes: cb1,
                    fnv1a64: hash,
                    provenance: "chunk1 mesh-law CB1 (QuadOffset/scale/heightscale)".to_string(),
                });
        }
    }
    Ok(payloads)
}

fn first_byte_diff(a: &[u8], b: &[u8]) -> Option<usize> {
    if a.len() != b.len() {
        return Some(a.len().min(b.len()));
    }
    a.iter().zip(b.iter()).position(|(x, y)| x != y)
}

pub fn terrain_mesh_law_validate_corpus_with_resolver(
    resolver: &GameFileResolver,
    library: &NativeD3d11TerrainChunkLibrary,
    session_label: &str,
) -> Result<TerrainMeshLawCorpusReport> {
    let heightmap_path =
        resolver.resolve_required(TERRAIN_CHUNK_HEIGHTMAP_SOURCE, "heightmap.bmp")?;
    let heightmap = decode_bmp_indexed(&resolver.read_resolved_bytes(&heightmap_path)?, &heightmap_path)?;
    terrain_mesh_law_validate_corpus(library, &heightmap, session_label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    mod corpus {
        include!("../test_gates.rs");
    }

    #[test]
    fn scale_law_matches_captured_lods() {
        assert_eq!(terrain_chunk_scale_for_tile_size(128).unwrap(), 2.0);
        assert_eq!(terrain_chunk_scale_for_tile_size(256).unwrap(), 4.0);
        assert_eq!(terrain_chunk_scale_for_tile_size(512).unwrap(), 8.0);
        assert!(terrain_chunk_scale_for_tile_size(0).is_err());
        assert!(terrain_chunk_scale_for_tile_size(100).is_err());
    }

    #[test]
    fn slot0_is_the_65x65_int_grid_with_edge_classes() {
        let slot0 = generate_terrain_chunk_slot0();
        assert_eq!(slot0.len(), TERRAIN_CHUNK_SLOT0_BYTES);
        let read = |i: usize| {
            let b = &slot0[i * 8..i * 8 + 8];
            [
                i16::from_le_bytes([b[0], b[1]]),
                i16::from_le_bytes([b[2], b[3]]),
                i16::from_le_bytes([b[4], b[5]]),
                i16::from_le_bytes([b[6], b[7]]),
            ]
        };
        assert_eq!(read(0), [0, 0, 0, 0]);
        assert_eq!(read(66), [1, 1, 0, 0]);
        assert_eq!(read(4224), [64, 64, 0, 0]);
        assert_eq!(read(1), [1, 0, 1, 0]);
        assert_eq!(read(65), [0, 1, 3, 1]);
        assert_eq!(read(129), [64, 1, 4, 1]);
        assert_eq!(read(4161), [1, 64, 2, 0]);
        let classed = (0..TERRAIN_CHUNK_GRID_VERTS as usize)
            .filter(|&i| read(i)[2] != 0)
            .count();
        assert_eq!(classed, 128);
    }

    #[test]
    fn index_buffer_is_checkerboard_tessellation() {
        let ib = generate_terrain_chunk_index();
        assert_eq!(ib.len(), TERRAIN_CHUNK_INDEX_BYTES);
        let idx = |k: usize| u16::from_le_bytes([ib[k * 2], ib[k * 2 + 1]]);
        assert_eq!(
            [idx(0), idx(1), idx(2), idx(3), idx(4), idx(5)],
            [0, 1, 66, 0, 66, 65]
        );
        assert_eq!(
            [idx(6), idx(7), idx(8), idx(9), idx(10), idx(11)],
            [1, 2, 66, 2, 67, 66]
        );
    }

    fn flat_heightmap(width: usize, height: usize, value: u8) -> IndexImage {
        IndexImage {
            width,
            height,
            pixels: vec![value; width * height],
        }
    }

    #[test]
    fn slot1_flat_heightmap_is_uniform_fixedpoint() {
        let hm = flat_heightmap(5632, 2048, 89);
        let slot1 = generate_terrain_chunk_slot1(&hm, 2048, 1024, 512).unwrap();
        assert_eq!(slot1.len(), TERRAIN_CHUNK_SLOT1_TILE_BYTES);
        for i in 0..TERRAIN_CHUNK_GRID_VERTS as usize {
            let u = i16::from_le_bytes([slot1[i * 4], slot1[i * 4 + 1]]);
            let v = i16::from_le_bytes([slot1[i * 4 + 2], slot1[i * 4 + 3]]);
            assert_eq!((u, v), (890, 890));
        }
    }

    #[test]
    fn slot1_alt_height_is_next_even_neighbour_on_odd_boundary() {
        let (w, h) = (256usize, 256usize);
        let mut hm = flat_heightmap(w, h, 0);
        for y in 0..h {
            for x in 0..w {
                hm.pixels[y * w + x] = (x % 200) as u8;
            }
        }
        let slot1 = generate_terrain_chunk_slot1(&hm, 0, 0, 128).unwrap();
        let vert = 1usize;
        let u = i16::from_le_bytes([slot1[vert * 4], slot1[vert * 4 + 1]]);
        let v = i16::from_le_bytes([slot1[vert * 4 + 2], slot1[vert * 4 + 3]]);
        assert_eq!(u, 2 * 10);
        assert_eq!(v, 4 * 10, "odd bottom-edge alt height = next even col");
    }

    #[test]
    fn cb1_encodes_offset_scale_heightscale() {
        let cb1 = generate_terrain_chunk_cb1(2048.0, 1024.0, 512, [0.0; 4]).unwrap();
        assert_eq!(cb1.len(), TERRAIN_CHUNK_CB1_BYTES);
        let f =
            |off: usize| f32::from_le_bytes([cb1[off], cb1[off + 1], cb1[off + 2], cb1[off + 3]]);
        for k in 0..16 {
            assert_eq!(f(k * 4), 0.0);
        }
        assert_eq!([f(64), f(68), f(72), f(76)], [2048.0, 1024.0, 8.0, 0.0]);
        assert_eq!([f(80), f(84), f(88), f(92)], [0.0; 4]);
        assert_eq!([f(96), f(100), f(104), f(108)], [0.0, 0.1, 8.0, 0.0]);
    }

    #[test]
    fn cb1_byte_offsets_agree_with_constants() {
        assert_eq!(TERRAIN_DRAW_CB1_QUAD_OFFSET_SCALE_BYTE_OFFSET, 64);
        assert_eq!(TERRAIN_DRAW_CB1_DETAIL_EDGE_FLAGS_BYTE_OFFSET, 80);
    }

    #[test]
    fn ia_layout_constants_match_the_decoded_terrain_grid() {
        // The generated IA formats/strides must equal the captured terrain-grid
        use crate::border_capture_evidence::dxgi;
        assert_eq!(TERRAIN_CHUNK_SLOT0_POSITION_FORMAT, dxgi::R16G16B16A16_SINT);
        assert_eq!(TERRAIN_CHUNK_SLOT1_TEXCOORD_FORMAT, dxgi::R16G16_SINT);
        assert_eq!(TERRAIN_CHUNK_INDEX_FORMAT, dxgi::R16_UINT);
        assert_eq!(TERRAIN_CHUNK_SLOT0_STRIDE, 8);
        assert_eq!(TERRAIN_CHUNK_SLOT1_STRIDE, 4);
        assert_eq!(
            TERRAIN_CHUNK_GRID_VERTS,
            crate::border_capture_evidence::DIAG23B_BORDERS_PASS_LAYOUT.vertex_count
        );
        assert_eq!(
            TERRAIN_CHUNK_INDEX_COUNT,
            crate::border_capture_evidence::DIAG23B_BORDERS_PASS_LAYOUT.index_count
        );
    }

    #[test]
    fn slot1_pool_dims_match_captured_lods() {
        assert_eq!(
            terrain_chunk_pool_grid_dims(5632, 2048, 512).unwrap(),
            (11, 4)
        );
        assert_eq!(
            terrain_chunk_pool_grid_dims(5632, 2048, 256).unwrap(),
            (22, 8)
        );
        assert_eq!(
            terrain_chunk_pool_grid_dims(5632, 2048, 128).unwrap(),
            (44, 16)
        );
        assert!(terrain_chunk_pool_grid_dims(5632, 2048, 100).is_err());
    }

    #[test]
    fn slot1_window_is_within_pool_and_never_over_fetches() {
        let max_index = generate_terrain_chunk_index()
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .max()
            .unwrap();
        assert_eq!(max_index, (TERRAIN_CHUNK_GRID_VERTS - 1) as u16);
        assert_eq!((max_index as usize + 1) * 4, TERRAIN_CHUNK_SLOT1_TILE_BYTES);
    }

    #[test]
    fn synthetic_payload_overrides_disk_and_never_reads_the_file() {
        // Negative proof that a generated payload is what gets bound: point the
        let bytes = generate_terrain_chunk_slot0();
        let hash = fnv1a64_bytes(&bytes);
        let file = "terrain_chunk_map_frame_shared_payload_ia_vertex_buffer_slot0_unnamed_1e5572789d573945.d3d11";
        let mut payloads = BTreeMap::new();
        payloads.insert(
            file.to_string(),
            MapFrameSyntheticPayload {
                bytes: bytes.clone(),
                fnv1a64: hash.clone(),
                provenance: "test".to_string(),
            },
        );
        let out = read_map_frame_resource_payload(
            std::path::Path::new("C:/nonexistent-dir-for-chunk1-test"),
            &payloads,
            file,
            bytes.len(),
            &hash,
            "vertex buffer",
        )
        .expect("payload must be returned without touching disk");
        assert_eq!(out, bytes);
        let empty = BTreeMap::new();
        assert!(read_map_frame_resource_payload(
            std::path::Path::new("C:/nonexistent-dir-for-chunk1-test"),
            &empty,
            file,
            bytes.len(),
            &hash,
            "vertex buffer",
        )
        .is_err());
    }

    #[test]
    #[cfg_attr(
        not(gate_terrain_mesh_corpus),
        ignore = "a terrain chunk corpus and the game install (TERRAIN_MESH_LAW_CORPUS_CAPTURE plus TERRAIN_MESH_LAW_CORPUS_GAME, else preserved gate18_final plus the default install)"
    )]
    fn corpus_byte_exact_when_session_available() {
        let (capture, game) = corpus::terrain_mesh_corpus()
            .expect("the terrain mesh corpus gate was true at build time");
        let resolver = GameFileResolver::vanilla(game).expect("vanilla resolver");
        let library = NativeD3d11TerrainChunkLibrary::load_records_only(&capture)
            .expect("load chunk library records");
        let report =
            terrain_mesh_law_validate_corpus_with_resolver(&resolver, &library, "env-corpus")
                .expect("run corpus gate");
        eprintln!(
            "corpus gate: {} draws checked, {} identical; per-LOD {:?}",
            report.draws_checked,
            report.draws_identical,
            report
                .per_lod
                .iter()
                .map(|l| (l.tile_size, l.draws_checked, l.draws_identical))
                .collect::<Vec<_>>()
        );
        assert!(report.draws_checked > 0, "corpus had no chunk draws");
        for c in &report.comparisons {
            assert!(
                c.identical(),
                "chunk {} (size {}) mismatched: {:?}",
                c.chunk_id,
                c.tile_size,
                c.first_mismatch
            );
        }
        assert!(report.all_identical());
    }
}
