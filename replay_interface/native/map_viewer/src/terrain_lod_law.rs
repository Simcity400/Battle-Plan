
use crate::*;
use std::collections::BTreeMap;

pub const TERRAIN_LOD_EDGE_COUNT: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainTileEdge {
    Bottom = 0,
    Top = 1,
    Left = 2,
    Right = 3,
}

impl TerrainTileEdge {
    pub const ALL: [TerrainTileEdge; TERRAIN_LOD_EDGE_COUNT] = [
        TerrainTileEdge::Bottom,
        TerrainTileEdge::Top,
        TerrainTileEdge::Left,
        TerrainTileEdge::Right,
    ];

    pub fn neighbour_step(self) -> (i64, i64) {
        match self {
            TerrainTileEdge::Bottom => (0, -1),
            TerrainTileEdge::Top => (0, 1),
            TerrainTileEdge::Left => (-1, 0),
            TerrainTileEdge::Right => (1, 0),
        }
    }
}

pub fn terrain_vsnap_from_neighbour_lods(
    this_tile_size: u32,
    neighbour_tile_sizes: [Option<u32>; TERRAIN_LOD_EDGE_COUNT],
) -> [f32; TERRAIN_LOD_EDGE_COUNT] {
    let mut vsnap = [0.0f32; TERRAIN_LOD_EDGE_COUNT];
    for (edge, neighbour) in neighbour_tile_sizes.iter().enumerate() {
        vsnap[edge] = match neighbour {
            Some(size) if *size > this_tile_size => 1.0,
            _ => 0.0,
        };
    }
    vsnap
}


pub const TERRAIN_LOD_THRESHOLDS_SQ: [f64; 4] = [22500.0, 122500.0, 562500.0, 2560000.0];

pub const TERRAIN_LOD_REFERENCE_HEIGHT: f64 = 25.5;

pub const TERRAIN_LOD_BASE_TILE_SIZE: u32 = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerrainLodCullOptions {
    pub suppress_near_stride_one: bool,
    pub high_gfx_shaders: bool,
    pub texture_quality: u8,
}

#[derive(Clone, Debug)]
pub struct TerrainLodStrideMap {
    pub cols: u32,
    pub rows: u32,
    strides: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainLodTile {
    pub min_x: u32,
    pub min_z: u32,
    pub tile_size: u32,
    pub vsnap: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainRenderViewFrustum {
    side_planes: [[f32; 4]; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainRenderViews {
    pub main: TerrainRenderViewFrustum,
    pub west_wrap: Option<TerrainRenderViewFrustum>,
    pub east_wrap: Option<TerrainRenderViewFrustum>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerrainLodCullRecord {
    pub stride: i8,
    pub visibility_mask: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainLodCullState {
    pub cols: u32,
    pub rows: u32,
    records: Vec<TerrainLodCullRecord>,
    candidate_strides: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainFrustumNeighbour {
    MapEdge,
    CulledOwner,
    OwnerStride(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TerrainDerivedVsnapFailure {
    pub min_x: u32,
    pub min_z: u32,
    pub tile_size: u32,
    pub edge: usize,
    pub captured: f32,
    pub predicted: f32,
    pub neighbour: TerrainFrustumNeighbour,
}

impl TerrainRenderViewFrustum {
    pub fn side_planes_f32_bits(&self) -> [[u32; 4]; 4] {
        self.side_planes.map(|plane| plane.map(f32::to_bits))
    }

    pub fn from_view_projection_matrix(matrix: [[f32; 4]; 4]) -> Result<Self> {
        Self::from_view_projection_matrix_with_bias(matrix, 0.0)
    }

    pub fn from_view_projection_matrix_with_bias(
        matrix: [[f32; 4]; 4],
        side_plane_bias: f32,
    ) -> Result<Self> {
        if !side_plane_bias.is_finite() {
            return Err("terrain LOD law: render-view side-plane bias is non-finite".into());
        }
        let raw = [
            [
                matrix[0][0] + matrix[0][3],
                matrix[1][0] + matrix[1][3],
                matrix[2][0] + matrix[2][3],
                matrix[3][0] + matrix[3][3],
            ],
            [
                matrix[0][3] - matrix[0][0],
                matrix[1][3] - matrix[1][0],
                matrix[2][3] - matrix[2][0],
                matrix[3][3] - matrix[3][0],
            ],
            [
                matrix[0][3] - matrix[0][1],
                matrix[1][3] - matrix[1][1],
                matrix[2][3] - matrix[2][1],
                matrix[3][3] - matrix[3][1],
            ],
            [
                matrix[0][1] + matrix[0][3],
                matrix[1][1] + matrix[1][3],
                matrix[2][1] + matrix[2][3],
                matrix[3][1] + matrix[3][3],
            ],
        ];
        let mut side_planes = [[0.0f32; 4]; 4];
        for (out, plane) in side_planes.iter_mut().zip(raw) {
            let len = (plane[0] * plane[0] + plane[1] * plane[1] + plane[2] * plane[2]).sqrt();
            if !len.is_finite() || len == 0.0 {
                return Err(
                    "terrain LOD law: captured ViewProjectionMatrix produced a degenerate frustum plane"
                        .into(),
                );
            }
            let inv_len = 1.0 / len;
            for i in 0..4 {
                out[i] = -plane[i] * inv_len;
            }
            out[3] -= side_plane_bias;
        }
        Ok(Self { side_planes })
    }

    pub fn aabb_outside(&self, min_x: f32, min_z: f32, tile_size: f32) -> bool {
        self.aabb_outside_with_x_offset(min_x, min_z, tile_size, 0.0)
    }

    pub fn aabb_outside_wrapped(
        &self,
        min_x: f32,
        min_z: f32,
        tile_size: f32,
        map_width: f32,
    ) -> bool {
        [0.0, map_width, -map_width]
            .into_iter()
            .all(|x_offset| self.aabb_outside_with_x_offset(min_x, min_z, tile_size, x_offset))
    }

    pub fn bounds_outside(&self, min: [f32; 3], max: [f32; 3]) -> bool {
        self.side_planes.iter().any(|plane| {
            let x = if plane[0] > 0.0 { min[0] } else { max[0] };
            let y = if plane[1] > 0.0 { min[1] } else { max[1] };
            let z = if plane[2] > 0.0 { min[2] } else { max[2] };
            plane[0] * x + plane[1] * y + plane[2] * z + plane[3] > 0.0
        })
    }

    fn aabb_outside_with_x_offset(
        &self,
        min_x: f32,
        min_z: f32,
        tile_size: f32,
        x_offset: f32,
    ) -> bool {
        let max_x = min_x + tile_size;
        let max_z = min_z + tile_size;
        let y_high = TERRAIN_LOD_REFERENCE_HEIGHT as f32;
        self.side_planes.iter().any(|plane| {
            let x = if plane[0] > 0.0 { min_x } else { max_x } - x_offset;
            let y = if plane[1] > 0.0 { 0.0 } else { y_high };
            let z = if plane[2] > 0.0 { min_z } else { max_z };
            plane[0] * x + plane[1] * y + plane[2] * z + plane[3] > 0.0
        })
    }
}

impl TerrainLodCullState {
    pub fn record_at(&self, col: u32, row: u32) -> Option<TerrainLodCullRecord> {
        if col >= self.cols || row >= self.rows {
            return None;
        }
        Some(self.records[(row * self.cols + col) as usize])
    }

    pub fn draw_origins(&self, render_view_mask: u8) -> Vec<(u32, u32, u32)> {
        let mut out = Vec::new();
        for row in 0..self.rows {
            for col in 0..self.cols {
                let record = self.records[(row * self.cols + col) as usize];
                if record.stride > 0 && record.visibility_mask & render_view_mask != 0 {
                    out.push((
                        col * TERRAIN_LOD_BASE_TILE_SIZE,
                        row * TERRAIN_LOD_BASE_TILE_SIZE,
                        record.stride as u32 * TERRAIN_LOD_BASE_TILE_SIZE,
                    ));
                }
            }
        }
        out
    }

    pub fn draw_tiles(&self, render_view_mask: u8) -> Vec<TerrainLodTile> {
        let mut out = Vec::new();
        for row in 0..self.rows {
            for col in 0..self.cols {
                let record = self.records[(row * self.cols + col) as usize];
                if record.stride <= 0 || record.visibility_mask & render_view_mask == 0 {
                    continue;
                }
                let stride = record.stride as u32;
                let tile_size = stride * TERRAIN_LOD_BASE_TILE_SIZE;
                let neighbours = [
                    self.draw_neighbour_z(col, row, -1, render_view_mask),
                    self.draw_neighbour_z(col, row, stride as i64, render_view_mask),
                    self.draw_neighbour_x(col, row, -1, render_view_mask),
                    self.draw_neighbour_x(col, row, stride as i64, render_view_mask),
                ];
                let mut vsnap = [0.0; TERRAIN_LOD_EDGE_COUNT];
                for (edge, neighbour) in neighbours.into_iter().enumerate() {
                    vsnap[edge] = match neighbour {
                        TerrainFrustumNeighbour::CulledOwner => 1.0,
                        TerrainFrustumNeighbour::OwnerStride(other) if other > stride => 1.0,
                        TerrainFrustumNeighbour::MapEdge
                        | TerrainFrustumNeighbour::OwnerStride(_) => 0.0,
                    };
                }
                out.push(TerrainLodTile {
                    min_x: col * TERRAIN_LOD_BASE_TILE_SIZE,
                    min_z: row * TERRAIN_LOD_BASE_TILE_SIZE,
                    tile_size,
                    vsnap,
                });
            }
        }
        out
    }

    fn draw_neighbour_z(
        &self,
        col: u32,
        row: u32,
        delta: i64,
        render_view_mask: u8,
    ) -> TerrainFrustumNeighbour {
        let neighbour_row = row as i64 + delta;
        if neighbour_row < 0 || neighbour_row >= self.rows as i64 {
            return TerrainFrustumNeighbour::MapEdge;
        }
        self.draw_neighbour(col, neighbour_row as u32, render_view_mask)
    }

    fn draw_neighbour_x(
        &self,
        col: u32,
        row: u32,
        delta: i64,
        render_view_mask: u8,
    ) -> TerrainFrustumNeighbour {
        let cols = self.cols as i64;
        let neighbour_col = (((col as i64 + delta) % cols) + cols) % cols;
        self.draw_neighbour(neighbour_col as u32, row, render_view_mask)
    }

    fn draw_neighbour(&self, col: u32, row: u32, render_view_mask: u8) -> TerrainFrustumNeighbour {
        let index = (row * self.cols + col) as usize;
        let stride = self.candidate_strides[index] as u32;
        if col % stride == 0
            && row % stride == 0
            && self.records[index].visibility_mask & render_view_mask == 0
        {
            TerrainFrustumNeighbour::CulledOwner
        } else {
            TerrainFrustumNeighbour::OwnerStride(stride)
        }
    }

    pub fn deterministic_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.records.len() * 2);
        for record in &self.records {
            bytes.push(record.stride as u8);
            bytes.push(record.visibility_mask);
        }
        bytes
    }
}

impl TerrainLodStrideMap {
    pub fn stride_at(&self, col: u32, row: u32) -> u32 {
        self.strides[(row * self.cols + col) as usize]
    }

    pub fn owner_tiles(&self) -> Vec<TerrainLodTile> {
        let mut tiles = Vec::new();
        for row in 0..self.rows {
            for col in 0..self.cols {
                let stride = self.stride_at(col, row);
                if col % stride != 0 || row % stride != 0 {
                    continue;
                }
                let this_size = stride * TERRAIN_LOD_BASE_TILE_SIZE;
                let neigh = [
                    self.neighbour_size_z(col, row, -1),
                    self.neighbour_size_z(col, row, stride as i64),
                    self.neighbour_size_x_wrap(col, row, -1),
                    self.neighbour_size_x_wrap(col, row, stride as i64),
                ];
                tiles.push(TerrainLodTile {
                    min_x: col * TERRAIN_LOD_BASE_TILE_SIZE,
                    min_z: row * TERRAIN_LOD_BASE_TILE_SIZE,
                    tile_size: this_size,
                    vsnap: terrain_vsnap_from_neighbour_lods(this_size, neigh),
                });
            }
        }
        tiles
    }

    fn neighbour_size_z(&self, col: u32, row: u32, delta: i64) -> Option<u32> {
        let r = row as i64 + delta;
        if r < 0 || r >= self.rows as i64 {
            return None;
        }
        Some(self.stride_at(col, r as u32) * TERRAIN_LOD_BASE_TILE_SIZE)
    }

    fn neighbour_size_x_wrap(&self, col: u32, row: u32, delta: i64) -> Option<u32> {
        let cols = self.cols as i64;
        let c = (((col as i64 + delta) % cols) + cols) % cols;
        Some(self.stride_at(c as u32, row) * TERRAIN_LOD_BASE_TILE_SIZE)
    }
}

pub fn terrain_lod_stride_map(
    cam_ref: [f32; 3],
    map_width: u32,
    map_height: u32,
) -> Result<TerrainLodStrideMap> {
    terrain_lod_stride_map_with_wrap_views(cam_ref, map_width, map_height, true, true)
}

fn terrain_lod_stride_map_with_wrap_views(
    cam_ref: [f32; 3],
    map_width: u32,
    map_height: u32,
    west_wrap_enabled: bool,
    east_wrap_enabled: bool,
) -> Result<TerrainLodStrideMap> {
    if map_width % TERRAIN_LOD_BASE_TILE_SIZE != 0 || map_height % TERRAIN_LOD_BASE_TILE_SIZE != 0 {
        return Err(format!(
            "terrain LOD law: world {map_width}x{map_height} is not a multiple of {TERRAIN_LOD_BASE_TILE_SIZE}"
        )
        .into());
    }
    if cam_ref.iter().any(|v| !v.is_finite()) {
        return Err("terrain LOD law: camera reference position is non-finite".into());
    }
    let cols = map_width / TERRAIN_LOD_BASE_TILE_SIZE;
    let rows = map_height / TERRAIN_LOD_BASE_TILE_SIZE;
    let max_lod = terrain_max_lod_for_dimensions(cols, rows);
    let cx = cam_ref[0] as f64;
    let cy = cam_ref[1] as f64;
    let cz = cam_ref[2] as f64;
    let map_x = map_width as f64;
    let dy2 = (TERRAIN_LOD_REFERENCE_HEIGHT - cy).powi(2);
    let cell = TERRAIN_LOD_BASE_TILE_SIZE as f64;

    let mut base = vec![0.0f64; (cols * rows) as usize];
    for row in 0..rows {
        for col in 0..cols {
            let mut dmin = f64::INFINITY;
            for czp in [row as f64 * cell, (row as f64 + 1.0) * cell] {
                let dz2 = (czp - cz).powi(2);
                for cxp in [col as f64 * cell, (col as f64 + 1.0) * cell] {
                    let wraps = [
                        Some(0.0),
                        west_wrap_enabled.then_some(map_x),
                        east_wrap_enabled.then_some(-map_x),
                    ];
                    for wrap in wraps.into_iter().flatten() {
                        let dx = cxp - cx + wrap;
                        let d2 = dx * dx + dz2 + dy2;
                        if d2 < dmin {
                            dmin = d2;
                        }
                    }
                }
            }
            base[(row * cols + col) as usize] = dmin;
        }
    }

    let mut mips: Vec<(u32, u32, Vec<f64>)> = vec![(cols, rows, base)];
    for _ in 0..max_lod {
        let (pw, ph, prev) = mips.last().unwrap();
        let (pw, ph) = (*pw, *ph);
        let prev = prev.clone();
        let w = pw.div_ceil(2);
        let h = ph.div_ceil(2);
        let mut cur = vec![f64::INFINITY; (w * h) as usize];
        for r in 0..ph {
            for c in 0..pw {
                let v = prev[(r * pw + c) as usize];
                let idx = ((r / 2) * w + (c / 2)) as usize;
                if v < cur[idx] {
                    cur[idx] = v;
                }
            }
        }
        mips.push((w, h, cur));
    }

    let mut strides = vec![1u32; (cols * rows) as usize];
    for row in 0..rows {
        for col in 0..cols {
            let mut lod = 0u32;
            for l in (0..max_lod).rev() {
                let idx = (l + 1) as usize;
                let (mw, _mh, buf) = &mips[idx];
                let mc = col >> (l + 1);
                let mr = row >> (l + 1);
                let block_min = buf[(mr * *mw + mc) as usize];
                if block_min > TERRAIN_LOD_THRESHOLDS_SQ[l as usize] {
                    lod = (l + 1) as u32;
                    break;
                }
            }
            strides[(row * cols + col) as usize] = 1u32 << lod;
        }
    }
    Ok(TerrainLodStrideMap {
        cols,
        rows,
        strides,
    })
}

fn terrain_max_lod_for_dimensions(cols: u32, rows: u32) -> u32 {
    fn dimension_cap(size: u32) -> u32 {
        let mut divisor = 16;
        let mut lod = 4;
        while divisor > 2 && size % divisor != 0 {
            divisor /= 2;
            lod -= 1;
        }
        lod
    }
    dimension_cap(cols).min(dimension_cap(rows))
}

pub fn terrain_lod_cull_update(
    previous: Option<&TerrainLodCullState>,
    cam_ref: [f32; 3],
    map_width: u32,
    map_height: u32,
    views: &TerrainRenderViews,
    options: TerrainLodCullOptions,
) -> Result<TerrainLodCullState> {
    let west = views.west_wrap.is_some();
    let east = views.east_wrap.is_some();
    let candidates =
        terrain_lod_stride_map_with_wrap_views(cam_ref, map_width, map_height, west, east)?;
    let len = (candidates.cols * candidates.rows) as usize;
    let mut state = match previous {
        Some(old) if old.cols == candidates.cols && old.rows == candidates.rows => old.clone(),
        Some(_) => {
            return Err("terrain LOD law: previous map+0x4970 dimensions do not match".into())
        }
        None => TerrainLodCullState {
            cols: candidates.cols,
            rows: candidates.rows,
            records: vec![TerrainLodCullRecord::default(); len],
            candidate_strides: vec![1; len],
        },
    };
    state.candidate_strides = candidates
        .strides
        .iter()
        .map(|stride| *stride as u8)
        .collect();

    for row in 0..candidates.rows {
        for col in 0..candidates.cols {
            let index = (row * candidates.cols + col) as usize;
            let stride = candidates.stride_at(col, row);
            let record = &mut state.records[index];
            if col % stride != 0 || row % stride != 0 {
                record.stride = -1;
                continue;
            }

            record.visibility_mask = 0;
            let min_x = (col * TERRAIN_LOD_BASE_TILE_SIZE) as f32;
            let min_z = (row * TERRAIN_LOD_BASE_TILE_SIZE) as f32;
            let tile_size = (stride * TERRAIN_LOD_BASE_TILE_SIZE) as f32;
            if !views.main.aabb_outside(min_x, min_z, tile_size) {
                record.visibility_mask |= 1;
            }
            if let Some(view) = views.west_wrap {
                if !view.aabb_outside(min_x, min_z, tile_size) {
                    record.visibility_mask |= 2;
                }
            }
            if let Some(view) = views.east_wrap {
                if !view.aabb_outside(min_x, min_z, tile_size) {
                    record.visibility_mask |= 4;
                }
            }

            if record.visibility_mask != 0 {
                record.stride = stride as i8;
                if stride == 1
                    && options.suppress_near_stride_one
                    && nearest_tile_corner_distance_sq(col, row, cam_ref, map_width, west, east)
                        < 5625.0
                {
                    record.stride = 0;
                }
            }
        }
    }
    Ok(state)
}

fn nearest_tile_corner_distance_sq(
    col: u32,
    row: u32,
    cam_ref: [f32; 3],
    map_width: u32,
    west_wrap_enabled: bool,
    east_wrap_enabled: bool,
) -> f64 {
    let cell = TERRAIN_LOD_BASE_TILE_SIZE as f64;
    let cx = cam_ref[0] as f64;
    let cz = cam_ref[2] as f64;
    let dy2 = (TERRAIN_LOD_REFERENCE_HEIGHT - cam_ref[1] as f64).powi(2);
    let wraps = [
        Some(0.0),
        west_wrap_enabled.then_some(map_width as f64),
        east_wrap_enabled.then_some(-(map_width as f64)),
    ];
    let mut nearest = f64::INFINITY;
    for z in [row as f64 * cell, (row + 1) as f64 * cell] {
        for x in [col as f64 * cell, (col + 1) as f64 * cell] {
            for wrap in wraps.into_iter().flatten() {
                nearest = nearest.min((x - cx + wrap).powi(2) + (z - cz).powi(2) + dy2);
            }
        }
    }
    nearest
}

pub fn terrain_lod_tiles(
    cam_ref: [f32; 3],
    map_width: u32,
    map_height: u32,
) -> Result<Vec<TerrainLodTile>> {
    Ok(terrain_lod_stride_map(cam_ref, map_width, map_height)?.owner_tiles())
}

impl TerrainLodStrideMap {
    pub fn vsnap_for_owner(&self, col: u32, row: u32) -> [f32; 4] {
        let stride = self.stride_at(col, row);
        let this = stride * TERRAIN_LOD_BASE_TILE_SIZE;
        let neigh = [
            self.neighbour_size_z(col, row, -1),
            self.neighbour_size_z(col, row, stride as i64),
            self.neighbour_size_x_wrap(col, row, -1),
            self.neighbour_size_x_wrap(col, row, stride as i64),
        ];
        terrain_vsnap_from_neighbour_lods(this, neigh)
    }

    pub fn vsnap_for_owner_with_frustum(
        &self,
        col: u32,
        row: u32,
        frustum: &TerrainRenderViewFrustum,
        map_width: u32,
    ) -> ([f32; 4], [TerrainFrustumNeighbour; TERRAIN_LOD_EDGE_COUNT]) {
        let stride = self.stride_at(col, row);
        let neigh = [
            self.frustum_neighbour_z(col, row, -1, frustum, map_width),
            self.frustum_neighbour_z(col, row, stride as i64, frustum, map_width),
            self.frustum_neighbour_x_wrap(col, row, -1, frustum, map_width),
            self.frustum_neighbour_x_wrap(col, row, stride as i64, frustum, map_width),
        ];
        let mut vsnap = [0.0f32; TERRAIN_LOD_EDGE_COUNT];
        for (i, neighbour) in neigh.iter().enumerate() {
            vsnap[i] = match neighbour {
                TerrainFrustumNeighbour::CulledOwner => 1.0,
                TerrainFrustumNeighbour::OwnerStride(other) if *other > stride => 1.0,
                TerrainFrustumNeighbour::MapEdge | TerrainFrustumNeighbour::OwnerStride(_) => 0.0,
            };
        }
        (vsnap, neigh)
    }

    fn frustum_neighbour_z(
        &self,
        col: u32,
        row: u32,
        delta: i64,
        frustum: &TerrainRenderViewFrustum,
        map_width: u32,
    ) -> TerrainFrustumNeighbour {
        let r = row as i64 + delta;
        if r < 0 || r >= self.rows as i64 {
            return TerrainFrustumNeighbour::MapEdge;
        }
        self.frustum_cell_value(col, r as u32, frustum, map_width)
    }

    fn frustum_neighbour_x_wrap(
        &self,
        col: u32,
        row: u32,
        delta: i64,
        frustum: &TerrainRenderViewFrustum,
        map_width: u32,
    ) -> TerrainFrustumNeighbour {
        let cols = self.cols as i64;
        let c = (((col as i64 + delta) % cols) + cols) % cols;
        self.frustum_cell_value(c as u32, row, frustum, map_width)
    }

    fn frustum_cell_value(
        &self,
        col: u32,
        row: u32,
        frustum: &TerrainRenderViewFrustum,
        map_width: u32,
    ) -> TerrainFrustumNeighbour {
        let stride = self.stride_at(col, row);
        if col % stride != 0 || row % stride != 0 {
            return TerrainFrustumNeighbour::OwnerStride(stride);
        }
        let tile_size = (stride * TERRAIN_LOD_BASE_TILE_SIZE) as f32;
        let min_x = (col * TERRAIN_LOD_BASE_TILE_SIZE) as f32;
        let min_z = (row * TERRAIN_LOD_BASE_TILE_SIZE) as f32;
        if frustum.aabb_outside_wrapped(min_x, min_z, tile_size, map_width as f32) {
            TerrainFrustumNeighbour::CulledOwner
        } else {
            TerrainFrustumNeighbour::OwnerStride(stride)
        }
    }
}

#[derive(Clone, Debug)]
pub struct TerrainLodCorpusReport {
    pub session_label: String,
    pub captured_tiles: usize,
    pub self_lod_matched: usize,
    pub disagree: Vec<(u32, u32, u32, u32)>,
    pub derived_vsnap_edges_matched: usize,
    pub frustum_vsnap_edges_matched: usize,
    pub frustum_self_visible: usize,
    pub frustum_self_culled: Vec<(u32, u32, u32)>,
    pub edges_checked: usize,
    pub frustum_vsnap_failures: Vec<TerrainDerivedVsnapFailure>,
}

impl TerrainLodCorpusReport {
    pub fn self_lod_exact(&self) -> bool {
        self.captured_tiles > 0 && self.self_lod_matched == self.captured_tiles
    }

    pub fn derived_vsnap_fraction(&self) -> f64 {
        if self.edges_checked == 0 {
            0.0
        } else {
            self.derived_vsnap_edges_matched as f64 / self.edges_checked as f64
        }
    }

    pub fn frustum_vsnap_fraction(&self) -> f64 {
        if self.edges_checked == 0 {
            0.0
        } else {
            self.frustum_vsnap_edges_matched as f64 / self.edges_checked as f64
        }
    }
}

pub fn terrain_lod_map_validate_corpus(
    library: &NativeD3d11TerrainChunkLibrary,
    map_width: u32,
    map_height: u32,
    session_label: &str,
) -> Result<TerrainLodCorpusReport> {
    let bundle_dir = library.bundle_dir();
    let mut cache: BTreeMap<(i64, i64, i64), TerrainLodStrideMap> = BTreeMap::new();
    let mut report = TerrainLodCorpusReport {
        session_label: session_label.to_string(),
        captured_tiles: 0,
        self_lod_matched: 0,
        disagree: Vec::new(),
        derived_vsnap_edges_matched: 0,
        frustum_vsnap_edges_matched: 0,
        frustum_self_visible: 0,
        frustum_self_culled: Vec::new(),
        edges_checked: 0,
        frustum_vsnap_failures: Vec::new(),
    };
    for chunk in library.chunk_records() {
        let size = terrain_chunk_world_tile_size(chunk)?;
        let min_x = round_world(chunk.bounds.min_x)?.max(0) as u32;
        let min_z = round_world(chunk.bounds.min_z)?.max(0) as u32;
        let col = min_x / TERRAIN_LOD_BASE_TILE_SIZE;
        let row = min_z / TERRAIN_LOD_BASE_TILE_SIZE;
        let scene = decode_map_frame_terrain_scene_cb0(&chunk.draw, bundle_dir)?;
        let cam = [scene.v_cam_pos.x, scene.v_cam_pos.y, scene.v_cam_pos.z];
        let key = (
            (cam[0] * 100.0).round() as i64,
            (cam[1] * 100.0).round() as i64,
            (cam[2] * 100.0).round() as i64,
        );
        if !cache.contains_key(&key) {
            cache.insert(key, terrain_lod_stride_map(cam, map_width, map_height)?);
        }
        let map = &cache[&key];
        report.captured_tiles += 1;
        if col < map.cols && row < map.rows {
            let algo = map.stride_at(col, row) * TERRAIN_LOD_BASE_TILE_SIZE;
            if algo == size {
                report.self_lod_matched += 1;
            } else {
                report.disagree.push((min_x, min_z, size, algo));
            }
            let predicted = map.vsnap_for_owner(col, row);
            let frustum = TerrainRenderViewFrustum::from_view_projection_matrix(
                scene.view_projection_matrix,
            )?;
            if !frustum.aabb_outside_wrapped(
                min_x as f32,
                min_z as f32,
                size as f32,
                map_width as f32,
            ) {
                report.frustum_self_visible += 1;
            } else {
                report.frustum_self_culled.push((min_x, min_z, size));
            }
            let (frustum_predicted, frustum_neighbours) =
                map.vsnap_for_owner_with_frustum(col, row, &frustum, map_width);
            let captured_flags = terrain_chunk_draw_detail_edge_flags(&chunk.draw, bundle_dir)?;
            for e in 0..TERRAIN_LOD_EDGE_COUNT {
                report.edges_checked += 1;
                if (predicted[e] - captured_flags[e]).abs() <= f32::EPSILON {
                    report.derived_vsnap_edges_matched += 1;
                }
                if (frustum_predicted[e] - captured_flags[e]).abs() <= f32::EPSILON {
                    report.frustum_vsnap_edges_matched += 1;
                } else {
                    report
                        .frustum_vsnap_failures
                        .push(TerrainDerivedVsnapFailure {
                            min_x,
                            min_z,
                            tile_size: size,
                            edge: e,
                            captured: captured_flags[e],
                            predicted: frustum_predicted[e],
                            neighbour: frustum_neighbours[e],
                        });
                }
            }
        }
    }
    Ok(report)
}

pub fn terrain_lod_map_validate_corpus_with_resolver(
    resolver: &GameFileResolver,
    library: &NativeD3d11TerrainChunkLibrary,
    session_label: &str,
) -> Result<TerrainLodCorpusReport> {
    let heightmap_path =
        resolver.resolve_required(TERRAIN_CHUNK_HEIGHTMAP_SOURCE, "heightmap.bmp")?;
    let heightmap = load_bmp_indexed(&heightmap_path)?;
    terrain_lod_map_validate_corpus(
        library,
        heightmap.width as u32,
        heightmap.height as u32,
        session_label,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainVsnapEdgeClass {
    Match,
    MapEdge,
    CoarsestFill,
    UnknownNeighbour,
    CrossFramePan,
    Unexplained,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TerrainVsnapEdgeCheck {
    pub chunk_id: String,
    pub tile_size: u32,
    pub min_x: i64,
    pub min_z: i64,
    pub edge: usize,
    pub captured: f32,
    pub predicted: f32,
    pub neighbour_tile_size: Option<u32>,
    pub class: TerrainVsnapEdgeClass,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TerrainVsnapCorpusReport {
    pub session_label: String,
    pub tiles_checked: usize,
    pub edges_checked: usize,
    pub edges_matched: usize,
    pub map_edges: usize,
    pub coarsest_fills: usize,
    pub unknown_neighbours: usize,
    pub cross_frame_pan: usize,
    pub unexplained: Vec<TerrainVsnapEdgeCheck>,
}

impl TerrainVsnapCorpusReport {
    pub fn passes(&self) -> bool {
        self.edges_checked > 0 && self.unexplained.is_empty()
    }

    pub fn strict_match_fraction(&self) -> f64 {
        if self.edges_checked == 0 {
            0.0
        } else {
            self.edges_matched as f64 / self.edges_checked as f64
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct CapturedTile {
    min_x: i64,
    min_z: i64,
    tile_size: u32,
}

pub fn terrain_vsnap_law_validate_corpus(
    library: &NativeD3d11TerrainChunkLibrary,
    world_width: u32,
    world_height: u32,
    session_label: &str,
) -> Result<TerrainVsnapCorpusReport> {
    let bundle_dir = library.bundle_dir();
    let mut tiles = Vec::new();
    let mut origin_counts: BTreeMap<(i64, i64, u32), usize> = BTreeMap::new();
    for chunk in library.chunk_records() {
        let tile_size = terrain_chunk_world_tile_size(chunk)?;
        let min_x = round_world(chunk.bounds.min_x)?;
        let min_z = round_world(chunk.bounds.min_z)?;
        tiles.push((
            chunk.chunk_id.clone(),
            CapturedTile {
                min_x,
                min_z,
                tile_size,
            },
            terrain_chunk_draw_detail_edge_flags(&chunk.draw, bundle_dir)?,
        ));
        *origin_counts.entry((min_x, min_z, tile_size)).or_insert(0) += 1;
    }
    let placed: Vec<CapturedTile> = tiles.iter().map(|(_, tile, _)| *tile).collect();
    let coarsest = placed.iter().map(|t| t.tile_size).max().unwrap_or(0);

    let mut report = TerrainVsnapCorpusReport {
        session_label: session_label.to_string(),
        tiles_checked: tiles.len(),
        edges_checked: 0,
        edges_matched: 0,
        map_edges: 0,
        coarsest_fills: 0,
        unknown_neighbours: 0,
        cross_frame_pan: 0,
        unexplained: Vec::new(),
    };

    for (chunk_id, tile, captured) in &tiles {
        let this_dup = origin_counts
            .get(&(tile.min_x, tile.min_z, tile.tile_size))
            .copied()
            .unwrap_or(0)
            > 1;
        for edge in TerrainTileEdge::ALL {
            let e = edge as usize;
            let (dx, dz) = edge.neighbour_step();
            let ts = tile.tile_size as i64;
            let n_origin = (tile.min_x + dx * ts, tile.min_z + dz * ts);
            let probe = edge_probe_point(tile, edge);
            let in_world = probe.0 >= 0.0
                && probe.1 >= 0.0
                && probe.0 < world_width as f64
                && probe.1 < world_height as f64;

            let neighbour_tile_size = finest_tile_covering(&placed, probe.0, probe.1);
            let predicted = if neighbour_tile_size.map(|n| n > tile.tile_size) == Some(true) {
                1.0
            } else {
                0.0
            };
            let cap = captured[e];
            report.edges_checked += 1;

            let class = if (predicted - cap).abs() <= f32::EPSILON {
                report.edges_matched += 1;
                TerrainVsnapEdgeClass::Match
            } else if !in_world {
                report.map_edges += 1;
                TerrainVsnapEdgeClass::MapEdge
            } else if neighbour_tile_size.is_none() && tile.tile_size == coarsest {
                report.coarsest_fills += 1;
                TerrainVsnapEdgeClass::CoarsestFill
            } else if this_dup
                || placed.iter().any(|t| {
                    t.tile_size == tile.tile_size
                        && (t.min_x, t.min_z) == n_origin
                        && origin_counts
                            .get(&(t.min_x, t.min_z, t.tile_size))
                            .copied()
                            .unwrap_or(0)
                            > 1
                })
            {
                report.cross_frame_pan += 1;
                TerrainVsnapEdgeClass::CrossFramePan
            } else if neighbour_tile_size.is_none() {
                report.unknown_neighbours += 1;
                TerrainVsnapEdgeClass::UnknownNeighbour
            } else {
                TerrainVsnapEdgeClass::Unexplained
            };

            if class == TerrainVsnapEdgeClass::Unexplained {
                report.unexplained.push(TerrainVsnapEdgeCheck {
                    chunk_id: chunk_id.clone(),
                    tile_size: tile.tile_size,
                    min_x: tile.min_x,
                    min_z: tile.min_z,
                    edge: e,
                    captured: cap,
                    predicted,
                    neighbour_tile_size,
                    class,
                });
            }
        }
    }
    Ok(report)
}

pub fn terrain_vsnap_law_validate_corpus_with_resolver(
    resolver: &GameFileResolver,
    library: &NativeD3d11TerrainChunkLibrary,
    session_label: &str,
) -> Result<TerrainVsnapCorpusReport> {
    let heightmap_path =
        resolver.resolve_required(TERRAIN_CHUNK_HEIGHTMAP_SOURCE, "heightmap.bmp")?;
    let heightmap = load_bmp_indexed(&heightmap_path)?;
    terrain_vsnap_law_validate_corpus(
        library,
        heightmap.width as u32,
        heightmap.height as u32,
        session_label,
    )
}

fn edge_probe_point(tile: &CapturedTile, edge: TerrainTileEdge) -> (f64, f64) {
    let x = tile.min_x as f64;
    let z = tile.min_z as f64;
    let t = tile.tile_size as f64;
    match edge {
        TerrainTileEdge::Bottom => (x + t * 0.5, z - 0.5),
        TerrainTileEdge::Top => (x + t * 0.5, z + t + 0.5),
        TerrainTileEdge::Left => (x - 0.5, z + t * 0.5),
        TerrainTileEdge::Right => (x + t + 0.5, z + t * 0.5),
    }
}

fn finest_tile_covering(tiles: &[CapturedTile], px: f64, pz: f64) -> Option<u32> {
    tiles
        .iter()
        .filter(|t| {
            let ts = t.tile_size as f64;
            px >= t.min_x as f64
                && px < t.min_x as f64 + ts
                && pz >= t.min_z as f64
                && pz < t.min_z as f64 + ts
        })
        .map(|t| t.tile_size)
        .min()
}

fn round_world(value: f32) -> Result<i64> {
    if !value.is_finite() {
        return Err("terrain LOD law: non-finite tile origin".into());
    }
    let rounded = value.round();
    if (value - rounded).abs() > 0.01 {
        return Err(format!("terrain LOD law: tile origin {value} is not integer-aligned").into());
    }
    Ok(rounded as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    mod corpus {
        include!("../test_gates.rs");
    }

    #[test]
    fn dimension_cap_prevents_unadmitted_vanilla_stride_16() {
        assert_eq!(terrain_max_lod_for_dimensions(88, 32), 3);
        assert_eq!(terrain_max_lod_for_dimensions(96, 32), 4);
        let map = terrain_lod_stride_map([2958.0, 3000.0, 1400.0], 5632, 2048)
            .expect("vanilla dimension-capped map");
        assert!(map.strides.iter().all(|stride| *stride <= 8));
    }

    #[test]
    fn render_view_bias_subtracts_from_each_normalized_side_plane_d() {
        let matrix = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let zero = TerrainRenderViewFrustum::from_view_projection_matrix(matrix).unwrap();
        let biased =
            TerrainRenderViewFrustum::from_view_projection_matrix_with_bias(matrix, 2.5).unwrap();
        for plane in 0..4 {
            assert_eq!(biased.side_planes[plane][0], zero.side_planes[plane][0]);
            assert_eq!(biased.side_planes[plane][1], zero.side_planes[plane][1]);
            assert_eq!(biased.side_planes[plane][2], zero.side_planes[plane][2]);
            assert_eq!(
                biased.side_planes[plane][3],
                zero.side_planes[plane][3] - 2.5
            );
        }
        assert!(
            TerrainRenderViewFrustum::from_view_projection_matrix_with_bias(matrix, f32::NAN)
                .is_err()
        );
    }

    #[test]
    fn stateful_cull_retains_owner_stride_when_all_views_reject_it() {
        let inside = TerrainRenderViewFrustum {
            side_planes: [[0.0, 0.0, 0.0, -1.0]; 4],
        };
        let outside = TerrainRenderViewFrustum {
            side_planes: [[0.0, 0.0, 0.0, 1.0]; 4],
        };
        let camera = [2821.977_3, 3000.0, 988.758_96];
        let visible = terrain_lod_cull_update(
            None,
            camera,
            5632,
            2048,
            &TerrainRenderViews {
                main: inside,
                west_wrap: None,
                east_wrap: None,
            },
            TerrainLodCullOptions::default(),
        )
        .unwrap();
        let prior = visible.record_at(0, 0).unwrap();
        assert!(prior.stride > 0);
        assert_eq!(prior.visibility_mask, 1);

        let culled = terrain_lod_cull_update(
            Some(&visible),
            camera,
            5632,
            2048,
            &TerrainRenderViews {
                main: outside,
                west_wrap: None,
                east_wrap: None,
            },
            TerrainLodCullOptions::default(),
        )
        .unwrap();
        let retained = culled.record_at(0, 0).unwrap();
        assert_eq!(retained.stride, prior.stride);
        assert_eq!(retained.visibility_mask, 0);
        assert!(culled.draw_origins(1).is_empty());
    }

    #[test]
    fn optional_wrap_masks_and_near_stride_one_suppression_are_exact() {
        let inside = TerrainRenderViewFrustum {
            side_planes: [[0.0, 0.0, 0.0, -1.0]; 4],
        };
        let views = TerrainRenderViews {
            main: inside,
            west_wrap: Some(inside),
            east_wrap: Some(inside),
        };
        let camera = [32.0, TERRAIN_LOD_REFERENCE_HEIGHT as f32, 32.0];
        let unsuppressed = terrain_lod_cull_update(
            None,
            camera,
            128,
            128,
            &views,
            TerrainLodCullOptions::default(),
        )
        .unwrap();
        assert_eq!(
            unsuppressed.record_at(0, 0),
            Some(TerrainLodCullRecord {
                stride: 1,
                visibility_mask: 7,
            })
        );

        let suppressed = terrain_lod_cull_update(
            None,
            camera,
            128,
            128,
            &views,
            TerrainLodCullOptions {
                suppress_near_stride_one: true,
                ..TerrainLodCullOptions::default()
            },
        )
        .unwrap();
        assert_eq!(
            suppressed.record_at(0, 0),
            Some(TerrainLodCullRecord {
                stride: 0,
                visibility_mask: 7,
            })
        );
        assert!(suppressed.draw_origins(1).is_empty());
        assert_eq!(
            suppressed.deterministic_bytes()[..2],
            [0, 7],
            "map+0x4970 byte order is signed stride then visibility mask"
        );
    }

    #[test]
    fn vsnap_flags_only_coarser_neighbours() {
        let vsnap =
            terrain_vsnap_from_neighbour_lods(128, [Some(256), Some(64), Some(256), Some(128)]);
        assert_eq!(vsnap, [1.0, 0.0, 1.0, 0.0]);
    }

    #[test]
    fn vsnap_zero_for_uniform_lod_and_map_edges() {
        assert_eq!(
            terrain_vsnap_from_neighbour_lods(256, [Some(256); 4]),
            [0.0; 4]
        );
        assert_eq!(
            terrain_vsnap_from_neighbour_lods(512, [None, None, None, None]),
            [0.0; 4]
        );
    }

    #[test]
    fn vsnap_never_flags_finer_neighbours() {
        let vsnap = terrain_vsnap_from_neighbour_lods(512, [Some(128), Some(256), None, Some(512)]);
        assert_eq!(vsnap, [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn stitch_reciprocity_holds_across_a_seam() {
        let fine_right = terrain_vsnap_from_neighbour_lods(128, [None, None, None, Some(256)]);
        let coarse_left = terrain_vsnap_from_neighbour_lods(256, [None, None, Some(128), None]);
        assert_eq!(fine_right[TerrainTileEdge::Right as usize], 1.0);
        assert_eq!(coarse_left[TerrainTileEdge::Left as usize], 0.0);
    }

    #[test]
    fn edge_indices_match_slot0_edge_classes() {
        assert_eq!(TerrainTileEdge::Bottom as usize, 1 - 1);
        assert_eq!(TerrainTileEdge::Top as usize, 2 - 1);
        assert_eq!(TerrainTileEdge::Left as usize, 3 - 1);
        assert_eq!(TerrainTileEdge::Right as usize, 4 - 1);
    }

    #[test]
    fn lod_thresholds_are_the_clean_squared_distances() {
        assert_eq!(TERRAIN_LOD_THRESHOLDS_SQ[0], 150.0 * 150.0);
        assert_eq!(TERRAIN_LOD_THRESHOLDS_SQ[1], 350.0 * 350.0);
        assert_eq!(TERRAIN_LOD_THRESHOLDS_SQ[2], 750.0 * 750.0);
        assert_eq!(TERRAIN_LOD_THRESHOLDS_SQ[3], 1600.0 * 1600.0);
        assert_eq!(TERRAIN_LOD_REFERENCE_HEIGHT, 0.1 * 255.0);
    }

    #[test]
    fn high_camera_coarsens_the_whole_map() {
        let map = terrain_lod_stride_map([2958.0, 800.0, 1400.0], 5632, 2048).unwrap();
        let col = 2816 / 64;
        let row = 1408 / 64;
        assert_eq!(map.stride_at(col, row), 8);
    }

    #[test]
    fn low_camera_rings_finest_at_the_ground_point() {
        let map = terrain_lod_stride_map([2737.0, 270.0, 1555.0], 5632, 2048).unwrap();
        let near = map.stride_at(2737 / 64, 1555 / 64);
        let far = map.stride_at(4600 / 64, 300 / 64);
        assert!(near < far, "near {near} should be finer than far {far}");
        assert!(near <= 2, "near the ground point should be 64 or 128");
    }

    #[test]
    fn owner_tiles_are_aligned_and_carry_stitch_flags() {
        let map = terrain_lod_stride_map([2737.0, 300.0, 1555.0], 5632, 2048).unwrap();
        let tiles = map.owner_tiles();
        assert!(!tiles.is_empty());
        for t in &tiles {
            assert_eq!(t.min_x % t.tile_size, 0);
            assert_eq!(t.min_z % t.tile_size, 0);
            for f in t.vsnap {
                assert!(f == 0.0 || f == 1.0);
            }
        }
    }

    #[test]
    #[cfg_attr(
        not(gate_terrain_mesh_corpus),
        ignore = "a terrain chunk corpus and the game install (TERRAIN_MESH_LAW_CORPUS_CAPTURE plus TERRAIN_MESH_LAW_CORPUS_GAME, else preserved gate18_final plus the default install)"
    )]
    fn lod_selection_gate_env_when_session_available() {
        let (capture, game) = corpus::terrain_mesh_corpus()
            .expect("the terrain mesh corpus gate was true at build time");
        let resolver = GameFileResolver::vanilla(game).expect("vanilla resolver");
        let library = NativeD3d11TerrainChunkLibrary::load_records_only(&capture)
            .expect("load chunk library records");
        let report =
            terrain_lod_map_validate_corpus_with_resolver(&resolver, &library, "env-lodring")
                .expect("run LODRING gate");
        eprintln!(
            "LODRING gate: self-LOD {}/{} (byte-exact); derived-field vSnap {}/{}; \
             frustum-field vSnap {}/{}; frustum self-visible {}/{}; \
             frustum residual_edges={} self_culled={:?}",
            report.self_lod_matched,
            report.captured_tiles,
            report.derived_vsnap_edges_matched,
            report.edges_checked,
            report.frustum_vsnap_edges_matched,
            report.edges_checked,
            report.frustum_self_visible,
            report.captured_tiles,
            report.frustum_vsnap_failures.len(),
            report.frustum_self_culled,
        );
        if !report.frustum_vsnap_failures.is_empty() {
            let mut edge_counts = [0usize; TERRAIN_LOD_EDGE_COUNT];
            let mut map_boundary = 0usize;
            let mut interior_owner = 0usize;
            let mut culled_owner = 0usize;
            let mut captured_one_predicted_zero = 0usize;
            let mut captured_zero_predicted_one = 0usize;
            for failure in &report.frustum_vsnap_failures {
                edge_counts[failure.edge] += 1;
                match failure.neighbour {
                    TerrainFrustumNeighbour::MapEdge => map_boundary += 1,
                    TerrainFrustumNeighbour::CulledOwner => culled_owner += 1,
                    TerrainFrustumNeighbour::OwnerStride(_) => interior_owner += 1,
                }
                if failure.captured == 1.0 && failure.predicted == 0.0 {
                    captured_one_predicted_zero += 1;
                } else if failure.captured == 0.0 && failure.predicted == 1.0 {
                    captured_zero_predicted_one += 1;
                }
            }
            let failure_tiles: Vec<_> = report
                .frustum_vsnap_failures
                .iter()
                .map(|failure| {
                    (
                        failure.min_x,
                        failure.min_z,
                        failure.tile_size,
                        failure.edge,
                        failure.captured as i32,
                        failure.predicted as i32,
                        failure.neighbour,
                    )
                })
                .collect();
            eprintln!(
                "LODRING frustum residual census: edge_counts={:?} map_boundary={} \
                 interior_owner={} culled_owner={} captured1_pred0={} captured0_pred1={}",
                edge_counts,
                map_boundary,
                interior_owner,
                culled_owner,
                captured_one_predicted_zero,
                captured_zero_predicted_one,
            );
            eprintln!(
                "LODRING frustum residual tiles{}: {:?}",
                failure_tiles.len(),
                failure_tiles
            );
            let preview: Vec<_> = report.frustum_vsnap_failures.iter().take(12).collect();
            eprintln!(
                "LODRING frustum residual first{}: {:?}",
                preview.len(),
                preview
            );
        }
        assert!(
            report.self_lod_exact(),
            "LODRING did not reproduce the captured LOD assignment: {}/{} (disagreements {:?})",
            report.self_lod_matched,
            report.captured_tiles,
            report.disagree,
        );
    }

    #[test]
    #[cfg_attr(
        not(gate_terrain_mesh_corpus),
        ignore = "a terrain chunk corpus and the game install (TERRAIN_MESH_LAW_CORPUS_CAPTURE plus TERRAIN_MESH_LAW_CORPUS_GAME, else preserved gate18_final plus the default install)"
    )]
    fn corpus_gate_env_when_session_available() {
        let (capture, game) = corpus::terrain_mesh_corpus()
            .expect("the terrain mesh corpus gate was true at build time");
        let resolver = GameFileResolver::vanilla(game).expect("vanilla resolver");
        let library = NativeD3d11TerrainChunkLibrary::load_records_only(&capture)
            .expect("load chunk library records");
        let report =
            terrain_vsnap_law_validate_corpus_with_resolver(&resolver, &library, "env-lodsnap")
                .expect("run LODSNAP corpus gate");
        eprintln!(
            "LODSNAP gate: {}/{} edges matched directly; map_edge={} coarsest_fill={} \
             unknown_neighbour={} cross_frame_pan={} unexplained={}",
            report.edges_matched,
            report.edges_checked,
            report.map_edges,
            report.coarsest_fills,
            report.unknown_neighbours,
            report.cross_frame_pan,
            report.unexplained.len(),
        );
        assert!(
            report.passes(),
            "LODSNAP law left unexplained vSnap mismatches: {:?}",
            report.unexplained
        );
    }
}
