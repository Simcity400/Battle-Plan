
use map_viewer::{
    CountryLabelPlacement, ENGINE_TITLE_CAP_COEF, ENGINE_TITLE_FIT_COEF, LABEL_HEIGHT_BREADTH_K,
};

const LINE_HEIGHT: f32 = 84.0;
const CAP_PX: f32 = 55.0;

fn size_placement(half_length: f32, pixel_count: u64) -> CountryLabelPlacement {
    CountryLabelPlacement {
        tag: "T".to_string(),
        is_home: true,
        component_id: 1,
        anchor_x: 0.0,
        anchor_y: 0.0,
        angle: 0.0,
        half_length,
        bow: 0.0,
        world_height: 0.0,
        pixel_count,
        major_extent: 2.0 * half_length,
        minor_extent: 0.0,
        minor_extent_incl: 0.0,
        breadth_span: 0.0,
        breadth_p2_98: 0.0,
        curve: None,
    }
}

#[test]
fn constants_are_the_rdata_products() {
    assert_eq!(ENGINE_TITLE_FIT_COEF, 0.72_f32);
    assert_eq!(ENGINE_TITLE_CAP_COEF, 0.40_f32);
    assert!((ENGINE_TITLE_FIT_COEF - 0.8_f32 * 0.9_f32).abs() < 1e-7);
    assert!((ENGINE_TITLE_CAP_COEF - 0.8_f32 * 0.5_f32).abs() < 1e-7);
}

#[test]
fn fill_regime_is_exactly_fit_coef_times_len_over_textw() {
    let half_length = 100.0_f32;
    let text_w = 150.0_f32;
    let p = size_placement(half_length, 1_000_000);
    let gs = p.engine_glyph_scale(text_w, LINE_HEIGHT);
    let len = 2.0 * half_length;
    let expected = ENGINE_TITLE_FIT_COEF * len / text_w;
    assert!(!gs.cap_bound, "wide area must be fill-bound, not cap-bound");
    assert_eq!(
        gs.scale, expected,
        "fill scale must be 0.72*len/text_w exactly"
    );
}

#[test]
fn cap_regime_is_exactly_cap_coef_times_nlen_over_texth() {
    let half_length = 400.0_f32;
    let text_w = 100.0_f32;
    let n = 8_000_u64;
    let p = size_placement(half_length, n);
    let gs = p.engine_glyph_scale(text_w, LINE_HEIGHT);
    let len = 2.0 * half_length;
    let expected = ENGINE_TITLE_CAP_COEF * (n as f32 / len) / LINE_HEIGHT;
    assert!(gs.cap_bound, "thin strip must be breadth-cap-bound");
    assert_eq!(
        gs.scale, expected,
        "cap scale must be 0.40*(n/len)/text_h exactly"
    );
}

struct Iran {
    arc_w: f32,
    h_cap: f32,
    w_nat_px: f32,
}

fn iran_row() -> Iran {
    let raw = include_str!("fixtures/mapname/frame4700_labels.json");
    let v: serde_json::Value = serde_json::from_str(raw).expect("answer key parses");
    assert_eq!(
        v["schema"].as_str(),
        Some("battle-plan-frame4700-mapname-answer-key-v1")
    );
    assert_eq!(v["cap_px"].as_f64(), Some(f64::from(CAP_PX)));
    let row = v["labels"]
        .as_array()
        .expect("labels array")
        .iter()
        .find(|r| r["tag"].as_str() == Some("PER") && r["text"].as_str() == Some("IRAN"))
        .expect("IRAN (PER) row present in the answer key");
    Iran {
        arc_w: row["arc_w"].as_f64().unwrap() as f32,
        h_cap: row["h_cap"].as_f64().unwrap() as f32,
        w_nat_px: row["w_nat_px"].as_f64().unwrap() as f32,
    }
}

#[test]
fn iran_old_eigen_minor_law_mis_fits_but_nlen_reproduces_the_engine() {
    let iran = iran_row();

    let fill_h = iran.arc_w / iran.w_nat_px * CAP_PX;
    let fill_ratio = fill_h / iran.h_cap;
    assert!(
        (fill_ratio - 1.0).abs() < 0.05,
        "IRAN is a fill-regime label: fill {fill_h:.3} vs engine {:.3} (ratio {fill_ratio:.3})",
        iran.h_cap
    );

    const IRAN_EIGEN_MINOR: f32 = 164.0;
    let old_cap_h = LABEL_HEIGHT_BREADTH_K * IRAN_EIGEN_MINOR;
    let old_h = fill_h.min(old_cap_h);
    let old_ratio = old_h / iran.h_cap;
    assert!(
        (old_ratio - 0.807).abs() < 0.01,
        "regression check on the DEFECT: old eigen-minor ratio should reproduce \
         b2's ~0.81 (got {old_ratio:.3})"
    );
    assert!(
        old_ratio < 0.85,
        "the retired eigen-minor law under-sizes IRAN (ratio {old_ratio:.3})"
    );

    let nlen_floor = fill_h * LINE_HEIGHT / (ENGINE_TITLE_CAP_COEF * CAP_PX);
    assert!(
        nlen_floor > IRAN_EIGEN_MINOR,
        "the FIX: IRAN's n/len mean width (>= {nlen_floor:.1}) must exceed its \
         eigen-minor {IRAN_EIGEN_MINOR} — that is why n/len draws fill where the \
         eigen-minor proxy wrongly capped"
    );

    let len = iran.arc_w / ENGINE_TITLE_FIT_COEF;
    let half_length = 0.5 * len;
    let n = (nlen_floor * len).ceil() as u64 + 1;
    let p = size_placement(half_length, n);
    let gs = p.engine_glyph_scale(iran.w_nat_px, LINE_HEIGHT);
    assert!(
        !gs.cap_bound,
        "IRAN must be fill-bound under the n/len law (cap_bound={})",
        gs.cap_bound
    );
    let new_h = gs.scale * CAP_PX;
    let new_ratio = new_h / iran.h_cap;
    assert!(
        (new_ratio - 1.0).abs() < 0.05,
        "n/len law must reproduce the engine IRAN height: {new_h:.3} vs {:.3} \
         (ratio {new_ratio:.3})",
        iran.h_cap
    );
    assert!(
        (new_ratio - 1.0).abs() < (old_ratio - 1.0).abs(),
        "n/len (ratio {new_ratio:.3}) must fit IRAN better than eigen-minor \
         (ratio {old_ratio:.3})"
    );
}
