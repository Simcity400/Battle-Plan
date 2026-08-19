
pub mod dxgi {
    pub const R32G32B32_FLOAT: u32 = 6;
    pub const R16G16B16A16_SINT: u32 = 14;
    pub const R32G32_SINT: u32 = 18;
    pub const R16G16_SINT: u32 = 38;
    pub const R16_UINT: u32 = 57;
    #[cfg(test)]
    pub const R32_UINT: u32 = 42;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapturedBorderIaLayout {
    pub position_format: u32,
    pub position_stride: u32,
    pub texcoord_format: u32,
    pub texcoord_stride: u32,
    pub index_format: u32,
    pub index_count: u32,
    pub vertex_count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapturedBorderGeometryKind {
    BorderPolylines,
    TerrainGridPatch { grid_side_cells: u32 },
    Unresolved,
}

pub const DIAG23B_BORDERS_PASS_LAYOUT: CapturedBorderIaLayout = CapturedBorderIaLayout {
    position_format: dxgi::R16G16B16A16_SINT,
    position_stride: 8,
    texcoord_format: dxgi::R16G16_SINT,
    texcoord_stride: 4,
    index_format: dxgi::R16_UINT,
    index_count: 24576,
    vertex_count: 4225,
};

pub const SYNTHESIZED_BORDER_LAYOUT_POSITION_FORMAT: u32 = dxgi::R32G32B32_FLOAT;
pub const SYNTHESIZED_BORDER_LAYOUT_TEXCOORD_FORMAT: u32 = dxgi::R32G32_SINT;
pub const SYNTHESIZED_BORDER_VERTEX_STRIDE: u32 = 20;

fn detect_grid_side_cells(vertex_count: u32, index_count: u32) -> Option<u32> {
    if vertex_count == 0 {
        return None;
    }
    // thus misclassification of garbage as a valid grid — in release).
    let side = (vertex_count as f64).sqrt().round() as u64;
    if side < 2 || side * side != u64::from(vertex_count) {
        return None;
    }
    let cells = side - 1;
    if cells >= 2 && cells * cells * 6 == u64::from(index_count) {
        Some(cells as u32)
    } else {
        None
    }
}

pub fn classify_captured_border_geometry(
    layout: &CapturedBorderIaLayout,
) -> CapturedBorderGeometryKind {
    let int16_grid_positions = layout.position_format == dxgi::R16G16B16A16_SINT
        && layout.position_stride == 8
        && layout.texcoord_format == dxgi::R16G16_SINT;
    if let Some(grid_side_cells) = detect_grid_side_cells(layout.vertex_count, layout.index_count) {
        if int16_grid_positions {
            return CapturedBorderGeometryKind::TerrainGridPatch { grid_side_cells };
        }
    }

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
        let kind = classify_captured_border_geometry(&DIAG23B_BORDERS_PASS_LAYOUT);
        assert_eq!(
            kind,
            CapturedBorderGeometryKind::TerrainGridPatch {
                grid_side_cells: 64
            },
            "diag23b must classify as a mis-captured terrain grid, not usable border geometry"
        );
        assert_eq!(DIAG23B_BORDERS_PASS_LAYOUT.vertex_count, 65 * 65);
        assert_eq!(DIAG23B_BORDERS_PASS_LAYOUT.index_count, 64 * 64 * 6);
    }

    #[test]
    fn synthesized_border_layout_classifies_as_border_polylines() {
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
        assert_eq!(detect_grid_side_cells(4225, 24576), Some(64));
        assert_eq!(detect_grid_side_cells(4225, 4225 / 4 * 6), None);
        assert_eq!(detect_grid_side_cells(4224, 24576), None);
        assert_eq!(detect_grid_side_cells(4, 6), None);
    }

    #[test]
    fn grid_detector_does_not_overflow_on_adversarial_counts() {
        // arithmetic wrapped that product to 333_920 (release) — matching an
        let wrapped_index_count = ((26756u64 * 26756 * 6) % (1u64 << 32)) as u32;
        assert_eq!(
            detect_grid_side_cells(715_937_049, wrapped_index_count),
            None
        );
        assert_eq!(detect_grid_side_cells(u32::MAX, 0), None);
    }

    #[test]
    fn float_grid_shaped_geometry_is_not_falsely_called_terrain() {
        let layout = CapturedBorderIaLayout {
            position_format: dxgi::R32G32B32_FLOAT,
            position_stride: 20,
            texcoord_format: dxgi::R32G32_SINT,
            texcoord_stride: 20,
            index_format: dxgi::R32_UINT,
            index_count: 64 * 64 * 6,
            vertex_count: 65 * 65,
        };
        assert_eq!(
            classify_captured_border_geometry(&layout),
            CapturedBorderGeometryKind::Unresolved
        );
    }
}
