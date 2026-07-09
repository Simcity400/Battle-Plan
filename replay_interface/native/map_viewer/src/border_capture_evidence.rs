//! BORD3 — decode & classify the one captured "borders_pass" IA geometry.
//!
//! BORDER_LAW.md flagged the single `diag23b` `borders_pass` capture as an
//! *unresolved* unknown; this module turns that suspicion into a **decoded,
//! regression-protected verdict**. It reads the recorded IA vertex/index layout
//! (as the bundle JSON reports it) and classifies whether the geometry is a real
//! border polyline draw or a mis-captured terrain tessellation patch.
//!
//! # Why this matters (the standing "don't make things up" rule)
//!
//! The temptation is to treat *any* buffer captured at the borders_pass moment
//! as border ground truth. The `diag23b` capture is a trap: its recorded
//! POSITION stream is `R16G16B16A16_SINT` (stride 8) and decodes to a perfect
//! 65×65 integer grid (`vertex i = (i % 65, i / 65, …)`), its index buffer is a
//! 64×64-cell grid tessellation (`0,1,66 / 0,66,65`, row stride 65), and its
//! recorded `BorderDiffuse` SRV payload is all-zero with pre==post render-target
//! bytes (the draw changed nothing). That is a **terrain tessellation patch**,
//! not border geometry, so it must NOT be used to derive width/UV/tint. This
//! module encodes the grid-signature test so a future reader (or a fresh
//! capture) is classified automatically instead of being trusted on faith.
//!
//! # The border-geometry contract this checks against
//!
//! A *genuine* synthesized border draw ([`crate::border_world_synthesis`]) has:
//! POSITION `R32G32B32_FLOAT` + TEXCOORD `R32G32_SINT`, one interleaved slot,
//! stride 20; indices in 6-per-quad runs; world-space float positions spanning
//! the 5632×2048 map, not a tiny `0..64` integer grid. The captured `diag23b`
//! draw matches NONE of that. The classifier below returns
//! [`CapturedBorderGeometryKind::TerrainGridPatch`] for it and
//! [`CapturedBorderGeometryKind::BorderPolylines`] only for geometry that fits
//! the border contract.

/// DXGI formats referenced by the captured/expected IA layouts (the numeric
/// codes the bundle JSON stores). Named here so the classifier reads in terms of
/// formats, not magic numbers.
pub mod dxgi {
    /// `DXGI_FORMAT_R32G32B32_FLOAT` — the synthesized border POSITION format.
    pub const R32G32B32_FLOAT: u32 = 6;
    /// `DXGI_FORMAT_R16G16B16A16_SINT` — the captured `diag23b` POSITION format
    /// (4×i16 grid coords, stride 8).
    pub const R16G16B16A16_SINT: u32 = 14;
    /// `DXGI_FORMAT_R32G32_SINT` — the synthesized border TEXCOORD format.
    pub const R32G32_SINT: u32 = 18;
    /// `DXGI_FORMAT_R16G16_SINT` — the captured `diag23b` TEXCOORD format.
    pub const R16G16_SINT: u32 = 38;
    /// `DXGI_FORMAT_R16_UINT` — the captured index format.
    pub const R16_UINT: u32 = 57;
    /// `DXGI_FORMAT_R32_UINT` — the synthesized index format.
    #[cfg(test)]
    pub const R32_UINT: u32 = 42;
}

/// The IA layout the capture (or a candidate draw) declares, distilled to the
/// fields the classifier needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapturedBorderIaLayout {
    /// POSITION element DXGI format.
    pub position_format: u32,
    /// POSITION vertex buffer stride (bytes).
    pub position_stride: u32,
    /// TEXCOORD element DXGI format.
    pub texcoord_format: u32,
    /// TEXCOORD vertex buffer stride (bytes).
    pub texcoord_stride: u32,
    /// Index buffer DXGI format.
    pub index_format: u32,
    /// Number of indices the draw issued (`draw_count`).
    pub index_count: u32,
    /// Number of vertices in the POSITION stream.
    pub vertex_count: u32,
}

/// Whether a captured/candidate border draw is real border geometry or the
/// mis-captured terrain grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapturedBorderGeometryKind {
    /// Real per-segment border polyline quads: float positions, 6-index quads.
    BorderPolylines,
    /// A gridded terrain tessellation patch mis-recorded at the borders moment
    /// (int16 grid positions, an `n×n`-cell quad grid). Carries the detected
    /// grid side length (cells) so callers can report it.
    TerrainGridPatch { grid_side_cells: u32 },
    /// Neither signature matched cleanly — unresolved, needs a fresh capture.
    Unresolved,
}

/// The `diag23b` capture's layout, exactly as recorded in
/// `map_frame_borders_pass_bundle.json` (POSITION fmt 14 / stride 8, TEXCOORD
/// fmt 38 / stride 4, index fmt 57, 24576 indices, 4225 vertices). Kept as a
/// constant so a change to the capture — or a re-decode disagreeing with it —
/// fails a test loudly.
pub const DIAG23B_BORDERS_PASS_LAYOUT: CapturedBorderIaLayout = CapturedBorderIaLayout {
    position_format: dxgi::R16G16B16A16_SINT,
    position_stride: 8,
    texcoord_format: dxgi::R16G16_SINT,
    texcoord_stride: 4,
    index_format: dxgi::R16_UINT,
    index_count: 24576,
    vertex_count: 4225,
};

/// The IA layout the synthesized BORD2/BORD3 border pass uploads
/// (`border_pass.rs`): interleaved float POSITION + int TEXCOORD, stride 20,
/// R32 indices. The reference the classifier compares a candidate against.
pub const SYNTHESIZED_BORDER_LAYOUT_POSITION_FORMAT: u32 = dxgi::R32G32B32_FLOAT;
pub const SYNTHESIZED_BORDER_LAYOUT_TEXCOORD_FORMAT: u32 = dxgi::R32G32_SINT;
pub const SYNTHESIZED_BORDER_VERTEX_STRIDE: u32 = 20;

/// If `vertex_count` is a perfect square `s²` and `index_count == (s-1)² * 6`,
/// the geometry is an `(s-1)×(s-1)`-cell grid tessellation — return the grid
/// side. This is the exact shape a terrain patch has and a border polyline draw
/// never has (borders are `4` verts / `6` indices per independent segment, so
/// `index_count == vertex_count / 4 * 6 == vertex_count * 1.5`, not a grid law).
fn detect_grid_side_cells(vertex_count: u32, index_count: u32) -> Option<u32> {
    if vertex_count == 0 {
        return None;
    }
    let side = (vertex_count as f64).sqrt().round() as u32;
    if side < 2 || side * side != vertex_count {
        return None;
    }
    let cells = side - 1;
    // A 1×1 "grid" is just a single quad (4 verts / 6 indices) — the shape a
    // lone border segment has. A terrain tessellation patch is many cells, so
    // require at least a 2×2 grid before calling it a grid; this keeps a single
    // border quad from being mis-classified as a terrain patch.
    if cells >= 2 && cells * cells * 6 == index_count {
        Some(cells)
    } else {
        None
    }
}

/// Classify a captured/candidate border IA layout as real border geometry, a
/// mis-captured terrain grid, or unresolved — from the layout alone (no buffer
/// bytes needed; the format+count signature is decisive).
pub fn classify_captured_border_geometry(
    layout: &CapturedBorderIaLayout,
) -> CapturedBorderGeometryKind {
    // Terrain-grid signature: int16 grid positions AND a perfect-grid
    // vertex/index count. Both must hold; either alone is not enough.
    let int16_grid_positions = layout.position_format == dxgi::R16G16B16A16_SINT
        && layout.position_stride == 8
        && layout.texcoord_format == dxgi::R16G16_SINT;
    if let Some(grid_side_cells) = detect_grid_side_cells(layout.vertex_count, layout.index_count) {
        if int16_grid_positions {
            return CapturedBorderGeometryKind::TerrainGridPatch { grid_side_cells };
        }
    }

    // Border-polyline signature: float positions, 6-per-quad indexing
    // (index_count == vertex_count / 4 * 6), and NOT a grid.
    let float_positions = layout.position_format == dxgi::R32G32B32_FLOAT;
    let quad_indexed = layout.vertex_count % 4 == 0
        && layout.index_count == (layout.vertex_count / 4) * 6
        && layout.index_count % 6 == 0;
    if float_positions && quad_indexed {
        return CapturedBorderGeometryKind::BorderPolylines;
    }

    CapturedBorderGeometryKind::Unresolved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diag23b_capture_is_a_terrain_grid_patch_not_borders() {
        // The single captured borders_pass is a 64×64-cell (65×65-vertex) terrain
        // tessellation, NOT border geometry — proven, not assumed.
        let kind = classify_captured_border_geometry(&DIAG23B_BORDERS_PASS_LAYOUT);
        assert_eq!(
            kind,
            CapturedBorderGeometryKind::TerrainGridPatch {
                grid_side_cells: 64
            },
            "diag23b must classify as a mis-captured terrain grid, not usable border geometry"
        );
        // Sanity on the recorded counts: 65² vertices, 64²·6 indices.
        assert_eq!(DIAG23B_BORDERS_PASS_LAYOUT.vertex_count, 65 * 65);
        assert_eq!(DIAG23B_BORDERS_PASS_LAYOUT.index_count, 64 * 64 * 6);
    }

    #[test]
    fn synthesized_border_layout_classifies_as_border_polylines() {
        // A synthesized border draw: N quads -> 4N verts, 6N indices, float pos.
        for quads in [1u32, 10, 278_286] {
            let layout = CapturedBorderIaLayout {
                position_format: SYNTHESIZED_BORDER_LAYOUT_POSITION_FORMAT,
                position_stride: SYNTHESIZED_BORDER_VERTEX_STRIDE,
                texcoord_format: SYNTHESIZED_BORDER_LAYOUT_TEXCOORD_FORMAT,
                texcoord_stride: SYNTHESIZED_BORDER_VERTEX_STRIDE,
                index_format: dxgi::R32_UINT,
                index_count: quads * 6,
                vertex_count: quads * 4,
            };
            assert_eq!(
                classify_captured_border_geometry(&layout),
                CapturedBorderGeometryKind::BorderPolylines,
                "quads={quads} must classify as border polylines"
            );
        }
    }

    #[test]
    fn grid_detector_requires_both_square_verts_and_grid_indices() {
        // 65² verts but non-grid index count -> not a grid.
        assert_eq!(detect_grid_side_cells(4225, 24576), Some(64));
        assert_eq!(detect_grid_side_cells(4225, 4225 / 4 * 6), None);
        // Non-square vertex count is never a grid.
        assert_eq!(detect_grid_side_cells(4224, 24576), None);
        // A border quad count (4 verts, 6 indices) is not a grid.
        assert_eq!(detect_grid_side_cells(4, 6), None);
    }

    #[test]
    fn float_grid_shaped_geometry_is_not_falsely_called_terrain() {
        // Even if vertex/index counts happen to be grid-shaped, FLOAT positions
        // are never the terrain-patch int16 signature, so it is not TerrainGrid.
        let layout = CapturedBorderIaLayout {
            position_format: dxgi::R32G32B32_FLOAT,
            position_stride: 20,
            texcoord_format: dxgi::R32G32_SINT,
            texcoord_stride: 20,
            index_format: dxgi::R32_UINT,
            index_count: 64 * 64 * 6,
            vertex_count: 65 * 65,
        };
        // Not a terrain grid (float positions), and not clean border polylines
        // (index_count != vertex_count/4*6) -> Unresolved.
        assert_eq!(
            classify_captured_border_geometry(&layout),
            CapturedBorderGeometryKind::Unresolved
        );
    }
}
