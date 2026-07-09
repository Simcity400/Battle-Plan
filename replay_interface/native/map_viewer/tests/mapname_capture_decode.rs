//! LAB3 integration test: decode the REAL captured engine `mapname` vertex
//! buffer (the first-ever captured country-label draw) with field-level
//! assertions, and score the placement law's arc model against the engine's own
//! glyph baseline with explicit tolerances.
//!
//! Fixtures under `tests/fixtures/mapname/` are the exact bytes captured in
//! `session_CAPFIX2_GAPS_4` (`map_frame_mapname_pass_bundle.json`, draw 0). They
//! are DECODER INPUTS for validation only — they never feed the runtime label
//! path (see `documents/MAP_LABEL_LAW.md` §5, "captures validate, never drive").
//!
//! Ground truth for the layout is the reflected VS input signature of
//! `4e1b73f39c128df0` (bundle `input_layout.elements`): POSITION (DXGI 6,
//! R32G32B32_FLOAT) @ offset 0, TEXCOORD0 (DXGI 16, R32G32_FLOAT) @ offset 12,
//! stride 20. The draw is non-indexed, TRIANGLELIST, `draw_count = 66` → 11
//! glyph quads.

use map_viewer::{
    decode_label, decode_vertices, fit_baseline, glyph_quads, match_glyph, FntChar,
    MAPNAME_VERTEX_STRIDE,
};

/// The exact captured VB slot0 bytes (1320 B = 66 verts × 20 B stride).
const SOVIETUNION_VB: &[u8] = include_bytes!("fixtures/mapname/sovietunion_gaps4_vb_slot0.bin");
/// The exact captured VS cb0 (416 B) for the same draw. Layout per shipped
/// `gfx/FX/standardfuncsgfx.fxh` `ConstantBuffer(0,0)`: floats 0..16 =
/// `ViewProjectionMatrix`, floats 52..55 = `vCamPos`, 56..59 = `vCamLookAtDir`.
const SOVIETUNION_CB0: &[u8] = include_bytes!("fixtures/mapname/sovietunion_gaps4_cb0.bin");

fn cb0_floats() -> Vec<f32> {
    SOVIETUNION_CB0
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// The `hoi_mapfont4.fnt` rects (scaleW=2048, scaleH=1024) for the eight
/// distinct letters of SOVIET UNION, transcribed from the shipped font file
/// (`gfx/fonts/hoi_mapfont4.fnt`, char lines). Used to recover the string from
/// the captured glyph UVs.
fn sovietunion_chars() -> Vec<FntChar> {
    vec![
        FntChar {
            id: 83,
            x: 1424.0,
            y: 473.0,
            width: 45.0,
            height: 57.0,
            xadvance: 0.0,
        }, // S
        FntChar {
            id: 79,
            x: 677.0,
            y: 479.0,
            width: 54.0,
            height: 57.0,
            xadvance: 0.0,
        }, // O
        FntChar {
            id: 86,
            x: 522.0,
            y: 598.0,
            width: 53.0,
            height: 55.0,
            xadvance: 0.0,
        }, // V
        FntChar {
            id: 73,
            x: 1031.0,
            y: 647.0,
            width: 32.0,
            height: 55.0,
            xadvance: 0.0,
        }, // I
        FntChar {
            id: 69,
            x: 845.0,
            y: 649.0,
            width: 39.0,
            height: 55.0,
            xadvance: 0.0,
        }, // E
        FntChar {
            id: 84,
            x: 1624.0,
            y: 585.0,
            width: 48.0,
            height: 55.0,
            xadvance: 0.0,
        }, // T
        FntChar {
            id: 85,
            x: 326.0,
            y: 543.0,
            width: 48.0,
            height: 56.0,
            xadvance: 0.0,
        }, // U
        FntChar {
            id: 78,
            x: 1575.0,
            y: 585.0,
            width: 48.0,
            height: 55.0,
            xadvance: 0.0,
        }, // N
    ]
}

#[test]
fn real_vb_has_66_verts_at_stride_20() {
    // The captured buffer_byte_width / draw geometry: 1320 B, stride 20, 66 verts.
    assert_eq!(SOVIETUNION_VB.len(), 1320);
    assert_eq!(SOVIETUNION_VB.len() % MAPNAME_VERTEX_STRIDE, 0);
    let verts = decode_vertices(SOVIETUNION_VB).unwrap();
    assert_eq!(verts.len(), 66);
    assert_eq!(verts.len() / 6, 11, "11 glyph quads");
}

#[test]
fn real_vb_position_field_is_world_space_map_plane() {
    // POSITION @ offset 0: the label sits on the map plane at world Y ≈ 10 and
    // world X∈[28,67], Z∈[2026,2037] (the Soviet Union near the save camera).
    // This is the layout ground truth: a float3 world position per vertex.
    let verts = decode_vertices(SOVIETUNION_VB).unwrap();
    for v in &verts {
        assert!(v.y > 9.5 && v.y < 11.0, "map-plane Y out of band: {}", v.y);
        assert!(v.x > 20.0 && v.x < 75.0, "world X out of band: {}", v.x);
        assert!(v.z > 2020.0 && v.z < 2045.0, "world Z out of band: {}", v.z);
        // UV in [0,1] (atlas coordinates).
        assert!((0.0..=1.0).contains(&v.u) && (0.0..=1.0).contains(&v.v));
    }
    // Exact first-vertex values (TL of the 'S'), byte-for-byte from the capture.
    let v0 = verts[0];
    assert!((v0.x - 28.2785).abs() < 1e-3, "v0.x {}", v0.x);
    assert!((v0.y - 10.0111).abs() < 1e-3, "v0.y {}", v0.y);
    assert!((v0.z - 2033.3943).abs() < 1e-2, "v0.z {}", v0.z);
    assert!((v0.u - 0.69531).abs() < 1e-4, "v0.u {}", v0.u);
    assert!((v0.v - 0.51758).abs() < 1e-4, "v0.v {}", v0.v);
}

#[test]
fn real_vb_glyph_uvs_decode_to_sovietunion() {
    // The whole point of the layout: each glyph's UV bbox matches exactly one
    // `.fnt` char rect. Recovering the 11 glyphs yields "SOVIETUNION" (no space
    // quad — the engine omits spaces from the VB), each at ~0 texel error.
    let verts = decode_vertices(SOVIETUNION_VB).unwrap();
    let quads = glyph_quads(&verts, 2048.0, 1024.0).unwrap();
    assert_eq!(quads.len(), 11);
    let chars = sovietunion_chars();
    let mut text = String::new();
    for q in &quads {
        let (id, err) = match_glyph(q, &chars).unwrap();
        assert!(err < 1.0, "glyph UV rect off by {err} texels (aliasing?)");
        text.push(char::from_u32(id).unwrap());
    }
    assert_eq!(text, "SOVIETUNION");
}

#[test]
fn real_vb_full_decode_label() {
    let chars = sovietunion_chars();
    let label = decode_label(SOVIETUNION_VB, 2048.0, 1024.0, &chars).unwrap();
    assert_eq!(label.glyph_count, 11);
    assert_eq!(label.text, "SOVIETUNION");
    // Near-constant cap height within the label (one uniform scale per label):
    // engine cap heights are ~4.0–4.18 world units.
    let hmin = label
        .glyphs
        .iter()
        .map(|g| g.cap_height)
        .fold(f32::INFINITY, f32::min);
    let hmax = label
        .glyphs
        .iter()
        .map(|g| g.cap_height)
        .fold(0.0f32, f32::max);
    assert!(hmin > 3.8 && hmax < 4.4, "cap height band [{hmin},{hmax}]");
    assert!(
        (hmax - hmin) < 0.5,
        "cap height should be ~constant within a label: [{hmin},{hmax}]"
    );
}

#[test]
fn law_arc_model_reproduces_engine_baseline_within_tolerance() {
    // The engine's baseline (per-glyph bottom-edge midpoints) is reproduced by
    // the placement law's quadratic-arc model
    // (`CountryLabelPlacement::baseline_point`, same centroid/PCA/quadratic-bow
    // form) to well under one world unit. TOLERANCE: RMS < 0.6, MAX < 1.2 world
    // units. On this label (a gentle curve) the residual is ~0.6/1.1.
    //
    // What this DOES prove: the arc MODEL is the right shape for the engine's
    // baseline. What it does NOT prove: that the law's PCA-over-ownership-raster
    // predicts this anchor/angle (that needs the capture-time province raster,
    // absent from this VB-only fixture) — see the report's underdetermination
    // list.
    let verts = decode_vertices(SOVIETUNION_VB).unwrap();
    let quads = glyph_quads(&verts, 2048.0, 1024.0).unwrap();
    let fit = fit_baseline(&quads).unwrap();
    assert!(
        fit.residual_rms < 0.6,
        "arc-model RMS residual {} exceeds 0.6 world units",
        fit.residual_rms
    );
    assert!(
        fit.residual_max < 1.2,
        "arc-model MAX residual {} exceeds 1.2 world units",
        fit.residual_max
    );
    // The label is genuinely curved (not a straight line): nonzero bow, and the
    // baseline angle is the diagonal the glyphs trace (~-13° in world XZ).
    assert!(
        fit.bow.abs() > 0.3,
        "expected a bowed baseline, bow {}",
        fit.bow
    );
    assert!(
        fit.angle.to_degrees().abs() > 5.0 && fit.angle.to_degrees().abs() < 25.0,
        "baseline angle {} deg out of expected band",
        fit.angle.to_degrees()
    );
}

#[test]
fn captured_vp_matrix_projects_world_verts_on_screen_only_when_transposed() {
    // Lock the load-bearing finding: the VB POSITION is world-space and the
    // captured cb0 `ViewProjectionMatrix` (16 floats, row 0..16) must be read
    // COLUMN-major (transposed) for CPU `M·v` — the shader's
    // `mul(ViewProjectionMatrix, vPos)` (mapname.shader:55). Row-major gives an
    // absurd clip.w (~1e5); transposed gives a sane view depth (~300, =
    // vCamPos.y 310 − plane_y 10) and screen coords inside 1920×1080.
    let f = cb0_floats();
    assert_eq!(f.len(), 104, "cb0 is 416 B = 104 floats");
    // vCamPos / vCamLookAtDir at the documented offsets.
    let cam_y = f[53];
    let look_y = f[57];
    assert!((cam_y - 310.0).abs() < 1.0, "vCamPos.y {cam_y}");
    assert!(
        look_y < -0.9,
        "vCamLookAtDir.y {look_y} (camera looks down)"
    );

    let m = |r: usize, c: usize| f[r * 4 + c] as f64; // stored 4x4, row-major bytes
    let verts = decode_vertices(SOVIETUNION_VB).unwrap();
    let v0 = &verts[0];
    let p = [v0.x as f64, v0.y as f64, v0.z as f64, 1.0];

    // Row-major CPU read: clip[r] = sum_c M[r][c]*p[c].
    let w_row = (0..4).map(|c| m(3, c) * p[c]).sum::<f64>();
    // Column-major (transpose) read: clip[r] = sum_c M[c][r]*p[c].
    let w_col = (0..4).map(|c| m(c, 3) * p[c]).sum::<f64>();

    assert!(
        w_row.abs() > 10_000.0,
        "row-major clip.w should be absurd, got {w_row}"
    );
    assert!(
        (250.0..350.0).contains(&w_col),
        "transposed clip.w should be ~view depth 300, got {w_col}"
    );

    // Full transposed projection of v0 lands inside the captured 1920×1080.
    let clip: Vec<f64> = (0..4)
        .map(|r| (0..4).map(|c| m(c, r) * p[c]).sum::<f64>())
        .collect();
    let (sx, sy) = (
        (clip[0] / clip[3] * 0.5 + 0.5) * 1920.0,
        (1.0 - (clip[1] / clip[3] * 0.5 + 0.5)) * 1080.0,
    );
    assert!((0.0..1920.0).contains(&sx), "screen x {sx} off-frame");
    assert!((0.0..1080.0).contains(&sy), "screen y {sy} off-frame");
}
