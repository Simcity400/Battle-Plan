//! PCA-over-ownership-raster predicts THESE anchors/angles from the 7B owner

use map_viewer::{
    decode_label, decode_vertices, draw_country_names_cutoff_from_resolver, fit_baseline,
    glyph_quads, label_draw_visible, parse_fnt_chars, FntChar, GameFileResolver, GlyphQuad,
};

const NUM_DRAWS: usize = 23;
const FNT: &str = include_str!("fixtures/mapname_7b/hoi_mapfont4.fnt");

fn draws() -> Vec<(Vec<u8>, Vec<u8>)> {
    macro_rules! pair {
        ($n:literal) => {
            (
                include_bytes!(concat!("fixtures/mapname_7b/draw", $n, "_vb.bin")).to_vec(),
                include_bytes!(concat!("fixtures/mapname_7b/draw", $n, "_cb0.bin")).to_vec(),
            )
        };
    }
    vec![
        pair!("000"),
        pair!("001"),
        pair!("002"),
        pair!("003"),
        pair!("004"),
        pair!("005"),
        pair!("006"),
        pair!("007"),
        pair!("008"),
        pair!("009"),
        pair!("010"),
        pair!("011"),
        pair!("012"),
        pair!("013"),
        pair!("014"),
        pair!("015"),
        pair!("016"),
        pair!("017"),
        pair!("018"),
        pair!("019"),
        pair!("020"),
        pair!("021"),
        pair!("022"),
    ]
}

fn cb0_floats(cb0: &[u8]) -> Vec<f32> {
    cb0.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn fnt_chars() -> Vec<FntChar> {
    let (sw, sh, chars) = parse_fnt_chars(FNT);
    assert!((sw - 2048.0).abs() < 1e-3, "fnt scaleW {sw}");
    assert!((sh - 1024.0).abs() < 1e-3, "fnt scaleH {sh}");
    assert_eq!(chars.len(), 575, "shipped hoi_mapfont4.fnt has 575 chars");
    chars
}

#[test]
fn all_23_draws_have_valid_geometry_and_camera() {
    let draws = draws();
    assert_eq!(draws.len(), NUM_DRAWS);
    for (i, (vb, cb0)) in draws.iter().enumerate() {
        assert!(!vb.is_empty() && vb.len() % 20 == 0, "draw {i} VB stride");
        let verts = decode_vertices(vb).unwrap();
        assert!(verts.len() % 6 == 0, "draw {i} whole glyphs");
        let quads = glyph_quads(&verts, 2048.0, 1024.0).unwrap();
        let f = cb0_floats(cb0);
        assert_eq!(f.len(), 104, "draw {i} cb0 is 416 B = 104 floats");
        assert!(f[53] > 100.0, "draw {i} vCamPos.y {}", f[53]);
        assert!(f[57] < -0.5, "draw {i} vCamLookAtDir.y {}", f[57]);
        let m = |r: usize, c: usize| f[r * 4 + c] as f64;
        let mid: &GlyphQuad = &quads[quads.len() / 2];
        let (cx, cz) = mid.baseline_mid;
        let p = [cx as f64, mid.y as f64, cz as f64, 1.0];
        let clip: Vec<f64> = (0..4)
            .map(|r| (0..4).map(|c| m(c, r) * p[c]).sum::<f64>())
            .collect();
        assert!(
            clip[3] > 50.0 && clip[3] < 2000.0,
            "draw {i} transposed clip.w {} not a sane view depth",
            clip[3]
        );
        let sx = (clip[0] / clip[3] * 0.5 + 0.5) * 1920.0;
        let sy = (1.0 - (clip[1] / clip[3] * 0.5 + 0.5)) * 1080.0;
        assert!(
            (-96.0..2016.0).contains(&sx) && (-54.0..1134.0).contains(&sy),
            "draw {i} centre projected far off-frame at ({sx:.0},{sy:.0})"
        );
    }
}

#[test]
fn all_23_strings_resolve_cleanly() {
    let chars = fnt_chars();
    let draws = draws();
    let mut texts = Vec::new();
    for (i, (vb, _cb0)) in draws.iter().enumerate() {
        let label = decode_label(vb, 2048.0, 1024.0, &chars).unwrap();
        assert!(label.glyph_count >= 1, "draw {i} empty");
        assert!(
            !label.text.contains('?'),
            "draw {i} has an unresolved glyph: {:?}",
            label.text
        );
        assert!(
            label.text.chars().all(|c| c.is_ascii_uppercase()),
            "draw {i} unexpected glyph class: {:?}",
            label.text
        );
        texts.push(label.text);
    }
    let joined = texts.join(" ");
    for needle in [
        "UNITEDKINGDOM",
        "FRANCE",
        "MANDATORYPALESTINE",
        "REGNODELSUD",
        "BRITISHIRAQ",
    ] {
        assert!(
            texts.iter().any(|t| t == needle),
            "expected engine label {needle:?} not found; decoded set: {joined}"
        );
    }
    let uk = texts.iter().filter(|t| *t == "UNITEDKINGDOM").count();
    assert!(
        uk > 1,
        "UNITED KINGDOM should be labelled multiple times (detached regions), got {uk}"
    );
    assert_eq!(texts.len(), NUM_DRAWS);
}

#[test]
#[cfg_attr(not(gate_hoi4_install), ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path")]
fn draw_time_cull_matches_the_captured_drawn_set() {
    let game = replay_test_gates::require_vanilla_game_root();
    let resolver = GameFileResolver::vanilla(game).expect("vanilla resolver");
    let cutoff = draw_country_names_cutoff_from_resolver(&resolver)
        .expect("shipped defines carry DRAW_COUNTRY_NAMES_CUTOFF");
    assert!(
        (cutoff - 260.0).abs() < 1e-3,
        "vanilla cutoff is 260, defines gave {cutoff}"
    );
    let draws = draws();
    let mut kept = 0usize;
    let mut onscreen = 0usize;
    let mut offscreen_submitted: Vec<usize> = Vec::new();
    for (i, (vb, cb0)) in draws.iter().enumerate() {
        let verts = decode_vertices(vb).unwrap();
        let world_pts: Vec<[f32; 3]> = verts.iter().map(|v| [v.x, v.y, v.z]).collect();
        let f = cb0_floats(cb0);
        let vp: [f32; 16] = f[0..16].try_into().unwrap();
        let cam_height = f[53];
        assert!(
            cam_height >= cutoff,
            "draw {i}: engine drew a label at cam_height {cam_height} BELOW the \
             shipped cutoff {cutoff} — the cutoff law is wrong"
        );
        let m = |r: usize, c: usize| vp[r * 4 + c] as f64;
        let has_pixels = world_pts.iter().any(|p| {
            let v = [p[0] as f64, p[1] as f64, p[2] as f64, 1.0];
            let clip: [f64; 4] = std::array::from_fn(|r| (0..4).map(|c| m(c, r) * v[c]).sum());
            clip[3] > 0.0 && (clip[0] / clip[3]).abs() <= 1.0 && (clip[1] / clip[3]).abs() <= 1.0
        });
        let visible = label_draw_visible(&world_pts, &vp, cam_height, cutoff);
        if has_pixels {
            onscreen += 1;
            assert!(
                visible,
                "draw {i}: label has on-screen pixels but the cull rejected it"
            );
        } else if !visible {
            offscreen_submitted.push(i);
        }
        if visible {
            kept += 1;
        }
        assert!(
            !label_draw_visible(&world_pts, &vp, cutoff - 10.0, cutoff),
            "draw {i}: cutoff cull failed"
        );
        let far: Vec<[f32; 3]> = world_pts
            .iter()
            .map(|p| [p[0] + 100_000.0, p[1], p[2]])
            .collect();
        assert!(
            !label_draw_visible(&far, &vp, cam_height, cutoff),
            "draw {i}: frustum cull failed"
        );
    }
    eprintln!(
        "draw-cull: cutoff 23/23 pass; pixel-producing recall {onscreen}/{onscreen}; \
         kept {kept}/{}; engine-submitted-but-offscreen (pixel-neutral skips): {:?}",
        draws.len(),
        offscreen_submitted
    );
    assert!(onscreen >= 20, "only {onscreen}/23 draws project on-screen");
}

#[test]
fn law_arc_model_reproduces_all_23_engine_baselines() {
    let draws = draws();
    let mut rms_sum = 0.0f64;
    let mut rms_max = 0.0f32;
    let mut worst_max = 0.0f32;
    let mut n = 0usize;
    for (i, (vb, _cb0)) in draws.iter().enumerate() {
        let verts = decode_vertices(vb).unwrap();
        let quads = glyph_quads(&verts, 2048.0, 1024.0).unwrap();
        let Some(fit) = fit_baseline(&quads) else {
            continue;
        };
        assert!(
            fit.residual_rms.is_finite() && fit.residual_max.is_finite(),
            "draw {i} non-finite residual"
        );
        rms_sum += fit.residual_rms as f64;
        rms_max = rms_max.max(fit.residual_rms);
        worst_max = worst_max.max(fit.residual_max);
        n += 1;
    }
    assert!(n >= 20, "expected ~23 multi-glyph labels, fit {n}");
    let mean_rms = (rms_sum / n as f64) as f32;
    eprintln!(
        "arc-model vs 23 engine baselines: mean RMS {mean_rms:.4} wu, worst per-label \
         RMS {rms_max:.4}, worst max residual {worst_max:.4}"
    );
    assert!(
        mean_rms < 0.35,
        "mean arc-model RMS {mean_rms} across {n} labels exceeds 0.35 world units"
    );
    assert!(rms_max < 0.8, "worst per-label RMS {rms_max} exceeds 0.8");
    assert!(
        worst_max < 2.0,
        "worst per-label MAX residual {worst_max} exceeds 2.0 world units"
    );
}
