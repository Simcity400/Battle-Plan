
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use map_viewer::{
    compute_country_label_placements_with_text, localisation_table_from_resolver, parse_fnt_chars,
    CountryLabelPlacement, GameFileResolver, LabelLawInputs, LabelTextMetrics, ProvinceKind,
};

struct Bundle {
    width: usize,
    height: usize,
    province_ids: Vec<u32>,
    province_to_state: BTreeMap<u32, u32>,
    owners: BTreeMap<u32, String>,
    impassable_states: BTreeSet<u32>,
    province_importance: BTreeMap<u32, u64>,
    province_type: BTreeMap<u32, ProvinceKind>,
}

fn bundle_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("out/map_bundle")
}

fn load_bundle(dir: &Path) -> Option<Bundle> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).ok()?).ok()?;
    let width = manifest["dimensions"]["width"].as_u64()? as usize;
    let height = manifest["dimensions"]["height"].as_u64()? as usize;
    let file = |key: &str, default: &str| -> String {
        manifest["files"][key]
            .as_str()
            .unwrap_or(default)
            .to_string()
    };
    let bytes = std::fs::read(dir.join(file("province_ids", "province_ids.r32"))).ok()?;
    if bytes.len() != width * height * 4 {
        return None;
    }
    let province_ids = bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let p2s: BTreeMap<String, u32> = serde_json::from_slice(
        &std::fs::read(dir.join(file("province_to_state", "province_to_state.json"))).ok()?,
    )
    .ok()?;
    let province_to_state = p2s
        .into_iter()
        .filter_map(|(k, v)| k.parse::<u32>().ok().map(|k| (k, v)))
        .collect();
    let own: BTreeMap<String, String> = serde_json::from_slice(
        &std::fs::read(dir.join(file("state_owner_1936", "state_owner_1936.json"))).ok()?,
    )
    .ok()?;
    let owners = own
        .into_iter()
        .filter_map(|(k, v)| k.parse::<u32>().ok().map(|k| (k, v)))
        .collect();
    let meta: BTreeMap<String, serde_json::Value> = serde_json::from_slice(
        &std::fs::read(dir.join(file("state_meta", "state_meta.json"))).ok()?,
    )
    .ok()?;
    let mut impassable_states = BTreeSet::new();
    let mut province_importance = BTreeMap::new();
    for (id, value) in &meta {
        let Ok(id) = id.parse::<u32>() else { continue };
        if value["impassable"].as_bool() == Some(true) {
            impassable_states.insert(id);
        }
        if let Some(vps) = value["victory_points"].as_object() {
            for (pid, weight) in vps {
                if let (Ok(pid), Some(w)) = (pid.parse::<u32>(), weight.as_u64()) {
                    province_importance.insert(pid, w);
                }
            }
        }
    }
    let pdefs: BTreeMap<String, serde_json::Value> = serde_json::from_slice(
        &std::fs::read(dir.join(file("province_definitions", "province_definitions.json"))).ok()?,
    )
    .ok()?;
    let mut province_type = BTreeMap::new();
    for (pid, def) in &pdefs {
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        let kind = def
            .get("province_type")
            .and_then(|t| t.as_str())
            .map(ProvinceKind::from_type_str)
            .unwrap_or(ProvinceKind::Land);
        province_type.insert(pid, kind);
    }
    Some(Bundle {
        width,
        height,
        province_ids,
        province_to_state,
        owners,
        impassable_states,
        province_importance,
        province_type,
    })
}

fn oracle_text_metrics(
    rows: &[serde_json::Value],
    game: &Path,
) -> BTreeMap<String, LabelTextMetrics> {
    let fnt_path = game.join("gfx/fonts/hoi_mapfont4.fnt");
    let fnt_text = std::fs::read_to_string(&fnt_path).expect("read shipped map font");
    let (_, _, chars) = parse_fnt_chars(&fnt_text);
    let advance_of = |c: char| -> Option<f32> {
        chars
            .iter()
            .find(|fc| fc.id == c as u32)
            .map(|fc| fc.xadvance)
    };
    let Some(space_advance) = advance_of(' ') else {
        eprintln!("SKIP text metrics: font has no space glyph");
        return BTreeMap::new();
    };
    let mut texts: BTreeMap<String, String> = BTreeMap::new();
    let key: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/mapname/frame4700_labels.json"))
            .expect("answer key parses");
    if let Some(labels) = key["labels"].as_array() {
        for r in labels {
            if let (Some(tag), Some(text)) = (r["tag"].as_str(), r["text"].as_str()) {
                texts.entry(tag.to_string()).or_insert(text.to_string());
            }
        }
    }
    let loc = GameFileResolver::vanilla(game.to_path_buf())
        .ok()
        .and_then(|resolver| localisation_table_from_resolver(&resolver, "english").ok())
        .unwrap_or_default();
    let mut metrics = BTreeMap::new();
    for r in rows {
        let tag = r["tag"].as_str().unwrap();
        let oracle_glyphs = r["glyphs"].as_u64().unwrap_or(0) as usize;
        if metrics.contains_key(tag) {
            continue;
        }
        let Some(text) = texts
            .get(tag)
            .cloned()
            .or_else(|| loc.get(tag).map(|s| s.to_uppercase()))
        else {
            eprintln!("  no display text for {tag}; straight fallback");
            continue;
        };
        let non_space = text.chars().filter(|c| !c.is_whitespace()).count();
        if oracle_glyphs != 0 && non_space != oracle_glyphs {
            eprintln!(
                "  {tag}: derived text {text:?} has {non_space} glyphs, oracle drew \
                 {oracle_glyphs}; straight fallback"
            );
            continue;
        }
        let mut advances = Vec::with_capacity(text.chars().count());
        let mut ok = true;
        for c in text.chars() {
            match advance_of(c) {
                Some(a) => advances.push((a, c.is_whitespace())),
                None => {
                    eprintln!("  {tag}: font lacks {c:?}; straight fallback");
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        metrics.insert(
            tag.to_string(),
            LabelTextMetrics {
                advances,
                line_height_px: 84.0,
                space_advance_px: space_advance,
                straight_layout_stations: Vec::new(),
            },
        );
    }
    metrics
}

fn ang_err_deg(a: f32, b: f32) -> f32 {
    let mut d = (a - b).rem_euclid(180.0);
    if d > 90.0 {
        d = 180.0 - d;
    }
    d
}

struct Score {
    n: usize,
    median: f32,
    p90: f32,
    max: f32,
    chile: Option<f32>,
    ang_median: f32,
    ang_p90: f32,
    ang_max: f32,
    ang_flip_median: f32,
    worst: Vec<(String, f32)>,
    curved: usize,
}

fn score(placements: &[CountryLabelPlacement], rows: &[serde_json::Value]) -> Score {
    let mut dists: Vec<f32> = Vec::new();
    let mut chile: Option<f32> = None;
    let mut worst: Vec<(String, f32)> = Vec::new();
    let mut ang_same: Vec<f32> = Vec::new();
    let mut ang_flip: Vec<f32> = Vec::new();
    for r in rows {
        let tag = r["tag"].as_str().unwrap();
        let ex = r["anchor_x"].as_f64().unwrap() as f32;
        let ey = r["anchor_y"].as_f64().unwrap() as f32;
        let eang = r["angle_deg"].as_f64().unwrap() as f32;
        let best = placements.iter().filter(|p| p.tag == tag).min_by(|a, b| {
            (a.anchor_x - ex)
                .hypot(a.anchor_y - ey)
                .total_cmp(&(b.anchor_x - ex).hypot(b.anchor_y - ey))
        });
        let Some(p) = best else { continue };
        let d = (p.anchor_x - ex).hypot(p.anchor_y - ey);
        dists.push(d);
        worst.push((tag.to_string(), d));
        let pang = p.angle.to_degrees();
        ang_same.push(ang_err_deg(pang, eang));
        ang_flip.push(ang_err_deg(-pang, eang));
        if tag == "CHL" {
            chile = Some(d);
        }
    }
    worst.sort_by(|a, b| b.1.total_cmp(&a.1));
    let stat = |v: &[f32]| -> (f32, f32, f32) {
        let mut s = v.to_vec();
        s.sort_by(f32::total_cmp);
        (
            s[s.len() / 2],
            s[(s.len() as f32 * 0.9) as usize],
            s[s.len() - 1],
        )
    };
    let (median, p90, max) = stat(&dists);
    let (ang_median, ang_p90, ang_max) = stat(&ang_same);
    let (ang_flip_median, _, _) = stat(&ang_flip);
    Score {
        n: dists.len(),
        median,
        p90,
        max,
        chile,
        ang_median,
        ang_p90,
        ang_max,
        ang_flip_median,
        worst,
        curved: placements.iter().filter(|p| p.curve.is_some()).count(),
    }
}

#[test]
#[cfg_attr(not(gate_map_bundle_with_hoi4), ignore = "the map bundle directory at map_viewer/out/map_bundle plus the game install")]
fn oracle63_engine_baseline_reproduces() {
    let dir = replay_test_gates::require_existing_dir(bundle_dir());
    let bundle = load_bundle(&dir).expect("load map bundle");
    let game = replay_test_gates::require_vanilla_game_root();
    let inputs = LabelLawInputs {
        province_ids: &bundle.province_ids,
        width: bundle.width,
        height: bundle.height,
        province_to_state: &bundle.province_to_state,
        owners: &bundle.owners,
        impassable_states: &bundle.impassable_states,
        province_importance: &bundle.province_importance,
        province_type: &bundle.province_type,
    };
    let raw = include_str!("fixtures/mapname/engine_label_oracle.json");
    let v: serde_json::Value = serde_json::from_str(raw).expect("oracle parses");
    let rows = v["labels"].as_array().expect("labels array");

    let straight = compute_country_label_placements_with_text(&inputs, &BTreeMap::new());
    assert!(!straight.is_empty(), "1936 bundle must place labels");
    let metrics = oracle_text_metrics(rows, &game);
    let t0 = std::time::Instant::now();
    let curved = compute_country_label_placements_with_text(&inputs, &metrics);
    let elapsed = t0.elapsed();

    eprintln!("emission: {} labels on the 1936 owner raster", curved.len());
    assert!(
        curved.len() < 700,
        "emission count {} regressed toward pixel-fragmentation \
         (province-atomic ~399, pixel-split was 1238)",
        curved.len()
    );
    assert_eq!(
        straight.len(),
        curved.len(),
        "the medial-curve stage must not add or drop labels"
    );

    let before = score(&straight, rows);
    let after = score(&curved, rows);
    eprintln!(
        "oracle63 STRAIGHT: n={} anchor median={:.1} p90={:.1} max={:.1} CHILE={:.1} | \
         angle median={:.2} p90={:.2} max={:.2}",
        before.n,
        before.median,
        before.p90,
        before.max,
        before.chile.unwrap_or(-1.0),
        before.ang_median,
        before.ang_p90,
        before.ang_max,
    );
    eprintln!(
        "oracle63 CURVED  : n={} anchor median={:.1} p90={:.1} max={:.1} CHILE={:.1} | \
         angle median={:.2} p90={:.2} max={:.2} | metrics for {} tags, {} curved \
         placements, compute={:?}",
        after.n,
        after.median,
        after.p90,
        after.max,
        after.chile.unwrap_or(-1.0),
        after.ang_median,
        after.ang_p90,
        after.ang_max,
        metrics.len(),
        after.curved,
        elapsed,
    );
    eprintln!(
        "  worst 8 anchor (curved): {:?}",
        &after.worst[..after.worst.len().min(8)]
    );

    assert!(
        after.n >= 60,
        "only {}/63 oracle anchors matched a same-tag placement",
        after.n
    );
    assert!(
        after.median < 8.0,
        "engine-baseline anchor median drifted: {:.1} px (port baseline ~3.7)",
        after.median
    );
    let chile = after.chile.expect("CHILE must place");
    assert!(
        chile < 15.0,
        "CHILE anchor drifted: {chile:.1} px — the deep-south thin-country acid \
         test (straight-port baseline ~5.0; the stride-4 grid shattered it at >250)"
    );
    assert!(
        after.p90 < 30.0,
        "engine-baseline anchor p90 drifted: {:.1} px (port baseline ~17)",
        after.p90
    );
    assert!(
        (after.ang_median - after.ang_flip_median).abs() > 10.0
            && after.ang_median < after.ang_flip_median,
        "angle sign convention ambiguous: same={:.1} flip={:.1}",
        after.ang_median,
        after.ang_flip_median
    );
    assert!(
        after.ang_median < 5.0,
        "engine-baseline angle median drifted: {:.2} deg (port baseline ~3.1)",
        after.ang_median
    );
    assert!(
        after.ang_p90 < 35.0,
        "engine-baseline angle p90 drifted: {:.2} deg (port baseline ~17)",
        after.ang_p90
    );
    assert!(
        after.median <= before.median + 1.0,
        "medial curve REGRESSED the anchor median: {:.2} -> {:.2}",
        before.median,
        after.median
    );
    assert!(
        after.ang_median <= before.ang_median + 1.0,
        "medial curve REGRESSED the angle median: {:.2} -> {:.2}",
        before.ang_median,
        after.ang_median
    );
    if !metrics.is_empty() {
        assert!(
            after.curved >= 30,
            "text metrics for {} tags but only {} placements curved",
            metrics.len(),
            after.curved
        );
    }
}
