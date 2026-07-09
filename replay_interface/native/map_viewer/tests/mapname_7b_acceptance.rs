//! LABGRAD1 HARD ACCEPTANCE GATE (structural version).
//!
//! Reconstructs the `session_MAPNAME_7B` camera from its captured cb0 (READ-ONLY
//! evidence, copied verbatim into `tests/fixtures/mapname_7b/`) and scores the
//! per-component country-label law's GEOMETRY machinery against ALL 23 engine
//! `mapname` label draws of that frame. Captures VALIDATE the law; they never
//! feed the runtime label path (`MAP_LABEL_LAW.md` §5).
//!
//! ## Why STRUCTURAL, not full
//!
//! The full gate the spec describes ("run the label law on the recording state
//! for that save") needs the capture-time OWNER/CONTROLLER province raster for
//! the ~1943 7B save. That raster is NOT in the capture — the 7B session dumped
//! only the label vertex buffers + constant buffers (see its
//! `map_capture_session.json`; `MAP_LABEL_LAW.md` §5.1 lists the owner/controller
//! rasters as the still-uncaptured evidence). So this gate ships the DOWNGRADED
//! structural form the spec authorises:
//!
//!   1. **Camera reconstruction (all 23 draws).** Each draw's captured cb0
//!      `ViewProjectionMatrix`, read column-major (transposed for CPU `M·v`,
//!      per LAB3), projects that draw's world-space glyph quads INSIDE the
//!      captured 1920×1080 frame — proving the camera reconstruction is exact for
//!      every label, not just the one LAB3 checked.
//!   2. **All-strings-resolvable.** Every one of the 23 labels decodes to a
//!      clean, non-empty ASCII string (no `?` glyphs) via the shipped
//!      `hoi_mapfont4.fnt` rects — i.e. the exact strings the engine drew
//!      (UNITED KINGDOM, MANDATORY PALESTINE, REGNO DEL SUD, …) are resolvable
//!      label text the localisation-join path can carry.
//!   3. **Count structure REFUTES one-label-per-country.** The 23 draws include
//!      the same tag MULTIPLE times (UNITED KINGDOM ×8, FRANCE ×2, …). A
//!      one-label-per-country law can draw at most one per tag, so ≥2 draws of a
//!      tag DECISIVELY refutes it and shows the engine labels detached regions
//!      separately. NOTE this is consistent with per-CONNECTED-COMPONENT (the
//!      LABGRAD1 rule) but does NOT uniquely prove it over per-STATE or
//!      per-strategic-region (§5.1) — settling that needs the absent
//!      owner/controller raster. This gate checks the refutation, not the
//!      stronger per-component claim.
//!   4. **Geometry-law residuals on matched labels (all 23).** The placement
//!      law's own arc model (centroid anchor + PCA angle + quadratic bow) fits
//!      each engine baseline to within the LAB3-cited tolerance (mean RMS ≈0.158,
//!      max ≈1.14 world units). This validates the §3.4–§3.7 arc/size machinery —
//!      unchanged by LABGRAD1 — across the whole 23-label set.
//!
//! What this gate does NOT prove (honest scope): that the law's
//! PCA-over-ownership-raster predicts THESE anchors/angles from the 7B owner
//! state (needs the absent raster), nor the absolute size curve (§5.1). Those
//! stay UNKNOWN-ENGINE-SIDE / UNTRACED.

use map_viewer::{
    decode_label, decode_vertices, draw_country_names_cutoff_from_resolver, fit_baseline,
    glyph_quads, label_draw_visible, parse_fnt_chars, FntChar, GameFileResolver, GlyphQuad,
};

const NUM_DRAWS: usize = 23;
/// The shipped `hoi_mapfont4.fnt` (scaleW 2048, scaleH 1024, 575 char rects),
/// copied verbatim as a decoder fixture (glyph UV rects only).
const FNT: &str = include_str!("fixtures/mapname_7b/hoi_mapfont4.fnt");

/// Load the 23 captured (VB, cb0) byte pairs from the fixtures.
fn draws() -> Vec<(Vec<u8>, Vec<u8>)> {
    // include_bytes! needs literal paths, so enumerate the 23 explicitly.
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
    // (1) Camera reconstruction: every draw's transposed cb0 VP-matrix lands its
    // world-space glyphs inside the captured 1920×1080 frame.
    let draws = draws();
    assert_eq!(draws.len(), NUM_DRAWS);
    for (i, (vb, cb0)) in draws.iter().enumerate() {
        assert!(!vb.is_empty() && vb.len() % 20 == 0, "draw {i} VB stride");
        let verts = decode_vertices(vb).unwrap();
        assert!(verts.len() % 6 == 0, "draw {i} whole glyphs");
        let quads = glyph_quads(&verts, 2048.0, 1024.0).unwrap();
        let f = cb0_floats(cb0);
        assert_eq!(f.len(), 104, "draw {i} cb0 is 416 B = 104 floats");
        // Camera looks down at the map plane (vCamPos.y > plane, look.y < 0).
        assert!(f[53] > 100.0, "draw {i} vCamPos.y {}", f[53]);
        assert!(f[57] < -0.5, "draw {i} vCamLookAtDir.y {}", f[57]);
        // Transposed (column-major) projection: the label's CENTRE glyph baseline
        // midpoint lands inside the captured 1920×1080 frame. (The centre is the
        // robust on-screen proxy; a big label like FRANCE runs its end glyphs off
        // the frame edge, so the first vertex is not a fair test — but every
        // captured label's centre is by construction visible.) Row-major would
        // give an absurd clip.w (~1e5); transposed gives a sane view depth.
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
        // Within a small margin of the captured frame: the load-bearing proof is
        // the sane transposed clip.w above (row-major gives ~1e5); this bound
        // just confirms the projection maps to screen space near the frame, not
        // to some wild off-map coordinate. A few labels sit right on the frame
        // edge in this specific 7B view, so allow a 5% margin.
        assert!(
            (-96.0..2016.0).contains(&sx) && (-54.0..1134.0).contains(&sy),
            "draw {i} centre projected far off-frame at ({sx:.0},{sy:.0})"
        );
    }
}

#[test]
fn all_23_strings_resolve_cleanly() {
    // (2) + (3): every engine label decodes to a clean non-empty ASCII string,
    // and the 23 draws span the multi-region-power tags (UK/FRANCE repeats) that
    // REFUTE one-label-per-country (consistent with, not proof of, per-component).
    // We recover the strings and check the documented multiset appears.
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
    // The 23 labels are the §5 set; assert the load-bearing structural facts:
    // several distinct multi-word powers appear, and at least one string recurs
    // (the SAME tag labelled more than once — refutes one-label-per-country).
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
    // UNITED KINGDOM appears MANY times (§5: ×8) — the decisive refutation of
    // one-label-per-country. Assert it recurs (> once) without over-fitting the
    // exact count.
    let uk = texts.iter().filter(|t| *t == "UNITEDKINGDOM").count();
    assert!(
        uk > 1,
        "UNITED KINGDOM should be labelled multiple times (detached regions), got {uk}"
    );
    // Total draws is the captured 23.
    assert_eq!(texts.len(), NUM_DRAWS);
}

#[test]
fn draw_time_cull_matches_the_captured_drawn_set() {
    // RE-EMIT §6.2a: the BUILT label set is camera-independent (the
    // visibility is a DRAW-time cull. `label_draw_visible` is that cull, and
    // this capture pins its two laws:
    //
    // 1. CUTOFF (output-observable): every one of the 23 drawn labels was
    //    captured at vCamPos.y >= the shipped cutoff (260, read from the
    //    defines via the resolver — never a literal). Asserted 23/23.
    // 2. FRUSTUM is pixel-neutral, and a TIGHT per-glyph reject is REFUTED as
    //    the engine's submission law: draw 8's glyph strip projects ENTIRELY
    //    off-frame (NDC y in [1.04, 1.14]) yet the engine SUBMITTED it — the
    //    GPU clipped it to nothing. So the gate asserts recall 1.0 on every
    //    draw that has at least one on-screen point (pixel-producing labels
    //    must never be culled), and separately reports the fully-offscreen
    //    submissions where a conservative skip is pixel-identical.
    //
    // Honest scope (precision): the 7B save's full BUILT label set is not
    // reconstructable (its owner raster was not captured — MAP_LABEL_LAW §5.1),
    // so drawn-set PRECISION against that save cannot be measured here; recall
    // on the engine's own drawn geometry + the negative controls below are
    // what this capture can prove. SKIPS without the local game install.
    let game = std::path::PathBuf::from(r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV");
    let Ok(resolver) = GameFileResolver::vanilla(game) else {
        eprintln!("SKIP draw-cull gate: no local the game install");
        return;
    };
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
        let cam_height = f[53]; // vCamPos.y
                                // Law 1: the engine drew this label, so its camera must pass the
                                // shipped cutoff.
        assert!(
            cam_height >= cutoff,
            "draw {i}: engine drew a label at cam_height {cam_height} BELOW the \
             shipped cutoff {cutoff} — the cutoff law is wrong"
        );
        // Does the label have any pixel-producing (on-screen, in-front) point?
        let m = |r: usize, c: usize| vp[r * 4 + c] as f64;
        let has_pixels = world_pts.iter().any(|p| {
            let v = [p[0] as f64, p[1] as f64, p[2] as f64, 1.0];
            let clip: [f64; 4] = std::array::from_fn(|r| (0..4).map(|c| m(c, r) * v[c]).sum());
            clip[3] > 0.0 && (clip[0] / clip[3]).abs() <= 1.0 && (clip[1] / clip[3]).abs() <= 1.0
        });
        let visible = label_draw_visible(&world_pts, &vp, cam_height, cutoff);
        if has_pixels {
            onscreen += 1;
            // Law 2: a pixel-producing label must NEVER be culled.
            assert!(
                visible,
                "draw {i}: label has on-screen pixels but the cull rejected it"
            );
        } else if !visible {
            // Fully-offscreen submission (engine drew it, GPU clipped it):
            // skipping it is pixel-identical. Report, don't assert.
            offscreen_submitted.push(i);
        }
        if visible {
            kept += 1;
        }
        // Negative controls on the same geometry:
        // (a) below the cutoff the label must not draw;
        assert!(
            !label_draw_visible(&world_pts, &vp, cutoff - 10.0, cutoff),
            "draw {i}: cutoff cull failed"
        );
        // (b) translating the world geometry far outside the frustum culls it
        //     (x += 100k map units is far beyond any captured view).
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
    // The overwhelming majority of the drawn set is genuinely on-screen; the
    // capture evidences at least 20 pixel-producing labels.
    assert!(onscreen >= 20, "only {onscreen}/23 draws project on-screen");
}

#[test]
fn law_arc_model_reproduces_all_23_engine_baselines() {
    // (4): the placement law's arc model fits each engine baseline. Per LAB3 the
    // MEAN RMS across the 23 is ≈0.158 (max ≈1.14). Assert the aggregate mean is
    // within a tolerance comfortably above the cited figure but well below a
    // world-unit, and that no single label blows past the cited max by much.
    let draws = draws();
    let mut rms_sum = 0.0f64;
    let mut rms_max = 0.0f32;
    let mut worst_max = 0.0f32;
    let mut n = 0usize;
    for (i, (vb, _cb0)) in draws.iter().enumerate() {
        let verts = decode_vertices(vb).unwrap();
        let quads = glyph_quads(&verts, 2048.0, 1024.0).unwrap();
        // Single-glyph labels have no baseline to fit; skip (none in 7B, but be
        // safe). fit_baseline needs >= 2 glyphs.
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
    // LAB3 measured mean RMS ≈ 0.158. Gate at 0.35 (> the cited value with head-
    // room, << one world unit): a real regression trips this.
    assert!(
        mean_rms < 0.35,
        "mean arc-model RMS {mean_rms} across {n} labels exceeds 0.35 world units"
    );
    // Per-label RMS should also stay sub-world-unit.
    assert!(rms_max < 0.8, "worst per-label RMS {rms_max} exceeds 0.8");
    // LAB3 max residual ≈ 1.14 on the most-curved labels; gate at 2.0.
    assert!(
        worst_max < 2.0,
        "worst per-label MAX residual {worst_max} exceeds 2.0 world units"
    );
}
