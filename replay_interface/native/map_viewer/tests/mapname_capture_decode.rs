
use map_viewer::{
    decode_label, decode_vertices, fit_baseline, glyph_quads, match_glyph, FntChar,
    MAPNAME_VERTEX_STRIDE,
};

const SOVIETUNION_VB: &[u8] = include_bytes!("fixtures/mapname/sovietunion_gaps4_vb_slot0.bin");
const SOVIETUNION_CB0: &[u8] = include_bytes!("fixtures/mapname/sovietunion_gaps4_cb0.bin");

fn cb0_floats() -> Vec<f32> {
    SOVIETUNION_CB0
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn sovietunion_chars() -> Vec<FntChar> {
    vec![
        FntChar {
            id: 83,
            x: 1424.0,
            y: 473.0,
            width: 45.0,
            height: 57.0,
            xadvance: 0.0,
        },
        FntChar {
            id: 79,
            x: 677.0,
            y: 479.0,
            width: 54.0,
            height: 57.0,
            xadvance: 0.0,
        },
        FntChar {
            id: 86,
            x: 522.0,
            y: 598.0,
            width: 53.0,
            height: 55.0,
            xadvance: 0.0,
        },
        FntChar {
            id: 73,
            x: 1031.0,
            y: 647.0,
            width: 32.0,
            height: 55.0,
            xadvance: 0.0,
        },
        FntChar {
            id: 69,
            x: 845.0,
            y: 649.0,
            width: 39.0,
            height: 55.0,
            xadvance: 0.0,
        },
        FntChar {
            id: 84,
            x: 1624.0,
            y: 585.0,
            width: 48.0,
            height: 55.0,
            xadvance: 0.0,
        },
        FntChar {
            id: 85,
            x: 326.0,
            y: 543.0,
            width: 48.0,
            height: 56.0,
            xadvance: 0.0,
        },
        FntChar {
            id: 78,
            x: 1575.0,
            y: 585.0,
            width: 48.0,
            height: 55.0,
            xadvance: 0.0,
        },
    ]
}

#[test]
fn real_vb_has_66_verts_at_stride_20() {
    assert_eq!(SOVIETUNION_VB.len(), 1320);
    assert_eq!(SOVIETUNION_VB.len() % MAPNAME_VERTEX_STRIDE, 0);
    let verts = decode_vertices(SOVIETUNION_VB).unwrap();
    assert_eq!(verts.len(), 66);
    assert_eq!(verts.len() / 6, 11, "11 glyph quads");
}

#[test]
fn real_vb_position_field_is_world_space_map_plane() {
    let verts = decode_vertices(SOVIETUNION_VB).unwrap();
    for v in &verts {
        assert!(v.y > 9.5 && v.y < 11.0, "map-plane Y out of band: {}", v.y);
        assert!(v.x > 20.0 && v.x < 75.0, "world X out of band: {}", v.x);
        assert!(v.z > 2020.0 && v.z < 2045.0, "world Z out of band: {}", v.z);
        assert!((0.0..=1.0).contains(&v.u) && (0.0..=1.0).contains(&v.v));
    }
    let v0 = verts[0];
    assert!((v0.x - 28.2785).abs() < 1e-3, "v0.x {}", v0.x);
    assert!((v0.y - 10.0111).abs() < 1e-3, "v0.y {}", v0.y);
    assert!((v0.z - 2033.3943).abs() < 1e-2, "v0.z {}", v0.z);
    assert!((v0.u - 0.69531).abs() < 1e-4, "v0.u {}", v0.u);
    assert!((v0.v - 0.51758).abs() < 1e-4, "v0.v {}", v0.v);
}

#[test]
fn real_vb_glyph_uvs_decode_to_sovietunion() {
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
    // baseline. What it does NOT prove: that the law's PCA-over-ownership-raster
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
    let f = cb0_floats();
    assert_eq!(f.len(), 104, "cb0 is 416 B = 104 floats");
    let cam_y = f[53];
    let look_y = f[57];
    assert!((cam_y - 310.0).abs() < 1.0, "vCamPos.y {cam_y}");
    assert!(
        look_y < -0.9,
        "vCamLookAtDir.y {look_y} (camera looks down)"
    );

    let m = |r: usize, c: usize| f[r * 4 + c] as f64;
    let verts = decode_vertices(SOVIETUNION_VB).unwrap();
    let v0 = &verts[0];
    let p = [v0.x as f64, v0.y as f64, v0.z as f64, 1.0];

    let w_row = (0..4).map(|c| m(3, c) * p[c]).sum::<f64>();
    let w_col = (0..4).map(|c| m(c, 3) * p[c]).sum::<f64>();

    assert!(
        w_row.abs() > 10_000.0,
        "row-major clip.w should be absurd, got {w_row}"
    );
    assert!(
        (250.0..350.0).contains(&w_col),
        "transposed clip.w should be ~view depth 300, got {w_col}"
    );

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
